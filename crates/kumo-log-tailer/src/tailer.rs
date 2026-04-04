use crate::batch::LogBatch;
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

// ---------------------------------------------------------------------------
// Default helpers
// ---------------------------------------------------------------------------

fn default_pattern() -> String {
    "*".to_string()
}

fn default_max_batch_size() -> usize {
    100
}

fn default_max_batch_latency() -> Duration {
    Duration::from_secs(1)
}

// ---------------------------------------------------------------------------
// ConsumerConfig
// ---------------------------------------------------------------------------

/// Per-consumer batching, checkpoint, and filter configuration.
pub struct ConsumerConfig {
    /// A name that identifies this consumer.  Returned by
    /// [`LogBatch::consumer_name`].
    pub name: String,
    /// Maximum number of records per batch.
    pub max_batch_size: usize,
    /// Maximum time to wait for a partial batch to fill before yielding it.
    pub max_batch_latency: Duration,
    /// If set, enables checkpoint persistence with this name.
    /// The checkpoint file will be stored as `.<name>` in the log directory.
    pub checkpoint_name: Option<String>,
    /// Optional filter applied to each record.  If the filter returns
    /// `Ok(false)` the record is not added to this consumer's batch.
    pub filter: Option<Box<dyn Fn(&serde_json::Value) -> anyhow::Result<bool> + Send>>,
}

impl ConsumerConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            max_batch_size: default_max_batch_size(),
            max_batch_latency: default_max_batch_latency(),
            checkpoint_name: None,
            filter: None,
        }
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

    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: Fn(&serde_json::Value) -> anyhow::Result<bool> + Send + 'static,
    {
        self.filter = Some(Box::new(f));
        self
    }
}

// ---------------------------------------------------------------------------
// MultiConsumerTailerConfig
// ---------------------------------------------------------------------------

/// Configuration for a tailer that fans out records to multiple consumers.
pub struct MultiConsumerTailerConfig {
    /// The directory containing zstd-compressed JSONL log files.
    pub directory: Utf8PathBuf,
    /// Glob pattern for matching log filenames.
    pub pattern: String,
    /// If set, use a polling-based filesystem watcher.
    pub poll_watcher: Option<Duration>,
    /// If true, ignore checkpoints and start from the most recent segment.
    pub tail: bool,
    /// The set of consumers that receive records.
    pub consumers: Vec<ConsumerConfig>,
}

impl MultiConsumerTailerConfig {
    pub fn new(directory: Utf8PathBuf, consumers: Vec<ConsumerConfig>) -> Self {
        Self {
            directory,
            pattern: default_pattern(),
            poll_watcher: None,
            tail: false,
            consumers,
        }
    }

    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = pattern.into();
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

    /// Build the multi-consumer tailer.
    pub async fn build(self) -> anyhow::Result<MultiConsumerTailer> {
        // Collect checkpoint paths without borrowing consumers across
        // an await (consumers contains non-Sync filter closures).
        let cp_paths: Vec<Option<Utf8PathBuf>> = self
            .consumers
            .iter()
            .map(|c| {
                c.checkpoint_name
                    .as_ref()
                    .map(|name| self.directory.join(format!(".{name}")))
            })
            .collect();

        // Now load checkpoints (async) without borrowing consumers.
        let mut consumer_checkpoints: Vec<Option<CheckpointData>> =
            Vec::with_capacity(cp_paths.len());
        let mut earliest_checkpoint: Option<CheckpointData> = None;

        for cp_path in &cp_paths {
            let cp = if self.tail {
                resolve_tail_checkpoint(&self.directory, &self.pattern)?
            } else if let Some(cp_path) = cp_path {
                CheckpointData::load(cp_path).await?
            } else {
                None
            };

            match (&earliest_checkpoint, &cp) {
                (None, Some(cp)) => {
                    earliest_checkpoint = Some(cp.clone());
                }
                (Some(existing), Some(cp)) => {
                    if cp.file < existing.file
                        || (cp.file == existing.file && cp.line < existing.line)
                    {
                        earliest_checkpoint = Some(cp.clone());
                    }
                }
                _ => {}
            }

            consumer_checkpoints.push(cp);
        }

        let closed = Arc::new(AtomicBool::new(false));
        let close_notify = Arc::new(Notify::new());

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
        });

        let stream = make_multi_stream(
            self.directory,
            self.pattern,
            self.consumers,
            earliest_checkpoint,
            consumer_checkpoints,
            cp_paths,
            fs_notify,
            shared.clone(),
        );

        Ok(MultiConsumerTailer {
            close_handle: CloseHandle { shared },
            _watcher: watcher,
            stream: Box::pin(stream),
        })
    }
}

