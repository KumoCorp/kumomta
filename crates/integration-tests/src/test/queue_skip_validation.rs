use crate::kumod::target_bin;
use anyhow::Context;
use nix::unistd::{Uid, User};
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn skip_queue_config_hook_skips_queue_ordering_validation() -> anyhow::Result<()> {
    let temp = tempdir()?;

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let assets_path = workspace
        .join("assets")
        .join("?.lua")
        .to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "\\'");

    let policy_path = temp.path().join("policy.lua");
    std::fs::write(
        &policy_path,
        format!(
            r#"
local kumo = require 'kumo'
package.path = '{assets_path};' .. package.path

require 'policy-extras.sources'
local queue_module = require 'policy-extras.queue'

kumo.on('get_queue_config', function()
end)

queue_module:setup_with_options {{
  skip_queue_config_hook = true,
  file_names = {{
    {{
      queues = {{
        default = {{
          retry_interval = '1 minute',
        }},
      }},
    }},
  }},
}}

kumo.on('get_queue_config', function()
  return kumo.make_queue_config {{
    retry_interval = '1 minute',
  }}
end)
"#
        ),
    )?;

    let user = User::from_uid(Uid::current())?
        .context("determine current uid")?
        .name;

    let output = Command::new(target_bin("kumod")?)
        .args([
            "--policy",
            policy_path.to_str().context("policy path is utf8")?,
            "--user",
            &user,
            "--validate",
        ])
        .output()
        .context("run kumod --validate")?;

    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);

    anyhow::ensure!(output.status.success(), "validate failed:\n{combined}");
    assert!(
        !combined.contains(
            "queue.lua is in use, but it is not the last module to register for the \
             get_queue_config event."
        ),
        "{combined}"
    );

    Ok(())
}
