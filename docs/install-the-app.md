# Install the Baton app

Baton is pre-alpha and is not in the App Store. You build the app on your Mac, and put it on an iPhone or iPad with a cable. A free Apple ID is enough: you do not need to join the Apple Developer Program. The price of the free Apple ID is that an app you install this way stops opening after seven days, and you install it again with the same command ([Every seven days](#every-seven-days)).

This version asks for a name for the device and shows the devices paired with it, which is none yet. The screens that pair devices and send links come next. Until then the [`baton` program](try-it.md) is the way to pair and send.

Everything below is typed in Terminal, which is a zsh shell on a Mac. Each command block can be pasted whole.

## What you need

- A Mac, and an iPhone or iPad with iOS or iPadOS 15 or newer, if you want the phone app. The Mac app needs macOS 12 or newer.
- [Xcode](https://apps.apple.com/app/xcode/id497799835), from the Mac App Store, new enough to know the iOS on your phone: Xcode says "Unsupported" for a phone with a newer iOS than it was made for. Open Xcode once, accept the license and let it download what it offers, including the iOS platform. Then run this once, to point the command line at it:

  ```sh
  sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
  sudo xcodebuild -runFirstLaunch
  ```

- [Flutter](https://docs.flutter.dev/get-started/install/macos) 3.47 or newer. `flutter --version` says which you have.
- Rust, installed with [rustup](https://rustup.rs). Rust from Homebrew will not do, because the build adds targets to it. Then add the targets for the phone and for the simulator, and make sure the toolchain is recent:

  ```sh
  rustup update stable
  rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin x86_64-apple-darwin
  ```

- CocoaPods, which Flutter uses to put the Rust library into the Xcode project. With [Homebrew](https://brew.sh):

  ```sh
  brew install cocoapods
  ```

Then ask Flutter what is still missing:

```sh
flutter doctor
```

It lists Android and Chrome too. Those can stay red: Baton's phone app is built for iOS first. Everything under "Xcode" and "CocoaPods" should be green.

## Get the code

```sh
git clone https://github.com/jayunewal/device-handoff.git
cd device-handoff/apps/baton
flutter pub get
```

The commands below are all run in `device-handoff/apps/baton`.

## The Mac app

Do this first. It builds the same app and the same Rust library as the phone, so if it works, your tools are in order, and nothing in it involves your Apple ID.

```sh
flutter build macos --release
open build/macos/Build/Products/Release/Baton.app
```

The first build takes a while, because it compiles the Rust library for both kinds of Mac. Later builds are faster.

If you only want to run the Mac app and not build anything, CI builds it on every change. Open the [CI runs](https://github.com/jayunewal/device-handoff/actions/workflows/ci.yml), pick the latest run on `main` with a green tick and download `baton-app-macos` under **Artifacts** at the bottom of the page. GitHub asks you to sign in to download. Unzip it, then unzip the `Baton-macos-app.zip` inside it, move `Baton.app` to your Applications folder and run:

```sh
xattr -dr com.apple.quarantine /Applications/Baton.app
```

macOS blocks a program that was downloaded and is not signed by a developer; that command is what tells it that you trust this one. Then open it. If macOS still refuses, open **System Settings**, then **Privacy & Security**, and click **Open Anyway** next to the message about Baton.

## The iPhone or iPad

### Sign in to Xcode with your Apple ID, once

1. Open Xcode, then **Xcode**, **Settings**, **Accounts**, and click **+** to add your Apple ID.
2. If Xcode says there is an agreement to accept, sign in once at [developer.apple.com/account](https://developer.apple.com/account) with the same Apple ID and accept it there.

### Connect the phone

1. Plug the phone into the Mac with a cable that carries data, and unlock it.
2. Tap **Trust** on the phone when it asks whether to trust this computer, and enter the phone's passcode.
3. On iOS or iPadOS 16 or newer, turn on Developer Mode: **Settings**, **Privacy & Security**, **Developer Mode**. The switch only appears after the phone has been connected to a Mac that has Xcode. The phone restarts; unlock it and confirm when it asks. On iOS 15 there is nothing to turn on.

### Tell the project who you are, once

Flutter has to set up the iOS project before Xcode can open it:

```sh
flutter build ios --config-only
open ios/Runner.xcworkspace
```

In Xcode:

1. Click **Runner** at the top of the list on the left, then the **Runner** target, then the **Signing & Capabilities** tab.
2. Tick **Automatically manage signing**.
3. Under **Team**, choose your name followed by **(Personal Team)**. Xcode makes the certificate and registers your phone; this is why the phone has to be plugged in.
4. If Xcode says the bundle identifier `io.github.jayunewal.baton` is not available, change it to one of your own, for example `com.example.yourname.baton`. The identifier has to be different from the identifier of any other app made with a free Apple ID.

You can then close Xcode. It writes your team into `ios/Runner.xcodeproj/project.pbxproj`, so Git shows that file as changed. That is expected. Before you update the code later, put the file back with `git checkout ios/Runner.xcodeproj/project.pbxproj`, and choose your team again afterwards.

### Build it and put it on the phone

With the phone still plugged in and unlocked:

```sh
flutter devices
flutter run --release --no-resident
```

`flutter devices` lists what Flutter can see, and the phone should be on the list. If you have more than one, put `-d` and then the phone's name or its id from that list after `--release`.

Build with `--release`. A debug build of a Flutter app does not open when you tap its icon on the phone, only while a computer is attached. `--no-resident` makes the command finish once the app is on the phone, instead of waiting for you.

The first time, your Mac may ask whether `codesign` can use a key in your keychain. Enter your Mac's password and choose **Always Allow**.

### Trust yourself on the phone, once

The first time, the command ends with an error, because the phone does not yet trust the developer of the app. That is expected: the app is on the phone all the same. Go to **Settings**, **General**, **VPN & Device Management**, tap your Apple ID under **Developer App**, and tap **Trust**. Then open Baton from the home screen.

### What you should see

1. The first time, a screen called **Name this device**. Choose a name you will recognise, such as "Kitchen iPad", and tap **Continue**.
2. A screen with your device's name, **Online**, a **Fingerprint** of five groups of four letters and digits, and **No other devices yet**. **Online** can take a few seconds to replace **Connecting…**.

If you see **Baton could not start** instead, the message under it says why. Open an [issue](https://github.com/jayunewal/device-handoff/issues) and paste that sentence into it.

## Every seven days

An app made with a free Apple ID stops opening after seven days. Its icon is still there, but it closes at once. To install it again, plug the phone in, unlock it and run, in `device-handoff/apps/baton`:

```sh
flutter run --release --no-resident
```

That installs over the old copy, so the app keeps what it had stored, such as its name and, once pairing is built, the devices it was paired with. If you delete the app from the phone, that is deleted with it.

To get a newer version of Baton first:

```sh
git checkout ios/Runner.xcodeproj/project.pbxproj
git pull
flutter pub get
flutter build ios --config-only
open ios/Runner.xcworkspace
```

Choose your team again as above, close Xcode, and run `flutter run --release --no-resident`.

## If something does not work

- **`Signing for "Runner" requires a development team`**: choose your Personal Team in Xcode as above. Plug the phone in before you open **Signing & Capabilities**, so that Xcode can register it.
- **`Failed to register bundle identifier`**, or a message about the number of apps or App IDs: change the bundle identifier as above. A free Apple ID can make only a few app identifiers a week and can have only a few apps of this kind on a phone, so delete another app you installed this way, or wait.
- **`Developer Mode disabled`**: turn it on as above.
- **The phone is listed as unsupported, or Xcode says it cannot use this iOS**: your Xcode is older than the iOS on the phone. Update macOS and Xcode.
- **`Untrusted Developer` when you open Baton, or Baton closes at once**: trust yourself as above. If it was working and stopped, seven days have passed: install it again as above.
- **`cargo` or `rustup` not found**: Xcode does not use your shell's search path when you build from its window. Build with `flutter run --release --no-resident` in Terminal, as above, and if it still happens, check that `rustup --version` works in the same Terminal.
- **`Unable to install` or `The code signature is invalid`**: unlock the phone, plug it in again and try once more. If it repeats, delete the Baton app from the phone and run `flutter run --release --no-resident` again. That also deletes what the app had stored.
- **Anything else**: open an [issue](https://github.com/jayunewal/device-handoff/issues) and paste the last 30 lines that Terminal printed.
