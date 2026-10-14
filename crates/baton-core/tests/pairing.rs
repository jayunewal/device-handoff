// SPDX-License-Identifier: Apache-2.0

//! Pairing between two devices that run in this process.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
    },
    time::Duration,
};

use baton_core::{ConfirmationCode, Error, PairApprovalRequest, error::OfferEnd};
use baton_proto::{
    ALPN_PAIR, DeviceId, frame,
    pair::{
        CHANNEL_BINDING_LEN, EXPORTER_LABEL, PairProof, PairRejection, PairRequest, PairResponse,
        PairSecret,
    },
    ticket::CLOCK_SKEW_ALLOWANCE_SECS,
};
use common::{Raw, Spec, approve, device, header, info, refuse};
use tokio::sync::{Notify, mpsc};

fn rejected(error: &Error) -> Option<PairRejection> {
    match error {
        Error::PairingRejected(why) => Some(*why),
        _ => None,
    }
}

#[tokio::test]
async fn two_devices_pair_and_trust_each_other() {
    let a = device("Laptop").await;
    let b = device("Phone").await;

    let offer = a.node.offer_pairing(approve).await.unwrap();
    assert!(offer.ticket_text().starts_with("BATON1:"));
    let joined = b.node.join_pairing(offer.ticket(), |_| {}).await.unwrap();
    let accepted = offer.wait().await.unwrap();

    assert_eq!((joined.id, joined.name.as_str()), (a.id, "Laptop"));
    assert_eq!((accepted.id, accepted.name.as_str()), (b.id, "Phone"));
    assert!(a.trust.contains(&b.id));
    assert!(b.trust.contains(&a.id));
    // Each side remembers how it reached the other, so it can find it again.
    assert!(!joined.addrs.is_empty(), "the joiner kept no address");
    assert!(!accepted.addrs.is_empty(), "the offerer kept no address");
}

#[tokio::test]
async fn the_person_is_shown_who_is_asking_as_the_transport_saw_it() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let offer = a
        .node
        .offer_pairing(move |request: PairApprovalRequest| {
            let tx = tx.clone();
            async move {
                tx.send((
                    request.peer,
                    request.device.name.clone(),
                    request.fingerprint(),
                ))
                .unwrap();
                true
            }
        })
        .await
        .unwrap();
    b.node.join_pairing(offer.ticket(), |_| {}).await.unwrap();

    let (peer, name, fingerprint) = rx.recv().await.unwrap();
    assert_eq!(peer, b.id);
    assert_eq!(name, "Phone");
    assert_eq!(fingerprint, b.id.fingerprint());
}

#[tokio::test]
async fn both_screens_show_the_same_code_and_a_stranger_could_not_have_it() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let codes = b.pair_with(&a).await;
    assert_eq!(codes.offerer, codes.joiner);
    // Six digits in two groups of three.
    let shown = codes.joiner.to_string();
    assert_eq!(shown.len(), 7, "{shown}");
    assert_eq!(shown.as_bytes()[3], b' ', "{shown}");
}

#[tokio::test]
async fn the_joiner_sees_its_code_while_the_person_is_still_deciding() {
    let a = device("Laptop").await;
    let b = device("Phone").await;

    // The person does not answer until the joiner's screen has the code. If the joiner
    // only showed it after the answer, this would wait forever.
    let shown = Arc::new(Notify::new());
    let offer = {
        let shown = Arc::clone(&shown);
        a.node
            .offer_pairing(move |_request: PairApprovalRequest| {
                let shown = Arc::clone(&shown);
                async move {
                    tokio::time::timeout(Duration::from_secs(4), shown.notified())
                        .await
                        .is_ok()
                }
            })
            .await
            .unwrap()
    };
    let code_was_shown = std::cell::Cell::new(false);
    b.node
        .join_pairing(offer.ticket(), |_code| {
            code_was_shown.set(true);
            shown.notify_one();
        })
        .await
        .expect("the person said yes once the code was on the joiner's screen");
    assert!(code_was_shown.get());
    offer.wait().await.unwrap();
}

