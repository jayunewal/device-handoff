// SPDX-License-Identifier: Apache-2.0

//! Property tests: whatever the network sends, decoding returns a value or an
//! error and never panics, and everything we can encode we can decode again.

// Tests may unwrap freely. Clippy exempts #[test] functions and #[cfg(test)] modules, but not
// the helper functions of an integration test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{cmp::Ordering, collections::BTreeMap};

use baton_proto::{
    DeviceId, DeviceInfo, ItemId, Platform, SessionId, TransferId, frame,
    limits::{
        MAX_CAPABILITY_BYTES, MAX_CODECS, MAX_FAILURES_LISTED, MAX_FILE_BYTES, MAX_FRAME_RATE,
        MAX_LANES, MAX_LIST_FRAME_ENTRIES, MAX_PICTURE_EDGE, MAX_PREVIEW_ITEMS, MAX_WINDOW_EDGE,
    },
    message::{
        ClipboardText, DeclineReason, ErrorCode, ErrorReply, Handoff, HandoffReply, Hello, Item,
        Link, Outcome, Ping, Request, Response,
    },
    pair::{PairProof, PairRejection, PairRequest, PairResponse, PairSecret},
    stream::{self, Codec, SourceKind, StopReason, StreamReply, StreamRequest},
    ticket::PairTicket,
    transfer::{
        Accepted, Cancel, Cancelled, Declined, Done, FailReason, Failure, Finish, Offer,
        PreviewItem, Summary, TransferReply, TransferRequest,
        hash::{FileHasher, SPAN_LEN},
        lane::{FrameHead, LaneHeader, SkipReason},
        list::{ListChecker, ListChunk, ListEntry, ListError, Listing, hash_list},
        path,
    },
};
use proptest::{prelude::*, test_runner::FileFailurePersistence};

fn text(max_chars: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..=max_chars).prop_map(|c| c.into_iter().collect())
}

fn non_empty_text(max_chars: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 1..=max_chars).prop_map(|c| c.into_iter().collect())
}

/// A capability name that a `Hello` accepts, whatever its characters are. A character takes up to
/// 4 bytes, so this many of them can never come to more than `MAX_CAPABILITY_BYTES`. (Twenty
/// characters could, which made these tests fail on about one run in a hundred.)
const CAPABILITY_CHARS: usize = MAX_CAPABILITY_BYTES / 4;

fn capability() -> impl Strategy<Value = String> {
    non_empty_text(CAPABILITY_CHARS)
}

fn device_id() -> impl Strategy<Value = DeviceId> {
    any::<[u8; 32]>().prop_map(DeviceId::from_bytes)
}

fn platform() -> impl Strategy<Value = Platform> {
    prop_oneof![
        Just(Platform::MacOs),
        Just(Platform::Windows),
        Just(Platform::Linux),
        Just(Platform::Ios),
        Just(Platform::IpadOs),
        Just(Platform::Android),
        Just(Platform::Other),
    ]
}

fn device_info() -> impl Strategy<Value = DeviceInfo> {
    (non_empty_text(40), platform(), text(10)).prop_map(|(name, platform, app_version)| {
        DeviceInfo {
            name,
            platform,
            app_version,
        }
    })
}

fn item() -> impl Strategy<Value = Item> {
    prop_oneof![
        (non_empty_text(200), proptest::option::of(text(100)))
            .prop_map(|(url, title)| Item::Link(Link { url, title })),
        non_empty_text(2000).prop_map(|text| Item::ClipboardText(ClipboardText { text })),
    ]
}

fn request() -> impl Strategy<Value = Request> {
    prop_oneof![
        (
            any::<u16>(),
            device_info(),
            proptest::collection::vec(capability(), 0..8)
        )
            .prop_map(
                |(protocol_minor, device, capabilities)| Request::Hello(Hello {
                    protocol_minor,
                    device,
                    capabilities
                })
            ),
        any::<u64>().prop_map(|nonce| Request::Ping(Ping { nonce })),
        (any::<[u8; 16]>(), item()).prop_map(|(id, item)| Request::Handoff(Handoff {
            id: ItemId::from_bytes(id),
            item
        })),
        transfer_request().prop_map(Request::Transfer),
        stream_request().prop_map(Request::Stream),
    ]
}

