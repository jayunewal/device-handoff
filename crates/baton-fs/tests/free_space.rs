// SPDX-License-Identifier: Apache-2.0

//! Asking how much room a volume has, on whatever system the tests run on.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::{fs, io::ErrorKind};

use baton_fs::free_space;

#[test]
fn a_folder_has_room() {
    let dir = tempfile::tempdir().unwrap();

    let room = free_space(dir.path()).unwrap();

    assert!(room > 0);
}

#[test]
fn the_answer_is_in_bytes() {
    let dir = tempfile::tempdir().unwrap();

    let room = free_space(dir.path()).unwrap();

    // The system counts in blocks, and the answer is the number of blocks times the size of one,
    // which is a power of two of at least 512 bytes. A count of blocks that was not multiplied
    // would be any number at all, so this fails for it all but once in 512 times, and the other
    // way round, a size in bytes counted as blocks would be far more than any disk holds.
    assert_eq!(room % 512, 0, "{room}");
    assert!(room < 1 << 60, "{room}");
}

#[test]
fn a_folder_inside_a_folder_is_asked_about_as_well() {
    let dir = tempfile::tempdir().unwrap();
    let inner = dir.path().join("a").join("b");
    fs::create_dir_all(&inner).unwrap();

    assert!(free_space(&inner).unwrap() > 0);
}

#[test]
fn a_folder_that_is_not_there_is_an_error() {
    let dir = tempfile::tempdir().unwrap();

    let error = free_space(dir.path().join("missing")).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn a_path_with_a_nul_character_is_refused() {
    let error = free_space("a\0b").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
}
