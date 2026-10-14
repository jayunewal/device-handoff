// SPDX-License-Identifier: Apache-2.0

//! The QUIC settings every Baton connection uses: who may open streams, when a quiet
//! connection is kept alive, and how much data may be in flight.
//!
//! # How much data may be in flight
//!
//! A QUIC receiver says how many bytes it is willing to hold that the application has
//! not read yet, one limit for each stream and one for the whole connection. A sender
//! that has used up the limit has to wait for the receiver to raise it, which takes a
//! round trip. So a stream can move at most one window per round trip, however fast
//! the link is:
//!
//! ```text
//! top speed of a stream  =  stream window  ÷  round trip time
//! ```
//!
//! The QUIC library's default stream window is 1.25 MB, which it sizes for a
//! 100 Mbit/s link with a round trip of 100 ms. At that round trip the default caps
//! a stream at 12.5 MB/s, a fifth of what a 500 Mbit/s connection could carry, and the
//! cap falls further as the round trip grows. A relay in another country, a link
//! between two homes or a congested Wi-Fi hop can all have a round trip of 100 ms, and
//! file transfer is where it shows.
//!
//! The windows below are sized for a path of 500 Mbit/s with a round trip of 100 ms,
//! which holds 6.25 MB in flight and is about what fast fibre at both ends of a long
//! path can do. The receiver raises a window in steps of an eighth of it, so only
//! seven eighths is always available, and 8 MiB is the smallest power of two that
//! still covers the path. A faster path would need more, and what that would cost is
//! now only a matter for paired devices (see below), so it is a question for the file
//! transfer benchmark, which can measure long transfers on real paths.
//!
//! # What it costs, and who gets the big window
//!
//! The windows also decide how much memory a peer can make a device use. Data that
//! has arrived and not been read waits in a buffer, and the receiver's windows limit
//! how much, so a connection can make a device hold at most its connection window
//! whatever the other side does. The library's default leaves the connection limit
//! open and relies on the stream limits alone, which with 16 streams comes to about
//! 20 MB.
//!
//! A device announces its settings before it knows who is calling: they are on the
//! endpoint, so they are the same for a stranger on `baton-pair/1` and for a paired
//! device. So every connection starts with a small connection window,
//! [`INITIAL_WINDOW`], which is all a stranger ever gets, and the gate raises it to
//! [`CONNECTION_WINDOW`] once the device on the other end is known to be paired
//! ([`raise_window`]). The peer is told in a `MAX_DATA` frame, and the hook that does
//! this runs for connections the device dials as well as those it accepts, so the
//! rule is the same whichever side of a transfer dials. What a stranger can make a
//! device hold is the number of pairing connections it serves times
//! [`INITIAL_WINDOW`], and that is checked below.
//!
//! The stream window cannot be changed once a connection exists. It does not need to
//! be: a connection never holds more than its connection window, so the stream window
//! only decides how much of that window one stream can use, and it is sized for speed.
//! What a paired device can make a device hold is [`CONNECTION_WINDOW`] for each of the
//! connections it may keep, and that is checked below as well.
//!
//! Settings that bound what a device *sends*, the send window and the congestion
//! controller, cost nothing that a stranger can reach, because a device sends only
//! what it chose to. They are set for bulk transfer on the whole endpoint.
//!
//! `docs/adr/0007-quic-windows-and-connection-memory.md` has the arithmetic, the
//! measurements and the alternatives.

use std::time::Duration;

use baton_proto::{frame::HEADER_LEN, limits::MAX_PAIR_FRAME_LEN};
use iroh::endpoint::{Connection, QuicTransportConfig, VarInt};

use crate::{gate::MAX_CONNECTIONS_PER_PEER, slots::MAX_PAIRING_CONNECTIONS};

/// Streams a peer may have open on one connection at once.
///
/// Requests are one per stream and a device needs a handful at a time. This and the
/// time limit on a request are for the small control messages; file transfer will have
/// a budget of its own (see section 3 of `docs/protocol.md`).
pub(crate) const MAX_CONCURRENT_REQUESTS: u32 = 16;

/// How often a quiet connection is pinged. A person approving a pairing request leaves
/// the connection silent for a while, and the pings stop the idle timeout from ending
/// it.
const KEEP_ALIVE: Duration = Duration::from_secs(10);

