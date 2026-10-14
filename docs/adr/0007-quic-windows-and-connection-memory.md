# 0007: QUIC windows, and how much a peer can make a device hold

Status: accepted

## Context

Every session connection is QUIC, and QUIC flow control has two limits that the receiver announces: how much unread data it will hold for each stream, and for the connection as a whole. A sender that has used up a limit stops until the receiver reads some data and raises it, which takes a round trip. So a stream moves at most one window per round trip:

```
top speed of a stream  =  stream window  ÷  round trip time
```

The QUIC library inside iroh 1.3.0 starts every stream with a window of 1.25 MB, which its source sizes for 100 Mbit/s at 100 ms. At a round trip of 100 ms that caps a stream at 12.5 MB/s, whatever the link can carry. A relay in another country, a link between two homes, or a Wi-Fi hop with a queue can each have a round trip like that, and file transfer is where it shows.

The same limits decide how much memory a peer can make a device use, because unread data waits in a buffer and the window is what bounds it. The library leaves the connection's limit open and counts on the streams' limits, which with the 16 streams Baton allows comes to about 20 MB for one connection.

The windows are a local choice: each device announces its own and its peers respect them, so nothing on the wire depends on the numbers and they can be changed later. They are worth settling deliberately now, because file transfer will be built to depend on the speed they allow, and because they set how much memory a peer can make a device use.

## Decision

**8 MiB for each stream, 16 MiB for a connection, and 16 MiB that this device will have sent and not yet had acknowledged.** The number of concurrent streams stays at 16, and peers still may not open unidirectional streams or send datagrams.

- **The stream window is sized for 500 Mbit/s at a round trip of 100 ms.** That path holds 6.25 MB in flight. noq sends a credit update only after a stream has been read through an eighth of its window, so the sender can count on seven eighths, and 8 MiB is the smallest power of two for which seven eighths (7.3 MB) covers the path. One stream can then carry up to about 73 MB/s at 100 ms, a full gigabit up to a round trip of about 58 ms, and 37 MB/s at 200 ms.
- **The connection window is twice the stream window.** The library's own documentation says to keep the stream window below the connection window so that one stream nobody reads cannot take all the buffer space. With twice, a stalled stream still leaves a full stream window for the others.
- **The send window equals the connection window.** It limits how much this device keeps for retransmission when a peer offers more room than it wants to use. A peer with these settings offers 16 MiB, so a smaller send window would hold back data the peer is willing to take.

**A peer can make a device hold less than before.** A connection holds at most its window, 16 MiB (about 17 MB), where the library default allowed about 20 MB. The settings are on the endpoint, so they apply to every connection it accepts, including the pairing connections that anyone can open. A device serves at most 8 of those at a time and keeps at most 8 connections for each paired device, so what one source can make it hold is at most about 134 MB, where before it was 160 MB. Those figures are reached only by a peer that fills every window and never lets the device read, and the device reads each request as it arrives and closes a pairing connection once it has been served or has timed out.

**The windows are checked when the crate is built.** `transport.rs` has compile-time assertions that seven eighths of the stream window and of the connection window cover the path the windows are sized for, that the connection window is at least twice the stream window, that the send window is at least the connection window, and that a connection cannot hold more than the library default allowed. Editing one number without the others stops the build, and raising the windows past the library's worst case has to be argued in a record like this one.

**Larger windows for bulk transfer belong on the receiver's side of a dial, and are not built yet.** What limits a sender is the window the *receiver* announced, and a device that dials announces windows of its own choosing for that connection (iroh's `Endpoint::connect_with_opts` takes a transport configuration per connection, which iroh calls experimental). So when file transfer arrives, a receiver can dial the sender and ask for the file on a connection with large windows, while every connection other devices open to it keeps the small ones. No device ends up holding more because of it, and a file that moves from a Mac to a phone is limited by what the phone announces, which the phone chooses. This changes who dials for a transfer, which is a decision for the file transfer design, and has to be measured there.

## What was measured

The test `windows_on_a_long_path` in `crates/baton-core/src/transport.rs` sends 128 MiB on one stream between two devices on one machine and times it. It is ignored in ordinary runs. The devices have no direct address and reach each other only through an in-process relay, behind a TCP relay of its own that holds every chunk for a set time in each direction and queues it behind the ones before it as a link of a set speed would. It measures the round trip it really gave, on the idle path, before each run and prints it, and a smaller test that runs every time fails if the devices ever find a way around the delay, since iroh switches to any direct path that is a few milliseconds faster.

