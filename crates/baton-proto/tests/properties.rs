// SPDX-License-Identifier: Apache-2.0

//! Property tests: whatever the network sends, decoding returns a value or an
//! error and never panics, and everything we can encode we can decode again.

use baton_proto::{
    DeviceId, DeviceInfo, ItemId, Platform, frame,
    message::{
        ClipboardText, ErrorCode, ErrorReply, Handoff, HandoffReply, Hello, Item, Link, Outcome,
        Ping, Request, Response,
    },
    pair::{PairProof, PairRejection, PairRequest, PairResponse, PairSecret},
    ticket::PairTicket,
};
use proptest::{prelude::*, test_runner::FileFailurePersistence};

fn text(max_chars: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..=max_chars).prop_map(|c| c.into_iter().collect())
}

fn non_empty_text(max_chars: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 1..=max_chars).prop_map(|c| c.into_iter().collect())
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
            proptest::collection::vec(non_empty_text(20), 0..8)
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
    ]
}

fn response() -> impl Strategy<Value = Response> {
    prop_oneof![
        (
            any::<u16>(),
            device_info(),
            proptest::collection::vec(non_empty_text(20), 0..8)
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
    ]
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
        let _ = frame::decode::<Request>(&bytes);
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
}
