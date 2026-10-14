// SPDX-License-Identifier: Apache-2.0

//! This device's identity: an Ed25519 key pair.
//!
//! The public half is the [`DeviceId`] other devices pair with and dial. The
//! secret half never leaves this machine; it is stored in `identity.toml`,
//! readable by its owner only. Keeping it in an operating system keystore is
//! planned (see `docs/security.md`).

use std::path::Path;

use baton_proto::DeviceId;
use data_encoding::HEXLOWER_PERMISSIVE;
use iroh_base::SecretKey;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    error::{Error, Result},
    fsutil::{read_text, require_owner_only, write_atomic},
};

/// Newest identity file format this build reads and writes.
const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    version: u32,
    secret_key: String,
}

/// A device's key pair.
pub struct Identity {
    secret: SecretKey,
    id: DeviceId,
}

impl Identity {
    /// Makes a fresh identity from the operating system's random number generator.
    pub fn generate() -> Self {
        Self::from_secret(SecretKey::generate())
    }

    /// Rebuilds an identity from its 32 secret key bytes.
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Self {
        Self::from_secret(SecretKey::from_bytes(bytes))
    }

    fn from_secret(secret: SecretKey) -> Self {
        let id = DeviceId::from_bytes(*secret.public().as_bytes());
        Self { secret, id }
    }

    /// The public identity other devices see.
    pub const fn device_id(&self) -> DeviceId {
        self.id
    }

    /// The key the transport needs. Not exposed outside the crate.
    #[expect(
        dead_code,
        reason = "first used by the transport, which lands in the next change"
    )]
    pub(crate) const fn secret_key(&self) -> &SecretKey {
        &self.secret
    }

    /// Loads the identity at `path`, or creates and saves a new one if there is none.
    ///
    /// On Unix this refuses to read a key file that other users can access. The
    /// caller should hold the [`DataDir`](crate::DataDir) lock so two processes
    /// cannot both create one.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if let Some(identity) = Self::load(path)? {
            return Ok(identity);
        }
        let identity = Self::generate();
        identity.save(path)?;
        tracing::info!(device_id = %identity.id, "created a new device identity");
        Ok(identity)
    }

    /// Loads the identity at `path`, if the file exists.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let Some(text) = read_text(path)?.map(Zeroizing::new) else {
            return Ok(None);
        };
        require_owner_only(path)?;

        let stored: StoredIdentity =
            toml::from_str(&text).map_err(|e| Error::invalid(path, e.message()))?;
        if stored.version > FORMAT_VERSION {
            return Err(Error::UnsupportedVersion {
                path: path.to_owned(),
                found: stored.version,
                supported: FORMAT_VERSION,
            });
        }
        let stored = Zeroizing::new(stored.secret_key);
        let bytes = Zeroizing::new(
            HEXLOWER_PERMISSIVE
                .decode(stored.trim().as_bytes())
                .map_err(|_| Error::invalid(path, "secret_key is not hexadecimal"))?,
        );
        let bytes: &[u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| Error::invalid(path, "secret_key must be 32 bytes (64 hex characters)"))?;
        Ok(Some(Self::from_secret_bytes(bytes)))
    }

    fn save(&self, path: &Path) -> Result<()> {
        let secret = Zeroizing::new(HEXLOWER_PERMISSIVE.encode(&self.secret.to_bytes()));
        let stored = StoredIdentity {
            version: FORMAT_VERSION,
            secret_key: (*secret).clone(),
        };
        let text = Zeroizing::new(toml::to_string(&stored).map_err(|e| Error::invalid(path, &e))?);
        write_atomic(path, text.as_bytes())
    }
}

// Never print the secret key.
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("device_id", &self.id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_identity_is_created_once_and_then_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.toml");

        let first = Identity::load_or_create(&path).unwrap();
        let second = Identity::load_or_create(&path).unwrap();
        assert_eq!(first.device_id(), second.device_id());
        assert_eq!(first.secret.to_bytes(), second.secret.to_bytes());
    }

    #[test]
    fn identities_are_distinct() {
        assert_ne!(
            Identity::generate().device_id(),
            Identity::generate().device_id()
        );
    }

    #[test]
    fn the_device_id_is_the_public_key() {
        let identity = Identity::from_secret_bytes(&[7; 32]);
        assert_eq!(
            identity.device_id().as_bytes(),
            identity.secret.public().as_bytes()
        );
    }

    #[test]
    fn debug_output_does_not_contain_the_secret() {
        let identity = Identity::from_secret_bytes(&[0xAB; 32]);
        let shown = format!("{identity:?}");
        assert!(!shown.contains("abab"), "{shown}");
        assert!(shown.contains("device_id"));
    }

    #[test]
    fn the_file_holds_the_secret_in_hex_with_a_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.toml");
        Identity::from_secret_bytes(&[0x11; 32])
            .save(&path)
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("version = 1"), "{text}");
        assert!(text.contains(&"11".repeat(32)), "{text}");
    }

    #[test]
    fn damaged_files_are_errors_not_silently_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.toml");
        let write = |text: &str| {
            std::fs::write(&path, text).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
        };

        for bad in [
            "",
            "garbage",
            "version = 1\nsecret_key = \"zz\"",
            "version = 1\nsecret_key = \"abcd\"",
            "version = 1",
        ] {
            write(bad);
            let err = Identity::load_or_create(&path).unwrap_err();
            assert!(matches!(err, Error::Invalid { .. }), "{bad:?} gave {err:?}");
            // The damaged file must still be there for the person to inspect.
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
        }

        write(&format!(
            "version = 99\nsecret_key = \"{}\"",
            "00".repeat(32)
        ));
        assert!(matches!(
            Identity::load(&path),
            Err(Error::UnsupportedVersion {
                found: 99,
                supported: 1,
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_other_users_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.toml");
        Identity::generate().save(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(matches!(
            Identity::load(&path),
            Err(Error::InsecurePermissions { .. })
        ));
    }
}