#[tokio::test]
async fn a_joiner_that_cannot_connect_shows_no_code() {
    let b = device("Phone").await;
    let a = device("Laptop").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let mut ticket = offer.ticket().clone();
    ticket.addrs = vec!["127.0.0.1:9".parse().unwrap()];
    ticket.relay = None;
    drop(offer);
    // The wrong address and no relay: the offerer cannot be reached by this ticket
    // even though its key is right, so there is no session to take a code from.
    let mut shown = false;
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        b.node.join_pairing(&ticket, |_| shown = true),
    )
    .await;
    assert!(!matches!(result, Ok(Ok(_))));
    assert!(!shown, "a code was shown without a session");
}

#[tokio::test]
async fn a_declined_request_pairs_nobody() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let offer = a.node.offer_pairing(refuse).await.unwrap();

    let error = b
        .node
        .join_pairing(offer.ticket(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Declined), "{error}");
    let ended = offer.wait().await.unwrap_err();
    assert!(
        matches!(ended, Error::OfferEnded(OfferEnd::Declined)),
        "{ended}"
    );
    assert!(a.trust.is_empty());
    assert!(b.trust.is_empty());
}

#[tokio::test]
async fn a_person_who_never_answers_is_a_no() {
    let mut spec = Spec::new("Laptop");
    spec.tuning.approval_timeout = Duration::from_millis(300);
    let a = spec.start().await;
    let b = device("Phone").await;
    let offer = a
        .node
        .offer_pairing(|_request: PairApprovalRequest| std::future::pending::<bool>())
        .await
        .unwrap();

    let error = b
        .node
        .join_pairing(offer.ticket(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Declined), "{error}");
    assert!(offer.wait().await.is_err());
    assert!(a.trust.is_empty() && b.trust.is_empty());
}

#[tokio::test]
async fn a_wrong_secret_is_rejected_the_same_way_as_everything_else_and_changes_nothing() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();

    let mut forged = offer.ticket().clone();
    forged.secret = PairSecret::from_bytes([0x42; 32]);
    for _ in 0..5 {
        let error = b.node.join_pairing(&forged, |_| {}).await.unwrap_err();
        assert_eq!(rejected(&error), Some(PairRejection::Invalid), "{error}");
    }
    assert!(a.trust.is_empty() && b.trust.is_empty());

    // Guessing does not use the offer up: the real ticket still works.
    b.node.join_pairing(offer.ticket(), |_| {}).await.unwrap();
    offer.wait().await.unwrap();
}

#[tokio::test]
async fn a_ticket_works_once() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let c = device("Tablet").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let ticket = offer.ticket().clone();

    b.node.join_pairing(&ticket, |_| {}).await.unwrap();
    offer.wait().await.unwrap();

    let error = c.node.join_pairing(&ticket, |_| {}).await.unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Invalid), "{error}");
    assert!(!a.trust.contains(&c.id));
}

#[tokio::test]
async fn a_device_cannot_use_its_own_ticket() {
    let a = device("Laptop").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let error = a
        .node
        .join_pairing(offer.ticket(), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(error, Error::OwnTicket), "{error}");
}

#[tokio::test]
async fn an_expired_ticket_is_refused_by_the_joiner_without_connecting() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let lifetime = offer.expires_at() - common::START;
    b.clock.advance(lifetime + CLOCK_SKEW_ALLOWANCE_SECS);

    let error = b
        .node
        .join_pairing(offer.ticket(), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(error, Error::TicketExpired), "{error}");
}

#[tokio::test]
async fn a_joiner_whose_clock_runs_a_little_ahead_still_tries() {
    // The two clocks of two phones can differ by minutes. The offerer's answer counts.
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let lifetime = offer.expires_at() - common::START;
    b.clock.advance(lifetime + CLOCK_SKEW_ALLOWANCE_SECS - 1);

    b.node.join_pairing(offer.ticket(), |_| {}).await.unwrap();
    offer.wait().await.unwrap();
    assert!(a.trust.contains(&b.id));
}

#[tokio::test]
async fn the_offerer_enforces_expiry_even_if_the_joiners_clock_is_behind() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let ticket = offer.ticket().clone();
    a.clock.advance(offer.expires_at() - common::START);

    // The joiner still believes the ticket is good.
    let error = b.node.join_pairing(&ticket, |_| {}).await.unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Invalid), "{error}");
    // And the person who made the offer is told it ran out.
    let ended = tokio::time::timeout(Duration::from_secs(5), offer.wait())
        .await
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(ended, Error::OfferEnded(OfferEnd::Expired)),
        "{ended}"
    );
}

