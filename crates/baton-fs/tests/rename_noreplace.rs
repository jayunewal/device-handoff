// SPDX-License-Identifier: Apache-2.0

//! Moving a name without replacing what has it, on whatever system the tests run on.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};

use baton_fs::rename_noreplace;

fn write(path: impl AsRef<Path>, content: &str) {
    fs::write(path, content).unwrap();
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap()
}

/// A folder with a file in it and another folder with a file in that.
fn folder(path: &Path) {
    fs::create_dir_all(path.join("inner")).unwrap();
    write(path.join("one.txt"), "one");
    write(path.join("inner").join("two.txt"), "two");
}

fn kind(result: std::io::Result<()>) -> ErrorKind {
    result.unwrap_err().kind()
}

#[test]
fn a_file_moves_to_a_name_that_is_free() {
    let dir = tempfile::tempdir().unwrap();
    let (from, to) = (dir.path().join("staged"), dir.path().join("notes.txt"));
    write(&from, "received");

    rename_noreplace(&from, &to).unwrap();

    assert!(!from.exists(), "the old name is gone");
    assert_eq!(read(&to), "received");
}

#[test]
fn a_file_does_not_replace_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let (from, to) = (dir.path().join("staged"), dir.path().join("notes.txt"));
    write(&from, "received");
    write(&to, "what the person had");

    assert_eq!(kind(rename_noreplace(&from, &to)), ErrorKind::AlreadyExists);

    assert_eq!(read(&from), "received", "nothing was moved");
    assert_eq!(read(&to), "what the person had", "nothing was replaced");
}

#[test]
fn a_file_does_not_take_the_name_of_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (from, to) = (dir.path().join("staged"), dir.path().join("photos"));
    write(&from, "received");
    fs::create_dir(&to).unwrap();

    assert_eq!(kind(rename_noreplace(&from, &to)), ErrorKind::AlreadyExists);

    assert_eq!(read(&from), "received");
    assert!(to.is_dir());
}

#[test]
fn a_folder_moves_with_everything_in_it() {
    let dir = tempfile::tempdir().unwrap();
    let (from, to) = (dir.path().join("staged"), dir.path().join("photos"));
    folder(&from);

    rename_noreplace(&from, &to).unwrap();

    assert!(!from.exists());
    assert_eq!(read(to.join("one.txt")), "one");
    assert_eq!(read(to.join("inner").join("two.txt")), "two");
}

// `std::fs::rename` replaces a folder that is empty, which is the case that a check made before
// the move cannot rule out, so it has a test of its own.
#[test]
fn a_folder_does_not_replace_a_folder_even_when_that_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("staged");
    folder(&from);

    let empty = dir.path().join("empty");
    fs::create_dir(&empty).unwrap();
    assert_eq!(
        kind(rename_noreplace(&from, &empty)),
        ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);

    let full = dir.path().join("full");
    folder(&full);
    assert_eq!(
        kind(rename_noreplace(&from, &full)),
        ErrorKind::AlreadyExists
    );
    assert_eq!(read(full.join("one.txt")), "one");

    assert_eq!(read(from.join("one.txt")), "one", "nothing was moved");
}

#[test]
fn a_folder_does_not_take_the_name_of_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let (from, to) = (dir.path().join("staged"), dir.path().join("notes.txt"));
    folder(&from);
    write(&to, "what the person had");

    assert_eq!(kind(rename_noreplace(&from, &to)), ErrorKind::AlreadyExists);

    assert_eq!(read(&to), "what the person had");
    assert_eq!(read(from.join("one.txt")), "one");
}

#[test]
fn a_name_cannot_be_moved_onto_itself() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    write(&path, "received");

    assert_eq!(
        kind(rename_noreplace(&path, &path)),
        ErrorKind::AlreadyExists
    );

    assert_eq!(read(&path), "received");
}

#[test]
fn a_missing_file_or_folder_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    write(&staged, "received");

    assert_eq!(
        kind(rename_noreplace(
            dir.path().join("nothing"),
            dir.path().join("notes.txt")
        )),
        ErrorKind::NotFound
    );
    // The folder of the new name is not made on the way.
    let to = dir.path().join("missing").join("notes.txt");
    assert_eq!(kind(rename_noreplace(&staged, &to)), ErrorKind::NotFound);
    assert!(!dir.path().join("missing").exists());
    assert_eq!(read(&staged), "received");
}

#[test]
fn a_file_moves_into_a_folder_that_exists() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("Downloads");
    write(&staged, "received");
    fs::create_dir(&target).unwrap();

    rename_noreplace(&staged, target.join("notes.txt")).unwrap();

    assert_eq!(read(target.join("notes.txt")), "received");
}

#[test]
fn a_path_with_a_nul_character_is_refused_before_anything_is_called() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    write(&staged, "received");
    let bad = dir.path().join("a\0b");

    assert_eq!(
        kind(rename_noreplace(&staged, &bad)),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(rename_noreplace(&bad, dir.path().join("free"))),
        ErrorKind::InvalidInput
    );
    assert_eq!(read(&staged), "received");
}

#[test]
fn paths_can_be_text_owned_text_or_paths() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap().to_owned();
    write(dir.path().join("a"), "a");

    rename_noreplace(format!("{root}/a"), format!("{root}/b")).unwrap();
    rename_noreplace(format!("{root}/b"), PathBuf::from(&root).join("c")).unwrap();
    rename_noreplace(Path::new(&root).join("c"), format!("{root}/d").as_str()).unwrap();

    assert_eq!(read(dir.path().join("d")), "a");
}

