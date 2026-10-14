// SPDX-License-Identifier: Apache-2.0

//! This device's identity: an Ed25519 key pair.
//!
//! The public half is the [`DeviceId`] other devices pair with and dial. The
//! secret half never leaves this machine; it is stored in `identity.toml`,
//! readable by its owner only. Keeping it in an operating system keystore is
//! planned (see `docs/security.md`).
//!
//! The file also records which machine made the key. Copying the state folder to
//! another machine, by a migration tool or a backup restore, would make two machines
//! answer to one identity, so Baton refuses a key file that it finds on a machine other
//! than the one that made it. `docs/security.md` says how the machine is told apart and
//! where that cannot be done.

use std::path::Path;

use baton_proto::DeviceId;
use data_encoding::HEXLOWER_PERMISSIVE;
use iroh_base::SecretKey;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    error::{Error, Result},
    fsutil::{Access, read_text, write_atomic},
    machine::{self, MachineId},
};

/// Newest identity file format this build reads and writes.
const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    version: u32,
    secret_key: String,
    /// Which machine made the key, as a hash. Absent in a file made before this was
    /// recorded, or on a machine that could not be identified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    machine: Option<String>,
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
    pub(crate) const fn secret_key(&self) -> &SecretKey {
        &self.secret
    }

    /// Loads the identity at `path`, or creates and saves a new one if there is none.
    ///
    /// On Unix this refuses to read a key file that other users can access, before any
    /// of it is read. It also refuses one that another machine made, see
    /// [`Error::WrongMachine`]. A key file from before the machine was recorded is
    /// recorded as belonging to this one. The caller should hold the
    /// [`DataDir`](crate::DataDir) lock so two processes cannot both create one.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        Self::load_or_create_on(path, machine::current().as_ref())
    }

    /// Loads the identity at `path`, if the file exists.
    ///
    /// Refuses a key file that other users can access, or that another machine made.
    /// Does not change the file.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        Self::load_on(path, machine::current().as_ref())
    }

    /// Takes over the identity at `path` for this machine, if the file exists.
    ///
    /// For when [`load`](Self::load) says the key was made on another machine and that is
    /// wrong: the machine's own identifier changed, say, after a reinstall. If the folder
    /// really was copied from another machine, adopt the identity only once the other copy
    /// is gone, or both will answer to it.
    pub fn adopt_on_this_machine(path: &Path) -> Result<Option<Self>> {
        Self::adopt_on(path, machine::current().as_ref())
    }

    fn load_or_create_on(path: &Path, here: Option<&MachineId>) -> Result<Self> {
        if let Some((identity, made_on)) = Self::read(path)? {
            check_machine(path, made_on.as_ref(), here)?;
            if made_on.is_none() && here.is_some() {
                // Nothing says where this key came from, so it belongs to the first machine
                // that sees it. Failing to say so must not stop Baton from starting.
                if let Err(e) = identity.save(path, here) {
                    tracing::warn!(error = %e, "could not record which machine made the device identity");
                }
            }
            return Ok(identity);
        }
        let identity = Self::generate();
        identity.save(path, here)?;
        tracing::info!(device_id = %identity.id, "created a new device identity");
        Ok(identity)
    }

    fn load_on(path: &Path, here: Option<&MachineId>) -> Result<Option<Self>> {
        let Some((identity, made_on)) = Self::read(path)? else {
            return Ok(None);
        };
        check_machine(path, made_on.as_ref(), here)?;
        Ok(Some(identity))
    }

    fn adopt_on(path: &Path, here: Option<&MachineId>) -> Result<Option<Self>> {
        let Some((identity, _)) = Self::read(path)? else {
            return Ok(None);
        };
        identity.save(path, here)?;
        Ok(Some(identity))
    }

    /// Reads the key file, and the machine it says made the key, without judging either.
    fn read(path: &Path) -> Result<Option<(Self, Option<MachineId>)>> {
        let Some(text) = read_text(path, Access::OwnerOnly)?.map(Zeroizing::new) else {
            return Ok(None);
        };

        let stored: StoredIdentity =
            toml::from_str(&text).map_err(|e| Error::invalid(path, e.message()))?;
        if stored.version > FORMAT_VERSION {
            return Err(Error::UnsupportedVersion {
                path: path.to_owned(),
                found: stored.version,
                supported: FORMAT_VERSION,
            });
        }
        let made_on = stored
            .machine
            .as_deref()
            .map(|value| {
                MachineId::parse(value)
                    .ok_or_else(|| Error::invalid(path, "machine is not a hash made by Baton"))
            })
            .transpose()?;
        let secret = Zeroizing::new(stored.secret_key);
        let bytes = Zeroizing::new(
            HEXLOWER_PERMISSIVE
                .decode(secret.trim().as_bytes())
                .map_err(|_| Error::invalid(path, "secret_key is not hexadecimal"))?,
        );
        let bytes: &[u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| Error::invalid(path, "secret_key must be 32 bytes (64 hex characters)"))?;
        Ok(Some((Self::from_secret_bytes(bytes), made_on)))
    }

    fn save(&self, path: &Path, machine: Option<&MachineId>) -> Result<()> {
        let secret = Zeroizing::new(HEXLOWER_PERMISSIVE.encode(&self.secret.to_bytes()));
        let stored = StoredIdentity {
            version: FORMAT_VERSION,
            secret_key: (*secret).clone(),
            machine: machine.map(|m| m.as_str().to_owned()),
        };
        let text = Zeroizing::new(toml::to_string(&stored).map_err(|e| Error::invalid(path, &e))?);
        write_atomic(path, text.as_bytes())
    }
}