#[tokio::test]
async fn waiting_on_an_offer_ends_when_its_time_is_up() {
    let a = device("Laptop").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    a.clock.advance(offer.expires_at() - common::START);
    let ended = tokio::time::timeout(Duration::from_secs(5), offer.wait())
        .await
        .expect("the offer should have ended")
        .unwrap_err();
    assert!(
        matches!(ended, Error::OfferEnded(OfferEnd::Expired)),
        "{ended}"
    );
}

#[tokio::test]
async fn a_wall_clock_that_jumps_forward_does_not_end_an_offer_early() {
    // Someone sets the clock, or a time service corrects it, while the code is on screen.
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let lifetime = offer.expires_at() - common::START;
    a.clock.set_wall_clock(common::START + 10 * lifetime);

    b.node.join_pairing(offer.ticket(), |_| {}).await.unwrap();
    offer.wait().await.unwrap();
}

#[tokio::test]
async fn a_wall_clock_that_jumps_back_does_not_extend_an_offer() {
    let a = device("Laptop").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let lifetime = offer.expires_at() - common::START;
    a.clock.advance(lifetime);
    // Setting the clock back must not buy the offer more time.
    a.clock.set_wall_clock(common::START);

    let ended = tokio::time::timeout(Duration::from_secs(5), offer.wait())
        .await
        .expect("the offer should have ended")
        .unwrap_err();
    assert!(
        matches!(ended, Error::OfferEnded(OfferEnd::Expired)),
        "{ended}"
    );
}

#[tokio::test]
async fn an_offer_does_not_print_its_ticket() {
    // Debug output ends up in logs and bug reports.
    let a = device("Laptop").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let text = offer.ticket_text().to_owned();
    let body = &text["BATON1:".len()..];
    for shown in [format!("{offer:?}"), format!("{:?}", offer.ticket())] {
        assert!(!shown.contains("BATON1"), "{shown}");
        assert!(!shown.contains(&body[..24]), "{shown}");
    }
}

#[tokio::test]
async fn a_cancelled_offer_cannot_be_used() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let ticket = offer.ticket().clone();
    offer.cancel();

    let error = b.node.join_pairing(&ticket, |_| {}).await.unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Invalid), "{error}");
    let ended = offer.wait().await.unwrap_err();
    assert!(
        matches!(ended, Error::OfferEnded(OfferEnd::Cancelled)),
        "{ended}"
    );
}

#[tokio::test]
async fn dropping_an_offer_withdraws_it() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let ticket = a
        .node
        .offer_pairing(approve)
        .await
        .unwrap()
        .ticket()
        .clone();
    let error = b.node.join_pairing(&ticket, |_| {}).await.unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Invalid), "{error}");
}

#[tokio::test]
async fn a_newer_offer_replaces_the_older_one() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let first = a.node.offer_pairing(approve).await.unwrap();
    let second = a.node.offer_pairing(approve).await.unwrap();

    let error = b
        .node
        .join_pairing(first.ticket(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Invalid), "{error}");
    assert!(matches!(
        first.wait().await,
        Err(Error::OfferEnded(OfferEnd::Cancelled))
    ));
    b.node.join_pairing(second.ticket(), |_| {}).await.unwrap();
    second.wait().await.unwrap();
}

#[tokio::test]
async fn a_second_request_is_told_to_wait_while_the_first_is_with_the_person() {
    let a = device("Laptop").await;
    let b = Arc::new(device("Phone").await);
    let c = device("Tablet").await;

    let asked = Arc::new(Notify::new());
    let answer = Arc::new(Notify::new());
    let offer = {
        let (asked, answer) = (Arc::clone(&asked), Arc::clone(&answer));
        a.node
            .offer_pairing(move |_request: PairApprovalRequest| {
                let (asked, answer) = (Arc::clone(&asked), Arc::clone(&answer));
                async move {
                    asked.notify_one();
                    answer.notified().await;
                    true
                }
            })
            .await
            .unwrap()
    };
    let ticket = offer.ticket().clone();

    let first = {
        let (b, ticket) = (Arc::clone(&b), ticket.clone());
        tokio::spawn(async move { b.node.join_pairing(&ticket, |_| {}).await })
    };
    asked.notified().await;

    let error = c.node.join_pairing(&ticket, |_| {}).await.unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Busy), "{error}");

    answer.notify_one();
    first.await.unwrap().unwrap();
    offer.wait().await.unwrap();
    assert!(a.trust.contains(&b.id));
    assert!(!a.trust.contains(&c.id));
}

