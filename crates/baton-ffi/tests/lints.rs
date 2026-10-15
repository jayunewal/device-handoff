// SPDX-License-Identifier: Apache-2.0

//! This crate lists its lints itself, because it cannot take the workspace's (see the comment
//! above `[lints.rust]` in its `Cargo.toml`). A lint added to the workspace and not here would
//! quietly leave this crate unchecked, so this test keeps the two lists equal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{fs, path::Path};

use toml::Table;

fn read(path: &Path) -> Table {
    let text = fs::read_to_string(path).unwrap();
    text.parse().unwrap()
}

fn table<'a>(root: &'a Table, path: &[&str]) -> &'a Table {
    let mut at = root;
    for key in path {
        at = at
            .get(*key)
            .and_then(toml::Value::as_table)
            .unwrap_or_else(|| panic!("no table {path:?} in the manifest"));
    }
    at
}

#[test]
fn the_lints_are_the_workspaces_except_for_unsafe_code() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let own = read(&here.join("Cargo.toml"));
    let workspace = read(&here.join("../../Cargo.toml"));

    // The two differences, both on purpose: `unsafe_code` is denied here and forbidden in the
    // workspace, and the macros of flutter_rust_bridge test for a cfg that rustc does not know.
    let mut own_rust = table(&own, &["lints", "rust"]).clone();
    let mut workspace_rust = table(&workspace, &["workspace", "lints", "rust"]).clone();
    assert_eq!(
        own_rust
            .remove("unsafe_code")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("deny".to_owned()),
        "unsafe code is denied everywhere except the generated module"
    );
    assert_eq!(
        workspace_rust
            .remove("unsafe_code")
            .and_then(|v| v.as_str().map(str::to_owned)),
        Some("forbid".to_owned()),
        "the workspace has changed how it treats unsafe code; revisit this crate's lints"
    );
    assert!(own_rust.remove("unexpected_cfgs").is_some());
    assert_eq!(
        own_rust, workspace_rust,
        "[lints.rust] differs from the workspace's"
    );

    assert_eq!(
        table(&own, &["lints", "clippy"]),
        table(&workspace, &["workspace", "lints", "clippy"]),
        "[lints.clippy] differs from the workspace's"
    );
}
