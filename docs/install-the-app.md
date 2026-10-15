# Install the Baton app

Baton is pre-alpha and is not in the App Store. You build the app on your Mac, and put it on an iPhone or iPad with a cable. A free Apple ID is enough: you do not need to join the Apple Developer Program. The price of the free Apple ID is that an app you install this way stops opening after seven days, and you install it again with the same command ([Every seven days](#every-seven-days)).

This version asks for a name for the device, shows the devices paired with it, pairs two devices (one shows a QR code and the other scans it, or has it pasted in; [Pair two devices](#pair-two-devices)), and sends links and text between them ([Send a link or some text](#send-a-link-or-some-text)). Files are not in the app yet, and neither is a Baton that receives while the app is closed; the [`baton` program](try-it.md) is the one that can send the clipboard and run in the background of a computer.

Everything below is typed in Terminal, which is a zsh shell on a Mac. Each command block can be pasted whole.

## What you need

- A Mac, and an iPhone or iPad with iOS or iPadOS 15 or newer, if you want the phone app. The Mac app needs macOS 12 or newer.
- [Xcode](https://apps.apple.com/app/xcode/id497799835), from the Mac App Store, new enough to know the iOS on your phone: Xcode says "Unsupported" for a phone with a newer iOS than it was made for. Open Xcode once, accept the license and let it download what it offers, including the iOS platform. Then run this once, to point the command line at it:

  ```sh
  sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
  sudo xcodebuild -runFirstLaunch
  ```

- [Flutter](https://docs.flutter.dev/get-started/install/macos) 3.47 or newer. `flutter --version` says which you have. If it is older and you installed Flutter with Homebrew, this brings it up to date, and the second command checks it. If you installed Flutter in another way, `flutter upgrade` does the same:

  ```sh
  brew upgrade --cask flutter
  flutter --version
  ```

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

The first build takes a while, because it compiles the Rust library for both kinds of Mac. Later builds are faster. Flutter may warn that the `baton_ffi` plugin does not support Swift Package Manager, and that this will become an error in a future version of Flutter. It is only a warning in Flutter 3.47, and the build goes on.

The first time Baton opens, macOS may ask whether it may find devices on your local network. Choose **Allow**.

If you only want to run the Mac app and not build anything, CI builds it for every change that is merged into `main`. Open the [CI runs](https://github.com/jayunewal/device-handoff/actions/workflows/ci.yml), pick the latest run on `main` with a green tick and download `baton-app-macos` under **Artifacts** at the bottom of the page. GitHub asks you to sign in to download. Unzip it, then unzip the `Baton-macos-app.zip` inside it, move `Baton.app` to your Applications folder and run:

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
flutter build ios --config-only --no-codesign
open ios/Runner.xcworkspace
```

Flutter prints a warning that it is building for a device with code signing disabled, and that you will have to sign the app yourself. That is expected: Xcode signs the app in the next step, once you have chosen your team. The flag is there because a Mac that has never signed an app has no certificate yet, and without the flag Flutter stops with `No development certificates available to code sign app for device deployment`. Xcode makes the certificate when you choose your team.

In Xcode:

1. Click **Runner** at the top of the list on the left, then the **Runner** target, then the **Signing & Capabilities** tab.
2. Tick **Automatically manage signing**.
3. Under **Team**, choose your name followed by **(Personal Team)**. Xcode makes the certificate and registers your phone; this is why the phone has to be plugged in.
4. If Xcode says the bundle identifier `io.github.jayunewal.baton` is not available, change it to one of your own, for example `com.example.yourname.baton`. The identifier has to be different from the identifier of any other app made with a free Apple ID.

You can then close Xcode. Building the app and choosing your team change a few files under `ios/` and `macos/` (Flutter and CocoaPods add their settings, and Xcode adds your team), so Git shows them as changed. That is expected. Before you update the code later, put them all back with `git checkout -- ios macos`, then choose your team again, and your own bundle identifier if you changed it.

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
2. A screen with your device's name, **Online**, a **Fingerprint** of five groups of four letters and digits, **No other devices yet**, and an **Add a device** button. **Online** can take a few seconds to replace **Connecting…**. At the bottom are two tabs, **Devices** and **Inbox**.

If your Mac or phone asks whether Baton may find devices on your local network, choose **Allow**. Baton finds your other devices that way. If you chose **Don't Allow**, change it in **Settings** (**System Settings** on the Mac), then **Privacy & Security**, then **Local Network**.

If you see **Baton could not start** instead, the message under it says why. **Show details** opens a page with more to paste into an [issue](https://github.com/jayunewal/device-handoff/issues), described under **Anything else** below.

## Pair two devices

You need Baton on two devices, for example the Mac and the phone, and open on both. Put them on the same Wi-Fi network. Baton can also reach a device on another network by way of a relay, but that has not been tried from the app yet.

In the steps below, the first device shows the code and the second one reads it. It does not matter which is which, but with a Mac and a phone, show the code on the Mac and read it with the phone. A phone's camera focuses on a screen held close, which the camera of many Macs cannot do, and some Macs have no camera at all.

1. On the first device, tap **Add a device**, then **Show a code**. It shows a QR code, and for how long the code works: a few minutes.
2. On the second device, tap **Add a device**, then **Scan or enter a code**. The first time, it asks whether Baton may use the camera: choose **OK**. Point the camera at the QR code on the first device.
3. The second device now shows six digits, and the first one shows the same six, with the name of the second device. If the digits are the same, tap **Yes, they match** on the first device. If they are not, tap **No, cancel**: someone else may be trying to pair.
4. Both devices say **Paired with** and the name of the other. Tap **Done**. Each now lists the other.

A code pairs one device, once. If it ran out, or it did not work, tap **Show a new code**.

While a phone or an iPad shows a code, reads one or waits to be asked about one, Baton keeps its screen on, so that it does not lock itself while you are looking at the other device. It lets Auto-Lock work as you set it again as soon as the pairing is over, you leave the screen, or five minutes pass with nothing happening on it, for example a camera that waits for a code that nobody holds up. Each step of the pairing, such as a code that is read or a question that is answered, keeps the screen on for five minutes more.

If the second device cannot scan the code, for example a Mac with no camera or a phone that was not allowed to use it, tap **Copy the code as text** on the first device, and paste the text into the box under **Or paste the code** on the second, and tap **Connect**. To get the text from one device to the other, copy it on one and paste it on the other. A Mac and an iPhone share their clipboard (Apple calls it Universal Clipboard) when they use the same Apple ID, Wi-Fi and Bluetooth are turned on on both, Handoff is turned on on both (search for **Handoff** in **System Settings** on the Mac and in **Settings** on the phone), and they are close to each other. If what you paste is not what you copied, send the text to yourself in Messages or Notes instead.

To forget a device, tap the three dots at the end of its row, then **Forget this device**, then **Forget**. That is done on one device: the other can no longer reach it, but still lists it until you forget it there too.

## Send a link or some text

You need two paired devices, with Baton open on both. They should be on the same Wi-Fi network.

1. On the device you send from, tap the other device's row on the **Devices** tab, or the paper plane at the end of it. A screen called **Send to** and the device's name opens.
2. Type or paste a link or some text. **Paste** puts what is on the clipboard into the box. Under the box Baton says how it will go: a web address on its own goes as a link, which the other device can open, and anything else goes as text, which the other device can copy. Text arrives exactly as you wrote it.
3. Tap **Send link** or **Send text**. Baton says **Sent the link to** or **Sent the text to** and the device's name, and empties the box. If it did not work, it says why and leaves what you typed, so that you can try again.
4. On the other device a notice appears at the bottom, such as "Kitchen iPad sent you a link", with a **View** button, and the **Inbox** tab shows how many items are new. The notice says who sent what kind of thing, never what it says: it could be a password, and a notice can be seen by anyone who is looking at the screen. Tap **View**, or the **Inbox** tab.

The inbox lists what other devices sent, newest first, and keeps it until you remove it, also after you close the app. On a link, **Open** opens it in the web browser, **Copy** copies it, and the bin removes it. Text can be copied and removed. Long text is cut short until you tap **Show more**. The icon at the top of the inbox clears all of it, after asking.

Baton opens only web addresses, the ones that start with `http://` or `https://`. A link of another kind, such as a phone number, a file or an app, is shown and can be copied, but has no **Open** button: opening it could start something you did not ask for. Characters that cannot be seen, or that turn text round, show as a replacement mark (�) in the inbox, so that a line cannot read as one address and be copied as another. **Copy** copies what was sent, as it was sent.

**Check the connection** on the send screen asks the other device to answer, and says how long it took and whether the traffic went over your Wi-Fi, directly over the internet or through the relay.

In this version a device receives only while Baton is running on it. On an iPhone or iPad that means open on the screen: a locked phone, or one that has Baton in the background, cannot be reached, and a send to it ends after a while with a message that says so. Receiving on a locked phone needs a push notification, which is not built yet.

Until then, keep Baton open and the phone in your hand when you send to it, or lengthen **Auto-Lock** while you try Baton (**Settings**, **Display & Brightness**, **Auto-Lock**) and set it back afterwards. If a send to a phone keeps ending with **Could not reach**, check **Low Power Mode** first: it sets Auto-Lock to 30 seconds. Baton keeps the screen on only while two devices pair (see **Pair two devices**). At any other time Auto-Lock works as you set it, because a phone that never locks is the opposite of what Baton is for.

## Every seven days

An app made with a free Apple ID stops opening after seven days. Its icon is still there, but it closes at once. To install it again, plug the phone in, unlock it and run, in `device-handoff/apps/baton`:

```sh
flutter run --release --no-resident
```

That installs over the old copy, so the app keeps what it had stored, such as its name, the devices it was paired with and its inbox. If you delete the app from the phone, that is deleted with it.

To get a newer version of Baton first:

```sh
git checkout -- ios macos
git pull
flutter pub get
flutter build ios --config-only --no-codesign
open ios/Runner.xcworkspace
```

Choose your team again as above (and your own bundle identifier, if you changed it), close Xcode, and run `flutter run --release --no-resident`.

## If something does not work

- **`Signing for "Runner" requires a development team`**: choose your Personal Team in Xcode as above. Plug the phone in before you open **Signing & Capabilities**, so that Xcode can register it.
- **`Failed to register bundle identifier`**, or a message about the number of apps or App IDs: change the bundle identifier as above. A free Apple ID can make only a few app identifiers a week and can have only a few apps of this kind on a phone, so delete another app you installed this way, or wait.
- **`Developer Mode disabled`**: turn it on as above.
- **The phone is listed as unsupported, or Xcode says it cannot use this iOS**: your Xcode is older than the iOS on the phone. Update macOS and Xcode.
- **`Untrusted Developer` when you open Baton, or Baton closes at once**: trust yourself as above. If it was working and stopped, seven days have passed: install it again as above.
- **`cargo` or `rustup` not found**: Xcode does not use your shell's search path when you build from its window. Build with `flutter run --release --no-resident` in Terminal, as above, and if it still happens, check that `rustup --version` works in the same Terminal.
- **`Unable to install` or `The code signature is invalid`**: unlock the phone, plug it in again and try once more. If it repeats, delete the Baton app from the phone and run `flutter run --release --no-resident` again. That also deletes what the app had stored.
- **The camera does not start, or the picture stays black**: check that Baton may use the camera, in **Settings** on the phone, or **System Settings**, **Privacy & Security**, **Camera** on the Mac. The iPhone simulator has no camera. You can always paste the code instead.
- **`Could not reach` a device, after a wait**: Baton is not running on the other device, or that device is asleep, locked or on another network. On a phone, Baton has to be open on the screen. Open it there and try again; **Check the connection** on the send screen says whether it answers. If it never does, **Diagnostics** (see **Anything else**) shows whether this device is connected to the relay and what addresses it found, the ones on the internet without their numbers. A device with neither is not online yet, or has not been allowed to use the local network.
- **A link in the inbox has no Open button**: it is not a web address, so Baton will not open it, and says so under the link. **Copy** still copies it.
- **`Pairing did not work`**: the sentence under it says why. The usual causes are a code that ran out or was used already (show a new one), and a second device that is not open, or is on another network.
- **The two devices show different six digits**: tap **No, cancel** on the first device, and start again.
- **Anything else**: on the **Devices** tab tap the three dots at the top right, choose **Diagnostics** and tap **Copy**. The page lists a few facts (the system, this device's name and short id, how many devices are paired, the relay and the addresses) and the last lines Baton logged, including the message of a crash. Pairing codes, long keys and the account name in a path are left out, and an address on the internet is shown only as `public IPv4 (hidden)` with its port, because it says where you are. Read it first, because it still names your devices and shows the addresses of your own network and the relay. Then open an [issue](https://github.com/jayunewal/device-handoff/issues) and paste it in, with what you did and what you saw. If Baton does not start, **Show details** on the first screen opens the same page.