/// Fails if the key was made on a different machine from this one. If either is unknown
/// there is nothing to compare, and the key is let through.
fn check_machine(path: &Path, made_on: Option<&MachineId>, here: Option<&MachineId>) -> Result<()> {
    match (made_on, here) {
        (Some(made_on), Some(here)) if made_on != here => Err(Error::WrongMachine {
            path: path.to_owned(),
        }),
        _ => Ok(()),
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
            .save(&path, None)
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
            &format!(
                "version = 1\nsecret_key = \"{}\"\nmachine = \"not a hash\"",
                "00".repeat(32)
            ),
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
        Identity::generate().save(&path, None).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(matches!(
            Identity::load(&path),
            Err(Error::InsecurePermissions { .. })
        ));
    }

    const HERE: &str = "5a5e7e4b-2d5c-5b7c-8f1e-0123456789ab";
    const ELSEWHERE: &str = "4c4c4544-0042-4210-8039-b6c04f453032";

    fn machine(raw: &str) -> MachineId {
        MachineId::from_raw(raw).unwrap()
    }

    fn key_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.toml");
        (dir, path)
    }

    #[test]
    fn a_new_identity_records_the_machine_that_made_it() {
        let (_dir, path) = key_file();
        let here = machine(HERE);
        let made = Identity::load_or_create_on(&path, Some(&here)).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(here.as_str()), "{text}");
        assert!(
            !text.contains(HERE),
            "the identifier itself must not be written: {text}"
        );

        let again = Identity::load_or_create_on(&path, Some(&here)).unwrap();
        assert_eq!(again.device_id(), made.device_id());
        assert!(Identity::load_on(&path, Some(&here)).unwrap().is_some());
    }

    #[test]
    fn a_key_file_copied_to_another_machine_is_refused_and_left_alone() {
        let (_dir, path) = key_file();
        Identity::load_or_create_on(&path, Some(&machine(ELSEWHERE))).unwrap();
        let before = std::fs::read(&path).unwrap();

        let here = machine(HERE);
        assert!(matches!(
            Identity::load_on(&path, Some(&here)),
            Err(Error::WrongMachine { .. })
        ));
        // Starting up must not quietly make a second identity or take over the first.
        assert!(matches!(
            Identity::load_or_create_on(&path, Some(&here)),
            Err(Error::WrongMachine { .. })
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);

        let message = Identity::load_on(&path, Some(&here))
            .unwrap_err()
            .to_string();
        assert!(message.contains("identity.toml"), "{message}");
        assert!(message.contains("pair again"), "{message}");
    }

    #[test]
    fn a_key_file_from_before_machines_were_recorded_belongs_to_the_first_to_load_it() {
        let (_dir, path) = key_file();
        let original = Identity::generate();
        original.save(&path, None).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("machine"));

        // Looking at it does not change it.
        let here = machine(HERE);
        assert!(Identity::load_on(&path, Some(&here)).unwrap().is_some());
        assert!(!std::fs::read_to_string(&path).unwrap().contains("machine"));

        // Starting up with it records the machine.
        let loaded = Identity::load_or_create_on(&path, Some(&here)).unwrap();
        assert_eq!(loaded.device_id(), original.device_id());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains(here.as_str())
        );

        // From then on another machine is turned away.
        assert!(matches!(
            Identity::load_on(&path, Some(&machine(ELSEWHERE))),
            Err(Error::WrongMachine { .. })
        ));
    }

    #[test]
    fn a_machine_that_cannot_be_identified_is_not_checked() {
        let (_dir, path) = key_file();
        // A key made where the machine could not be told is not tied to one.
        let made = Identity::load_or_create_on(&path, None).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("machine"));
        let first = Identity::load_or_create_on(&path, Some(&machine(HERE))).unwrap();
        assert_eq!(first.device_id(), made.device_id());

        // A key that is tied to a machine still loads where the machine cannot be told.
        let before = std::fs::read(&path).unwrap();
        assert!(Identity::load_on(&path, None).unwrap().is_some());
        assert_eq!(
            Identity::load_or_create_on(&path, None)
                .unwrap()
                .device_id(),
            made.device_id()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn adopting_an_identity_moves_it_to_this_machine() {
        let (_dir, path) = key_file();
        let there = machine(ELSEWHERE);
        let here = machine(HERE);
        let made = Identity::load_or_create_on(&path, Some(&there)).unwrap();
        assert!(Identity::load_on(&path, Some(&here)).is_err());

        let adopted = Identity::adopt_on(&path, Some(&here)).unwrap().unwrap();
        assert_eq!(adopted.device_id(), made.device_id());
        assert!(Identity::load_on(&path, Some(&here)).unwrap().is_some());
        // And now it is the other machine that is turned away.
        assert!(matches!(
            Identity::load_on(&path, Some(&there)),
            Err(Error::WrongMachine { .. })
        ));

        let (_dir, missing) = key_file();
        assert!(Identity::adopt_on(&missing, Some(&here)).unwrap().is_none());
        assert!(!missing.exists());
    }

    #[test]
    fn the_real_machine_can_load_what_it_created() {
        let (_dir, path) = key_file();
        let made = Identity::load_or_create(&path).unwrap();
        assert_eq!(
            Identity::load(&path).unwrap().unwrap().device_id(),
            made.device_id()
        );
        assert_eq!(
            Identity::adopt_on_this_machine(&path)
                .unwrap()
                .unwrap()
                .device_id(),
            made.device_id()
        );
    }
}
