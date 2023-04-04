use anyhow::Context;
use camino::Utf8PathBuf;
use clap::Parser;
use filenamegen::Glob;
use notify::event::{CreateKind, ModifyKind};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;
use thiserror::Error;
use zstd_safe::{DCtx, InBuffer, OutBuffer};

/// Tail logs
#[derive(Parser, Debug)]
#[command(about)]
struct Opt {
    /// Glob expression used to select matching log filenames.
    #[arg(long, default_value = "*")]
    pattern: String,

    /// The name of the checkpoint file that will be stored
    /// in the log directory
    #[arg(long, default_value = ".tailer-checkpoint")]
    checkpoint: String,

    /// When processing log lines, how many to feed into
    /// the actor at once
    #[arg(long, default_value = "1")]
    batch_size: usize,

    /// Ignore the checkpoint, just tail the logs, starting
    /// with the most recent segment
    #[arg(long)]
    tail: bool,

    /// The directory which contains the logs
    directory: Utf8PathBuf,
}

impl Opt {
    fn build_plan(&self) -> anyhow::Result<Vec<Utf8PathBuf>> {
        let glob = Glob::new(&self.pattern)?;
        let mut result = vec![];
        for path in glob.walk(&self.directory) {
            let path = self.directory.join(Utf8PathBuf::try_from(path)?);
            if path.is_file() {
                result.push(path);
            }
        }
        result.sort();
        Ok(result)
    }
}

fn is_file_done(path: &Utf8PathBuf) -> anyhow::Result<bool> {
    let perms = path
        .metadata()
        .with_context(|| format!("getting metadata for {path}"))?;
    Ok(perms.permissions().readonly())
}

#[derive(Error, Debug)]
#[error("{}", zstd_safe::get_error_name(self.0))]
struct ZStdError(usize);

struct FileTailerParams<'a> {
    path: &'a Utf8PathBuf,
    /// If the checkpoint file indicates that we were part way
    /// through through file, it will pass in the line number here
    skip_first_n_lines: usize,
    /// How many lines to batch together to pass to the actor
    batch_size: usize,

    /// function to act upon a batch
    actor: Box<dyn FnMut(&[String]) -> anyhow::Result<()>>,

    checkpoint: Option<&'a Utf8PathBuf>,
}

/// Tail data from a zstd compressed file segment.
/// When EOF is indicated, we don't immediately give up on
/// the file, but instead pause for a few seconds before trying
/// again.
///
/// The writer of the log file will remove the `w` bits from
/// the file permissions when it is finished writing to the segment,
/// so we look at those to determine if we should consider the
/// segment to be complete.
///
/// We're using the lower level zstd_safe functions for this because
/// the zstd crate has a few issues dealing with the EOF condition
/// that make it unsuitable for our tailing purposes.
fn tail_single_file(params: &mut FileTailerParams) -> anyhow::Result<()> {
    let mut file = BufReader::new(
        std::fs::File::open(params.path)
            .with_context(|| format!("opening {} for read", params.path))?,
    );

    let mut context = DCtx::create();
    context
        .init()
        .map_err(ZStdError)
        .context("initialize zstd decompression context")?;
    context
        .load_dictionary(&[])
        .map_err(ZStdError)
        .context("load empty dictionary")?;

    let mut out_buffer = vec![0u8; DCtx::out_size()];
    let mut line_start = 0;
    let mut out_pos = 0;
    let mut line_number = 0usize;

    let mut batch = vec![];

    loop {
        let in_buffer = file.fill_buf()?;
        if in_buffer.is_empty() {
            let done = is_file_done(params.path)?;

            if done {
                if out_pos > 0 {
                    anyhow::bail!(
                        "Error: unexpected EOF for {} with \
                        {out_pos} bytes of partial line data remaining",
                        params.path
                    );
                }
                break;
            }

            std::thread::sleep(Duration::from_secs(10));
            continue;
        }

        let mut src = InBuffer::around(in_buffer);
        let mut dest = OutBuffer::around_pos(&mut out_buffer, out_pos);

        context
            .decompress_stream(&mut dest, &mut src)
            .map_err(ZStdError)?;

        let bytes_read = {
            let pos = src.pos();
            drop(src);
            pos
        };
        file.consume(bytes_read);
        out_pos = dest.pos();

        while let Some(idx) = memchr::memchr(b'\n', &out_buffer[line_start..out_pos]) {
            if line_number >= params.skip_first_n_lines {
                let this_line = &out_buffer[line_start..line_start + idx];
                let line = String::from_utf8_lossy(this_line).into_owned();

                batch.push(line);

                if batch.len() == params.batch_size {
                    (params.actor)(&batch)?;
                    if let Some(cp) = &params.checkpoint {
                        CheckpointData::save(cp, &params.path, line_number + 1)?;
                    }
                    batch.clear();
                }
            }
            line_start += idx + 1;
            line_number += 1;
        }

        if line_start == out_pos {
            // Consumed whole buffer, just reset its start
            out_pos = 0;
            line_start = 0;
        } else {
            // Copy buffer down
            out_buffer.copy_within(line_start..out_pos, 0);
            out_pos -= line_start;
            line_start = 0;
        }
    }

    if !batch.is_empty() {
        (params.actor)(&batch)?;
        if let Some(cp) = &params.checkpoint {
            CheckpointData::save(cp, &params.path, line_number)?;
        }
    }

    Ok(())
}

