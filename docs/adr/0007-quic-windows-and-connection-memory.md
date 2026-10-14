# 0007: QUIC windows, and how much a peer can make a device hold

Status: accepted

## Context

Every session connection is QUIC, and QUIC flow control has two limits that the receiver announces: how much unread data it will hold for each stream, and for the connection as a whole. A sender that has used up a limit stops until the receiver reads some data and raises it, which takes a round trip. So a stream moves at most one window per round trip:

```
top speed of a stream  =  stream window  ÷  round trip time
```

The QUIC library inside iroh 1.3.0 starts every stream with a window of 1.25 MB, which its source sizes for 100 Mbit/s at 100 ms. At a round trip of 100 ms that caps a stream at 12.5 MB/s, whatever the link can carry. A relay in another country, a link between two homes, or a Wi-Fi hop with a queue can each have a round trip like that, and file transfer is where it shows.

The same limits decide how much memory a peer can make a device use, because unread data waits in a buffer and the window is what bounds it. The library leaves the connection's limit open and counts on the streams' limits, which with the 16 streams Baton allows comes to about 20 MB for one connection.

Two facts make this more than a choice of numbers:

- **The settings are on the endpoint, so they are announced before the device knows who is calling.** A stranger who opens a `baton-pair/1` connection, which anyone can do, would get the same windows as a paired device. The first version of this record accepted that and kept the windows small enough to live with it.
- **A sender has limits of its own.** What it may have in flight is also capped by its send window and its congestion controller, and both come from the configuration of its own endpoint. iroh 1.3 can change the receive window of a connection that exists (`Connection::set_receive_window`) and has no such call for the send window.

The windows are a local choice: each device announces its own and its peers respect them, so nothing on the wire depends on the numbers and they can be changed later. They are worth settling deliberately now, because file transfer will be built to depend on the speed they allow, and because they set how much memory a peer can make a device use.

## Decision

**Every connection starts with a connection window of 64 KiB, and the gate raises it to 16 MiB once it has seen that the device on the other end is paired.** The stream window is 8 MiB and the send window 16 MiB. The number of concurrent streams stays at 16, and peers still may not open unidirectional streams or send datagrams.

- **The first window is 64 KiB.** A pairing frame is at most 4 KiB, so that holds fifteen of them with their headers, which is far more than a real pairing needs, and eight pairing connections together come to 512 KiB at most. A `baton-pair/1` connection is never raised: whoever opens one gets 64 KiB for as long as it lives.
- **The raise is made by the gate, for connections in both directions.** iroh runs `after_handshake` for connections a device dials as well as those it accepts, with the peer's key known. When the trust check passes for `baton/1` the gate calls `set_receive_window`, and the library announces the larger limit in a `MAX_DATA` frame, which QUIC lets a receiver send at any time (RFC 9000, section 4.1). So the rule is the same whichever device asks for a transfer. A peer with more than 64 KiB to send on a new connection waits about one round trip for the raise, which a client that opens with a request never notices.
- **The stream window is sized for speed alone.** A connection never holds more than its connection window, so a stranger's connection is held to 64 KiB however large the stream window is. The stream window has no per-connection setter, so it stays on the endpoint, and it costs nothing against strangers. It is sized for 500 Mbit/s at a round trip of 100 ms, a path that holds 6.25 MB in flight. noq sends a credit update only after a stream has been read through an eighth of its window, so the sender can count on seven eighths, and 8 MiB is the smallest power of two for which seven eighths (7.3 MB) covers the path. One stream can then carry up to about 73 MB/s at 100 ms, a full gigabit up to a round trip of about 58 ms, and 37 MB/s at 200 ms.
- **The connection window is twice the stream window.** The library's own documentation says to keep the stream window below the connection window so that one stream nobody reads cannot take all the buffer space. With twice, a stalled stream still leaves a full stream window for the others.
- **The send window equals the connection window.** It limits how much this device keeps for retransmission when a peer offers more room than it wants to use. A peer with these settings offers 16 MiB once it knows we are paired, so a smaller send window would hold back data the peer is willing to take.

**One rule says which settings are raised for a connection and which are not, and it does not depend on who dials.** Settings that bound what peers send to us, the receive windows, start small and are raised for each connection once the peer is known to be paired. Settings that bound what we send, the send window and the congestion controller, cost nothing that a stranger can reach, because a device sends only what it chose to send. They are set for bulk transfer on the whole endpoint. A transfer is then limited by the receiver's window, raised after the trust check on the receiving side, and by the sender's own send window and controller, and the file transfer design does not have to choose who dials for the sake of windows.

**What a peer can make a device hold.**

- A stranger can make it hold at most 512 KiB: 8 pairing connections are served at once and each is held to 64 KiB. A stranger on `baton/1` is closed with 403 straight after the handshake.
- A paired device can make it hold at most 128 MiB (134 MB): 16 MiB on each of at most 8 connections. The library default allowed about 160 MB for the same 8 connections. That figure is reached only by a paired peer that fills every window and never lets the device read, and the device reads each request as it arrives.

