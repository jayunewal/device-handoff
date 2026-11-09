// SPDX-License-Identifier: Apache-2.0

//! Tests against an SSH server in the same process that behaves like the Mac's: it takes a
//! password, or asks for it as a question, or takes none, and counts what it was given.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{HashAlg, PrivateKey};
use russh::server::{self, Auth, Response};
use russh::{MethodKind, MethodSet};
use tokio::net::TcpListener;

use super::{Mac, Outcome, fingerprint, unlock};

const ACCOUNT: &str = "jay";
const PASSWORD: &str = "right horse battery";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asks {
    /// Takes a password, as Remote Login does.
    Password,
    /// Asks for the password as a question (keyboard-interactive), in this many rounds, and takes
    /// nothing else. Each round asks for something that is shown as it is typed, and for the
    /// password.
    Question(usize),
    /// Takes a password and asks for it as a question too, as OpenSSH does by default.
    Both,
    /// Takes keys only.
    NoPassword,
}

#[derive(Debug, Clone)]
struct FakeMac {
    asks: Asks,
    /// How many times the server was given a password, in either way.
    given: Arc<AtomicUsize>,
    /// How many rounds of answers this connection has given.
    rounds: usize,
}

impl server::Handler for FakeMac {
    type Error = russh::Error;

    fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> impl Future<Output = Result<Auth, Self::Error>> + Send {
        self.given.fetch_add(1, Ordering::SeqCst);
        let takes = matches!(self.asks, Asks::Password | Asks::Both);
        let right = takes && user == ACCOUNT && password == PASSWORD;
        std::future::ready(Ok(if right { Auth::Accept } else { Auth::reject() }))
    }

    fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> impl Future<Output = Result<Auth, Self::Error>> + Send {
        std::future::ready(Ok(self.answer(user, response)))
    }
}

impl FakeMac {
    fn answer(&mut self, user: &str, response: Option<Response<'_>>) -> Auth {
        let rounds = match self.asks {
            Asks::Question(rounds) => rounds,
            Asks::Both => 1,
            Asks::Password | Asks::NoPassword => return Auth::reject(),
        };
        let question = Auth::Partial {
            name: Cow::Borrowed(""),
            instructions: Cow::Borrowed("This system is locked."),
            prompts: Cow::Owned(vec![
                (Cow::Borrowed("Name:"), true),
                (Cow::Borrowed("Password:"), false),
            ]),
        };
        let Some(response) = response else {
            self.rounds = 0;
            return question;
        };
        self.given.fetch_add(1, Ordering::SeqCst);
        self.rounds += 1;
        let answers: Vec<Vec<u8>> = response.map(|answer| answer.to_vec()).collect();
        // The shown question is answered with nothing, and the hidden one with the password.
        let right = user == ACCOUNT && answers == [Vec::new(), PASSWORD.as_bytes().to_vec()];
        if !right {
            return Auth::reject();
        }
        if self.rounds < rounds {
            question
        } else {
            Auth::Accept
        }
    }
}

fn host_key(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

fn fingerprint_of(key: &PrivateKey) -> String {
    key.public_key().fingerprint(HashAlg::Sha256).to_string()
}

struct Running {
    port: u16,
    key: String,
    given: Arc<AtomicUsize>,
}

/// Starts a fake Mac on a free port of this computer, which answers any number of connections.
async fn start(asks: Asks) -> Running {
    let key = host_key(7);
    let methods: &[MethodKind] = match asks {
        Asks::Password => &[MethodKind::Password],
        Asks::Question(_) => &[MethodKind::KeyboardInteractive],
        Asks::Both => &[MethodKind::Password, MethodKind::KeyboardInteractive],
        Asks::NoPassword => &[MethodKind::PublicKey],
    };
    let config = Arc::new(server::Config {
        methods: MethodSet::from(methods),
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![key.clone()],
        ..server::Config::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let given = Arc::new(AtomicUsize::new(0));
    let mac = FakeMac {
        asks,
        given: given.clone(),
        rounds: 0,
    };
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let config = config.clone();
            let mac = mac.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, stream, mac).await {
                    let _ = session.await;
                }
            });
        }
    });
    Running {
        port,
        key: fingerprint_of(&key),
        given,
    }
}

fn mac(port: u16, account: &str) -> Mac {
    Mac {
        host: "127.0.0.1".to_owned(),
        port,
        account: account.to_owned(),
    }
}

