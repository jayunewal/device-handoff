// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/always_on.dart';

/// Says what a test wants the system to say about opening Baton at sign-in, and notes what it was
/// asked to do.
class FakeAlwaysOn extends AlwaysOnKeeper {
  FakeAlwaysOn([this.opening = SignInOpening.off]);

  /// What `signIn` gives. `setSignIn` changes it.
  SignInOpening opening;

  /// What the system says to a request to turn it on, when it is turned on at all: `on`, or
  /// `needsApproval` where the person has to say yes in System Settings.
  SignInOpening onAnswer = SignInOpening.on;

  /// What `setSignIn` throws, if it should fail.
  AlwaysOnFailure? failure;

  /// What `setSignIn` was given, in order.
  final List<bool> sets = [];

  /// How many times Settings was asked to open, and whether it says that it did.
  int settingsOpened = 0;
  bool settingsOpen = true;

  /// How many times the app said that it is up.
  int readies = 0;

  @override
  Future<SignInOpening> signIn() async => opening;

  @override
  Future<SignInOpening> setSignIn(bool on) async {
    sets.add(on);
    final failure = this.failure;
    if (failure != null) throw failure;
    if (opening != SignInOpening.unavailable) {
      if (on && opening == SignInOpening.off) opening = onAnswer;
      if (!on && opening != SignInOpening.off) opening = SignInOpening.off;
    }
    return opening;
  }

  @override
  Future<bool> openSignInSettings() async {
    settingsOpened++;
    return settingsOpen;
  }

  @override
  Future<void> ready() async => readies++;
}
