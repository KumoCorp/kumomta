use anyhow::Context;
use camino::Utf8PathBuf;
use clap::Parser;
use filenamegen::Glob;
use notify::event::{CreateKind, ModifyKind};
use notify::{Event, EventKind, RecursiveMode, Watcher};
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
fn tail_single_file(path: &Utf8PathBuf) -> anyhow::Result<()> {
    let mut file = BufReader::new(
        std::fs::File::open(path).with_context(|| format!("opening {path} for read"))?,
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

    loop {
        let in_buffer = file.fill_buf()?;
        if in_buffer.is_empty() {
            let done = is_file_done(path)?;

            if done {
                if out_pos > 0 {
                    eprintln!("Error: unexpected EOF for {path} with {out_pos} bytes of partial line data remaining");
                }
                return Ok(());
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
            let this_line = &out_buffer[line_start..line_start + idx];
            println!("{}", String::from_utf8_lossy(this_line));
            line_start += idx + 1;
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

    // We've read all of the files, now we wait for more files to show up
    let timeout = Duration::from_secs(60);

    loop {
        let mut plan = opts.build_plan()?;
        if let Some(last) = &last_processed {
            plan.retain(|item| item > last);
        }
        let plan = if opts.tail && first_time && !plan.is_empty() {
            vec![plan.pop().expect("not empty")]
        } else {
            plan
        };

        first_time = false;

        for path in plan {
            eprintln!("{path}");
            tail_single_file(&path)?;
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
