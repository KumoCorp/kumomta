use anyhow::anyhow;
use bounce_classify::{
    BounceClass, BounceClassifier, BounceClassifierBuilder, BounceClassifierFile,
};
use clap::Parser;
use ordermap::OrderMap;
use regex::Regex;
use rfc5321::parse_response_line;
use serde::Deserialize;

/// KumoMTA bounce classification configuration validator
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about)]
struct Opt {
    files: Vec<String>,

    #[arg(long, num_args = 1..)]
    samples: Vec<String>,
}

#[derive(Deserialize, Debug)]
struct SampleFile {
    pub rules: OrderMap<BounceClass, Vec<String>>,
}

fn build_classifier(rules: &[(String, BounceClass, String)]) -> anyhow::Result<BounceClassifier> {
    let mut builder = BounceClassifierBuilder::new();
    for (_file, class, pattern) in rules {
        builder.add_rule(class.clone(), pattern.clone());
    }
    builder.build().map_err(|err| anyhow!("{err}"))
}

/// Loads the individual (class, sample input) pairs out of
/// `samples_files`. This is the single read of each samples file;
/// `check_samples` works off the result rather than doing its own I/O.
fn load_samples(samples_files: &[String]) -> anyhow::Result<Vec<(BounceClass, String)>> {
    let mut samples = vec![];
    for samples_file in samples_files {
        let data = std::fs::read_to_string(samples_file)
            .map_err(|err| anyhow!("reading file: {samples_file}: {err:#}"))?;
        let decoded: SampleFile = toml::from_str(&data)
            .map_err(|err| anyhow!("decoding {samples_file} as SampleFile: {err:#}"))?;

        for (class, inputs) in decoded.rules {
            for input in inputs {
                samples.push((class.clone(), input));
            }
        }
    }
    Ok(samples)
}

/// Classifies every sample against `classifier`, returning the list of
/// mismatches/invalid samples.
fn check_samples(classifier: &BounceClassifier, samples: &[(BounceClass, String)]) -> Vec<String> {
    let mut failures = vec![];
    for (class, input_str) in samples {
        let expected = String::from(class.clone());
        match parse_response_line(input_str) {
            Ok(_) => {
                let got = classifier.classify_str(input_str);
                if got != *class {
                    let got = String::from(got);
                    failures.push(format!("{input_str:?}: expected {expected} but got {got}"));
                }
            }
            Err(e) => {
                failures.push(format!(
                    "Sample {expected} bounce message is not a valid SMTP response: {e}"
                ));
            }
        }
    }
    failures
}

/// Loads the individual (file, class, pattern) rules out of `files`.
/// This is the single read of each rule file; `build_classifier` and
/// `check_coverage` both work off the result rather than re-parsing.
/// The file is kept alongside each rule so `check_coverage` can group
/// its report by where a rule came from.
fn load_rules(files: &[String]) -> anyhow::Result<Vec<(String, BounceClass, String)>> {
    let mut rules = vec![];
    for file_name in files {
        let decoded: BounceClassifierFile = if file_name.ends_with(".json") {
            let f = std::fs::File::open(file_name)
                .map_err(|err| anyhow!("reading file: {file_name}: {err:#}"))?;
            serde_json::from_reader(f)
                .map_err(|err| anyhow!("decoding {file_name} as BounceClassifierFile: {err:#}"))?
        } else if file_name.ends_with(".toml") {
            let data = std::fs::read_to_string(file_name)
                .map_err(|err| anyhow!("reading file: {file_name}: {err:#}"))?;
            toml::from_str(&data)
                .map_err(|err| anyhow!("decoding {file_name} as BounceClassifierFile: {err:#}"))?
        } else {
            anyhow::bail!(
                "{file_name}: classifier files must have either .toml or .json filename extension"
            );
        };
        for (class, patterns) in decoded.rules {
            for pattern in patterns {
                rules.push((file_name.clone(), class.clone(), pattern));
            }
        }
    }
    Ok(rules)
}

/// Returns each (file, class, pattern) rule with no matching sample.
fn check_coverage(
    rules: &[(String, BounceClass, String)],
    samples: &[(BounceClass, String)],
) -> anyhow::Result<Vec<(String, BounceClass, String)>> {
    let mut uncovered = vec![];
    for (file, class, pattern) in rules {
        let re = Regex::new(pattern)
            .map_err(|err| anyhow!("rule {pattern:?} failed to compile: {err:#}"))?;
        if !samples.iter().any(|(_, input)| re.is_match(input)) {
            uncovered.push((file.clone(), class.clone(), pattern.clone()));
        }
    }
    Ok(uncovered)
}

/// Groups uncovered rules by file, then by class, so a report with
/// many gaps in the same file/class reads as one block instead of a
/// repeated wall of near-identical lines.
fn format_uncovered(uncovered: &[(String, BounceClass, String)]) -> String {
    let mut by_file: OrderMap<&str, OrderMap<String, Vec<&str>>> = OrderMap::new();
    for (file, class, pattern) in uncovered {
        by_file
            .entry(file)
            .or_default()
            .entry(String::from(class.clone()))
            .or_default()
            .push(pattern);
    }

    let mut out = String::new();
    for (file, by_class) in &by_file {
        out.push_str(&format!("{file}:\n"));
        for (class, patterns) in by_class {
            out.push_str(&format!("  {class}:\n"));
            for pattern in patterns {
                out.push_str(&format!("    - {pattern:?}\n"));
            }
        }
    }
    out.pop(); // trailing newline
    out
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();

    let rules = load_rules(&opts.files)?;
    let classifier = build_classifier(&rules)?;
    let samples = load_samples(&opts.samples)?;
    let failures = check_samples(&classifier, &samples);
    let uncovered = check_coverage(&rules, &samples)?;

    if !failures.is_empty() {
        anyhow::bail!(
            "{} test case(s) failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    if !uncovered.is_empty() {
        eprintln!(
            "warning: {} bounce rule(s) have no sample coverage; add a sample input for each to one of the files listed behind the --samples flag:\n{}",
            uncovered.len(),
            format_uncovered(&uncovered)
        );
    }

    println!("OK");

    Ok(())
}
