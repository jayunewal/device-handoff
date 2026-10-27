// SPDX-License-Identifier: Apache-2.0

//! The sound of a session over real connections: what a source does with a `Hear` and how it sends
//! the packets, what a viewer does with the messages and the packets that come, the rules that each
//! side holds the other to, and sound that goes from the platform of one node to the speaker of
//! another.
//!
//! The first part has a real source in front of a viewer that a test writes by hand, the second a
//! real viewer in front of a source that a test writes by hand, and the third has two nodes.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use baton_proto::{
    id::SessionId, limits::MAX_SOUND_PACKET_BYTES, message::capability, stream::Codec,
};
use baton_stream::{
    KIND_CONTROL, KIND_SOUND,
    control::{
        Config as Format, Hear, InputBatch, Pause, PauseReason, Report, Silent, SilentReason,
        Sound, SoundCodec, SourceMessage, Thermal, ViewerMessage, Want,
    },
    header::{PREFIX_LEN, encode_prefix},
    sender::LtrToken,
    sound::{PacketFlags, PacketHeader, send::Stats as SoundStats},
};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::sync::mpsc;

use super::{
    End, Granted, MayHear, NotSent, Play, ShareOutcome, SoundPacket, SoundPlan, SourceEvents,
    SourceHandle, SourceParams, SourceSetup, Streaming, Takes, ViewHandle, ViewParams, Viewer,
    Viewing, WindowHandler, WindowOffer, WindowShare, WindowVerdict,
    answer_tests::{App, Heard, Plan, Platform, Speaking, Speech, Spoken, quick},
    offer_tests::fit,
    unlock::UnlockSetup,
};
use crate::{
    Config, Error, Identity, NodeBuilder, TrustStore,
    config::RelayMode,
    policy::{DevicePolicy, StartPolicy},
    testing::{Device, PATIENCE, QUIET, TestNode, connect, next, nothing_arrives, stopped_with},
    trust::PairedDevice,
    uni::{Arrival, Kind, Sinks},
    util::{BoxFuture, lock},
    wire::{read_next, write_message},
};

fn session_id() -> SessionId {
    SessionId::from_bytes([7; SessionId::LEN])
}

/// The form of sound that the first version sends.
pub(super) fn aac() -> Sound {
    Sound {
        codec: SoundCodec::Aac,
        rate: 48_000,
        channels: 2,
        samples: 1024,
    }
}

/// A packet for a source to send. Its first byte tells it from the others.
pub(super) fn packet(byte: u8, capture_time: u64) -> SoundPacket {
    SoundPacket {
        bytes: vec![byte; 300],
        capture_time,
    }
}

/// Waits until `condition` holds.
async fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// A report that says that the viewer is there.
fn report() -> ViewerMessage {
    ViewerMessage::Report(Report {
        shown: None,
        decode_us: 1000,
        dropped: 0,
        thermal: Thermal::Nominal,
    })
}

// A real source, in front of a viewer that the test writes.

/// What the platform of a source was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    Hear(bool),
    Ended,
}

#[derive(Clone, Default)]
struct Asks(Arc<Mutex<Vec<Asked>>>);

impl Asks {
    fn all(&self) -> Vec<Asked> {
        lock(&self.0).clone()
    }
}

struct Listening(Asks);

impl SourceEvents for Listening {
    fn report(&mut self, _: Report) {}

    fn want(&mut self, _: Want) {}

    fn input(&mut self, _: InputBatch, _: Duration) {}

    fn held(&mut self, _: Vec<LtrToken>) {}

    fn recovery(&mut self) {}

    fn hear(&mut self, on: bool) {
        lock(&(self.0).0).push(Asked::Hear(on));
    }

    fn ended(&mut self) {
        lock(&(self.0).0).push(Asked::Ended);
    }
}

/// The picture format of the tests.
fn format() -> Format {
    Format {
        epoch: 0,
        codec: Codec::H264,
        width: 1440,
        height: 900,
        fps: 60,
    }
}

/// What a viewer that can take a great deal says.
fn takes() -> Takes {
    Takes {
        max_width: 3840,
        max_height: 2160,
        max_fps: 120,
        codecs: vec![Codec::H264],
    }
}

fn quick_source() -> SourceParams {
    SourceParams {
        tick_every: Duration::from_millis(20),
        ..SourceParams::default()
    }
}

/// A source session, and the viewer's device in front of it.
struct SourceRig {
    _source: Device,
    _viewer: Device,
    accepted: Connection,
    packets: mpsc::UnboundedReceiver<Arrival>,
    control: mpsc::UnboundedReceiver<Arrival>,
    handle: SourceHandle,
    asks: Asks,
}

impl SourceRig {
    async fn new(plan: SoundPlan) -> Self {
        Self::with(plan, quick_source()).await
    }

    async fn with(plan: SoundPlan, params: SourceParams) -> Self {
        Self::letting(plan, params, MayHear::always()).await
    }

    /// A rig whose person at the source answers `may_hear` each time the viewer asks to hear.
    async fn letting(plan: SoundPlan, params: SourceParams, may_hear: MayHear) -> Self {
        let streaming = Streaming::new();
        let sinks = Sinks::default().install(Kind::Control, 1, streaming.clone());
        let source = Device::new(sinks, PATIENCE).await;
        let (viewer, mut receivers) =
            Device::taking(&[(Kind::Sound, 16), (Kind::Control, 1)]).await;
        let control = receivers.remove(1);
        let packets = receivers.remove(0);
        let (dialed, accepted) = connect(&source, &viewer).await;
        let asks = Asks::default();
        let paired = source.trust.get(&viewer.id).unwrap();
        let setup = SourceSetup {
            config: format(),
            control: false,
            private: false,
            takes: takes(),
            sound: plan,
            may_hear,
            unlock: UnlockSetup::Unsupported,
            leave: false,
            params,
        };
        let handle = streaming
            .source(
                &paired,
                dialed,
                None,
                session_id(),
                setup,
                Listening(asks.clone()),
            )
            .unwrap();
        Self {
            _source: source,
            _viewer: viewer,
            accepted,
            packets,
            control,
            handle,
            asks,
        }
    }

    /// The source's control stream, as the viewer reads it, after the picture format.
    async fn source_control(&mut self) -> RecvStream {
        let arrival = next(&mut self.control).await;
        assert!(matches!(arrival.kind, Kind::Control));
        let mut recv = arrival.recv;
        assert!(matches!(
            read_source_message(&mut recv).await,
            SourceMessage::Config(_)
        ));
        recv
    }

    /// Opens the viewer's control stream, as a viewer does once it has taken the source's.
    async fn viewer_control(&self) -> SendStream {
        let mut send = self.accepted.open_uni().await.unwrap();
        send.write_all(&encode_prefix(KIND_CONTROL, session_id()))
            .await
            .unwrap();
        send
    }

    /// Waits until the platform has been asked exactly this.
    async fn until_asked(&self, expected: &[Asked]) {
        until(&format!("the platform to be asked {expected:?}"), || {
            self.asks.all() == expected
        })
        .await;
    }

    async fn ended(&self) -> End {
        tokio::time::timeout(PATIENCE, self.handle.ended())
            .await
            .expect("the session did not end")
    }

    /// A viewer that asks to hear, and a platform that begins the sound: the viewer's control
    /// stream, and the source's after the `Sound`.
    async fn listening(plan: SoundPlan, params: SourceParams) -> (Self, SendStream, RecvStream) {
        Self::listening_letting(plan, params, MayHear::always()).await
    }

    /// The same, for a person at the source who answers `may_hear`.
    async fn listening_letting(
        plan: SoundPlan,
        params: SourceParams,
        may_hear: MayHear,
    ) -> (Self, SendStream, RecvStream) {
        let mut rig = Self::letting(plan, params, may_hear).await;
        let mut control = rig.source_control().await;
        let mut viewer = rig.viewer_control().await;
        // The viewer is there from the start, so that it is not quiet.
        write_message(&mut viewer, &report()).await.unwrap();
        write_message(&mut viewer, &ViewerMessage::Hear(Hear { on: true }))
            .await
            .unwrap();
        rig.until_asked(&[Asked::Hear(true)]).await;
        rig.handle.begin_sound(aac()).unwrap();
        assert_eq!(
            read_source_message(&mut control).await,
            SourceMessage::Sound(aac())
        );
        (rig, viewer, control)
    }
}

