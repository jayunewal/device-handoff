#!/usr/bin/env bash
# Says which groups of CI jobs a change can affect, from the paths the change touches.
#
# The paths come on standard input, one to a line. The answer is a line for each group, such
# as `rust=true`, ready to be appended to $GITHUB_OUTPUT:
#
#   rust    Format, Clippy, Test, Minimum supported Rust version, Rustdoc, Licenses and
#           advisories, Build
#   app     App (Flutter, Linux)
#   mac     App (macOS)
#   iphone  App (iPhone)
#
# A group is false only when every path is one that cannot change what its jobs do. A path that
# none of the rules below knows, such as a file in a new folder, makes every group true, and so
# does an empty list: a job that is skipped by mistake costs more than one that runs for
# nothing.
#
# "What CI runs" in CONTRIBUTING.md says the same in words, and changed-paths-test.sh holds the
# rules to the changes people really make. CI runs it before it uses this script.
set -eu

# Whether a change to the path $2 can affect the jobs of the group $1.
affects() {
  case "$2" in
    # Only the Build jobs read this one, because they pack it into the downloads.
    docs/try-it.md)
      test "$1" = rust
      return
      ;;
    # Words for people, and files that only GitHub reads. No job reads any of them.
    *.md | docs/* | .github/ISSUE_TEMPLATE/* | .github/CODEOWNERS | .github/dependabot.yml)
      return 1
      ;;
    # The script for whoever makes the app's icons. Nothing at build time runs it. A Dart file in
    # that folder would be read by the analyzer, so only Python is left out.
    apps/baton/tool/*.py)
      return 1
      ;;
  esac
  case "$1:$2" in
    # Nothing under apps/ is part of the Rust workspace. The app jobs build the Rust that they
    # need from crates/, and a change there runs them.
    rust:apps/*) return 1 ;;
    # Each Mac job builds one platform's project. The tests of the Dart code run in the Linux
    # job, and so do the checks of the names in both platform projects.
    mac:apps/baton/ios/* | mac:apps/baton/test/*) return 1 ;;
    iphone:apps/baton/macos/* | iphone:apps/baton/test/*) return 1 ;;
  esac
  return 0
}

rust=false
app=false
mac=false
iphone=false
seen=false
while IFS= read -r path || [ -n "$path" ]; do
  [ -n "$path" ] || continue
  seen=true
  for group in rust app mac iphone; do
    if affects "$group" "$path"; then
      eval "$group=true"
    fi
  done
done

if [ "$seen" = false ]; then
  rust=true
  app=true
  mac=true
  iphone=true
fi
printf 'rust=%s\napp=%s\nmac=%s\niphone=%s\n' "$rust" "$app" "$mac" "$iphone"
