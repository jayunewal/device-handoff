// SPDX-License-Identifier: Apache-2.0

//! Unlocking a Mac that `FileVault` holds locked after a restart, by giving its password over
//! SSH (the crate `baton_unlock_ssh`). This needs no running Baton on either side: the Mac is
//! not running anything yet, Baton included.
//!
//! The app keeps the Mac's address, the account, the password and the fingerprints of the host
//! keys it trusts in the keychain of the phone, and hands them in for each attempt. Nothing here
//! keeps any of them.

// The Dart side hands over owned values, and the generated glue cannot pass references.
#![allow(clippy::needless_pass_by_value)]

use baton_unlock_ssh::{Mac, Outcome};
use zeroize::Zeroizing;

/// What the Mac answered when it was asked for its host key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshKeyCheck {
    /// The Mac answered with this key. The fingerprint is written as `ssh-keygen -l` writes it:
    /// `SHA256:` and the hash in base64, which is what the person compares with the Mac.
    Heard {
        /// The key's fingerprint.
        fingerprint: String,
    },
    /// Nothing answered at the address in time.
    Unreachable {
        /// What failed, for a log.
        detail: String,
    },
    /// Something answered but gave no key, such as a server that is not an SSH server.
    Failed {
        /// What failed, for a log.
        detail: String,
    },
}

/// How an attempt to unlock a Mac ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshUnlock {
    /// The Mac took the password. A locked Mac is unlocking its disk and starting up; a Mac that
    /// was already running only checked it.
    Accepted,
    /// The Mac said the account name or the password is wrong.
    Rejected,
    /// The Mac answered with a key that is not trusted, and was given nothing.
    UnknownKey {
        /// The fingerprint of the key it answered with.
        fingerprint: String,
    },
    /// The Mac does not take a password.
    NoPassword,
    /// Nothing answered at the address in time.
    Unreachable {
        /// What failed, for a log.
        detail: String,
    },
    /// Anything else.
    Failed {
        /// What failed, for a log.
        detail: String,
    },
}

/// Asks the Mac at `host` and `port` for its host key, and gives it nothing.
pub async fn ssh_mac_key(host: String, port: u16) -> SshKeyCheck {
    match baton_unlock_ssh::fingerprint(&host, port).await {
        Ok(fingerprint) => SshKeyCheck::Heard { fingerprint },
        Err(Outcome::Unreachable { detail }) => SshKeyCheck::Unreachable { detail },
        Err(other) => SshKeyCheck::Failed {
            detail: format!("{other:?}"),
        },
    }
}

/// Gives `password` to the Mac at `host` and `port` for `account`, if the Mac answers with a
/// host key whose fingerprint is one of `trusted`.
pub async fn ssh_unlock_mac(
    host: String,
    port: u16,
    account: String,
    password: String,
    trusted: Vec<String>,
) -> SshUnlock {
    let password = Zeroizing::new(password);
    let mac = Mac {
        host,
        port,
        account,
    };
    tracing::info!(port, "unlock over SSH: trying");
    let outcome = baton_unlock_ssh::unlock(&mac, &password, &trusted).await;
    // The outcome holds no secret: at most a fingerprint, which is public, and an error text.
    tracing::info!(?outcome, "unlock over SSH: done");
    match outcome {
        Outcome::Accepted => SshUnlock::Accepted,
        Outcome::Rejected => SshUnlock::Rejected,
        Outcome::UnknownKey { fingerprint } => SshUnlock::UnknownKey { fingerprint },
        Outcome::NoPassword => SshUnlock::NoPassword,
        Outcome::Unreachable { detail } => SshUnlock::Unreachable { detail },
        Outcome::Failed { detail } => SshUnlock::Failed { detail },
    }
}
