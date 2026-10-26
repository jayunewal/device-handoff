#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Builds the checks for the privacy mode of a Mac into a throwaway app, signs it, and runs the stages
# with a person at the Mac (docs/research/privacy-mode/README.md).
#
#   sh run.sh                run the stages tap, secure, shield and cursor
#   sh run.sh shield tap     run these stages, in this order
#   sh run.sh guard          the helper that the system starts (SMAppService), and how it is started again
#   sh run.sh lock           three ways to lock the Mac, which lock it three times
#   sh run.sh clean          take away what a run left behind
#
# The app is sandboxed as the app of Baton is. It and this script leave notes for each other in
# /tmp/baton-probe. Everything it does to the Mac ends by itself, and this script undoes what a
# crash could leave when it ends. The app is built into /tmp/baton-probe, or into the folder in
# BATON_PROBE_APPS (the system may want the helper of the guard stage to be in an Applications
# folder).
set -eu

if [ "$(uname)" != Darwin ]; then
  echo "This runs on a Mac." >&2
  exit 1
fi

here=$(cd "$(dirname "$0")" && pwd)
work=/tmp/baton-probe
app=${BATON_PROBE_APPS:-$work}/BatonProbe.app
exe=$app/Contents/MacOS/probe
helper=$app/Contents/MacOS/guard
agent=io.github.jayunewal.baton.probe.guard

if [ "${1:-}" = clean ]; then
  pkill -f "$exe" 2>/dev/null || true
  pkill -f "$helper" 2>/dev/null || true
  launchctl bootout "gui/$(id -u)/$agent" 2>/dev/null || true
  "$exe" restore >/dev/null 2>&1 || true
  rm -rf "$app" "$work"
  echo "Removed $app and $work"
  exit 0
fi

stages=${*:-tap secure shield cursor}
appstages=
locking=
for stage in $stages; do
  if [ "$stage" = lock ]; then locking=yes; else appstages="$appstages $stage"; fi
done

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
    "$here/guard.plist" | shasum | cut -d' ' -f1)
  sum="$sum $1 $app"
}

build() {
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Library/LaunchAgents"
  swiftc -O -swift-version 5 -o "$exe" "$here/probe.swift"
  swiftc -O -swift-version 5 -o "$helper" "$here/guard.swift"
  cp "$here/Info.plist" "$app/Contents/Info.plist"
  cp "$here/guard.plist" "$app/Contents/Library/LaunchAgents/$agent.plist"
  # The helper is signed first and is not sandboxed. The app is sandboxed.
  codesign --force --sign "$1" --identifier "$agent" "$helper"
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
  "$exe" restore >/dev/null 2>&1 || true
}
trap finish EXIT INT TERM

if [ -n "$appstages" ]; then
  cat <<'TEXT'

This holds the keys and the pointer of this Mac for 10 seconds at a time, covers the screens with
black for up to 40 seconds, hides the pointer and turns the backlight down for 8 seconds, and, in the
guard stage, adds a helper to the login items of the person (it takes it away at the end). Each of
these ends by itself. Stay at the Mac, turn Focus off, and press Escape three times to end a hold.

TEXT

  rm -f "$work/phase" "$work/report.txt" "$work/field-ready" "$work/field-stop" "$work/field-result" \
    "$work/guard-heartbeat" "$work/guard-kill.txt" "$work/lease" "$work/private-flag"
  printf '%s\n' "$appstages" >"$work/stages"

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

  echo
  echo "---- report ($work/report.txt) ----"
  cat "$work/report.txt" 2>/dev/null ||
    echo "No report was written: the app did not start. Look at: log show --last 2m --predicate 'process == \"probe\"'"
fi

if [ -n "$locking" ]; then
  : >>"$work/report.txt"
  {
    echo
    echo "---- lock ----"
    echo "The setting of the Mac for the password after the screen locks or sleeps:"
    sysadminctl -screenLock status 2>&1 | head -3 || true
  } | tee -a "$work/report.txt"
  echo "The Mac will lock three times, ten seconds after each message. Unlock it each time."
  for method in sac cgsession displaysleep; do
    echo "Locking with: $method (in 10 seconds)"
    sleep 10
    "$helper" lock "$method" 2>&1 | tee -a "$work/report.txt" || true
  done
fi
