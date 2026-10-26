// SPDX-License-Identifier: Apache-2.0

// The guard of the privacy mode (docs/adr/0011-privacy-mode.md): a small program that the app
// starts for each private session, and that locks the Mac if the app goes away without ending the
// session. What it reads from the app and when it locks is BatonGuard.swift, and how it locks is
// BatonLock.swift, both in apps/baton/rust_builder/macos/Classes. build-guard.sh builds them with
// this file into the program, and puts it into the app.
//
// It is signed to inherit the sandbox of the app that starts it (Guard.entitlements), and it runs
// nowhere else: started by hand it ends at once. Nothing is logged but that the Mac is locked and
// why.

import Foundation
import os

private let guardLog = Logger(subsystem: "io.github.jayunewal.baton", category: "guard")

exit(
  runGuardProgram(
    mac: MacLock(), log: { guardLog.notice("Baton guard: \($0, privacy: .public)") }))
