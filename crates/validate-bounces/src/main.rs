use anyhow::anyhow;
use bounce_classify::{BounceClassifierBuilder, BounceClass};
use ordermap::OrderMap;
use serde::{Deserialize, Serialize};
use clap::Parser;

/// KumoMTA bounce classification configuration validator
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about)]
struct Opt {
    files: Vec<String>,

    #[arg(long)]
    samples: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug)]
struct SampleFile {
    pub rules: OrderMap<BounceClass, Vec<String>>,
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

    let classifier = builder.build().map_err(|err| anyhow!("{err}"))?;

    let mut failures = vec![];
    for samples_file in &opts.samples {
        let data = std::fs::read_to_string(samples_file)
            .map_err(|err| anyhow!("reading file: {samples_file}: {err:#}"))?;
        let samples: SampleFile = toml::from_str(&data)
            .map_err(|err| anyhow!("decoding {samples_file} as BounceClassifierFile: {err:#}"))?;

        for (class, inputs) in samples.rules {
            for input in inputs {
                let got = classifier.classify_str(&input);
                if got != class {
                    let expected = String::from(class.clone());
                    let got = String::from(got);
                    failures.push(format!("{input:?}: expected {expected} but got {got}"));
                }
            }
        }
    }
    if !failures.is_empty() {
        anyhow::bail!(
            "{} test case(s) failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    println!("OK");

    Ok(())
}
