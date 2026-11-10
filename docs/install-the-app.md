# Install the Baton app

Baton is pre-alpha and is not in the App Store. You build the app on your Mac, and put it on an iPhone or iPad with a cable. A free Apple ID is enough: you do not need to join the Apple Developer Program. The price of the free Apple ID is that an app you install this way stops opening after seven days, and you install it again with the same command ([Every seven days](#every-seven-days)).

This version asks for a name for the device, shows the devices paired with it, pairs two devices (one shows a QR code and the other scans it, or has it pasted in; [Pair two devices](#pair-two-devices)), sends links and text between them ([Send a link or some text](#send-a-link-or-some-text)), and sends files and folders ([Send files](#send-files)). From an iPhone or iPad you can also look at the Mac's screen or one of its windows, control it, hear its sound and unlock it with Face ID ([Look at and control the Mac](#look-at-and-control-the-mac)). A Baton that receives while the app is closed is not built; the [`baton` program](try-it.md) is the one that can run in the background of a computer.

Everything below is typed in Terminal, which is a zsh shell on a Mac. Each command block can be pasted whole.

## What you need

- A Mac, and an iPhone or iPad with iOS or iPadOS 15 or newer, if you want the phone app. The Mac app needs macOS 12 or newer. Sharing its screen needs macOS 14, its sound macOS 14.2, and control of a window or the whole screen macOS 15.2; privacy mode and the unlock go with control, so they need it too.
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

Closing the window does not quit Baton on a Mac. It hides the window and stays in the menu bar, so that your other devices can still reach this Mac. Choose **Open Baton** from the menu bar item, or click Baton in the Dock, to bring the window back, and **Quit Baton** to end it. The **Close** item of the **Window** menu (Command-W) does what the close button does. To have Baton open when you sign in, turn on **Open Baton when you sign in** in **Settings**. It needs macOS 13 or newer, and macOS may ask you to allow it under **System Settings**, **General**, **Login Items**; Baton opens that page for you.

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
3. Tap **Send link** or **Send text**. Baton says **Sent the link to** or **Sent the text to** and the device's name, and empties the box. If the other device cannot be reached, Baton says that the link or the text **waits here**, and empties the box: see **When the other device is not open**, below. If it did not work for another reason, it says why and leaves what you typed, so that you can try again.
4. On the other device a notice appears at the bottom, such as "Kitchen iPad sent you a link", with a **View** button, and the **Inbox** tab shows how many items are new. The notice says who sent what kind of thing, never what it says: it could be a password, and a notice can be seen by anyone who is looking at the screen. Tap **View**, or the **Inbox** tab. On a Mac whose Baton window is closed, behind other windows or not in front, the Mac shows a notification of the system instead, if you allowed notifications for Baton: it says the same, and a click on it brings Baton's window forward. The same goes for a device that asks to send you files and one that offers to show you a window or its screen, which wait for your yes in the window.

The inbox lists what other devices sent, newest first, and keeps it until you remove it, also after you close the app. On a link, **Open** opens it in the web browser, **Copy** copies it, and the bin removes it. Text can be copied and removed. Long text is cut short until you tap **Show more**. The icon at the top of the inbox clears all of it, after asking.

Baton opens only web addresses, the ones that start with `http://` or `https://`. A link of another kind, such as a phone number, a file or an app, is shown and can be copied, but has no **Open** button: opening it could start something you did not ask for. Characters that cannot be seen, or that turn text round, show as a replacement mark (�) in the inbox, so that a line cannot read as one address and be copied as another. **Copy** copies what was sent, as it was sent.

**Check the connection** on the send screen asks the other device to answer, and says how long it took and whether the traffic went over your Wi-Fi, directly over the internet or through the relay.

In this version a device receives only while Baton is running on it. On an iPhone or iPad that means open on the screen: a locked phone, or one that has Baton in the background, cannot be reached. Receiving on a locked phone needs a push notification, which is not built yet. A link or some text that you send to a device that cannot be reached waits and is sent later: see **When the other device is not open**, below.

To have what you send arrive at once, keep Baton open and the phone in your hand when you send to it, or lengthen **Auto-Lock** while you try Baton (**Settings**, **Display & Brightness**, **Auto-Lock**) and set it back afterwards. If what you send to a phone keeps waiting, check **Low Power Mode** first: it sets Auto-Lock to 30 seconds. Baton keeps the screen on only while two devices pair (see **Pair two devices**) and while files go (see **Send files**). At any other time Auto-Lock works as you set it, because a phone that never locks is the opposite of what Baton is for.

### When the other device is not open

If the other device cannot be reached, the link or the text is not lost. It waits on the device that you sent from, and Baton sends it as soon as the other device can be reached, for up to a day. The send screen says so, and lists what waits for that device under **Waiting to be sent**: what each item is, such as **A link (example.com)** or **Some text (23 characters)**, and never what it says, because it can be a password; how long ago you sent it; when Baton gives up; and a **Cancel** button. At the bottom of the screen, whichever screen you are on, Baton says how each item ended: **Sent a link (example.com) to Kitchen iPad.**, or that the device did not take it and why, or that it could not be reached for a day and Baton gave up. Cancelling says nothing more than that it will not be sent.

- A Mac keeps trying while Baton runs, also with its window closed (see **The Mac app**), unless the Mac is asleep. An iPhone or an iPad tries only while Baton is open on the screen, so what waits on a phone is sent the next time you open Baton there.
- A device that comes back is sent what waits within about half a minute, and not at the very moment.
- At most 50 items wait, and 20 for one device. When they are full, Baton says so and takes no more: cancel some.
- What waits is kept in a file that only you can read, on the device you sent from, until it is sent, cancelled or a day old, and it goes to the device you chose and no other. **Forget this device** cancels what waits for it, without a word.
- Files do not wait. The other device is asked before anything is kept, so it has to be open, with someone there to say yes.
- A Mac whose window is closed does not tell you how an item ended: the sentence shows in the window.
- If Baton closes at the very moment that the other device takes an item, the item can be sent again the next time Baton runs, and arrive twice.
- This has been built and tested without real devices, and has not been tried on a Mac and a phone yet.

### From the share sheet

An iPhone or iPad with iOS or iPadOS 16 or newer can send what it is showing, such as a page in Safari, to a paired device in two taps. Baton has an action for the **Shortcuts** app, **Send with Baton**, which takes a link or some text and brings Baton forward with it in the box of the send screen. Baton does not add itself to the share sheet: you make a shortcut that has the action and shows in the share sheet. This part is built and tested in CI, and has not been tried on real devices yet.

1. Open **Shortcuts**, tap **+**, and search for **Baton**. Add **Send with Baton**.
2. Tap **Link or text** in the action and choose **Shortcut Input**.
3. Name the shortcut, for example **Send to my Mac**. In its details (the **i** button), turn on **Show in Share Sheet**, and keep at least **Text** and **URLs** among the kinds that it accepts.
4. In Safari, or any app with a **Share** button, tap **Share** and then **Send to my Mac**. Baton opens at the send screen with the link or the text in the box. **Nothing is sent yet**: check it, and tap **Send link** or **Send text**. With more than one paired device, Baton first asks which one.

The text a shortcut hands to Baton is kept in memory only, until Baton is on the screen and shows it, and is written nowhere. A text of more than a mebibyte is not handed over, because Baton cannot send so much. If Baton has no paired device, it says so and sends nothing. A normal share extension would put Baton in the share sheet by itself, but it needs an App Group to hand what it was given to the app, and a free Apple ID cannot have one.

## Send files

You need two paired devices, with Baton open on both. They should be on the same Wi-Fi network, as for a link.

1. On the device you send from, open the **Send to** screen of the other device, as for a link. Under **Files**, tap **Choose files** and pick them in the dialog that the system opens. On a Mac, **Choose a folder** sends a folder with everything in it. An iPhone or an iPad chooses files and not folders.
2. The other device asks first. A card at the bottom of its screen says who wants to send what: how many files and how big, and the first names. Nothing is kept until the person at that device taps **Accept**, and they have two minutes. **Decline** keeps nothing, and the device you send from says that it was not taken. A phone that shows the card keeps its screen on for as long as it takes someone to answer, up to five minutes.
3. While the files go, the device you send from shows a bar and what Baton is doing, with **Stop sending** to give up. It keeps the screen of a phone on until the transfer ends, and lets Auto-Lock work again after that. If you leave the screen while files go, Baton asks first, because leaving gives the transfer up.
4. When the transfer is over, Baton says how many files arrived, and the device that took them shows a card with where they are and a button that takes you there: **Show in Finder** on a Mac, which opens Finder with the files selected, and **Show in Files** on an iPhone or an iPad, which opens the Files app at the Baton folder (at the folder itself when a single folder arrived). Baton shows what arrived and never opens it. If you moved or deleted the files since, the card says that Baton cannot find them. Files are put in place only when all of them have arrived and have been checked, so what you find is whole, and a name that is already taken is never replaced: `notes.txt` arrives as `notes 2.txt`.

Where the files go:

- **A Mac:** your **Downloads** folder. The first time, macOS may ask whether Baton may use it. Say yes.
- **An iPhone or an iPad:** the **Baton** folder in the **Files** app, under **On My iPhone** or **On My iPad**.

What to know in this version:

- A phone takes files only while Baton is open on its screen, as for links. The device that sends to it needs it to say yes within two minutes.
- The device that takes files shows no bar. It says that files are coming and keeps its screen on for up to five minutes without news, so a very large transfer to a phone can outlast that. Keep touching the phone if you send something big.
- Links inside folders, files that cannot be read and files that are too large are left out. Baton says how many after it has sent the rest.
- On an iPhone or an iPad the dialog copies what you choose into the app's temporary space before it is sent, so a very large file takes room for a second copy while it goes. Baton deletes the copies when the sending is over, however it went. If Baton is closed while files go, the copies stay until iOS clears them.
- If Baton is closed on the device that sends while files go, nothing is kept of that transfer, and the device that takes the files still lists it and counts it. When Baton is opened again on the device that sent, it tells the other device to let go of it as soon as the two can reach each other, and keeps trying, less often as time goes by, until the other device answers. Until then, which is up to an hour if the sending device stays closed, a device that has left two transfers like that is turned away. The other device counts that hour only while it is awake, so a phone that slept in between may still hold the transfer after more than an hour of the clock. Stopping a transfer with **Stop sending** tells the other device at once, and if it cannot be reached then, Baton tells it later.

## Continue from the Mac

An iPhone or an iPad can ask a Mac which page is open in its browser and what was copied last, and open or copy them with one tap. It is off until you turn it on at the Mac, for each phone. This part is built and tested in CI, and has not been tried on real devices yet.

### Turn it on at the Mac, once for each phone

1. On the Mac's **Devices** tab, tap the three dots at the end of the phone's row, then **Let this device continue from this Mac…**.
2. Turn on **Let** and the phone's name **continue from this Mac**, and tap **Save**. Nobody at the Mac is asked when the phone asks, so this is the one place that says yes.
3. macOS asks whether Baton may control **Safari** and **Chrome**, one question for each that is open. Baton uses that only to read the address and the title of the page in front. Choose **OK**. If you choose **Don't Allow**, the phone still gets what you copied, and Baton tells you where to change it: **System Settings**, **Privacy & Security**, **Automation**. A browser that is not open is not asked, and macOS asks the first time a phone reads from it while it is open. When no browser is allowed yet and one is not open, Baton says so after you save. To be asked now, open the browser and save again. After a rebuild of the Mac app macOS asks again, so save again then too.

### Ask from the phone

1. Keep Baton running on the Mac. With its window closed, its icon in the menu bar is enough.
2. On the phone's **Devices** tab, tap the browser button on the Mac's row (**Continue from** and the Mac's name). A sheet opens with the page that is in front in the Mac's browser and the text on its clipboard.
3. **Open** opens the page on the phone, **Copy address** copies its address, and **Copy** copies the text. Nothing opens and nothing is copied until you tap, and Baton keeps none of it after you close the sheet.

The Mac gives the address and the title of the page in front in Safari or Chrome (the one in front, or the one you were in last), and the text on its clipboard, as they are when the phone asks. It gives nothing while it is locked, its screen is off or another account is using it, and never text that a password manager marked as secret. Baton leaves out the private windows of Chrome. It cannot promise that for Safari, so turn the setting off for a phone if you keep private pages open that it should not see. The phone says **Nothing to continue** when the Mac gave nothing, and cannot tell why. A Mac that is asleep cannot be asked.

## Look at and control the Mac

An iPhone or iPad can show the Mac's whole screen or one of its windows, control it by touch and keyboard, and play its sound. The phone cannot be controlled itself, which iOS does not allow, and showing its screen is not built. This part is built and tested in CI, and has been tried on real devices only in part: see the table in the [README](../README.md#what-works-today).

### Give the Mac its permissions, once

The Mac asks the first time it needs each of these. If you answered **Don't Allow**, turn it on in **System Settings**, **Privacy & Security**:

- **Screen & System Audio Recording**, to share the screen or a window and its sound.
- **Accessibility**, to let the phone control the Mac and to keep the Mac's own keyboard and trackpad held in privacy mode.
- **Notifications**, so that the Mac can tell you afterwards that it was unlocked from the phone, and can tell you that something arrived while Baton has no window in front.
- **Automation**, for Safari and Chrome, only if you let a phone continue from the Mac (see above). macOS asks when you turn that on.

**After every rebuild of the Mac app**, macOS treats it as a new program and the old permissions no longer apply, but the switches can still look on. Sharing or control then stops working without saying why. Reset them and grant them again:

```sh
tccutil reset All io.github.jayunewal.baton
```

Start Baton, allow each permission when it asks, then quit Baton and start it once more. To stop this from happening after each rebuild, sign the Mac app with a certificate of your own, as in the next section.

### Keep the permissions when you rebuild

A build of the Mac app is signed ad hoc unless you say otherwise, and an ad hoc signature is new with every build. That is why macOS forgets the permissions above each time. Signing every build with one certificate keeps them: macOS then sees the same program again. The certificate is made on your Mac in a few minutes, costs nothing, needs no Apple ID and no Apple Developer Program, and goes nowhere. Do this once on each Mac that builds Baton.

1. Open **Keychain Access**: search for it with Spotlight. Since macOS 15 it is no longer in **Applications**, **Utilities**. In its menu bar choose **Keychain Access**, then **Certificate Assistant**, then **Create a Certificate…**.
2. Name it **Baton Signing**. Set **Identity Type** to **Self Signed Root** and **Certificate Type** to **Code Signing**. Click **Create**, then **Done**. It is kept in your **login** keychain.
3. Under **My Certificates**, double-click **Baton Signing**, open **Trust**, and set **Code Signing** to **Always Trust**. Close the window, and give your password when macOS asks.
4. Tell the build to use it. In `device-handoff/apps/baton`, run:

   ```sh
   printf 'CODE_SIGN_STYLE = Manual\nCODE_SIGN_IDENTITY = Baton Signing\n' > macos/Runner/Configs/Signing.local.xcconfig
   ```

   Git does not keep this file, so it is for this Mac only. Delete it to go back to ad hoc signing.
5. Build as before with `flutter build macos --release`. The first time, macOS may ask whether `codesign` may use the key. Choose **Always Allow**.
6. Check that it took: `codesign -d -r- build/macos/Build/Products/Release/Baton.app` should print a line that has `certificate leaf =` in it. An ad hoc build prints `cdhash` instead.
7. A build with the certificate is a new program to macOS the first time, so reset the permissions and grant them once more, as in the section above. After that a rebuild keeps them.

The private key of the certificate stays in your keychain and is not needed by anyone else: do not export it or put it in the repository. A build that is made somewhere else, such as the one CI makes, is still signed ad hoc and starts with no permissions.

### Start from the Mac

On the Mac's **Devices** tab, the screen button on the phone's row offers **Share to look at** and **Share a window or screen to look at and control**. The system's picker opens; choose a window or a whole display. The phone shows it. **Stop sharing** ends it.

### Start from the phone

First allow it at the Mac, once: in the **More** menu (three dots) of the phone's row choose **Let this device ask for the screen…** and pick look only, or look and control. Then, on the phone, the eye button on the Mac's row offers **Look at its screen** and **Look at its screen and control it**. Either one asks for your face (Face ID or Touch ID), or the phone's passcode, before the phone asks the Mac, every time; a phone with no passcode cannot start one.

### Privacy mode

A session that the phone controls is private: the Mac's screens stay dark, its own keyboard and trackpad are held, and the Mac locks when the session ends or the connection drops. Three presses of Escape within two seconds at the Mac end the session. Two switches in the settings button of the phone's bar lift this for one session: **Show screen on the Mac** and **Allow the Mac's keyboard and trackpad**. Both are off unless you turn them on.

When you end a private session at the phone while the Mac is unlocked, the phone asks whether to lock the Mac: **Lock**, **Leave it** or **Cancel**.

### Sound

The speaker button in the phone's bar plays the Mac's sound on the phone (**Hear the sound of** and the Mac's name), and the Mac stays quiet while it does. In a private session the sound is on from the start. Sound goes with a whole display, not a single window. At the Mac you choose which paired devices may hear it.

### Unlock the Mac with Face ID

When the Mac locks during a session, the phone shows a card saying so with an **Unlock** button. To set it up, at the Mac choose **Let this device unlock this Mac…** in the **More** menu of the phone's row and type the Mac's password once; it is kept locked by a key that only the phone's Face ID or Touch ID opens. Then **Unlock** asks for your face and the Mac types its own password. It works only in a private session with both switches off. The Mac shows a notification afterwards and keeps a list of attempts in that same window.

The first tries on real devices worked. When you try it, use a Mac account whose password you can change back.

### Unlock the Mac after a restart

A Mac with FileVault keeps its disk locked after a restart until someone gives it a password, and Baton does not run until then. On an iPhone or an iPad, **Settings** and **Unlock a Mac after restart** give the Mac that password over SSH, behind Face ID. At the Mac, turn on Remote Login in System Settings, under **General** and **Sharing**. At the phone, tap **Add a Mac**, type the Mac's address and the account, compare the key with the command that the phone shows, and save. This has not been tried on a restart yet. [Reach your Mac from away](away-from-home.md) says how to give the Mac a fixed address, and how to make this work from outside your home network.

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
(cd ios && pod install)
(cd macos && pod install)
flutter build ios --config-only --no-codesign
open ios/Runner.xcworkspace
```

Choose your team again as above (and your own bundle identifier, if you changed it), close Xcode, and run `flutter run --release --no-resident`.

The two `pod install` lines matter when a newer version adds or removes files of Baton's plugin, which Flutter does not always notice. Without them a build can stop with `Build input file cannot be found`, naming a file under `rust_builder` (in target `baton_ffi` from project `Pods`). If you see that, run them and build again.

## If something does not work

- **`Signing for "Runner" requires a development team`**: choose your Personal Team in Xcode as above. Plug the phone in before you open **Signing & Capabilities**, so that Xcode can register it.
- **`Failed to register bundle identifier`**, or a message about the number of apps or App IDs: change the bundle identifier as above. A free Apple ID can make only a few app identifiers a week and can have only a few apps of this kind on a phone, so delete another app you installed this way, or wait.
- **`Developer Mode disabled`**: turn it on as above.
- **The phone is listed as unsupported, or Xcode says it cannot use this iOS**: your Xcode is older than the iOS on the phone. Update macOS and Xcode.
- **`Untrusted Developer` when you open Baton, or Baton closes at once**: trust yourself as above. If it was working and stopped, seven days have passed: install it again as above.
- **Sharing, control or sound stopped working after a rebuild of the Mac app**: reset the permissions as in [Give the Mac its permissions, once](#give-the-mac-its-permissions-once), and sign the app with a certificate of your own so that it does not happen again ([Keep the permissions when you rebuild](#keep-the-permissions-when-you-rebuild)).
- **`cargo` or `rustup` not found**: Xcode does not use your shell's search path when you build from its window. Build with `flutter run --release --no-resident` in Terminal, as above, and if it still happens, check that `rustup --version` works in the same Terminal.
- **`Unable to install` or `The code signature is invalid`**: unlock the phone, plug it in again and try once more. If it repeats, delete the Baton app from the phone and run `flutter run --release --no-resident` again. That also deletes what the app had stored.
- **The camera does not start, or the picture stays black**: check that Baton may use the camera, in **Settings** on the phone, or **System Settings**, **Privacy & Security**, **Camera** on the Mac. The iPhone simulator has no camera. You can always paste the code instead.
- **`Could not reach` a device, after a wait**: Baton is not running on the other device, or that device is asleep, locked or on another network. On a phone, Baton has to be open on the screen. Open it there and try again; **Check the connection** on the send screen says whether it answers. If it never does, **Diagnostics** (see **Anything else**) shows whether this device is connected to the relay and what addresses it found, the ones on the internet without their numbers. A device with neither is not online yet, or has not been allowed to use the local network.
- **A link in the inbox has no Open button**: it is not a web address, so Baton will not open it, and says so under the link. **Copy** still copies it.
- **`Pairing did not work`**: the sentence under it says why. The usual causes are a code that ran out or was used already (show a new one), and a second device that is not open, or is on another network.
- **The two devices show different six digits**: tap **No, cancel** on the first device, and start again.
- **Anything else**: on the **Devices** tab tap the three dots at the top right, choose **Diagnostics** and tap **Copy**. The page lists a few facts (the system, this device's name and short id, how many devices are paired, the relay and the addresses) and the last lines Baton logged, including the message of a crash. Pairing codes, long keys and the account name in a path are left out, and an address on the internet is shown only as `public IPv4 (hidden)` with its port, because it says where you are. Read it first, because it still names your devices and shows the addresses of your own network and the relay. Then open an [issue](https://github.com/jayunewal/device-handoff/issues) and paste it in, with what you did and what you saw. If Baton does not start, **Show details** on the first screen opens the same page.
