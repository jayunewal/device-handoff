// SPDX-License-Identifier: Apache-2.0

// The words of the unlock (docs/adr/0013-unlock-from-a-phone.md), in one place: what the person at
// the device that unlocks is told while it goes and when it ends, and what the person at the
// device that is unlocked is told of an attempt. The names come from other devices, so each goes
// through `visibleText`. Every sentence says what happened, and the ones that end the set-up say
// where to set it up again.

import 'backend.dart';
import 'format.dart';
import 'notice_access.dart';

/// Where an unlock that the person here asked for has got to, for the screen that shows the device
/// that is locked.
enum UnlockStage {
  /// Nothing is going on. A screen that is locked offers the button.
  idle,

  /// The person tapped Unlock and the other device has not sent its question yet.
  starting,

  /// This device asks for the face of the person.
  asking,

  /// The face was recognised and the key went to the other device, which types its password.
  typing,

  /// It is over, and [ScreenUnlock.result] says how.
  ended,
}

/// How an unlock stands: where it has got to, and, when it is over, how it ended.
typedef ScreenUnlock = ({UnlockStage stage, UnlockResult? result});

/// An unlock that nobody asked for.
const ScreenUnlock unlockIdle = (stage: UnlockStage.idle, result: null);

/// What the person at the device that unlocks is told while an unlock goes, or `null` while
/// nothing does and when it is over ([describeUnlockEnd] says how it ended). [device] is the name
/// of the device that is unlocked.
String? describeUnlocking(UnlockStage stage, String device) {
  final name = visibleText(device);
  return switch (stage) {
    UnlockStage.idle || UnlockStage.ended => null,
    UnlockStage.starting => 'Asking $name to unlock…',
    UnlockStage.asking => 'Waiting for Face ID or Touch ID…',
    UnlockStage.typing => '$name is typing its password…',
  };
}

/// How an unlock that the person asked for ended, in a sentence for them. [device] is the name of
/// the device that was to be unlocked. A result that ends the set-up says where to set it up
/// again, since nothing at this device can.
String describeUnlockEnd(UnlockResult result, String device) {
  final name = visibleText(device);
  final again = 'Set it up again in Baton on $name.';
  return switch (result) {
    UnlockResult.unlocked => '$name is unlocked.',
    UnlockResult.notSetUp =>
      'The unlock is not set up on $name for this device, or it ended. $again',
    UnlockResult.notLocked => '$name is not locked, so nothing was typed.',
    UnlockResult.notPrivate =>
      'Nothing was typed. The screen, keyboard or trackpad of $name were not '
          'kept private, which an unlock needs. Switch off what is switched on '
          'in the settings for this session, then try again.',
    UnlockResult.busy => '$name was busy with another unlock. Try again.',
    UnlockResult.stale =>
      'What $name kept did not open with the key from this device, so nothing '
          'was typed. $again',
    UnlockResult.rejected =>
      '$name did not accept the password it typed. If the password of $name '
          'changed, set it up again in Baton on $name.',
    UnlockResult.timedOut =>
      '$name waited for this device and gave up. Try again.',
    UnlockResult.failed =>
      '$name could not unlock itself. Try again, or unlock it at $name.',
    UnlockResult.other =>
      '$name answered in a way that this version of Baton does not know. Look '
          'at the screen, and try again if it is still locked.',
    UnlockResult.declined =>
      'Not unlocked. This device did not confirm that it was you, or you '
          'cancelled. Try again.',
    UnlockResult.keyLost =>
      'This device can no longer open the unlock for $name, because a face or '
          'fingerprint was added or removed, or the device was restored. $again',
    UnlockResult.gaveUp =>
      'This device could not ask for your face. Try again.',
    UnlockResult.noQuestion => '$name did not ask in time. Try again.',
    UnlockResult.noResult =>
      'The key was sent, and $name did not say what came of it. Look at the '
          'screen: it may be unlocked.',
    UnlockResult.sessionEnded =>
      'The session ended before $name could be unlocked.',
  };
}

