use maildir::*;

#[cfg(unix)]
use std::borrow::Cow;
#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use percent_encoding::percent_decode;
use tempfile::tempdir;
use walkdir::WalkDir;

static TESTDATA_DIR: &str = "testdata";
static MAILDIR_NAME: &str = "maildir";
static SUBMAILDIRS_NAME: &str = "submaildirs";

// `cargo package` doesn't package files with certain characters, such as
// colons, in the name, so we percent-decode the file names when copying the
// data for the tests.
// This code can likely be improved (for correctness, particularly on Windows)
// but there's no good docs on what `cargo package` does percent-encoding for,
// and how it deals with multibyte characters in filenames. In practice this
// code works fine for this crate, because we have a restricted set of ASCII
// characters in the filenames.
fn with_maildir<F>(name: &str, func: F)
where
    F: FnOnce(Maildir),
{
    let tmp_dir = tempdir().expect("could not create temporary directory");
    let tmp_path = tmp_dir.path();
    for entry in WalkDir::new(TESTDATA_DIR) {
        let entry = entry.expect("directory walk error");
        let relative = entry.path().strip_prefix(TESTDATA_DIR).unwrap();
        if relative.parent().is_none() {
            continue;
        }

        #[cfg(unix)]
        let decoded_bytes: Cow<[u8]> = percent_decode(relative.as_os_str().as_bytes()).into();
        #[cfg(unix)]
        let decoded = OsStr::from_bytes(&decoded_bytes);

        #[cfg(windows)]
        let decoded_bytes = relative
            .as_os_str()
            .encode_wide()
            .map(|b| b as u8)
            .collect::<Vec<_>>();
        #[cfg(windows)]
        let decoded_bytes = percent_decode(decoded_bytes.as_slice())
            .map(|b| (if b == b':' { b';' } else { b }) as u16)
            .collect::<Vec<_>>();
        #[cfg(windows)]
        let decoded = OsString::from_wide(decoded_bytes.as_slice());

        if entry.path().is_dir() {
            fs::create_dir(tmp_path.join(&decoded)).expect("could not create directory");
        } else {
            fs::copy(entry.path(), tmp_path.join(decoded)).expect("could not copy test data");
        }
    }
    func(Maildir::from(tmp_path.join(name)));
}

fn with_maildir_empty<F>(name: &str, func: F)
where
    F: FnOnce(Maildir),
{
    let tmp_dir = tempdir().expect("could not create temporary directory");
    let tmp_path = tmp_dir.path();
    func(Maildir::from(tmp_path.join(name)));
}

#[test]
fn maildir_count() {
    with_maildir(MAILDIR_NAME, |maildir| {
        assert_eq!(maildir.count_cur(), 1);
        assert_eq!(maildir.count_new(), 1);
    });
}

#[test]
fn maildir_list() {
    with_maildir(MAILDIR_NAME, |maildir| {
        let mut iter = maildir.list_new();
        let mut first = iter.next().unwrap().unwrap();
        assert_eq!(first.id(), "1463941010.5f7fa6dd4922c183dc457d033deee9d7");
        assert_eq!(first.headers().unwrap().subject().unwrap().unwrap(), "test",);
        assert_eq!(first.is_seen(), false);
        let second = iter.next();
        assert!(second.is_none());

        let mut iter = maildir.list_cur();
        let mut first = iter.next().unwrap().unwrap();
        assert_eq!(first.id(), "1463868505.38518452d49213cb409aa1db32f53184");
        assert_eq!(
            first
                .parsed()
                .unwrap()
                .headers()
                .subject()
                .unwrap()
                .unwrap(),
            "test"
        );
        assert_eq!(first.is_seen(), true);
        let second = iter.next();
        assert!(second.is_none());
    })
}

