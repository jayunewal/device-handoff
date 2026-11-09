// SPDX-License-Identifier: Apache-2.0

//! Unlocks a Mac that `FileVault` holds locked after a restart.
//!
//! A Mac with `FileVault` turned on keeps its disk locked after a restart until someone gives the
//! password of an account on it. Nothing else runs before that, Baton included, so a Mac that
//! restarted while its owner was away could not be reached. Since macOS 26 the Mac can take that
//! password over SSH: with Remote Login turned on, a small SSH server answers before the disk is
//! unlocked, and the password of a local account unlocks the disk and lets the Mac start. The
//! connection then closes by itself.
//!
//! This crate is the SSH client for that one step, and does nothing else: it never opens a
//! shell, runs a command or keeps the connection. It is also safe to use on a Mac that is
//! already running, where the password is checked and the connection is closed.
//!
//! The Mac proves who it is with its host key. The caller keeps the fingerprints of the keys it
//! trusts and passes them in, and a Mac that answers with any other key is not given the password
//! ([`Outcome::UnknownKey`]). The first time, [`fingerprint`] asks the Mac for its key without
//! giving anything, so that the person can compare it with what the Mac shows.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, AuthResult, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use russh::{Disconnect, MethodKind};

/// The port that Remote Login answers on.
pub const SSH_PORT: u16 = 22;

/// How long one attempt may take, from the first try to reach the Mac to the Mac's answer. A Mac
/// that is starting up can be slow to answer, and a phone on a weak network slower, but someone
/// is looking at the phone and waiting.
const DEADLINE: Duration = Duration::from_secs(25);

/// How many rounds of questions a Mac may ask when it asks for the password as a question
/// (keyboard-interactive). A Mac asks once; more than a few is a server that will never be done.
const MAX_ROUNDS: usize = 3;

/// The Mac to unlock and the account whose password unlocks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mac {
    /// The Mac's name on the network, such as `Jays-MBP.local`, or its address.
    pub host: String,
    /// The port of its SSH server, normally [`SSH_PORT`].
    pub port: u16,
    /// The short name of an account on the Mac that may unlock its disk.
    pub account: String,
}

/// How an attempt ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The Mac took the password. A locked Mac is unlocking its disk and starting up; a Mac that
    /// was already running only checked it.
    Accepted,
    /// The Mac said the account name or the password is wrong.
    Rejected,
    /// The Mac answered with a host key that is not one of the trusted ones, and was told nothing.
    UnknownKey {
        /// The fingerprint of the key it answered with, as `ssh-keygen -l` writes it
        /// (`SHA256:` and the hash in unpadded base64).
        fingerprint: String,
    },
    /// The Mac answered but does not take a password at all.
    NoPassword,
    /// Nothing answered at the address in time.
    Unreachable {
        /// What failed, for a log.
        detail: String,
    },
    /// Anything else, such as a server that does not speak SSH.
    Failed {
        /// What failed, for a log.
        detail: String,
    },
}

/// Asks the Mac for its host key and gives it nothing. The answer is the key's fingerprint, as
/// `ssh-keygen -l` writes it, for the person to compare with what the Mac shows before it is
/// trusted. Any other outcome means that no key was heard.
pub async fn fingerprint(host: &str, port: u16) -> Result<String, Outcome> {
    let heard = Heard::default();
    let checker = Checker {
        trusted: Vec::new(),
        heard: heard.clone(),
    };
    let reached = tokio::time::timeout(DEADLINE, connect(host, port, checker)).await;
    if let Some(fingerprint) = heard.take() {
        return Ok(fingerprint);
    }
    Err(match reached {
        Err(_) => Outcome::Unreachable {
            detail: "no answer in time".to_owned(),
        },
        Ok(Err(e)) => failure(e),
        // A checker that trusts nothing never lets the connection through.
        Ok(Ok(_)) => Outcome::Failed {
            detail: "the Mac gave no key".to_owned(),
        },
    })
}

/// Gives `password` to the Mac for `mac.account`, if the Mac answers with one of the `trusted`
/// host keys (fingerprints as [`fingerprint`] gives them).
///
/// The password is given once, in one way: as a password when the Mac takes one, otherwise as the
/// answer to the Mac's question. Nothing more is done once the Mac has answered: the connection is
/// closed whatever happened.
pub async fn unlock(mac: &Mac, password: &str, trusted: &[String]) -> Outcome {
    match tokio::time::timeout(DEADLINE, attempt(mac, password, trusted)).await {
        Ok(outcome) => outcome,
        Err(_) => Outcome::Unreachable {
            detail: "no answer in time".to_owned(),
        },
    }
}

async fn attempt(mac: &Mac, password: &str, trusted: &[String]) -> Outcome {
    let heard = Heard::default();
    let checker = Checker {
        trusted: trusted.to_vec(),
        heard: heard.clone(),
    };
    let mut session = match connect(&mac.host, mac.port, checker).await {
        Ok(session) => session,
        Err(e) => return refused(trusted, heard.take(), e),
    };
    let outcome = authenticate(&mut session, &mac.account, password).await;
    // A Mac that has just unlocked closes the connection itself, and a failure to say goodbye to
    // it changes nothing.
    let _ = session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await;
    outcome
}

