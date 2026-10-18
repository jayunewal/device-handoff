#!/usr/bin/env bash
# Says how much of the Rust build of an app job came out of the cache.
#
# Cargokit builds in apps/baton/build/cargokit (apps/baton/rust_builder/README.md says why) and the
# job keeps that folder from one run to the next. A library of a dependency that the cache had is
# older than the file that the "Set up Cargo" step touches, and one that this run had to build is
# newer. The libraries of this repository are built in every run, so they are left out. A number
# of built libraries close to the number of all of them means that the cache did not help.
#
# The answer is printed, and also written as a notice, which shows on the page of the run.
dir=apps/baton/build/cargokit
marker="$RUNNER_TEMP/cargo-set-up"
if [ ! -d "$dir" ] || [ ! -f "$marker" ]; then
  echo "There is no Rust build here to count."
  exit 0
fi
used=$(find "$dir" -name '*.rlib' ! -name 'libbaton_*' | wc -l | tr -d ' ')
built=$(find "$dir" -name '*.rlib' ! -name 'libbaton_*' -newer "$marker" | wc -l | tr -d ' ')
size=$(du -sh "$dir" | cut -f1)
echo "Rust dependencies built in this run: $built of $used (the rest came from the cache). The folder is $size."
echo "::notice title=Rust build::Dependencies built in this run: $built of $used. The folder is $size."
