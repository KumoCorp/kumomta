//! Manual reproduction tool for the split real/effective uid hazard that
//! [`dir_probe::probe_directory`] detects.
//!
//! Creating a real/effective uid split requires privilege, so the unit
//! tests cannot exercise the headline cross-check.  This example fills
//! that gap: run as root, it mirrors kumod's privilege drop (setgid and
//! seteuid to a target user, then trimming capabilities so
//! CAP_DAC_OVERRIDE is gone) and runs the probe against a directory, so
//! an operator can confirm the detection end to end.
//!
//! Usage:
//!   sudo cargo run -p dir-probe --example probe -- <dir> [--user <name>]
//!
//! To reproduce the RocksDB corruption scenario:
//!   sudo install -d -o <name> -g <name> -m 2700 /tmp/probe-demo
//!   sudo cargo run -p dir-probe --example probe -- /tmp/probe-demo --user <name>
//!
//! With mode 2700 the probe should FAIL (real-root cannot search the
//! directory, so access(2) and open(2) disagree); after `chmod 2755
//! /tmp/probe-demo` it should PASS.

use std::path::{Path, PathBuf};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut dir: Option<PathBuf> = None;
    let mut user: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--user" => user = args.next(),
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let dir = dir.ok_or_else(|| anyhow::anyhow!("usage: probe <dir> [--user <name>]"))?;

    if let Some(user_name) = &user {
        drop_privs_like_kumod(user_name)?;
    }

    report_identity(&dir);

    match dir_probe::probe_directory(&dir) {
        Ok(()) => {
            println!("PASS: {} is usable by this identity", dir.display());
            Ok(())
        }
        Err(err) => {
            println!("FAIL: {err:#}");
            std::process::exit(1);
        }
    }
}

/// Reproduce kumod's `drop_privs`: lower the effective (not real) uid to
/// the target user and trim capabilities down to CAP_NET_BIND_SERVICE.
/// Dropping CAP_DAC_OVERRIDE is the crucial part; it is what subjects the
/// leftover real-root identity to ordinary permission checks, which is
/// what makes access(2) and open(2) able to disagree.
fn drop_privs_like_kumod(user_name: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    use nix::unistd::User;

    let user =
        User::from_name(user_name)?.ok_or_else(|| anyhow::anyhow!("unknown user {user_name}"))?;

    nix::unistd::setgid(user.gid).context("setgid")?;
    nix::unistd::seteuid(user.uid).context("seteuid")?;

    #[cfg(target_os = "linux")]
    {
        use caps::{CapSet, Capability, CapsHashSet};
        let mut target = CapsHashSet::new();
        target.insert(Capability::CAP_NET_BIND_SERVICE);
        caps::set(None, CapSet::Effective, &target).context("setting effective caps")?;
        caps::set(None, CapSet::Permitted, &target).context("setting permitted caps")?;
    }

    Ok(())
}

fn report_identity(dir: &Path) {
    use nix::unistd::{getegid, geteuid, getgid, getuid};
    println!(
        "probing {} as ruid={} euid={} rgid={} egid={}",
        dir.display(),
        getuid(),
        geteuid(),
        getgid(),
        getegid(),
    );
}