/// Whether a person who has read how an unlock ended can try it again with the same set-up: the
/// ones that end the set-up, and the one that says the screen was not locked, are not helped by
/// another tap.
bool canTryUnlockAgain(UnlockResult result) => switch (result) {
  UnlockResult.unlocked ||
  UnlockResult.notSetUp ||
  UnlockResult.notLocked ||
  UnlockResult.stale ||
  UnlockResult.rejected ||
  UnlockResult.keyLost ||
  UnlockResult.sessionEnded => false,
  UnlockResult.notPrivate ||
  UnlockResult.busy ||
  UnlockResult.timedOut ||
  UnlockResult.failed ||
  UnlockResult.other ||
  UnlockResult.declined ||
  UnlockResult.gaveUp ||
  UnlockResult.noQuestion ||
  UnlockResult.noResult => true,
};

/// An attempt to unlock this device, in a sentence for the person here. The name is the one the
/// person gave the other device.
String describeAttempt(UnlockEntry entry) {
  final name = visibleText(entry.deviceName);
  final again = 'Set the unlock up again.';
  return switch (entry.result) {
    UnlockResult.unlocked => '$name unlocked this device.',
    UnlockResult.notSetUp =>
      '$name tried to unlock this device, and no unlock was set up for it. '
          'Nothing was typed.',
    UnlockResult.notLocked =>
      '$name tried to unlock this device while it was not locked. Nothing was '
          'typed.',
    UnlockResult.notPrivate =>
      '$name tried to unlock this device, and its screen, keyboard or '
          'trackpad were not kept private. Nothing was typed.',
    UnlockResult.busy =>
      '$name tried to unlock this device while another unlock was going on.',
    UnlockResult.stale =>
      '$name tried to unlock this device, and what it sent did not open the '
          'stored password. Nothing was typed. $again',
    UnlockResult.rejected =>
      'The password was typed for $name and this device did not accept it. '
          '$again',
    UnlockResult.timedOut => '$name did not answer in time. Nothing was typed.',
    UnlockResult.failed => 'This device could not unlock itself for $name.',
    UnlockResult.other =>
      '$name tried to unlock this device, and it ended in a way that this '
          'version of Baton does not know.',
    UnlockResult.declined =>
      '$name asked, and the person there did not confirm: their face was not '
          'recognised, or they cancelled. Nothing was typed.',
    UnlockResult.keyLost =>
      '$name no longer has the key for the unlock, so nothing was typed. '
          '$again',
    UnlockResult.gaveUp => '$name gave up. Nothing was typed.',
    UnlockResult.noQuestion =>
      '$name asked, and this device did not send its question in time. '
          'Nothing was typed.',
    UnlockResult.noResult =>
      '$name sent its key, and this device did not say what came of it.',
    UnlockResult.sessionEnded =>
      'The session ended while this device waited for $name. Nothing was '
          'typed.',
  };
}

/// What the person is told when the system will not show the notifications that tell them of an
/// attempt to unlock this device, or `null` when it will, or will ask. [computer] names this
/// device: "Mac".
String? describeNoticeAccess(NoticeAccess access, String computer) =>
    switch (access) {
      NoticeAccess.allowed || NoticeAccess.needed => null,
      NoticeAccess.denied =>
        'Notifications are off for Baton, so it cannot tell you when this '
            '$computer is unlocked. Allow them in System Settings, under '
            'Notifications.',
      NoticeAccess.unavailable =>
        'This $computer cannot show notifications for Baton, so look at the '
            'recent attempts to see when it is unlocked.',
    };

/// What a notification says of an attempt to unlock this device that [UnlockEntry.notify] says to
/// tell the person of: a short title, and a body that says what to do about it.
({String title, String body}) describeNotice(UnlockEntry entry) {
  final name = visibleText(entry.deviceName);
  if (entry.result == UnlockResult.unlocked) {
    return (
      title: 'This device was unlocked from $name',
      body:
          'If that was not you, forget $name in Baton and change the password '
          'of this device.',
    );
  }
  return (
    title: 'Unlocking this device from $name did not work',
    body: describeAttempt(entry),
  );
}
