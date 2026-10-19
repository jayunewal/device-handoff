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

fn level(table: &mut Table, lint: &str) -> Option<String> {
    table
        .remove(lint)
        .and_then(|value| value.as_str().map(str::to_owned))
}

#[test]
fn the_lints_are_the_workspaces_except_for_unsafe_code() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let own = read(&here.join("Cargo.toml"));
    let workspace = read(&here.join("../../Cargo.toml"));

    // The one difference, on purpose: `unsafe_code` is denied here, with an allow on the two
    // modules that cross the interface to the platform, and forbidden in the workspace.
    let mut own_rust = table(&own, &["lints", "rust"]).clone();
    let mut workspace_rust = table(&workspace, &["workspace", "lints", "rust"]).clone();
    assert_eq!(
        level(&mut own_rust, "unsafe_code").as_deref(),
        Some("deny"),
        "unsafe code is denied everywhere except the modules that cross the interface"
    );
    assert_eq!(
        level(&mut workspace_rust, "unsafe_code").as_deref(),
        Some("forbid"),
        "the workspace has changed how it treats unsafe code; revisit this crate's lints"
    );
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