async fn read_source_message(recv: &mut RecvStream) -> SourceMessage {
    read_next::<SourceMessage>(recv, PATIENCE)
        .await
        .expect("the source wrote a message that cannot be read")
        .expect("the control stream of the source ended")
}

/// The next packet of sound, as the viewer reads it: the header, and the whole packet.
async fn read_packet(packets: &mut mpsc::UnboundedReceiver<Arrival>) -> (PacketHeader, Vec<u8>) {
    let arrival = next(packets).await;
    assert!(matches!(arrival.kind, Kind::Sound));
    let mut recv = arrival.recv;
    let mut head = encode_prefix(KIND_SOUND, session_id()).to_vec();
    head.resize(PacketHeader::LEN, 0);
    recv.read_exact(head.split_at_mut(PREFIX_LEN).1)
        .await
        .unwrap();
    let (header, _) = PacketHeader::parse(&head).unwrap();
    let packet = recv.read_to_end(MAX_SOUND_PACKET_BYTES).await.unwrap();
    (header, packet)
}

#[tokio::test]
async fn a_viewer_that_asks_to_hear_a_display_gets_the_sound_and_then_the_packets() {
    let (mut rig, _viewer, _control) =
        SourceRig::listening(SoundPlan::Display, quick_source()).await;
    assert_eq!(rig.asks.all(), [Asked::Hear(true)]);

    assert_eq!(rig.handle.sound(packet(1, 1_000)).unwrap(), Some(0));
    assert_eq!(rig.handle.sound(packet(2, 22_333)).unwrap(), Some(1));
    // Each is a stream of its own, and streams arrive in any order.
    let mut got = [
        read_packet(&mut rig.packets).await,
        read_packet(&mut rig.packets).await,
    ];
    got.sort_by_key(|(header, _)| header.number);
    let at = |number, capture_time, flags| PacketHeader {
        session: session_id(),
        number,
        capture_time,
        flags,
    };
    assert_eq!(got[0].0, at(0, 1_000, PacketFlags::FIRST));
    assert_eq!(got[0].1, vec![1; 300]);
    assert_eq!(got[1].0, at(1, 22_333, PacketFlags::NONE));
    assert_eq!(got[1].1, vec![2; 300]);
    let stats = rig.handle.stats();
    assert_eq!(stats.sound.sent, 2);
    until("the streams to stop counting", || {
        rig.handle.stats().sound_streams == 0
    })
    .await;
}

#[tokio::test]
async fn no_packet_is_taken_before_the_platform_has_said_what_the_packets_are() {
    let mut rig = SourceRig::new(SoundPlan::Display).await;
    let _control = rig.source_control().await;
    // Nobody has asked to hear.
    assert_eq!(rig.handle.sound(packet(1, 0)).unwrap(), None);
    let mut viewer = rig.viewer_control().await;
    write_message(&mut viewer, &report()).await.unwrap();
    write_message(&mut viewer, &ViewerMessage::Hear(Hear { on: true }))
        .await
        .unwrap();
    rig.until_asked(&[Asked::Hear(true)]).await;
    // The viewer has asked, and the platform has not begun.
    assert_eq!(rig.handle.sound(packet(1, 0)).unwrap(), None);
    nothing_arrives(&mut rig.packets).await;
    assert_eq!(rig.handle.stats().sound.sent, 0);
}

#[tokio::test]
async fn a_viewer_that_turns_sound_off_ends_the_run_and_the_platform_is_told() {
    let (mut rig, mut viewer, mut control) =
        SourceRig::listening(SoundPlan::Display, quick_source()).await;
    write_message(&mut viewer, &ViewerMessage::Hear(Hear { on: false }))
        .await
        .unwrap();
    rig.until_asked(&[Asked::Hear(true), Asked::Hear(false)])
        .await;
    assert_eq!(rig.handle.sound(packet(1, 5)).unwrap(), None);
    nothing_arrives(&mut rig.packets).await;
    // The platform may not have heard yet. Neither a run that it begins nor a `Silent` that it
    // says is for a viewer that does not want to hear, so the next the viewer reads is the pause.
    rig.handle.begin_sound(aac()).unwrap();
    rig.handle.silent(SilentReason::Failed);
    rig.handle.pause(PauseReason::Locked);
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Pause(Pause {
            reason: PauseReason::Locked
        })
    );
}

#[tokio::test]
async fn a_session_of_a_window_answers_a_hear_that_is_on_with_silent_and_asks_nothing() {
    let mut rig = SourceRig::new(SoundPlan::Window).await;
    let mut control = rig.source_control().await;
    let mut viewer = rig.viewer_control().await;
    let hear = |on| ViewerMessage::Hear(Hear { on });
    write_message(&mut viewer, &hear(true)).await.unwrap();
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Silent(Silent {
            reason: SilentReason::Window
        })
    );
    // Off says nothing, and on says it again.
    write_message(&mut viewer, &hear(false)).await.unwrap();
    write_message(&mut viewer, &hear(true)).await.unwrap();
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Silent(Silent {
            reason: SilentReason::Window
        })
    );
    assert_eq!(rig.asks.all(), []);
    assert_eq!(rig.handle.sound(packet(1, 0)).unwrap(), None);
}

/// What the person at a source says of a viewer that hears: nothing is allowed until the flag is
/// set, and what the flag says by then is what counts each time the viewer asks.
fn permission() -> (Arc<AtomicBool>, MayHear) {
    let allowed = Arc::new(AtomicBool::new(false));
    let may_hear = MayHear::new({
        let allowed = Arc::clone(&allowed);
        move || allowed.load(Ordering::SeqCst)
    });
    (allowed, may_hear)
}

#[tokio::test]
async fn a_viewer_that_may_not_hear_a_display_is_told_so_and_the_platform_is_not_asked() {
    let (allowed, may_hear) = permission();
    let mut rig = SourceRig::letting(SoundPlan::Display, quick_source(), may_hear).await;
    let mut control = rig.source_control().await;
    let mut viewer = rig.viewer_control().await;
    let hear = |on| ViewerMessage::Hear(Hear { on });
    let not_allowed = SourceMessage::Silent(Silent {
        reason: SilentReason::NotAllowed,
    });
    write_message(&mut viewer, &report()).await.unwrap();
    write_message(&mut viewer, &hear(true)).await.unwrap();
    assert_eq!(read_source_message(&mut control).await, not_allowed);
    // Off says nothing, and on says it again.
    write_message(&mut viewer, &hear(false)).await.unwrap();
    write_message(&mut viewer, &hear(true)).await.unwrap();
    assert_eq!(read_source_message(&mut control).await, not_allowed);
    // The platform was never asked, and what it hands over goes nowhere.
    assert_eq!(rig.asks.all(), []);
    rig.handle.begin_sound(aac()).unwrap();
    assert_eq!(rig.handle.sound(packet(1, 0)).unwrap(), None);
    nothing_arrives(&mut rig.packets).await;
    // The person lets the device hear. Its next request is taken, in the session that runs.
    allowed.store(true, Ordering::SeqCst);
    write_message(&mut viewer, &hear(true)).await.unwrap();
    rig.until_asked(&[Asked::Hear(true)]).await;
    rig.handle.begin_sound(aac()).unwrap();
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Sound(aac())
    );
    assert_eq!(rig.handle.sound(packet(1, 100)).unwrap(), Some(0));
}

#[tokio::test]
async fn a_run_goes_on_when_the_person_takes_the_permission_back_until_the_viewer_asks_again() {
    let (allowed, may_hear) = permission();
    allowed.store(true, Ordering::SeqCst);
    let (rig, mut viewer, mut control) =
        SourceRig::listening_letting(SoundPlan::Display, quick_source(), may_hear).await;
    allowed.store(false, Ordering::SeqCst);
    // Nothing is asked of the viewer or the platform when the person says so.
    assert_eq!(rig.handle.sound(packet(1, 100)).unwrap(), Some(0));
    assert_eq!(rig.asks.all(), [Asked::Hear(true)]);
    // The next request is the one that it governs: the platform is told to stop, so the device
    // that shares plays its own sound again, and the viewer is told why.
    write_message(&mut viewer, &ViewerMessage::Hear(Hear { on: true }))
        .await
        .unwrap();
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Silent(Silent {
            reason: SilentReason::NotAllowed
        })
    );
    rig.until_asked(&[Asked::Hear(true), Asked::Hear(false)])
        .await;
    assert_eq!(rig.handle.sound(packet(2, 200)).unwrap(), None);
}

