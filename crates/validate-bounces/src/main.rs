use anyhow::anyhow;
use bounce_classify::{BounceClass, BounceClassifierBuilder};
use clap::Parser;
use ordermap::OrderMap;
use serde::Deserialize;
use std::convert::TryFrom;
use rfc5321::parse_response_line;

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
#[serde(try_from="String")]
struct ResponseLineWrapper(String);

impl TryFrom<String> for ResponseLineWrapper {
    type Error = String;
    fn try_from(line: String) -> Result<Self, String> {
        let _resp = parse_response_line(&line).map_err(|e| format!("{e}"))?;
        Ok(Self(line))
    }
}

impl From<ResponseLineWrapper> for String {
    fn from(resp: ResponseLineWrapper) -> String { resp.0 }
}

#[derive(Deserialize, Debug)]
struct SampleFile {
    pub rules: OrderMap<BounceClass, Vec<ResponseLineWrapper>>,
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
                let input_str: String = input.into();
                let got = classifier.classify_str(&input_str);
                if got != class {
                    let expected = String::from(class.clone());
                    let got = String::from(got);
                    failures.push(format!("{input_str:?}: expected {expected} but got {got}"));
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