/// How long a connection may be silent before it is dropped, in milliseconds.
const IDLE_TIMEOUT_MS: u32 = 30_000;

const MIB: u32 = 1024 * 1024;

/// The speed of the path the windows are sized for, in bytes a second: 500 Mbit/s.
const TARGET_RATE: u64 = 62_500_000;

/// The round trip of the path the windows are sized for, in milliseconds.
const TARGET_ROUND_TRIP_MS: u64 = 100;

/// Bytes in flight on the path the windows are sized for.
const TARGET_IN_FLIGHT: u64 = TARGET_RATE * TARGET_ROUND_TRIP_MS / 1000;

/// What the QUIC library allows a connection to hold when it is left alone: its stream
/// window for each of the streams a peer may open.
const LIBRARY_STREAM_WINDOW: u64 = 1_250_000;

/// How much unread data one stream may hold, and so how much a sender may have in
/// flight on it. A stranger never gets this much: its connection is held to
/// [`INITIAL_WINDOW`], which is smaller.
pub(crate) const STREAM_WINDOW: u32 = 8 * MIB;

/// How much unread data a whole connection to a paired device may hold. It is twice a
/// stream's window, so that a stream nobody reads cannot take all of it and stall the
/// others. A connection has this only once the gate has seen that the device is paired.
pub(crate) const CONNECTION_WINDOW: u32 = 2 * STREAM_WINDOW;

/// How much unread data a connection may hold until the device on the other end is known
/// to be paired, which is all a stranger ever gets. A pairing frame is at most 4 KiB, so
/// this holds fifteen of them with their headers.
pub(crate) const INITIAL_WINDOW: u32 = 64 * 1024;

/// How much this device sends on a connection before the other side has acknowledged
/// it. It bounds our own memory when a peer offers more room than we want to use.
pub(crate) const SEND_WINDOW: u64 = CONNECTION_WINDOW as u64;

/// The settings for the endpoint, and so for every connection it makes or accepts. The
/// connection window it announces is the small one, until [`raise_window`].
pub(crate) fn config() -> QuicTransportConfig {
    build(STREAM_WINDOW, INITIAL_WINDOW, SEND_WINDOW)
}

/// Raises a connection's window to [`CONNECTION_WINDOW`], for a device that is paired.
/// The peer learns of it in a `MAX_DATA` frame, so nothing is lost if data is already
/// on its way.
pub(crate) fn raise_window(connection: &Connection) {
    connection.set_receive_window(VarInt::from_u32(CONNECTION_WINDOW));
}

fn build(stream_window: u32, connection_window: u32, send_window: u64) -> QuicTransportConfig {
    QuicTransportConfig::builder()
        .max_concurrent_bidi_streams(VarInt::from_u32(MAX_CONCURRENT_REQUESTS))
        // Nothing in the protocol uses these, so peers may not open them.
        .max_concurrent_uni_streams(VarInt::from_u32(0))
        .datagram_receive_buffer_size(None)
        .stream_receive_window(VarInt::from_u32(stream_window))
        .receive_window(VarInt::from_u32(connection_window))
        .send_window(send_window)
        .keep_alive_interval(KEEP_ALIVE)
        .max_idle_timeout(Some(VarInt::from_u32(IDLE_TIMEOUT_MS).into()))
        .build()
}