#[tokio::test]
async fn a_hear_to_a_source_that_does_not_send_sound_is_a_violation() {
    let rig = SourceRig::new(SoundPlan::None).await;
    let mut viewer = rig.viewer_control().await;
    write_message(&mut viewer, &ViewerMessage::Hear(Hear { on: false }))
        .await
        .unwrap();
    assert_eq!(rig.ended().await, End::Violation);
    assert_eq!(stopped_with(&viewer).await, 400);
    assert!(rig.asks.all().contains(&Asked::Ended));
    assert!(!rig.asks.all().contains(&Asked::Hear(false)));
}

#[tokio::test]
async fn a_hear_that_is_on_again_asks_the_platform_to_try_again_when_no_run_goes() {
    let mut rig = SourceRig::new(SoundPlan::Display).await;
    let mut control = rig.source_control().await;
    let mut viewer = rig.viewer_control().await;
    let hear = |on| ViewerMessage::Hear(Hear { on });
    write_message(&mut viewer, &report()).await.unwrap();
    write_message(&mut viewer, &hear(true)).await.unwrap();
    rig.until_asked(&[Asked::Hear(true)]).await;
    // The platform cannot, and says so. The viewer asks again.
    rig.handle.silent(SilentReason::NoPermission);
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Silent(Silent {
            reason: SilentReason::NoPermission
        })
    );
    write_message(&mut viewer, &hear(true)).await.unwrap();
    rig.until_asked(&[Asked::Hear(true), Asked::Hear(true)])
        .await;
    // Now it can. A run goes, and another `Hear` that is on changes nothing.
    rig.handle.begin_sound(aac()).unwrap();
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Sound(aac())
    );
    write_message(&mut viewer, &hear(true)).await.unwrap();
    write_message(&mut viewer, &hear(false)).await.unwrap();
    // The `off` was read after the `on`, and the `on` asked nothing.
    rig.until_asked(&[Asked::Hear(true), Asked::Hear(true), Asked::Hear(false)])
        .await;
    // Ending the run by `Silent` is for a source that has one.
    rig.handle.silent(SilentReason::Failed);
    nothing_arrives(&mut rig.packets).await;
}

#[tokio::test]
async fn a_silent_ends_the_run_and_the_next_run_begins_with_a_flagged_packet() {
    let (mut rig, mut viewer, mut control) =
        SourceRig::listening(SoundPlan::Display, quick_source()).await;
    assert_eq!(rig.handle.sound(packet(1, 100)).unwrap(), Some(0));
    rig.handle.silent(SilentReason::Failed);
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Silent(Silent {
            reason: SilentReason::Failed
        })
    );
    // No packet follows a `Silent` until the next `Sound`.
    assert_eq!(rig.handle.sound(packet(2, 200)).unwrap(), None);
    write_message(&mut viewer, &ViewerMessage::Hear(Hear { on: true }))
        .await
        .unwrap();
    rig.until_asked(&[Asked::Hear(true), Asked::Hear(true)])
        .await;
    rig.handle.begin_sound(aac()).unwrap();
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Sound(aac())
    );
    // The numbers go on, and the packet that begins the run is flagged.
    assert_eq!(rig.handle.sound(packet(3, 300)).unwrap(), Some(1));
    let mut got = [
        read_packet(&mut rig.packets).await,
        read_packet(&mut rig.packets).await,
    ];
    got.sort_by_key(|(header, _)| header.number);
    assert_eq!(got[0].0.flags, PacketFlags::FIRST);
    assert_eq!(got[1].0.number, 1);
    assert_eq!(got[1].0.flags, PacketFlags::FIRST);
    assert_eq!(got[1].1, vec![3; 300]);
}

#[tokio::test]
async fn a_viewer_that_goes_quiet_loses_the_run_and_gets_a_new_one_when_it_comes_back() {
    let params = SourceParams {
        quiet_after: Duration::from_millis(250),
        ..quick_source()
    };
    let (mut rig, mut viewer, mut control) = SourceRig::listening(SoundPlan::Display, params).await;
    // It says nothing, so the sound of the screen is not held back for it any more.
    rig.until_asked(&[Asked::Hear(true), Asked::Hear(false)])
        .await;
    assert_eq!(rig.handle.sound(packet(1, 0)).unwrap(), None);
    // The platform may not have heard yet, and a run that it begins now is not for a viewer that
    // is quiet: the next the viewer reads is the pause.
    rig.handle.begin_sound(aac()).unwrap();
    rig.handle.pause(PauseReason::Locked);
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Pause(Pause {
            reason: PauseReason::Locked
        })
    );
    // It comes back, and the run it asked for begins again.
    write_message(&mut viewer, &report()).await.unwrap();
    rig.until_asked(&[Asked::Hear(true), Asked::Hear(false), Asked::Hear(true)])
        .await;
    rig.handle.begin_sound(aac()).unwrap();
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Sound(aac())
    );
    assert_eq!(rig.handle.sound(packet(2, 0)).unwrap(), Some(0));
    let (header, _) = read_packet(&mut rig.packets).await;
    assert_eq!((header.number, header.flags), (0, PacketFlags::FIRST));
}

#[tokio::test]
async fn a_session_has_one_form_of_sound() {
    let (rig, mut viewer, mut control) =
        SourceRig::listening(SoundPlan::Display, quick_source()).await;
    let hear = |on| ViewerMessage::Hear(Hear { on });
    write_message(&mut viewer, &hear(false)).await.unwrap();
    write_message(&mut viewer, &hear(true)).await.unwrap();
    rig.until_asked(&[Asked::Hear(true), Asked::Hear(false), Asked::Hear(true)])
        .await;
    // A form that is another is refused, and nothing is said of it.
    let other = Sound {
        rate: 44_100,
        ..aac()
    };
    assert!(matches!(
        rig.handle.begin_sound(other),
        Err(Error::Config(_))
    ));
    // A form that is beyond the protocol is refused too.
    for beyond in [
        Sound { rate: 0, ..aac() },
        Sound {
            channels: 3,
            ..aac()
        },
        Sound {
            samples: 255,
            ..aac()
        },
    ] {
        assert!(rig.handle.begin_sound(beyond).is_err(), "{beyond:?}");
    }
    // The same form begins the new run.
    rig.handle.begin_sound(aac()).unwrap();
    assert_eq!(
        read_source_message(&mut control).await,
        SourceMessage::Sound(aac())
    );
}

#[tokio::test]
async fn a_packet_over_the_limit_is_not_sent_and_takes_no_number() {
    let (mut rig, _viewer, _control) =
        SourceRig::listening(SoundPlan::Display, quick_source()).await;
    let packet_of = |len| SoundPacket {
        bytes: vec![0x5a; len],
        capture_time: 0,
    };
    assert!(matches!(
        rig.handle.sound(packet_of(MAX_SOUND_PACKET_BYTES + 1)),
        Err(NotSent::TooLarge)
    ));
    assert_eq!(
        rig.handle.sound(packet_of(MAX_SOUND_PACKET_BYTES)).unwrap(),
        Some(0)
    );
    let (header, bytes) = read_packet(&mut rig.packets).await;
    assert_eq!(header.number, 0);
    assert_eq!(bytes.len(), MAX_SOUND_PACKET_BYTES);

    rig.handle.end();
    assert!(matches!(rig.handle.sound(packet(1, 0)), Err(NotSent::Over)));
}

#[tokio::test]
async fn a_packet_is_skipped_while_sixteen_streams_still_count_at_the_viewer() {
    let (rig, _viewer, _control) = SourceRig::listening(SoundPlan::Display, quick_source()).await;
    // The runtime of the test does not run what the calls spawn before the test waits for
    // something, so no stream is let go of between the calls.
    let sent: Vec<Option<u64>> = (0..20_u8)
        .map(|number| rig.handle.sound(packet(number, u64::from(number))).unwrap())
        .collect();
    assert_eq!(sent[..16], (0..16).map(Some).collect::<Vec<_>>());
    assert!(sent[16..].iter().all(Option::is_none), "{sent:?}");
    assert_eq!(rig.handle.stats().sound_streams, 16);
    // The packets that were not sent took numbers all the same, so that the viewer sees that
    // they are missing.
    until("the streams to be let go of", || {
        rig.handle.stats().sound_streams == 0
    })
    .await;
    assert_eq!(rig.handle.sound(packet(99, 99)).unwrap(), Some(20));
}

