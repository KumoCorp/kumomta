use crate::checkpoint::CheckpointData;
use crate::decompress::FileDecompressor;
use camino::Utf8PathBuf;
use filenamegen::Glob;
use futures::Stream;
use notify::event::{CreateKind, ModifyKind};
use notify::{Event, EventKind, Watcher};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// Configuration for constructing a [`LogTailer`].
#[derive(Deserialize, Serialize)]
pub struct LogTailerConfig {
    /// The directory containing zstd-compressed JSONL log files.
    pub directory: Utf8PathBuf,
    /// Glob pattern for matching log filenames. Defaults to `"*"`.
    #[serde(default = "default_pattern")]
    pub pattern: String,
    /// Maximum number of records per batch.
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,
    /// Maximum time to wait for a partial batch to fill before yielding it.
    #[serde(default = "default_max_batch_latency", with = "duration_serde")]
    pub max_batch_latency: Duration,
    /// If set, enables checkpoint persistence with this name.
    /// The checkpoint file will be stored as `.<name>` in the log directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_name: Option<String>,
    /// If set, use a polling-based filesystem watcher with the given
    /// poll interval instead of the platform's native filesystem
    /// notification mechanism. This can be useful in environments where
    /// native watchers are unreliable (e.g., network filesystems, some
    /// container runtimes).
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub poll_watcher: Option<Duration>,
    /// If true, ignore any existing checkpoint and start tailing from
    /// the most recent log segment. Subsequent segments will be
    /// processed in order as they appear.
    #[serde(default)]
    pub tail: bool,
}

fn default_pattern() -> String {
    "*".to_string()
}

fn default_max_batch_size() -> usize {
    100
}

fn default_max_batch_latency() -> Duration {
    Duration::from_secs(1)
}

impl LogTailerConfig {
    /// Create a new config with required fields and sensible defaults.
    pub fn new(directory: Utf8PathBuf) -> Self {
        Self {
            directory,
            pattern: default_pattern(),
            max_batch_size: default_max_batch_size(),
            max_batch_latency: default_max_batch_latency(),
            checkpoint_name: None,
            poll_watcher: None,
            tail: false,
        }
    }

    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = pattern.into();
        self
    }

    pub fn max_batch_size(mut self, size: usize) -> Self {
        self.max_batch_size = size;
        self
    }

    pub fn max_batch_latency(mut self, latency: Duration) -> Self {
        self.max_batch_latency = latency;
        self
    }

    pub fn checkpoint_name(mut self, name: impl Into<String>) -> Self {
        self.checkpoint_name = Some(name.into());
        self
    }

    pub fn poll_watcher(mut self, interval: Duration) -> Self {
        self.poll_watcher = Some(interval);
        self
    }

    pub fn tail(mut self, enable: bool) -> Self {
        self.tail = enable;
        self
    }

    /// Find the most recent log segment and return a synthetic checkpoint
    /// pointing to line 0 of that file, so the tailer starts there.
    fn resolve_tail_checkpoint(&self) -> anyhow::Result<Option<CheckpointData>> {
        let glob = Glob::new(&self.pattern)?;
        let mut files = vec![];
        for path in glob.walk(&self.directory) {
            let path = self
                .directory
                .join(Utf8PathBuf::try_from(path).map_err(|e| anyhow::anyhow!("{e}"))?);
            if path.is_file() {
                files.push(path);
            }
        }
        files.sort();
        Ok(files.last().map(|f| CheckpointData {
            file: f.to_string(),
            line: 0,
        }))
    }

    /// Build the tailer. Loads the checkpoint (if any) and sets up
    /// the filesystem watcher.
    pub async fn build(self) -> anyhow::Result<LogTailer> {
        let checkpoint_path = self
            .checkpoint_name
            .as_ref()
            .map(|name| self.directory.join(format!(".{name}")));

        let checkpoint = if self.tail {
            // When tailing, ignore any saved checkpoint and start from
            // the most recent segment.
            self.resolve_tail_checkpoint()?
        } else if let Some(cp_path) = &checkpoint_path {
            CheckpointData::load(cp_path).await?
        } else {
            None
        };

        let closed = Arc::new(AtomicBool::new(false));
        let close_notify = Arc::new(Notify::new());

        // Set up filesystem watcher bridged to async
        let fs_notify = Arc::new(Notify::new());
        let fs_notify_tx = fs_notify.clone();
        let event_handler = move |res: Result<Event, _>| match res {
            Ok(event) => match event.kind {
                EventKind::Create(CreateKind::File) | EventKind::Modify(ModifyKind::Data(_)) => {
                    fs_notify_tx.notify_one();
                }
                _ => {}
            },
            Err(_) => {}
        };
        let mut watcher: Box<dyn Watcher + Send> = if let Some(interval) = self.poll_watcher {
            Box::new(notify::PollWatcher::new(
                event_handler,
                notify::Config::default().with_poll_interval(interval),
            )?)
        } else {
            Box::new(notify::recommended_watcher(event_handler)?)
        };
        watcher.watch(
            &self.directory.clone().into_std_path_buf(),
            notify::RecursiveMode::NonRecursive,
        )?;

        let shared = Arc::new(TailerShared {
            closed,
            close_notify,
            checkpoint_path,
        });

        let current_file_path = Arc::new(tokio::sync::Mutex::new(None));
        let current_line_number = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let stream = make_stream(
            self.directory,
            self.pattern,
            self.max_batch_size,
            self.max_batch_latency,
            checkpoint,
            fs_notify,
            shared.clone(),
            current_file_path.clone(),
            current_line_number.clone(),
        );

        Ok(LogTailer {
            close_handle: CloseHandle {
                shared,
                current_file_path,
                current_line_number,
            },
            _watcher: watcher,
            stream: Box::pin(stream),
        })
    }
}

