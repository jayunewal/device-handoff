# 0015: Continue from another device: the phone asks the Mac what is open, and the Mac reads it then

Status: proposed. The messages, the setting for each device and the node are built on `main` and tested with two nodes on loopback. Reading the page and the clipboard on a Mac, and the screen on the phone, are the next change, and the checks on real devices in this record are not run yet.

## Context

Baton hands things off from the device that is being used: the person sends a link, some text or files to another device. It cannot do the other thing. A person who sits down at the phone and wants what was in front of them on the Mac has to go back to the Mac, send it and come again. The review of Baton against its goal (2026-11-10) found this the thin half of handoff and put a way to pull first: the phone asks the Mac for the page that is open and the text that was copied, and opens or copies them with one tap.

What is known:

- Section 10 of the protocol and [locked-devices.md](../locked-devices.md) already plan a pull, as a list of what a device has ready (`ListActivities`) and a fetch of one of them (`Fetch`), over items that the device *stages*: it reads the clipboard and the tab as the person works, keeps them, and serves them later. That is what lets a locked or sleeping device answer, and it is also a device that reads what its person does all day and keeps it. Nothing in the app stages anything today.
- What a person is working on is private in ways that a title bar hides. A page address can carry a sign-in token, a title can name a document, and the clipboard can hold a password. The person has to have chosen who may read it, and a device that they did not choose must get nothing.
- Asking the person at the Mac is no use, because the phone asks when the person is not at the Mac. [ADR 0010](0010-sessions-that-the-viewer-starts.md) met the same thing for screens and settled it by allowing a paired device once, at the Mac.
- The Mac can read the page that is in front in Safari and in Chrome with Apple Events, which macOS lets a program do only after the person said yes to a question that names the browser, and the clipboard with `NSPasteboard`. Both are calls to a program that is running, so they can be made when somebody asks, and need no staging. How long they take is not known yet (C1).

## Decision

**The phone asks, and the Mac reads then.** A new request, `Continue`, carries an empty question, `Ask`, and the answer is `Items`: the page that is in front in the browser (an address and a title, as a link has) and the text on the clipboard, each left out when there is none. Nothing is read in advance, nothing is kept after the answer, and neither the node nor the app layer holds any of it: the platform of the Mac reads when the request arrives, and the answer is as old as the question. A device advertises `continue.serve` when its platform can read; a device that does not is told `Unsupported`.

**It is off until the person at the Mac turns it on, for each device.** The policy of a paired device has one more setting, `continuing`, off by default. Only the person at the Mac changes it, in the settings for that device, and nothing a paired device sends can. It is not written to the trust file while it is off, so a version that does not know it reads the file as it wrote it. A device that is not allowed is answered `Declined(Blocked)`, which the phone shows as "turn it on at the Mac", and it is not read for and does not count against the limit below. Unpairing removes the setting with the rest of the policy.

**Nothing is read that the person did not put in front of the Mac.** These are the rules for the platform, which the node cannot check and the interface says: a Mac that is locked, or whose display is off, gives an empty `Items` and not what it last held, and says nothing of why, so that a phone cannot tell a locked Mac from one with nothing open; clipboard text that its source marked as concealed or transient (password managers do) is never read; a private window of Chrome is left out; and the browser that is read is the one in front, or the one that was in front last, and one that is not running is not started. An answer with neither part is the right answer in each of these cases.

**An allowed device is limited like any sender, and is read for one question at a time.** A request counts as one item against the limit for items of each paired device (`handoff.max_items_per_minute`), once the device is allowed. The node answers one request at a time, and a second while one is being read gets `RateLimited`. The platform has 5 seconds to read: one that takes longer is given up on and the phone is told `Other`, and the phone waits those 5 seconds and 10 more for the network. A page or a text beyond the limits of a link and of clipboard text is dropped by the node, and the other part is still given.

**The phone acts on a tap.** It shows what it was given as a card for the page and a card for the text, each with a button: Open, which goes through `web_address` as a received link does, so only `http` and `https`, and Copy. Nothing opens and nothing is copied before the tap, and nothing is kept in the inbox.

