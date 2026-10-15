# 0008: File transfer: one offer, lanes of their own, hashing while sending

Status: accepted

## Context

Handing over a link or some text takes one small request. Handing over a photo library, a build folder or a 4 GB video is a different job, and it is where a person feels the speed of a tool. The roadmap sets two targets for it, stated against the raw throughput of the link so that they mean the same on any network: one large file at 90% or more of it (about 100 MB/s on gigabit Ethernet), and 10,000 files of 10 KiB in no more than twice the time of one file of the same total size. There is no compatibility with another tool's protocol, so the wire format can be shaped for those targets.

Four things decide whether a transfer reaches them, and each has a trap that a design written without them walks into:

- **One stream is a window per round trip.** ADR 0007 sized the QUIC windows so that one stream can carry about 73 MB/s at a round trip of 100 ms, and two streams about 147 MB/s. A design that sends a file on one stream leaves the rest of a faster long path unused.
- **A wait per file is a wait per round trip.** On Wi-Fi a round trip is a few milliseconds. A protocol that waits for an answer after each of 10,000 files spends 20 seconds waiting at 2 ms, against the one second that 100 MB takes at 100 MB/s. Small files are fast only if nothing waits between them.
- **Control and bulk traffic share one connection.** The session protocol allows 16 concurrent streams and times a stream out after 10 seconds without progress (protocol.md section 3). Those numbers suit requests that are small. A transfer that counted against them would be cut off by the stall timer, would crowd out a ping or a handoff, or would need the 16 raised for every peer.
- **The receiver is often a phone.** An iPhone or iPad runs Baton only while it is on the screen and has little memory to spare, so it can receive several gigabytes only if nothing is held whole. Memory has to stay bounded by the windows, not by the size of what is sent.

The standing rules from earlier decisions apply: a transfer is a request from a paired device, so it passes the same checks as a handoff before anything is written; the person decides what a device may put on their disk; and nothing here is opened or run (security.md).

## Decision

**One offer, decided once.** A transfer is a set of files and folders sent as a whole. The sender makes one `Offer` request that carries a random transfer id, a summary (number of files, folders and bytes) and a preview of the top-level names. The receiver answers once: accepted, by its policy for that device or by the person, or declined with a reason. A person is given two minutes, as in pairing, and the sender waits for the answer on the same stream. Everything after a yes is the enforcement of what was agreed, never a second question: the full list of files that follows must match the summary, and a list that does not is a protocol violation. The sender repeats the same id when it retries, as with handoffs, and a repeat gets the same decision without asking the person again.

A whole transfer counts as one item against the device's rate limit and as one entry in the inbox, whatever it holds. Files have limits of their own: a number of transfers active at once, a quota of bytes, and a reserve of free disk space that a transfer may not use.

**Data moves on lanes: unidirectional streams, with a budget of their own.** After a yes the sender opens up to four unidirectional QUIC streams to the receiver and writes the data on them. QUIC limits bidirectional and unidirectional streams separately, and the receiver sets both limits. Requests keep the 16 bidirectional streams and the stall timer of section 3. Lanes get their own budget of unidirectional streams, enforced by the QUIC layer, so that no accounting has to be written in Baton and a transfer cannot starve a request or the other way round. The budget is eight: four lanes for each of the two transfers that one device may have active at once. A stranger is given none: the limit starts at zero, as it does now, and the gate raises it for a paired device at the same moment and by the same hook as the connection window (ADR 0007). The call exists on the connection (`set_max_concurrent_uni_streams`, iroh 1.3.0, `endpoint/connection.rs`). The unidirectional limit does not change what a peer can make a device hold: the connection window is still the bound, and lanes only share it.

Lanes are sent at a lower priority than request streams (`SendStream::set_priority`, noq 1.3.0), so a transfer never delays a ping or a handoff. A receiver that wants no more data stops a lane with a stop code (`STOP_SENDING`), and the sender sees that its write failed, with the code. The receiver sends nothing back along a lane, so there are no acknowledgements to wait for or to count: QUIC already delivers the bytes reliably and in order within a lane.

**The list of files and the data share the lanes, and nothing waits per file.** The first frames on lane 0 are the full list of files: relative paths, sizes and modification times. Then every lane carries spans. A file is cut into spans of 1 MiB (the last one shorter), and a span is a small binary header followed by the raw bytes, with no CBOR around the data. Spans of one large file can go on different lanes, which is how a long path gets more than one window; many small files go back to back on one lane, each as a single span. There is no answer per file and no answer per span. The receiver writes each span at its offset in a file whose size it knows from the list.