struct TailerShared {
    closed: Arc<AtomicBool>,
    close_notify: Arc<Notify>,
    checkpoint_path: Option<Utf8PathBuf>,
}

/// A `Send + Sync` handle that can close a [`LogTailer`] from any context.
///
/// Obtained via [`LogTailer::close_handle`].  Calling [`CloseHandle::close`]
/// writes the final checkpoint (if enabled) and signals the stream to
/// terminate.
#[derive(Clone)]
pub struct CloseHandle {
    shared: Arc<TailerShared>,
    current_file_path: Arc<tokio::sync::Mutex<Option<Utf8PathBuf>>>,
    current_line_number: Arc<std::sync::atomic::AtomicUsize>,
}

impl CloseHandle {
    /// Immediately write the final checkpoint (if checkpointing is enabled)
    /// and signal the stream to terminate.  Any in-progress or subsequent
    /// `poll_next` on the associated [`LogTailer`] will return `None`.
    pub async fn close(&self) -> anyhow::Result<()> {
        if let Some(cp_path) = &self.shared.checkpoint_path {
            let path_guard = self.current_file_path.lock().await;
            if let Some(path) = path_guard.as_ref() {
                let line = self
                    .current_line_number
                    .load(std::sync::atomic::Ordering::SeqCst);
                CheckpointData::save(cp_path, path, line).await?;
            }
        }
        self.shared.closed.store(true, Ordering::SeqCst);
        self.shared.close_notify.notify_waiters();
        Ok(())
    }

    /// Return the path of the log segment currently being read, if any.
    pub async fn current_file(&self) -> Option<Utf8PathBuf> {
        self.current_file_path.lock().await.clone()
    }
}

/// An async Stream that yields batches of log records from zstd-compressed
/// JSONL log files in a directory.
///
/// Call [`LogTailer::close`] or use a [`CloseHandle`] to write a final
/// checkpoint and terminate the stream.
pub struct LogTailer {
    close_handle: CloseHandle,
    _watcher: Box<dyn Watcher + Send>,
    stream: std::pin::Pin<Box<dyn Stream<Item = anyhow::Result<Vec<String>>> + Send>>,
}

impl LogTailer {
    /// Obtain a [`CloseHandle`] that can close this tailer from another
    /// task or context.  The handle is `Send + Sync`.
    pub fn close_handle(&self) -> CloseHandle {
        self.close_handle.clone()
    }