#[test]
fn names_with_characters_outside_ascii_move_as_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let wanted = dir.path().join("Käse – 文件 🧀.txt");
    write(&staged, "received");

    rename_noreplace(&staged, &wanted).unwrap();

    assert_eq!(read(&wanted), "received");
    assert!(!staged.exists());
    // One name is left, whichever way the file system writes it (some normalise).
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

// A path longer than the 260 characters that Windows allows without a prefix. Unix has no such
// limit at this length, so there the test only shows that the path is not cut short.
#[test]
fn a_path_longer_than_windows_allows_without_a_prefix_moves() {
    let dir = tempfile::tempdir().unwrap();
    let deep = ["a".repeat(100), "b".repeat(100), "c".repeat(100)]
        .iter()
        .fold(dir.path().to_path_buf(), |path, part| path.join(part));
    fs::create_dir_all(&deep).unwrap();
    assert!(deep.as_os_str().len() > 300);
    let (from, to) = (deep.join("staged"), deep.join("notes.txt"));
    write(&from, "received");
    write(&to, "what the person had");

    assert_eq!(kind(rename_noreplace(&from, &to)), ErrorKind::AlreadyExists);
    let free = deep.join("notes 2.txt");
    rename_noreplace(&from, &free).unwrap();

    assert_eq!(read(&free), "received");
    assert_eq!(read(&to), "what the person had");
}

// Of all the transfers that want one name, one gets it. This is what the call is for: with a
// look before the move, two of them could both see a free name.
#[test]
fn of_many_moves_to_one_name_exactly_one_wins() {
    const MOVERS: usize = 12;
    for round in 0..40 {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("notes.txt");
        let barrier = Arc::new(Barrier::new(MOVERS));
        let movers: Vec<_> = (0..MOVERS)
            .map(|index| {
                let staged = dir.path().join(format!("staged {index}"));
                write(&staged, &format!("from {index}"));
                let (wanted, barrier) = (wanted.clone(), Arc::clone(&barrier));
                thread::spawn(move || {
                    barrier.wait();
                    (index, staged.clone(), rename_noreplace(&staged, &wanted))
                })
            })
            .collect();
        let results: Vec<_> = movers.into_iter().map(|m| m.join().unwrap()).collect();

        let winners: Vec<_> = results.iter().filter(|(_, _, r)| r.is_ok()).collect();
        assert_eq!(winners.len(), 1, "round {round}: {results:?}");
        let (winner, _, _) = winners[0];
        assert_eq!(read(&wanted), format!("from {winner}"));
        for (index, staged, result) in &results {
            if index != winner {
                let error = result.as_ref().unwrap_err();
                assert_eq!(error.kind(), ErrorKind::AlreadyExists, "round {round}");
                assert_eq!(
                    read(staged),
                    format!("from {index}"),
                    "a loser keeps its file"
                );
            }
        }
    }
}

#[cfg(unix)]
mod links {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn a_link_that_points_nowhere_still_has_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("staged"), dir.path().join("notes.txt"));
        write(&from, "received");
        symlink(dir.path().join("nowhere"), &to).unwrap();

        assert_eq!(kind(rename_noreplace(&from, &to)), ErrorKind::AlreadyExists);

        assert_eq!(read(&from), "received");
        assert!(fs::symlink_metadata(&to).unwrap().is_symlink());
        assert!(
            !dir.path().join("nowhere").exists(),
            "the link was not followed"
        );
    }

    #[test]
    fn a_link_to_a_folder_still_has_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("staged"), dir.path().join("photos"));
        let elsewhere = dir.path().join("elsewhere");
        folder(&from);
        folder(&elsewhere);
        symlink(&elsewhere, &to).unwrap();

        assert_eq!(kind(rename_noreplace(&from, &to)), ErrorKind::AlreadyExists);

        assert_eq!(
            read(elsewhere.join("one.txt")),
            "one",
            "nothing went through the link"
        );
    }

    #[test]
    fn a_link_moves_as_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let (target, from, to) = (
            dir.path().join("target.txt"),
            dir.path().join("staged"),
            dir.path().join("link"),
        );
        write(&target, "target");
        symlink(&target, &from).unwrap();

        rename_noreplace(&from, &to).unwrap();

        assert!(fs::symlink_metadata(&to).unwrap().is_symlink());
        assert_eq!(fs::read_link(&to).unwrap(), target);
        assert_eq!(read(&target), "target");
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt, os::unix::fs::MetadataExt};

    use super::*;

    #[test]
    fn a_name_that_is_not_text_moves_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged");
        let wanted = dir.path().join(OsStr::from_bytes(b"caf\xe9 \xff.txt"));
        write(&staged, "received");

        rename_noreplace(&staged, &wanted).unwrap();

        assert_eq!(read(&wanted), "received");
        assert_eq!(
            kind(rename_noreplace(dir.path().join("nothing"), &wanted)),
            ErrorKind::NotFound
        );
    }

    // A move is made inside one file system. Another volume is an error, and is not made by
    // copying. The test needs a second file system, so it looks for one that most Linux systems
    // have (`/dev/shm`) and stands down where there is none.
    #[test]
    fn another_file_system_is_an_error_and_not_a_copy() {
        let here = tempfile::tempdir().unwrap();
        let shm = Path::new("/dev/shm");
        let there = match tempfile::tempdir_in(shm) {
            Ok(there)
                if there.path().metadata().unwrap().dev()
                    != here.path().metadata().unwrap().dev() =>
            {
                there
            }
            _ => {
                eprintln!("skipped: no second file system to move to");
                return;
            }
        };
        let (from, to) = (here.path().join("staged"), there.path().join("notes.txt"));
        write(&from, "received");

        assert_eq!(
            kind(rename_noreplace(&from, &to)),
            ErrorKind::CrossesDevices
        );

        assert_eq!(read(&from), "received");
        assert!(!to.exists());
    }
}