// A real viewer, in front of a source that the test writes.

/// An application with nothing to show: the tests start their sessions by hand.
struct NoOffers;

impl WindowHandler for NoOffers {
    fn offered<'a>(
        &'a self,
        _from: &'a PairedDevice,
        _offer: &'a WindowOffer,
    ) -> BoxFuture<'a, WindowVerdict> {
        Box::pin(async { WindowVerdict::Refuse })
    }

    fn started(&self, _from: &PairedDevice, _handle: ViewHandle) {}
}

/// A decoder that takes everything and does nothing.
struct Mute;

impl super::Decoder for Mute {
    fn decode(
        &mut self,
        _frame: &baton_stream::header::FrameHeader,
        _picture: Vec<u8>,
    ) -> Result<(), super::DecodeError> {
        Ok(())
    }

    fn show(&mut self, _number: u64) {}

    fn source(&mut self, _message: SourceMessage) {}

    fn state(&self) -> super::DecoderState {
        super::DecoderState {
            decode_us: 1000,
            thermal: Thermal::Nominal,
        }
    }

    fn ended(&mut self) {}
}

/// A viewer session of a device that plays sound, and the source's device in front of it.
struct ViewRig {
    _viewer: Device,
    _source: Device,
    dialed: Connection,
    control: mpsc::UnboundedReceiver<Arrival>,
    handle: ViewHandle,
    speech: Speech,
}

impl ViewRig {
    async fn new() -> Self {
        Self::with(quick(), true, true).await
    }

    /// `source_sounds` is whether the source advertised that it sends sound, `speaker` whether the
    /// application gave the session a speaker.
    async fn with(params: ViewParams, source_sounds: bool, speaker: bool) -> Self {
        let handler: Arc<dyn WindowHandler> = Arc::new(NoOffers);
        let viewer_side = Viewer::new(handler, |_| true, Duration::from_secs(1)).hearing(true);
        let streaming = Streaming::taking_part(Some(viewer_side), None);
        let sinks = Sinks::default()
            .install(Kind::Video, 12, streaming.clone())
            .install(Kind::Control, 2, streaming.clone())
            .install(Kind::Sound, 16, streaming.clone());
        let viewer = Device::new(sinks, PATIENCE).await;
        let (source, mut receivers) = Device::taking(&[(Kind::Control, 1)]).await;
        let control = receivers.remove(0);
        let (dialed, accepted) = connect(&source, &viewer).await;
        let speech = Speech::default();
        let mut viewing = Viewing::raw(Mute, takes(), params);
        if speaker {
            viewing = viewing.with_speaker(Speaking(speech.clone()));
        }
        let paired = viewer.trust.get(&source.id).unwrap();
        let granted = Granted {
            control: false,
            private: false,
            sound: source_sounds,
            unlock: false,
            leave: false,
        };
        let handle = streaming
            .view_granted(&paired, accepted, session_id(), viewing, granted)
            .unwrap();
        Self {
            _viewer: viewer,
            _source: source,
            dialed,
            control,
            handle,
            speech,
        }
    }

    /// Opens the source's control stream, as a source does once the viewer has accepted.
    async fn open_source_control(&self) -> SendStream {
        let mut send = self.dialed.open_uni().await.unwrap();
        send.write_all(&encode_prefix(KIND_CONTROL, session_id()))
            .await
            .unwrap();
        send
    }

    /// The viewer's control stream, as the source reads it. The source's must be open.
    async fn viewer_control(&mut self) -> RecvStream {
        let arrival = next(&mut self.control).await;
        assert!(matches!(arrival.kind, Kind::Control));
        arrival.recv
    }

    /// The bytes of a packet header.
    fn header(number: u64, flags: PacketFlags) -> Vec<u8> {
        PacketHeader {
            session: session_id(),
            number,
            capture_time: number * 21_333,
            flags,
        }
        .encode()
    }

    /// Opens a stream, writes a packet on it and finishes it.
    async fn packet(&self, number: u64, first: bool, packet: &[u8]) {
        let flags = if first {
            PacketFlags::FIRST
        } else {
            PacketFlags::NONE
        };
        let mut send = self.partial(number, flags, packet).await;
        send.finish().unwrap();
    }

    /// Like [`ViewRig::packet`], but the stream stays open.
    async fn partial(&self, number: u64, flags: PacketFlags, packet: &[u8]) -> SendStream {
        let mut send = self.dialed.open_uni().await.unwrap();
        let mut bytes = Self::header(number, flags);
        bytes.extend_from_slice(packet);
        send.write_all(&bytes).await.unwrap();
        send
    }

    async fn ended(&self) -> End {
        tokio::time::timeout(PATIENCE, self.handle.ended())
            .await
            .expect("the session did not end")
    }

    /// Waits until `condition` holds.
    async fn until(&self, what: &str, condition: impl Fn(&Self) -> bool) {
        until(what, || condition(self)).await;
    }

    /// The person asks to hear, and the source begins a run: the source's control stream, and the
    /// viewer's.
    async fn running(&mut self) -> (SendStream, RecvStream) {
        assert!(self.handle.hear(true));
        let mut source_control = self.open_source_control().await;
        let mut viewer_control = self.viewer_control().await;
        assert_eq!(heard(&mut viewer_control).await, Hear { on: true });
        write_message(&mut source_control, &SourceMessage::Sound(aac()))
            .await
            .unwrap();
        self.until("the speaker to be told the form", |rig| {
            rig.speech.all() == [Spoken::Sound(aac())]
        })
        .await;
        (source_control, viewer_control)
    }
}

/// The next `Hear` of the viewer, among its reports. The reports keep coming, so a viewer that
/// never says it is waited for only so long.
async fn heard(recv: &mut RecvStream) -> Hear {
    let deadline = Instant::now() + PATIENCE;
    loop {
        assert!(
            Instant::now() < deadline,
            "the viewer said no `Hear` in time"
        );
        match read_next::<ViewerMessage>(recv, PATIENCE).await {
            Ok(Some(ViewerMessage::Hear(hear))) => return hear,
            Ok(Some(ViewerMessage::Report(_))) => {}
            other => panic!("the viewer wrote something else: {other:?}"),
        }
    }
}

#[tokio::test]
async fn the_run_that_the_source_begins_is_played_in_order() {
    let mut rig = ViewRig::new().await;
    let (_source_control, _viewer_control) = rig.running().await;
    // Out of order, as streams may arrive.
    rig.packet(2, false, b"two").await;
    rig.packet(0, true, b"zero").await;
    rig.packet(1, false, b"one").await;
    rig.until("the three packets to be played", |rig| {
        rig.speech.played().len() == 3
    })
    .await;
    assert_eq!(
        rig.speech.played(),
        [
            (0, true, 0, b"zero".to_vec()),
            (1, false, 0, b"one".to_vec()),
            (2, false, 0, b"two".to_vec()),
        ]
    );
    // The speaker is told when each was captured, on the clock of the source.
    assert!(rig.speech.all().contains(&Spoken::Play(
        Play {
            number: 1,
            capture_time: 21_333,
            first: false,
            lost_before: 0,
        },
        b"one".to_vec()
    )));
    let sound = rig.handle.stats().sound;
    assert_eq!((sound.played, sound.lost, sound.late), (3, 0, 0));
    rig.until("the streams to be let go of", |rig| {
        rig.handle.stats().sound_streams == 0
    })
    .await;
}

#[tokio::test]
async fn a_packet_that_does_not_come_in_time_is_skipped_and_counted() {
    let mut rig = ViewRig::new().await;
    let (_source_control, _viewer_control) = rig.running().await;
    rig.packet(0, true, b"zero").await;
    rig.packet(2, false, b"two").await;
    // Packet 1 does not come within the hold of 40 ms, and the viewer goes on without it.
    rig.until("both to be played", |rig| rig.speech.played().len() == 2)
        .await;
    assert_eq!(rig.speech.played()[1], (2, false, 1, b"two".to_vec()));
    assert_eq!(rig.handle.stats().sound.lost, 1);
    // It comes after all, and is not played.
    rig.packet(1, false, b"one").await;
    rig.until("the late packet to be dropped", |rig| {
        rig.handle.stats().sound.late == 1
    })
    .await;
    assert_eq!(rig.speech.played().len(), 2);
}