// The relations the numbers above rely on. They are checked when the crate is built, so
// changing one value without the others is a compile error.
const _: () = {
    // The receiver raises a window in steps of an eighth, so seven eighths is what is
    // always available.
    assert!(
        STREAM_WINDOW as u64 * 7 / 8 >= TARGET_IN_FLIGHT,
        "a stream could not fill the path the windows are sized for"
    );
    assert!(
        CONNECTION_WINDOW as u64 * 7 / 8 >= TARGET_IN_FLIGHT,
        "the connection could not fill the path the windows are sized for"
    );
    assert!(
        CONNECTION_WINDOW >= 2 * STREAM_WINDOW,
        "a stream nobody reads could take the whole connection window and stall the others"
    );
    // A peer using these settings offers a connection window this big once we are paired,
    // and we should not hold back data it is willing to take.
    assert!(
        SEND_WINDOW >= CONNECTION_WINDOW as u64,
        "we would send less than the peer is willing to take"
    );
    // What a stranger can reach: pairing connections, each with the window every connection
    // starts with. A pairing request has to fit with room to spare, and all of them together
    // must stay small enough that a phone does not notice.
    assert!(
        INITIAL_WINDOW as usize >= 4 * (MAX_PAIR_FRAME_LEN + HEADER_LEN),
        "a pairing request would not fit in the window a stranger gets"
    );
    assert!(
        MAX_PAIRING_CONNECTIONS * INITIAL_WINDOW as usize <= MIB as usize,
        "strangers could make a device hold more than 1 MiB"
    );
    assert!(
        INITIAL_WINDOW < CONNECTION_WINDOW,
        "there would be nothing to raise"
    );
    // Bigger windows are a trade of memory for speed, and this is where the trade stops
    // for a paired device. A change that breaks either of these has to say, in the record
    // that explains these numbers, why a device may now hold more than it did.
    assert!(
        CONNECTION_WINDOW as u64 <= LIBRARY_STREAM_WINDOW * MAX_CONCURRENT_REQUESTS as u64,
        "a connection could make a device hold more than the library default allowed"
    );
    assert!(
        CONNECTION_WINDOW as u64 * MAX_CONNECTIONS_PER_PEER as u64 <= 128 * MIB as u64,
        "a paired device could make a device hold more than 128 MiB"
    );
};

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Instant};

    use iroh::{
        Endpoint, EndpointAddr, RelayMode, RelayUrl, endpoint::presets,
        test_utils::run_relay_server_with, tls::CaTlsConfig,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{
            TcpListener, TcpStream,
            tcp::{OwnedReadHalf, OwnedWriteHalf},
        },
        sync::mpsc,
    };

    use super::*;

    const ALPN: &[u8] = b"baton-window-test";

    #[test]
    fn the_stream_budget_is_still_small() {
        // The windows are large; the number of streams is not. This is what keeps
        // requests from piling up.
        assert_eq!(MAX_CONCURRENT_REQUESTS, 16);
    }

    /// A network path made by a TCP relay in front of the relay server: every chunk is
    /// held for `one_way` in each direction and, if there is a rate, queued behind the
    /// ones before it as on a link of that speed. Nothing is dropped, so a queue only
    /// gets longer, as it does behind a router with a large buffer.
    ///
    /// The devices in these tests have no direct address, so they reach each other only
    /// through the relay server, and the path they see is the one made here. A direct
    /// path between two devices on one machine would be found and used, since iroh
    /// switches to any path that is a few milliseconds faster.
    #[derive(Clone, Copy)]
    struct Path {
        one_way: Duration,
        bytes_per_second: Option<u64>,
    }

    /// Carries bytes from `from` to `to` along `path`.
    fn carry(path: Path, mut from: OwnedReadHalf, mut to: OwnedWriteHalf) {
        let (hold, mut release) = mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
        tokio::spawn(async move {
            let mut buffer = vec![0_u8; 64 * 1024];
            let mut link_free = Instant::now();
            while let Ok(len @ 1..) = from.read(&mut buffer).await {
                let mut leaves = Instant::now();
                if let Some(rate) = path.bytes_per_second {
                    let sending = u64::try_from(len).unwrap() * 1_000_000_000 / rate;
                    link_free = link_free.max(leaves) + Duration::from_nanos(sending);
                    leaves = link_free;
                }
                if hold
                    .send((leaves + path.one_way, buffer[..len].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            while let Some((due, chunk)) = release.recv().await {
                tokio::time::sleep_until(tokio::time::Instant::from_std(due)).await;
                if to.write_all(&chunk).await.is_err() {
                    break;
                }
            }
            let _ = to.shutdown().await;
        });
    }

    /// Starts a relay in front of `target` and returns the address to dial instead.
    async fn start_path(target: SocketAddr, path: Path) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let Ok(server) = TcpStream::connect(target).await else {
                    continue;
                };
                // Without this a small write can wait for the other side's delayed
                // acknowledgement, which would add tens of milliseconds to the path.
                if client.set_nodelay(true).is_err() || server.set_nodelay(true).is_err() {
                    continue;
                }
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                carry(path, client_read, server_write);
                carry(path, server_read, client_write);
            }
        });
        address
    }

    async fn endpoint(config: QuicTransportConfig, relay: &RelayUrl) -> Endpoint {
        let endpoint = Endpoint::builder(presets::Minimal)
            .alpns(vec![ALPN.to_vec()])
            .transport_config(config)
            .relay_mode(RelayMode::custom([relay.clone()]))
            .clear_ip_transports()
            .ca_tls_config(CaTlsConfig::insecure_skip_verify())
            .bind()
            .await
            .unwrap();
        endpoint.online().await;
        endpoint
    }

    /// What one run over a path saw.
    struct Measured {
        /// The median time for one byte to go to the other device and one to come back,
        /// on a path with nothing else on it.
        round_trip: Duration,
        /// From the first byte of the data written to the receiver's one-byte answer.
        elapsed: Duration,
        /// How long the receiver took to get the second half of the data, once the first
        /// half was in. A transfer starts slowly while the congestion controller opens up,
        /// so this is closer to what the settings allow than the whole run is.
        second_half: Duration,
    }

    /// Sends `bytes` bytes on one stream between two devices that reach each other only
    /// through `path`, after checking how long the path takes to answer. `raise_to` is
    /// what each device does once the connection is up, as the gate does for a paired
    /// device: raise its connection window.
    async fn transfer(
        config: &QuicTransportConfig,
        raise_to: Option<u32>,
        path: Path,
        bytes: usize,
    ) -> Measured {
        let (_map, _url, relay_server) = run_relay_server_with(false).await.unwrap();
        let through_path = start_path(relay_server.https_addr().unwrap(), path).await;
        let relay: RelayUrl = format!("https://{through_path}").parse().unwrap();

        let server = endpoint(config.clone(), &relay).await;
        let client = endpoint(config.clone(), &relay).await;

        let receiver = tokio::spawn({
            let server = server.clone();
            async move {
                let connection = server.accept().await.unwrap().await.unwrap();
                if let Some(window) = raise_to {
                    connection.set_receive_window(VarInt::from_u32(window));
                }
                let (mut echo, mut ping) = connection.accept_bi().await.unwrap();
                let mut byte = [0_u8; 1];
                while ping.read_exact(&mut byte).await.is_ok() {
                    echo.write_all(&byte).await.unwrap();
                }
                echo.finish().unwrap();
                let (mut reply, mut request) = connection.accept_bi().await.unwrap();
                let mut buffer = vec![0_u8; 256 * 1024];
                let mut received = 0;
                let mut half_in = None;
                while let Some(len) = request.read(&mut buffer).await.unwrap() {
                    received += len;
                    if half_in.is_none() && received >= bytes / 2 {
                        half_in = Some(Instant::now());
                    }
                }
                let second_half = half_in.map_or(Duration::ZERO, |at| at.elapsed());
                reply.write_all(b"k").await.unwrap();
                reply.finish().unwrap();
                connection.closed().await;
                (received, second_half)
            }
        });

        let address = EndpointAddr::new(server.id()).with_relay_url(relay);
        let connection = client.connect(address, ALPN).await.unwrap();
        if let Some(window) = raise_to {
            connection.set_receive_window(VarInt::from_u32(window));
        }

        // On an idle path, so this is the delay the path adds and nothing else.
        let (mut ping, mut echo) = connection.open_bi().await.unwrap();
        let mut times = Vec::new();
        for _ in 0..7 {
            let sent = Instant::now();
            ping.write_all(&[1]).await.unwrap();
            echo.read_exact(&mut [0_u8; 1]).await.unwrap();
            times.push(sent.elapsed());
        }
        ping.finish().unwrap();
        times.sort();
        let round_trip = times[times.len() / 2];

        let (mut send, mut answer) = connection.open_bi().await.unwrap();
        let chunk = vec![0x5a_u8; 64 * 1024];
        let chunks = bytes / chunk.len();
        let started = Instant::now();
        for _ in 0..chunks {
            send.write_all(&chunk).await.unwrap();
        }
        send.finish().unwrap();
        answer.read_to_end(8).await.unwrap();
        let elapsed = started.elapsed();
        connection.close(0_u8.into(), b"done");
        let (arrived, second_half) = receiver.await.unwrap();
        assert_eq!(arrived, chunks * chunk.len());
        client.close().await;
        server.close().await;
        Measured {
            round_trip,
            elapsed,
            second_half,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_settings_carry_data_through_a_relay() {
        let path = Path {
            one_way: Duration::from_millis(2),
            bytes_per_second: None,
        };
        let measured = tokio::time::timeout(
            Duration::from_mins(1),
            transfer(&config(), Some(CONNECTION_WINDOW), path, 4 * MIB as usize),
        )
        .await
        .expect("the transfer never finished");
        // Both devices sit behind the delay, so a round trip crosses it four times. If
        // the devices ever found a way around it, the measurement below would be of
        // nothing, so this test is the one that would notice.
        assert!(
            measured.round_trip >= 4 * path.one_way,
            "the round trip took {:?}, so the devices are not using the delayed path",
            measured.round_trip
        );
    }

    /// The median of `values`, and the smallest and largest of them.
    fn spread(mut values: Vec<f64>) -> (f64, f64, f64) {
        values.sort_by(f64::total_cmp);
        (
            values[values.len() / 2],
            values[0],
            values[values.len() - 1],
        )
    }

    /// Measures what the windows buy. It is ignored because it takes a while and
    /// reports numbers instead of checking them. Run it with
    ///
    /// ```text
    /// cargo test -p baton-core --release --lib windows_on_a_long_path -- --ignored --nocapture
    /// ```
    ///
    /// The paths include one with a short round trip, where the windows should make no
    /// difference, so a change that slows the ordinary case would show too. Each cell is
    /// run several times, because one run says little: two runs of the same cell on a
    /// busy machine can differ by a fifth. Each is reported over the whole transfer and
    /// over its second half, which leaves out the time the congestion controller takes to
    /// open up, as the median of the runs with the smallest and the largest in brackets.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "measures throughput over an emulated path; run it by hand"]
    #[allow(clippy::cast_precision_loss)]
    async fn windows_on_a_long_path() {
        const RUNS: usize = 5;
        let bytes = 128 * MIB as usize;
        let paths = [
            ("100 ms round trip, 40 MB/s (320 Mbit/s)", 25, 40_000_000),
            ("100 ms round trip, 100 MB/s (800 Mbit/s)", 25, 100_000_000),
            ("20 ms round trip, 40 MB/s (320 Mbit/s)", 5, 40_000_000),
            ("4 ms round trip, 100 MB/s (800 Mbit/s)", 1, 100_000_000),
        ];
        // The window each device raises its connection to once it knows the other is
        // paired, as the gate does, or none for the library's own settings.
        let settings = [
            (
                "library default, 1.25 MB per stream",
                QuicTransportConfig::default(),
                None,
                LIBRARY_STREAM_WINDOW,
            ),
            (
                "these settings: 8 MiB per stream",
                config(),
                Some(CONNECTION_WINDOW),
                u64::from(STREAM_WINDOW),
            ),
            (
                "twice as large: 16 MiB per stream",
                build(2 * STREAM_WINDOW, INITIAL_WINDOW, 2 * SEND_WINDOW),
                Some(2 * CONNECTION_WINDOW),
                u64::from(2 * STREAM_WINDOW),
            ),
        ];
        println!(
            "\nsending {} MiB on one stream through a relay, {RUNS} runs for each cell",
            bytes / MIB as usize
        );
        println!(
            "MB/s: median (smallest to largest); ceiling = seven eighths of the stream window / round trip"
        );
        for (name, one_way_ms, bytes_per_second) in paths {
            let path = Path {
                one_way: Duration::from_millis(one_way_ms),
                bytes_per_second: Some(bytes_per_second),
            };
            println!("\n{name}");
            for (label, settings, raise_to, window) in &settings {
                let (mut whole, mut half, mut trips) = (Vec::new(), Vec::new(), Vec::new());
                for _ in 0..RUNS {
                    let run = transfer(settings, *raise_to, path, bytes).await;
                    whole.push(bytes as f64 / run.elapsed.as_secs_f64() / 1e6);
                    half.push(
                        (bytes / 2) as f64 / run.second_half.as_secs_f64().max(f64::EPSILON) / 1e6,
                    );
                    trips.push(run.round_trip.as_secs_f64());
                }
                let (whole, low, high) = spread(whole);
                let (half, half_low, half_high) = spread(half);
                let (trip, _, _) = spread(trips);
                println!(
                    "  {label:<42} whole run {whole:>5.1} ({low:.1} to {high:.1}), \
                     second half {half:>5.1} ({half_low:.1} to {half_high:.1}); \
                     round trip {:.0} ms, ceiling {:.0}",
                    trip * 1e3,
                    *window as f64 * 7.0 / 8.0 / trip / 1e6,
                );
            }
        }
    }
}