#[tokio::test]
async fn cancelling_while_the_person_is_deciding_leaves_nobody_paired() {
    let a = device("Laptop").await;
    let b = Arc::new(device("Phone").await);

    let asked = Arc::new(Notify::new());
    let offer = {
        let asked = Arc::clone(&asked);
        a.node
            .offer_pairing(move |_request: PairApprovalRequest| {
                let asked = Arc::clone(&asked);
                async move {
                    asked.notify_one();
                    std::future::pending::<bool>().await
                }
            })
            .await
            .unwrap()
    };
    let join = {
        let (b, ticket) = (Arc::clone(&b), offer.ticket().clone());
        tokio::spawn(async move { b.node.join_pairing(&ticket, |_| {}).await })
    };
    asked.notified().await;
    offer.cancel();

    let error = join.await.unwrap().unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Invalid), "{error}");
    assert!(a.trust.is_empty() && b.trust.is_empty());
}

/// Notes that the question it belongs to went away before anyone answered it, which
/// is what a prompt on a screen sees when its future is dropped.
struct TakenAway {
    flag: Arc<AtomicBool>,
    answered: bool,
}

impl Drop for TakenAway {
    fn drop(&mut self) {
        if !self.answered {
            self.flag.store(true, SeqCst);
        }
    }
}

/// An offer whose approver says `asked` when it is put to the person, waits for `answer`,
/// and sets `taken_away` if the question is dropped before then.
async fn offer_that_waits_for_the_person(
    node: &baton_core::Node,
    asked: &Arc<Notify>,
    answer: &Arc<Notify>,
    taken_away: &Arc<AtomicBool>,
) -> baton_core::PairOffer {
    let (asked, answer, taken_away) = (
        Arc::clone(asked),
        Arc::clone(answer),
        Arc::clone(taken_away),
    );
    node.offer_pairing(move |_request: PairApprovalRequest| {
        let (asked, answer, taken_away) = (
            Arc::clone(&asked),
            Arc::clone(&answer),
            Arc::clone(&taken_away),
        );
        async move {
            let mut question = TakenAway {
                flag: taken_away,
                answered: false,
            };
            asked.notify_one();
            answer.notified().await;
            question.answered = true;
            true
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_request_that_arrived_before_the_deadline_is_still_answered_after_it() {
    let a = device("Laptop").await;
    let b = Arc::new(device("Phone").await);
    let c = device("Tablet").await;
    let (asked, answer, taken_away) = (
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
        Arc::new(AtomicBool::new(false)),
    );
    let offer = offer_that_waits_for_the_person(&a.node, &asked, &answer, &taken_away).await;
    let ticket = offer.ticket().clone();
    let lifetime = offer.expires_at() - common::START;

    let join = {
        let (b, ticket) = (Arc::clone(&b), ticket.clone());
        tokio::spawn(async move { b.node.join_pairing(&ticket, |_| {}).await })
    };
    asked.notified().await;

    // The offer runs out while the person is looking at the request. The task that ends
    // offers looks every half second, so give it time to act if it is going to.
    a.clock.advance(lifetime + 1);
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert!(
        !taken_away.load(SeqCst),
        "the deadline took the question away from the person who was answering it"
    );

    // Nobody new is let in after the deadline, and nobody is told to wait for a request
    // that is only still here because of the one already being answered.
    let error = c.node.join_pairing(&ticket, |_| {}).await.unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Invalid), "{error}");
    assert!(!taken_away.load(SeqCst));

    answer.notify_one();
    join.await.unwrap().unwrap();
    offer.wait().await.unwrap();
    assert!(a.trust.contains(&b.id));
    assert!(!a.trust.contains(&c.id));
}

#[tokio::test]
async fn a_prompt_that_outlived_the_deadline_still_ends_with_the_approval_time_out() {
    let mut spec = Spec::new("Laptop");
    spec.tuning.approval_timeout = Duration::from_millis(1_500);
    let a = spec.start().await;
    let b = Arc::new(device("Phone").await);
    let (asked, answer, taken_away) = (
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
        Arc::new(AtomicBool::new(false)),
    );
    let offer = offer_that_waits_for_the_person(&a.node, &asked, &answer, &taken_away).await;
    let lifetime = offer.expires_at() - common::START;
    let join = {
        let (b, ticket) = (Arc::clone(&b), offer.ticket().clone());
        tokio::spawn(async move { b.node.join_pairing(&ticket, |_| {}).await })
    };
    asked.notified().await;
    a.clock.advance(lifetime + 1);

    // Nobody answers. The deadline has passed, so only the time the person was given
    // decides, and the joiner hears what it hears from any request nobody answered.
    let error = tokio::time::timeout(Duration::from_secs(10), join)
        .await
        .expect("the approval time-out should end the request")
        .unwrap()
        .unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Declined), "{error}");
    assert!(offer.wait().await.is_err());
    assert!(a.trust.is_empty() && b.trust.is_empty());
}

