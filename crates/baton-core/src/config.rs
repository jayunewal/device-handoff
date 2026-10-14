// SPDX-License-Identifier: Apache-2.0

//! The settings a person can change, kept in `config.toml`.
//!
//! Every setting has a default, and the file may be missing or hold only the
//! few lines someone cares about. Unknown keys are an error rather than being
//! ignored: a misspelled `lan_discovery` that silently left discovery switched
//! on would be worse than a refusal to start.

use std::path::Path;

use baton_proto::{
    device::{FALLBACK_NAME, sanitize_display_name},
    limits::MAX_CLIPBOARD_TEXT_BYTES,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    error::{Error, Result},
    fsutil::{Access, read_text},
};

/// How long a pairing ticket stays valid unless configured otherwise.
pub const DEFAULT_OFFER_TTL_SECS: u64 = 300;
/// Shortest ticket lifetime that can be configured.
pub const MIN_OFFER_TTL_SECS: u64 = 30;
/// Longest ticket lifetime that can be configured. A ticket is a bearer secret, so it stays short.
pub const MAX_OFFER_TTL_SECS: u64 = 900;

/// Items kept in the inbox unless configured otherwise.
pub const DEFAULT_MAX_INBOX_ITEMS: usize = 200;
/// Most inbox items that can be configured.
pub const MAX_INBOX_ITEMS_LIMIT: usize = 2_000;

/// Largest clipboard text accepted or sent unless configured otherwise.
pub const DEFAULT_MAX_CLIPBOARD_BYTES: usize = 256 * 1024;

/// Items accepted from one paired device per minute unless configured otherwise.
pub const DEFAULT_MAX_ITEMS_PER_MINUTE: u32 = 30;
/// Most items per minute that can be configured.
pub const MAX_ITEMS_PER_MINUTE_LIMIT: u32 = 600;

/// Most custom relays that can be listed.
pub const MAX_RELAY_URLS: usize = 8;
/// Longest relay URL, in bytes.
pub const MAX_RELAY_URL_BYTES: usize = baton_proto::ticket::MAX_RELAY_BYTES;

/// All settings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The name other devices see. Defaults to the computer's host name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    /// How devices find and reach each other.
    pub network: NetworkConfig,
    /// How pairing tickets behave.
    pub pairing: PairingConfig,
    /// Limits on what is accepted from and sent to other devices.
    pub handoff: HandoffConfig,
}

/// Which relays to use when two devices cannot connect directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelayMode {
    /// The public relays run by the iroh project.
    ///
    /// Fine for trying Baton out. According to iroh's documentation these relays are rate
    /// limited at a level it does not publish, and are meant for development and hobby use.
    /// A transfer that has to go through one, as phones behind carrier-grade NAT often do,
    /// runs at whatever that shared limit allows. The self-hostable server is planned to run
    /// a relay of its own, with limits its owner sets.
    #[default]
    Default,
    /// No relay: devices connect directly or not at all.
    Disabled,
    /// Only the relays listed in `relay_urls`.
    Custom,
}

/// Network settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Which relays to use as a fallback.
    pub relay: RelayMode,
    /// Relay servers to use when `relay` is `custom`.
    pub relay_urls: Vec<String>,
    /// Announce this device on the local network and look for paired devices there.
    ///
    /// What is announced is public: the device key and its addresses.
    pub lan_discovery: bool,
    /// Publish this device's relay to, and look paired devices' relays up from, the
    /// public iroh DNS service. Without it devices find each other only on the local
    /// network or through addresses remembered from pairing.
    ///
    /// Off by default. A record published under the device key lets anyone who has ever
    /// seen the key ask public DNS, at any later time, whether the device is online and
    /// roughly where. Paired devices already remember each other's relay, which is enough to
    /// reach them, and the lookup only helps after a peer's relay changes. Turn it on if
    /// paired devices stop finding each other after moving between networks. See
    /// `docs/security.md`.
    pub dns_discovery: bool,
    /// Ask the router to forward a port to this device (`UPnP`, NAT-PMP, PCP), which
    /// lets devices on other networks connect directly instead of through a relay.
    ///
    /// Turn it off if you do not want this device to ask your router to open a port.
    pub port_mapping: bool,
}

/// Pairing settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PairingConfig {
    /// How long a pairing ticket stays valid, in seconds.
    pub offer_ttl_secs: u64,
}

