use anyhow::anyhow;
use bounce_classify::{BounceClass, BounceClassifierBuilder};
use clap::Parser;
use ordermap::OrderMap;
use rfc5321::parse_response_line;
use serde::Deserialize;

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

#[derive(Deserialize, Debug)]
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
            .map_err(|err| anyhow!("decoding {samples_file} as SampleFile: {err:#}"))?;

        for (class, inputs) in samples.rules {
            let expected = String::from(class.clone());
            for input in inputs {
                let input_str: String = input.into();

                match parse_response_line(&input_str) {
                    Ok(_) => {
                        let got = classifier.classify_str(&input_str);
                        if got != class {
                            let got = String::from(got);
                            failures
                                .push(format!("{input_str:?}: expected {expected} but got {got}"));
                        }
                    }
                    Err(e) => {
                        failures.push(format!(
                            "Sample {expected} bounce message is not a valid SMTP response: {e}"
                        ));
                    }
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