/// Makes a joiner's request by hand, so a test can send what a real device would not.
struct Forger {
    raw: Raw,
}

impl Forger {
    async fn new() -> Self {
        Self {
            raw: Raw::new().await,
        }
    }

    /// Connects to the offerer on the pairing protocol.
    async fn connect(&self, target: &common::Device) -> iroh::endpoint::Connection {
        self.raw.dial(target, ALPN_PAIR).await.unwrap()
    }

    fn proof_for(
        ticket: &baton_proto::ticket::PairTicket,
        joiner: &DeviceId,
        binding: &[u8; CHANNEL_BINDING_LEN],
    ) -> PairProof {
        PairProof::compute(&ticket.secret, &ticket.offerer, joiner, binding)
    }

    fn binding_of(connection: &iroh::endpoint::Connection) -> [u8; CHANNEL_BINDING_LEN] {
        let mut binding = [0u8; CHANNEL_BINDING_LEN];
        connection
            .export_keying_material(&mut binding, EXPORTER_LABEL, &[])
            .unwrap();
        binding
    }

    async fn ask(
        connection: &iroh::endpoint::Connection,
        request: &PairRequest,
    ) -> Result<PairResponse, String> {
        let (mut send, mut recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
        send.write_all(&frame::encode(request).unwrap())
            .await
            .map_err(|e| e.to_string())?;
        send.finish().unwrap();
        let bytes = tokio::time::timeout(Duration::from_secs(10), recv.read_to_end(64 * 1024))
            .await
            .map_err(|_| "no answer".to_owned())?
            .map_err(|e| e.to_string())?;
        frame::decode(&bytes).map_err(|e| e.to_string())
    }
}

#[tokio::test]
async fn a_proof_made_for_another_connection_is_useless() {
    let a = device("Laptop").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let forger = Forger::new().await;
    let connection = forger.connect(&a).await;

    // Right secret, wrong session: what someone who recorded a proof could replay.
    let stale_binding = [0x11; CHANNEL_BINDING_LEN];
    let request = PairRequest {
        device: info("Intruder"),
        proof: Forger::proof_for(offer.ticket(), &forger.raw.id, &stale_binding),
    };
    let response = Forger::ask(&connection, &request).await.unwrap();
    assert_eq!(response, PairResponse::Rejected(PairRejection::Invalid));
    assert!(a.trust.is_empty());
}

#[tokio::test]
async fn a_proof_made_for_another_joiner_is_useless() {
    let a = device("Laptop").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let forger = Forger::new().await;
    let connection = forger.connect(&a).await;

    // Made with the right secret and the right session, but naming somebody else.
    let someone_else = Raw::new().await.id;
    let request = PairRequest {
        device: info("Intruder"),
        proof: Forger::proof_for(
            offer.ticket(),
            &someone_else,
            &Forger::binding_of(&connection),
        ),
    };
    let response = Forger::ask(&connection, &request).await.unwrap();
    assert_eq!(response, PairResponse::Rejected(PairRejection::Invalid));
    assert!(a.trust.is_empty());
}

#[tokio::test]
async fn a_correct_hand_made_request_does_pair() {
    // The control for the two tests above: the forger's code is right when the inputs are.
    let a = device("Laptop").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();
    let forger = Forger::new().await;
    let connection = forger.connect(&a).await;

    let request = PairRequest {
        device: info("Hand made"),
        proof: Forger::proof_for(
            offer.ticket(),
            &forger.raw.id,
            &Forger::binding_of(&connection),
        ),
    };
    let response = Forger::ask(&connection, &request).await.unwrap();
    assert!(
        matches!(response, PairResponse::Accepted { .. }),
        "{response:?}"
    );
    assert!(a.trust.contains(&forger.raw.id));
}

#[tokio::test]
async fn someone_with_the_ticket_gets_a_code_of_their_own_not_the_real_joiners() {
    let a = device("Laptop").await;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let offer = {
        let seen = Arc::clone(&seen);
        a.node
            .offer_pairing(move |request: PairApprovalRequest| {
                seen.lock().unwrap().push(request.confirmation);
                std::future::ready(false)
            })
            .await
            .unwrap()
    };
    let ticket = offer.ticket().clone();
    let forger = Forger::new().await;
    let connection = forger.connect(&a).await;
    let binding = Forger::binding_of(&connection);

    let request = PairRequest {
        device: info("Laptop"), // a name proves nothing
        proof: Forger::proof_for(&ticket, &forger.raw.id, &binding),
    };
    let response = Forger::ask(&connection, &request).await.unwrap();
    assert_eq!(response, PairResponse::Rejected(PairRejection::Declined));

    // What the person was shown is a function of the forger's own key and session, which
    // is not what the device in their hand would compute with its key and session.
    let shown = seen.lock().unwrap().clone();
    let theirs = ConfirmationCode::compute(&ticket.secret, &a.id, &forger.raw.id, &binding);
    assert_eq!(shown, vec![theirs]);
    let real_joiner = device("Phone").await;
    let wrong_key = ConfirmationCode::compute(&ticket.secret, &a.id, &real_joiner.id, &binding);
    assert_ne!(wrong_key, theirs, "the code ignores which device is asking");
}

#[tokio::test]
async fn a_joiner_that_leaves_while_the_person_is_deciding_spends_the_offer() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let asked = Arc::new(Notify::new());
    let prompts = Arc::new(AtomicUsize::new(0));
    let offer = {
        let (asked, prompts) = (Arc::clone(&asked), Arc::clone(&prompts));
        a.node
            .offer_pairing(move |_request: PairApprovalRequest| {
                let (asked, prompts) = (Arc::clone(&asked), Arc::clone(&prompts));
                async move {
                    // The first request is left hanging; a second prompt would be accepted.
                    if prompts.fetch_add(1, SeqCst) == 0 {
                        asked.notify_one();
                        std::future::pending::<bool>().await
                    } else {
                        true
                    }
                }
            })
            .await
            .unwrap()
    };
    let ticket = offer.ticket().clone();

