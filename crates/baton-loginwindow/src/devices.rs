// SPDX-License-Identifier: Apache-2.0

//! The devices that are paired with the program: reading the list, and taking a device off it.
//!
//! The list is the program's own. A device that the person removes in the Baton app is still on
//! it, and so is one that is forgotten at the phone: only `forget`, here, and the remove script
//! end the trust (`docs/adr/0019-baton-at-the-login-window.md`).

use baton_app::{Device, StartAccess};
use baton_core::{PairedDevice, TrustStore};
use baton_proto::device::sanitize_display_name;

use crate::{Error, ErrorKind, Result, State};

/// The fewest characters of an id that `forget` takes, so that a slip of the keyboard does not
/// name a device by a letter or two.
const MIN_ID_CHARS: usize = 6;

/// A paired device, and what it may do here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The device.
    pub device: Device,
    /// What it may ask for here. The pairing sets it to [`StartAccess::Control`], and nothing in
    /// the program changes it.
    pub start: StartAccess,
}

impl Listed {
    /// The device as the person is shown it: its name, which it chose and which tells nothing of
    /// who it is, with the fingerprint and the id that name it.
    #[must_use]
    pub fn describe(&self) -> String {
        let may = match self.start {
            StartAccess::Off => "nothing",
            StartAccess::Look => "look at the login window",
            StartAccess::Control => "look at the login window and use it",
        };
        format!(
            "{}\n    id            {}\n    fingerprint   {}\n    may           {may}",
            sanitize_display_name(&self.device.name),
            self.device.id,
            self.device.fingerprint,
        )
    }
}

impl From<&PairedDevice> for Listed {
    fn from(device: &PairedDevice) -> Self {
        Self {
            device: Device::from(device),
            start: StartAccess::from(device.policy.start),
        }
    }
}

/// The devices that are paired with the program, by name. Reading the list needs no lock, so it
/// works while the program runs.
///
/// # Errors
///
/// [`ErrorKind::Other`] if the list cannot be read.
pub fn list(state: &State) -> Result<Vec<Listed>> {
    let trust = TrustStore::load(state.dir().trust_file()).map_err(Error::from)?;
    Ok(trust.list().iter().map(Listed::from).collect())
}

/// Takes a device off the list: the one whose id starts with `which`. It cannot open the login
/// window after this, and has to pair again. A session that is running is not ended, which is no
/// matter here: the program is not running when this can be done.
///
/// # Errors
///
/// [`ErrorKind::NoDevice`] if `which` is too short, or names no device or more than one, and
/// [`ErrorKind::Busy`] if the program is running with the folder.
pub fn forget(state: &State, which: &str) -> Result<Device> {
    let which = which.trim().to_ascii_lowercase();
    if which.chars().count() < MIN_ID_CHARS {
        return Err(Error::new(
            ErrorKind::NoDevice,
            format!(
                "Give at least {MIN_ID_CHARS} characters of the id of the device, as \
                 `baton-loginwindow devices` shows it."
            ),
        ));
    }
    // The lock keeps the program that serves from reading the list while it is changed, and
    // from keeping a copy in memory that this would not reach.
    let _lock = state.dir().lock().map_err(|error| match error {
        baton_core::Error::Locked { .. } => Error::new(
            ErrorKind::Busy,
            "Baton for the login window is running with this folder, so the list cannot be \
             changed now. Stop it first (the remove script does), and run this again.",
        ),
        other => Error::from(other),
    })?;
    let trust = TrustStore::load(state.dir().trust_file()).map_err(Error::from)?;
    let found: Vec<PairedDevice> = trust
        .list()
        .into_iter()
        .filter(|device| device.id.to_string().starts_with(&which))
        .collect();
    match found.as_slice() {
        [] => Err(Error::new(
            ErrorKind::NoDevice,
            "No paired device has an id that starts like that. `baton-loginwindow devices` lists \
             them.",
        )),
        [device] => {
            trust.remove(&device.id).map_err(Error::from)?;
            Ok(Device::from(device))
        }
        _ => Err(Error::new(
            ErrorKind::NoDevice,
            "More than one paired device has an id that starts like that. Give more characters.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use baton_app::Platform;
    use baton_proto::{
        DeviceId,
        device::{DeviceInfo, Platform as ProtoPlatform},
    };
    use tempfile::TempDir;

    use super::*;

    /// A folder whose list has a device for each of `lasts`: ids that are the same but for their
    /// last byte.
    fn state_with(lasts: &[u8]) -> (TempDir, State) {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(Some(dir.path().join("state"))).unwrap();
        let trust = TrustStore::load(state.dir().trust_file()).unwrap();
        for last in lasts {
            let mut bytes = [0xab_u8; DeviceId::LEN];
            bytes[DeviceId::LEN - 1] = *last;
            let info = DeviceInfo {
                name: format!("Phone {last}"),
                platform: ProtoPlatform::Ios,
                app_version: "0.1.0".to_owned(),
            };
            trust.pair(DeviceId::from_bytes(bytes), &info, 1).unwrap();
        }
        (dir, state)
    }

    fn listed(name: &str, start: StartAccess) -> Listed {
        Listed {
            device: Device {
                id: "ab".repeat(DeviceId::LEN),
                name: name.to_owned(),
                platform: Platform::Ios,
                fingerprint: "abcd-efgh-ijkl-mnop-qrst".to_owned(),
                paired_at: 1,
                last_seen_at: None,
                local_addresses: Vec::new(),
            },
            start,
        }
    }

    #[test]
    fn a_device_is_described_by_what_it_may_do() {
        for (start, may) in [
            (StartAccess::Off, "nothing"),
            (StartAccess::Look, "look at the login window"),
            (StartAccess::Control, "look at the login window and use it"),
        ] {
            let description = listed("Phone", start).describe();
            assert!(
                description.ends_with(&format!("may           {may}")),
                "{description}"
            );
            assert!(
                description.contains("abcd-efgh-ijkl-mnop-qrst"),
                "{description}"
            );
        }
    }

    #[test]
    fn what_a_device_calls_itself_cannot_change_the_screen_of_the_terminal() {
        let description = listed("Phone\u{1b}[2J\r\nid  forged", StartAccess::Control).describe();

        assert!(
            !description.chars().any(|c| c.is_control() && c != '\n'),
            "{description:?}"
        );
        assert_eq!(
            description
                .lines()
                .filter(|line| line.trim_start().starts_with("id "))
                .count(),
            1,
            "the name started a line of its own: {description:?}"
        );
    }

    #[test]
    fn a_start_of_an_id_that_two_devices_share_names_neither() {
        let (_dir, state) = state_with(&[1, 2]);
        let ids: Vec<String> = list(&state)
            .unwrap()
            .iter()
            .map(|l| l.device.id.clone())
            .collect();
        assert_eq!(ids.len(), 2);

        let error = forget(&state, &ids[0][..40]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NoDevice, "{error}");
        assert!(error.message().contains("More than one"), "{error}");
        assert_eq!(list(&state).unwrap().len(), 2, "a device was forgotten");

        // The whole id names one of them, and leaves the other.
        let gone = forget(&state, &ids[0]).unwrap();
        assert_eq!(gone.id, ids[0]);
        let left = list(&state).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].device.id, ids[1]);
    }
}
