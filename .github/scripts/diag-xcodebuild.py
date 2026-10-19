#!/usr/bin/env python3
# Measuring only; not for main.
"""Runs a command and prints the lines of its output that mark a task, with the second they came."""

import re
import subprocess
import sys
import time

# The start of a task of Xcode, a line of Cargo, and then everything after the timing summary.
TASK = re.compile(
    r"^(PhaseScriptExecution|Ld|CompileC|CompileSwift|SwiftCompile|SwiftDriver|CodeSign|Libtool|"
    r"ProcessInfoPlistFile|CopySwiftLibs|GenerateDSYMFile|Strip|\*\* )"
)
CARGO = re.compile(r"^\s+(Compiling|Finished|Fresh|Running|Blocking|Updating|Downloaded)\b")


def main():
    start = time.monotonic()
    process = subprocess.Popen(
        sys.argv[1:], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", bufsize=1
    )
    summary = False
    for line in process.stdout:
        line = line.rstrip("\n")
        if "Build Timing Summary" in line:
            summary = True
        if summary or TASK.match(line) or CARGO.match(line):
            print(f"{time.monotonic() - start:7.1f} s  {line[:170]}", flush=True)
    status = process.wait()
    print(f"{time.monotonic() - start:7.1f} s  ended with status {status}", flush=True)


if __name__ == "__main__":
    main()
