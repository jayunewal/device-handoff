// SPDX-License-Identifier: Apache-2.0

//! The calls about unlocking, as the Dart side makes them, on a Baton that has no platform and no
//! paired device. What the unlock does between two devices is tested in `baton-app` and
//! `baton-stream-abi`; this is what the bridge says on its own: that the numbers of a session are
//! taken as they are, that a device that is not paired is named as such, and what a device with no
//! platform code says it can do.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use baton_ffi::api::{baton::Baton, error::ErrorKind};
use tempfile::TempDir;

/// A Baton in a folder of its own, with settings that keep it to this computer.
async fn start() -> (Baton, TempDir) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("config.toml"),
        "[network]\nrelay = \"disabled\"\nlan_discovery = false\nport_mapping = false\n",
    )
    .unwrap();
    let baton = Baton::start(
        folder.path().to_string_lossy().into_owned(),
        Some(false),
        Some(false),
        None,
    )
    .await
    .unwrap();
    (baton, folder)
}

#[tokio::test]
async fn a_device_with_no_platform_code_neither_unlocks_nor_is_unlocked() {
    let (baton, _folder) = start().await;
    let abilities = baton.window_abilities();
    assert!(!abilities.can_be_unlocked);
    assert!(!abilities.can_unlock);
    baton.stop().await;
}

#[tokio::test]
async fn the_calls_about_a_device_that_is_not_paired_say_so() {
    let (baton, _folder) = start().await;
    for id in ["ab".repeat(32), "not an id".to_owned(), String::new()] {
        assert_eq!(
            baton.unlock_status(id.clone()).unwrap_err().kind,
            ErrorKind::NotPaired,
            "{id:?}"
        );
        assert_eq!(
            baton.turn_off_unlock(id.clone()).unwrap_err().kind,
            ErrorKind::NotPaired,
            "{id:?}"
        );
        assert_eq!(
            baton
                .set_up_unlock(id.clone(), "hunter2".to_owned())
                .await
                .unwrap_err()
                .kind,
            ErrorKind::NotPaired,
            "{id:?}"
        );
    }
    baton.stop().await;
}

#[tokio::test]
async fn a_session_that_does_not_exist_is_not_unlocked_and_nothing_is_in_the_history() {
    let (baton, _folder) = start().await;
    // A number that Dart cannot have been given, and ones that no window has.
    for session in [i64::MIN, -1, 0, 1, i64::MAX] {
        assert!(!baton.unlock_window(session), "{session}");
    }
    assert_eq!(baton.unlock_history().len(), 0);
    baton.stop().await;
}
