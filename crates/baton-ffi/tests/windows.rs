// SPDX-License-Identifier: Apache-2.0

//! The library starts the app with the viewer and the sharer that the platform registered through
//! the C interface of `baton-stream-abi`, as the Swift code of the app does: once for the process,
//! before the app starts.
//!
//! The platform of this test is written as a real one is, with C functions and pointers, and it
//! never makes a screen or a window: what is tested is that the app is given the viewer and the
//! sharer.

#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{ffi::c_void, mem::size_of, ptr};

use baton_ffi::api::baton::Baton;
use baton_stream_abi::{
    abi::{
        ABI_VERSION, BatonDecoderState, BatonFormat, BatonFrame, BatonLimits, BatonMessage,
        BatonPost, BatonScreen, BatonSharerVTable, BatonSourceMessage, BatonSurroundings,
        BatonTakes, BatonViewerVTable, BatonWindow, CODEC_H264, E_PLATFORM, OK,
    },
    exports::{baton_stream_register_sharer, baton_stream_register_viewer},
};
use tempfile::TempDir;

fn size_of_u32<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap()
}

unsafe extern "C" fn limits(_: *mut c_void, limits: *mut BatonLimits) -> i32 {
    // SAFETY: the library gives a struct of the interface, which is the platform's to write.
    unsafe {
        limits.write(BatonLimits {
            size: size_of_u32::<BatonLimits>(),
            max_width: 1920,
            max_height: 1200,
            max_fps: 60,
            codecs: CODEC_H264,
            reserved: 0,
        });
    }
    OK
}

unsafe extern "C" fn open(
    _: *mut c_void,
    _: *const BatonWindow,
    _: u64,
    _: *mut BatonScreen,
    _: *mut BatonMessage,
) -> i32 {
    E_PLATFORM
}

unsafe extern "C" fn decode(_: *mut c_void, _: *const BatonFrame) -> i32 {
    E_PLATFORM
}

unsafe extern "C" fn show(_: *mut c_void, _: u64) {}

unsafe extern "C" fn source(_: *mut c_void, _: *const BatonSourceMessage) {}

unsafe extern "C" fn state(_: *mut c_void, _: *mut BatonDecoderState) {}

unsafe extern "C" fn ended(_: *mut c_void) {}

unsafe extern "C" fn release(_: *mut c_void) {}

unsafe extern "C" fn control(_: *mut c_void) -> u32 {
    1
}

unsafe extern "C" fn pick(_: *mut c_void, _: u64, _: *mut BatonMessage) -> i32 {
    E_PLATFORM
}

unsafe extern "C" fn cancel_pick(_: *mut c_void, _: u64) {}

unsafe extern "C" fn prepare(_: *mut c_void, _: *const BatonTakes, _: *mut BatonFormat) -> i32 {
    E_PLATFORM
}

unsafe extern "C" fn begin(_: *mut c_void, _: u64, _: *mut BatonMessage) -> i32 {
    E_PLATFORM
}

unsafe extern "C" fn surroundings(_: *mut c_void, _: *mut BatonSurroundings) -> i32 {
    E_PLATFORM
}

unsafe extern "C" fn post(_: *mut c_void, _: *const BatonPost) -> i32 {
    E_PLATFORM
}

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
async fn the_app_is_started_with_the_viewer_and_the_sharer_that_the_platform_registered() {
    // This is the only test of the process, since a viewer and a sharer are registered once for
    // its life. Before they are, the device takes no window and shares none.
    let (before, _folder) = start().await;
    let abilities = before.window_abilities();
    assert!(!abilities.can_show);
    assert!(!abilities.can_share);
    before.stop().await;

    let vtable = BatonViewerVTable {
        version: ABI_VERSION,
        size: size_of_u32::<BatonViewerVTable>(),
        context: ptr::null_mut(),
        limits: Some(limits),
        open: Some(open),
        decode: Some(decode),
        show: Some(show),
        source: Some(source),
        state: Some(state),
        ended: Some(ended),
        release: Some(release),
    };
    // SAFETY: a vtable that is whole, with functions that keep to the header.
    assert_eq!(
        unsafe { baton_stream_register_viewer(&raw const vtable) },
        OK
    );

    // The next Baton to start takes the windows that other devices share, and shares none.
    let (after, _folder) = start().await;
    let abilities = after.window_abilities();
    assert!(abilities.can_show);
    assert!(!abilities.can_share);
    assert!(!abilities.can_control);
    after.stop().await;

    let vtable = BatonSharerVTable {
        version: ABI_VERSION,
        size: size_of_u32::<BatonSharerVTable>(),
        context: ptr::null_mut(),
        control: Some(control),
        pick: Some(pick),
        cancel_pick: Some(cancel_pick),
        prepare: Some(prepare),
        start: Some(begin),
        ended: Some(ended),
        release: Some(release),
        surroundings: Some(surroundings),
        post: Some(post),
    };
    // SAFETY: a vtable that is whole, with functions that keep to the header.
    assert_eq!(
        unsafe { baton_stream_register_sharer(&raw const vtable) },
        OK
    );

    // The next one also shares the windows of this device, and lets the person grant control of
    // them, since the platform says it can post input.
    let (last, _folder) = start().await;
    let abilities = last.window_abilities();
    assert!(abilities.can_show);
    assert!(abilities.can_share);
    assert!(abilities.can_control);
    last.stop().await;
}