#[tokio::test]
async fn turning_sound_off_stops_the_speaker_and_the_packets_and_tells_the_source() {
    let mut rig = ViewRig::new().await;
    let (mut source_control, mut viewer_control) = rig.running().await;
    rig.packet(0, true, b"zero").await;
    rig.until("the packet to be played", |rig| {
        rig.speech.played().len() == 1
    })
    .await;

    assert!(rig.handle.hear(false));
    assert_eq!(
        rig.speech.all().last().cloned(),
        Some(Spoken::Stopped(None))
    );
    assert_eq!(heard(&mut viewer_control).await, Hear { on: false });
    // A packet that was on its way is stopped, and no one is at fault.
    let late = rig.partial(1, PacketFlags::NONE, b"one").await;
    assert_eq!(stopped_with(&late).await, 499);
    // Off again says nothing: what the source reads next is the `on` that follows.
    assert!(rig.handle.hear(false));
    assert!(rig.handle.hear(true));
    assert_eq!(heard(&mut viewer_control).await, Hear { on: true });
    // The source begins a run, and the numbers go on.
    write_message(&mut source_control, &SourceMessage::Sound(aac()))
        .await
        .unwrap();
    rig.packet(5, true, b"five").await;
    rig.until("the packet of the new run to be played", |rig| {
        rig.speech.played().len() == 2
    })
    .await;
    assert_eq!(rig.speech.played()[1], (5, true, 0, b"five".to_vec()));
}

#[tokio::test]
async fn asking_again_while_sound_is_on_asks_the_source_again() {
    let mut rig = ViewRig::new().await;
    let (_source_control, mut viewer_control) = rig.running().await;
    // A source that could not capture at first tries again only if it hears of it.
    assert!(rig.handle.hear(true));
    assert_eq!(heard(&mut viewer_control).await, Hear { on: true });
}

#[tokio::test]
async fn a_silent_from_the_source_tells_the_speaker_why_and_the_run_can_begin_again() {
    let mut rig = ViewRig::new().await;
    let (mut source_control, _viewer_control) = rig.running().await;
    rig.packet(0, true, b"zero").await;
    rig.until("the packet to be played", |rig| {
        rig.speech.played().len() == 1
    })
    .await;
    write_message(
        &mut source_control,
        &SourceMessage::Silent(Silent {
            reason: SilentReason::NoPermission,
        }),
    )
    .await
    .unwrap();
    rig.until("the speaker to be stopped", |rig| {
        rig.speech.all().last() == Some(&Spoken::Stopped(Some(SilentReason::NoPermission)))
    })
    .await;
    // No packet follows a `Silent`, and the viewer still wants to hear.
    let stray = rig.partial(1, PacketFlags::NONE, b"one").await;
    assert_eq!(stopped_with(&stray).await, 499);
    write_message(&mut source_control, &SourceMessage::Sound(aac()))
        .await
        .unwrap();
    rig.packet(7, true, b"seven").await;
    rig.until("the new run to be played", |rig| {
        rig.speech.played().len() == 2
    })
    .await;
    assert_eq!(rig.speech.played()[1], (7, true, 0, b"seven".to_vec()));
}

#[tokio::test]
async fn sound_that_nobody_asked_for_is_dropped_and_is_no_violation() {
    let rig = ViewRig::new().await;
    let mut source_control = rig.open_source_control().await;
    write_message(&mut source_control, &SourceMessage::Sound(aac()))
        .await
        .unwrap();
    write_message(
        &mut source_control,
        &SourceMessage::Silent(Silent {
            reason: SilentReason::Failed,
        }),
    )
    .await
    .unwrap();
    let packet = rig.partial(0, PacketFlags::FIRST, b"zero").await;
    assert_eq!(stopped_with(&packet).await, 499);
    assert!(
        tokio::time::timeout(QUIET, rig.handle.ended())
            .await
            .is_err(),
        "the session ended"
    );
    assert_eq!(rig.speech.all(), []);
    assert_eq!(rig.handle.stats().sound.refused, 1);
}

#[tokio::test]
async fn a_session_without_a_speaker_drops_what_the_source_says_about_sound() {
    let mut rig = ViewRig::with(quick(), true, false).await;
    assert!(!rig.handle.can_hear());
    assert!(!rig.handle.hear(true));
    let mut source_control = rig.open_source_control().await;
    let mut viewer_control = rig.viewer_control().await;
    write_message(&mut source_control, &SourceMessage::Sound(aac()))
        .await
        .unwrap();
    // Nothing was said to the source: the next thing it reads is what is said after.
    rig.handle.want(Want {
        width: 800,
        height: 600,
        fps: 30,
    });
    loop {
        match read_next::<ViewerMessage>(&mut viewer_control, PATIENCE).await {
            Ok(Some(ViewerMessage::Report(_))) => {}
            Ok(Some(ViewerMessage::Want(_))) => break,
            other => panic!("the viewer wrote something else: {other:?}"),
        }
    }
    let packet = rig.partial(0, PacketFlags::FIRST, b"zero").await;
    assert_eq!(stopped_with(&packet).await, 499);
    assert!(
        tokio::time::timeout(QUIET, rig.handle.ended())
            .await
            .is_err(),
        "the session ended"
    );
}

#[tokio::test]
async fn a_viewer_cannot_ask_a_source_that_did_not_say_it_sends_sound() {
    let mut rig = ViewRig::with(quick(), false, true).await;
    assert!(!rig.handle.can_hear());
    assert!(!rig.handle.hear(true));
    let _source_control = rig.open_source_control().await;
    let mut viewer_control = rig.viewer_control().await;
    rig.handle.want(Want {
        width: 800,
        height: 600,
        fps: 30,
    });
    loop {
        match read_next::<ViewerMessage>(&mut viewer_control, PATIENCE).await {
            Ok(Some(ViewerMessage::Report(_))) => {}
            Ok(Some(ViewerMessage::Want(_))) => break,
            other => panic!("the viewer wrote something else: {other:?}"),
        }
    }
    rig.handle.end();
    assert!(!rig.handle.hear(true));
}

#[tokio::test]
async fn a_second_form_of_sound_in_a_session_is_a_violation() {
    let mut rig = ViewRig::new().await;
    let (mut source_control, _viewer_control) = rig.running().await;
    // The same again is a run that begins again.
    write_message(&mut source_control, &SourceMessage::Sound(aac()))
        .await
        .unwrap();
    rig.until("the speaker to be told again", |rig| {
        rig.speech.all().len() == 2
    })
    .await;
    let other = Sound {
        rate: 44_100,
        ..aac()
    };
    write_message(&mut source_control, &SourceMessage::Sound(other))
        .await
        .unwrap();
    assert_eq!(rig.ended().await, End::Violation);
    assert_eq!(stopped_with(&source_control).await, 400);
    assert_eq!(rig.speech.all().last(), Some(&Spoken::Ended));
}

#[tokio::test]
async fn a_form_that_cannot_be_played_turns_sound_off_and_the_source_is_told() {
    for (what, form) in [
        (
            "a codec that is not known",
            Sound {
                codec: SoundCodec::Other,
                ..aac()
            },
        ),
        (
            "samples that AAC-LC does not have",
            Sound {
                samples: 480,
                ..aac()
            },
        ),
        (
            "samples of the long variant",
            Sound {
                samples: 2048,
                ..aac()
            },
        ),
    ] {
        let mut rig = ViewRig::new().await;
        assert!(rig.handle.hear(true));
        let mut source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        assert_eq!(heard(&mut viewer_control).await, Hear { on: true });
        write_message(&mut source_control, &SourceMessage::Sound(form))
            .await
            .unwrap();
        // The speaker is told why there is no sound, and the source is told to stop.
        assert_eq!(
            heard(&mut viewer_control).await,
            Hear { on: false },
            "{what}"
        );
        assert_eq!(
            rig.speech.all(),
            [Spoken::Stopped(Some(SilentReason::Other))],
            "{what}"
        );
        // Packets are not taken, and the session goes on.
        let packet = rig.partial(0, PacketFlags::FIRST, b"zero").await;
        assert_eq!(stopped_with(&packet).await, 499, "{what}");
        assert!(
            tokio::time::timeout(QUIET, rig.handle.ended())
                .await
                .is_err(),
            "{what}"
        );
        // Sound is off by now, so the person turning it off says nothing more to the speaker or
        // the source: what the source reads next is the `on` that follows.
        assert!(rig.handle.hear(false));
        assert_eq!(
            rig.speech.all(),
            [Spoken::Stopped(Some(SilentReason::Other))],
            "{what}"
        );
        // The person can ask again, and a form that can be played begins a run.
        assert!(rig.handle.hear(true));
        assert_eq!(heard(&mut viewer_control).await, Hear { on: true });
        write_message(&mut source_control, &SourceMessage::Sound(aac()))
            .await
            .unwrap();
        rig.until("the speaker to be told the form", |rig| {
            rig.speech.all().last() == Some(&Spoken::Sound(aac()))
        })
        .await;
    }
}

