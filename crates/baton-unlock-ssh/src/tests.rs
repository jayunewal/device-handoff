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
    /// Asks for the password as a question (keyboard-interactive) and takes nothing else.
    Question,
    /// Takes keys only.
    NoPassword,
}

#[derive(Debug, Clone)]
struct FakeMac {
    asks: Asks,
    /// How many times the server was given a password, in either way.
    given: Arc<AtomicUsize>,
}

impl server::Handler for FakeMac {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.given.fetch_add(1, Ordering::SeqCst);
        if self.asks == Asks::Password && user == ACCOUNT && password == PASSWORD {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        if self.asks != Asks::Question {
            return Ok(Auth::reject());
        }
        let Some(response) = response else {
            return Ok(Auth::Partial {
                name: Cow::Borrowed(""),
                instructions: Cow::Borrowed("This system is locked."),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Password:"), false)]),
            });
        };
        self.given.fetch_add(1, Ordering::SeqCst);
        let answers: Vec<Vec<u8>> = response.map(|answer| answer.to_vec()).collect();
        if user == ACCOUNT && answers == [PASSWORD.as_bytes().to_vec()] {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
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
        Asks::Question => &[MethodKind::KeyboardInteractive],
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
    let fake = start(Asks::Question).await;
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
