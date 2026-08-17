//! A pre-flight check that a directory is genuinely usable for the
//! create-a-temp-file-then-rename-it-into-place pattern used by
//! databases like RocksDB and by maildir-style delivery.  See
//! [`probe_directory`].

use anyhow::Context;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::Builder;

/// Verify that the calling process can perform the file operations that
/// a RocksDB or maildir style store relies on within `dir`: create a
/// file, write and fsync it, atomically rename it into place, and
/// unlink it.
///
/// It also cross-checks the kernel's `access(2)` view of the renamed
/// file against the `open(2)` view.  That cross-check only bites when
/// the real and effective uids differ; when they are the same (tests,
/// non-root deploys, plain root) both syscalls consult one identity and
/// can never disagree, so it is inert.  It exists for the specific
/// failure mode of a privilege drop that keeps the real uid as root (to
/// retain a capability) while lowering the effective uid to a service
/// user:
///
/// - `access(2)` answers using the real uid (root here).
/// - `open(2)`, `rename(2)`, and `unlink(2)` act using the effective
///   (filesystem) uid (the service user here).
/// - On a directory whose group and other permission bits have been
///   stripped (for example a root-owned `mkdir` under a `077` umask,
///   later `chown`ed to the service user), those two identities
///   disagree about whether a file is reachable.
///
/// RocksDB decides whether to open an existing database or create a
/// fresh one via an `access(2)`-style existence check, then does its
/// I/O via `open`/`rename`/`unlink`.  When the two disagree it silently
/// creates a brand new database and aborts on the pre-existing
/// write-ahead log, which the operator sees only as an opaque
/// "wal_dir contains existing log file" failure on the second startup.
/// Running this probe first turns that late, cryptic corruption into an
/// early, actionable error.
///
/// The check runs as whatever real/effective/filesystem identity the
/// process currently has, so it needs no assumptions about which user
/// or which permission bits are "correct".  On failure the returned
/// error names the directory owner, mode, and the process ids so the
/// operator can see the mismatch.
pub fn probe_directory(dir: &Path) -> anyhow::Result<()> {
    // Create a file the way RocksDB creates its WAL and MANIFEST files.
    let source = Builder::new()
        .prefix(".kumo-dir-probe")
        .tempfile_in(dir)
        .with_context(|| {
            format!(
                "unable to create a file in {}. {}",
                dir.display(),
                describe_context(dir)
            )
        })?;

    source
        .as_file()
        .write_all(b"kumo dir probe")
        .and_then(|_| source.as_file().sync_all())
        .with_context(|| {
            format!(
                "unable to write and fsync a file in {}. {}",
                dir.display(),
                describe_context(dir)
            )
        })?;

    // Atomically install it under a new name, mirroring RocksDB's rename
    // of CURRENT.dbtmp onto CURRENT.  The target reuses the unique name
    // that tempfile chose for the source, so it can't collide either.
    let source_path = source.into_temp_path();
    let mut target = source_path.as_os_str().to_owned();
    target.push(".renamed");
    let target = PathBuf::from(target);
    std::fs::rename(&source_path, &target).with_context(|| {
        format!(
            "unable to rename a file within {}. {}",
            dir.display(),
            describe_context(dir)
        )
    })?;

    let consistency = consistency_check(&target).with_context(|| describe_context(dir));

    // Unlink the way RocksDB unlinks obsolete files.  This also cleans
    // up the probe artifact regardless of the outcome above.
    let unlink = std::fs::remove_file(&target).with_context(|| {
        format!(
            "unable to unlink a file within {}. {}",
            dir.display(),
            describe_context(dir)
        )
    });

    // Both cleanup and the verdict are bound before either `?` so the
    // probe file is always removed, even when the consistency check
    // fails.  Report the consistency verdict first because it is the
    // more informative diagnosis; a refactor that removes this eager
    // binding would reintroduce a leak on the error path.
    consistency?;
    unlink?;
    Ok(())
}

/// Compare the `access(2)` existence verdict (evaluated against the real
/// uid/gid) with the `open(2)` verdict (evaluated against the effective
/// filesystem identity).  They can only disagree when the two identities
/// differ and the directory permissions favor one over the other, which
/// is the condition that silently corrupts a RocksDB opened in this
/// directory.
#[cfg(unix)]
fn consistency_check(target: &Path) -> anyhow::Result<()> {
    use nix::unistd::{access, AccessFlags};

    // F_OK deliberately tests only existence plus parent-directory
    // search, matching how RocksDB's FileExists probes for CURRENT.  Do
    // not "upgrade" this to W_OK: writability is not the question, and
    // changing it would alter what divergence we detect.
    let access_present = access(target, AccessFlags::F_OK).is_ok();
    let open_present = std::fs::File::open(target).is_ok();

    anyhow::ensure!(
        access_present == open_present,
        "inconsistent view of {}: access(2) reports the file present={access_present} \
         but open(2) reports it present={open_present}. This means the process real and \
         effective user ids differ (a privilege drop) and the directory permissions are \
         too restrictive for one of those identities. A database opened here would decide \
         to create a fresh instance yet write over the existing files, corrupting itself \
         on the next startup. Ensure the directory is owned by, and grants rwx to, the \
         identity the service runs as.",
        target.display(),
    );

    Ok(())
}

#[cfg(not(unix))]
fn consistency_check(_target: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn describe_context(dir: &Path) -> String {
    use nix::unistd::{getegid, geteuid, getgid, getuid};
    use std::os::unix::fs::MetadataExt;

    let dir_info = match std::fs::metadata(dir) {
        Ok(md) => format!(
            "directory owner uid={} gid={} mode={:o}",
            md.uid(),
            md.gid(),
            md.mode() & 0o7777
        ),
        Err(err) => format!("directory metadata unavailable: {err}"),
    };

    format!(
        "{} ({dir_info}); process ruid={} euid={} rgid={} egid={}",
        dir.display(),
        getuid(),
        geteuid(),
        getgid(),
        getegid(),
    )
}

#[cfg(not(unix))]
fn describe_context(dir: &Path) -> String {
    dir.display().to_string()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn probe_succeeds_on_writable_dir() {
        let dir = tempfile::tempdir().unwrap();
        probe_directory(dir.path()).unwrap();
        // The probe must leave nothing behind.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        k9::assert_equal!(leftovers.len(), 0, "probe left files behind: {leftovers:?}");
    }

    #[test]
    fn probe_fails_on_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        probe_directory(&missing).unwrap_err();
    }

    #[cfg(unix)]
    #[test]
    fn probe_fails_on_unwritable_dir() {
        use nix::unistd::geteuid;
        use std::os::unix::fs::PermissionsExt;

        // root bypasses the permission bits via CAP_DAC_OVERRIDE, so the
        // unwritable case can only be exercised as a non-root user.
        if geteuid().is_root() {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("locked");
        std::fs::create_dir(&sub).unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o500)).unwrap();

        let err = probe_directory(&sub).unwrap_err();

        // Restore write so the tempdir can be cleaned up.
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o700)).unwrap();

        let msg = format!("{err:#}");
        assert!(
            msg.contains("unable to create a file"),
            "unexpected error: {msg}"
        );
    }
}