    /// Immediately write the final checkpoint (if checkpointing is enabled)
    /// and signal the stream to terminate. Any in-progress or subsequent
    /// `poll_next` will return `None`.
    pub async fn close(&self) -> anyhow::Result<()> {
        self.close_handle.close().await
    }
}

impl Stream for LogTailer {
    type Item = anyhow::Result<Vec<String>>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.close_handle.shared.closed.load(Ordering::SeqCst) {
            return std::task::Poll::Ready(None);
        }
        self.stream.as_mut().poll_next(cx)
    }
}

/// Build the file plan: sorted list of matching files in the directory,
/// filtered by checkpoint/last_processed.
fn build_plan(
    directory: &Utf8PathBuf,
    pattern: &str,
    checkpoint_path: &Option<Utf8PathBuf>,
    last_processed: &Option<Utf8PathBuf>,
    checkpoint: &Option<CheckpointData>,
) -> anyhow::Result<Vec<Utf8PathBuf>> {
    let glob = Glob::new(pattern)?;
    let mut result = vec![];
    for path in glob.walk(directory) {
        let path = directory.join(Utf8PathBuf::try_from(path).map_err(|e| anyhow::anyhow!("{e}"))?);
        // Skip checkpoint files
        if let Some(cp_path) = checkpoint_path {
            if &path == cp_path {
                continue;
            }
        }
        if path.is_file() {
            result.push(path);
        }
    }
    result.sort();

    if let Some(last) = last_processed {
        result.retain(|item| item > last);
    } else if let Some(cp) = checkpoint {
        let cp_file = &cp.file;
        result.retain(|item| item.as_str() >= cp_file.as_str());
    }

    Ok(result)
}

/// Check if a file is done (readonly = writer finished the segment).
fn is_file_done(path: &Utf8PathBuf) -> bool {
    path.metadata()
        .map(|m| m.permissions().readonly())
        .unwrap_or(false)
}

/// Save checkpoint synchronously (small JSON write).
fn save_checkpoint_sync(cp_path: &Utf8PathBuf, file: &Utf8PathBuf, line: usize) {
    let data = CheckpointData {
        file: file.to_string(),
        line,
    };
    if let Ok(json) = serde_json::to_string(&data) {
        let _ = std::fs::write(cp_path.as_std_path(), json);
    }
}

