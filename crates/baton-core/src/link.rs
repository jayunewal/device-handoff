// SPDX-License-Identifier: Apache-2.0

//! What route a connection to another device is taking.

use std::net::{IpAddr, SocketAddr};

use iroh::endpoint::Connection;
use iroh_base::TransportAddr;

use crate::trust::{ContactHints, MAX_ADDR_HINTS};

/// The kind of path network traffic to a device is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathKind {
    /// Directly over the local network (or this machine).
    Lan,
    /// Directly over the internet, after hole punching.
    Direct,
    /// Through a relay server. Slower, but works when nothing else does.
    Relay,
    /// Not known yet.
    Unknown,
}

impl PathKind {
    /// A short word for the path, for lists and logs.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lan => "local network",
            Self::Direct => "direct",
            Self::Relay => "relay",
            Self::Unknown => "unknown",
        }
    }
}

/// Whether `ip` belongs to this machine or its local network rather than the internet.
pub(crate) fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

/// The path `connection` is sending on, and the peer's address if that path is direct.
pub(crate) fn selected_path(connection: &Connection) -> (PathKind, Option<SocketAddr>) {
    let paths = connection.paths();
    let Some(path) = paths.iter().find(iroh::endpoint::Path::is_selected) else {
        return (PathKind::Unknown, None);
    };
    match path.remote_addr() {
        TransportAddr::Ip(addr) if is_local(addr.ip()) => (PathKind::Lan, Some(*addr)),
        TransportAddr::Ip(addr) => (PathKind::Direct, Some(*addr)),
        TransportAddr::Relay(_) => (PathKind::Relay, None),
        _ => (PathKind::Unknown, None),
    }
}

/// Where `connection`'s peer can be reached again, as far as the paths in use show.
///
/// Both kinds of address are hints: the peer may have moved by the time they are
/// used, and the transport tries other ways of finding it as well.
pub(crate) fn contact_hints(connection: &Connection) -> ContactHints {
    let mut hints = ContactHints::default();
    for path in connection.paths().iter() {
        match path.remote_addr() {
            TransportAddr::Ip(addr) if is_dialable(addr.ip()) => {
                if hints.addrs.len() < MAX_ADDR_HINTS && !hints.addrs.contains(addr) {
                    hints.addrs.push(*addr);
                }
            }
            TransportAddr::Relay(url) if hints.relay.is_none() => {
                hints.relay = Some(url.to_string());
            }
            _ => {}
        }
    }
    hints
}

/// Whether an address means the same thing on another machine and survives being
/// written down without its scope.
pub(crate) fn is_dialable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_unspecified() && !v4.is_multicast() && !v4.is_broadcast(),
        // A link-local IPv6 address is only meaningful together with an interface,
        // which neither tickets nor the trust store record.
        IpAddr::V6(v6) => !v6.is_unspecified() && !v6.is_multicast() && !v6.is_unicast_link_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn only_addresses_that_work_elsewhere_are_dialable() {
        for fine in [
            "192.168.1.5",
            "8.8.8.8",
            "127.0.0.1",
            "::1",
            "fd00::5",
            "2606:4700::1111",
        ] {
            assert!(is_dialable(ip(fine)), "{fine}");
        }
        for useless in [
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "ff02::1",
            "fe80::1",
        ] {
            assert!(!is_dialable(ip(useless)), "{useless}");
        }
    }

    #[test]
    fn local_addresses_are_recognised() {
        for local in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "172.31.255.1",
            "192.168.1.20",
            "169.254.3.4",
            "::1",
            "fd12:3456::1",
            "fe80::1",
        ] {
            assert!(is_local(ip(local)), "{local}");
        }
    }

    #[test]
    fn internet_addresses_are_not_local() {
        for remote in [
            "8.8.8.8",
            "172.32.0.1",
            "100.64.0.1",
            "2001:db8::1",
            "2606:4700::1111",
        ] {
            assert!(!is_local(ip(remote)), "{remote}");
        }
    }
}