**The hash is made while sending, in any order.** Each file has a BLAKE3 hash. BLAKE3 hashes a file as a tree of 1 KiB chunks, and a span of 1 MiB that starts on a 1 MiB boundary is a complete subtree of it. The `blake3` crate exposes the chaining value of such a subtree (`hazmat::HasherExt::set_input_offset` and `finalize_non_root`) and the merge of subtrees into a parent or the root (`merge_subtrees_non_root`, `merge_subtrees_root`). So either side can hash the spans in whatever order they come, at the speed of the lanes and in step with the disk, and merge the values at the end into the same value that `b3sum` prints for the whole file. The sender hashes what it reads and sends the file's hash after the last span has been read. The receiver merges its own values and compares. No pass over the data is made before the offer, so the first byte leaves at once, and none is made afterwards. A file whose hash does not match is not put in place.

**Nothing appears until it is complete, and nothing is replaced.** The receiver writes into a staging folder that it makes on the same volume as the destination, and renames the items into place once the transfer has ended and the files in them have been checked, leaving out any file that did not check out. Names are decided at that moment: an item that would take the name of one that already exists, compared the way the file system compares names (not case-sensitively, and after Unicode normalisation), gets a number before its extension. A person never finds half a folder, and a second transfer never overwrites the first. Names that arrive in the list are untrusted: a path that is absolute, that has a `..` or an empty part, or that is too long or too deep makes the whole transfer invalid; characters that a file system on the receiving side does not allow (control characters, the reserved Windows names, trailing dots and spaces) are replaced. Links, devices and other special files are not sent, permission bits are not carried (a received file is never executable), and where the system has a mark for files that came from elsewhere, as a Mac does, received files get it. The sizes and counts of the list are checked against the summary, the quota and the free space before the first byte is written.

**Progress is judged over time, and a transfer outlives a connection.** A transfer that has moved no byte for 30 seconds is stopped, and a long one is not stopped for being long. The sender ends a transfer with a `Finish` request after the last lane is finished. The receiver answers it when every file has been checked and put in place, with the result (done, or which files failed and why), so "sent" is shown only when the other device has it. If the connection drops, the sender reconnects and offers again under the same id. The receiver, which has kept what it received for a while, answers with the spans it already has, and the sender sends only the rest. A file that fails (it changed while it was being read, it did not verify) fails alone and is named in the result; the other files are kept.

**Everything that decides speed is a local choice, and the benchmark settles it.** The wire carries a transfer, its list and its spans, and the number of lanes the receiver allows. It does not carry the size of a read, the size of the windows, the congestion controller or how many files are read at once: each device chooses its own, and they can change without a new protocol. ADR 0007 left three questions to this record: the stream window for long transfers, the congestion controller (Cubic, which is the default, or BBRv3, which the QUIC library ships) and behaviour on a path with loss. They are answered by measurements, below, and not here.

The wire format, the limits and the rules of the receiver are specified in protocol.md when the types that carry them are added. The capability that guards all of it is `transfer.files`.

```
sender                                                   receiver
  | -- Offer {id, summary, preview} ----------------->   |  request stream, may wait for the person
  | <--------------------- Accepted {lanes: 4} -------   |
  | == lane 0: list of files, then spans, hashes ====>   |  unidirectional streams, lower priority
  | == lanes 1 to 3: spans, hashes ==================>   |
  | -- Finish {id, files, bytes} -------------------->   |  request stream, answered when all is in place
  | <---------------------------------- Done {..} ---   |
```

## How it will be measured

A benchmark that anyone can rerun is part of the work, with the four cases of the roadmap, each aimed at a different limit: one large file, thousands of small files, Wi-Fi under load, and a relayed connection. It is a command (`baton bench`) that can make the data itself, so that the network and the protocol are measured without a disk, and that can also send a real folder, which brings the disk in. Every run also measures the raw rate of the link with a plain TCP stream between the same two addresses, and reports Baton's rate as a share of it, so a result means the same on a laptop pair and on a phone.

- **Large file and many small files** are run on a real link, between two machines, and on an emulated path on one machine (the harness of ADR 0007, which can add delay and a rate, and a pair of virtual network devices with `netem` for loss). The two targets are tested on the real link.
- **Wi-Fi under load and relayed** have no target until there are measurements. They are reported with their loss, round trip and rate.
- **The settings to compare** are the number of lanes (1, 2, 4, 8), the congestion controller (Cubic, BBRv3), the stream window (8 and 16 MiB) and the number of files read and written at once. Rates are reported for long runs and apart from slow start, as ADR 0007 did, and with the spread of at least five runs.
- **Where the ceiling may be the CPU, not the network.** Baton builds iroh without its fast Apple datapath, which calls private system functions that the App Store rejects, so a Mac or an iPhone probably sends one packet per system call. That should be enough for gigabit and may not be for 10 Gbit/s. The benchmark reports processor use for each run so that a limit set by the CPU is not mistaken for one set by the network. If it turns out to be the limit, more than one QUIC connection for a big transfer is the next thing to try, and is a change to the sender alone.

