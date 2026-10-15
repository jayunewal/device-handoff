# Architecture

This document describes how Baton is put together. The trust model is in [security.md](security.md). The wire format is specified in [protocol.md](protocol.md).

## Layers

```
 device A                                        device B
┌──────────────────────────────┐                ┌──────────────────────────────┐
│ UI: command line, later      │                │ UI                           │
│ Flutter apps                 │                │                              │
├──────────────────────────────┤                ├──────────────────────────────┤
│ baton-agent                  │                │ baton-agent                  │
│ clipboard, opening links,    │                │                              │
│ acting on policies           │                │                              │
├──────────────────────────────┤                ├──────────────────────────────┤
│ baton-core                   │  QUIC + TLS    │ baton-core                   │
│ identity, trust store,       │◄──────────────►│                              │
│ pairing, handoff, inbox      │  (via iroh)    │                              │
├──────────────────────────────┤                ├──────────────────────────────┤
│ baton-proto                  │                │ baton-proto                  │
│ message types, framing       │                │                              │
└──────────────────────────────┘                └──────────────────────────────┘
```

| Layer | Responsibility | Platform code? |
| --- | --- | --- |
| `baton-proto` | Message types, framing and the pairing ticket format. No I/O, so it can be tested and fuzzed on its own. | No |
| `baton-core` | Device identity, the trust store, pairing, discovery, the encrypted transport, the handoff service and the inbox where received items rest. Everything that must behave identically on every platform. | No |
| `baton-agent` | The parts that touch the desktop: reading and writing the clipboard, opening links, and acting on the per-device policies that open or apply what arrives instead of leaving it in the inbox. | Yes (desktop) |
| `baton-app` | What a graphical front end calls: start the node, pair, send, read the inbox and hear what happens, as plain values and a stream of events. It puts the core's parts in the order a screen needs them and holds no protocol logic. | No |
| `baton-ffi` | The bridge to Dart: Dart-friendly copies of `baton-app`'s values and calls, and the glue that `flutter_rust_bridge` generates around them. It decides nothing. | Generated glue only |
| UI | Presents devices, pairing and received items. The command-line program (`baton-cli`, the `baton` binary) came first; the Flutter app in `apps/baton`, starting with iPhone, iPad and Mac, calls `baton-app` through `baton-ffi`. | Yes |

