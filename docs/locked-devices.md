# Working while a device is locked

Status: research summary and proposed design rules, 2026-10-14. Nothing in this document has been tested on real hardware. Section 8 lists the tests, and Baton promises nothing that this page marks as unknown until they have been run.

Handing work off means walking away from the first device. By the time the second device needs something, the first is usually locked and often asleep. A hand-off tool that needs the first device to stay unlocked fails at its one job, so working while locked is a requirement for every milestone and not a feature of one (see the [roadmap](roadmap.md)).

This page records what each operating system allows, so that Baton promises only what it can keep, and the design rules that follow. It summarises three research passes: Windows; macOS, iOS and iPadOS; and Linux, Android and existing remote-access tools. [architecture.md](architecture.md#locked-devices) keeps the short form of the rules.

## 1. The short answer

What Baton can plan to deliver:

- **Hand something off, then lock the device.** A link, text or file that the person sends while at the device is delivered when the other device can be reached, and a lock on the source does not stop that: a locked agent keeps delivering what the person explicitly sent (section 5.3). The "send and lock" gesture goes one step further. It waits for the other device to acknowledge the item and only then locks the source, so the item has left before the device can lock or sleep (section 5.6). Where the platform has a way to lock a device (Windows and Linux; Android only with privileged permissions; macOS has no documented one; iOS has none), Baton can lock the source for the person once the other device has acknowledged the item. The gap that remains is a source that goes to sleep before the other device appears. Only the mailbox of milestone 4 covers that.
- **A locked desktop that is still awake should keep receiving.** On Linux (reported), Windows (inferred, to be tested by W2) and macOS (inferred, with the qualifiers in section 3) a lock covers the screen but should leave the person's processes and network running, so the agent can accept items into its inbox, answer pings and deliver what the person sent. The person sees what arrived when they unlock.
- **Phones are reached through push notifications.** A locked iPhone or iPad suspends Baton within seconds, so a push notification is the only dependable route. A locked Android phone keeps a listener only through a foreground service, which the phone's maker may still stop.
- **Baton never unlocks anything.** Not for the person, not for another device, not as a convenience. See section 5.5.

What Baton cannot promise, because the platforms do not allow it:

- **Reading what is open on a locked device when asked.** The clipboard cannot be read in the background on iOS and Android, and GNOME offers no sanctioned way to do it. On Windows and macOS this is unknown, and macOS has announced an alert for programmatic pasteboard reads (section 3).
- **Showing or controlling the live windows of a locked device.** No documented API that Baton could use does this on any platform examined. The one product built around a locked device is Apple's iPhone Mirroring, which keeps the iPhone locked (documented), works only between an iPhone and a Mac, and cannot be reused. GNOME closes every remote-access session when the screen locks, Android stops screen capture on lock, and Windows refuses desktop capture on the secure desktop. Products that stream from a "locked" computer unlock it first and blank its screen, or start a separate session, or are given the password. Window streaming (milestone 5) has to be designed around this, and the first releases rely on locking the source after the hand-off.
- **Reaching a device that is asleep.** On desktops, sleep and not the lock is what stops the agent. On iOS and iPadOS the app is suspended shortly after the lock itself, and on Android Doze and the phone maker's background killers can end a foreground service (section 3). Waking a sleeping device takes Wake-on-LAN (best effort, hardware dependent) or a push notification.
- **Reaching a computer that has restarted and has not been signed in to**, or an iPhone that has not been unlocked since it booted.

## 2. How to read this page

Every claim carries one label:

- *documented*: stated in official documentation or a formal specification;
- *reported*: shown by source code, a maintainer's or vendor's statement, or a credible report. This is evidence, not a promise by the platform;
- *inferred*: reasoned from other facts. No source says it;
- *unknown*: no evidence either way. Each of these is on the list in section 8.

In the tables, **Works**, **Partly**, **No** and **Unknown** say what the evidence supports, followed by its label. Works (inferred) means "should work, and nobody has shown it". Nothing is Works (tested).

The states used:

- *Locked, awake*: the person has signed in, the lock screen is up and the machine has not gone to sleep. The screen may be on or off.
- *Asleep*: the machine went to sleep (power button, lid, idle timeout, menu).
- *Restarted*: the machine has booted and nobody has signed in yet. On a phone this means not unlocked since it booted.

How far the evidence goes:

- Nothing was run on hardware. Everything comes from documentation, source code and reports.
- Some pages reached the researchers only through a summarising tool, and some hosts were unreachable. Every Windows source was read that way, and so were the Apple forum, support and vendor pages, so the "documented" labels that rest on them rest on model-written summaries. Re-read these before the rules that depend on them are relied on: how the Desktop Activity Moderator times its suspension and throttles services, `LockWorkStation`, automatic sign-in after an update restart, and the five-minute limit on power requests (all Windows), and every statement about what a remote-access product does on a locked Mac. Apple's developer documentation and release notes were read as raw text, which covers the 30 seconds of `willSleep` and the launchd rule on quarantined property lists.
- The researchers ran out of search budget before they had covered everything. Section 9 lists the gaps.
- Operating systems change. The releases that matter are named where they do: Windows 11 24H2 changed how services are suspended, macOS 15 added Local Network privacy, Android 15 QPR1 stops screen capture on lock, and Android 17 adds a local-network permission for apps that target SDK 37.

## 3. What each platform allows while locked

"Agent" means Baton's background process for the person's account. Section 5.1 says how it is started on each platform.

### Windows

| | Locked, awake | Asleep: Modern Standby | Asleep: S3 | Hibernated | Restarted, nobody signed in |
| --- | --- | --- | --- | --- | --- |
| Agent keeps running | **Works** (inferred, to be tested by W2). A lock changes only which desktop receives input, and the session's processes should keep running, but no source says so for a per-user process, and a locked session with the screen off is not documented. | **No** (documented). A few seconds after sleep starts, the Desktop Activity Moderator suspends every process in the person's session. Since 24H2 more services may be suspended as well. | **No** (documented). The whole system is suspended. | **No** (documented). The machine is off. Resuming from hibernation needs the BitLocker PIN or startup key when the disk uses TPM+PIN. Whether the agent survives a resume is **Unknown** (W8). | **No** (documented). A per-user agent starts at sign-in. A boot-start LocalSystem service could run, but not on a disk locked by TPM+PIN BitLocker (documented, inferred). |
| Network reachable | **Works** (reported, inferred). A lock does not touch networking. The evidence is that the host services of Sunshine, RustDesk and Chrome Remote Desktop serve clients while the console is locked, and those are services and not per-user processes (W2 tests the per-user case). | **Partly** (documented). The network adapter sleeps in a low-power mode and nothing answers until the machine resumes. | **Partly** (documented). Only a Wake-on-LAN packet wakes the machine, and the agent must announce itself again afterwards. | **No**. | **Partly** (inferred). The network is normally up, but only a service could listen. |
| Clipboard text | **Unknown**. The clipboard belongs to the session, so it probably works (inferred), but no source tests it. | **No**. The agent is not running. | **No**. The agent is not running. | **No**. | **No**. No user clipboard exists yet. |
| Open a link or file | **Partly** (inferred). The agent can probably start the handler, but its window opens on a desktop nobody sees until unlock. Queue the item, open it on unlock and show a notification. | **No**. | **No**. | **No**. | **No**. |
| Read local files | **Works** (inferred) for local files. OneDrive online-only files and encrypted (EFS) files are **Unknown**. | **No**. | **No**. | **No**. | **Partly** (inferred). Only a service running as SYSTEM, with no user context. |
| Capture a window | **No** (documented, reported). Desktop Duplication fails with access denied on the secure desktop unless the caller is LocalSystem, and automation vendors report paused sessions and black frames. Windows.Graphics.Capture is **Unknown**. A SYSTEM helper can capture the lock screen itself, not the person's windows. | **No**. | **No**. | **No**. | **Partly** (reported). Only the sign-in screen, through a SYSTEM helper. |
| Inject input | **No** for the person's apps (documented, reported). **Partly** at the lock screen through a SYSTEM helper, which means typing credentials. | **No**. | **No**. | **No**. | **Partly** (reported). SYSTEM helper only. |

- Whether a screen that is off, on a machine that has not yet gone to sleep, counts as Modern Standby is not documented. The table treats it as awake.
- The power model, Modern Standby or S3, is fixed by the hardware and firmware, and a program can tell which it is at run time (documented).
- Viewing a locked console remotely is not established. Community answers say a Remote Desktop connection by the same user takes the session off the console, and an automation vendor says closing it locks the computer (reported; no Microsoft statement was found). Chrome Remote Desktop is reported to use a loopback Remote Desktop connection on editions that can host Remote Desktop (Windows Home cannot). Whether that re-attaches the person's existing session or creates a new one is unknown (W10). It is heavy and weakens the host's security.
- A virtual display driver could be the one place where a lock does not interrupt frames, because such a driver runs in session 0. That is a hypothesis only. No source says whether a window on a virtual monitor stays composed and capturable while the session is locked (W11).
- After an update-initiated restart, Windows can sign the last user in again and leave the session locked (documented). The agent may therefore start inside an already-locked session and must read the state when it starts.

### macOS

| | Locked, awake | Asleep | After a FileVault restart |
| --- | --- | --- | --- |
| Agent keeps running | **Works** (inferred, to be tested by M1). No Apple page says a locked session is suspended, and a report from an administrator of CI Macs and the design of Chrome Remote Desktop both show session processes surviving. Three qualifiers. A vendor reports that a Mac left at its lock screen for 5 to 10 minutes can stop answering unless the setting that prevents sleeping when the display is off is on (reported), and on laptops that setting applies only on the power adapter (documented). Whether App Nap throttles an agent that has no windows is not documented, and holding an activity or a power assertion while a peer is connected is the proposed way around it (inferred; M1 runs with and without). | **No** (inferred). System sleep stops the CPU, and an archived Apple note says forced sleep cannot be prevented, only delayed (documented). Whether third-party code can run in Dark Wake or Power Nap is **Unknown**. | **No** (reported). No third-party code runs before credentials are entered. On Apple silicon with macOS 26 or later the Mac can be unlocked over SSH if Remote Login is on. |
| Inbound LAN connection | **Works** (inferred) after a one-time Local Network grant for the agent. Launch daemons are exempt from that prompt and launch agents are not (documented), and sending UDP and opening outgoing TCP connections need it (documented). That a QUIC reply counts as sending UDP is inferred, and so is the claim that relay traffic over the internet needs no grant (M2). | **Partly**, **Unknown**. Only through a Sleep Proxy or Wake-on-LAN. Wired Ethernet is the supported case, and Wi-Fi and Apple silicon are unverified. | **No**. |
| Clipboard: set | **Unknown**. No gate on writes is documented. | **No**. | **No**. |
| Clipboard: read | **Unknown**. Apple has announced an alert for programmatic pasteboard reads, and an alert on a locked, unattended Mac cannot be answered. Whether macOS 27.0 and 27.1 show it is unknown (M6). | **No**. | **No**. |
| Open a link | **Unknown**. Nothing documents `open` while locked. At most the page loads behind the lock screen (inferred). | **No**. | **No**. |
| Write a received file | **Works** (inferred). The volume is mounted. | **No**. | **No**. |
| Read local files | **Works** (inferred) for paths the agent already has access to. iCloud placeholder files are **Unknown**. | **No**. | **No**. |
| Capture a window or the display | **Unknown**, **Partly**. Apple's developer support says it would expect ScreenCaptureKit to work in a locked GUI session and warns of restrictions around the unlock screen. It is unavailable before login, and a sleeping display cannot be captured. A user report says screen recording does not work through the lock screen (a conflict, unresolved). | **No**. | **No**. |
| Inject input | **Unknown**, probably **No** for app windows (inferred from Apple's statement that UI automation, which uses the Accessibility API, cannot run in a locked session). | **No**. | **No**. |

- Every privacy mode examined on macOS (Parsec, Jump, Screens, TeamViewer, Splashtop, AnyDesk, RustDesk) blanks or hides the physical display during a session, and most block local input. That presupposes the Mac was unlocked first (inferred). Only Parsec and Jump are documented to lock the Mac when the session ends, and RustDesk locks only if the controlling side asks for it. One product documents connecting to a Mac at the lock screen and unlocking it, and another has a separate login-screen build that needs FileVault off and wired Ethernet. None is documented to work before the FileVault password is entered.
- A virtual display has no public API (private functions only). Apple's DriverKit lists no display family.
- macOS Wake-on-LAN is best effort. Nothing was found for waking a Mac from the internet.

### iOS and iPadOS

| | Locked |
| --- | --- |
| Agent keeps running | **No** (reported by Apple's developer support). The app is suspended shortly after the lock unless a background mode or task keeps it running. After `applicationDidEnterBackground` it has about five seconds, which `beginBackgroundTask` can extend (documented). |
| Inbound LAN connection | **No** (reported). A suspended app cannot accept connections, and Wi-Fi may be dropped when the device sleeps. |
| Reach the device | **Partly** (documented). An alert push notification, which a Notification Service Extension can rewrite within about 30 seconds, is the only documented way to reach a locked device. Apple asks for no more than two or three background pushes an hour, and the App Store guidelines require that the app also works without push. |
| Lock-screen notification | **Works** (documented). An alert push shows on the lock screen, and the preview text follows the person's setting. A tap is expected to lead to the unlock prompt and then to the app (inferred). |
| Clipboard: set | **Unknown**, probably **No**. `UIPasteboard` needs a running app, and writing from a notification extension is untested. |
| Clipboard: read | **No** (reported by Apple's developer support). There is no way to read the pasteboard in the background. |
| Open a link | **Partly** (documented, inferred). Delivered as a notification. The tap is expected to need an unlock. |
| Write a received file | **Partly** (documented). Default file protection works after the first unlock but not before it. The strictest class fails while locked. |
| Capture the screen | **Unknown**. The person has to start it in a system picker, and it is output only (inferred). Legacy ReplayKit broadcast stops on lock (vendor documentation). What the newer ScreenCaptureKit on iOS does on lock is undocumented. |
| Inject input | **No**. There is no public API. |
| After a restart | **Partly** (reported, undocumented). The system tries not to launch anything before the first unlock except a few components such as notification service extensions, with exceptions in both directions. Even those cannot read keychain items of the usual after-first-unlock class until then (documented, inferred). |

### Linux

| | GNOME (Wayland) | KDE Plasma (Wayland) | X11 desktops (Xfce, Cinnamon, MATE, i3 and others) |
| --- | --- | --- | --- |
| Agent keeps running | **Works** (reported). Locking only shows a shield and sets the logind `LockedHint`. User services keep running. The upstream default suspends an idle machine after 15 minutes. | **Works** (reported). The same mechanism. The default suspends after 15 minutes on AC and 10 on battery. | **Works** (reported, inferred). A locker is a window plus input grabs, and other programs keep running. |
| Network reachable | **Partly** (documented, inferred). Yes while awake. After suspend only if Wake-on-LAN or Wake-on-WLAN works, which depends on the hardware. | **Partly** (reported). The same. After a network wake PowerDevil suspends again after 30 seconds unless user activity is registered. | **Partly** (inferred). Awake only. |
| Clipboard | **No** through sanctioned routes (reported, inferred). Mutter has no data-control protocol, core Wayland needs keyboard focus, and portal clipboard sessions are closed on lock. A Shell extension might work and is untested. | **Unknown**. KWin 6.7 gives native programs background clipboard access through `ext-data-control`. Behaviour while locked is untested. | **Works** (inferred). Selections are not tied to focus and a locker does not isolate programs. Untested. |
| Open a link | **Partly** (reported, inferred). A handler can be started, but GNOME hides all windows in lock mode, so the person sees it only after unlocking. | **Partly** (inferred). The lock covers every output. KDE's overlay protocol for showing a surface above the lock is internal and says regular clients must not use it. | **Partly** (inferred). The process starts and the locker covers it. |
| Read files | **Works** (inferred). | **Works** (inferred). | **Works** (inferred). |
| Capture a window | **No** (reported). GNOME Shell inhibits remote access in lock mode, and Mutter closes its ScreenCast, RemoteDesktop and InputCapture sessions and refuses new ones. | **Unknown**. No lock handling was found in KWin's screencast plugin or the KDE portal backend, so frames may show the lock screen. | **Partly** (inferred). Capturing the root window shows the locker. Per-window capture may still work. |
| Inject input | **No** (reported, inferred). The remote-control interfaces are closed on lock, and events from `uinput` would reach only the lock screen. | **No** for applications (reported). KWin delivers input only to lock-screen windows while locked. | **No**, **Partly** (inferred). Synthetic events pass through the locker's grab and land in the locker. |

- GNOME has dropped its X11 session and Plasma 6.8 is Wayland-only (both reported), so the X11 column concerns the other desktops.
- A locked, idle laptop is stopped by its idle-suspend setting or by closing the lid, not by the lock. See section 4.
- Existing tools do not stream the real windows of a locked GNOME session. On Linux the field is thin: Parsec cannot host, and AnyDesk refuses incoming Wayland sessions (both documented). RustDesk and Sunshine capture at root level and see only the lock screen (reported). Chrome Remote Desktop and GNOME Remote Desktop's headless mode start a separate virtual session. KDE's remote-access mode does the same, and it is known only from the master branch of its source code and a blog post, with its handling of the lock unfinished (reported). Elsewhere, tools use stored credentials or lock when the remote session ends.

### Android

| | Locked (Android 16 and 17) |
| --- | --- |
| Agent keeps running | **Partly** (documented, reported). Only as a foreground service of a declared type, and phone makers' background killers, Low Power Standby and a reported automatic reboot after 72 hours locked can still end it. |
| Network reachable | **Partly** (reported). AOSP code lets a foreground service keep the network in Doze, but the documentation is inconsistent about it, and Low Power Standby is documented to cut the network even for such apps. Apps that target SDK 37 need the `ACCESS_LOCAL_NETWORK` permission for traffic on the local network. |
| Clipboard: set | **Works** (reported). A comment and the code in the Android source allow a write without focus and have no check for a locked device. What the person sees when a background write happens is unknown (A6). |
| Clipboard: read | **No** (reported). Reads return nothing when the device is locked, and background reads are refused. |
| Open a link | **Partly** (documented). Only through a notification, whose action can require an unlock. Starting an activity from the background is blocked and full-screen intents are restricted. |
| Receive a file | **Partly** (documented). App-private storage works after the first unlock. Shared storage needs permissions. Before the first unlock only device-encrypted storage is available. |
| Capture the screen | **No** (documented, reported). MediaProjection stops when the screen locks (Android 15 QPR1 and later), and the lock screen is protected from capture. |
| Inject input | **Partly**, privileged only (reported). Debug-bridge tools can type a PIN blind. An ordinary app's accessibility service on the lock screen is **Unknown**. |

## 4. Sleep is what stops the agent, not the lock

| Platform | What stops it | What Baton can do |
| --- | --- | --- |
| Windows | On Modern Standby, the Desktop Activity Moderator suspends the person's processes a few seconds after sleep starts. On S3 the whole system is suspended (documented). | A keep-awake request cannot stop a sleep that the person starts with the lid, the power button or the Start menu: all power requests end when a user-initiated sleep begins. On Modern Standby on battery, system-required and execution-required power requests end five minutes after the sleep timeout (both documented for `PowerSetRequest`). Whether `SetThreadExecutionState` behaves the same is not documented in the research, and W6 tests both. Staying awake during a hand-off is therefore possible only on AC power, and only until the lid closes (inferred). On Modern Standby a program must register for suspend notifications to receive the suspend message at all (reported). |
| macOS | System sleep stops the CPU (inferred). Power assertions are suggestions that lid close, low battery, thermal limits and Dark Wake override, and the user-idle assertions have no effect in Dark Wake (documented). | Hold an assertion only while a peer is connected, and expect a sleeping Mac to be unreachable. The will-sleep notification gives up to 30 seconds to finish what is running (documented). |
| Linux | Suspend freezes the person's processes (documented). The upstream defaults suspend an idle machine after 15 minutes on GNOME, and after 15 minutes on AC and 10 on battery on KDE (reported, from source code). What distributions set instead was not checked (unknown). In logind, closing the lid ignores ordinary `sleep` inhibitors by default and honours only the `handle-lid-switch` inhibitor (documented), but GNOME's and KDE's own lid handling differs from logind's: KDE's PowerDevil handles the lid itself, and what GNOME does with suspend inhibited is unknown (L1). | A delay inhibitor is allowed for every user (reported) and holds a suspend for five seconds at most by default (documented). Whether a user service may take a blocking inhibitor is unknown. Baton must not take an idle inhibitor just to stay reachable, because that also disables the person's auto-lock, and must not take the lid inhibitor by default, because a closed laptop in a bag would stay awake. |
| Android | Doze, app standby, phone makers' background killers and Low Power Standby. An automatic reboot after 72 hours locked is reported but not documented by Google. | A foreground service of a declared type, and the local-network permission. How much battery an always-on service costs is unknown. |
| iOS and iPadOS | The app is suspended shortly after the lock, and Wi-Fi may drop when the device sleeps. | Push notifications. There is no background agent. |

The rules that follow:

- **Assume every connection dies when the device sleeps.** After every resume the agent treats all peer connections as dead: it closes them, announces itself again on the local network and reconnects to the relay.
- **Hold a sleep inhibitor only while a transfer or a session is running**, for a bounded time, and never just to stay reachable while idle. The bound should come from the tests in section 8.
- **Do not promise delivery to a sleeping device.** Items for it wait (section 5.3).

## 5. Design rules

These are proposals for review. They refine the rules in [architecture.md](architecture.md#locked-devices), and section 5.2 replaces one of them (the push at lock time).

### 5.1 The agent is a per-user process that starts at sign-in

A lock should leave the agent running (reported on Linux, inferred on Windows and macOS). A restart before sign-in does not start it. So the agent is the person's own background process and not a system service:

- **Windows.** Started at sign-in by a Run key or a logon task (documented), or by the Startup folder (inferred; the research did not source it separately). A boot-start LocalSystem service could answer before sign-in, but it would have no user session, no clipboard and no OneDrive client, it cannot work on a machine whose disk is locked by TPM+PIN BitLocker, and it would be a large security surface because it runs as SYSTEM. It is not planned. If it is ever offered, it will be a separate install that the person opts in to.
- **macOS.** A LaunchAgent installed by the signed app. The one-time Local Network grant is asked for during onboarding, while the person is at the Mac, because launch agents are not exempt from it. Until the grant is given the first connection uses the relay, which is inferred to need no grant (M2). macOS 27's launchd refuses property lists that carry the quarantine attribute (documented).
- **Linux.** A `systemd --user` service. Running with nobody logged in ("lingering") gives no display, clipboard or browser, so it would suit only mailbox work (section 5.3).
- **Android.** A foreground service of the `connectedDevice` type, started while the app is in the foreground. For apps that target SDK 37, the local-network permission is asked for during onboarding, and without it the app uses the relay.
- **iOS and iPadOS.** No background agent. The device is reached through push notifications, which need the push gateway of milestone 4.

After a restart a device cannot be reached until its person signs in (or unlocks the phone once). The apps say so instead of failing quietly.

### 5.2 Stage before leaving, and do not read a locked device

What a person wants to continue cannot be counted on to be readable once the device locks. iOS and Android do not allow background clipboard reads, GNOME has no sanctioned background access, and on Windows and macOS it is unknown (section 3). So a hand-off starts while the person is there:

- by an explicit gesture that names the item and the device it goes to: a share target, a tray menu, a hotkey, a notification action, or "send and lock" (section 5.6). The result is a **sent item**; or
- where the platform allows background clipboard reads (X11 and KDE; unknown on Windows and macOS), by staging the clipboard continuously, but only if the person turns that on, and never items that a password manager marks as concealed (see [security.md](security.md)). The result is a **staged item**, which nobody addressed to a device.

Sent items wait in an outbox until their target can be reached (section 5.3). Staged items wait on the device until a paired device asks for them, and section 5.4 says when a locked device answers.

**The lock event sends nothing new.** Many locks are idle timeouts with the person still at the desk (inferred; the research has no data on how often), and a push at every lock would deliver content that nobody asked to move. On GNOME and Android nothing would be readable at that moment anyway (section 3). A lock does not hold back what the person already chose, though: sent items keep going out while the device is locked, to the device the person picked.

**A sleep sends nothing new either, and the time before it is short.** That is a separate argument from the lock:

- Linux: a delay inhibitor holds a suspend for five seconds at most by default (documented), and the network may already be going down.
- macOS: an observer of `willSleepNotification` can delay sleep for up to 30 seconds (documented).
- Windows: on Modern Standby the person's processes are suspended a few seconds after the sleep starts (documented), and a program receives `PBT_APMSUSPEND` only if it registered for it (reported).

The agent may use these moments to send the sent items that are still in the outbox, within those limits. It does not read or push anything else.

This replaces the earlier rule that the agent pushes the latest clipboard and tab "when the device is about to lock or sleep". It also overrides a recommendation of the Apple research, which was to flush pending clipboard, link and file announcements at the lock and screen-saver notifications and at `willSleep`. Baton flushes only what the person has already sent, because a staged item was never chosen for a device.

### 5.3 A hand-off must not need the source afterwards

Delivery has three paths, tried in this order:

1. **Direct.** Both devices are awake and can reach each other, on the same network or through a relay. The item is delivered at once.
2. **Sender outbox.** The item waits on the sender until the other device can be reached. This needs the sender to be awake, and it does not need the sender to be unlocked: **a locked agent delivers what the person explicitly sent.** It may dial the named device when that device appears on the network or becomes reachable through the relay, and it dials no other device. The person chose both the item and the target while at the device, so delivering it reveals nothing that they had not decided to share.
3. **Mailbox on the account server** (milestone 4). The server holds the item, encrypted, until the other device comes online. This is the only path that works when the sender has gone to sleep or off before the target appears.

Until the mailbox exists, an item for a device that is off or out of reach is delivered when both devices are awake and reachable at the same time, and the apps say so. A sender that is locked and awake counts as awake. The case that stays open is a laptop that goes to sleep with the item still in the outbox, and the "send and lock" gesture of section 5.6 exists to avoid it: the source is not locked until the other device has the item.

The limits on a locked outbox:

- **Sent items only.** A locked agent does not push staged items. It serves them on request, and only under the opt-in in section 5.4.
- **Only the named device.** The agent never widens an item to another device. Unpairing a device cancels everything queued for it.
- **A lifetime.** An item that has not been delivered within its lifetime is dropped, and the person is told. The proposed default is 24 hours, to be decided before the outbox ships.
- **The receiver decides what happens next.** The receiving device applies its own locked mode and its own policy (section 5.4).

The mailbox is a proposal, drawn from how Magic Wormhole's rendezvous server queues messages for clients that connect later and how Firebase Cloud Messaging stores a message until the device connects (both documented). It needs:

- opaque ciphertext plus minimal routing data, and a sender authenticated by the key from pairing;
- a sequence number for each item and a replay window kept by the receiver, because a relay may not deduplicate or order messages (Magic Wormhole's server does not);
- a short default lifetime, per-device quotas, deletion on acknowledgement, and a purge of a device's mailbox when it is unpaired;
- for large files, an encrypted blob with only its content key and location in the small message, which also fits the 4 KB limit on a Firebase data message;
- no promise to hide metadata. The relay learns who talks to whom, when and how much. Padding, rotating mailbox identifiers and keeping no address logs reduce this and do not remove it. A compromised relay can drop, delay, replay and watch items, and must not be able to read or forge them.

Items for a locked Android phone can be sealed to a key in the Android Keystore that was created with `setUnlockedDeviceRequired(true)`. Public-key operations stay allowed while the phone is locked, and the private key is usable only after unlock (documented, with bugs reported on Android 12 to 14). Section 5.9 says more.

### 5.4 Locked mode on the receiving side

- The agent learns the lock state from the platform's events (section 6), reads it once when it starts, because it may start inside a locked session, and reads it again after every resume. **If the state cannot be known, it acts as if the device were locked.** iOS has no public API for it, and on Linux only GNOME's and KDE's lockers were verified to set the logind `LockedHint`.
- While locked, the agent accepts items from paired devices into the inbox and does nothing else with them. No link is opened, no clipboard is written, and no file is opened. Policies that open or apply items automatically are suspended until unlock.
- It shows a notification that names the sender and hides the content where the platform lets it (Android visibility settings and actions that require authentication, the iOS preview setting and actions that require authentication). On unlock the inbox is shown, and what the person's policy allows is opened then.
- It starts no hand-off of its own, but it finishes the ones the person started: it delivers sent items to the devices they were addressed to, dialing them if needed (section 5.3). It answers pings and `Hello`. It answers a request from a paired device for the items that were sent to that device. It serves staged items, which nobody addressed, to a paired device only if the person turned that on for that device, which is off by default.
- An open pairing offer ends when the device locks. Pairing needs a person to compare the codes and approve, so it is never completed on a locked device.

Why: on Windows a handler started while locked opens on a desktop nobody sees, and GNOME hides every window in lock mode, so a link that "opened" is invisible and anything it did goes unobserved. Sending is different: it shows nothing on the lock screen, and it goes only where the person pointed it. A stolen device that is locked but running also holds the device key, which section 7 covers.

### 5.5 Baton never unlocks

Baton has no feature that unlocks a device, no way to ask another device to do so, and no code that calls the platform's unlock function.

On Linux any process of the same user can call logind `Session.Unlock` without a password, and GNOME Shell and KDE's locker both obey it (reported, to be confirmed by test L6). A compromised agent could therefore unlock the machine. KDE Connect has a remote-unlock feature that is on by default (reported). The research also found that on Windows a SYSTEM-level helper can type credentials at the lock screen. Baton will build neither. A lock is not a barrier against malware that runs as the person, and Baton does not weaken it further.

### 5.6 Lock the old device after a hand-off (optional)

The person's worry is leaving the first device unlocked. Baton can address it in two ways, and both lock only after the other device has acknowledged the item:

- **Send and lock.** One gesture on the source: send these items to that device, wait for its acknowledgement, then lock this device. The lock is a local act on the device the person is using, so it needs no request to another device and no setting beyond the gesture. With a reachable target this meets the requirement in milestone 1, without waiting for the mailbox. If no acknowledgement arrives within a short wait (a setting, with a default that the tests should set), the app says so and asks whether to lock anyway. Locking anyway leaves the item in the outbox, where the locked agent keeps trying (section 5.3) for as long as the device stays awake.
- **Lock on request.** The receiving device asks the sending device to lock itself once it has acknowledged a hand-off. This is for hand-offs that were not started with "send and lock", and it is the setting described in the rules below.

| Platform | How a lock is started |
| --- | --- |
| Windows | `LockWorkStation()` from the agent in the person's session. It is asynchronous, a credential provider can deny it, and it needs a caller on the interactive desktop (documented), so it does not work from a service (inferred). |
| Linux | logind `Session.Lock()`, allowed for the session owner without a password and honoured by GNOME Shell and KDE's locker (reported). |
| macOS | No documented public API. Private functions exist and other tools use them. Baton will not depend on a private function without a fallback, so macOS is unsupported until a supported route is verified. |
| Android | `DevicePolicyManager.lockNow()` for an app that is an active device administrator, or an accessibility service's lock-screen action (documented). Both are privileged, the accessibility route has Google Play policy consequences, and whether Android's deprecation of some device-admin policies affects the first was not checked. So this is offered only as an opt-in. |
| iOS and iPadOS | None. |

The rules:

- Lock on request is off by default and set on the device that gets locked, per paired device ("let this device lock me"). "Send and lock" is the person's own gesture on that device and needs no setting.
- It happens only after the item is acknowledged. On GNOME a lock closes remote-access sessions immediately, so a transfer that depends on one would die.
- **A lock is confirmed, not assumed.** After asking for a lock the agent waits for the platform's lock signal (`WTS_SESSION_LOCK` on Windows, `LockedHint` on Linux; section 6). It tells the person the device is locked only when that signal arrives. If it does not arrive within a few seconds, the app reports that the lock was refused or failed, on the device the person is now using, because they may already have walked away.
- The trigger is the acknowledgement from a device that is unlocked and in use. Proximity is not a trigger: it needs Bluetooth signal-strength heuristics that were not researched.
- It is rate limited and the person is told when it happens. A compromised paired device could lock the person out repeatedly, which is a nuisance and not a leak.
- There is no opposite command. Locking can only make a device safer, which is why it is allowed where unlocking is not.

### 5.7 Waking a sleeping device (optional, best effort)

- **Wake-on-LAN.** A magic packet is unauthenticated and sent in the clear, it needs a sender on the same link (or a helper on that network), and whether it works depends on the hardware and driver. It wakes the hardware only. The machine is expected to come back at its lock screen, but that is inferred and is tested on every desktop (section 8), because an old device that wakes up unlocked is what the person is worried about. A network wake on a Mac may be only a Dark Wake. KDE suspends again after 30 seconds unless user activity is registered. An Android sender needs the local-network permission when it targets SDK 37 (documented). It is offered as a setting, never relied on.
- **Bonjour Sleep Proxy.** Apple devices only. The search found no client or server for Linux, Windows or Android (inferred), so it is not a cross-platform route.
- **Push.** A high-priority Firebase message can wake an Android device if it produces a visible notification, and an alert push reaches an iPhone or iPad. Both need the push gateway of milestone 4.
- **Desktops.** A suspended Linux desktop has its user processes frozen, so the routes are Wake-on-LAN, a hardware alarm, or the person opening the lid (documented, inferred). A systemd timer with `WakeSystem=` can wake it, but that is generally available only to the system service manager (documented). On Windows the research names timer-triggered Microsoft Store background tasks as the sanctioned way to run code during Modern Standby (they run only on AC power, at most every 15 minutes), and whether a package from outside the Store could use them was not researched. On macOS nothing was found. So no push route is established for desktops. The mailbox in section 5.3 is the real answer and Wake-on-LAN is a helper.
- A wake never unlocks.

### 5.8 Streaming from a device that stays locked (milestone 5)

No documented way for a third party captures a locked session on any platform examined (section 3). Apple's iPhone Mirroring is built around a locked device and keeps the iPhone locked (documented), but it works only between an iPhone and a Mac and its protocol cannot be reused. Window streaming therefore starts from these rules:

- **The first releases need the source unlocked while the stream runs**, with a visible notice on it. When the stream ends, or the hand-off completes, Baton can lock the source (section 5.6).
- **A separate session** (a second login session or a virtual display) gives the viewer a new desktop and not the windows the person left. On Linux, Chrome Remote Desktop and GNOME Remote Desktop's headless mode do this, and so does KDE's remote-access mode, which is known only from the master branch of its source code and a blog post, with its handling of the lock unfinished (reported). On Windows it is not established that Chrome Remote Desktop opens a new session and does not re-attach the existing one (W10), and a virtual display is a hypothesis only (W11). Continuity would then come from files, links and application profiles, which Baton carries (sections 5.2 and 5.3). It is heavy, and it has weaknesses of its own: KDE's mode deliberately leaves the person's session unlocked behind the login screen, so someone at the keyboard may reach it (reported, to be tested). A separate session as the same user is also not isolated from a compromised agent.
- **A privacy mode** that blanks the physical screen and blocks local input while streaming has a public-API version on macOS (RustDesk's approach, inferred from a summarised read of its source code). A person at the keyboard can defeat it, so it is a convenience and never a security feature.
- **If the source must stay locked while it streams**, the pattern the Apple research offers is the one in iPhone Mirroring: a pinned, hardware-backed device identity, an authentication step on the controlling device, a time-boxed session, and visible indicators on the source. It is a design option for milestone 5 and not a promise.
- **Nothing touches the lock screen itself.** No capture of it, and no typing into it.
- The apps promise no streaming from a locked device until the tests in section 8 show it works.

### 5.9 Keys and the lock screen

- **The device identity key must stay readable at the lock screen**, or the agent could not authenticate or receive: Keychain items of the after-first-unlock class and iOS files protected until first user authentication, Android Keystore keys without `setUnlockedDeviceRequired`, and a different home for the key where a Secret Service collection locks with the screen. [security.md](security.md) lists this under key storage.
- **Content keys for queued items may be tied to unlock where the platform has a control for it.** On Android, items can be sealed to a key that opens only on an unlocked phone (section 5.3). The cost is that a locked phone cannot show a preview of such an item, only that one arrived, which matches the notification rules in section 5.4. On iOS the keys that the notification extension needs stay in the after-first-unlock class, and before the first unlock after a restart the extension shows a generic placeholder and the item is processed after the unlock. No equivalent control was found on desktops, and whether GNOME Keyring or KWallet stay unlocked while the screen is locked was not checked.

### 5.10 Where this goes beyond what exists

As far as the research could tell, and without having tested any of these tools:

- RustDesk, AnyDesk, Parsec, KDE Connect and Chrome Remote Desktop treat a locked remote machine as something to get past: stored credentials, remote unlock, privacy modes, or a separate session. Baton treats locked as a mode to respect: stage before leaving, queue, quarantine, and never unlock.
- Sealing queued items to a key whose private half can be used only while the receiving phone is unlocked (the key itself persists) is an idea I did not come across in the hand-off tools examined (Magic Wormhole, Taildrop, KDE Connect), though I did not search them for it. It rests on a documented Android feature that has reported bugs on Android 12 to 14 (section 5.3), and it is untested.
- Locking the old device as part of a hand-off is related to RustDesk's option to lock a host when a remote session ends, to Windows Dynamic Lock, which locks a PC when a paired phone's signal falls (documented), and to the "Lock PC" action of Link to Windows, which locks a PC from an Android phone and has no unlock (weak sources). None of them is tied to the delivery of something. "Send and lock", which locks the source only after the other device has acknowledged the item, is the new part, and its value is unproven.

### 5.11 Where each rule lands

- **Milestone 1.** The per-user agent on desktops. Lock detection on Windows, macOS and Linux, with unknown treated as locked. Locked mode for received items. Resume handling. The sender outbox, which a locked agent keeps delivering, and "send and lock" on Windows and Linux. The tests in section 8, with a small probe program built for them.
- **Milestone 2.** The Android foreground service and its permission. The apps' share targets as the explicit "continue on" gesture. The iOS app works while it is in the foreground.
- **When the agent exists.** Lock on request from the receiving device, on Windows and Linux first.
- **Milestone 4.** The mailbox, the push gateway for Firebase and Apple push, wake helpers, and unlock-gated content keys on Android.
- **Milestone 5.** Window streaming under the rules in section 5.8.

## 6. Detecting lock, sleep and resume

Events are the source of truth, and a read of the current state corrects them when the agent starts and after every resume.

**Windows** (the agent runs in the person's session):

- Lock and unlock: create a hidden top-level window and register it with `WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)`, then handle `WM_WTSSESSION_CHANGE` with `WTS_SESSION_LOCK` and `WTS_SESSION_UNLOCK` (documented). Chromium and Electron do the same (reported). Whether a message-only window receives these messages is unknown, so use a top-level one.
- State at start: `WTSQuerySessionInformation` with `WTSSessionInfoEx` returns a lock flag. There is no function that simply asks whether the workstation is locked (documented).
- Sleep and resume: `RegisterSuspendResumeNotification` on the window (`PBT_APMSUSPEND`, `PBT_APMRESUMEAUTOMATIC`), or `PowerRegisterSuspendResumeNotification` with a callback when there is no window (documented).
- Display and presence: `RegisterPowerSettingNotification` for `GUID_CONSOLE_DISPLAY_STATE` and `GUID_SESSION_USER_PRESENCE` (documented).
- Fast user switching and Remote Desktop: `WTS_CONSOLE_CONNECT`, `WTS_CONSOLE_DISCONNECT`, `WTS_REMOTE_CONNECT` and `WTS_REMOTE_DISCONNECT` (documented).
- Power model: `AoAc` in `SYSTEM_POWER_CAPABILITIES` from `CallNtPowerInformation` is true on Modern Standby (documented).

**macOS:**

- Lock and unlock: the distributed notifications `com.apple.screenIsLocked` and `com.apple.screenIsUnlocked`, and `com.apple.screensaver.didstart` and `.didstop`. They are undocumented. Chromium, Electron and Hammerspoon have used them for a long time (reported), and nobody checked macOS 26 or 27.
- Fast user switching: `NSWorkspace.sessionDidResignActiveNotification` (documented). It does not fire for the screen lock.
- Sleep and wake: `NSWorkspace.willSleepNotification` (the observer may delay sleep by up to 30 seconds), `didWakeNotification`, `screensDidSleepNotification`, and `IORegisterForSystemPower` (documented).
- Polling: `CGSessionCopyCurrentDictionary()` for session ownership and `CGDisplayIsAsleep` for the display. The key that says the screen is locked, `CGSSessionScreenIsLocked`, is private and was seen only in community scripts.

**Linux:**

- Lock: the logind session property `LockedHint`, which the locker writes. GNOME and KDE set it (reported). Read it from the session's own path (`/org/freedesktop/login1/session/<id>`) and watch property changes there. Whether the `auto` alias delivers change signals is uncertain: a comment in KDE Connect's source says it does not, and other code reads the property through it (reported; L11 settles it). The `Lock` and `Unlock` signals are requests to the locker and not state, so they say nothing about whether the device is locked. `org.gnome.ScreenSaver.ActiveChanged` (GNOME, which can trail the real lock by about 0.3 seconds) and `org.freedesktop.ScreenSaver.ActiveChanged` (KDE) are also available. Other lockers (xsecurelock, i3lock, swaylock, hyprlock) were not verified to set `LockedHint`, which is why an unknown state counts as locked.
- Sleep: `PrepareForSleep` on logind's manager, together with a `sleep` delay inhibitor (documented).

**Android:**

- Screen state: `ACTION_SCREEN_OFF` and `ACTION_SCREEN_ON` through a receiver registered at run time, because the manifest cannot declare them. `ACTION_USER_PRESENT` when the lock screen has gone. Poll `KeyguardManager.isDeviceLocked()`, which honours Smart Lock. The listener APIs need a permission that ordinary apps cannot get (documented).
- Storage: `UserManager.isUserUnlocked()`, `ACTION_USER_UNLOCKED`, and `ACTION_LOCKED_BOOT_COMPLETED` for components marked `directBootAware` (documented).
- Power: `PowerManager.isDeviceIdleMode()` and `isLowPowerStandbyEnabled()`, with their broadcasts (documented).

**iOS and iPadOS:**

- There is no public API for the lock state. `UIApplication.protectedDataWillBecomeUnavailableNotification`, `protectedDataDidBecomeAvailableNotification` and `isProtectedDataAvailable` say when protected files become unreadable, which follows a lock on a device with a passcode (documented). Scene transitions happen at a lock too, but they are not a lock signal by themselves.

## 7. A locked device that is stolen while the agent runs

The agent holds the device key while the device is locked, and the network is up. What that allows, and what limits it:

| Attack | Finding | Response |
| --- | --- | --- |
| Probe the running agent over the network | A lock stops nothing, and Android keeps its foreground service alive. | In place: mutual authentication in the handshake, nothing served to unpaired devices, and rate limits (see [security.md](security.md)). |
| Use the stolen device's key to push links or files to the person's other devices | The agent can sign while locked. | Receivers open nothing on their own unless the person opted in, and show the sender. Unpairing the device on any other device ends its access. Proposed: per-device rights (links yes, files into a quarantine only) and trust that expires after a number of days without contact. |
| Items waiting in the outbox of a stolen device | A locked agent delivers sent items, so they leave the device on their own, and until then a thief holds them. | They go only to the device the person chose, and that device applies its own locked mode and policy. Unpairing it cancels them, and a lifetime (proposed: 24 hours) drops the rest. Someone who has only the locked device cannot create new items, because sending needs a gesture of the person (section 5.3). |
| Read items queued for the stolen device | The agent can decrypt while it runs. | Proposed: on Android a key that opens only on an unlocked phone, a short lifetime, and a purge on unpairing. No desktop equivalent was found. |
| Turn the agent into an unlocker | Any process of the same Linux user can call the unlock method. | Baton never unlocks (section 5.5), and the Linux agent runs as a hardened user unit. |
| Leak incoming content on the lock screen | Android visibility is user-controlled, GNOME hides notifications in lock mode, and KDE was not checked. | Hide content on a locked device (section 5.4). |
| Act on content that arrived while locked | Opening links or saving files unseen is a drive-by risk. | Locked mode is receive and quarantine only (section 5.4). |
| Attacks that skip the lock (boot from USB, DMA, cold boot) | Out of scope for the agent. Android's credential-protected storage stays available from the first unlock until a reboot. | Full-disk encryption on laptops, no unprotected long-term keys, and the knowledge that a phone unlocked once since boot is more exposed than a rebooted one. |
| Compromise of the relay, or traffic analysis | Magic Wormhole's design is the precedent. | End-to-end encryption and signatures, padding, minimal retention (section 5.3). |
| Wake a stolen sleeping laptop | The magic packet is unauthenticated. | It only wakes the machine. The machine is expected to return to its lock screen (inferred, tested on every desktop in section 8), and the risk is low if it does. |
| Lock the person out repeatedly from a compromised paired device | KDE Connect has a remote lock. | Rate limit lock requests and tell the person (section 5.6). |

## 8. To verify on real hardware

None of the behaviour in section 3 marked inferred or unknown should be relied on until these have been run. Record the operating system and build, the power model, AC or battery, wired or Wi-Fi, the disk-encryption mode, and whether the display was on or off.

A small **probe** program does the observing, and it is part of the first milestone. It writes a line with a monotonic timestamp every second, logs the lock, unlock, sleep, resume and display events that section 6 describes, and answers a TCP and a UDP echo, while a second machine probes it once a second. Each test below is "run the probe, do this, and look for gaps".

**Windows**

- W1. Read the power model and standby networking (`powercfg /a`, the wake-armed devices, adapter power settings). Decides which column of the table applies to each machine.
- W2. Liveness while locked: lock, let the display go off, wait 15 minutes, on AC and on battery. Then set a short sleep timeout and see whether the heartbeat stops before the machine visibly sleeps. Decides the first row of the Windows table and whether a display that is off suspends processes on Modern Standby.
- W3. Clipboard while locked: set it from the agent and listen for changes, then unlock and paste.
- W4. Opening a link and a file while locked, with each browser already running and not running.
- W5. Capture while locked with Desktop Duplication, Windows.Graphics.Capture, `PrintWindow` and `BitBlt`, as the person and as SYSTEM, for a window that animates. **Decides whether streaming from a locked device is possible at all.**
- W6. Modern Standby liveness and wake: enter sleep by the Start menu, the lid and the idle timeout, on AC and on battery, with and without a power request held, then send a Wake-on-LAN packet and see whether the agent runs while the display stays off.
- W7. The same on an S3 machine, including how long the machine stays awake after a Wake-on-LAN packet and whether the session comes back locked.
- W8. Before sign-in: whether a boot-start service is reachable after a restart and after a Fast Startup shutdown, with no BitLocker, TPM-only and TPM+PIN.
- W9. Lock detection over 20 cycles each of Win+L, Ctrl+Alt+Del then Lock, a lock from the agent, an idle lock, Dynamic Lock, a user switch, a Remote Desktop connection and sleep with resume, comparing a top-level window with a message-only one.
- Also: reading files while locked, including OneDrive online-only files and EFS files; toasts on the lock screen; a SYSTEM helper at the lock screen; input while locked; Remote Desktop behaviour on the console; virtual displays while locked; start-up after an update restart.

**macOS**

- M1. Liveness and network while locked: a LaunchAgent that logs a heartbeat and echoes on TCP and UDP, for 30 minutes with the display on, asleep and left to sleep, on AC and on battery, with a second account switched out, and with a screen saver and a password. Pass: no gap above two seconds and every probe answered.
- M2. Local Network grant: install with SMAppService, lock before the first LAN operation, and see what is denied, whether a prompt waits for the unlock, and whether the relay works with the grant denied.
- M3. Assertions, `caffeinate` and the lid: which assertion each creates, and which cases still sleep on AC and on battery, with the lid closed with and without an external display.
- M4. Wake-on-LAN, Sleep Proxy and scheduled wake on Apple silicon, on Ethernet and on Wi-Fi.
- M5. FileVault restart: what is reachable before the password is entered, and the SSH unlock on macOS 26 and later.
- M6. Pasteboard from the agent while locked, set and read, on macOS 27.0, 27.1 and the 27.2 beta, and Universal Clipboard to a locked Mac.
- M7. Opening a link and posting a notification while locked, with each preview setting.
- M8. Capture while locked with ScreenCaptureKit and the older capture APIs, with the display on, asleep, kept awake and under a screen saver. Decides whether streaming from a locked Mac is possible.
- M9. Input while locked through CGEvent and the Accessibility API, and whether the lock screen's password field blocks synthetic keys.
- M10. Lock detection: log the distributed notifications, the workspace notifications and the session keys through a Control-Command-Q lock, a hot corner, the screen saver, lid close and open, Touch ID, Apple Watch unlock and a restart, on macOS 26 and 27.

**Linux**

- L1. Lid close against inhibitors (`sleep` block against `handle-lid-switch`) on GNOME and KDE, with and without an external monitor.
- L2. Idle suspend while locked, and whether a `systemd --user` service may take delay and block inhibitors. With auto-suspend set to one minute, time how long a UDP echo keeps answering.
- L3. Reachability while locked: a UDP echo as a user service, `loginctl lock-session`, screen blanking, one probe every 30 seconds for two hours on GNOME, KDE and one X11 session, with Wi-Fi power save on and off.
- L4. Wake-on-LAN and Wake-on-WLAN from s2idle and S3, and how long KDE stays awake after a network wake.
- L5. What a `PrepareForSleep` hook can still send before the network goes down.
- L6. Same-user unlock: `loginctl unlock-session` over SSH and from a user service, on GNOME and KDE. Confirms section 5.5.
- L7. Whether `LockedHint` is set by `xdg-screensaver lock` on KDE, Xfce, Cinnamon and i3 with `xss-lock`, and by swaylock, i3lock and hyprlock.
- L8. GNOME: confirm that screen casts and remote-control sessions are closed on lock and refused while locked. Whether a Shell extension can use the clipboard in the lock-screen session mode. What opening a URL does while locked.
- L9. KDE: capture, input and clipboard while locked, and whether KDE's remote-access mode leaves an unlocked session reachable from a text console at the keyboard.
- L10. X11 lockers: clipboard, synthetic input and capture while locked.

**Android**

- A1. Whether a foreground service keeps the network in Doze: a UDP echo, forced deep Doze, charging and not, against a control with the service stopped. Settles a conflict between the documentation and AOSP code.
- A2. The default of Low Power Standby on Android 16 and 17 on a Pixel and a Samsung, and whether it breaks a foreground-service listener.
- A3. Latency of a packet to a naturally sleeping phone after 5, 15, 30, 60 and 120 minutes, with keep-alives every 30 seconds, 5 minutes and 15 minutes, and when the router or the phone drops the path.
- A4. Battery cost of the service over 12 hours, in three arms.
- A5. The local-network permission on Android 17: the same app built for SDK 36 and 37, granted, denied and revoked, for inbound and outbound UDP and TCP, broadcast and mDNS.
- A6. Clipboard while locked: set it from the service, check that a read returns nothing while locked, and look for access notices.
- A7. Receiving a file before and after the first unlock: a `directBootAware` service after a restart, then writes to app-private storage, MediaStore and a granted folder while locked.
- A8. Background killers on a Pixel, a Samsung, a Xiaomi and a OnePlus, for 24 and 72 hours, with default settings and then with the makers' recommended exemptions.
- A9. Keystore keys that need an unlocked device: encrypt while locked, confirm that decryption fails while locked and works after unlock, on Android 12, 13, 14 and 16 or 17 and with face unlock.
- A10. Notifications on the lock screen at each visibility level, and an action that requires authentication.

**iOS and iPadOS**

- I1. Locked reachability: ping, UDP, Bonjour and a listener probed every 10 seconds for 30 minutes, plugged in and not, with and without cellular, noting when Wi-Fi drops.
- I2. Push paths: an alert push with `mutable-content` to the notification extension, locked after the first unlock and after a restart before it. Measure latency, a keychain read, writes in each file-protection class, a pasteboard write, and a request on the local network. Then background pushes at 1, 3 and 10 an hour.
- I3. Notification presentation with each preview setting, with Face ID on and off, and for actions with and without authentication, including whether an action handler runs while the device stays locked.
- I4. ScreenCaptureKit on iOS across a lock, and whether App Store validation accepts the `screen-capture` background mode.
- I5. What an App Intent or a control may do while locked: a network request, a pasteboard write, a file write.
- I6. AirPlay mirroring from the lock screen, and a Bluetooth keyboard or pointer at the lock screen.

**Mailbox and wake**

- D1. A prototype mailbox that stores ciphertext: a locked Linux source pushes an item, an Android phone in natural deep sleep is woken by a high-priority message and pulls it. Ten trials at 5 minutes, 1 hour and overnight, including after a reboot.
- D2. Wake-on-LAN sent from an Android phone to a sleeping wired laptop, with the local-network permission granted and denied.

## 9. What this does not cover

- Everything above comes from documents, source code and reports. No device was used.
- Not researched: other Linux compositors than Mutter and KWin (Sway, Hyprland, COSMIC), the suspend defaults of distributions, push without Google services, how GNOME Keyring and KWallet behave while locked, KDE's lock-screen notification policy, Bluetooth-proximity locking, whether Android's device-admin lock is affected by its deprecations, and the default of Android's Low Power Standby.
- Not verified: TeamViewer and how AnyDesk and Parsec build their privacy modes; Dark Wake and Power Nap for third-party code on macOS; waking a Mac from the internet; the lock behaviour of iOS's newer screen-capture API; AirPlay from a locked iPhone; whether a sideloaded Windows package could run in Modern Standby.
- What would change these conclusions: a change to GNOME's policy on remote access while locked (for example an allow-list), KWin handling the lock in its screen capture, Google documenting how foreground services behave in Doze or changing Low Power Standby, the final behaviour of Android 17's local-network permission when it is denied, and any test result that contradicts an entry marked inferred.