/// Limits on handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HandoffConfig {
    /// Most items kept in the inbox. The oldest are dropped first.
    ///
    /// A file transfer is one item, however many files it holds.
    pub max_inbox_items: usize,
    /// Largest clipboard text, in bytes, that is accepted from or sent to another device.
    pub max_clipboard_bytes: usize,
    /// Most items accepted from one paired device per minute.
    ///
    /// A whole file transfer counts as one item, not one per file or per chunk, so a folder
    /// of a thousand photos is not throttled. Transfers get limits of their own, a byte quota
    /// per device and a minimum of free disk space, which come with file transfer.
    pub max_items_per_minute: u32,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            relay: RelayMode::Default,
            relay_urls: Vec::new(),
            lan_discovery: true,
            dns_discovery: false,
            port_mapping: true,
        }
    }
}

impl Default for PairingConfig {
    fn default() -> Self {
        Self {
            offer_ttl_secs: DEFAULT_OFFER_TTL_SECS,
        }
    }
}

impl Default for HandoffConfig {
    fn default() -> Self {
        Self {
            max_inbox_items: DEFAULT_MAX_INBOX_ITEMS,
            max_clipboard_bytes: DEFAULT_MAX_CLIPBOARD_BYTES,
            max_items_per_minute: DEFAULT_MAX_ITEMS_PER_MINUTE,
        }
    }
}

impl Config {
    /// Reads the settings from `path`. A missing file means all defaults.
    ///
    /// A file that exists but is wrong is an error naming the file and the problem. On Unix
    /// it is also refused if other users can change it, since it decides which relays and
    /// discovery services this device uses.
    pub fn load(path: &Path) -> Result<Self> {
        let Some(text) = read_text(path, Access::OthersMayRead)? else {
            return Ok(Self::default());
        };
        Self::from_toml(&text).map_err(|e| match e {
            Error::Config(message) => Error::invalid(path, &message),
            other => other,
        })
    }

    /// Parses and checks settings written as TOML.
    pub fn from_toml(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).map_err(|e| Error::Config(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// The settings, including defaults, as TOML.
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))
    }

    /// Checks that the settings make sense together.
    pub fn validate(&self) -> Result<()> {
        if let Some(name) = &self.device_name
            && sanitize_display_name(name) == FALLBACK_NAME
            && name.trim() != FALLBACK_NAME
        {
            return Err(config_error(
                "device_name must contain at least one visible character",
            ));
        }
        self.network.validate()?;
        self.pairing.validate()?;
        self.handoff.validate()
    }

    /// The name to show other devices: the configured one, else the host name,
    /// cleaned up for display either way.
    pub fn device_name(&self) -> String {
        match &self.device_name {
            Some(name) => sanitize_display_name(name),
            None => sanitize_display_name(&gethostname::gethostname().to_string_lossy()),
        }
    }
}

impl NetworkConfig {
    fn validate(&self) -> Result<()> {
        match self.relay {
            RelayMode::Custom => {
                if self.relay_urls.is_empty() {
                    return Err(config_error(
                        "network.relay is \"custom\" but network.relay_urls is empty",
                    ));
                }
            }
            RelayMode::Default | RelayMode::Disabled => {
                if !self.relay_urls.is_empty() {
                    return Err(config_error(
                        "network.relay_urls is only used when network.relay is \"custom\"",
                    ));
                }
            }
        }
        // Parsing every entry is the check.
        self.parsed_relay_urls().map(drop)
    }

    /// The configured relay servers, parsed.
    ///
    /// Empty unless `relay` is `custom`. Each must be an `http` or `https` URL
    /// naming a host, without embedded credentials, short and plain ASCII: the
    /// value is copied into pairing tickets and the paired-device list.
    pub fn parsed_relay_urls(&self) -> Result<Vec<Url>> {
        if self.relay_urls.len() > MAX_RELAY_URLS {
            return Err(config_error(format!(
                "network.relay_urls lists {} relays, more than the limit of {MAX_RELAY_URLS}",
                self.relay_urls.len()
            )));
        }
        self.relay_urls
            .iter()
            .map(|raw| parse_relay_url(raw))
            .collect()
    }
}

impl PairingConfig {
    fn validate(&self) -> Result<()> {
        check_range(
            "pairing.offer_ttl_secs",
            self.offer_ttl_secs,
            MIN_OFFER_TTL_SECS..=MAX_OFFER_TTL_SECS,
        )
    }
}