The Flutter app reaches `baton-app` through `baton-ffi` (see [The bridge](#the-bridge)), the only crate in the repository that holds `unsafe` code, and only in the glue it generates. On mobile the app implements the equivalent of `baton-agent` natively (share sheet, Shortcuts, Android intents), because those platforms do not let a background process read the clipboard or open links the way a desktop agent does.

## The command-line program

`baton-cli` builds the `baton` program, the first front end. It holds no protocol logic: it opens a state directory, starts a `Node` from `baton-core` when a command needs the network, and turns what the core reports into text for a person.

- **One-shot commands** (`id`, `devices`, `pair offer`, `pair join`, `unpair`, `ping`, `send`, `inbox`) do one thing and exit. **`baton listen`** keeps a node running so that other devices can reach this one, announces what arrives, and doubles as a prompt that takes the same commands without the word `baton`. A device can be reached only while something is listening, and a pairing offer is made at the same prompt so that the address in its ticket is still the right one when the other device joins.
- **Only one process runs the node.** The commands that start the network or change the list of devices take `baton.lock` first and say so when another window already holds it. Reading needs no lock: `baton inbox` and `baton devices` work while `baton listen` runs, because the inbox is a file with a lock of its own (see [State on disk](#state-on-disk)).
- **Questions and the prompt share standard input.** One thread reads lines. A line goes to the question that is waiting for an answer, such as "type yes to pair", and otherwise to the prompt, so a pairing request can interrupt a listening prompt without the two taking each other's lines. A question is registered before it is printed, so an answer that arrives at once, as when input is piped, is never taken for a command. Questions are asked one at a time: a pairing request that arrives while the person is answering `inbox clear` waits until that command has printed its result, so a `yes` meant for one question cannot answer another.
- **Everything another device chose is escaped before it is printed**, error messages included (some carry the reason a device gave for hanging up), and only `http` and `https` links are opened ([security.md](security.md)). Nothing is opened, copied or applied by itself: the program has no agent and cannot yet tell whether the device is locked, so it only keeps what arrives.
- **Files are opened by the commands that need them.** An inbox file that cannot be read stops `baton inbox` and `baton listen`, which would have nowhere to keep what arrives, and says how to start a new one. It does not stop `baton id`, `baton devices` or `baton unpair`.

## The app layer

`baton-app` is what a graphical front end calls. Like the command-line program it holds no protocol logic. It puts the core's parts in the order a screen needs them, so that each front end does not do that again and get it wrong in its own way. A front end written in another language reaches it through a bridge that copies the values below and keeps nothing of its own.

- **One object, plain values.** `App::start` opens the state directory, takes its lock, loads the identity, the paired devices and the inbox, and starts the node. Everything it hands back is plain data a bridge can copy without knowing Baton's types: ids are 64 hexadecimal characters, times are seconds since the Unix epoch, and a device is shown by its name, system and fingerprint. `stop` can be called more than once and from anywhere: the work runs on a task of its own, and every call returns once the network is down and the folder is free. Neither a question waiting for the person nor an offer that is showing keeps a dropped app alive: dropping the last copy ends everything that runs in the background and the stream of events, though without telling the connected devices, which `stop` does.
- **Screens hear what happens.** `App::events` is a stream of changes: an item arrived, the list of devices changed, a device is asking to pair, a pairing offer ended. A screen that falls too far behind gets one "resync" event and reads the lists again, so a slow screen can neither stall the app nor make it hold events without limit. Reading the lists (`devices`, `inbox`) never waits for the network.
- **Pairing is two calls and an answer.** The device that shows a ticket calls `offer_pairing`, which returns the ticket to show as text or a QR code and later reports how the offer ended: paired, expired, withdrawn, declined, abandoned or failed. The device that has the ticket calls `join_pairing`, which hands over the six-digit code to show while it waits. When someone joins, the offering side gets a request with the other device's name, system, fingerprint and code, and answers it with `answer_pairing`. A request that cannot be answered any more, because the offer was withdrawn or the other device left, is taken off the screen with an event, and a late answer to it counts for nothing. One offer and one join are open at a time, and a new offer replaces the old one; offers are made one after another, in the order they were asked for, so that two quick taps cannot leave the older offer showing. A "yes" cannot be taken back once the node has claimed it, and the screen is always told what happened to the devices. A screen that withdraws the offer just after the yes is told whichever of the two reached the node first: that the devices paired, or that the offer was withdrawn and the other device turned away. A pairing screen therefore leaves the offer alone after a yes. The offer ends by itself, with an event.
- **Errors say what a screen can do about them.** Every failure carries a kind for the screen to choose its words by, such as the other device being unreachable or the folder being in use by another Baton, and a message of one short line that is safe to show: whatever another device chose to say has been cleaned first. Tickets and the content of received items are kept out of `Debug` output, so logging a value cannot leak them. A received link is a string another device chose, so the layer says which ones a screen may open and what to open (`web_address`: the address of an `http` or `https` link in the form that was checked, as the command-line program does) instead of leaving each screen to remember. A screen opens that address and not the text that was sent, because another parser can read the same text differently.
- **The name is the person's to choose.** The core falls back to a name taken from the computer, which on a phone is rarely one a person would pick. The app asks for a name the first time and saves it with `settings::set_device_name` before it starts (a name with nothing visible in it is refused with a kind of its own, so the name field can say so), and `Options::lan_discovery` and `Options::port_mapping` override those settings for one run. On iOS the app turns both off, because multicast needs an entitlement that a free Apple ID cannot sign with, and asking the router for a port starts with a multicast message; until discovery through Apple's own service is added, an iPhone or iPad finds the other device through the relay and then moves to a direct path. That follows Apple's documentation and has not been tried on a device yet.
- **No platform code.** The layer runs the same on every system and is tested on Linux, macOS and Windows with two copies on one machine. The screens, the camera that reads a QR code and the share sheet belong to the front end.

## The bridge

`baton-ffi` is what lets Dart call `baton-app`. It decides nothing: a call goes straight to the app layer and what comes back is copied into plain values, so the bridge cannot let a device do more than the app layer does.

- **A written API and generated glue.** `crates/baton-ffi/src/api` is the whole surface: copies of the app layer's values (`model.rs`, `error.rs`), one object, `Baton`, that wraps a running `App` (`baton.rs`), and the few functions that run before it starts, which choose and save the device's name. [flutter_rust_bridge](https://github.com/fzyzcjy/flutter_rust_bridge) turns those into Dart classes in `apps/baton/lib/src/rust` and writes the glue on both sides. The generated files are committed, and CI writes them again and fails if they differ, so a change to the API cannot reach Dart half done. The Rust crate, the Dart package and the code generator must be the same version (2.13.0), and each says so where it is set.
- **What the app sees.** `Baton.start` returns the running object, whose methods are the app layer's. A few calls that cannot wait for anything are synchronous (`me`, `answerPairing`, `cancelOffer`, `cancelJoin` and `webAddress`); the rest return a `Future`. A failure throws a `BatonError`, whose `kind` the screen chooses its words by and whose `message` is one line that is safe to show, as in the app layer. `joinPairing()` returns a `Stream` of one pairing's progress: the code to show, then the device that was paired or the reason it was not. `events()` returns a `Future` of a `Stream`, and the stream carries events from the moment that future completes until the app stops, so a screen awaits it before it reads the devices and the inbox and loses nothing in between. The stream does not keep the `Baton` alive: one that Dart lets go of without calling `stop` ends it, and the folder is free again. A join that fails ends the stream with an item, not a thrown error, so a screen handles the outcome in one place.
- **Dart prints what it holds.** On the Rust side, `Debug` output leaves out tickets and the content of received items. The classes the generator writes for choices (`Event`, `JoinProgress`, `OfferOutcome` and `ReceivedItem`) do not: `ReceivedItem.toString()` includes the link or the text that was sent, which may be a password or a sign-in link, and `JoinProgress.code` the pairing code. A screen does not print or log these, or put them in an error message; it shows what the person needs to see and nothing else.
- **A quiet stream cannot be cancelled at once.** Dart builds these streams from an `async*` generator, and cancelling one finishes only when the generator next yields. The `Future` that `cancel()` returns therefore stays open until the next event arrives. A screen holds one `events()` subscription for as long as the app runs, lets `stop` end it, and does not wait for `cancel()` where it has other work to do.
- **`unsafe` stays in the generated module.** The workspace forbids `unsafe_code`, and a forbid cannot be lifted, so this crate lists its lints itself: `unsafe_code` is denied everywhere, with an allow on the generated module alone. The rest of the list is copied from the workspace, and a test fails if the two drift apart.
- **How the library reaches the app.** For iOS and macOS the Xcode build runs [Cargokit](https://github.com/irondash/cargokit), which builds `baton-ffi` as a static library and links it in (`apps/baton/rust_builder` holds the copy and its MIT license). The Dart tests load the library built for the computer they run on instead; [apps/baton/README.md](../apps/baton/README.md) says how.

## Connecting devices

Every device has an Ed25519 key pair. The public key is the device's identity. Devices dial each other by that key, and the QUIC/TLS handshake proves both sides hold the keys they claim, so a connection to a given identity cannot be impersonated.

Reaching a device uses the best path available, tried in this order:

1. **Local network.** Devices announce themselves on the LAN with mDNS and connect directly.
2. **Direct over the internet.** A relay helps both sides learn each other's public addresses and hole-punch through NAT.
3. **Relay.** If no direct path works, traffic flows through a relay. The relay forwards encrypted packets addressed to a public key. It cannot read or alter them.

The relays Baton uses by default are the iroh project's public ones. According to iroh's documentation they are rate limited and meant for development and hobby use, so a transfer that cannot go direct is only as fast as that shared limit. That is fine for trying Baton out. The self-hostable server runs a relay of its own with limits its owner sets, and the file transfer benchmark includes a relayed run so the cost shows up in the numbers.

Connection handling comes from [iroh](https://github.com/n0-computer/iroh) (see [ADR 0003](adr/0003-iroh-transport.md)). The rest of Baton talks to it through a small boundary in `baton-core` so the transport can change.

What a device knows about how to reach another is kept with the pairing: the ticket carries the offerer's relay and direct addresses, and after every connection each side remembers the relay and addresses that worked. A device that restarts can therefore reach its paired devices before discovery has found anything, and devices that cannot find each other any other way still connect through a relay.

## Inside baton-core

| Module | Job |
| --- | --- |
| `identity`, `trust`, `config`, `paths` | The device key, the list of paired devices, settings, and the state directory. |
| `node` | Starts and stops everything: the network endpoint, discovery and the router. A `Node` is what an application holds. |
| `gate` | Decides who may connect: `Admission` before the handshake, `Gate` after it (which also raises the connection window for paired devices), and a registry of open connections so that unpairing can close them. |
| `pairing` | Offers, tickets, approval, and joining another device's offer. |
| `session` | The `baton/1` protocol: serving requests, the pool of outgoing connections, and `Intake`, which checks every incoming item before the application sees it. |
| `transport` | The QUIC settings every connection uses: how many streams a peer may open, keep-alives, and the flow-control windows that set how fast one stream can go and how much unread data a peer can make a device hold. Connections start with a small window and the gate raises it for paired devices. |
| `ratelimit`, `wire`, `link`, `clock`, `util` | Small shared parts: a sliding-window limiter, reading and writing frames with deadlines, describing which path a connection took, a clock that tests can control, and helpers that keep one failing task from taking others down. |

An incoming connection passes through these checks, in this order:

```
incoming packet
   │  Admission     Asks the sender to prove its address first when attempts arrive faster than
   │                the node wants to handle. Costs the node a small reply, not a handshake.
   ▼
QUIC and TLS handshake        both ends are now known by their keys
   │  Gate          baton/1 from a key that is not paired: closed with 403, nothing remembered.
   │                baton/1 from a paired key: the connection's window is raised.
   │                baton-pair/1: allowed, because the person is not paired yet, and left small.
   ▼
SessionServer or PairServer   each connection runs as a task of its own, so a bug reachable
   │                          from the network cannot stop the node accepting connections
   │  per request   Still paired? Then Intake: kind of item, size, repeat, rate limit.
   ▼
the application's HandoffHandler
```

Outgoing requests use a pool that keeps one connection per paired device. It opens the connection on first use and says `Hello`, reuses it for later requests, retries once on a new connection if a reused one turned out to be dead, and closes connections that have been idle for two minutes. Unpairing a device closes its connections, whichever side opened them.

## Trust

A device only talks to devices it has paired with. Pairing is explicit: one device offers a one-time ticket (shown as a QR code or text), the other redeems it, and the person on the offering device approves the request after checking that a six-digit code shown on both devices is the same. There is no way to switch the approval off. Both sides then remember each other's public key in a local trust store (`devices.json`, see [State on disk](#state-on-disk)). Connections from any other key are refused after the handshake, before any application data is read. The check is repeated for each request, so unpairing a device ends its access at once, including on connections that are already open.

There are no accounts in milestone 1. Accounts arrive in milestone 4 as an *introducer*: a server that tells your devices about each other. It is never in the data path and never holds the keys needed to read content.

### Same-account sync: a signed roster (design, not built)

Pairing every pair of devices by hand is how tools such as KDE Connect work, and it is tedious past three or four devices. The plan for milestone 4 is a *roster* instead: the list of a person's device keys, kept as a chain of membership changes.

- Each change, adding a device or removing one, is signed by a device that is already in the roster and carries the hash of the change before it. A device checks the whole chain from the first entry before it trusts anything in it.
- Pairing a new phone with any one device adds it to the roster. The others trust it because the chain is signed by a key they already trust, with no further pairing.
- The account server only relays roster changes. It holds no roster key, so it cannot add a device on its own. A compromised server could hold updates back or replay old ones, which the chain and a sequence number make visible, but it could not forge a device. That keeps the promise that the server is an introducer and never in the data path, even if it is compromised.
- A removal is a signed entry too. A device that sees it drops the key like a manual unpairing, including closing live connections.

Open questions: what to do when two devices change the roster at the same time (a fork), how a person recovers when every device is lost, and whether a device that was offline for a long time should accept a long chain without asking.

The trust store leaves room for this before its format is in use: each entry records how the device came to be trusted (`provenance`, today only `paired`), and a device trusted through the roster will also say who vouched for it.

## State on disk

Each device keeps its state in one directory, which belongs to a single device identity. By default that is the per-user *local* data directory (`~/.local/share/baton` on Linux, `~/Library/Application Support/baton` on macOS, `%LOCALAPPDATA%\baton\data` on Windows), never a roaming or synced one, because a copied device key would make two machines the same device. As a second line of defence the identity file records which machine made the key, and Baton refuses it on another one. Set `BATON_HOME` to use another directory, for example to run two instances on one machine.

| File | Contents | Notes |
| --- | --- | --- |
| `identity.toml` | The device's Ed25519 secret key. | Created on first run. Readable by its owner only; Baton refuses to use it if other users can read or change it. |
| `devices.json` | The trust store: paired devices, the name the person knows each by, how each came to be trusted, their last known addresses and their per-device policies. | Replaced atomically on every change. A file that cannot be read is an error, never an empty list. Refused if other users can read or change it. |
| `config.toml` | Settings a person edits by hand (device name, relays, discovery, limits). | Optional. Unknown keys are rejected so a typo cannot silently leave a setting on. Refused if other users can change it. |
| `inbox.json` | Links and text received and not yet dealt with, each with its sender, the time and a number that counts up and is never reused. | Replaced atomically on every change. At most `handoff.max_inbox_items` items and 3 MiB, the oldest dropped first. Readable by its owner only, because clipboard text can be a password. A file that cannot be read is an error, never an empty inbox. |
| `inbox.lock` | An advisory lock held for the length of one change to the inbox. | The listener adds to the inbox while other commands read and clear it, so unlike the rest of the state it is changed by several processes. A change takes this lock and reads the file again first. Reading takes no lock. |
| `baton.lock` | An advisory lock. | Only one process may run a node in a state directory at a time. |

State files are written to a temporary file in the same directory, flushed, and renamed over the target, so a crash leaves the old file or the new one. On Unix, files and a directory that Baton creates are readable by their owner only. A state directory that already exists is not changed, because it may be used for other things (think `BATON_HOME=$HOME`), but it is refused if other users can write to it, since they could replace the files in it. `identity.toml`, `devices.json` and `inbox.json` are refused if other users can read or change them, and `config.toml` if they can change it; the check is made on the file that is then read. Files are read with a size cap, and a file written by a newer, incompatible version is refused rather than misread or overwritten.

## Handoff

Moving something to another device is a *handoff item*: a link, some clipboard text, and later files and tabs. The receiver does not blindly act on an item. What happens next is decided by a per-device policy on the receiving side. By default received links and clipboard text go to an inbox and nothing is opened or overwritten until the person chooses; auto-open and auto-apply are opt-in, per device. See [security.md](security.md) for why.

The inbox is part of `baton-core`: `InboxHandler` is the handler that keeps a link or some clipboard text in `inbox.json` and answers the sender, and the command-line program uses it. A device set to ignore links, or clipboard text, from another device declines them as blocked. Everything else is kept, including links set to open and text set to apply, because those settings are carried out by the agent, which comes with the locked-device rules below, and a device that cannot yet tell whether it is locked should not open anything. The agent will act on an item that is already in the inbox, so a setting never changes what is stored.

## Locked devices

Handing work off means walking away from the first device, so that device is usually locked and often asleep. Baton has to work in that state: an app that needs the source unlocked fails at its one job. [locked-devices.md](locked-devices.md) records what each operating system allows, with the evidence and the tests still to run on real hardware. The rules below are the short form of what follows from it.

- **The agent does not need an unlocked session.** On desktops it is a per-user background process that starts at sign-in, not a system service. A lock should leave it running (reported on Linux, inferred on Windows and macOS until the tests in locked-devices.md have been run). Sleep and a restart before sign-in do not. Receiving items, answering pings and delivering what the person sent should work with the screen locked.
- **Stage before leaving.** What a person might want to continue cannot be counted on to be readable once the device locks, and on phones it is readable only while the app is in the foreground. So a hand-off starts while the person is at the device: an explicit "continue on" gesture that names the device or, where the platform allows it and the person turns it on, a clipboard that is staged continuously. Items sent by a gesture wait in an outbox for their device. The lock event itself sends nothing new, because many locks are idle timeouts and a push at every one would deliver what nobody asked to move.
- **A hand-off does not need the source afterwards.** An item goes directly when both devices are awake, waits in the sender's outbox otherwise, and, once accounts exist (milestone 4), waits encrypted in a mailbox on the server. A locked sender keeps delivering what the person explicitly sent, including by dialing the device it was sent to, and a "send and lock" gesture waits for the other device's acknowledgement before it locks the source. Only the mailbox works when the sender has gone to sleep before the other device appears.
- **Locked mode.** While a device is locked, or when it cannot tell, it accepts items into the inbox and does nothing else with them: nothing is opened or applied. It starts no hand-off of its own, but it finishes the ones the person started, delivering what was explicitly sent to the device it was sent to. It serves staged items, which nobody addressed, to a paired device only if the person allowed that for that device, which is off by default. A locked device refuses to start a send, and every phone action that starts one asks for authentication (see [locked-devices.md](locked-devices.md#52-stage-before-leaving-and-do-not-read-a-locked-device)).
- **Baton never unlocks anything.** There is no remote-unlock feature, now or later. Where the platform allows it, the source can lock itself once the other device has acknowledged a hand-off ("send and lock"), and a paired device can be allowed to ask for that lock, which is off by default.
- **Keys stay readable at the lock screen.** The device key and the trust store live in storage that is available after the first unlock of the device, not in storage that is sealed whenever the screen locks. A keystore that locks with the screen would make the agent deaf exactly when it is needed.
- **Sleep is the real limit.** Waking a device is Wake-on-LAN on desktops and push notifications through a gateway on phones (milestone 4). Both are best effort and nothing depends on them, and after every resume all connections are treated as dead.
- **Limits stay visible.** Some things cannot be done on a locked device, such as capturing a window from a locked desktop. No documented API allows it on any platform examined. Those limits are documented per platform, not hidden.

## Platform boundaries

Some things only native code can do, and they are kept out of the core:

| Capability | macOS | Windows | Linux | Android | iOS and iPadOS |
| --- | --- | --- | --- | --- | --- |
| Clipboard | Pasteboard | Win32 clipboard | X11 or Wayland | ClipboardManager (foreground only) | UIPasteboard (foreground only, paste banner) |
| Open a link | Launch Services | ShellExecute | xdg-open | Intents | UIApplication |
| Window capture (milestone 5) | ScreenCaptureKit | Windows.Graphics.Capture | PipeWire | MediaProjection | ReplayKit (view only) |
| Background connection | Yes | Yes | Yes | Foreground service | No; push notification wakes the app |

## Non-goals

- Extracting the internal state of arbitrary apps. No platform allows it.
- Remote control of iOS or iPadOS through Apple's private interfaces.
- Being a general remote desktop or VPN product.
