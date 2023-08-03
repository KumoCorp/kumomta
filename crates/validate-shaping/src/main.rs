use clap::Parser;
use kumo_api_types::shaping::Shaping;

/// KumoMTA shaping configuration validator
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about)]
struct Opt {
    files: Vec<String>,
}

#[tokio::main]
async fn main() {
    let opts = Opt::parse();
    let mut failed = false;

    match Shaping::merge_files(&opts.files).await {
        Ok(merged) => {
            for warn in merged.get_warnings() {
                eprintln!("{warn}");
                failed = true;
            }
            if !failed {
                eprintln!("OK");
            }
        }
        Err(err) => {
            eprintln!("{err:#}");
            failed = true;
        }
    }

    if failed {
        std::process::exit(1);
    }
}