fn response() -> impl Strategy<Value = Response> {
    prop_oneof![
        (
            any::<u16>(),
            device_info(),
            proptest::collection::vec(capability(), 0..8)
        )
            .prop_map(
                |(protocol_minor, device, capabilities)| Response::Hello(Hello {
                    protocol_minor,
                    device,
                    capabilities
                })
            ),
        any::<u64>().prop_map(|nonce| Response::Pong(Ping { nonce })),
        any::<[u8; 16]>().prop_map(|id| Response::Handoff(HandoffReply {
            id: ItemId::from_bytes(id),
            outcome: Outcome::Accepted
        })),
        text(100).prop_map(|message| Response::Error(ErrorReply {
            code: ErrorCode::Malformed,
            message
        })),
        transfer_reply().prop_map(Response::Transfer),
        stream_reply().prop_map(Response::Stream),
    ]
}

fn transfer_id() -> impl Strategy<Value = TransferId> {
    any::<[u8; 16]>().prop_map(TransferId::from_bytes)
}

fn session_id() -> impl Strategy<Value = SessionId> {
    any::<[u8; 16]>().prop_map(SessionId::from_bytes)
}

fn codec() -> impl Strategy<Value = Codec> {
    prop_oneof![Just(Codec::H264), Just(Codec::Other)]
}

fn stream_request() -> impl Strategy<Value = StreamRequest> {
    prop_oneof![
        (
            session_id(),
            prop_oneof![Just(SourceKind::Window), Just(SourceKind::Other)],
            // A title may be 512 bytes and an application name 256, and a character is up to four.
            text(120),
            text(60),
            1..=MAX_WINDOW_EDGE,
            1..=MAX_WINDOW_EDGE,
            any::<bool>()
        )
            .prop_map(|(session, source, title, app, width, height, control)| {
                StreamRequest::Offer(stream::Offer {
                    session,
                    source,
                    title,
                    app,
                    width,
                    height,
                    control,
                })
            }),
        (
            session_id(),
            prop_oneof![
                Just(StopReason::Ended),
                Just(StopReason::WindowClosed),
                Just(StopReason::Failed),
                Just(StopReason::Other)
            ]
        )
            .prop_map(|(session, reason)| StreamRequest::Stop(stream::Stop { session, reason })),
    ]
}

fn stream_reply() -> impl Strategy<Value = StreamReply> {
    prop_oneof![
        (
            session_id(),
            1..=MAX_PICTURE_EDGE,
            1..=MAX_PICTURE_EDGE,
            1..=MAX_FRAME_RATE,
            proptest::collection::vec(codec(), 1..=MAX_CODECS)
        )
            .prop_map(|(session, max_width, max_height, max_fps, codecs)| {
                StreamReply::Accepted(stream::Accepted {
                    session,
                    max_width,
                    max_height,
                    max_fps,
                    codecs,
                })
            }),
        (session_id(), decline_reason()).prop_map(|(session, reason)| StreamReply::Declined(
            stream::Declined { session, reason }
        )),
        session_id().prop_map(|session| StreamReply::Stopped(stream::Stopped { session })),
    ]
}

/// What a transfer holds, as the sender found it.
#[derive(Debug, Clone)]
enum Node {
    File { size: u64, modified_ms: Option<i64> },
    Folder(BTreeMap<String, Node>),
}

/// Names that are prefixes of each other, and that sort below `/` (a space, `-` and `.`) or
/// above it, which is what makes the order of a list worth testing.
const NAMES: [&str; 12] = [
    "a", "a-b", "a b", "a.b", "a0", "ab", "b", "B", "e", "é", "日本", "x",
];

fn name() -> impl Strategy<Value = String> {
    prop::sample::select(NAMES.to_vec()).prop_map(str::to_owned)
}

fn node() -> impl Strategy<Value = Node> {
    let file = (
        0..5_000_000_u64,
        proptest::option::of(0..4_000_000_000_000_i64),
    )
        .prop_map(|(size, modified_ms)| Node::File { size, modified_ms });
    file.prop_recursive(3, 40, 4, |inner| {
        proptest::collection::btree_map(name(), inner, 0..4).prop_map(Node::Folder)
    })
}

