#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Builds the checks for the privacy mode of a Mac into a throwaway app, signs it, and runs the stages
# with a person at the Mac (docs/research/privacy-mode/README.md).
#
#   sh run.sh                run the stages capture, gamma, sblock and guard
#   sh run.sh tap shield     run these stages, in this order
#   sh run.sh clean          take away what a run left behind
#
# The stages are tap, secure, shield and cursor (the first round), and capture, gamma, sblock and
# guard (the second). The app and its helper are sandboxed as the app of Baton is. They and this
# script leave notes for each other in /tmp/baton-probe. Everything they do to the Mac ends by
# itself, and this script undoes what a crash could leave when it ends. The app is built into
# ~/Applications, or into the folder in BATON_PROBE_APPS: the system does not let the app be allowed
# for the keys and the pointer when it is in /tmp.
set -eu

if [ "$(uname)" != Darwin ]; then
  echo "This runs on a Mac." >&2
  exit 1
fi

here=$(cd "$(dirname "$0")" && pwd)
work=/tmp/baton-probe
app=${BATON_PROBE_APPS:-$HOME/Applications}/BatonProbe.app
exe=$app/Contents/MacOS/probe
helper=$app/Contents/MacOS/guard
agent=io.github.jayunewal.baton.probe.guard

if [ "${1:-}" = clean ]; then
  pkill -f "$exe" 2>/dev/null || true
  pkill -f "$helper" 2>/dev/null || true
  launchctl bootout "gui/$(id -u)/$agent" 2>/dev/null || true
  "$exe" restore >/dev/null 2>&1 || true
  rm -rf "$app" "$work"
  rm -rf "$HOME/Library/Containers/$agent" "$HOME/Library/Containers/io.github.jayunewal.baton.probe" \
    2>/dev/null || true
  echo "Removed $app and $work"
  exit 0
fi

appstages=${*:-capture gamma sblock guard}

mkdir -p "$work" "$(dirname "$app")"

# The identity that signs the app is the one that a free Apple ID gives, when there is one, since
# that is what Baton can be signed with. Without one the app is signed ad hoc.
identity=${BATON_PROBE_IDENTITY:-}
if [ -z "$identity" ]; then
  identity=$(security find-identity -v -p codesigning 2>/dev/null |
    sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)
fi
[ -n "$identity" ] || identity=-

checksum() {
  sum=$(cat "$here/probe.swift" "$here/guard.swift" "$here/Info.plist" "$here/probe.entitlements" \
    "$here/guard.plist" "$here/guard.entitlements" "$here/guard-Info.plist" | shasum | cut -d' ' -f1)
  sum="$sum $1 $app"
}

build() {
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Library/LaunchAgents"
  swiftc -O -swift-version 5 -o "$exe" "$here/probe.swift"
  # The helper has an Info.plist of its own in its binary, since a sandboxed program is named by it.
  swiftc -O -swift-version 5 -o "$helper" "$here/guard.swift" \
    -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$here/guard-Info.plist"
  cp "$here/Info.plist" "$app/Contents/Info.plist"
  cp "$here/guard.plist" "$app/Contents/Library/LaunchAgents/$agent.plist"
  # The helper is signed first. It is sandboxed, since the system refuses to register an agent that
  # is not when the app is (macOS 14.2 and later). The app is sandboxed too.
  codesign --force --sign "$1" --identifier "$agent" --entitlements "$here/guard.entitlements" "$helper"
  codesign --force --sign "$1" --entitlements "$here/probe.entitlements" "$app"
  printf '%s\n' "$1" >"$work/.identity"
}

# A build that is there already is used again, since signing it again would take away what the
# person allowed it in System Settings when it is signed ad hoc.
checksum "$identity"
if [ ! -d "$app" ] || [ "$(cat "$work/.sum" 2>/dev/null || true)" != "$sum" ]; then
  echo "Building the checks, signed with: $identity"
  if ! build "$identity"; then
    echo "Signing with that identity failed, signing ad hoc instead" >&2
    identity=-
    checksum "$identity"
    build "$identity"
  fi
  printf '%s\n' "$sum" >"$work/.sum"
fi

