use chrono::Utc;
use clap::Parser;
use human_bytes::human_bytes;
use spool::rocks::RocksSpool;
use spool::{Spool, SpoolEntry};
use std::path::PathBuf;
use tokio::runtime::Handle;

/// KumoMTA Spool Utility
///
/// This program is for analyzing and understanding the spool from
/// a node that is offline; it cannot be used while kumod is running
/// and has the spool open.
#[derive(Debug, Parser)]
struct Opt {
    #[arg(long)]
    meta: PathBuf,
    #[arg(long)]
    data: PathBuf,

    #[command(subcommand)]
    cmd: SubCommand,
}

#[derive(Debug, Parser)]
enum SubCommand {
    MetaSize,
    DataSize,
}

async fn show_size_stats(label: &str, spool: &dyn Spool) -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    let (tx, rx) = flume::bounded(1024);
    spool.enumerate(tx, Utc::now())?;
    let mut stats = incr_stats::incr::Stats::new();
    let mut hist = hdrhistogram::Histogram::<u64>::new(3)?;
    eprintln!("enumerating...");
    while let Ok(info) = rx.recv_async().await {
        match info {
            SpoolEntry::Item { id: _, data } => {
                stats.update(data.len() as f64)?;
                hist.record(data.len() as u64)?;
            }
            SpoolEntry::Corrupt { id, error } => {
                eprintln!("ERROR: entry {id} is corrupt: {error}");
            }
        }
    }

    println!("{label} size stats computed in {:?}", start.elapsed());
    println!("count = {}", stats.count());
    println!("min = {}", human_bytes(stats.min()?));
    println!("max = {}", human_bytes(stats.max()?));
    println!("mean = {}", human_bytes(stats.mean()?));
    println!("sum = {}", human_bytes(stats.sum()?));

    for p in [50., 75., 90.0, 95., 99.0, 99.9] {
        println!(
            "p{p:<4} {}",
            human_bytes(hist.value_at_quantile(p / 100.) as f64)
        );
    }
    for thresh in [
        1024,
        2048,
        4096,
        10 * 1024,
        32 * 1024,
        64 * 1024,
        128 * 1024,
        1024 * 1024,
        2 * 1024 * 1024,
    ] {
        let q = hist.quantile_below(thresh) * 100.;
        println!("{q:.3}% <= {}", human_bytes(thresh as f64));
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let meta_spool = RocksSpool::new(&opts.meta, false, None, Handle::current())?;
    let data_spool = RocksSpool::new(&opts.data, false, None, Handle::current())?;

    match opts.cmd {
        SubCommand::MetaSize => {
            show_size_stats("meta", &meta_spool).await?;
        }
        SubCommand::DataSize => {
            show_size_stats("data", &data_spool).await?;
        }
    }

    Ok(())
}
