#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Runs a command, and runs it once more if it hangs at a line that it should soon leave.

The iPhone job starts the app in a simulator with `flutter test integration_test`. Once the app
is running, Flutter prints "Waiting for VM Service port to be available..." and, a few seconds
later, "VM Service URL on device: ...". Now and then the second line never comes, and nothing
else does, until the 25 minutes of the step are over and someone starts the whole job again
(3 of the 23 iPhone jobs that ran to the end between 2026-10-14 and 2026-10-18). A second start
of the same command is nearly always enough.

    hang-guard.py --waiting LINE --finished LINE --seconds N [--attempts N] [--grace N]
                  [--cleanup SHELL-COMMAND] [--silences N] -- COMMAND [ARGUMENT...]

The output of the command is passed on as it comes. When a line with --waiting in it has been
printed, and no line with --finished in it follows within --seconds, everything that the command
started is stopped, the --cleanup command runs, and the command starts again, up to --attempts
times in all (two unless told otherwise). Only that hang starts it again: a command that ends,
whatever its status, is not run twice, so a test that fails is still a failure.

With --silences, the end of each start also lists the stretches of that many seconds or more in
which the command printed nothing, with the line on each side, and says how long the wait for the
--finished line took. A build of an app says nothing for minutes, and the log of a long step is
too long to read from its top, so this is where the time that a start took shows.

The status is the one of the command, or 124 (as `timeout` has it) when every attempt hung.
"""

import argparse
import os
import signal
import subprocess
import sys
import threading
import time

HUNG = 124
# How often the output is looked at while the command runs.
POLL = 0.2
# How many stretches without output the report of --silences lists.
MOST_SILENCES = 12
# How long the --cleanup command may take. The iPhone job prints what the simulator logged, which
# takes some time.
CLEANUP_SECONDS = 180


def stop(process, grace):
    """Ends the command and everything that it started, which share its process group."""
    group = process.pid
    try:
        os.killpg(group, signal.SIGTERM)
    except ProcessLookupError:
        return
    deadline = time.monotonic() + grace
    while process.poll() is None and time.monotonic() < deadline:
        time.sleep(POLL)
    # Whatever ignored the polite signal, or outlived the command, goes now.
    try:
        os.killpg(group, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def shorten(line, missing):
    """A line of output for the report: one line, and not too long. None is the start or the end."""
    if line is None:
        return missing
    line = line.strip()
    return line if len(line) <= 120 else line[:117] + "..."


def say_silences(quiet, minimum):
    """Lists the longest stretches in which the command printed nothing."""
    if not quiet:
        print(f"No stretch of {minimum:g} seconds or more without output in this start.", flush=True)
        return
    print(f"Stretches of {minimum:g} seconds or more without output in this start, longest first:", flush=True)
    longest = sorted(quiet, key=lambda stretch: stretch[1] - stretch[0], reverse=True)
    for began, ended, before, after in longest[:MOST_SILENCES]:
        print(f"  {ended - began:7.1f} s, from {began:7.1f} s to {ended:7.1f} s after the start", flush=True)
        print(f"      the line before it: {shorten(before, '(the start)')}", flush=True)
        print(f"      the line after it:  {shorten(after, '(the end)')}", flush=True)


def run_once(command, waiting, finished, seconds, grace, silences=None):
    """Runs the command once, and says whether it hung, with the status it ended with."""
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        # A process group of its own, so that one signal reaches the whole tree.
        start_new_session=True,
    )
    began = time.monotonic()
    seen = {"waiting_since": None, "finished_at": None}
    # The stretches without output: when each began and ended (in seconds after the start), and
    # the lines on both sides of it.
    quiet = []
    last = {"at": began, "line": None}

    def pass_on():
        for line in iter(process.stdout.readline, b""):
            now = time.monotonic()
            sys.stdout.buffer.write(line)
            sys.stdout.buffer.flush()
            text = line.decode("utf-8", errors="replace")
            if silences is not None and now - last["at"] >= silences:
                quiet.append((last["at"] - began, now - began, last["line"], text))
            last["at"], last["line"] = now, text
            if seen["waiting_since"] is None and waiting in text:
                seen["waiting_since"] = now
            if seen["finished_at"] is None and finished in text:
                seen["finished_at"] = now

    reader = threading.Thread(target=pass_on, daemon=True)
    reader.start()
    hung = False
    while process.poll() is None:
        time.sleep(POLL)
        since = seen["waiting_since"]
        if since is not None and seen["finished_at"] is None and time.monotonic() - since > seconds:
            hung = True
            stop(process, grace)
            break
    # A process that got away with the pipe must not keep this one waiting for the end of it.
    reader.join(timeout=10)
    if silences is not None:
        ended = time.monotonic()
        if ended - last["at"] >= silences:
            quiet.append((last["at"] - began, ended - began, last["line"], None))
        say_silences(quiet, silences)
        if seen["waiting_since"] is not None and seen["finished_at"] is not None:
            delay = seen["finished_at"] - seen["waiting_since"]
            print(f'"{finished}" came {delay:.1f} seconds after "{waiting}".', flush=True)
    return hung, process.returncode


def main():
    if "--" not in sys.argv:
        sys.exit("usage: hang-guard.py [options] -- COMMAND [ARGUMENT...]")
    split = sys.argv.index("--")
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--waiting", required=True, help="the line of output that starts the wait")
    parser.add_argument("--finished", required=True, help="the line of output that ends the wait")
    parser.add_argument("--seconds", type=float, required=True, help="how long the wait may last")
    parser.add_argument("--attempts", type=int, default=2, help="how many times the command may start")
    parser.add_argument("--grace", type=float, default=10, help="seconds between the two signals that stop it")
    parser.add_argument("--cleanup", help="a shell command to run after a hang, before the next attempt")
    parser.add_argument(
        "--silences",
        type=float,
        help="list the stretches of this many seconds or more without output at the end of each start",
    )
    args = parser.parse_args(sys.argv[1:split])
    command = sys.argv[split + 1 :]
    if not command:
        parser.error("there is no command after --")

    for attempt in range(1, args.attempts + 1):
        hung, status = run_once(command, args.waiting, args.finished, args.seconds, args.grace, args.silences)
        if not hung:
            return status
        print(
            f"::warning::Attempt {attempt} of {args.attempts} hung: {args.seconds:g} seconds after "
            f'"{args.waiting}" there was still no "{args.finished}".',
            flush=True,
        )
        if args.cleanup:
            try:
                subprocess.run(args.cleanup, shell=True, check=False, timeout=CLEANUP_SECONDS)
            except subprocess.TimeoutExpired:
                print(f"::warning::The cleanup command did not end within {CLEANUP_SECONDS} seconds.", flush=True)
    print(f"::error::The command hung every time it started ({args.attempts}).", flush=True)
    return HUNG


if __name__ == "__main__":
    sys.exit(main())
