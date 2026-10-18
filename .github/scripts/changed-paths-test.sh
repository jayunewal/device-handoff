#!/usr/bin/env bash
# Holds the rules in changed-paths.sh to the changes people really make. Run it from anywhere:
#
#   bash .github/scripts/changed-paths-test.sh
#
# CI runs it first, so a change to the rules that skips a job it should not fails there.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
failed=0

# check "<the groups that should run, in the order rust app mac iphone, or none>" <path>...
check() {
  expected=$1
  shift
  actual=$(printf '%s\n' "$@" | bash "$here/changed-paths.sh" | sed -n 's/=true$//p' | tr '\n' ' ')
  actual=${actual% }
  [ -n "$actual" ] || actual=none
  if [ "$actual" != "$expected" ]; then
    echo "FAILED: $* -> [$actual], expected [$expected]" >&2
    failed=1
  fi
}

all="rust app mac iphone"

# The Rust, and the app that is built from it, run everything.
check "$all" crates/baton-core/src/node.rs
check "$all" crates/baton-ffi/src/api/startup.rs
check "$all" crates/baton-app/tests/app.rs
check "$all" Cargo.lock
check "$all" Cargo.toml
check "$all" deny.toml
check "$all" rust-toolchain.toml
check "$all" LICENSE

# So does a change to CI itself, and so does anything the rules do not know.
check "$all" .github/workflows/ci.yml
check "$all" .github/scripts/changed-paths.sh
check "$all" .github/scripts/changed-paths-test.sh
check "$all" tools/something.sh
check "$all" new-file-at-the-top
check "$all" apps/baton/ios/Runner/Info.plist apps/baton/macos/Runner/Info.plist crates/baton-core/src/lib.rs

# The app: Dart code is built for both kinds of Apple device and tested on Linux.
check "app mac iphone" apps/baton/lib/src/awake.dart
check "app mac iphone" apps/baton/lib/src/rust/api/startup.dart
check "app mac iphone" apps/baton/pubspec.yaml
check "app mac iphone" apps/baton/pubspec.lock
check "app mac iphone" apps/baton/integration_test/app_test.dart
check "app mac iphone" apps/baton/rust_builder/cargokit/build_tool/pubspec.yaml
check "app mac iphone" apps/baton/rust_builder/cargokit/build_pod.sh
check "app mac iphone" apps/baton/flutter_rust_bridge.yaml
check "app mac iphone" apps/baton/ios/Runner/Info.plist apps/baton/macos/Runner/Info.plist
check "app mac iphone" apps/baton/lib/main.dart README.md
# The analyzer reads a Dart file wherever it is in the app's folder, so one in tool/ is not left out.
check "app mac iphone" apps/baton/tool/generate.dart

# One platform's project runs that platform's job, and the Linux job, which checks the names
# in both projects.
check "app iphone" apps/baton/ios/Runner/AppDelegate.swift
check "app iphone" "apps/baton/ios/Runner/Assets.xcassets/App Icon.appiconset/icon 1024.png"
check "app mac" apps/baton/macos/Runner/DebugProfile.entitlements
check "app mac" apps/baton/macos/Runner.xcodeproj/project.pbxproj

# Tests of the Dart code run on Linux only.
check "app" apps/baton/test/awake_test.dart
check "app" apps/baton/test/fake_screen.dart

# Words for people, and files only GitHub reads, run nothing.
check none README.md
check none CONTRIBUTING.md SECURITY.md CODE_OF_CONDUCT.md
check none docs/architecture.md docs/adr/0007-quic-windows-and-connection-memory.md
check none docs/research/locked-devices/apple.md docs/images/overview.svg
check none apps/baton/README.md apps/baton/rust_builder/README.md
check none .github/PULL_REQUEST_TEMPLATE.md .github/ISSUE_TEMPLATE/bug_report.yml
check none .github/CODEOWNERS .github/dependabot.yml
check none apps/baton/tool/make_icons.py
check none README.md docs/security.md .github/ISSUE_TEMPLATE/config.yml

# The Build jobs pack docs/try-it.md into the downloads.
check "rust" docs/try-it.md
check "rust" docs/try-it.md README.md
check "rust app mac iphone" docs/try-it.md apps/baton/lib/main.dart

# A change that is partly words runs what the rest of it needs.
check "app mac iphone" README.md apps/baton/lib/main.dart docs/install-the-app.md

# Nothing to go by runs everything: an empty list, and a list of blank lines.
actual=$(printf '' | bash "$here/changed-paths.sh" | tr '\n' ' ')
if [ "$actual" != "rust=true app=true mac=true iphone=true " ]; then
  echo "FAILED: an empty list gave [$actual]" >&2
  failed=1
fi
actual=$(printf '\n\n' | bash "$here/changed-paths.sh" | tr '\n' ' ')
if [ "$actual" != "rust=true app=true mac=true iphone=true " ]; then
  echo "FAILED: blank lines gave [$actual]" >&2
  failed=1
fi

# The last path may come without a newline after it.
actual=$(printf 'README.md\napps/baton/test/awake_test.dart' | bash "$here/changed-paths.sh" | tr '\n' ' ')
if [ "$actual" != "rust=false app=true mac=false iphone=false " ]; then
  echo "FAILED: a last line without a newline gave [$actual]" >&2
  failed=1
fi

if [ "$failed" != 0 ]; then
  exit 1
fi
echo "changed-paths: every case gave the answer it should"
