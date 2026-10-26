# Sound from a Mac: what has to be measured

The person asked for the sound of the Mac to play on the phone or the iPad and not on the Mac, as if an audio device had been attached to it. Whether a program can take the sound of the Mac and keep it from the speakers, with nothing installed outside the app and nothing paid for, was not known for macOS 27, and the design of [ADR 0012](../../adr/0012-sound.md) branches on it, so the first step was to measure.

The checks are small programs that are run on a Mac with a person at it, who listens. They are not part of the app and nothing in the app depends on them. This folder records the first one. The others (A2 to A7) are listed in the ADR and are run with the app itself.

## The first check (A1)

`probes/a1-audio-tap` is a tone program (`tonegen`) and an app bundle (`tapprobe`) that starts the tone program and taps it. The tone is a quiet 440 Hz sine with a short 1 kHz click every 500 ms. The bundle taps only the sound of its own tone program, with a Core Audio process tap, and stores no audio and changes no setting of the Mac.

The version in this folder is the one for the ear check. It plays the tone, and after 4 seconds it taps the tone program with the mute mode `muted` and reads the tap through a private aggregate device for 10 seconds, while a voice (the `say` command) tells the person what to expect. Then it destroys the tap. The tone goes on for about 15 seconds more, so the person can hear it return. The program writes what it measured to `tap-run.log` next to the bundle: the status of every call, the format of the tap, how many frames it read and how loud they were, whether the sample times had gaps, and the delay from the tone program rendering a click to the callback that held it. The first run, which is not kept here, did the same with `muted` and with `mutedWhenTapped`, and read each tap for 4 seconds without anyone listening for silence.

### Running it

```sh
cd docs/research/sound/probes/a1-audio-tap
swift build -c release
```

Make the bundle `BatonAudioProbe.app` with `Info.plist` in `Contents`, the binary `tapprobe` as `Contents/MacOS/BatonAudioProbe`, and `tonegen` beside it in `Contents/MacOS`. Sign it with an Apple Development identity (a free Apple ID gives one), and start it with `open`. The bundle needs the usage text for audio capture that `Info.plist` has, and the first run asks to allow it under Privacy & Security, Screen & System Audio Recording. A person has to stay at the Mac and listen. The entry that the bundle leaves under that setting can be removed when the check is done.

### What came out (macOS 27.0.1, a MacBook Pro with an M1 Pro, 26 October 2026)

| | `muted` | `mutedWhenTapped` |
| --- | --- | --- |
| Create the tap, the aggregate device and the IOProc, and start | every call returned 0 | every call returned 0 |
| Format of the tap | 48 kHz, 2 channels, 32-bit float | the same |
| Read | 191,488 frames in 374 callbacks of 512 frames (10.7 ms), RMS 0.0228 (the tone alone is about 0.021), no gaps in the sample times | the same |
| Delay from the tone program rendering a click to the callback that held it | median 23.7 ms (13.1 to 23.7) | median 25.0 ms (14.4 to 25.1) |
| The same, against the time at which the click was due at the speaker | median 11.6 ms after | median 12.9 ms after |

The delay includes up to one buffer of the tap (10.7 ms), because it is measured at the time of the callback. The tap gets the sound about when the speaker would have played it, and not before.

**The ear check.** The tone played for 4 seconds, then the tap with `muted` read it for 10 seconds while the voice said that the tone was being captured. The person heard the tone in the first part and only the voice in the second. During those 10 seconds the tap read the tone at full level: 479,232 frames, RMS 0.0228, all 20 clicks, no gaps, and a median delay of 21.0 ms (20.8 to 31.6). A second run gave the same capture (median 23.5 ms), and this time the person heard the tone return after the tap was destroyed. So a process tap can read the sound of a program and keep it off the speakers, with no driver, and the sound returns when the tap is destroyed.

### What else it found

- Without the System Audio Recording permission a tap does not fail. Every call returns 0 and what the tap reads is silence, so the app has to notice that itself.
- A command-line tool reads silence. The program has to be an app bundle with a usage text for audio capture (`NSAudioCaptureUsageDescription`).
- The assessment of the system (`spctl`) rejects a bundle that is signed ad hoc, so the bundle was signed with an Apple Development identity. Whether an ad hoc signature works with `open` was not tried.
- `open --stdout` failed with error -10810 on this system, and `open` alone works.

### Not checked

- That the speakers go silent, by an instrument. No call tells what reaches the hardware, so the check is by ear, and only the built-in speakers were listened to.
- A tap of every program except the app, which is what Baton would use. It was not run because it would also read whatever else is playing on the Mac.
- That the sound returns when the program that made the tap is killed, and not only when it destroys the tap.
- Whether a tap without the permission still keeps the sound from the speakers.
- The entitlement that a sandboxed app needs.