// ---------------------------------------------------------------------------
// Shared internals
// ---------------------------------------------------------------------------

struct TailerShared {
    closed: Arc<AtomicBool>,
    close_notify: Arc<Notify>,
}

/// A `Send + Sync` handle that can close a tailer from any context.
#[derive(Clone)]
pub struct CloseHandle {
    shared: Arc<TailerShared>,
}

impl CloseHandle {
    /// Signal the stream to terminate.
    pub fn close(&self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        self.shared.close_notify.notify_waiters();
    }
}

// ---------------------------------------------------------------------------
// MultiConsumerTailer
// ---------------------------------------------------------------------------

/// An async Stream that yields vectors of [`LogBatch`], one per consumer
/// whose batch is ready.
pub struct MultiConsumerTailer {
    close_handle: CloseHandle,
    _watcher: Box<dyn Watcher + Send>,
    stream: std::pin::Pin<Box<dyn Stream<Item = anyhow::Result<Vec<LogBatch>>> + Send>>,
}

impl MultiConsumerTailer {
    pub fn close_handle(&self) -> CloseHandle {
        self.close_handle.clone()
    }

    pub fn close(&self) {
        self.close_handle.close();
    }
}

impl Stream for MultiConsumerTailer {
    type Item = anyhow::Result<Vec<LogBatch>>;

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

// ---------------------------------------------------------------------------
// Single-consumer LogTailerConfig / LogTailer (delegates to multi-consumer)
// ---------------------------------------------------------------------------

/// Configuration for constructing a single-consumer [`LogTailer`].
#[derive(Deserialize, Serialize)]
pub struct LogTailerConfig {
    pub directory: Utf8PathBuf,
    #[serde(default = "default_pattern")]
    pub pattern: String,
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,
    #[serde(default = "default_max_batch_latency", with = "duration_serde")]
    pub max_batch_latency: Duration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_name: Option<String>,
    #[serde(
        default,
        with = "duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub poll_watcher: Option<Duration>,
    #[serde(default)]
    pub tail: bool,
}

impl LogTailerConfig {
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

    /// Build a single-consumer tailer.
    pub async fn build(self) -> anyhow::Result<LogTailer> {
        self.build_with_filter(None::<fn(&serde_json::Value) -> anyhow::Result<bool>>)
            .await
    }

    /// Build a single-consumer tailer with an optional record filter.
    pub async fn build_with_filter<F>(self, filter: Option<F>) -> anyhow::Result<LogTailer>
    where
        F: Fn(&serde_json::Value) -> anyhow::Result<bool> + Send + 'static,
    {
        let mut consumer = ConsumerConfig::new("default")
            .max_batch_size(self.max_batch_size)
            .max_batch_latency(self.max_batch_latency);
        if let Some(name) = self.checkpoint_name.clone() {
            consumer = consumer.checkpoint_name(name);
        }
        if let Some(f) = filter {
            consumer = consumer.filter(f);
        }

        let multi_config = MultiConsumerTailerConfig {
            directory: self.directory,
            pattern: self.pattern,
            poll_watcher: self.poll_watcher,
            tail: self.tail,
            consumers: vec![consumer],
        };

        let multi = multi_config.build().await?;

        Ok(LogTailer { inner: multi })
    }
}

/// A single-consumer async Stream that yields one [`LogBatch`] at a time.
///
/// This is a convenience wrapper around [`MultiConsumerTailer`] with
/// exactly one consumer.
pub struct LogTailer {
    inner: MultiConsumerTailer,
}

impl LogTailer {
    pub fn close_handle(&self) -> CloseHandle {
        self.inner.close_handle()
    }