now() { perl -MTime::HiRes=time -e 'printf "%.2f\n", time'; }

banner() {
  osascript -e "display notification \"$1 banner of the check\" with title \"Baton privacy check\"" || true
}

# Kills the agent as the system would find it after a crash, and tells how long the system takes to
# start another one.
kill_the_agent() {
  old=$(sed -n 's/^pid=//p' "$work/guard-heartbeat" 2>/dev/null || true)
  if [ -z "$old" ]; then
    echo "no agent was running to kill" >"$work/guard-kill.txt"
    return
  fi
  began=$(now)
  kill -9 "$old" 2>/dev/null || true
  new=
  n=0
  while [ "$n" -lt 160 ]; do
    new=$(sed -n 's/^pid=//p' "$work/guard-heartbeat" 2>/dev/null || true)
    [ -n "$new" ] && [ "$new" != "$old" ] && break
    new=
    sleep 0.25
    n=$((n + 1))
  done
  ended=$(now)
  if [ -n "$new" ]; then
    echo "agent $old killed, agent $new was writing again after $(perl -e "printf '%.2f', $ended - $began") s" \
      >"$work/guard-kill.txt"
  else
    echo "agent $old killed, no new agent within 40 s" >"$work/guard-kill.txt"
  fi
}

opener=
finish() {
  [ -n "$opener" ] && kill "$opener" 2>/dev/null || true
  pkill -f "$exe" 2>/dev/null || true
  pkill -f "$helper" 2>/dev/null || true
  launchctl bootout "gui/$(id -u)/$agent" 2>/dev/null || true
  "$exe" restore >/dev/null 2>&1 || true
}
trap finish EXIT INT TERM

cat <<'TEXT'

This covers the screens with black for up to 40 seconds, turns them black by their tables for 20
seconds, locks the Mac up to six times (unlock it each time), and, in the guard stage, adds a helper to
the login items of the person (it takes it away at the end). The first stage needs the app to be
allowed to record the screen, once: it says so if it is not, and then it is run again after the
person allowed it (sh run.sh capture). The stages of the first round hold the keys and the pointer
for 10 seconds at a time. Each of these ends by itself. Stay at the Mac, turn Focus off, and press
Escape three times to end a hold.

TEXT

rm -f "$work/report.txt" "$work/guard-heartbeat" "$work/guard-kill.txt" "$work/lease" \
  "$work/private-flag" "$work/lock-request" "$work/lock-result"

# One launch of the app for each stage, so that a stage that ends the app does not end the others.
for stage in $appstages; do
  rm -f "$work/phase" "$work/field-ready" "$work/field-stop" "$work/field-result"
  printf '%s\n' "$stage" >"$work/stages"

  open -n -W "$app" &
  opener=$!

  last=
  while kill -0 "$opener" 2>/dev/null; do
    phase=$(cat "$work/phase" 2>/dev/null || true)
    if [ "$phase" != "$last" ]; then
      last=$phase
      case "$phase" in
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
      guard-kill) kill_the_agent & ;;
      esac
    fi
    sleep 0.4
  done
  opener=
  [ "$(cat "$work/phase" 2>/dev/null || true)" = done ] ||
    echo "The app ended in the stage $stage without saying that it was done." | tee -a "$work/report.txt"
done

echo
echo "---- report ($work/report.txt) ----"
cat "$work/report.txt" 2>/dev/null ||
  echo "No report was written: the app did not start. Look at: log show --last 2m --predicate 'process == \"probe\"'"

# What the system logged about the sandbox and the background items of the checks, since a refusal is
# often only there.
{
  echo
  echo "---- what the system logged (sandbox denials, background items) ----"
  log show --last 20m --style compact --predicate \
    '(eventMessage CONTAINS "deny(" AND (eventMessage CONTAINS "probe" OR eventMessage CONTAINS "guard")) OR (subsystem == "com.apple.backgroundtaskmanagement" AND (eventMessage CONTAINS[c] "reject" OR eventMessage CONTAINS[c] "sandbox" OR eventMessage CONTAINS[c] "error"))' \
    2>&1 | grep -v "file-read-metadata" | cut -c1-300 | sort -u | head -80 || true
} | tee -a "$work/report.txt"