#[tokio::test]
async fn a_platform_that_cannot_play_the_run_turns_sound_off_and_the_source_is_told() {
    let mut rig = ViewRig::new().await;
    let (mut source_control, mut viewer_control) = rig.running().await;
    rig.packet(0, true, b"zero").await;
    rig.until("the packet to be played", |rig| {
        rig.speech.played().len() == 1
    })
    .await;

    // The platform says so from a thread of its own, after it took the run. The speaker is told
    // that it was the device that could not play, and the source is told to stop.
    rig.handle.sound_failed();
    assert_eq!(rig.speech.all().last(), Some(&Spoken::CouldNotPlay));
    assert_eq!(heard(&mut viewer_control).await, Hear { on: false });
    // A packet that was on its way is stopped, and no one is at fault.
    let late = rig.partial(1, PacketFlags::NONE, b"one").await;
    assert_eq!(stopped_with(&late).await, 499);
    assert!(
        tokio::time::timeout(QUIET, rig.handle.ended())
            .await
            .is_err(),
        "the session ended"
    );

    // Sound is off by now: saying so again, or the person turning it off, tells the speaker and
    // the source nothing more, and what the source reads next is the `on` that follows.
    rig.handle.sound_failed();
    assert!(rig.handle.hear(false));
    assert_eq!(
        rig.speech
            .all()
            .iter()
            .filter(|spoken| **spoken == Spoken::CouldNotPlay)
            .count(),
        1
    );
    assert_eq!(rig.speech.all().last(), Some(&Spoken::CouldNotPlay));

    // The person asks again, and the source begins a new run, which the platform is given to try.
    assert!(rig.handle.hear(true));
    assert_eq!(heard(&mut viewer_control).await, Hear { on: true });
    write_message(&mut source_control, &SourceMessage::Sound(aac()))
        .await
        .unwrap();
    rig.packet(5, true, b"five").await;
    rig.until("the packet of the new run to be played", |rig| {
        rig.speech.played().len() == 2
    })
    .await;
    assert_eq!(rig.speech.played()[1], (5, true, 0, b"five".to_vec()));
    assert_eq!(rig.speech.all()[3], Spoken::Sound(aac()));
}

#[tokio::test]
async fn a_report_that_the_platform_cannot_play_is_about_a_run_that_is_over_unless_sound_is_on() {
    // Sound that nobody asked for: there is no run, and nothing is told to anyone.
    let mut rig = ViewRig::new().await;
    rig.handle.sound_failed();
    assert_eq!(rig.speech.all(), []);
    // The person asks, and the first thing the source reads is that. The report said nothing.
    assert!(rig.handle.hear(true));
    let _source_control = rig.open_source_control().await;
    let mut viewer_control = rig.viewer_control().await;
    assert_eq!(heard(&mut viewer_control).await, Hear { on: true });

    // The person turned sound off, and then the report of the run they turned off comes.
    assert!(rig.handle.hear(false));
    assert_eq!(heard(&mut viewer_control).await, Hear { on: false });
    rig.handle.sound_failed();
    assert_eq!(rig.speech.all(), [Spoken::Stopped(None)]);
    assert!(rig.handle.hear(true));
    assert_eq!(heard(&mut viewer_control).await, Hear { on: true });

    // A session without a speaker has no sound to fail, and one that is over takes no report.
    let none = ViewRig::with(quick(), true, false).await;
    none.handle.sound_failed();
    assert!(!none.handle.hear(true));
    rig.handle.end();
    rig.ended().await;
    rig.handle.sound_failed();
    assert_eq!(rig.speech.all(), [Spoken::Stopped(None), Spoken::Ended,]);
}

#[tokio::test]
async fn a_packet_that_breaks_a_rule_ends_the_session() {
    // The last byte of a header holds the flags, and bit 1 is not defined.
    let reserved = {
        let mut bytes = ViewRig::header(0, PacketFlags::FIRST);
        *bytes.last_mut().unwrap() = 0b10;
        bytes
    };
    let mut rig = ViewRig::new().await;
    let (_source_control, _viewer_control) = rig.running().await;
    let mut send = rig.dialed.open_uni().await.unwrap();
    send.write_all(&reserved).await.unwrap();
    send.write_all(&[0; 10]).await.unwrap();
    assert_eq!(stopped_with(&send).await, 400);
    assert_eq!(rig.ended().await, End::Violation);
    assert_eq!(rig.speech.all().last(), Some(&Spoken::Ended));
}

#[tokio::test]
async fn a_header_that_is_cut_short_ends_the_session() {
    let mut rig = ViewRig::new().await;
    let (_source_control, _viewer_control) = rig.running().await;
    let mut send = rig.dialed.open_uni().await.unwrap();
    send.write_all(&ViewRig::header(0, PacketFlags::FIRST)[..30])
        .await
        .unwrap();
    send.finish().unwrap();
    assert_eq!(rig.ended().await, End::Violation);
}

#[tokio::test]
async fn a_packet_number_that_comes_twice_ends_the_session() {
    let mut rig = ViewRig::new().await;
    let (_source_control, _viewer_control) = rig.running().await;
    rig.packet(0, true, b"zero").await;
    // Packet 2 waits for packet 1, and a second packet 2 cannot be told from it.
    rig.packet(2, false, b"two").await;
    rig.until("packet 2 to wait", |rig| {
        rig.handle.stats().sound_streams == 0 && rig.speech.played().len() == 1
    })
    .await;
    rig.packet(2, false, b"two").await;
    assert_eq!(rig.ended().await, End::Violation);
    assert_eq!(rig.speech.all().last(), Some(&Spoken::Ended));
}

#[tokio::test]
async fn a_packet_over_the_limit_is_stopped_with_413_and_no_one_is_at_fault() {
    let mut rig = ViewRig::new().await;
    let (_source_control, _viewer_control) = rig.running().await;
    let mut send = rig.partial(0, PacketFlags::FIRST, &[]).await;
    // Written in pieces, so that the viewer reads while the source writes. The writes fail once
    // the viewer has stopped the stream.
    for _ in 0..4 {
        if send.write_all(&[0; 4096]).await.is_err() {
            break;
        }
    }
    assert_eq!(stopped_with(&send).await, 413);
    assert_eq!(rig.speech.played(), []);
    rig.until("the stream to be forgotten", |rig| {
        rig.handle.stats().sound_streams == 0
    })
    .await;
    // The session goes on, and the next packet is played.
    rig.packet(1, true, b"one").await;
    rig.until("the packet to be played", |rig| {
        rig.speech.played().len() == 1
    })
    .await;
}

#[tokio::test]
async fn a_packet_stream_beyond_what_the_viewer_takes_is_stopped_with_499() {
    let mut params = quick();
    params.sound.max_streams = 2;
    let mut rig = ViewRig::with(params, true, true).await;
    let (_source_control, _viewer_control) = rig.running().await;
    let first = rig.partial(0, PacketFlags::FIRST, b"zero").await;
    let second = rig.partial(1, PacketFlags::NONE, b"one").await;
    rig.until("both streams to be read", |rig| {
        rig.handle.stats().sound_streams == 2
    })
    .await;
    let third = rig.partial(2, PacketFlags::NONE, b"two").await;
    assert_eq!(stopped_with(&third).await, 499);
    assert_eq!(rig.handle.stats().sound.refused, 1);
    // The two that were taken are played once they are whole.
    for mut send in [first, second] {
        send.finish().unwrap();
    }
    rig.until("both to be played", |rig| rig.speech.played().len() == 2)
        .await;
    assert!(
        tokio::time::timeout(QUIET, rig.handle.ended())
            .await
            .is_err(),
        "the session ended"
    );
}

