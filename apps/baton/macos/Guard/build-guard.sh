#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Builds the guard of the privacy mode (docs/adr/0011-privacy-mode.md) and puts it into the app, next
# to the app's own program, signed to inherit the sandbox of the app that starts it. Xcode runs this
# as the "Build Guard" phase of the Runner target, after the sources are built and before the app is
# signed, so that the signature of the app has the guard in it.
#
# The guard is three files of Swift: main.swift of this folder joins BatonGuard.swift (what it reads
# from the app, and when it locks the Mac) and BatonLock.swift (how it locks it) of the plugin. It is
# built here and not by the plugin, since a pod makes libraries and this has to be a program.
set -eu

here="$SRCROOT/Guard"
classes="$SRCROOT/../rust_builder/macos/Classes"
programs="$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH/MacOS"
work="$DERIVED_FILE_DIR/baton-guard"
mkdir -p "$programs" "$work"

# One program for each kind of processor that the app is built for.
slices=""
for arch in ${ARCHS:-$(uname -m)}; do
  xcrun swiftc -O -swift-version 5 -sdk "$SDKROOT" \
    -target "$arch-apple-macos${MACOSX_DEPLOYMENT_TARGET:-12.0}" \
    "$here/main.swift" "$classes/BatonGuard.swift" "$classes/BatonLock.swift" \
    -o "$work/BatonGuard-$arch"
  slices="$slices $work/BatonGuard-$arch"
done
# shellcheck disable=SC2086
lipo -create $slices -output "$programs/BatonGuard"

runtime=""
if [ "${ENABLE_HARDENED_RUNTIME:-NO}" = "YES" ]; then
  runtime="--options runtime"
fi
# The guard has an identifier of its own, which the system tells it from the app by. A build that is
# signed with no identity is signed ad hoc, as the app is.
# shellcheck disable=SC2086
codesign --force --sign "${EXPANDED_CODE_SIGN_IDENTITY:--}" $runtime \
  --identifier io.github.jayunewal.baton.guard \
  --entitlements "$here/Guard.entitlements" "$programs/BatonGuard"