/// Entries in the order of a list: a walk of the tree, folders first, siblings by name.
fn flatten(prefix: &str, children: &BTreeMap<String, Node>, out: &mut Vec<ListEntry>) {
    for (name, node) in children {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        match node {
            Node::File { size, modified_ms } => out.push(ListEntry {
                path,
                folder: false,
                size: *size,
                modified_ms: *modified_ms,
            }),
            Node::Folder(inside) => {
                out.push(ListEntry {
                    path: path.clone(),
                    folder: true,
                    size: 0,
                    modified_ms: None,
                });
                flatten(&path, inside, out);
            }
        }
    }
}

/// A list of one to some dozens of entries, in the order the protocol gives.
fn tree() -> impl Strategy<Value = Vec<ListEntry>> {
    proptest::collection::btree_map(name(), node(), 1..8).prop_map(|top| {
        let mut out = Vec::new();
        flatten("", &top, &mut out);
        out
    })
}

/// The offer that a sender makes for a list.
fn offer_for(id: TransferId, entries: &[ListEntry]) -> Offer {
    let top: Vec<&ListEntry> = entries.iter().filter(|e| !e.path.contains('/')).collect();
    let preview = top
        .iter()
        .take(MAX_PREVIEW_ITEMS)
        .map(|item| {
            let prefix = format!("{}/", item.path);
            let inside = || {
                entries
                    .iter()
                    .filter(|e| !e.folder && e.path.starts_with(&prefix))
            };
            PreviewItem {
                name: item.path.clone(),
                folder: item.folder,
                files: if item.folder {
                    u32::try_from(inside().count()).unwrap()
                } else {
                    0
                },
                bytes: if item.folder {
                    inside().map(|e| e.size).sum()
                } else {
                    item.size
                },
            }
        })
        .collect();
    Offer {
        id,
        summary: Summary {
            items: u32::try_from(top.len()).unwrap(),
            files: u32::try_from(entries.iter().filter(|e| !e.folder).count()).unwrap(),
            folders: u32::try_from(entries.iter().filter(|e| e.folder).count()).unwrap(),
            bytes: entries.iter().map(|e| e.size).sum(),
        },
        preview,
        list_hash: hash_list(entries),
    }
}

/// Carries `entries` to a checker for `offer` as a receiver would get them: in frames of
/// `piece` entries each, over the wire.
fn check(entries: &[ListEntry], offer: &Offer, piece: usize) -> Result<Listing, String> {
    let mut checker = ListChecker::new(offer).map_err(|e| e.to_string())?;
    for chunk in entries.chunks(piece) {
        let chunk = ListChunk {
            entries: chunk.to_vec(),
        };
        let frame = frame::encode(&chunk).map_err(|e| e.to_string())?;
        let chunk = frame::decode::<ListChunk>(&frame).map_err(|e| e.to_string())?;
        checker.push(chunk).map_err(|e: ListError| e.to_string())?;
    }
    checker.finish().map_err(|e| e.to_string())
}

fn transfer_request() -> impl Strategy<Value = TransferRequest> {
    prop_oneof![
        (transfer_id(), tree())
            .prop_map(|(id, entries)| TransferRequest::Offer(offer_for(id, &entries))),
        (
            transfer_id(),
            1..=baton_proto::limits::MAX_TRANSFER_FILES,
            any::<u64>()
        )
            .prop_map(|(id, files, bytes)| TransferRequest::Finish(Finish {
                id,
                files,
                bytes: bytes % (MAX_FILE_BYTES + 1)
            })),
        transfer_id().prop_map(|id| TransferRequest::Cancel(Cancel { id })),
    ]
}

fn decline_reason() -> impl Strategy<Value = DeclineReason> {
    prop_oneof![
        Just(DeclineReason::Unsupported),
        Just(DeclineReason::TooLarge),
        Just(DeclineReason::RateLimited),
        Just(DeclineReason::Blocked),
        Just(DeclineReason::Invalid),
        Just(DeclineReason::NoSpace),
        Just(DeclineReason::Refused),
        Just(DeclineReason::Other),
    ]
}

fn fail_reason() -> impl Strategy<Value = FailReason> {
    prop_oneof![
        Just(FailReason::Incomplete),
        Just(FailReason::Mismatch),
        Just(FailReason::NoSpace),
        Just(FailReason::WriteFailed),
        Just(FailReason::Skipped),
        Just(FailReason::Other),
    ]
}

