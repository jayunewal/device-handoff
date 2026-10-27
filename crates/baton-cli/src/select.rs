// SPDX-License-Identifier: Apache-2.0

//! Choosing which paired device a command is about.

use baton_core::PairedDevice;

use crate::error::CliError;

/// How many characters of a fingerprint or device id must be given to match by them.
const MIN_FINGERPRINT_CHARS: usize = 4;
const MIN_ID_CHARS: usize = 8;

/// The device `query` names, or the only paired device when there is no query.
///
/// A query is tried against the devices in this order, and the first kind that matches
/// anything decides: the whole name, ignoring case; the start of the key's fingerprint
/// (`abcd` or `abcd-efgh`); the start of the device id in hexadecimal; the start of the
/// name. A query that matches more than one device in its kind is an error that lists
/// them, so a name two devices share is told apart by fingerprint.
pub fn choose(devices: &[PairedDevice], query: Option<&str>) -> Result<PairedDevice, CliError> {
    let Some(query) = query.map(str::trim) else {
        return match devices {
            [] => Err(none_paired()),
            [only] => Ok(only.clone()),
            many => Err(CliError::new(format!(
                "You have paired several devices, so say which one with --to: {}",
                list(many)
            ))),
        };
    };
    if query.is_empty() {
        return Err(CliError::new("The device name is empty."));
    }
    if devices.is_empty() {
        return Err(none_paired());
    }

    let lower = query.to_lowercase();
    let compact: String = lower
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    let by_name = |device: &PairedDevice| device.display_name().to_lowercase() == lower;
    let by_fingerprint = |device: &PairedDevice| {
        compact.chars().count() >= MIN_FINGERPRINT_CHARS
            && device
                .id
                .fingerprint()
                .to_string()
                .replace('-', "")
                .starts_with(&compact)
    };
    let by_id = |device: &PairedDevice| {
        compact.len() >= MIN_ID_CHARS && device.id.to_string().starts_with(&compact)
    };
    let by_name_prefix =
        |device: &PairedDevice| device.display_name().to_lowercase().starts_with(&lower);

    let kinds: [&dyn Fn(&PairedDevice) -> bool; 4] =
        [&by_name, &by_fingerprint, &by_id, &by_name_prefix];
    for matches in kinds {
        let found: Vec<&PairedDevice> = devices.iter().filter(|d| matches(d)).collect();
        match found.as_slice() {
            [] => {}
            [one] => return Ok((*one).clone()),
            many => {
                let many: Vec<PairedDevice> = many.iter().map(|d| (*d).clone()).collect();
                return Err(CliError::new(format!(
                    "More than one device matches \"{}\": {}. Use more of the fingerprint to tell them apart.",
                    crate::term::line(query),
                    list(&many)
                )));
            }
        }
    }
    Err(CliError::new(format!(
        "No paired device matches \"{}\". Paired devices: {}",
        crate::term::line(query),
        list(devices)
    )))
}

fn none_paired() -> CliError {
    CliError::new(
        "No devices are paired yet. Run `baton listen` on one computer and type `pair offer` \
         at its prompt, then run `baton pair join <ticket>` on the other.",
    )
}

/// `name (fingerprint), name (fingerprint)`.
fn list(devices: &[PairedDevice]) -> String {
    devices.iter().map(describe).collect::<Vec<_>>().join(", ")
}

/// `name (fingerprint)`: the name is only a label, so the fingerprint comes with it. The
/// name is one the device chose, so it is escaped for the terminal.
pub fn describe(device: &PairedDevice) -> String {
    format!(
        "{} ({})",
        crate::term::line(device.display_name()),
        device.id.fingerprint()
    )
}

#[cfg(test)]
mod tests {
    use baton_core::DevicePolicy;
    use baton_proto::{DeviceId, Platform};

    use super::*;

    fn device(n: u8, name: &str) -> PairedDevice {
        PairedDevice {
            id: DeviceId::from_bytes([n; 32]),
            name: name.to_owned(),
            local_name: Some(name.to_owned()),
            name_changed: false,
            platform: Platform::Linux,
            added_at: 0,
            provenance: baton_core::Provenance::Paired,
            last_seen_at: None,
            relay: None,
            addrs: Vec::new(),
            policy: DevicePolicy::default(),
            unlock: None,
        }
    }

    fn message(result: Result<PairedDevice, CliError>) -> String {
        result.unwrap_err().message().to_owned()
    }

    #[test]
    fn with_no_query_the_only_device_is_chosen() {
        let devices = [device(1, "Laptop")];
        assert_eq!(choose(&devices, None).unwrap().name, "Laptop");
    }