    // Someone with the ticket asks, and walks away while the person is looking at it.
    let leaver = Forger::new().await;
    let connection = leaver.connect(&a).await;
    let request = PairRequest {
        device: info("Flaky"),
        proof: Forger::proof_for(&ticket, &leaver.raw.id, &Forger::binding_of(&connection)),
    };
    let (mut send, _recv) = connection.open_bi().await.unwrap();
    send.write_all(&frame::encode(&request).unwrap())
        .await
        .unwrap();
    send.finish().unwrap();
    asked.notified().await;
    connection.close(0u32.into(), b"gone");

    // The person who made the offer is told the request was abandoned, because they never
    // got to decline it, and the ticket is spent, because it may be in someone else's hands.
    let ended = tokio::time::timeout(Duration::from_secs(5), offer.wait())
        .await
        .expect("the offerer should be told the request was abandoned")
        .unwrap_err();
    assert!(
        matches!(ended, Error::OfferEnded(OfferEnd::Abandoned)),
        "{ended}"
    );

    // Whoever asks next with the same ticket, the real joiner or a stranger with a new key,
    // gets no prompt. Each prompt shows a fresh code, so a ticket that could raise any
    // number of them would give a stranger any number of chances to match the code.
    let error = b.node.join_pairing(&ticket, |_| {}).await.unwrap_err();
    assert_eq!(rejected(&error), Some(PairRejection::Invalid), "{error}");
    for _ in 0..3 {
        let stranger = Forger::new().await;
        let connection = stranger.connect(&a).await;
        let request = PairRequest {
            device: info("Stranger"),
            proof: Forger::proof_for(&ticket, &stranger.raw.id, &Forger::binding_of(&connection)),
        };
        let response = Forger::ask(&connection, &request).await.unwrap();
        assert_eq!(response, PairResponse::Rejected(PairRejection::Invalid));
    }
    assert_eq!(prompts.load(SeqCst), 1, "one ticket raised a second prompt");
    assert!(a.trust.is_empty());
}

