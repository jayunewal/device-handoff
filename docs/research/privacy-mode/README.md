# Privacy mode on a Mac: what has to be measured

Privacy mode is for a person who uses their own Mac from a phone or a tablet while other people are at home. While the session lasts the screen of the Mac shows nothing and gives no light, nobody at the Mac can use its keys or its pointer, and when the session ends or the connection is lost the Mac is locked again. The sourced facts that the design starts from are in the notes of the project (the lock screen takes posted keys, a sandboxed app cannot use the Accessibility interface, secure input blinds event taps, nothing can be captured while a display sleeps). None of them was tested on macOS 27, and the design branches on some of them, so the first step is to measure.

The checks here are a small app, built and run by `probes/run.sh` on a Mac with a person at it. It is sandboxed as the app of Baton is, so that what it finds is what the app can do. It asks the person what it cannot see for itself. It is not part of the app and nothing in the app depends on it.

## Running the checks

```sh
sh docs/research/privacy-mode/probes/run.sh              # every stage
sh docs/research/privacy-mode/probes/run.sh shield tap   # these stages, in this order
sh docs/research/privacy-mode/probes/run.sh clean        # take away what a run left
```

The first run asks to allow the app under Privacy & Security (Input Monitoring and Accessibility), once the tap cannot be made without. The app is signed with the identity that a free Apple ID gives when the Mac has one, and ad hoc when it does not; the report says which. A person has to stay at the Mac, and Focus has to be off for the banners. Each hold of the keys and the pointer lasts ten seconds, a cover of black lasts at most forty, and Escape pressed three times ends a hold. Everything ends by itself, and `run.sh` puts the backlight and the pointer back if the app dies. It prints the report at the end, and leaves it in `/tmp/baton-probe/report.txt`.

## What each stage settles

| Stage | What it does | What the design takes from it |
| --- | --- | --- |
| `tap` | Makes an active event tap in the sandbox, holds the keys and the pointer for ten seconds while a text field has the keyboard, posts its own pointer moves through the tap, and reads the counters of the system before and during | Whether the block can live in the app, or needs a helper outside the sandbox. Whether the moves that the app posts pass its own tap. Whether the counters of the system can tell the person from the app (the tripwire for input that the tap did not see) |
| `secure` | The same with a password field, in another process, that has the keyboard | Whether the tap is blind to keys then, as the guidance for secure input says, whether the keys still reach the field, and whether the counters see them |
| `shield` | Covers every screen with black at the shielding level and lists the windows of other programs that stay above or at that level while the person tries Spotlight, Mission Control, the edges of the screen and two banners | Whether the cover is enough, or whether the keys that open these are also held, and which windows could show the activity of the remote person |
| `cursor` | Hides the pointer over the cover while it moves, with the plain call and with the call that lets a program that is not in front do it, and turns the backlight to zero and back | Whether the pointer can be hidden with public calls, and whether the backlight can be turned off and restored from the sandbox |

Not here yet, and next: a helper that the system starts for the person (`SMAppService`) with the identity of a free Apple ID, the time it takes to be started again after it is killed, how the app and the helper talk, and the ways to lock the Mac that need no consent.

## What came out

Not run yet.