    pub fn close(&self) {
        self.inner.close();
    }
}

impl Stream for LogTailer {
    type Item = anyhow::Result<LogBatch>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;
        // The inner multi-consumer stream yields Vec<LogBatch> with exactly
        // one element.  Unwrap it.
        match std::pin::Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(mut batches))) => Poll::Ready(Some(Ok(batches
                .pop()
                .expect("single consumer yields one batch")))),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared utilities
// ---------------------------------------------------------------------------

fn resolve_tail_checkpoint(
    directory: &Utf8PathBuf,
    pattern: &str,
) -> anyhow::Result<Option<CheckpointData>> {
    let glob = Glob::new(pattern)?;
    let mut files = vec![];
    for path in glob.walk(directory) {
        let path = directory.join(Utf8PathBuf::try_from(path).map_err(|e| anyhow::anyhow!("{e}"))?);
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

/// Build the file plan: sorted list of matching files in the directory.
fn build_plan(
    directory: &Utf8PathBuf,
    pattern: &str,
    checkpoint_paths: &[Option<Utf8PathBuf>],
    last_processed: &Option<Utf8PathBuf>,
    checkpoint: &Option<CheckpointData>,
) -> anyhow::Result<Vec<Utf8PathBuf>> {
    let glob = Glob::new(pattern)?;
    let mut result = vec![];
    for path in glob.walk(directory) {
        let path = directory.join(Utf8PathBuf::try_from(path).map_err(|e| anyhow::anyhow!("{e}"))?);
        // Skip checkpoint files
        if checkpoint_paths.iter().any(|cp| cp.as_ref() == Some(&path)) {
            continue;
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

fn is_file_done(path: &Utf8PathBuf) -> bool {
    path.metadata()
        .map(|m| m.permissions().readonly())
        .unwrap_or(false)
}

fn save_checkpoint_sync(cp_path: &Utf8PathBuf, file: &Utf8PathBuf, line: usize) {
    let data = CheckpointData {
        file: file.to_string(),
        line,
    };
    if let Ok(json) = serde_json::to_string(&data) {
        let _ = std::fs::write(cp_path.as_std_path(), json);
    }
}

// ---------------------------------------------------------------------------
// Per-consumer state used during stream construction
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Multi-consumer stream
// ---------------------------------------------------------------------------

fn make_multi_stream(
    directory: Utf8PathBuf,
    pattern: String,
    consumers: Vec<ConsumerConfig>,
    earliest_checkpoint: Option<CheckpointData>,
    mut consumer_checkpoints: Vec<Option<CheckpointData>>,
    cp_paths: Vec<Option<Utf8PathBuf>>,
    fs_notify: Arc<Notify>,
    shared: Arc<TailerShared>,
) -> impl Stream<Item = anyhow::Result<Vec<LogBatch>>> + Send {
    let num_consumers = consumers.len();

    // Extract per-consumer config into parallel vecs
    let consumer_names: Vec<String> = consumers.iter().map(|c| c.name.clone()).collect();
    let max_batch_sizes: Vec<usize> = consumers.iter().map(|c| c.max_batch_size).collect();
    let max_batch_latencies: Vec<Duration> =
        consumers.iter().map(|c| c.max_batch_latency).collect();
    let filters: Vec<Option<Box<dyn Fn(&serde_json::Value) -> anyhow::Result<bool> + Send>>> =
        consumers.into_iter().map(|c| c.filter).collect();

    async_stream::try_stream! {
        let mut last_processed: Option<Utf8PathBuf> = None;
        let mut global_checkpoint = earliest_checkpoint;
        let mut skip_lines: usize;
        let retry_delay = Duration::from_millis(200);

        // Per-consumer skip lines (for the first file only, when
        // resuming from checkpoint).  The global skip_lines is the
        // minimum across all consumers for that file, and individual
        // consumers that are further ahead will have their records
        // filtered out by index comparison.
        let mut consumer_skip: Vec<usize> = vec![0; num_consumers];

        'outer: loop {
            if shared.closed.load(Ordering::SeqCst) {
                break;
            }

            let plan = build_plan(
                &directory,
                &pattern,
                &cp_paths,
                &last_processed,
                &global_checkpoint,
            )?;

            if plan.is_empty() {
                tokio::select! {
                    _ = shared.close_notify.notified() => break,
                    _ = fs_notify.notified() => continue,
                }
            }

            // Determine skip_lines from the earliest checkpoint
            if let Some(cp) = &global_checkpoint {
                if plan.first().map(|p| p.as_str()) == Some(cp.file.as_str()) {
                    skip_lines = cp.line;
                } else {
                    skip_lines = 0;
                }
            } else {
                skip_lines = 0;
            }

            // Determine per-consumer skip lines
            for i in 0..num_consumers {
                if let Some(cp) = &consumer_checkpoints[i] {
                    if plan.first().map(|p| p.as_str()) == Some(cp.file.as_str()) {
                        consumer_skip[i] = cp.line;
                    } else {
                        consumer_skip[i] = 0;
                    }
                } else {
                    consumer_skip[i] = 0;
                }
            }
            global_checkpoint.take();
            for cp in consumer_checkpoints.iter_mut() {
                cp.take();
            }

            let mut plan_index = 0;
            let mut decomp: Option<FileDecompressor> = None;
            let mut current_path: Option<&Utf8PathBuf> = None;
            let mut last_lines_consumed: usize = 0;
            // Track the global line number for the current file so we
            // can apply per-consumer skip logic.
            let mut global_line_in_file: usize = skip_lines;

            if let Some(path) = plan.get(plan_index) {
                let path_std = path.as_std_path().to_owned();
                decomp = Some(FileDecompressor::open(&path_std)?);
                current_path = Some(path);
            }

            // Per-consumer batches and deadlines persist across
            // fill/yield cycles.  A consumer's batch is "ready" when
            // it is full or its deadline has expired.  Only ready
            // batches are yielded; others keep accumulating.
            let mut batches: Vec<LogBatch> = (0..num_consumers)
                .map(|i| LogBatch::with_consumer_name(consumer_names[i].clone()))
                .collect();
            let mut deadlines: Vec<Option<tokio::time::Instant>> = vec![None; num_consumers];

            while decomp.is_some() {
                if shared.closed.load(Ordering::SeqCst) {
                    break 'outer;
                }

                // Fill batches until at least one is ready
                'fill: loop {
                    if shared.closed.load(Ordering::SeqCst) {
                        break 'outer;
                    }

                    // Check if any consumer already has a ready batch
                    let now = tokio::time::Instant::now();
                    let any_ready = (0..num_consumers).any(|i| {
                        !batches[i].is_empty()
                            && (batches[i].len() >= max_batch_sizes[i]
                                || deadlines[i].map_or(false, |d| now >= d))
                    });
                    if any_ready {
                        break 'fill;
                    }

                    let d = decomp.as_mut().expect("checked above");
                    let path = current_path.expect("set with decomp");

                    match d.next_line(skip_lines) {
                        Ok(Some(line)) => {
                            let value: serde_json::Value = serde_json::from_str(&line.text)
                                .map_err(|err| {
                                    anyhow::anyhow!(
                                        "Failed to parse a line from {path} (byte offset {}) \
                                         as json: {err}. Is the file corrupt? You may need \
                                         to move the file aside to make progress",
                                        line.byte_offset
                                    )
                                })?;

                            for i in 0..num_consumers {
                                if global_line_in_file < consumer_skip[i] {
                                    continue;
                                }
                                if let Some(ref f) = filters[i] {
                                    if !f(&value)? {
                                        continue;
                                    }
                                }
                                batches[i].push_value(
                                    value.clone(),
                                    path,
                                    line.byte_offset,
                                );
                                // Start the deadline timer on first record
                                if deadlines[i].is_none() {
                                    deadlines[i] = Some(
                                        tokio::time::Instant::now() + max_batch_latencies[i],
                                    );
                                }
                            }
                            global_line_in_file += 1;
                        }
                        Ok(None) => {
                            // EOF on current file
                            if is_file_done(path) {
                                if d.has_partial_data() {
                                    Err(anyhow::anyhow!(
                                        "unexpected EOF for {} with partial line data remaining",
                                        path
                                    ))?;
                                }
                                last_lines_consumed = d.lines_consumed;
                                last_processed = Some(path.clone());
                                skip_lines = 0;
                                global_line_in_file = 0;
                                for cs in consumer_skip.iter_mut() {
                                    *cs = 0;
                                }

                                plan_index += 1;
                                if let Some(next_path) = plan.get(plan_index) {
                                    let path_std = next_path.as_std_path().to_owned();
                                    decomp = Some(FileDecompressor::open(&path_std)?);
                                    current_path = Some(next_path);
                                    continue 'fill;
                                } else {
                                    decomp = None;
                                    current_path = None;
                                    break 'fill;
                                }
                            }

                            // File not done; find the earliest deadline
                            // among non-empty batches to bound the wait.
                            let earliest_deadline = (0..num_consumers)
                                .filter(|&i| !batches[i].is_empty())
                                .filter_map(|i| deadlines[i])
                                .min();

                            if let Some(deadline) = earliest_deadline {
                                let remaining = deadline.saturating_duration_since(
                                    tokio::time::Instant::now(),
                                );
                                if remaining.is_zero() {
                                    break 'fill;
                                }
                                tokio::select! {
                                    _ = shared.close_notify.notified() => break 'outer,
                                    _ = tokio::time::sleep(remaining.min(retry_delay)) => {},
                                    _ = fs_notify.notified() => {},
                                }
                            } else {
                                // All batches empty, file not done — wait
                                tokio::select! {
                                    _ = shared.close_notify.notified() => break 'outer,
                                    _ = tokio::time::sleep(retry_delay) => {},
                                    _ = fs_notify.notified() => {},
                                }
                            }
                            d.reset_eof();
                        }
                        Err(e) => {
                            Err(e)?;
                        }
                    }
                }

                // Determine which batches are ready to yield
                let now = tokio::time::Instant::now();
                let mut ready: Vec<LogBatch> = Vec::new();
                for i in 0..num_consumers {
                    let is_ready = !batches[i].is_empty()
                        && (batches[i].len() >= max_batch_sizes[i]
                            || deadlines[i].map_or(false, |d| now >= d)
                            || decomp.is_none()); // end of plan: flush all

                    if !is_ready {
                        continue;
                    }

                    // Swap out the ready batch, replace with a fresh one
                    let mut batch = std::mem::replace(
                        &mut batches[i],
                        LogBatch::with_consumer_name(consumer_names[i].clone()),
                    );
                    deadlines[i] = None;

                    // Set the commit callback
                    if let Some(ref cp_path) = cp_paths[i] {
                        let (cp_file, cp_line) = if let Some(d) = &decomp {
                            let path = current_path.expect("set with decomp");
                            (path.clone(), d.lines_consumed)
                        } else if let Some(last) = &last_processed {
                            (last.clone(), last_lines_consumed)
                        } else {
                            unreachable!("non-empty batch without a source");
                        };
                        let cp_path = cp_path.clone();
                        batch.set_commit_fn(Box::new(move || {
                            save_checkpoint_sync(&cp_path, &cp_file, cp_line);
                            Ok(())
                        }));
                    }
                    ready.push(batch);
                }

                if !ready.is_empty() {
                    skip_lines = 0;
                    yield ready;
                }
            }
        }
    }
}
