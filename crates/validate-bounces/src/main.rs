use anyhow::anyhow;
use bounce_classify::BounceClassifierBuilder;
use clap::Parser;

/// KumoMTA bounce classification configuration validator
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about)]
struct Opt {
    files: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let mut builder = BounceClassifierBuilder::new();
    for file_name in &opts.files {
        if file_name.ends_with(".json") {
            builder
                .merge_json_file(file_name)
                .map_err(|err| anyhow!("{file_name}: {err}"))?;
        } else if file_name.ends_with(".toml") {
            builder
                .merge_toml_file(file_name)
                .map_err(|err| anyhow!("{err}"))?;
        } else {
            anyhow::bail!(
                "{file_name}: classifier files must have either .toml or .json filename extension"
            );
        }
    }

    let _classifier = builder.build().map_err(|err| anyhow!("{err}"))?;

    println!("OK");

    Ok(())
}