#[tokio::test]
async fn garbage_on_the_pairing_protocol_gets_the_uniform_answer() {
    let a = device("Laptop").await;
    let _offer = a.node.offer_pairing(approve).await.unwrap();
    let forger = Forger::new().await;
    let connection = forger.connect(&a).await;

    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let junk = b"this is not cbor";
    send.write_all(&header(u32::try_from(junk.len()).unwrap()))
        .await
        .unwrap();
    send.write_all(junk).await.unwrap();
    send.finish().unwrap();
    let bytes = recv.read_to_end(64 * 1024).await.unwrap();
    let response: PairResponse = frame::decode(&bytes).unwrap();
    assert_eq!(response, PairResponse::Rejected(PairRejection::Invalid));
}

#[tokio::test]
async fn an_oversized_pairing_frame_is_cut_off_before_it_is_read() {
    let a = device("Laptop").await;
    let forger = Forger::new().await;
    let connection = forger.connect(&a).await;

    let (mut send, _recv) = connection.open_bi().await.unwrap();
    // Claims 100 MiB. The receiver must give up on this peer without waiting for them.
    send.write_all(&header(100 * 1024 * 1024)).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(5), connection.closed())
        .await
        .expect("the receiver should answer straight away");
    match closed {
        iroh::endpoint::ConnectionError::ApplicationClosed(close) => {
            assert_eq!(close.error_code.into_inner(), 400);
        }
        other => panic!("expected the connection to be closed with code 400, got {other}"),
    }
}

/// The application error code a connection was closed with by the other side, if any.
fn closed_with(connection: &iroh::endpoint::Connection) -> Option<u64> {
    match connection.close_reason() {
        Some(iroh::endpoint::ConnectionError::ApplicationClosed(close)) => {
            Some(close.error_code.into_inner())
        }
        _ => None,
    }
}

#[tokio::test]
async fn idle_connections_do_not_keep_a_real_joiner_out() {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    let offer = a.node.offer_pairing(approve).await.unwrap();

    // Eight strangers connect and then say nothing, one for each place the device has.
    // Each stays well inside the admission limits, and none has to do anything else.
    let strangers = Forger::new().await;
    let mut idle = Vec::new();
    for _ in 0..8 {
        idle.push(strangers.connect(&a).await);
    }
    // Let the device take them all in before the real joiner arrives.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(idle.iter().all(|c| closed_with(c).is_none()));

    let joined = tokio::time::timeout(
        Duration::from_secs(5),
        b.node.join_pairing(offer.ticket(), |_| {}),
    )
    .await
    .expect("the real joiner was kept waiting");
    let joined = joined.expect("the real joiner was turned away by idle connections");
    assert_eq!(joined.id, a.id);
    offer.wait().await.unwrap();
    assert!(a.trust.contains(&b.id));

    // The joiner took the place of exactly one idle connection, which was told why.
    let pushed: Vec<_> = idle.iter().filter_map(closed_with).collect();
    assert_eq!(pushed, vec![429]);
}