async fn authenticate(
    session: &mut client::Handle<Checker>,
    account: &str,
    password: &str,
) -> Outcome {
    // Asking with no credentials first makes the Mac name the ways it takes, so that the password
    // is given once, in one way: a wrong password given as a password and again as an answer would
    // count as two failed tries at a Mac whose disk is locked.
    let ways = match session.authenticate_none(account).await {
        Ok(AuthResult::Success) => return Outcome::Accepted,
        Ok(AuthResult::Failure {
            remaining_methods, ..
        }) => remaining_methods,
        Err(e) => return failure(e),
    };
    let password_taken = ways.contains(&MethodKind::Password);
    if !password_taken && ways.contains(&MethodKind::KeyboardInteractive) {
        return answer_questions(session, account, password).await;
    }
    // A Mac that names no way at all is given the password, which is what Remote Login takes.
    if !password_taken && !ways.is_empty() {
        return Outcome::NoPassword;
    }
    match session.authenticate_password(account, password).await {
        Ok(AuthResult::Success) => Outcome::Accepted,
        Ok(AuthResult::Failure { .. }) => Outcome::Rejected,
        Err(e) => failure(e),
    }
}

/// Keyboard-interactive: the Mac asks questions and the client answers them. Every hidden
/// question is answered with the password, which is the only thing a Mac asks for this way, and a
/// question that would be shown as it is typed is answered with nothing.
async fn answer_questions(
    session: &mut client::Handle<Checker>,
    account: &str,
    password: &str,
) -> Outcome {
    let mut reply = session
        .authenticate_keyboard_interactive_start(account, None)
        .await;
    let mut rounds = 0;
    loop {
        match reply {
            Ok(KeyboardInteractiveAuthResponse::Success) => return Outcome::Accepted,
            Ok(KeyboardInteractiveAuthResponse::Failure { .. }) => return Outcome::Rejected,
            Ok(KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. }) => {
                if rounds == MAX_ROUNDS {
                    return Outcome::Failed {
                        detail: "the Mac kept asking questions".to_owned(),
                    };
                }
                rounds += 1;
                let answers = prompts
                    .iter()
                    .map(|p| {
                        if p.echo {
                            String::new()
                        } else {
                            password.to_owned()
                        }
                    })
                    .collect();
                reply = session
                    .authenticate_keyboard_interactive_respond(answers)
                    .await;
            }
            Err(e) => return failure(e),
        }
    }
}

async fn connect(
    host: &str,
    port: u16,
    checker: Checker,
) -> Result<client::Handle<Checker>, russh::Error> {
    let config = client::Config {
        inactivity_timeout: Some(DEADLINE),
        ..client::Config::default()
    };
    client::connect(Arc::new(config), (host, port), checker).await
}

/// How a connection that failed before the Mac was given anything ended: the Mac's key was not
/// trusted, or, with a trusted key or none heard, the connection itself failed.
fn refused(trusted: &[String], heard: Option<String>, e: russh::Error) -> Outcome {
    match heard {
        Some(fingerprint) if !is_trusted(trusted, &fingerprint) => {
            Outcome::UnknownKey { fingerprint }
        }
        _ => failure(e),
    }
}

fn failure(e: russh::Error) -> Outcome {
    match e {
        russh::Error::IO(e) => Outcome::Unreachable {
            detail: e.to_string(),
        },
        russh::Error::ConnectionTimeout | russh::Error::InactivityTimeout => Outcome::Unreachable {
            detail: "no answer in time".to_owned(),
        },
        e => Outcome::Failed {
            detail: e.to_string(),
        },
    }
}

fn is_trusted(trusted: &[String], fingerprint: &str) -> bool {
    trusted.iter().any(|t| t.trim() == fingerprint)
}

/// The fingerprint of the key that the Mac answered with, kept for after the connection, which
/// fails when the key is not trusted.
#[derive(Debug, Clone, Default)]
struct Heard(Arc<Mutex<Option<String>>>);

impl Heard {
    fn put(&self, fingerprint: String) {
        if let Ok(mut heard) = self.0.lock() {
            *heard = Some(fingerprint);
        }
    }

    fn take(&self) -> Option<String> {
        self.0.lock().ok().and_then(|mut heard| heard.take())
    }
}

/// Lets the connection through only for a trusted host key, and notes the key either way.
#[derive(Debug)]
struct Checker {
    trusted: Vec<String>,
    heard: Heard,
}

impl client::Handler for Checker {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        std::future::ready(Ok(self.check(key)))
    }
}

impl Checker {
    fn check(&self, key: &PublicKeyOrCertificate) -> bool {
        // A Mac answers with a plain key. A certificate would need an authority to trust.
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return false;
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let trusted = is_trusted(&self.trusted, &fingerprint);
        self.heard.put(fingerprint);
        trusted
    }
}

#[cfg(test)]
mod tests;