#[tokio::test]
async fn a_packet_stream_that_delivers_nothing_is_stopped_with_408() {
    let mut params = quick();
    params.stall = Duration::from_millis(200);
    let mut rig = ViewRig::with(params, true, true).await;
    let (_source_control, _viewer_control) = rig.running().await;
    let mut send = rig.dialed.open_uni().await.unwrap();
    // The start and half the rest of the header, and then nothing.
    send.write_all(&ViewRig::header(0, PacketFlags::FIRST)[..30])
        .await
        .unwrap();
    assert_eq!(stopped_with(&send).await, 408);
    assert_eq!(rig.handle.stats().sound_streams, 0);
    assert!(
        tokio::time::timeout(QUIET, rig.handle.ended())
            .await
            .is_err(),
        "the session ended"
    );
}

#[tokio::test]
async fn a_packet_that_comes_before_its_sound_is_dropped_as_lost() {
    let mut rig = ViewRig::new().await;
    assert!(rig.handle.hear(true));
    let mut source_control = rig.open_source_control().await;
    let mut viewer_control = rig.viewer_control().await;
    assert_eq!(heard(&mut viewer_control).await, Hear { on: true });
    // The packet has overtaken the message that describes it.
    let early = rig.partial(0, PacketFlags::FIRST, b"zero").await;
    assert_eq!(stopped_with(&early).await, 499);
    write_message(&mut source_control, &SourceMessage::Sound(aac()))
        .await
        .unwrap();
    rig.until("the speaker to be told the form", |rig| {
        rig.speech.all() == [Spoken::Sound(aac())]
    })
    .await;
    rig.packet(1, true, b"one").await;
    rig.until("the packet to be played", |rig| {
        rig.speech.played().len() == 1
    })
    .await;
    assert_eq!(rig.speech.played(), [(1, true, 0, b"one".to_vec())]);
}

#[tokio::test]
async fn the_end_of_a_session_tells_the_speaker_once_and_later_packets_are_not_played() {
    let mut rig = ViewRig::new().await;
    let (_source_control, _viewer_control) = rig.running().await;
    rig.handle.end();
    assert_eq!(rig.ended().await, End::Local);
    rig.handle.end();
    assert_eq!(
        rig.speech
            .all()
            .iter()
            .filter(|spoken| **spoken == Spoken::Ended)
            .count(),
        1
    );
    // The session is forgotten, so its packets are stopped before they are read.
    let late = rig.partial(0, PacketFlags::FIRST, b"zero").await;
    assert_eq!(stopped_with(&late).await, 499);
    assert_eq!(rig.speech.played(), []);
    assert!(!rig.handle.hear(true));
}

// Two nodes.

/// A viewer that plays sound, with an application that accepts what it is offered, and a paired
/// source that was set up to send it.
struct NodeRig {
    viewer: TestNode,
    source: TestNode,
    app: Arc<App>,
}

impl NodeRig {
    async fn new() -> Self {
        Self::of(true, true).await
    }

    /// The same, with a source that was set up to send sound only if `sends`, and a viewer that
    /// was set up to play it only if `plays`.
    async fn of(sends: bool, plays: bool) -> Self {
        Self::with_control(sends, plays, false).await
    }

    /// The same as [`new`](Self::new), with a source that can also take input, so that it can
    /// share a screen with control.
    async fn controlling() -> Self {
        Self::with_control(true, true, true).await
    }

    /// The same as [`of`](Self::of), with a source that can take input only if `control`.
    async fn with_control(sends: bool, plays: bool, control: bool) -> Self {
        let app = App::new(Plan::Accept);
        let viewer = TestNode::with(|builder| {
            let builder = builder.window_handler(Arc::clone(&app));
            if plays { builder.play_sound() } else { builder }
        })
        .await;
        let source = TestNode::with(|builder| {
            let builder = builder.share_windows(control);
            if sends {
                builder.share_sound()
            } else {
                builder
            }
        })
        .await;
        source.pair_with(&viewer);
        // The person at the source lets the viewer hear, which the tests that follow assume. The
        // one about the setting begins from what a device that was just paired has.
        source
            .trust
            .update(&viewer.id, |device| device.policy.hear = Some(true))
            .unwrap();
        Self {
            viewer,
            source,
            app,
        }
    }

    /// What the person at the source says of the viewer from now on.
    fn say(&self, says: impl FnOnce(&mut DevicePolicy)) {
        self.source
            .trust
            .update(&self.viewer.id, |device| says(&mut device.policy))
            .unwrap();
    }

    /// Offers `window`, and gives the session that a yes starts and what its platform hears.
    async fn share(&self, window: WindowShare) -> (SourceHandle, Heard) {
        let heard = Heard::default();
        let outcome = self
            .source
            .node
            .share(&self.viewer.id, window, Platform(heard.clone()), fit)
            .await;
        match outcome.unwrap() {
            ShareOutcome::Started(handle) => (handle, heard),
            other => panic!("the window was not shared: {other:?}"),
        }
    }
}

fn display() -> WindowShare {
    WindowShare::new("", "", 3024, 1964).whole_display()
}

#[tokio::test]
async fn a_node_advertises_sound_only_when_set_up_to() {
    let plain = TestNode::with(|builder| builder.share_windows(false)).await;
    let sounding = TestNode::with(|builder| builder.share_windows(false).share_sound()).await;
    let app = App::new(Plan::Refuse);
    let hearing =
        TestNode::with(|builder| builder.window_handler(Arc::clone(&app)).play_sound()).await;
    let deaf = TestNode::with(|builder| builder.window_handler(Arc::clone(&app))).await;
    let probe = TestNode::start().await;
    for other in [&plain, &sounding, &hearing, &deaf] {
        probe.pair_with(other);
    }
    let says = |peer: &TestNode, capability| {
        let probe = &probe;
        let id = peer.id;
        async move {
            probe
                .node
                .pooled_session(&id)
                .await
                .unwrap()
                .peer
                .supports(capability)
        }
    };
    assert!(says(&sounding, capability::STREAM_SOUND).await);
    assert!(!says(&sounding, capability::STREAM_HEAR).await);
    assert!(says(&hearing, capability::STREAM_HEAR).await);
    assert!(!says(&hearing, capability::STREAM_SOUND).await);
    for peer in [&plain, &deaf] {
        assert!(!says(peer, capability::STREAM_SOUND).await);
        assert!(!says(peer, capability::STREAM_HEAR).await);
    }
}