#[test]
fn maildir_list_subdirs() {
    with_maildir(SUBMAILDIRS_NAME, |maildir| {
        let subdirs: Vec<_> = maildir
            .list_subdirs()
            .map(|dir| {
                dir.unwrap()
                    .path()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        assert_eq!(2, subdirs.len());
        assert!(subdirs.contains(&".Subdir1".into()));
        assert!(subdirs.contains(&".Subdir2".into()));
        assert!(!subdirs.contains(&"..Subdir3".into()));
    });
}

#[test]
fn maildir_find() {
    with_maildir(MAILDIR_NAME, |maildir| {
        assert_eq!(
            maildir
                .find("1463941010.5f7fa6dd4922c183dc457d033deee9d7")
                .is_some(),
            true
        );
        assert_eq!(
            maildir
                .find("1463868505.38518452d49213cb409aa1db32f53184")
                .is_some(),
            true
        );
    })
}

#[test]
fn check_delete() {
    with_maildir(MAILDIR_NAME, |maildir| {
        assert_eq!(
            maildir
                .find("1463941010.5f7fa6dd4922c183dc457d033deee9d7")
                .is_some(),
            true
        );
        assert_eq!(
            maildir
                .delete("1463941010.5f7fa6dd4922c183dc457d033deee9d7")
                .is_ok(),
            true
        );
        assert_eq!(
            maildir
                .find("1463941010.5f7fa6dd4922c183dc457d033deee9d7")
                .is_some(),
            false
        );
    })
}

#[test]
fn check_copy_and_move() {
    with_maildir(MAILDIR_NAME, |maildir| {
        with_maildir(SUBMAILDIRS_NAME, |submaildir| {
            let id = "1463868505.38518452d49213cb409aa1db32f53184";

            // check that we cannot copy a message from and to the same maildir
            assert_eq!(
                maildir.copy_to(id, &maildir).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput,
            );

            // check that the message is present in "maildir" but not in "submaildir"
            assert!(maildir.find(id).is_some());
            assert!(submaildir.find(id).is_none());
            // also check that the failed self-copy a few lines up didn't corrupt the
            // message file.
            assert!(maildir.find(id).unwrap().date().is_ok());

            // copy the message from "maildir" to "submaildir"
            maildir.copy_to(id, &submaildir).unwrap();

            // check that the message is now present in both
            assert!(maildir.find(id).is_some());
            assert!(submaildir.find(id).is_some());

            // move the message from "submaildir" to "maildir"
            submaildir.move_to(id, &maildir).unwrap();

            // check that the message is now only present in "maildir"
            assert!(maildir.find(id).is_some());
            assert!(submaildir.find(id).is_none());
        })
    })
}

#[test]
fn mark_read() {
    with_maildir(MAILDIR_NAME, |maildir| {
        assert_eq!(
            maildir
                .move_new_to_cur("1463941010.5f7fa6dd4922c183dc457d033deee9d7")
                .unwrap(),
            ()
        );
    });
}

#[test]
fn check_received() {
    with_maildir(MAILDIR_NAME, |maildir| {
        let mut iter = maildir.list_cur();
        let mut first = iter.next().unwrap().unwrap();
        assert_eq!(first.received().unwrap().timestamp(), 1_463_868_507);
    });
}

#[test]
fn check_create_dirs() {
    with_maildir_empty("maildir2", |maildir| {
        assert!(!maildir.path().exists());
        for name in &["cur", "new", "tmp"] {
            assert!(!maildir.path().join(name).exists());
        }

        maildir.create_dirs().unwrap();
        assert!(maildir.path().exists());
        for name in &["cur", "new", "tmp"] {
            assert!(maildir.path().join(name).exists());
        }
    });
}

const TEST_MAIL_BODY: &[u8] = b"Return-Path: <of82ecuq@cip.cs.fau.de>
X-Original-To: of82ecuq@cip.cs.fau.de
Delivered-To: of82ecuq@cip.cs.fau.de
Received: from faui0fl.informatik.uni-erlangen.de (unknown [IPv6:2001:638:a000:4160:131:188:60:117])
        by faui03.informatik.uni-erlangen.de (Postfix) with ESMTP id 466C1240A3D
        for <of82ecuq@cip.cs.fau.de>; Fri, 12 May 2017 10:09:45 +0000 (UTC)
Received: by faui0fl.informatik.uni-erlangen.de (Postfix, from userid 303135)
        id 389CC10E1A32; Fri, 12 May 2017 12:09:45 +0200 (CEST)
To: of82ecuq@cip.cs.fau.de
MIME-Version: 1.0
Content-Type: text/plain; charset=\"UTF-8\"
Content-Transfer-Encoding: 8bit
Message-Id: <20170512100945.389CC10E1A32@faui0fl.informatik.uni-erlangen.de>
Date: Fri, 12 May 2017 12:09:45 +0200 (CEST)
From: of82ecuq@cip.cs.fau.de (Johannes Schilling)
Subject: maildir delivery test mail

Today is Boomtime, the 59th day of Discord in the YOLD 3183";

#[test]
fn check_store_new() {
    with_maildir_empty("maildir2", |maildir| {
        maildir.create_dirs().unwrap();

        assert_eq!(maildir.count_new(), 0);
        let id = maildir.store_new(TEST_MAIL_BODY);
        assert!(id.is_ok());
        assert_eq!(maildir.count_new(), 1);

        let id = id.unwrap();
        let msg = maildir.find(&id);
        assert!(msg.is_some());

        assert_eq!(
            msg.unwrap().parsed().unwrap().raw_body(),
            "Today is Boomtime, the 59th day of Discord in the YOLD 3183"
        );
    });
}

#[test]
fn check_store_cur() {
    with_maildir_empty("maildir2", |maildir| {
        maildir.create_dirs().unwrap();
        let testflags = "FRS";

        assert_eq!(maildir.count_cur(), 0);
        maildir
            .store_cur_with_flags(TEST_MAIL_BODY, testflags)
            .unwrap();
        assert_eq!(maildir.count_cur(), 1);

        let mut iter = maildir.list_cur();
        let first = iter.next().unwrap().unwrap();
        assert_eq!(first.flags(), testflags);
    });
}

#[test]
fn check_flag_fiddling() {
    with_maildir_empty("maildir2", |maildir| {
        maildir.create_dirs().unwrap();
        let id = maildir.store_cur_with_flags(TEST_MAIL_BODY, "SR").unwrap();

        assert_eq!(maildir.count_cur(), 1);
        assert_eq!(maildir.find(&id).unwrap().flags(), "RS");
        maildir.remove_flags(&id, "FS").unwrap();
        assert_eq!(maildir.find(&id).unwrap().flags(), "R");
        maildir.add_flags(&id, "RF").unwrap();
        assert_eq!(maildir.find(&id).unwrap().flags(), "FR");
        maildir.set_flags(&id, "SF").unwrap();
        assert_eq!(maildir.find(&id).unwrap().flags(), "FS");
    });
}