**The relations are checked when the crate is built.** `transport.rs` has compile-time assertions that seven eighths of the stream window and of the connection window cover the path the windows are sized for, that the connection window is at least twice the stream window, that the send window is at least the connection window, that the first window holds at least four pairing frames and eight of them together stay under 1 MiB, that there is something to raise, that a connection cannot hold more than the library default allowed, and that one paired device cannot make a device hold more than 128 MiB over all its connections. Editing one number without the others stops the build, and raising the windows past those bounds has to be argued in a record like this one. Five tests cover the mechanism. Two in the gate show that both ends of a paired connection raise their window, whichever one dialed, and that neither end of a pairing connection does. Two against a running device show that a pairing connection is sent no larger limit and that a stranger who writes as much as flow control allows gets no more than 64 KiB through. One shows that a paired device is sent the larger limit.

## What was measured

The test `windows_on_a_long_path` in `crates/baton-core/src/transport.rs` sends 128 MiB on one stream between two devices on one machine and times it. It is ignored in ordinary runs. The devices have no direct address and reach each other only through an in-process relay, behind a TCP relay of its own that holds every chunk for a set time in each direction and queues it behind the ones before it as a link of a set speed would. It measures the round trip it really gave, on the idle path, before each run and prints it, and a smaller test that runs every time fails if the devices ever find a way around the delay, since iroh switches to any direct path that is a few milliseconds faster.

Each cell is run five times, because one run says little, and each run is reported twice: over the whole transfer, and over its second half, which leaves out the time the congestion controller takes to open up. The figures are the median of the five, in MB/s, with the smallest and the largest in brackets. The "twice as large" column is a comparison and not a setting: 16 MiB for each stream and 32 MiB for a connection.

| Path | Setting | Whole run | Second half | Ceiling |
| --- | --- | --- | --- | --- |
| 100 ms, link of 40 MB/s (320 Mbit/s) | Library default (1.25 MB) | 10.2 (10.0 to 10.3) | 10.9 (10.9 to 11.0) | 10 |
| | These settings (8 MiB) | 27.8 (21.5 to 28.6) | 37.0 (30.0 to 37.0) | 69 |
| | Twice as large (16 MiB) | 28.2 (26.9 to 28.6) | 35.5 (31.7 to 37.0) | 137 |
| 100 ms, link of 100 MB/s (800 Mbit/s) | Library default | 10.4 (10.0 to 10.4) | 11.1 (10.5 to 11.2) | 10 |
| | These settings | 39.9 (36.9 to 40.5) | 63.5 (53.9 to 69.4) | 69 |
| | Twice as large | 45.0 (42.1 to 47.8) | 79.1 (62.1 to 87.1) | 138 |
| 20 ms, link of 40 MB/s | Library default | 34.3 (34.2 to 34.6) | 36.1 (35.8 to 36.7) | 42 |
| | These settings | 35.0 (34.8 to 35.1) | 37.0 (36.4 to 37.0) | 281 |
| | Twice as large | 34.6 (28.5 to 35.1) | 37.0 (25.8 to 37.0) | 567 |
| 4 ms, link of 100 MB/s | Library default | 82.3 (78.5 to 83.6) | 87.8 (84.3 to 88.7) | 112 |
| | These settings | 80.9 (66.7 to 86.8) | 91.1 (90.4 to 92.4) | 750 |
| | Twice as large | 85.5 (70.4 to 87.1) | 92.5 (86.2 to 93.4) | 1538 |

The round trip the paths really gave, measured before each run, was 106 to 108 ms for the two "100 ms" paths, 26 ms for the third and 10 ms for the fourth. The ceiling is seven eighths of the stream window divided by that round trip, in MB/s.

What it shows:

- The arithmetic describes what happens at 100 ms. The library default reaches 10.2 and 10.4 MB/s, which is its ceiling, and the 8 MiB window reaches 63.5 MB/s in the second half on the 800 Mbit/s path, 92% of its ceiling of 69.
- These settings are 2.7 to 3.8 times as fast as the library default over the whole run at a round trip of 100 ms. Where the round trip is short they make no difference within the spread of the runs (35.0 against 34.3 MB/s at 26 ms, 80.9 against 82.3 at 10 ms), which is what the arithmetic predicts. Nothing got slower.
- On the 320 Mbit/s path both sizes end near 37 MB/s in the second half. The link is the limit there, not the window.
- On the 800 Mbit/s path the window is the limit in the second half, and the larger one is ahead: 13% over the whole run and 25% over the second half, in medians. An earlier set of five runs of the same cells gave medians within 9% of these and the same direction (8% and 18%). The ranges of the second half overlap in both. It is the one place in the table where doubling helps, on a path faster than the 500 Mbit/s these windows are sized for.

What it does not show, which matters because file transfer will lean on these numbers:

- **Congestion control is never exercised.** The emulated path drops no packet and has an unlimited queue, so Cubic stays in slow start until flow control stops it. On real Wi-Fi, loss and Cubic's backing off at 100 ms will probably decide whether the speed target for file transfer holds. noq-proto 1.3 ships BBRv3 (`congestion::Bbr3Config`), and `QuicTransportConfig::congestion_controller_factory` can select it. That comparison belongs in the file transfer benchmark, on a direct path with loss (netem on a veth pair works in CI), and the endpoint-wide controller is a setting of the kind the rule above puts on the whole endpoint.
- **Part of every run at 100 ms is slow start.** noq's Cubic starts at 12,000 bytes (ten packets of 1,200 bytes), so it takes about nine round trips, around a second at 107 ms, to reach a window of several megabytes, which is a large share of a run of three to five seconds. The link rate also counts the TLS, relay and QUIC framing bytes. The second-half column is there to separate steady state from the ramp, and most of what the first version of this record put down to "something other than flow control" was the ramp. Even the second half of a 128 MiB transfer is short, so the file transfer benchmark should report rates over long transfers.
- The harness tops out near 90 MB/s, with both devices, the relay and the delay line sharing the cores of one machine, so it cannot say anything about a path faster than that.
- It is an emulation: no loss, no jitter, an unlimited queue, and only the relayed route, not a direct UDP path.

Run it again with `cargo test -p baton-core --release --lib windows_on_a_long_path -- --ignored --nocapture`, for example after an iroh upgrade, which can change the library's defaults.

## Prior art and where this differs

This comes from the QUIC library's source and documentation, and from RFC 9000, which defines the two levels of flow control and lets a receiver raise a limit at any time. It is not copied from another application.

- The library's documentation for `stream_receive_window` says to set it to at least the expected latency multiplied by the throughput wanted, and to keep it below `receive_window` so that one stream cannot monopolise the buffers. These settings follow that advice and write the numbers down.
- The library's defaults, as its source shows, are sized for a 100 Mbit/s path and leave the connection window unlimited. Baton sizes for a faster path and gives the connection a limit.
- What goes beyond that: every connection announces a small window to everyone and is raised only once the peer is known to be paired, which keeps the cost of a bigger window away from strangers entirely and lets the stream window be chosen for speed. I have not seen another file transfer tool describe doing this, and I have not searched for one exhaustively. It is built and tested here. Also new are the seven eighths that credit updates leave available as part of the sizing, relations that are enforced when the crate is built, and a measurement that reports steady state apart from the ramp.

## Alternatives considered

- **Keep the library defaults.** Costs nothing, and caps a stream at 12.5 MB/s at 100 ms. Measured above at 10 MB/s. File transfer is a headline feature, so no.
- **16 MiB for each stream, 32 MiB for a connection.** Strangers no longer pay for it, so what remains is the cost to a paired device: a connection that can hold 32 MiB, 256 MiB over its 8 connections, which is more than the library allowed. The benefit is real but narrow: 13% over a whole run and 25% in the second half on a path of 800 Mbit/s at 100 ms, and nothing on the other paths. Few devices will have a path like that, and the congestion controller, which this harness cannot exercise, may limit such a path before the window does. It is a change of two numbers and the build's relations, with no change to the protocol, and the file transfer benchmark, which can report steady-state rates over long transfers on real paths, is the place to decide it.
- **A connection window no larger than the stream window.** One stream nobody reads would then stall every other stream on the connection, which the library's documentation warns about.
- **The same windows for strangers and paired devices.** This was the first version of this record. A stranger could then make a device hold 8 pairing connections at the full window each, 134 MB, which on a phone risks the app being killed in the background. iroh can accept an incoming connection with a configuration of its own, but at that point it shows only the source address, so the raise after the handshake is how to tell the two apart.
- **A receiver dials the sender for a bulk transfer, to announce large windows for that connection.** This was the first version's proposal for the file transfer design. It cannot choose the sender's side of the connection: the send window and the congestion controller come from the sender's endpoint, and iroh has no per-connection setter for the send window, so a pull is capped at 16 MiB in flight (about 168 MB/s at 100 ms) whatever the dialing receiver asks for. The rule above works from whichever side dials.

## Consequences

- One stream is limited to about 73 MB/s at a round trip of 100 ms and 37 MB/s at 200 ms, and a whole connection to about 147 MB/s at 100 ms. A faster long path needs larger windows, which is a change to this record.
- A stranger can make a device hold 512 KiB at most. A paired device can make it hold 128 MiB at most over all its connections, against 160 MB with the library default.
- A connection to a paired device spends its first round trip with a 64 KiB window. A client that opens with a request does not notice.
- This device may keep up to 16 MiB of its own data unacknowledged on one connection, against 10 MB before. It sends only what it chose to send, which for now is small replies.
- The windows affect only how data is held and how fast it can flow. Limits on item sizes, rates and requests are separate and unchanged.
- The file transfer design does not have to choose who dials for the sake of windows, and it owns three questions this record leaves open: the stream window for long transfers, the congestion controller, and behaviour on a path with loss.
- The measurement can be repeated, and the build fails if the numbers drift out of the relations they depend on.