**What goes on the wire, and where the code lives.** One capability, one request and its reply, in section 3 of the protocol. `Ask` is a case of an enum and not a bare struct on purpose: a request is read by the name of its case, so a response can never be read as a request, which a struct of keys that a reader ignores would allow (the decoder ignores keys it does not know). A case that a version does not know is an error, as for every request. The messages and their limits are in `baton-proto`. The rules (the setting, the limit, one at a time, the time, what is given) are in `baton-core`, tested with two real nodes on loopback. `baton-app` gives the platform an event to read for (`ContinueAsked`) and a call to answer with (`answer_continue`), and the bridge copies them. The Swift code of the Mac reads, and the Dart screens ask and show.

## How it will be measured

Without Apple hardware, on every pull request: the messages have known encodings and are fuzzed, and the compatibility tests hold the rules that keep old and new versions apart (keys that are ignored, a reason that is not known, the two directions not read as each other, a part over its limit); the node is tested with two real nodes (a device that is not allowed is not read for and not counted, one question at a time, the time, the limits of what is given, a device with no way to read); the app layer is tested with a platform that the test plays, including a phone that gives up, a platform that is too slow and an app with nobody to read for it.

On a Mac and an iPhone, as in ADR 0009, with the builds, models and network recorded. Each check is done by the person at the devices, with the steps in the pull request that adds the code.

- **C1. Safari.** Open a page, ask from the phone. macOS asks once whether Baton may control Safari; after the yes the phone is given the title and the address. How long from the tap to the cards, with the Mac's window closed.
- **C2. Chrome.** The same. A window in incognito is left out.
- **C3. A private window in Safari.** Whether Apple Events show it, and what the phone is given. The limit is written down in the settings text either way.
- **C4. A no.** Answer "Don't Allow" at the question. What the phone says, and that the settings for Automation in System Settings bring it back.
- **C5. Locked.** A locked Mac and a Mac with its display off give nothing, and the phone says that there is nothing to continue.
- **C6. Concealed.** Copy a password from a password manager: the phone is not given it. Copy plain text: it is.
- **C7. No window.** With the window of Baton closed and only the menu bar item left, ask from the phone.
- **C8. Off.** Turn the setting off at the Mac: the next ask is turned away without a question and without a read.

## Alternatives considered

- **Stage on the Mac and serve later** (the plan in section 10). It answers while the Mac is locked or asleep, but it reads and keeps what the person does all day, needs a list, a fetch and a store with rules of their own, and serves what may be hours old. Asking at the time gets the same page with nothing kept. A Mac that is asleep gives nothing here, and the plan stays open for it.
- **Ask the person at the Mac each time.** Nobody is there.
- **Push what is open to the phone whenever it changes.** It keeps the phone's picture of the Mac fresh at the cost of battery and network, and shows what the person reads to a device that may not be in their hand. A pull is enough.
- **A browser extension that reports the tab.** It works in any browser, and needs one install for each browser and profile, a way to reach Baton and a listing in a store. Apple Events need one question. An extension remains the way to add Firefox and the others later.
- **A list of items in the reply.** It is more general, and each new kind would then need its own check of what the other device understands. Two named parts keep the limits plain, and a later version can add a part, which an older reader ignores.

## Consequences

- An allowed phone can read the address and title of the page in front in the Mac's browser and the text on its clipboard whenever the Mac is unlocked and Baton is running, for as long as the person leaves the setting on. The text of the setting says so. The Mac shows no notice when a phone reads in this version. The event that tells the platform to read names the device that asks, so a notice can be added without a protocol change.
- The Mac has to be running Baton. The menu bar item of the app, which stays on without a window (the change before this one), is what lets the phone's question reach a Mac that nobody sits at.
- Reading needs the Automation permission for each browser, and macOS ties it to the signature of the app. A build that is signed ad hoc and differently each time asks again; stable signing is a step of its own.
- Only a Mac reads. A device whose platform does not advertises nothing, and a phone that asks it is told that it cannot.
- The trust file gets one more optional key. It gives an unpaired device nothing.

## Open points

- A notice at the Mac when a phone reads: "Pocket Phone took the page that you were reading".
- Whether Safari shows its private windows to Apple Events (C3).
- Browsers other than Safari and Chrome.
