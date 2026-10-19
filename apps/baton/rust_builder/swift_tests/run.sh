#!/bin/sh
# Builds the Swift of the viewer with its tests, and runs them. Any computer with a Swift compiler
# can: the tests of the decoder that is VideoToolbox, which need a Mac, are left out on the others.
#
#     apps/baton/rust_builder/swift_tests/run.sh [extra arguments for swiftc, such as -sanitize=thread]
set -eu

here=$(cd "$(dirname "$0")" && pwd)
classes="$here/../macos/Classes"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

swiftc -swift-version 5 "$@" \
  -import-objc-header "$classes/baton_stream_abi.h" \
  "$classes/BatonViewer.swift" "$classes/BatonH264Decoder.swift" "$here"/*.swift \
  -o "$work/tests"
"$work/tests"