#[tokio::test]
async fn sound_needs_what_it_is_for() {
    async fn bound(build: impl FnOnce(NodeBuilder) -> NodeBuilder) -> crate::Result<crate::Node> {
        let identity = Identity::generate();
        let dir = tempfile::tempdir().unwrap();
        let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
        let mut config = Config::default();
        config.network.relay = RelayMode::Disabled;
        config.network.lan_discovery = false;
        config.network.dns_discovery = false;
        config.network.port_mapping = false;
        build(NodeBuilder::new(&identity, trust, config))
            .customize_endpoint(|builder| {
                builder
                    .clear_ip_transports()
                    .bind_addr("127.0.0.1:0")
                    .expect("loopback is a valid address")
            })
            .bind()
            .await
    }
    // Sending sound needs windows to share, and playing it needs windows to show.
    assert!(matches!(
        bound(NodeBuilder::share_sound).await,
        Err(Error::Config(_))
    ));
    assert!(matches!(
        bound(NodeBuilder::play_sound).await,
        Err(Error::Config(_))
    ));
    assert!(
        bound(|builder| builder.share_windows(false).share_sound())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn sound_goes_from_the_platform_of_the_source_to_the_speaker_of_the_viewer() {
    let rig = NodeRig::new().await;
    let (handle, heard) = rig.share(display()).await;
    let viewing = rig.app.handle(0);
    assert!(viewing.can_hear());
    assert!(viewing.hear(true));
    until("the platform to be asked for sound", || {
        heard.count("hear") == 1
    })
    .await;

    handle.begin_sound(aac()).unwrap();
    until("the speaker to be told the form", || {
        rig.app.speech.all() == [Spoken::Sound(aac())]
    })
    .await;
    for number in 0..3_u8 {
        let sent = handle
            .sound(packet(number, u64::from(number) * 21_333))
            .unwrap();
        assert_eq!(sent, Some(u64::from(number)));
    }
    until("three packets to be played", || {
        rig.app.speech.played().len() == 3
    })
    .await;
    assert_eq!(
        rig.app
            .speech
            .played()
            .iter()
            .map(|(number, first, lost, packet)| (*number, *first, *lost, packet[0]))
            .collect::<Vec<_>>(),
        [(0, true, 0, 0), (1, false, 0, 1), (2, false, 0, 2)]
    );
    // The source was told that each stream was acknowledged, and that none was reset or stopped.
    until("the streams to stop counting", || {
        handle.stats().sound_streams == 0
    })
    .await;
    assert_eq!(
        handle.stats().sound,
        SoundStats {
            sent: 3,
            acked: 3,
            ..SoundStats::default()
        }
    );

    // The person turns it off, and the platform is told to let the sound of the screen play.
    assert!(viewing.hear(false));
    until("the platform to be told", || heard.count("hush") == 1).await;
    assert_eq!(handle.sound(packet(9, 0)).unwrap(), None);
    assert_eq!(rig.app.speech.all().last(), Some(&Spoken::Stopped(None)));

    // The person closes the screen, and the speaker is let go of.
    drop(viewing);
    assert_eq!(
        tokio::time::timeout(PATIENCE, handle.ended())
            .await
            .expect("the source did not hear that the session ended"),
        End::Peer
    );
    until("the speaker to be let go of", || {
        rig.app.speech.all().last() == Some(&Spoken::Ended)
    })
    .await;
}

#[tokio::test]
async fn what_the_person_at_the_source_says_of_the_viewer_counts_at_its_next_request_to_hear() {
    let rig = NodeRig::new().await;
    // A device that was just paired has been given nothing, so in a session in which it can only
    // look it may not hear.
    rig.say(|policy| *policy = DevicePolicy::default());
    let (handle, heard) = rig.share(display()).await;
    let viewing = rig.app.handle(0);
    assert!(!viewing.control());
    assert!(viewing.hear(true));
    until("the speaker to be told why", || {
        rig.app.speech.all() == [Spoken::Stopped(Some(SilentReason::NotAllowed))]
    })
    .await;
    assert_eq!(heard.count("hear"), 0);

    // What it may ask for in general is not what lets it hear in this session, where it can only
    // look: it is told the same again.
    rig.say(|policy| policy.start = StartPolicy::Control);
    assert!(viewing.hear(true));
    until("the speaker to be told why again", || {
        let told = || Spoken::Stopped(Some(SilentReason::NotAllowed));
        rig.app.speech.all() == [told(), told()]
    })
    .await;
    assert_eq!(heard.count("hear"), 0);

    // The person says yes. It asks again, in the session that runs, and the platform is asked for
    // the sound.
    rig.say(|policy| policy.hear = Some(true));
    assert!(viewing.hear(true));
    until("the platform to be asked for sound", || {
        heard.count("hear") == 1
    })
    .await;
    handle.begin_sound(aac()).unwrap();
    until("the speaker to be told the form", || {
        rig.app.speech.all().last() == Some(&Spoken::Sound(aac()))
    })
    .await;

    // They take the yes back. The next request ends the run, and the platform is told, so that
    // the device that shares plays its own sound again.
    rig.say(|policy| policy.hear = Some(false));
    assert!(viewing.hear(true));
    until("the platform to be told to let the sound play", || {
        heard.count("hush") == 1
    })
    .await;
    until("the speaker to be told why", || {
        rig.app.speech.all().last() == Some(&Spoken::Stopped(Some(SilentReason::NotAllowed)))
    })
    .await;
    assert_eq!(handle.sound(packet(1, 0)).unwrap(), None);
}

#[tokio::test]
async fn a_device_hears_in_a_session_that_lets_it_control_the_screen_when_nothing_is_said_whatever_it_may_ask_for()
 {
    let rig = NodeRig::controlling().await;
    // A device that was just paired may ask for nothing, and nothing is said of its hearing. The
    // person here shares the whole screen with control, and the device did not ask for it.
    rig.say(|policy| *policy = DevicePolicy::default());
    let (handle, heard) = rig.share(display().with_control()).await;
    let viewing = rig.app.handle(0);
    assert!(viewing.control());
    assert!(viewing.hear(true));
    until("the platform to be asked for sound", || {
        heard.count("hear") == 1
    })
    .await;
    handle.begin_sound(aac()).unwrap();
    until("the speaker to be told the form", || {
        rig.app.speech.all().last() == Some(&Spoken::Sound(aac()))
    })
    .await;
    assert!(
        !rig.app
            .speech
            .all()
            .iter()
            .any(|spoken| matches!(spoken, Spoken::Stopped(Some(SilentReason::NotAllowed))))
    );

    // What the person said of the device counts all the same: the next request ends the run.
    rig.say(|policy| policy.hear = Some(false));
    assert!(viewing.hear(true));
    until("the platform to be told to let the sound play", || {
        heard.count("hush") == 1
    })
    .await;
    until("the speaker to be told why", || {
        rig.app.speech.all().last() == Some(&Spoken::Stopped(Some(SilentReason::NotAllowed)))
    })
    .await;
}

#[tokio::test]
async fn a_device_that_may_ask_for_control_does_not_hear_in_a_session_that_lets_it_only_look() {
    let rig = NodeRig::controlling().await;
    // The device may ask to control the screen, and nothing is said of its hearing. The person
    // here shares the screen for looking only.
    rig.say(|policy| {
        *policy = DevicePolicy {
            start: StartPolicy::Control,
            ..DevicePolicy::default()
        };
    });
    let (_handle, heard) = rig.share(display()).await;
    let viewing = rig.app.handle(0);
    assert!(!viewing.control());
    assert!(viewing.hear(true));
    until("the speaker to be told why", || {
        rig.app.speech.all() == [Spoken::Stopped(Some(SilentReason::NotAllowed))]
    })
    .await;
    tokio::time::sleep(QUIET).await;
    assert_eq!(heard.count("hear"), 0);
}

#[tokio::test]
async fn a_session_of_a_window_has_no_sound_and_the_person_is_told_why() {
    let rig = NodeRig::new().await;
    let (_handle, heard) = rig
        .share(WindowShare::new("Report.pdf", "Preview", 1440, 900))
        .await;
    let viewing = rig.app.handle(0);
    assert!(viewing.can_hear());
    assert!(viewing.hear(true));
    until("the speaker to be told why", || {
        rig.app.speech.all() == [Spoken::Stopped(Some(SilentReason::Window))]
    })
    .await;
    assert_eq!(heard.count("hear"), 0);
}

/// The viewer asks for sound as a viewer that broke the rules would: the source ends the session,
/// and its platform is never asked.
async fn asks_in_breach(viewing: &ViewHandle, handle: &SourceHandle, heard: &Heard) {
    viewing.say_anyway(ViewerMessage::Hear(Hear { on: true }));
    let ended = tokio::time::timeout(PATIENCE, handle.ended())
        .await
        .expect("the source did not end the session");
    assert_eq!(ended, End::Violation);
    assert_eq!(heard.count("hear"), 0);
}

#[tokio::test]
async fn a_viewer_cannot_ask_for_sound_from_a_source_that_was_not_set_up_to_send_it() {
    let rig = NodeRig::of(false, true).await;
    let (handle, heard) = rig.share(display()).await;
    let viewing = rig.app.handle(0);
    assert!(!viewing.can_hear());
    assert!(!viewing.hear(true));
    // A viewer that asks all the same breaks a rule, and a Mac that was not set up to send sound
    // is not made quiet or asked to capture.
    asks_in_breach(&viewing, &handle, &heard).await;
}

#[tokio::test]
async fn a_viewer_that_was_not_set_up_to_play_sound_has_nothing_to_ask_with() {
    let rig = NodeRig::of(true, false).await;
    let (handle, heard) = rig.share(display()).await;
    // The application gave the session a speaker, but the device does not play sound.
    let viewing = rig.app.handle(0);
    assert!(!viewing.can_hear());
    assert!(!viewing.hear(true));
    // A viewer that asks all the same breaks a rule, and a Mac is not made quiet for a device that
    // plays nothing.
    asks_in_breach(&viewing, &handle, &heard).await;
}
