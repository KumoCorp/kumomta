use anyhow::Context;
use camino::Utf8PathBuf;
use clap::Parser;
use futures::StreamExt;
use kumo_log_tailer::LogTailerConfig;
use std::time::Duration;

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

    /// Maximum time to wait for a partial batch to fill before
    /// yielding it.  Accepts human-readable durations like "1s",
    /// "500ms", "2m".
    #[arg(long, default_value = "1s", value_parser = humantime::parse_duration)]
    batch_latency: Duration,

    /// Ignore the checkpoint, just tail the logs, starting
    /// with the most recent segment
    #[arg(long)]
    tail: bool,

    /// The directory which contains the logs
    directory: Utf8PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let mut config = LogTailerConfig::new(opts.directory.clone())
        .pattern(&opts.pattern)
        .max_batch_size(opts.batch_size)
        .max_batch_latency(opts.batch_latency)
        .tail(opts.tail);

    // When not tailing, enable checkpoint persistence
    if !opts.tail {
        config = config.checkpoint_name(&opts.checkpoint);
    }

    let tailer = config.build().await.context("building log tailer")?;

    tokio::pin!(tailer);

    while let Some(result) = tailer.next().await {
        let batch = result?;
        for line in &batch {
            println!("{line}");
        }
    }

    Ok(())
}
