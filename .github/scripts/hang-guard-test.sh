#!/usr/bin/env bash
# Holds hang-guard.py to what the iPhone job needs of it. Run it from anywhere:
#
#   bash .github/scripts/hang-guard-test.sh
#
# CI runs it first, next to the test of the path rules. A guard that starts a test twice for no
# reason, or never when the test hangs, hides what the iPhone job is there to find.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failed=0

export STARTS="$work/starts"

# The guard, with a wait of half a second where the iPhone job allows minutes.
guard() {
  python3 "$here/hang-guard.py" --waiting "waiting for it" --finished "got it" --seconds 0.5 --grace 1 "$@"
}

# Counts the lines of a file, which is 0 when there is none.
lines() {
  if [ -f "$1" ]; then wc -l < "$1" | tr -d ' '; else echo 0; fi
}

# check "<what the case shows>" "<what the case gave>" "<what it should have given>"
check() {
  if [ "$2" != "$3" ]; then
    echo "FAILED: $1: got [$2], expected [$3]" >&2
    failed=1
  fi
}

# Each case below writes a script that notes every start of it in $STARTS.

# A command that gets there is left alone, and its output arrives as it was written.
rm -f "$STARTS"
cat > "$work/good.sh" <<'EOF'
echo x >> "$STARTS"
echo "first"
echo "waiting for it"
sleep 0.3
echo "got it"
echo "last" >&2
EOF
status=0
out=$(guard -- bash "$work/good.sh") || status=$?
check "a command that gets there: status" "$status" 0
check "a command that gets there: starts" "$(lines "$STARTS")" 1
check "a command that gets there: output" "$out" "$(printf 'first\nwaiting for it\ngot it\nlast')"

# A command that fails after it got there fails, and is not started again.
rm -f "$STARTS"
cat > "$work/bad.sh" <<'EOF'
echo x >> "$STARTS"
echo "waiting for it"
echo "got it"
exit 3
EOF
status=0
guard -- bash "$work/bad.sh" > /dev/null || status=$?
check "a command that fails: status" "$status" 3
check "a command that fails: starts" "$(lines "$STARTS")" 1

# A command that is slow without ever waiting at that line is not a hang.
rm -f "$STARTS"
cat > "$work/slow.sh" <<'EOF'
echo x >> "$STARTS"
sleep 1.2
EOF
status=0
guard -- bash "$work/slow.sh" > /dev/null || status=$?
check "a slow command: status" "$status" 0
check "a slow command: starts" "$(lines "$STARTS")" 1

# A hang is followed by a second start, and the cleanup runs between the two.
rm -f "$STARTS" "$work/cleaned"
cat > "$work/flaky.sh" <<'EOF'
echo x >> "$STARTS"
echo "waiting for it"
if [ "$(wc -l < "$STARTS" | tr -d ' ')" -lt 2 ]; then
  sleep 30
fi
echo "got it"
EOF
status=0
out=$(guard --cleanup "echo cleaned >> '$work/cleaned'" -- bash "$work/flaky.sh") || status=$?
check "a hang, then a second start: status" "$status" 0
check "a hang, then a second start: starts" "$(lines "$STARTS")" 2
check "a hang, then a second start: cleanups" "$(lines "$work/cleaned")" 1
case "$out" in
  *"::warning::Attempt 1 of 2 hung"*) ;;
  *) check "a hang, then a second start: the warning" "$out" "a line that starts with ::warning::Attempt 1 of 2 hung" ;;
esac

# A command that hangs every time ends with 124, after the starts that it was allowed.
rm -f "$STARTS"
cat > "$work/stuck.sh" <<'EOF'
echo x >> "$STARTS"
echo "waiting for it"
sleep 30
EOF
status=0
out=$(guard -- bash "$work/stuck.sh") || status=$?
check "a command that always hangs: status" "$status" 124
check "a command that always hangs: starts" "$(lines "$STARTS")" 2
case "$out" in
  *"::error::"*) ;;
  *) check "a command that always hangs: the error" "$out" "a line that starts with ::error::" ;;
esac

# What the command started goes with it, even a process that ignores the polite signal.
rm -f "$STARTS"
cat > "$work/tree.sh" <<'EOF'
echo x >> "$STARTS"
( trap '' TERM; exec sleep 71 ) &
sleep 72 &
echo "waiting for it"
wait
EOF
status=0
guard --attempts 1 -- bash "$work/tree.sh" > /dev/null || status=$?
check "a hung tree: status" "$status" 124
sleep 0.3
left=$(pgrep -f 'sleep 7[12]' | tr '\n' ' ' || true)
check "a hung tree: processes left" "$left" ""
pkill -f 'sleep 7[12]' 2> /dev/null || true

if [ "$failed" != 0 ]; then
  exit 1
fi
echo "hang-guard: every case behaved as it should"