## Alternatives considered

- **iroh-blobs.** It is a content-addressed transfer on iroh, but a transfer starts with a hash of the whole content, which is a pass over a large file before the first byte moves, its own documentation calls this version not yet production quality, and it brings a second protocol with its own rules for who may fetch what into a design where the paired device is the only authority. Hashing while sending gives the same integrity without the pass.
- **A stream for each file, on the request streams.** Simple, and no state to keep, but the 16-stream budget would limit a transfer to 16 files in flight, a stream's setup would be paid for each of 10,000 files, and a transfer would be cut off by the timer meant for requests.
- **Bidirectional lanes with acknowledgements.** The receiver could report progress and errors on the same stream as the data. It would take streams from the 16 that requests have, and an acknowledgement per span adds traffic that QUIC's own delivery makes unnecessary. A stop code and a final `Finish` carry what is needed.
- **A second protocol on its own port, such as HTTP over TLS.** It is a common choice for local transfer tools. It cannot use the relay or hole punching Baton already has, needs its own way of knowing who is calling, and would be a second thing to keep in step with the first.
- **A hash of every file before the offer.** It would put hashes in the list and let the receiver skip what it already has. It costs a full read of everything before the first byte moves, which on a large folder is more time than the transfer. Skipping what is already there is a sync feature and is left for later.
- **Hashes made in order.** Simpler, but then every file would have to travel on one lane, in order, and a large file could use only one window.
- **Compression.** Photos, video, archives and most documents are compressed already, and compressing costs a phone's battery. A capability can add it per span later for text-like content if a measurement shows it pays.
- **Rsync-style differences, and the receiver pulling files.** Both are real needs and neither is part of the first transfer. Pulling is planned in protocol.md section 8 and would reuse the lanes.

## Prior art and where this differs

Cutting a file into pieces that are hashed on their own is how BitTorrent works, and the `blake3` crate documents its tree interface for building such protocols. Several streams or connections for one transfer are how download managers and parallel SFTP clients get past a window. Writing to a temporary name and renaming when finished is common practice. This record does not claim to have invented any of them.

What it adds, as far as I know, and I have not searched exhaustively:

- **A stream budget for bulk data that is granted after the trust check and enforced by QUIC.** The same idea as the window raise of ADR 0007, applied to the number of streams: a stranger can open none, a paired device a small fixed number, and nothing in Baton counts them.
- **One decision for a whole transfer, and a staging folder that makes it atomic.** The person is asked once, and what they accepted appears whole or not at all.
- **Hashing that needs neither a pass before the offer nor in-order data.** The value is the standard BLAKE3 hash of the file, so a person can check it with `b3sum`.

## Consequences

- The transport settings change in one place: the unidirectional stream limit is zero until the gate raises it for a paired device, and a test shows both halves, as it does for the window. The compile-time relations of ADR 0007 hold unchanged, because lanes share the connection window.
- protocol.md section 3 says that peers do not open unidirectional streams. It is updated when the lane format is specified: they may after the peer is paired, for transfers only.
- A paired device can make another hold at most what ADR 0007 says, and can have two offers waiting for a person at the same time at most. Further offers are declined with `RateLimited`, so a peer cannot use up the 16 request streams with questions.
- Disk is the new resource a peer can use. It is bounded by what the person accepted, the quota of bytes, the reserve of free space, and a limit on the size of what is staged. Staging folders that are not resumed are removed.
- A transfer needs both devices to be awake and connected. A phone that leaves Baton or locks stops receiving; the sender keeps trying for a while, and the transfer resumes when Baton is on the screen again. Whether the screen should stay on while a transfer runs is a decision about the product, made separately.
- An iPhone or iPad keeps what it receives in the app's own folder, which the Files app can show, and a Mac in the Downloads folder. Those choices, the picker and the screens belong to the app and are described with it.
- A device that sends files must be told that the other one can receive them, by the capability, before it offers. An older device is never sent an offer it cannot read.
- The benchmark can be repeated, and its numbers go into this record when they exist. Until then the two targets are goals and not results.
