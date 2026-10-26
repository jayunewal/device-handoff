#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Builds the checks for the privacy mode of a Mac into a throwaway app, signs it, and runs the stages
# with a person at the Mac (docs/research/privacy-mode/README.md).
#
#   sh run.sh                run every stage: tap secure shield cursor
#   sh run.sh shield tap     run these stages, in this order
#   sh run.sh clean          take away what a run left behind
#
# The app is sandboxed as the app of Baton is. It and this script leave notes for each other in
# /tmp/baton-probe. Everything it does to the Mac ends by itself, and this script undoes what a
# crash could leave when it ends.
set -eu

if [ "$(uname)" != Darwin ]; then
  echo "This runs on a Mac." >&2
  exit 1
fi

here=$(cd "$(dirname "$0")" && pwd)
work=/tmp/baton-probe
app=$work/BatonProbe.app
exe=$app/Contents/MacOS/probe

if [ "${1:-}" = clean ]; then
  pkill -f "$exe" 2>/dev/null || true
  "$exe" restore >/dev/null 2>&1 || true
  rm -rf "$work"
  echo "Removed $work"
  exit 0
fi

stages=${*:-tap secure shield cursor}

mkdir -p "$work"

# The identity that signs the app is the one that a free Apple ID gives, when there is one, since
# that is what Baton can be signed with. Without one the app is signed ad hoc.
identity=${BATON_PROBE_IDENTITY:-}
if [ -z "$identity" ]; then
  identity=$(security find-identity -v -p codesigning 2>/dev/null |
    sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)
fi
[ -n "$identity" ] || identity=-

build() {
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS"
  swiftc -O -swift-version 5 -o "$exe" "$here/probe.swift"
  cp "$here/Info.plist" "$app/Contents/Info.plist"
  codesign --force --sign "$1" --entitlements "$here/probe.entitlements" "$app"
  printf '%s\n' "$1" >"$work/.identity"
}

# A build that is there already is used again, since signing it again would take away what the
# person allowed it in System Settings when it is signed ad hoc.
sum=$(cat "$here/probe.swift" "$here/Info.plist" "$here/probe.entitlements" | shasum | cut -d' ' -f1)
sum="$sum $identity"
if [ ! -d "$app" ] || [ "$(cat "$work/.sum" 2>/dev/null || true)" != "$sum" ]; then
  echo "Building the checks, signed with: $identity"
  if ! build "$identity"; then
    echo "Signing with that identity failed, signing ad hoc instead" >&2
    identity=-
    sum=$(cat "$here/probe.swift" "$here/Info.plist" "$here/probe.entitlements" | shasum | cut -d' ' -f1)
    sum="$sum $identity"
    build "$identity"
  fi
  printf '%s\n' "$sum" >"$work/.sum"
fi

cat <<'TEXT'

This holds the keys and the pointer of this Mac for 10 seconds at a time, covers the screens with
black for up to 40 seconds, hides the pointer and turns the backlight down for 8 seconds. Each of
these ends by itself. Stay at the Mac, turn Focus off, and press Escape three times to end a hold.

TEXT

rm -f "$work/phase" "$work/report.txt" "$work/field-ready" "$work/field-stop" "$work/field-result"
printf '%s\n' "$stages" >"$work/stages"

banner() {
  osascript -e "display notification \"$1 banner of the check\" with title \"Baton privacy check\"" || true
}

opener=
finish() {
  [ -n "$opener" ] && kill "$opener" 2>/dev/null || true
  pkill -f "$exe" 2>/dev/null || true
  "$exe" restore >/dev/null 2>&1 || true
}
trap finish EXIT INT TERM

open -n -W "$app" &
opener=$!

last=
while kill -0 "$opener" 2>/dev/null; do
  now=$(cat "$work/phase" 2>/dev/null || true)
  if [ "$now" != "$last" ]; then
    last=$now
    case "$now" in
    field-plain) open -n -a "$app" --args field plain ;;
    field-secure) open -n -a "$app" --args field secure ;;
    shield-banners)
      (
        sleep 8
        banner first
        sleep 10
        banner second
      ) &
      ;;
    esac
  fi
  sleep 0.4
done

echo
echo "---- report ($work/report.txt) ----"
cat "$work/report.txt" 2>/dev/null ||
  echo "No report was written: the app did not start. Look at: log show --last 2m --predicate 'process == \"probe\"'"