impl HandoffConfig {
    fn validate(&self) -> Result<()> {
        check_range(
            "handoff.max_inbox_items",
            self.max_inbox_items,
            1..=MAX_INBOX_ITEMS_LIMIT,
        )?;
        check_range(
            "handoff.max_clipboard_bytes",
            self.max_clipboard_bytes,
            1..=MAX_CLIPBOARD_TEXT_BYTES,
        )?;
        check_range(
            "handoff.max_items_per_minute",
            self.max_items_per_minute,
            1..=MAX_ITEMS_PER_MINUTE_LIMIT,
        )
    }
}

fn parse_relay_url(raw: &str) -> Result<Url> {
    let bad = |why: &str| config_error(format!("network.relay_urls: {raw:?} {why}"));
    if raw.len() > MAX_RELAY_URL_BYTES {
        return Err(bad("is too long"));
    }
    if !raw.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(bad(
            "must be plain ASCII without spaces (write internationalized names in punycode)",
        ));
    }
    let url = Url::parse(raw).map_err(|e| bad(&format!("is not a valid URL: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(bad("must start with http:// or https://"));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(bad("has no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad("must not contain a user name or password"));
    }
    Ok(url)
}

fn check_range<T>(name: &str, value: T, range: std::ops::RangeInclusive<T>) -> Result<()>
where
    T: PartialOrd + std::fmt::Display + Copy,
{
    if range.contains(&value) {
        Ok(())
    } else {
        Err(config_error(format!(
            "{name} is {value}, but must be between {} and {}",
            range.start(),
            range.end()
        )))
    }
}

fn config_error(message: impl Into<String>) -> Error {
    Error::Config(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_cautious() {
        let config = Config::default();
        config.validate().unwrap();
        assert_eq!(config.network.relay, RelayMode::Default);
        assert!(config.network.lan_discovery);
        assert!(
            !config.network.dns_discovery,
            "publishing under the device key is a choice, not a default"
        );
        assert_eq!(config.pairing.offer_ttl_secs, 300);
        assert!(config.handoff.max_clipboard_bytes <= MAX_CLIPBOARD_TEXT_BYTES);
    }

    #[test]
    fn an_empty_file_is_all_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn defaults_round_trip_through_toml() {
        let config = Config::default();
        let text = config.to_toml().unwrap();
        assert_eq!(Config::from_toml(&text).unwrap(), config);
    }

    #[test]
    fn a_full_config_round_trips() {
        let config = Config {
            device_name: Some("Work laptop".into()),
            network: NetworkConfig {
                relay: RelayMode::Custom,
                relay_urls: vec!["https://relay.example.org/".into()],
                lan_discovery: false,
                dns_discovery: true,
                port_mapping: false,
            },
            pairing: PairingConfig {
                offer_ttl_secs: 120,
            },
            handoff: HandoffConfig {
                max_inbox_items: 10,
                max_clipboard_bytes: 1024,
                max_items_per_minute: 5,
            },
        };
        let text = config.to_toml().unwrap();
        assert_eq!(Config::from_toml(&text).unwrap(), config);
    }

    #[test]
    fn partial_files_fill_in_the_rest() {
        let config = Config::from_toml("[network]\nlan_discovery = false\n").unwrap();
        assert!(!config.network.lan_discovery);
        assert!(!config.network.dns_discovery);
        assert_eq!(config.pairing, PairingConfig::default());
    }

    #[test]
    fn misspelled_keys_are_rejected() {
        for text in [
            "lan_discovery = false",
            "[network]\nlan_discovry = false",
            "[pairing]\nrequire_approval = false",
            "[handoff]\nmax_inbox = 3",
            "[mystery]\nx = 1",
        ] {
            assert!(Config::from_toml(text).is_err(), "accepted: {text}");
        }
    }

    #[test]
    fn wrong_types_are_rejected() {
        assert!(Config::from_toml("[network]\nlan_discovery = \"no\"").is_err());
        assert!(Config::from_toml("[pairing]\noffer_ttl_secs = -5").is_err());
        assert!(Config::from_toml("[network]\nrelay = \"everywhere\"").is_err());
    }

    #[test]
    fn custom_relays_need_urls_and_other_modes_forbid_them() {
        assert!(Config::from_toml("[network]\nrelay = \"custom\"").is_err());
        assert!(
            Config::from_toml("[network]\nrelay_urls = [\"https://r.example/\"]").is_err(),
            "urls without custom mode"
        );
        assert!(
            Config::from_toml(
                "[network]\nrelay = \"disabled\"\nrelay_urls = [\"https://r.example/\"]"
            )
            .is_err()
        );
        let ok = Config::from_toml(
            "[network]\nrelay = \"custom\"\nrelay_urls = [\"https://r.example/\", \"http://127.0.0.1:3340\"]",
        )
        .unwrap();
        assert_eq!(ok.network.parsed_relay_urls().unwrap().len(), 2);
    }

    #[test]
    fn relay_urls_are_checked_strictly() {
        let long = format!("https://{}.example/", "a".repeat(300));
        let bad = [
            "ftp://r.example/",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://user:pass@r.example/",
            "https://user@r.example/",
            "not a url",
            "https://",
            "https://bücher.example/",
            "https://r.example/ x",
            long.as_str(),
        ];
        for url in bad {
            let text = format!("[network]\nrelay = \"custom\"\nrelay_urls = [{url:?}]");
            assert!(Config::from_toml(&text).is_err(), "accepted: {url}");
        }
    }

    #[test]
    fn too_many_relays_are_rejected() {
        let urls: Vec<_> = (0..=MAX_RELAY_URLS)
            .map(|i| format!("\"https://r{i}.example/\""))
            .collect();
        let text = format!(
            "[network]\nrelay = \"custom\"\nrelay_urls = [{}]",
            urls.join(",")
        );
        assert!(Config::from_toml(&text).is_err());
    }

    #[test]
    fn numbers_must_be_in_range() {
        for text in [
            "[pairing]\noffer_ttl_secs = 5",
            "[pairing]\noffer_ttl_secs = 100000",
            "[handoff]\nmax_inbox_items = 0",
            "[handoff]\nmax_inbox_items = 100000",
            "[handoff]\nmax_clipboard_bytes = 0",
            "[handoff]\nmax_clipboard_bytes = 99999999",
            "[handoff]\nmax_items_per_minute = 0",
            "[handoff]\nmax_items_per_minute = 100000",
        ] {
            assert!(Config::from_toml(text).is_err(), "accepted: {text}");
        }
        // Boundaries are inclusive.
        Config::from_toml("[pairing]\noffer_ttl_secs = 30").unwrap();
        Config::from_toml("[pairing]\noffer_ttl_secs = 900").unwrap();
    }

    #[test]
    fn device_names_are_cleaned_and_must_be_visible() {
        let config = Config::from_toml("device_name = \"Ann\\u202Es laptop\"").unwrap();
        assert_eq!(config.device_name(), "Anns laptop");
        assert!(Config::from_toml("device_name = \"\"").is_err());
        assert!(Config::from_toml("device_name = \"   \"").is_err());
        assert!(Config::from_toml("device_name = \"\\u202E\"").is_err());
    }

    #[test]
    fn the_default_name_is_never_empty() {
        let name = Config::default().device_name();
        assert_ne!(name, "");
        assert!(name.chars().count() <= baton_proto::device::MAX_DISPLAY_NAME_CHARS);
    }

    #[test]
    fn loading_handles_missing_and_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(Config::load(&path).unwrap(), Config::default());

        std::fs::write(&path, "[pairing]\noffer_ttl_secs = 1\n").unwrap();
        let err = Config::load(&path).unwrap_err().to_string();
        assert!(err.contains("config.toml"), "{err}");
        assert!(err.contains("offer_ttl_secs"), "{err}");

        std::fs::write(&path, "[pairing]\noffer_ttl_secs = 60\n").unwrap();
        assert_eq!(Config::load(&path).unwrap().pairing.offer_ttl_secs, 60);
    }

    #[cfg(unix)]
    #[test]
    fn a_settings_file_that_other_users_can_change_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[pairing]\noffer_ttl_secs = 60\n").unwrap();

        // A text editor tends to leave a file readable by others, which is fine for settings.
        for ok in [0o600, 0o644] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(ok)).unwrap();
            assert!(Config::load(&path).is_ok(), "{ok:o}");
        }
        // Changeable by others is not: they could point this device at their own relay.
        for bad in [0o664, 0o646, 0o666] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(bad)).unwrap();
            let err = Config::load(&path).unwrap_err();
            assert!(
                matches!(err, Error::WritableByOthers { .. }),
                "{bad:o}: {err}"
            );
        }
    }
}