    #[test]
    fn with_no_query_and_no_device_the_message_says_how_to_pair() {
        let text = message(choose(&[], None));
        assert!(text.contains("baton listen"), "{text}");
        assert!(text.contains("pair offer"), "{text}");
        assert!(text.contains("baton pair join"), "{text}");
        assert_eq!(message(choose(&[], Some("x"))), text);
    }

    #[test]
    fn with_no_query_and_several_devices_they_are_listed() {
        let devices = [device(1, "Laptop"), device(2, "Phone")];
        let text = message(choose(&devices, None));
        assert!(text.contains("--to"), "{text}");
        assert!(text.contains("Laptop") && text.contains("Phone"), "{text}");
        assert!(
            text.contains(&devices[0].id.fingerprint().to_string()),
            "{text}"
        );
    }

    #[test]
    fn a_name_matches_whatever_its_case() {
        let devices = [device(1, "Jay's Laptop"), device(2, "Phone")];
        assert_eq!(
            choose(&devices, Some("jay's laptop")).unwrap().id,
            devices[0].id
        );
        assert_eq!(
            choose(&devices, Some("  PHONE ")).unwrap().id,
            devices[1].id
        );
    }

    #[test]
    fn the_start_of_a_name_matches_when_it_is_unambiguous() {
        let devices = [device(1, "Jay's Laptop"), device(2, "Phone")];
        assert_eq!(choose(&devices, Some("jay")).unwrap().id, devices[0].id);
        let both = [device(1, "Laptop A"), device(2, "Laptop B")];
        assert!(message(choose(&both, Some("laptop"))).contains("More than one"));
    }

    #[test]
    fn the_start_of_a_fingerprint_matches_with_or_without_dashes() {
        let devices = [device(1, "Same"), device(2, "Same")];
        let fingerprint = devices[1].id.fingerprint().to_string();
        let first_group = &fingerprint[..4];
        let two_groups = &fingerprint[..9];
        assert_eq!(
            choose(&devices, Some(first_group)).unwrap().id,
            devices[1].id
        );
        assert_eq!(
            choose(&devices, Some(two_groups)).unwrap().id,
            devices[1].id
        );
        assert_eq!(
            choose(&devices, Some(&two_groups.to_uppercase().replace('-', " ")))
                .unwrap()
                .id,
            devices[1].id
        );
    }

    #[test]
    fn a_shared_name_is_ambiguous_and_the_message_offers_the_fingerprints() {
        let devices = [device(1, "Same"), device(2, "Same")];
        let text = message(choose(&devices, Some("same")));
        assert!(text.contains("More than one"), "{text}");
        assert!(
            text.contains(&devices[0].id.fingerprint().to_string()),
            "{text}"
        );
        assert!(
            text.contains(&devices[1].id.fingerprint().to_string()),
            "{text}"
        );
    }

    #[test]
    fn the_start_of_the_id_matches() {
        let devices = [device(1, "A"), device(2, "B")];
        let id = devices[1].id.to_string();
        assert_eq!(choose(&devices, Some(&id[..8])).unwrap().id, devices[1].id);
        assert_eq!(choose(&devices, Some(&id)).unwrap().id, devices[1].id);
    }

    #[test]
    fn short_scraps_of_a_fingerprint_do_not_match() {
        let devices = [device(1, "A")];
        let fingerprint = devices[0].id.fingerprint().to_string();
        let text = message(choose(&devices, Some(&fingerprint[..3])));
        assert!(text.contains("No paired device matches"), "{text}");
    }

    #[test]
    fn an_unknown_name_lists_what_is_paired() {
        let devices = [device(1, "Laptop")];
        let text = message(choose(&devices, Some("tablet")));
        assert!(
            text.contains("No paired device matches \"tablet\""),
            "{text}"
        );
        assert!(text.contains("Laptop"), "{text}");
    }

    #[test]
    fn a_name_that_is_a_whole_name_beats_one_that_is_a_prefix() {
        let devices = [device(1, "Mac"), device(2, "Mac mini")];
        assert_eq!(choose(&devices, Some("mac")).unwrap().id, devices[0].id);
    }

    #[test]
    fn the_query_is_escaped_in_messages() {
        let devices = [device(1, "Laptop")];
        let text = message(choose(&devices, Some("x\u{1b}[2J")));
        assert!(!text.contains('\u{1b}'), "{text:?}");
    }

    #[test]
    fn a_name_is_escaped_when_a_device_is_described() {
        let devices = [device(1, "x\u{1b}[2J")];
        let text = describe(&devices[0]);
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert!(text.contains("\\u{1b}"), "{text:?}");
    }

    #[test]
    fn an_empty_query_is_refused() {
        let devices = [device(1, "Laptop")];
        assert!(message(choose(&devices, Some("  "))).contains("empty"));
    }
}