#[tokio::test]
async fn new_connections_push_out_the_oldest_ones_that_have_shown_nothing() {
    let a = device("Laptop").await;
    let forger = Forger::new().await;

    let mut connections = Vec::new();
    for _ in 0..20 {
        connections.push(forger.connect(&a).await);
    }
    // A connection that is pushed out is closed as soon as the newcomer is accepted.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Eight places, so twelve of the twenty were pushed out, and they are the early ones.
    // The order in which the device takes connections in can differ a little from the
    // order they were made in, so the ends are checked, not every position.
    let pushed = connections
        .iter()
        .filter(|c| closed_with(c) == Some(429))
        .count();
    assert_eq!(
        pushed, 12,
        "12 of 20 connections should have been pushed out"
    );
    assert!(
        connections[..4].iter().all(|c| closed_with(c) == Some(429)),
        "the oldest connections should have been pushed out first"
    );
    assert!(
        connections[16..].iter().all(|c| closed_with(c).is_none()),
        "the newest connections should have been kept"
    );
}

#[tokio::test]
async fn a_request_with_a_valid_proof_keeps_its_place_whatever_arrives() {
    let a = device("Laptop").await;
    let asked = Arc::new(Notify::new());
    let answer = Arc::new(Notify::new());
    let offer = {
        let (asked, answer) = (Arc::clone(&asked), Arc::clone(&answer));
        a.node
            .offer_pairing(move |_request: PairApprovalRequest| {
                let (asked, answer) = (Arc::clone(&asked), Arc::clone(&answer));
                async move {
                    asked.notify_one();
                    answer.notified().await;
                    true
                }
            })
            .await
            .unwrap()
    };
    let joiner = Forger::new().await;
    let connection = joiner.connect(&a).await;
    let request = PairRequest {
        device: info("Phone"),
        proof: Forger::proof_for(
            offer.ticket(),
            &joiner.raw.id,
            &Forger::binding_of(&connection),
        ),
    };
    let reply = tokio::spawn(async move { Forger::ask(&connection, &request).await });
    asked.notified().await;

    // While the person is looking at the request, far more connections arrive than the
    // device has places for. All of them are strangers' that have shown nothing.
    let strangers = Forger::new().await;
    let mut flood = Vec::new();
    for _ in 0..30 {
        flood.push(strangers.connect(&a).await);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let pushed = flood.iter().filter(|c| closed_with(c) == Some(429)).count();
    assert!(pushed >= 22, "only {pushed} of 30 were pushed out");

    // The person says yes, and the joiner is still there to be told.
    answer.notify_one();
    let response = reply.await.unwrap().unwrap();
    assert!(
        matches!(response, PairResponse::Accepted { .. }),
        "{response:?}"
    );
    assert!(a.trust.contains(&joiner.raw.id));
}

#[tokio::test]
async fn a_connection_that_sends_nothing_is_cut_off_at_the_request_deadline() {
    let mut spec = Spec::new("Laptop");
    spec.tuning.request_deadline = Duration::from_millis(300);
    let a = spec.start().await;
    let forger = Forger::new().await;

    // No stream is ever opened. (A stream with nothing written on it is not seen by the
    // other side in QUIC, so opening one would change nothing.)
    let connection = forger.connect(&a).await;
    let closed = tokio::time::timeout(Duration::from_secs(5), connection.closed())
        .await
        .expect("a connection that sent nothing was left open");
    match closed {
        iroh::endpoint::ConnectionError::ApplicationClosed(close) => {
            assert_eq!(close.error_code.into_inner(), 408);
        }
        other => panic!("expected the connection to be closed with code 408, got {other}"),
    }
}

#[tokio::test]
async fn a_connection_that_starts_a_request_and_stalls_is_cut_off_at_the_same_deadline() {
    let mut spec = Spec::new("Laptop");
    spec.tuning.request_deadline = Duration::from_millis(300);
    let a = spec.start().await;
    let forger = Forger::new().await;
    let connection = forger.connect(&a).await;

    // Half a frame header, then nothing.
    let (mut send, _recv) = connection.open_bi().await.unwrap();
    send.write_all(&header(100)[..2]).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(5), connection.closed())
        .await
        .expect("a stalled request was left open");
    match closed {
        iroh::endpoint::ConnectionError::ApplicationClosed(close) => {
            assert_eq!(close.error_code.into_inner(), 408);
        }
        other => panic!("expected the connection to be closed with code 408, got {other}"),
    }
}
