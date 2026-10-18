// SPDX-License-Identifier: Apache-2.0

//! Marking a file as one that came from elsewhere, on whatever system the tests run on.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::{fs, io::ErrorKind, path::PathBuf};

use baton_fs::mark_received;

/// A folder with one file in it, as a received file is.
fn received() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("photo.jpg");
    fs::write(&file, "the bytes").unwrap();
    (dir, file)
}

#[test]
fn a_file_is_marked_and_its_bytes_are_not_touched() {
    let (dir, file) = received();

    mark_received(&file).unwrap();

    assert_eq!(fs::read_to_string(&file).unwrap(), "the bytes");
    // The mark is not a file of its own.
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_file_can_be_marked_again() {
    let (_dir, file) = received();

    mark_received(&file).unwrap();
    mark_received(&file).unwrap();

    assert_eq!(fs::read_to_string(&file).unwrap(), "the bytes");
}

#[test]
fn nothing_is_made_for_a_file_that_is_not_there() {
    let dir = tempfile::tempdir().unwrap();

    let error = mark_received(dir.path().join("missing")).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::NotFound);
    // On Windows, writing the stream of a name that is not there makes the file.
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_path_with_a_nul_character_is_refused() {
    let error = mark_received("a\0b").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
}

#[cfg(target_os = "macos")]
#[test]
fn the_mac_gets_the_quarantine_attribute_of_a_download() {
    use std::{
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    let (_dir, file) = received();
    mark_received(&file).unwrap();

    let shown = Command::new("xattr")
        .args(["-p", "com.apple.quarantine"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(shown.status.success(), "{shown:?}");
    let value = String::from_utf8(shown.stdout).unwrap();
    let fields: Vec<&str> = value.trim_end().split(';').collect();

    // Flags, time in hexadecimal, the name of the program, and the id of an entry in the record
    // of downloads, which there is none of.
    assert_eq!(fields.len(), 4, "{value}");
    let flags = u32::from_str_radix(fields[0], 16).unwrap();
    assert_ne!(flags & 0x1, 0, "a download: {value}");
    assert_eq!(flags & 0x40, 0, "not approved by the person: {value}");
    let time = u64::from_str_radix(fields[1], 16).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(now.abs_diff(time) < 120, "{value}");
    assert_eq!(fields[2], "Baton");
}

#[cfg(target_os = "macos")]
#[test]
fn a_mark_that_is_there_already_stays_as_it_is() {
    use std::process::Command;

    let (_dir, file) = received();
    // The kind of value Safari writes for a download, which is not the one this crate writes.
    let theirs = "0081;5f0e4b2a;Safari;5E3B2C1D-0000-4000-8000-000000000000";
    let set = Command::new("xattr")
        .args(["-w", "com.apple.quarantine", theirs])
        .arg(&file)
        .output()
        .unwrap();
    assert!(set.status.success(), "{set:?}");

    mark_received(&file).unwrap();

    let shown = Command::new("xattr")
        .args(["-p", "com.apple.quarantine"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(shown.status.success(), "{shown:?}");
    assert_eq!(String::from_utf8(shown.stdout).unwrap().trim_end(), theirs);
}

#[cfg(windows)]
#[test]
fn windows_gets_the_zone_stream_of_a_file_from_the_internet() {
    let (_dir, file) = received();
    mark_received(&file).unwrap();

    let mut stream = file.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    assert_eq!(
        fs::read_to_string(stream).unwrap(),
        "[ZoneTransfer]\r\nZoneId=3\r\n"
    );
}