fn make_stream(
    directory: Utf8PathBuf,
    pattern: String,
    max_batch_size: usize,
    max_batch_latency: Duration,
    initial_checkpoint: Option<CheckpointData>,
    fs_notify: Arc<Notify>,
    shared: Arc<TailerShared>,
    pos_file: Arc<tokio::sync::Mutex<Option<Utf8PathBuf>>>,
    pos_line: Arc<std::sync::atomic::AtomicUsize>,
) -> impl Stream<Item = anyhow::Result<Vec<String>>> + Send {
    async_stream::try_stream! {
        let mut last_processed: Option<Utf8PathBuf> = None;
        let mut checkpoint = initial_checkpoint;
        let mut skip_lines: usize;
        let retry_delay = Duration::from_millis(200);
        // Deferred checkpoint: written at the start of the next iteration
        // after the caller has consumed the yielded batch.
        let mut pending_checkpoint: Option<(Utf8PathBuf, usize)> = None;

        'outer: loop {
            if shared.closed.load(Ordering::SeqCst) {
                break;
            }

            let plan = build_plan(
                &directory,
                &pattern,
                &shared.checkpoint_path,
                &last_processed,
                &checkpoint,
            )?;

            if plan.is_empty() {
                // Wait for filesystem changes or close
                tokio::select! {
                    _ = shared.close_notify.notified() => break,
                    _ = fs_notify.notified() => continue,
                }
            }

            // Determine skip_lines for first file if resuming from checkpoint
            if let Some(cp) = &checkpoint {
                if plan.first().map(|p| p.as_str()) == Some(cp.file.as_str()) {
                    skip_lines = cp.line;
                } else {
                    skip_lines = 0;
                }
            } else {
                skip_lines = 0;
            }
            checkpoint.take();

            for path in &plan {
                if shared.closed.load(Ordering::SeqCst) {
                    break 'outer;
                }

                let path_std = path.as_std_path().to_owned();
                let mut decomp = FileDecompressor::open(&path_std)?;

                // Update shared position so close() can checkpoint
                {
                    let mut pf = pos_file.lock().await;
                    *pf = Some(path.clone());
                }
                pos_line.store(decomp.lines_consumed, std::sync::atomic::Ordering::SeqCst);

                loop {
                    if shared.closed.load(Ordering::SeqCst) {
                        break 'outer;
                    }

                    // Flush pending checkpoint from the prior yield.
                    // At this point the caller has consumed the previous batch.
                    if let Some((ref cp_file, cp_line)) = pending_checkpoint.take() {
                        if let Some(cp_path) = &shared.checkpoint_path {
                            save_checkpoint_sync(cp_path, cp_file, cp_line);
                        }
                    }

                    let mut batch = Vec::new();
                    let mut hit_eof = false;
                    let batch_deadline = tokio::time::Instant::now() + max_batch_latency;

                    // Fill the batch
                    loop {
                        if shared.closed.load(Ordering::SeqCst) {
                            break 'outer;
                        }

                        match decomp.next_line(skip_lines) {
                            Ok(Some(line)) => {
                                batch.push(line);
                                if batch.len() >= max_batch_size {
                                    break;
                                }
                            }
                            Ok(None) => {
                                // EOF — no more data right now
                                hit_eof = true;
                                break;
                            }
                            Err(e) => {
                                Err(e)?;
                            }
                        }
                    }

                    if !batch.is_empty() {
                        // If we hit EOF and batch is partial, wait for more
                        // data up to max_batch_latency (unless file is done)
                        if hit_eof && batch.len() < max_batch_size && !is_file_done(path) {
                            // Wait for more data up to the batch deadline
                            loop {
                                let remaining = batch_deadline.saturating_duration_since(tokio::time::Instant::now());
                                if remaining.is_zero() {
                                    break;
                                }
                                tokio::select! {
                                    _ = shared.close_notify.notified() => break 'outer,
                                    _ = tokio::time::sleep(remaining.min(retry_delay)) => {},
                                    _ = fs_notify.notified() => {},
                                }
                                // Reset EOF so we can try reading more
                                decomp.reset_eof();
                                loop {
                                    match decomp.next_line(skip_lines) {
                                        Ok(Some(line)) => {
                                            batch.push(line);
                                            if batch.len() >= max_batch_size {
                                                break;
                                            }
                                        }
                                        Ok(None) => {
                                            if is_file_done(path) {
                                                hit_eof = true;
                                            }
                                            break;
                                        }
                                        Err(e) => Err(e)?,
                                    }
                                }
                                if batch.len() >= max_batch_size || (hit_eof && is_file_done(path)) {
                                    break;
                                }
                            }
                        }

                        // Update shared position; defer checkpoint write
                        // until the next iteration, after the caller has
                        // consumed this batch.
                        pos_line.store(decomp.lines_consumed, std::sync::atomic::Ordering::SeqCst);
                        pending_checkpoint = Some((path.clone(), decomp.lines_consumed));

                        skip_lines = 0;
                        yield batch;

                        if hit_eof && is_file_done(path) {
                            if decomp.has_partial_data() {
                                Err(anyhow::anyhow!(
                                    "unexpected EOF for {} with partial line data remaining",
                                    path
                                ))?;
                            }
                            last_processed = Some(path.clone());
                            break; // Move to next file
                        }
                        // Not EOF or file not done, continue reading this file
                        continue;
                    }

                    // Empty batch at EOF
                    if hit_eof {
                        if is_file_done(path) {
                            if decomp.has_partial_data() {
                                Err(anyhow::anyhow!(
                                    "unexpected EOF for {} with partial line data remaining",
                                    path
                                ))?;
                            }
                            last_processed = Some(path.clone());
                            skip_lines = 0;
                            break; // Move to next file
                        }

                        // File not done, wait for more data
                        tokio::select! {
                            _ = shared.close_notify.notified() => break 'outer,
                            _ = tokio::time::sleep(retry_delay) => {},
                            _ = fs_notify.notified() => {},
                        }
                        // Reset EOF flag for retry
                        decomp.reset_eof();
                    }
                }
            }
        }
    }
}