One run on a development machine, in release mode, speeds in MB/s:

| Path | Round trip | Library default (1.25 MB) | These settings (8 MiB) | Twice as large (16 MiB) |
| --- | --- | --- | --- | --- |
| 100 ms, link of 40 MB/s (320 Mbit/s) | 107 ms | 10.0 | 27.9 | 27.9 |
| 100 ms, link of 100 MB/s (800 Mbit/s) | 107 ms | 10.4 | 34.8 | 43.0 |
| 20 ms, link of 40 MB/s | 26 ms | 34.5 | 34.8 | 35.1 |
| 4 ms, link of 100 MB/s | 10 ms | 82.8 | 86.4 | 84.4 |

What it shows:

- The arithmetic describes what happens. The library default reaches 10.0 and 10.4 MB/s at 107 ms, which is what a window of 1.25 MB, raised in eighths, allows (about 10.2 MB/s).
- These settings are 2.8 and 3.3 times faster at a round trip of 100 ms, and make no difference where the round trip is short, which is what the arithmetic predicts. Nothing got slower.
- Doubling the windows again gains 24% on the 800 Mbit/s path and nothing on the 320 Mbit/s one, at twice the worst-case memory, which would be above what the library allowed. That is why it is not the default.

What it does not show:

- At 100 ms every setting stays well below the link rate, and below what its window would allow, so something other than flow control is the limit there. The likeliest is how fast the congestion controller opens up during a transfer of 128 MiB, and the harness itself tops out near 85 MB/s. This was not investigated.
- It is one run for each cell, on one machine, with every device and the relay sharing its cores. Two runs of the same cell differed by about 3%.
- The path is an emulation: no loss, no jitter, an unlimited queue, and only the relayed route, not a direct UDP path. The file transfer benchmark is where real networks, and the direct route, come in.

Run it again with `cargo test -p baton-core --release --lib windows_on_a_long_path -- --ignored --nocapture`, for example after an iroh upgrade, which can change the library's defaults.

## Prior art and where this differs

This comes from the QUIC library's source and documentation, and from RFC 9000, which defines the two levels of flow control. It is not copied from another application.

- The library's documentation for `stream_receive_window` says to set it to at least the expected latency multiplied by the throughput wanted, and to keep it below `receive_window` so that one stream cannot monopolise the buffers. These settings follow that advice and write the numbers down.
- The library's defaults, as its source shows, are sized for a 100 Mbit/s path and leave the connection window unlimited. Baton sizes for a faster path and gives the connection a limit.
- What goes beyond that: the seven eighths that credit updates leave available are part of the sizing, the relations are enforced when the crate is built, with a test that holds the worst-case memory at or below the library's, and the proposal above, in which the receiver chooses the windows of a bulk connection by dialing it, so that large windows never reach the endpoints other devices connect to. I have not seen another file transfer tool describe that last idea, and I have not built or measured it yet.

## Alternatives considered

- **Keep the library defaults.** Costs nothing, and caps a stream at 12.5 MB/s at 100 ms. Measured above at 10 MB/s. File transfer is a headline feature, so no.
- **16 MiB for each stream, 32 MiB for a connection.** Doubles the ceiling and the worst-case memory. In the harness it helped only on the fastest path, by 24%. It would put a connection above what the library allowed, and a source at about 268 MB across its connections. A good setting for a dialing receiver if the file transfer benchmark asks for it, not for every endpoint.
- **A connection window no larger than the stream window.** One stream nobody reads would then stall every other stream on the connection, which the library's documentation warns about.
- **Different windows for strangers and paired devices.** iroh lets an incoming connection be accepted with its own configuration, but at that point it shows only the source address, not who is asking or for which protocol, so it cannot tell a stranger from a paired device.

## Consequences

- One stream is limited to about 73 MB/s at a round trip of 100 ms and 37 MB/s at 200 ms. A faster long path needs the dialing receiver's larger windows, or a change to this record.
- A connection can make a device hold at most 16 MiB, against about 20 MB before. A source can make it hold at most about 134 MB over all its connections, against 160 MB.
- This device may keep up to 16 MiB of its own data unacknowledged on one connection, against 10 MB before. It sends only what it chose to send, which for now is small replies.
- The windows affect only how data is held and how fast it can flow. Limits on item sizes, rates and requests are separate and unchanged.
- The measurement can be repeated, and the build fails if the numbers drift out of the relations they depend on.
