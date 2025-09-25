use anyhow::Context;
use filenamegen::Glob;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use nix::unistd::{Uid, User};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::Semaphore;

fn find_target_dir() -> String {
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(td) => td,
        Err(_) => {
            let cwd = std::env::current_dir().expect("failed to getcwd");
            cwd.join("target")
                .to_str()
                .expect("cwd is not utf8")
                .to_string()
        }
    }
}

fn find_kumod() -> String {
    let target = find_target_dir();
    for mode in ["debug", "release"] {
        let candidate = format!("{target}/{mode}/kumod");
        if Path::new(&candidate).is_file() {
            return candidate;
        }
    }
    panic!("could not find kumod");
}

fn discover_module_tests() -> Vec<String> {
    let glob =
        Glob::new("assets/policy-extras/**/*.lua").expect("failed to compile module test glob");
    let mut module_tests = vec![];
    for path in glob.walk(".") {
        let content = std::fs::read_to_string(&path).expect("failed to read test content");
        if content.contains("mod:test") {
            module_tests.push(path.to_str().expect("path is not utf8").to_string());
        }
    }
    module_tests.sort();

    module_tests
}

fn discover_crate_tests() -> Vec<String> {
    let glob = Glob::new("crates/*/**/test*.lua").expect("failed to compile crate test glob");
    let mut crate_tests = vec![];
    for path in glob.walk(".") {
        crate_tests.push(path.to_str().expect("path is not utf8").to_string());
    }
    crate_tests.sort();

    crate_tests
}

#[derive(Debug)]
struct TestResult {
    name: String,
    output: String,
    ok: bool,
}

impl TestResult {
    async fn spawn_module_test(semaphore: Arc<Semaphore>, test_file: String) -> TestResult {
        let _permit = semaphore.acquire_owned().await;
        match Self::run_module_test(&test_file).await {
            Ok(result) => result,
            Err(err) => TestResult {
                name: test_file,
                ok: false,
                output: format!("{err:#}"),
            },
        }
    }

    async fn run_module_test(test_file: &str) -> anyhow::Result<TestResult> {
        let mut script_file = NamedTempFile::new().context("failed to make temp file")?;

        let (_, basename) = test_file.rsplit_once('/').expect("to have have basename");
        let (stem, _) = basename.split_once('.').expect("to have stem");
        let module_name = format!("policy-extras.{stem}");

        write!(
            script_file,
            r#"
local kumo = require 'kumo'
package.path = 'assets/?.lua;' .. package.path

kumo.on('main', function()
  local mod = require("{module_name}")
  mod:test()
end)
"#
        )?;

        Self::run_test_script(
            script_file
                .path()
                .to_str()
                .context("temp file is not utf8")?,
            test_file,
        )
        .await
    }

    async fn spawn_crate_test(semaphore: Arc<Semaphore>, test_file: String) -> TestResult {
        let _permit = semaphore.acquire_owned().await;
        match Self::run_crate_test(&test_file).await {
            Ok(result) => result,
            Err(err) => TestResult {
                name: test_file,
                ok: false,
                output: format!("{err:#}"),
            },
        }
    }

    async fn run_crate_test(test_file: &str) -> anyhow::Result<TestResult> {
        let mut script_file = NamedTempFile::new().context("failed to make temp file")?;

        write!(
            script_file,
            r#"
local kumo = require 'kumo'
package.path = 'assets/?.lua;' .. package.path

kumo.on('main', function()
  dofile "{test_file}"
end)
"#
        )?;

        Self::run_test_script(
            script_file
                .path()
                .to_str()
                .context("temp file is not utf8")?,
            test_file,
        )
        .await
    }

    async fn run_test_script(script: &str, test_file: &str) -> anyhow::Result<TestResult> {
        let me = User::from_uid(Uid::current())?
            .ok_or_else(|| anyhow::anyhow!("cannot find my own username"))?;

        let output = tokio::process::Command::new(find_kumod())
            .args(["--user", &me.name, "--policy", script, "--script"])
            .output()
            .await?;

        let combined = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr);

        Ok(TestResult {
            name: test_file.to_string(),
            output: combined,
            ok: output.status.success(),
        })
    }

    fn summarize(&self) {
        let prefix = format!("{} {}: ", self.name, if self.ok { "OK" } else { "ERR" });
        if !self.output.is_empty() {
            for line in self.output.lines() {
                eprintln!("{prefix}{line}");
            }
        } else {
            eprintln!("{}: {}", self.name, if self.ok { "OK" } else { "FAILED" });
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let concurrency = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tests = FuturesUnordered::new();

    for ct in discover_crate_tests() {
        tests.push(tokio::spawn(TestResult::spawn_crate_test(
            semaphore.clone(),
            ct,
        )));
    }

    for mt in discover_module_tests() {
        tests.push(tokio::spawn(TestResult::spawn_module_test(
            semaphore.clone(),
            mt,
        )));
    }

    let num_tests = tests.len();

    let mut failed = vec![];
    while let Some(result) = tests.next().await {
        match result {
            Ok(result) => {
                if !result.ok {
                    failed.push(result.name.clone());
                }
                result.summarize();
            }
            Err(err) => {
                let error = format!("{err:#}");
                eprintln!("{error}");
                failed.push(error);
            }
        }
    }

    if !failed.is_empty() {
        failed.sort();
        eprintln!("{} of {num_tests} lua test(s) failed:", failed.len());
        for f in &failed {
            eprintln!("  {f}");
        }
        anyhow::bail!("{} of {num_tests} lua test(s) failed", failed.len());
    } else {
        eprintln!("All {num_tests} lua tests OK");
    }

    Ok(())
}
