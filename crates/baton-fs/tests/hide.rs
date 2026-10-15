// SPDX-License-Identifier: Apache-2.0

//! Hiding a file or a folder, on whatever system the tests run on.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::{fs, io::ErrorKind};

use baton_fs::hide;

#[test]
fn a_folder_is_hidden_and_stays_a_folder_that_can_be_used() {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join(".baton-incoming");
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join("part"), "before").unwrap();
    let before = fs::metadata(&staging).unwrap();

    hide(&staging).unwrap();

    let after = fs::metadata(&staging).unwrap();
    assert!(after.is_dir());
    assert_eq!(after.permissions(), before.permissions());
    assert_eq!(fs::read_to_string(staging.join("part")).unwrap(), "before");
    fs::write(staging.join("later"), "after").unwrap();
    assert_eq!(fs::read_dir(&staging).unwrap().count(), 2);
}

#[test]
fn a_file_is_hidden_and_its_bytes_are_not_touched() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".part");
    fs::write(&file, "bytes").unwrap();

    hide(&file).unwrap();

    assert_eq!(fs::read_to_string(&file).unwrap(), "bytes");
}

#[test]
fn hiding_what_is_hidden_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join(".baton-incoming");
    fs::create_dir(&staging).unwrap();

    hide(&staging).unwrap();
    hide(&staging).unwrap();
}

#[test]
fn something_that_is_not_there_is_an_error() {
    let dir = tempfile::tempdir().unwrap();

    let error = hide(dir.path().join(".missing")).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_path_with_a_nul_character_is_refused() {
    let error = hide("a\0b").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
}

#[cfg(windows)]
mod windows {
    use std::{fs, os::windows::fs::MetadataExt};

    use baton_fs::hide;

    /// The attributes that these tests look at.
    const HIDDEN: u32 = 0x2;
    const DIRECTORY: u32 = 0x10;

    fn attributes(path: &std::path::Path) -> u32 {
        fs::metadata(path).unwrap().file_attributes()
    }

    #[test]
    fn a_folder_gets_the_hidden_attribute_and_keeps_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(".baton-incoming");
        fs::create_dir(&staging).unwrap();
        assert_eq!(attributes(&staging) & HIDDEN, 0);

        hide(&staging).unwrap();

        let after = attributes(&staging);
        assert_ne!(after & HIDDEN, 0, "hidden: {after:#x}");
        assert_ne!(after & DIRECTORY, 0, "still a folder: {after:#x}");
    }

    #[test]
    fn a_file_gets_the_hidden_attribute() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("part");
        fs::write(&file, "bytes").unwrap();

        hide(&file).unwrap();

        assert_ne!(attributes(&file) & HIDDEN, 0);
        assert_eq!(fs::read_to_string(&file).unwrap(), "bytes");
    }
}