#[derive(Deserialize, Serialize, Debug)]
struct CheckpointData {
    file: String,
    line: usize,
}

impl CheckpointData {
    pub fn load(path: &Utf8PathBuf) -> anyhow::Result<Option<Self>> {
        let f = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(err.into());
            }
        };
        let data: Self = serde_json::from_reader(f)?;
        Ok(Some(data))
    }

    pub fn save(
        checkpoint_path: &Utf8PathBuf,
        file: &Utf8PathBuf,
        line: usize,
    ) -> anyhow::Result<()> {
        let data = Self {
            file: file.to_string(),
            line,
        };
        std::fs::write(checkpoint_path, serde_json::to_string(&data)?)?;
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| match res {
        Ok(event) => match event.kind {
            EventKind::Create(CreateKind::File) | EventKind::Modify(ModifyKind::Data(_)) => {
                tx.send(()).ok();
            }
            _ => {}
        },
        Err(err) => {
            eprintln!("watcher error: {err:#}");
        }
    })
    .context("create filesystem watcher")?;
    watcher
        .watch(
            &opts.directory.clone().into_std_path_buf(),
            RecursiveMode::NonRecursive,
        )
        .with_context(|| format!("establish filesystem watch on {}", opts.directory))?;

    let mut last_processed = None;
    let mut first_time = true;

    let timeout = Duration::from_secs(60);

    let checkpoint_path = opts.directory.join(&opts.checkpoint);
    let mut checkpoint = if opts.tail {
        None
    } else {
        CheckpointData::load(&checkpoint_path)?
    };

    loop {
        let mut plan = opts.build_plan()?;
        if let Some(last) = &last_processed {
            plan.retain(|item| item > last);
        } else if let Some(cp) = &checkpoint {
            plan.retain(|item| item >= &cp.file);
        }

        let plan = if opts.tail && first_time && !plan.is_empty() {
            vec![plan.pop().expect("not empty")]
        } else {
            plan
        };

        first_time = false;

        if let Some(cp) = &checkpoint {
            let file = Utf8PathBuf::try_from(&cp.file)?;
            match plan.iter().position(|f| f == &file) {
                None => {
                    // There are no files available that include the file
                    // listed by the checkpoint.
                    // Perhaps someone removed them all?
                    // Let's fixup the checkpoint to proceed with what
                    // is currently available, by simply forgetting it
                    checkpoint.take();
                }
                Some(0) => {
                    // Good!
                }
                Some(n) => {
                    // Should be impossible, given that we filter >= cp.file
                    // at the top of this loop
                    anyhow::bail!(
                        "Checkpoint references {file} which is not the zeroth \
                        element of the plan, but is at position {n}. \
                        Refusing to proceed until you have manually resolved \
                        the inconsistency. Plan is {plan:?}"
                    );
                }
            }
        }

        for path in plan {
            eprintln!("{path}");

            let skip_first_n_lines = checkpoint.take().map(|cp| cp.line).unwrap_or(0);

            let mut params = FileTailerParams {
                path: &path,
                actor: Box::new(|batch| {
                    for line in batch {
                        println!("{line}");
                    }
                    Ok(())
                }),
                skip_first_n_lines,
                batch_size: opts.batch_size,
                checkpoint: if opts.tail {
                    None
                } else {
                    Some(&checkpoint_path)
                },
            };
            tail_single_file(&mut params)?;
            last_processed.replace(path);
        }

        eprintln!("waiting for more files");
        match rx.recv_timeout(timeout) {
            Ok(_) => {
                // Let's drain any others that might be pending
                while let Ok(_) = rx.try_recv() {
                    // drain
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                // No notifications.
                // Some systems are not compatible with filesystem
                // watches, so we'll just take a look now anyway
                // to see if something new showed up
            }
        }
    }

    Ok(())
}
