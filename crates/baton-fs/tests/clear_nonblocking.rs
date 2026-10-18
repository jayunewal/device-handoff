// SPDX-License-Identifier: Apache-2.0

//! Making an open file wait for its data, on whatever system the tests run on.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::{
    fs::{self, File},
    io::Read,
};

use baton_fs::clear_nonblocking;

#[test]
fn a_plain_file_is_read_in_full_after_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photo.jpg");
    fs::write(&path, "the bytes").unwrap();
    let mut file = File::open(&path).unwrap();

    clear_nonblocking(&file).unwrap();
    // A second call, and a call on a copy of the descriptor, are as harmless as the first.
    clear_nonblocking(&file).unwrap();
    clear_nonblocking(&file.try_clone().unwrap()).unwrap();

    let mut read = String::new();
    file.read_to_string(&mut read).unwrap();
    assert_eq!(read, "the bytes");
}

#[test]
fn an_empty_file_is_as_harmless_as_any_other() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty");
    fs::write(&path, "").unwrap();

    clear_nonblocking(&File::open(&path).unwrap()).unwrap();
}

// A pipe shows what the flag does, and so what taking it off changes: with it on, a read that has
// nothing to read fails at once, and with it off, the read waits for the writer.
#[cfg(unix)]
#[test]
fn a_read_of_a_pipe_waits_for_its_data_once_the_flag_is_off() {
    use std::{
        fs::OpenOptions,
        io::{ErrorKind, Write},
        os::unix::fs::OpenOptionsExt,
        process::Command,
        thread,
        time::Duration,
    };

    let dir = tempfile::tempdir().unwrap();
    let pipe = dir.path().join("pipe");
    let made = Command::new("mkfifo").arg(&pipe).status().unwrap();
    assert!(made.success());

    // The way a name that cannot be trusted is opened: for a pipe, the open does not wait for a
    // writer to come. The writer is opened next, which the reader being there lets it do at once.
    let mut reader = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&pipe)
        .unwrap();
    let mut writer = OpenOptions::new().write(true).open(&pipe).unwrap();

    let mut byte = [0_u8; 1];
    let error = reader.read(&mut byte).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::WouldBlock, "{error}");

    clear_nonblocking(&reader).unwrap();

    // The writer writes a little later. If the flag were still on, the read below would fail at
    // once and the test would fail, and the thread would still end.
    let late = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        writer.write_all(b"x").unwrap();
    });
    assert_eq!(reader.read(&mut byte).unwrap(), 1);
    assert_eq!(&byte, b"x");
    late.join().unwrap();
}