fn failures(failed: impl Iterator<Item = (u32, FailReason)>) -> Vec<Failure> {
    failed
        .map(|(file, reason)| Failure { file, reason })
        .collect()
}

fn transfer_reply() -> impl Strategy<Value = TransferReply> {
    prop_oneof![
        (transfer_id(), 1..=MAX_LANES)
            .prop_map(|(id, lanes)| TransferReply::Accepted(Accepted { id, lanes })),
        (transfer_id(), decline_reason())
            .prop_map(|(id, reason)| TransferReply::Declined(Declined { id, reason })),
        // A `Done` names the files that failed in the order of the list, which is what a map
        // gives, and counts more of them only when it names as many as it may.
        (
            transfer_id(),
            proptest::collection::btree_map(any::<u32>(), fail_reason(), 0..6)
        )
            .prop_map(|(id, failed)| TransferReply::Done(Done {
                id,
                failed: failures(failed.into_iter()),
                more_failed: 0
            })),
        (
            transfer_id(),
            0..1_000_000_u32,
            proptest::collection::vec(fail_reason(), MAX_FAILURES_LISTED),
            1..=100_000_u32
        )
            .prop_map(
                |(id, first, reasons, more_failed)| TransferReply::Done(Done {
                    id,
                    failed: failures((first..).zip(reasons)),
                    more_failed
                })
            ),
        transfer_id().prop_map(|id| TransferReply::Cancelled(Cancelled { id })),
    ]
}

fn frame_head() -> impl Strategy<Value = FrameHead> {
    prop_oneof![
        Just(FrameHead::List),
        (any::<u32>(), any::<u32>()).prop_map(|(file, span)| FrameHead::Span { file, span }),
        (any::<u32>(), any::<[u8; 32]>()).prop_map(|(file, hash)| FrameHead::End {
            file,
            hash: blake3::Hash::from_bytes(hash)
        }),
        (
            any::<u32>(),
            prop_oneof![
                Just(SkipReason::Unreadable),
                Just(SkipReason::Changed),
                Just(SkipReason::Missing),
                Just(SkipReason::Other)
            ]
        )
            .prop_map(|(file, reason)| FrameHead::Skip { file, reason }),
    ]
}

/// Bytes that differ from place to place, so that a span hashed at the wrong place gives
/// another hash.
fn content(len: u64, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[3]
        })
        .collect()
}

/// The numbers `0..count` in an order that depends on `seed`.
fn shuffled(count: u32, seed: u64) -> Vec<u32> {
    let mut order: Vec<u32> = (0..count).collect();
    let mut state = seed | 1;
    for i in (1..order.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = usize::try_from(state % (u64::try_from(i).unwrap() + 1)).unwrap();
        order.swap(i, j);
    }
    order
}

fn ticket() -> impl Strategy<Value = PairTicket> {
    (
        device_id(),
        any::<[u8; 32]>(),
        any::<u64>(),
        proptest::option::of("(http|https)://[a-z0-9.-]{1,40}(:[0-9]{1,5})?(/[a-z0-9._~-]{0,20})?"),
        // The ticket carries an IP and a port only, not IPv6 flow info or scope ids.
        proptest::collection::vec(
            any::<std::net::SocketAddr>().prop_map(|a| std::net::SocketAddr::new(a.ip(), a.port())),
            0..=16,
        ),
    )
        .prop_map(|(offerer, secret, expires_at, relay, addrs)| PairTicket {
            offerer,
            secret: PairSecret::from_bytes(secret),
            expires_at,
            relay,
            addrs,
        })
}