#[tokio::test]
async fn the_fingerprint_is_the_one_ssh_keygen_shows_and_nothing_is_given() {
    let fake = start(Asks::Password).await;
    let heard = fingerprint("127.0.0.1", fake.port).await.unwrap();
    assert_eq!(heard, fake.key);
    assert!(heard.starts_with("SHA256:"), "{heard}");
    assert_eq!(fake.given.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_trusted_mac_takes_the_right_password() {
    let fake = start(Asks::Password).await;
    let outcome = unlock(
        &mac(fake.port, ACCOUNT),
        PASSWORD,
        std::slice::from_ref(&fake.key),
    )
    .await;
    assert_eq!(outcome, Outcome::Accepted);
}

#[tokio::test]
async fn any_of_the_trusted_keys_will_do() {
    let fake = start(Asks::Password).await;
    let other = fingerprint_of(&host_key(9));
    let outcome = unlock(
        &mac(fake.port, ACCOUNT),
        PASSWORD,
        &[other, fake.key.clone()],
    )
    .await;
    assert_eq!(outcome, Outcome::Accepted);
}

#[tokio::test]
async fn a_wrong_password_or_account_is_rejected() {
    let fake = start(Asks::Password).await;
    let trusted = [fake.key.clone()];
    assert_eq!(
        unlock(&mac(fake.port, ACCOUNT), "wrong", &trusted).await,
        Outcome::Rejected
    );
    assert_eq!(
        unlock(&mac(fake.port, "someone"), PASSWORD, &trusted).await,
        Outcome::Rejected
    );
}

#[tokio::test]
async fn a_mac_with_another_key_is_given_nothing() {
    let fake = start(Asks::Password).await;
    let other = fingerprint_of(&host_key(9));
    let outcome = unlock(&mac(fake.port, ACCOUNT), PASSWORD, &[other]).await;
    assert_eq!(
        outcome,
        Outcome::UnknownKey {
            fingerprint: fake.key.clone()
        }
    );
    let outcome = unlock(&mac(fake.port, ACCOUNT), PASSWORD, &[]).await;
    assert_eq!(
        outcome,
        Outcome::UnknownKey {
            fingerprint: fake.key
        }
    );
    assert_eq!(fake.given.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_mac_that_asks_a_question_gets_the_password_as_the_answer() {
    let fake = start(Asks::Question(1)).await;
    let trusted = [fake.key.clone()];
    assert_eq!(
        unlock(&mac(fake.port, ACCOUNT), PASSWORD, &trusted).await,
        Outcome::Accepted
    );
    assert_eq!(
        unlock(&mac(fake.port, ACCOUNT), "wrong", &trusted).await,
        Outcome::Rejected
    );
}

#[tokio::test]
async fn a_mac_that_takes_no_password_says_so() {
    let fake = start(Asks::NoPassword).await;
    let outcome = unlock(&mac(fake.port, ACCOUNT), PASSWORD, &[fake.key]).await;
    assert_eq!(outcome, Outcome::NoPassword);
}

#[tokio::test]
async fn nothing_listening_is_unreachable() {
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };
    let outcome = unlock(&mac(port, ACCOUNT), PASSWORD, &[]).await;
    assert!(
        matches!(outcome, Outcome::Unreachable { .. }),
        "{outcome:?}"
    );
    let heard = fingerprint("127.0.0.1", port).await;
    assert!(
        matches!(heard, Err(Outcome::Unreachable { .. })),
        "{heard:?}"
    );
}

#[tokio::test]
async fn a_mac_may_ask_up_to_three_rounds_and_no_more() {
    let fake = start(Asks::Question(3)).await;
    let outcome = unlock(&mac(fake.port, ACCOUNT), PASSWORD, &[fake.key]).await;
    assert_eq!(outcome, Outcome::Accepted);
    let fake = start(Asks::Question(4)).await;
    let outcome = unlock(&mac(fake.port, ACCOUNT), PASSWORD, &[fake.key]).await;
    assert!(matches!(outcome, Outcome::Failed { .. }), "{outcome:?}");
}

#[tokio::test]
async fn a_mac_that_takes_both_ways_is_given_the_password_once() {
    let fake = start(Asks::Both).await;
    let trusted = [fake.key.clone()];
    assert_eq!(
        unlock(&mac(fake.port, ACCOUNT), "wrong", &trusted).await,
        Outcome::Rejected
    );
    assert_eq!(fake.given.load(Ordering::SeqCst), 1);
    assert_eq!(
        unlock(&mac(fake.port, ACCOUNT), PASSWORD, &trusted).await,
        Outcome::Accepted
    );
    assert_eq!(fake.given.load(Ordering::SeqCst), 2);
}

#[test]
fn a_certificate_is_not_trusted_even_for_a_trusted_key() {
    use russh::keys::PublicKeyOrCertificate;
    use russh::keys::ssh_key::certificate::{Builder, CertType};

    let host = host_key(7);
    let mut builder = Builder::new(
        vec![1; 16],
        host.public_key().key_data().clone(),
        0,
        u64::MAX,
    )
    .unwrap();
    builder.cert_type(CertType::Host).unwrap();
    builder.valid_principal("mac").unwrap();
    let certificate = builder.sign(&host_key(8)).unwrap();
    let checker = super::Checker {
        trusted: vec![fingerprint_of(&host)],
        heard: super::Heard::default(),
    };
    assert!(!checker.check(&PublicKeyOrCertificate::Certificate(certificate)));
    let plain = PublicKeyOrCertificate::PublicKey {
        key: host.public_key().clone(),
        hash_alg: None,
    };
    assert!(checker.check(&plain));
}

#[tokio::test]
async fn a_trusted_fingerprint_may_carry_spaces_around_it() {
    let fake = start(Asks::Password).await;
    let trusted = [format!("  {}\n", fake.key)];
    let outcome = unlock(&mac(fake.port, ACCOUNT), PASSWORD, &trusted).await;
    assert_eq!(outcome, Outcome::Accepted);
}

#[test]
fn a_connection_that_fails_after_a_trusted_key_is_not_an_unknown_key() {
    let key = fingerprint_of(&host_key(7));
    let trusted = [key.clone()];
    let failed = super::refused(&trusted, Some(key), russh::Error::Disconnect);
    assert!(matches!(failed, Outcome::Failed { .. }), "{failed:?}");
    let other = fingerprint_of(&host_key(9));
    let unknown = super::refused(&trusted, Some(other.clone()), russh::Error::UnknownKey);
    assert_eq!(unknown, Outcome::UnknownKey { fingerprint: other });
    let nothing = super::refused(&trusted, None, russh::Error::Disconnect);
    assert!(matches!(nothing, Outcome::Failed { .. }), "{nothing:?}");
}
