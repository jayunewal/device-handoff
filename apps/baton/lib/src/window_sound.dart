// SPDX-License-Identifier: Apache-2.0

import 'backend.dart';
import 'format.dart';

/// Where the sound of a screen that is shown here has got to, for the person.
enum SoundStage {
  /// The person has not asked for sound, or has turned it off.
  off,

  /// The person asked, and the other device has not answered yet.
  asking,

  /// The other device sends its sound, and it plays here.
  playing,

  /// The person asked, and the sound cannot be had: the other device says that it sends none, or
  /// this device cannot play it. [ScreenSound.why] says which, and why.
  silent,
}

/// How the sound of a screen stands: where it has got to, and, when the other device sends none,
/// why.
typedef ScreenSound = ({SoundStage stage, WindowSound? why});

/// The sound of a screen the person has not asked for.
const ScreenSound soundOff = (stage: SoundStage.off, why: null);

/// Why there is no sound of the screen of the other device, in a sentence for the person, or `null`
/// for a state in which there is nothing to explain: the other device sends none, or this device
/// cannot play it. [device] is the name of the other device, which came from it.
String? describeSilence(WindowSound sound, String device) {
  final name = visibleText(device);
  return switch (sound) {
    WindowSound.playing || WindowSound.off => null,
    WindowSound.window =>
      'Sound goes with a whole screen, and this is a window on $name.',
    WindowSound.noPermission =>
      '$name has not let Baton record its sound. Someone at $name can allow '
          'it in the settings of that device. Then tap the speaker to try '
          'again.',
    WindowSound.failed =>
      '$name could not record its sound. Tap the speaker to try again.',
    WindowSound.other =>
      '$name cannot send its sound now. Tap the speaker to try again.',
    WindowSound.cannotPlay =>
      'This device could not play the sound of $name. Tap the speaker to try '
          'again.',
  };
}