/// The widest capability that the generator makes, to the byte: the longest one a `Hello` allows.
#[test]
fn the_widest_capability_the_generator_makes_is_accepted() {
    let name = "\u{1F600}".repeat(CAPABILITY_CHARS);
    assert!(name.len() <= MAX_CAPABILITY_BYTES, "{}", name.len());
    let request = Request::Hello(Hello {
        protocol_minor: 0,
        device: DeviceInfo {
            name: "phone".into(),
            platform: Platform::Ios,
            app_version: "1".into(),
        },
        capabilities: vec![name],
    });
    let frame = frame::encode(&request).unwrap();
    assert_eq!(frame::decode::<Request>(&frame).unwrap(), request);
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource("regressions"))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn requests_round_trip(request in request()) {
        let frame = frame::encode(&request).unwrap();
        prop_assert_eq!(frame::decode::<Request>(&frame).unwrap(), request);
    }

    #[test]
    fn responses_round_trip(response in response()) {
        let frame = frame::encode(&response).unwrap();
        prop_assert_eq!(frame::decode::<Response>(&frame).unwrap(), response);
    }

    #[test]
    fn pair_messages_round_trip(
        device in device_info(),
        proof_key in any::<[u8; 32]>(),
        binding in any::<[u8; 32]>(),
        ids in (device_id(), device_id()),
    ) {
        let proof = PairProof::compute(&PairSecret::from_bytes(proof_key), &ids.0, &ids.1, &binding);
        let request = PairRequest { device: device.clone(), proof };
        let decoded = frame::decode::<PairRequest>(&frame::encode(&request).unwrap()).unwrap();
        prop_assert_eq!(decoded.device, device.clone());
        prop_assert_eq!(decoded.proof.as_bytes(), proof.as_bytes());

        for response in [
            PairResponse::Accepted { device },
            PairResponse::Rejected(PairRejection::Invalid),
            PairResponse::Rejected(PairRejection::Declined),
            PairResponse::Rejected(PairRejection::Busy),
        ] {
            let decoded = frame::decode::<PairResponse>(&frame::encode(&response).unwrap()).unwrap();
            prop_assert_eq!(decoded, response);
        }
    }

    #[test]
    fn tickets_round_trip(ticket in ticket()) {
        let parsed: PairTicket = ticket.to_text().unwrap().parse().unwrap();
        prop_assert_eq!(parsed.offerer, ticket.offerer);
        prop_assert_eq!(parsed.secret.expose(), ticket.secret.expose());
        prop_assert_eq!(parsed.expires_at, ticket.expires_at);
        prop_assert_eq!(parsed.relay, ticket.relay);
        prop_assert_eq!(parsed.addrs, ticket.addrs);
    }

    #[test]
    fn optional_extensions_never_change_what_a_ticket_says(
        ticket in ticket(),
        extensions in proptest::collection::vec(
            // Types below 0x80 are optional; the length byte is the vector's length.
            (0u8..0x80, proptest::collection::vec(any::<u8>(), 0..=255)),
            0..=16,
        ),
        flags in 0u8..=0x0f,
    ) {
        let mut body = ticket.to_bytes().unwrap().to_vec();
        body.truncate(body.len() - 4);
        body[1] = flags;
        for (kind, value) in &extensions {
            body.push(*kind);
            body.push(u8::try_from(value.len()).unwrap());
            body.extend_from_slice(value);
        }
        let sum = blake3::derive_key("baton 2026-10-14 pairing ticket checksum v1", &body);
        body.extend_from_slice(&sum[..4]);

        let parsed = PairTicket::from_bytes(&body).unwrap();
        prop_assert_eq!(parsed.offerer, ticket.offerer);
        prop_assert_eq!(parsed.secret.expose(), ticket.secret.expose());
        prop_assert_eq!(parsed.expires_at, ticket.expires_at);
        prop_assert_eq!(parsed.relay, ticket.relay);
        prop_assert_eq!(parsed.addrs, ticket.addrs);
    }

    #[test]
    fn arbitrary_bytes_never_panic_the_decoders(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = frame::decode_payload::<Request>(&bytes);
        let _ = frame::decode_payload::<Response>(&bytes);
        let _ = frame::decode_payload::<PairRequest>(&bytes);
        let _ = frame::decode_payload::<PairResponse>(&bytes);
        let _ = frame::decode_payload::<ListChunk>(&bytes);
        let _ = frame::decode::<Request>(&bytes);
        let _ = frame::decode::<ListChunk>(&bytes);
        let _ = PairTicket::from_bytes(&bytes);
    }

    #[test]
    fn arbitrary_text_never_panics_the_ticket_parser(text in ".{0,3000}") {
        let _ = text.parse::<PairTicket>();
        let _ = text.parse::<DeviceId>();
    }

    #[test]
    fn corrupting_a_valid_frame_never_panics(
        request in request(),
        flip_at in any::<prop::sample::Index>(),
        flip_to in any::<u8>(),
        cut in any::<prop::sample::Index>(),
    ) {
        let mut frame = frame::encode(&request).unwrap();
        let i = flip_at.index(frame.len());
        frame[i] = flip_to;
        let _ = frame::decode::<Request>(&frame);
        frame.truncate(cut.index(frame.len() + 1));
        let _ = frame::decode::<Request>(&frame);
    }

    #[test]
    fn sanitized_names_are_always_safe_to_show(raw in ".{0,400}") {
        let clean = baton_proto::device::sanitize_display_name(&raw);
        prop_assert!(!clean.is_empty());
        prop_assert!(clean.chars().count() <= baton_proto::device::MAX_DISPLAY_NAME_CHARS);
        prop_assert!(!clean.chars().any(char::is_control));
        prop_assert!(!clean.contains("  "));
        prop_assert_eq!(clean.trim(), clean.as_str());
        // Idempotent: cleaning a clean name changes nothing.
        prop_assert_eq!(baton_proto::device::sanitize_display_name(&clean), clean);
    }

    #[test]
    fn a_list_that_matches_its_offer_is_accepted_however_it_is_cut_up(
        entries in tree(),
        id in transfer_id(),
        piece in 1..=MAX_LIST_FRAME_ENTRIES,
    ) {
        let offer = offer_for(id, &entries);
        // The offer travels too.
        let offer = match frame::decode::<Request>(&frame::encode(&Request::Transfer(TransferRequest::Offer(offer))).unwrap()).unwrap() {
            Request::Transfer(TransferRequest::Offer(offer)) => offer,
            other => panic!("not an offer: {other:?}"),
        };
        let listing = check(&entries, &offer, piece).map_err(TestCaseError::fail)?;
        prop_assert_eq!(listing.entries(), entries.as_slice());
    }

    #[test]
    fn a_list_that_is_not_what_was_offered_is_refused(
        entries in tree(),
        id in transfer_id(),
        at in any::<prop::sample::Index>(),
        how in 0..3_u8,
        piece in 1..=MAX_LIST_FRAME_ENTRIES,
    ) {
        let offer = offer_for(id, &entries);
        let mut changed = entries.clone();
        let i = at.index(changed.len());
        match how {
            // One entry missing, which no sum and no preview agrees with.
            0 => { changed.remove(i); }
            // One entry twice.
            1 => changed.insert(i, entries[i].clone()),
            // Two neighbours the wrong way round.
            _ => {
                prop_assume!(changed.len() > 1);
                let first = if i + 1 < changed.len() { i } else { i - 1 };
                changed.swap(first, first + 1);
            }
        }
        prop_assert!(check(&changed, &offer, piece).is_err());
    }

    #[test]
    fn a_list_that_agrees_in_every_count_but_not_in_a_time_is_refused_for_its_hash(
        entries in tree(),
        id in transfer_id(),
        at in any::<prop::sample::Index>(),
        time in any::<i64>(),
        piece in 1..=MAX_LIST_FRAME_ENTRIES,
    ) {
        // Another time on one file leaves every count, every size and the preview as they were,
        // so nothing but the hash can notice it.
        let files: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.folder)
            .map(|(i, _)| i)
            .collect();
        prop_assume!(!files.is_empty());
        let i = files[at.index(files.len())];
        prop_assume!(entries[i].modified_ms != Some(time));
        let offer = offer_for(id, &entries);
        let mut changed = entries.clone();
        changed[i].modified_ms = Some(time);
        prop_assert_eq!(
            check(&changed, &offer, piece).unwrap_err(),
            ListError::HashMismatch.to_string()
        );
    }

    #[test]
    fn two_lists_have_the_same_hash_only_if_they_are_the_same_list(
        a in tree(),
        b in tree(),
    ) {
        prop_assert_eq!(a == b, hash_list(&a) == hash_list(&b));
        // A list is not the same as itself with an entry less.
        if a.len() > 1 {
            prop_assert_ne!(hash_list(&a), hash_list(&a[..a.len() - 1]));
        }
    }

    #[test]
    fn a_tree_walk_is_in_the_order_of_paths(entries in tree()) {
        for pair in entries.windows(2) {
            prop_assert_eq!(path::compare(&pair[0].path, &pair[1].path), Ordering::Less);
        }
    }

    #[test]
    fn the_order_of_paths_is_a_total_order(
        a in "[a-b ./-]{0,10}",
        b in "[a-b ./-]{0,10}",
        c in "[a-b ./-]{0,10}",
    ) {
        prop_assert_eq!(path::compare(&a, &a), Ordering::Equal);
        prop_assert_eq!(path::compare(&a, &b), path::compare(&b, &a).reverse());
        if path::compare(&a, &b) != Ordering::Greater && path::compare(&b, &c) != Ordering::Greater {
            prop_assert_ne!(path::compare(&a, &c), Ordering::Greater);
        }
    }

    #[test]
    fn a_path_of_whole_names_is_valid_and_splits_into_its_folder_and_its_name(
        names in proptest::collection::vec(name(), 1..8),
    ) {
        let joined = names.join("/");
        prop_assert_eq!(path::check_path(&joined), Ok(names.len()));
        let (folder, last) = path::split_name(&joined);
        prop_assert_eq!(last, names.last().unwrap().as_str());
        prop_assert_eq!(folder, names[..names.len() - 1].join("/"));
        prop_assert_eq!(path::check_name(last), Ok(()));
    }

    #[test]
    fn arbitrary_text_never_panics_the_path_checks(text in ".{0,1200}") {
        if let Ok(depth) = path::check_path(&text) {
            // Whatever passes is whole names separated by `/`, and nothing else.
            prop_assert_eq!(depth, text.split('/').count());
            prop_assert!(text.split('/').all(|name| !name.is_empty() && name != "." && name != ".."));
            prop_assert!(!text.contains('\0'));
        }
        let _ = path::check_name(&text);
        let _ = path::split_name(&text);
    }

    #[test]
    fn frame_heads_round_trip(head in frame_head()) {
        let mut bytes = Vec::new();
        head.encode_into(&mut bytes);
        let (tag, rest) = bytes.split_first().unwrap();
        prop_assert_eq!(*tag, head.tag());
        prop_assert_eq!(FrameHead::rest_len(*tag), Ok(rest.len()));
        prop_assert!(rest.len() <= FrameHead::MAX_REST_LEN);
        prop_assert_eq!(FrameHead::decode(*tag, rest), Ok(head));
    }

    #[test]
    fn lane_headers_round_trip(transfer in transfer_id(), lane in any::<u8>()) {
        let header = LaneHeader { transfer, lane };
        prop_assert_eq!(LaneHeader::from_bytes(&header.to_bytes()), Ok(header));
    }

    #[test]
    fn arbitrary_bytes_never_panic_the_lane_decoders(
        tag in any::<u8>(),
        rest in proptest::collection::vec(any::<u8>(), 0..64),
        kind in prop_oneof![Just(1_u8), any::<u8>()],
        transfer in any::<[u8; 16]>(),
        lane in any::<u8>(),
    ) {
        let _ = FrameHead::rest_len(tag);
        let _ = FrameHead::decode(tag, &rest);

        let mut bytes = [0_u8; LaneHeader::LEN];
        bytes[0] = kind;
        bytes[1..17].copy_from_slice(&transfer);
        bytes[17] = lane;
        // A header that is read is written back the same.
        if let Ok(header) = LaneHeader::from_bytes(&bytes) {
            prop_assert_eq!(header.to_bytes(), bytes);
        }
    }
}

proptest! {
    // Each case hashes some megabytes, so there are fewer of them.
    #![proptest_config(ProptestConfig {
        cases: 12,
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource("regressions"))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn a_file_hashed_from_its_spans_in_any_order_has_the_ordinary_blake3_hash(
        size in prop_oneof![
            0..=4096_u64,
            (0..=4_u64).prop_map(|spans| spans * SPAN_LEN),
            (1..=3_u64, 1..=3_u64).prop_map(|(spans, extra)| spans * SPAN_LEN + extra),
            0..=(3 * SPAN_LEN + 5),
        ],
        seed in any::<u64>(),
    ) {
        let data = content(size, seed);
        let mut hasher = FileHasher::new(size).unwrap();
        for index in shuffled(hasher.span_count(), seed) {
            let start = usize::try_from(u64::from(index) * SPAN_LEN).unwrap();
            let end = usize::try_from((u64::from(index) + 1) * SPAN_LEN).unwrap().min(data.len());
            hasher.add_span(index, &data[start..end]).unwrap();
        }
        prop_assert!(hasher.is_complete());
        prop_assert_eq!(hasher.finish().unwrap(), blake3::hash(&data));
    }
}
