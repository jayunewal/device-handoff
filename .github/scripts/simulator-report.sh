#!/usr/bin/env bash
# Prints what the iPhone simulator knows about the app, for a job in which the app did not come up.
#
# An app that closes as it opens leaves the test waiting for a window that never comes. Its crash
# report and its log say why. So does the log of tccd, which asks the questions that the system
# puts in front of an app, such as the one about the camera (it writes the service and the app on
# lines of their own). The help of simctl lists the permissions that a simulator can be given in
# advance.
#
# The iPhone job runs it when it fails, and the guard around the test runs it when a start of the
# test hangs, because the next start may pass and nothing else would tell why the first did not.
# SIMULATOR_UDID names the simulator. Every command may fail, and none of them stops the others.
echo "== What the simulator logged about the app"
xcrun simctl list devices booted || true
ls -l ~/Library/Logs/DiagnosticReports || true
for report in ~/Library/Logs/DiagnosticReports/Runner*; do
  if [ -f "$report" ]; then echo "== $report"; head -c 8000 "$report"; fi
done
xcrun simctl spawn "$SIMULATOR_UDID" log show --last 30m --style compact --predicate 'process == "Runner"' | tail -n 100 || true
xcrun simctl spawn "$SIMULATOR_UDID" log show --last 30m --style compact --predicate 'process == "tccd" AND (eventMessage CONTAINS "kTCCServiceCamera" OR eventMessage CONTAINS "io.github.jayunewal.baton")' | tail -n 60 || true
xcrun simctl help privacy || true
