# 0016: The outbox: a link or some text for a device that is not there waits on the device that sent it

Status: proposed. Everything that can be built and checked without Apple hardware is built: the list and its file, the loop that sends what waits, the app layer, the bridge and the screens, tested with a courier that the tests play, with two real nodes on loopback (the device that is away is one that has unpaired the sender), and with a session that the screens' tests play. Nothing has been tried on a Mac and a phone: checks O1 to O6 below are for the person at the devices.

## Context

A person sends a link to the phone while Baton is closed on the phone. The dial gives up after the node's connect timeout (20 seconds), the screen says that the device could not be reached, and the person has to remember to send it again when they are at the phone. That is the common case, not a rare one: in this version a device receives only while Baton is running on it, and an iPhone or an iPad runs Baton only while it is on the screen. Waking a phone that is locked, or has Baton in the background, needs a push notification, which needs a paid Apple developer account and a server, and neither exists.

What is known:

- [Section 5.3 of locked-devices.md](../locked-devices.md#53-a-hand-off-must-not-need-the-source-afterwards) already names three paths for an item: direct, a sender outbox that waits on the sending device until the other can be reached, and a mailbox on an account server that works when the sender has gone to sleep too. It leaves the lifetime open ("proposed: 24 hours").
- The review of Baton against its goal (2026-11-10) found that what the Mac sends is lost to anyone who is not holding the phone with Baton open at that second, and made "what the Mac sends waits until Baton opens on the phone" a step of its own.
- The Mac app keeps running with its window closed, in the menu bar. A Baton that runs all day can try again; a Baton on a phone can try only while it is open.
- What a person sends can be a password, or an address with a token in it. A list that shows what is waiting is a list that someone can look at over a shoulder, and a file that holds it is a file that is read when the device is lost.

## Decision

**A send to a device that cannot be reached waits, and is not an error.** `send_link` and `send_text` dial as before. An answer of "not reachable" is put in the outbox instead of failing, and the call says so: `SendResult::Waiting`, where a device that took the item gives `SendResult::Delivered`. Every other failure is still a failure and nothing waits for it: a device that answered and did not take the item, an item that is not a web address or is too long, a device that is not paired, an app that has stopped. The command-line program does not go through the app layer and is unchanged: `baton send` still fails at once.

**It waits on the sender, in a file.** `outbox.json` in the state directory, readable by its owner only, holds what waits in the order it was queued. The item is kept as it was sent, not encrypted: the identity key of the device is a file in the same folder, so a key beside the items would add nothing, and the file is as open as the inbox is. It is read leniently, because an outbox that is lost loses a few links and one that cannot start loses the app: a file that cannot be read, or is of another format, is an empty outbox, an entry that makes no sense (an item over the limits of the protocol, say) is left out and the others are kept, a file with more entries than there is room for keeps the oldest, and the name of a device is made clean. A change that cannot be written is logged and kept in memory, so what waits is still sent while Baton runs.

**Only links and text, only to the device that was named.** Files do not wait: an offer of files needs the person at the other end to say yes while the files are where they were, and a transfer that starts a day late is a surprise. An item is never widened to another device. Unpairing a device drops everything that waits for it, and says nothing, because the person did that and knows why.

**For a day, and not for ever.** An item is dropped when a day has gone by since the person sent it, counted on the wall clock, because that is the only clock that goes on while Baton is not running. A clock that is set forward cuts the time short, and an item that the clock says was queued in the future is counted from now, so a clock that was set back by days does not keep it for days. At most 50 items wait, at most 20 for one device, and the file never grows past 3 MiB. An item that does not fit is refused, and the person is told, with the number: what waits is never dropped to make room, because it was promised. The send then ends with the sentence "Could not reach … and nothing more can wait for it".

**A loop sends it, and finds out by dialing.** One task in the app layer looks at the outbox when an item is added, when the list of paired devices changes, when a device takes something that the person sent it at once while other items wait for it, and after a pause. The pauses start at two seconds and double to half a minute, and they start again whenever something moves; while nothing waits, nothing is looked at. One look takes each device that something waits for on a task of its own, so a device that does not answer does not hold up one that does, and sends the items for a device oldest first, stopping at the first that cannot go, so a device that is away is dialed once for each look and not once for each item. Nothing else tells the loop that a device has come back, because a connection from a paired device changes nothing in the list: a device that comes back is sent what waits within the longest pause plus the time of a dial, and not at once, unless the person sends it something first and it takes that.

**Four ways to end, and two of them are the person's.**

- The device took it: `OutboxEnd::Delivered`.
- The device answered and did not take it, for a reason that waiting does not change (its settings for this device do not allow the kind, or it does not handle it): the item is dropped and the person is told why (`Refused`). A device that is receiving too fast from this one is the exception: that passes, and the item waits for the next look.
- A day went by without reaching the device (`Expired`).
- The person cancelled it, or unpaired the device. Nothing is said, beyond `Event::OutboxChanged`, which every change to the list sends.

The first three come as `Event::OutboxSettled`, with one sentence for the screen to show as it is.

**The list says what an item is, and never what it says.** "A link (example.com)" and "some text (23 characters)": the host of a link, and the number of characters of a text. The sentences of the endings use the same words. A notification shows on a screen that anyone may look at, so none is posted for these in this version (see the open points).

**An item is sent at least once.** An item is struck out of the file when the device's answer comes. If Baton closes between the device taking it and the file being written, the item goes again when Baton next runs, and the device may show it twice. Nothing can make the two steps one.

**The screens.** The send screen says that what was sent waits, in a line with a clock, and lists what waits for that device under **Waiting to be sent**, each with the time it has been waiting, when Baton gives up, and a **Cancel**. The line goes when nothing waits any more. The home screen shows each ending as a sentence at the bottom, on whatever screen is in front, because the person left the send screen long ago. Where the loop runs and where it does not is told as it is (see the consequences).

## How it will be measured

Without Apple hardware, on every pull request:

- The list and its file (`baton-core`, 20 tests): items wait in order with numbers that are not used again, the one that waited longest for a device is next for it, unpairing strikes out the device's items and nothing else, the day is counted from the queueing, and an item that the clock says was queued in the future is counted from now, the limits (all, for one device, the size of the file) refuse and change nothing, every change is in the file when the program starts again, the file is readable by its owner only and a file that others can read is not read, a file that is missing, unreadable or of another format is an empty outbox, an entry that makes no sense, or is over a limit of the protocol, is left out (and one that is at the limit stays) and the rest is kept, a file with more than there is room for keeps the oldest and is cut down, names are made safe to show as they are added and as they are read, and what waits is not in the logs.
- The loop (`baton-app`, 26 tests, most of them with a courier that the tests play, and the ones about time with the clock of the runtime stopped, so that the pauses are counted to the second): an item for a device that is not reachable waits and is tried again, items go oldest first, a device that is away is dialed once for each look, a device that does not answer does not hold up another, a device that refuses loses the item and is sent the next, a device that receives too fast keeps the item, an error that waiting does not change drops it, a day drops it and the person is told, the items of an unpaired device go without a word, also when it is unpaired as it is dialed, forgetting what is not there says nothing, closing as an item is sent keeps it, a new item and a change to the list are tried at once and not after the pause, the pauses double up to the longest and no further and start again at the first when an item is added, the paired devices change, an item goes (whichever device is done last), or an item comes when nothing waited, a loop with nothing to send looks at nothing until it is told, a list of devices that ends does not make it look over and over, what an earlier run left is found without anybody saying so, stopping ends the loop and keeps what waits, and the words for an item and for how long it is kept.
- The whole path (`baton-app/tests/outbox.rs`, seven tests with real nodes on loopback, the device that is away being one that has unpaired the sender, which refuses a connection at once): a link waits and goes when the device is back, what waits is still there when the app is opened again and goes then, cancelling and unpairing, the limit of twenty with the sentence, a device that took an item is told so, and one that answered and would not take it is told at once and nothing waits for it, text that is more than the settings of the sender allow is told at once and does not wait, though the device is away, and nothing is queued for what is wrong with the item or the device or a stopped app.
- The app layer (`baton-app`, `app.rs`, four tests): nothing waits for a device that was unpaired as it was dialed, or once the app has stopped, the list shows the name that a device has now, and the one that it had when the item was queued once the device is gone, a device that takes something sent to it at once makes the loop look if something waits for that device and does not if nothing does (the signal on its own), and what waited goes at once after it (two apps on this computer, with the item put in the list while the loop is asleep, so that only that wake can make it look).
- The screens (`outbox_controller_test.dart`, `send_controller_test.dart`, `send_screen_test.dart`, `home_screen_test.dart`, `format_test.dart`): the list and its events, each ending as a sentence, the cancel and what it says, the line that goes when nothing waits, names that hide or turn text round shown plainly, and the time left in words.

On a Mac and an iPhone, with the builds, models and network recorded. Each check is done by the person at the devices, with the steps in the pull request that adds the code.

- **O1. The phone is closed.** Close Baton on the phone (swipe it away). From the Mac, send a link to it. The send screen says that it waits and lists it. Open Baton on the phone and leave it on the screen. How long until the link is in the inbox, and the Mac says "Sent a link (…) to …"? It should be under a minute.
- **O2. The phone is locked.** The same with Baton in the background and the phone locked. The link must not arrive while the phone is locked, and must arrive after it is unlocked and Baton is on the screen.
- **O3. Cancel.** Send, cancel in the list, open Baton on the phone. Nothing arrives, and the Mac said "Cancelled".
- **O4. The Mac is quit.** Send, quit Baton on the Mac (menu bar item), start it again. The item is still listed, and goes when the phone is open. Then send, close the window only, and open the phone: it goes with the window closed.
- **O5. The Mac sleeps.** Send, close the lid for two minutes, open it, and open Baton on the phone. The item goes after the Mac is awake.
- **O6. The dial.** How long does "Sending…" stay on the screen before it says that the link waits, with Baton closed on the phone? It is the connect timeout, 20 seconds, and the check is whether that is bearable.

The day is checked by tests only. Setting the clock of a Mac forward to see it end is not something to ask of a person.

## Alternatives considered

- **A mailbox on an account server** (section 5.3 of locked-devices.md, milestone 4). It is the only path that works when the sender has gone to sleep before the other device appears. It needs a server, an account, keys for items at rest and a design for metadata that nobody has written, and a self-hosted server is a thing to run. The outbox is the part that needs none of that, and it does not stop the mailbox from coming later: a sender that cannot reach a device could try the mailbox next.
- **A push notification to wake the phone.** It needs a paid developer account and a server that holds the notification keys. The person has chosen not to pay until the development is finished.
- **The phone asks the Mac for what waits, when it opens.** It would deliver at once and with no repeated dialing, as ADR 0015 does for what is open on the Mac. It needs a new request on the wire, a store on the Mac that the phone can read, and a rule for what a phone that is unpaired is told. The sender dialing is the same work that a send does already, and nothing on the wire changes. A phone that opens and says so is the open point that would take most of what is slow in this.
- **Keep failing, and tell the person to send it again.** This is what it did, and the review found that a person does not.
- **Queue behind what already waits, so that the order is kept.** It makes a send to a device that is there wait for the next look, and says "waits" for an item that goes a second later. The person is better told the truth about the device at that moment.
- **Encrypt the file.** The key would be next to it, as the identity key of the device is. A key in the system's keychain is a different design and a different pull request.

## Consequences

- A send no longer fails because the other device is closed. "Could not reach" now shows only when the outbox is full. Code that took a send that returned as "it arrived" has to look at the result: the screens do, and the app layer's `send_link` and `send_text` give `SendResult`. The command-line program is unchanged and still fails at once.
- Items wait as long as Baton runs on the sender, up to a day. A Mac that sleeps, or a Baton that was quit, sends nothing until it is running and awake again, and the file keeps what waits over that. An iPhone or an iPad has no loop while the app is in the background: what waits on a phone goes the next time Baton is open on it, and while it is open.
- A device that is away is dialed about twice a minute at most for as long as something waits for it, which is up to a day. The pause (`Pace::longest`) is a guess until O6 and a day of use say what it costs.
- The order of delivery is the order of the items for one device, except that an item sent while the device is reachable goes at once, ahead of older ones. They go right after it: the device taking the new item makes the loop look, and not after the pause that it may be in.
- An item can arrive twice, in the one case that is described above.
- The file holds what the person sent, in plain text, for up to a day: a link with a token in it, or a password. It is readable by the owner only, and it is as safe as the inbox on the other device is. The list and the sentences never show it.
- A locked Mac that is awake and running Baton still delivers what the person sent before it was locked, as section 5.3 of locked-devices.md says: the person chose the item and the device while at it, so delivering it shows nothing that was not decided.
- A Mac with its window closed is not told how an item ended: the sentences are shown in the window, and no notification is posted for them. A person who sends, goes away and comes back to a closed window sees nothing, and the item is simply on the other device, or gone after a day.
- Unpairing a device drops what waits for it without a word, and a device that is renamed is shown under the name it has now.
- Nothing changes on the wire, in the trust file or in what a device accepts.

## Open points

- Waking the loop at once when a paired device connects, so that a phone that opens Baton is sent what waits in a second and not within half a minute. A connection from a paired device is noted in the list of devices without a change, so the list says nothing now; a signal from the node would.
- A notification at the Mac, with the always-on notices, when an item ends while the window is hidden. It would say the kind and the device, as the others do, and never what.
- Whether half a minute is the right longest pause, for the battery of a laptop and the load on a relay.
- Whether the file should be kept in a form that only an unlocked device can read, where the platform has a control for it (section 5.9 of locked-devices.md).
- Files that wait. They need the file to stay where it was, and the person's yes at the other end.
- The mailbox, for a sender that has gone to sleep (section 5.3 of locked-devices.md, milestone 4).
- Finishing a send in the few seconds that an iPhone gives an app after the person leaves it (a background task).
