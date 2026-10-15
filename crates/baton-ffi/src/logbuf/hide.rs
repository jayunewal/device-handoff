// SPDX-License-Identifier: Apache-2.0

//! What is taken out of a log line before the app shows it, for the parts of a line that name
//! a person's network or computer: long keys, the account name in a path and public addresses.
//!
//! The lines are copied into bug reports on a public page, so a line should say what went wrong
//! and where on this device it went wrong, and not who this device belongs to or where it is.
//! Each function gives back the line it was given, borrowed, when there is nothing to take out.

use std::{
    borrow::Cow,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

/// A copy of a line in which some stretches are replaced. The line is copied only when the first
/// one is.
struct Edits<'a> {
    line: &'a str,
    copy: Option<String>,
    done: usize,
}

impl<'a> Edits<'a> {
    const fn new(line: &'a str) -> Self {
        Self {
            line,
            copy: None,
            done: 0,
        }
    }

    /// Replaces `line[start..end]` by `with`. The stretches come in order and do not overlap, and
    /// both ends are on character boundaries.
    fn replace(&mut self, start: usize, end: usize, with: &str) {
        let copy = self
            .copy
            .get_or_insert_with(|| String::with_capacity(self.line.len()));
        copy.push_str(&self.line[self.done..start]);
        copy.push_str(with);
        self.done = end;
    }

    fn finish(self) -> Cow<'a, str> {
        match self.copy {
            None => Cow::Borrowed(self.line),
            Some(mut copy) => {
                copy.push_str(&self.line[self.done..]);
                Cow::Owned(copy)
            }
        }
    }
}

/// How many hexadecimal digits in a row make a key. A device's identity is 32 bytes, which is
/// 64 digits, and so is a hash of one.
const KEY_DIGITS: usize = 64;

/// How many digits of a key are kept: the same as `DeviceId::short`, which is what the rest of
/// the log calls a device.
const KEY_KEPT: usize = 10;

/// `line` with every run of [`KEY_DIGITS`] or more hexadecimal digits cut to its first
/// [`KEY_KEPT`] and `…`. With the address of the relay, a device's whole key is enough to dial
/// it, and a key that never changes lets anyone who sees it again tell that it is the same
/// device. Baton's own lines log the short form. This catches a whole key that got into a line
/// some other way, such as in an error message or a warning of a library.
pub(super) fn shorten_keys(line: &str) -> Cow<'_, str> {
    let bytes = line.as_bytes();
    let mut edits = Edits::new(line);
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_hexdigit() {
            at += 1;
            continue;
        }
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_hexdigit) {
            at += 1;
        }
        if at - start >= KEY_DIGITS {
            edits.replace(start + KEY_KEPT, at, "…");
        }
    }
    edits.finish()
}

/// `line` with the account name in a path replaced by `~`: the folder after `/Users/` or
/// `/home/`, or after `\Users\` (also as it looks when a path is printed with `{:?}`, with each
/// backslash doubled). The folder of the app is under the account's home folder, and an account
/// is often named after a person.
pub(super) fn hide_account_names(line: &str) -> Cow<'_, str> {
    let mut edits = Edits::new(line);
    let mut at = 0;
    while at < line.len() {
        let Some((marker, windows)) = home_folder(&line[at..]) else {
            at += line[at..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let start = at + marker;
        let name = line[start..]
            .find(|c: char| ends_account_name(c, windows))
            .unwrap_or(line.len() - start);
        if name > 0 && &line[start..start + name] != "~" {
            edits.replace(start, start + name, "~");
        }
        at = start + name;
    }
    edits.finish()
}

/// How long the start of `text` is if it ends in the folder that holds the accounts, and whether
/// it is the Windows kind.
fn home_folder(text: &str) -> Option<(usize, bool)> {
    for marker in ["/Users/", "/home/"] {
        if text.starts_with(marker) {
            return Some((marker.len(), false));
        }
    }
    let before = text.len() - text.trim_start_matches('\\').len();
    let rest = text[before..].strip_prefix("Users")?;
    let after = rest.len() - rest.trim_start_matches('\\').len();
    (before > 0 && after > 0).then_some((before + "Users".len() + after, true))
}

/// Whether the name of an account ends before `c`. A Windows name can hold spaces, and a
/// Unix one ends at the end of the word.
fn ends_account_name(c: char, windows: bool) -> bool {
    matches!(c, '/' | '\\' | '"' | '\'')
        || (!windows && (c.is_whitespace() || matches!(c, ')' | ']' | ',' | ';')))
}

/// `line` with each public IP address replaced by its kind, as in `public IPv4 (hidden)`. The
/// port stays, because it helps and says nothing about where a device is: `public IPv4
/// (hidden):41641`. Addresses that belong to no one network in particular (this host, a private
/// or shared network, link-local, multicast and the ones set aside for documents) stay as they
/// are. They answer most questions about why a device cannot be reached.
///
/// A public address names the network the device is on, and with the port mapping that Baton
/// asks the router for, a port on the router that leads to the device. Candidates are parsed as
/// addresses and not matched by their look, because a time (`08:00:00`) or a version (`1.3.0`)
/// looks like part of one. An IPv6 address counts as public only inside `2000::/3`, where networks
/// are given addresses (and `64:ff9b::/96` by the IPv4 address it holds): `baton_core::node` holds
/// `e::`, and `baton_ffi::api` holds `::a`, which are addresses to the parser and are outside it.
/// A candidate that is joined to a letter or a digit on either side is a piece of a word and is
/// left alone as well. The first and the last address of a list printed with `{:?}` are found
/// although a bracket is joined to them.
pub(super) fn hide_public_addresses(line: &str) -> Cow<'_, str> {
    let bytes = line.as_bytes();
    let mut edits = Edits::new(line);
    let mut at = 0;
    while at < bytes.len() {
        if !is_address_byte(bytes[at]) {
            at += 1;
            continue;
        }
        let start = at;
        while bytes.get(at).copied().is_some_and(is_address_byte) {
            at += 1;
        }
        let run = &line[start..at];
        // The run may hold the full stop or the colon of the sentence it stands in, or the colon
        // that comes before it. `::1` begins with colons, so the whole run is tried first.
        let to = run.trim_end_matches(['.', ':', '%', '[']).len();
        let from = to - run[..to].trim_start_matches(['.', ':', '%', ']']).len();
        // A list printed with `{:?}`, as in `[9.9.9.9, 1.1.1.1]`, puts a bracket before its first
        // address and after its last. The brackets of an address with a port belong to it, so
        // the stretches without them come after the ones above, and a single bracket is cut
        // first: `[[2606:4700::1]:2` is the first item of a list of such addresses, and
        // `[2606:4700::1]:2]` is the last.
        let opener = usize::from(run.starts_with('['));
        let closer = run.len() - usize::from(run.ends_with(']'));
        let hi = run.trim_end_matches(['.', ':', '%', '[', ']']).len();
        let lo = hi
            - run[..hi]
                .trim_start_matches(['.', ':', '%', '[', ']'])
                .len();
        for (first, last) in [
            (0, run.len()),
            (0, to),
            (from, to),
            (opener, run.len()),
            (0, closer),
            (opener, closer),
            (lo, hi),
        ] {
            let (begin, end) = (start + first, start + last);
            if is_part_of_a_word(line, begin, end) {
                continue;
            }
            if let Some(shown) = public_address(&line[begin..end]) {
                edits.replace(begin, end, &shown);
                break;
            }
        }
    }
    edits.finish()
}

/// Whether `line[start..end]` is joined to a letter or a digit on either side. A colon or a
/// bracket between a word and an address, as in `peer:8.8.8.8`, is not part of the word: the
/// stretch that is tried without the colon is not joined to anything.
fn is_part_of_a_word(line: &str, start: usize, end: usize) -> bool {
    line[..start]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
        || line[end..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric)
}

/// Whether `byte` can be part of an IP address, a port or a zone, as it is written.
const fn is_address_byte(byte: u8) -> bool {
    byte.is_ascii_hexdigit() || matches!(byte, b':' | b'.' | b'[' | b']' | b'%')
}

/// What to show for `text` if it is a public address, with or without a port.
fn public_address(text: &str) -> Option<String> {
    let (ip, port) = if let Ok(socket) = text.parse::<SocketAddr>() {
        (socket.ip(), Some(socket.port()))
    } else {
        // An address on its own, in brackets or not, with the zone that a link-local one can
        // carry cut off.
        let bare = text
            .strip_prefix('[')
            .and_then(|text| text.strip_suffix(']'))
            .unwrap_or(text);
        let bare = bare
            .split_once('%')
            .map_or(bare, |(address, _zone)| address);
        (bare.parse::<IpAddr>().ok()?, None)
    };
    if !is_public(ip) {
        return None;
    }
    let kind = if ip.is_ipv4() { "IPv4" } else { "IPv6" };
    Some(match port {
        Some(port) => format!("public {kind} (hidden):{port}"),
        None => format!("public {kind} (hidden)"),
    })
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        // `::ffff:8.8.8.8` is an IPv4 address in an IPv6 form.
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map_or_else(|| is_public_v6(ip), is_public_v4),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();
    // The range for carrier-grade NAT, where VPNs such as Tailscale put their addresses.
    let shared = first == 100 && (64..128).contains(&second);
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || shared)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let [first, second, third, fourth, fifth, sixth, high, low] = ip.segments();
    // `64:ff9b::/96` is how an IPv6-only mobile network reaches IPv4 addresses. The address holds
    // one in its last 32 bits and is as public as that one, as `::ffff:0:0/96` is.
    if (first, second, third, fourth, fifth, sixth) == (0x64, 0xff9b, 0, 0, 0, 0) {
        return is_public_v4(Ipv4Addr::from((u32::from(high) << 16) | u32::from(low)));
    }
    // Only `2000::/3` is handed out to networks. Everything else is this host, a private network,
    // link-local, multicast or not assigned, and so are the short forms that the names of modules
    // make: `e::` in `core::` and `::a` in `::api`.
    let global = first & 0xe000 == 0x2000;
    // `2001:db8::/32` and `3fff::/20` are set aside for documents.
    let documentation =
        (first == 0x2001 && second == 0x0db8) || (first == 0x3fff && second & 0xf000 == 0);
    global && !documentation
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 64 hexadecimal digits, as a device's key is written.
    const KEY: &str = "b0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecf";

    #[test]
    fn a_whole_key_is_cut_to_the_short_form_of_a_device() {
        assert_eq!(
            shorten_keys(&format!("created a new identity device_id={KEY} ok")),
            "created a new identity device_id=b0b1b2b3b4… ok"
        );
        assert_eq!(
            shorten_keys(&format!("PublicKey({KEY}) and {KEY}")),
            "PublicKey(b0b1b2b3b4…) and b0b1b2b3b4…"
        );
    }

    #[test]
    fn a_key_in_capitals_or_one_that_runs_on_is_cut_too() {
        assert_eq!(shorten_keys(&KEY.to_uppercase()), "B0B1B2B3B4…");
        // 64 digits and more, such as a signature.
        assert_eq!(shorten_keys(&format!("{KEY}{KEY} end")), "b0b1b2b3b4… end");
    }

    #[test]
    fn what_is_shorter_than_a_key_stays() {
        let short = &KEY[..63];
        let line = format!("a short id b0b1b2b3b4, a hash {short}, and the word deadbeef");
        assert!(matches!(shorten_keys(&line), Cow::Borrowed(_)));
        // One that stops at a character that is not a digit is not joined to what follows.
        let split = format!("{}-{}", &KEY[..40], &KEY[..40]);
        assert_eq!(shorten_keys(&split), split);
    }

    #[test]
    fn the_account_name_in_a_path_becomes_a_tilde() {
        assert_eq!(
            hide_account_names(
                "could not create /Users/anna/Library/Containers/io.github.jayunewal.baton/Data: denied"
            ),
            "could not create /Users/~/Library/Containers/io.github.jayunewal.baton/Data: denied"
        );
        assert_eq!(
            hide_account_names("error=\"/home/anna.b/baton-app/identity\" and /home/zoe, ok"),
            "error=\"/home/~/baton-app/identity\" and /home/~, ok"
        );
    }

    #[test]
    fn a_windows_account_name_goes_too_with_its_spaces() {
        assert_eq!(
            hide_account_names(r"C:\Users\Anna Berg\AppData\Local\baton"),
            r"C:\Users\~\AppData\Local\baton"
        );
        // As a path looks when it is printed with {:?}.
        assert_eq!(
            hide_account_names(r#"path: "C:\\Users\\Anna Berg\\AppData""#),
            r#"path: "C:\\Users\\~\\AppData""#
        );
        assert_eq!(
            hide_account_names("C:/Users/anna/AppData"),
            "C:/Users/~/AppData"
        );
    }

    #[test]
    fn a_path_that_names_no_account_stays() {
        for line in [
            "/private/var/mobile/Containers/Data/Application/1234/Library",
            "/Users/",
            "/Users/~/Library",
            "the Users folder and /home alone",
            r"C:\Users\",
        ] {
            assert!(
                matches!(hide_account_names(line), Cow::Borrowed(_)),
                "{line}"
            );
        }
    }

    #[test]
    fn a_name_with_letters_of_other_alphabets_is_cut_on_a_character_boundary() {
        assert_eq!(hide_account_names("Zoë’s /home/é/x"), "Zoë’s /home/~/x");
        assert_eq!(hide_account_names("/Users/Zoë"), "/Users/~");
    }

    #[test]
    fn a_public_address_is_shown_by_its_kind_and_keeps_its_port() {
        for (line, shown) in [
            ("8.8.8.8:53", "public IPv4 (hidden):53"),
            ("1.1.1.1", "public IPv4 (hidden)"),
            ("[2606:4700:4700::1111]:443", "public IPv6 (hidden):443"),
            ("2606:4700:4700::1111", "public IPv6 (hidden)"),
            ("[2606:4700:4700::1111]", "public IPv6 (hidden)"),
            ("::ffff:8.8.8.8", "public IPv6 (hidden)"),
            ("[::ffff:8.8.8.8]:99", "public IPv6 (hidden):99"),
            // An IPv6-only mobile network reaches an IPv4 address through `64:ff9b::/96`.
            ("64:ff9b::808:808", "public IPv6 (hidden)"),
            ("[64:ff9b::8.8.8.8]:443", "public IPv6 (hidden):443"),
            // The ends of `2000::/3`, and an address that a 6to4 gateway made.
            ("2000::1", "public IPv6 (hidden)"),
            ("[3fff:1000::1]:7", "public IPv6 (hidden):7"),
            (
                "3fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
                "public IPv6 (hidden)",
            ),
            ("2002:808:808::1", "public IPv6 (hidden)"),
        ] {
            assert_eq!(hide_public_addresses(line), shown, "{line}");
        }
    }

    #[test]
    fn addresses_that_belong_to_no_one_network_stay() {
        for line in [
            "127.0.0.1:41641",
            "[::1]:80",
            "0.0.0.0:0",
            "[::]:0",
            "10.1.2.3",
            "172.16.0.1:9",
            "172.31.255.254",
            "192.168.1.20:41641",
            "100.64.0.1",
            "100.127.255.254:1",
            "169.254.10.1",
            "[fe80::1%2]:41641",
            "fe80::abcd:1",
            "fc00::1",
            "[fd12:3456:789a::1]:5",
            "224.0.0.251:5353",
            "[ff02::fb]:5353",
            "255.255.255.255",
            "192.0.2.7",
            "2001:db8::1",
            "3fff::1",
            "[3fff:fff:ffff::1]:5",
            "::ffff:192.168.1.5",
            // `64:ff9b::/96` holds the IPv4 address that it reaches, and that one is not public.
            "64:ff9b::a00:1",
            "[64:ff9b::192.168.1.5]:9",
            "64:ff9b::7f00:1",
            // Outside `2000::/3` no network is given an address, just below it and just above.
            "1fff:ffff::1",
            "4000::1",
        ] {
            assert!(
                matches!(hide_public_addresses(line), Cow::Borrowed(_)),
                "{line}"
            );
        }
    }

    #[test]
    fn the_edges_of_the_private_ranges_are_public_on_the_other_side() {
        for address in [
            "172.15.255.255",
            "172.32.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "11.0.0.1",
            "9.255.255.255",
            "169.253.1.1",
            "192.169.0.1",
        ] {
            assert_eq!(
                hide_public_addresses(address),
                "public IPv4 (hidden)",
                "{address}"
            );
        }
    }

    #[test]
    fn addresses_are_found_in_the_middle_of_a_line() {
        assert_eq!(
            hide_public_addresses(
                "peer at 8.8.8.8:53, then 192.168.1.2:5 and 1.1.1.1. Ip(9.9.9.9:1) [2606:4700::1]:2;"
            ),
            "peer at public IPv4 (hidden):53, then 192.168.1.2:5 and public IPv4 (hidden). \
             Ip(public IPv4 (hidden):1) public IPv6 (hidden):2;"
        );
        assert_eq!(
            hide_public_addresses("addr=8.8.8.8:5353 ip:1.1.1.1 to:[2606:4700::1]: done"),
            "addr=public IPv4 (hidden):5353 ip:public IPv4 (hidden) to:public IPv6 (hidden): done"
        );
        assert_eq!(
            hide_public_addresses("connect https://8.8.8.8:8080/path failed"),
            "connect https://public IPv4 (hidden):8080/path failed"
        );
    }

    #[test]
    fn the_first_and_the_last_address_of_a_printed_list_are_found_too() {
        // A list printed with `{:?}` starts with a bracket and ends with one, and the stretch that
        // holds its first or its last address holds the bracket as well.
        for (line, shown) in [
            (
                "addrs=[9.9.9.9:41641, 10.0.0.7:41641]",
                "addrs=[public IPv4 (hidden):41641, 10.0.0.7:41641]",
            ),
            (
                "[9.9.9.9, 1.1.1.1]",
                "[public IPv4 (hidden), public IPv4 (hidden)]",
            ),
            ("[2606:4700::1, fe80::1]", "[public IPv6 (hidden), fe80::1]"),
            (
                "[[2606:4700::1]:2, [fe80::1%2]:3]",
                "[public IPv6 (hidden):2, [fe80::1%2]:3]",
            ),
            (
                "addrs: [9.9.9.9:41641]",
                "addrs: [public IPv4 (hidden):41641]",
            ),
            (
                "last: [10.0.0.7:1, [2606:4700::1]:2]",
                "last: [10.0.0.7:1, public IPv6 (hidden):2]",
            ),
            ("[[2606:4700::1]:2]", "[public IPv6 (hidden):2]"),
            (
                "[[2606:4700::1]:2, [2606:4700::2]:3]",
                "[public IPv6 (hidden):2, public IPv6 (hidden):3]",
            ),
        ] {
            assert_eq!(hide_public_addresses(line), shown, "{line}");
        }
    }

    #[test]
    fn brackets_around_what_is_not_an_address_change_nothing() {
        // The brackets are cut from a stretch only to see whether an address is left. None is
        // here, and none of these is joined to a word.
        let line = "[08:00:00] [1.3.0] [dead:beef] [e::] [::a] [1.2.3] [v0.1] [::1] [192.168.1.2:5] \
                    [fe80::1%2] [[::a]:1] [2001:db8::1]";
        assert!(matches!(hide_public_addresses(line), Cow::Borrowed(_)));
    }

    #[test]
    fn times_versions_and_words_are_not_addresses() {
        let line = "2026-10-15T08:00:00.123456Z  INFO baton_app: iroh 1.3.0 dead:beef \
                    aa:bb:cc:dd:ee:ff 08:00:00 v0.1 1.2.3 deadbeef";
        assert!(matches!(hide_public_addresses(line), Cow::Borrowed(_)));
    }

    #[test]
    fn the_short_forms_that_module_names_make_are_outside_the_public_range() {
        // Names made of the letters a to f, with nothing joined to them for the guard against
        // pieces of words to see: only the range tells these from an address.
        for line in [
            "e::",
            "::a",
            "dead::beef",
            "cafe::babe",
            "face::b00c",
            "bad::add",
            "ace::fade",
            "2026-10-15T08:00:00.123456Z  WARN dead::beef: a line",
            "2026-10-15T08:00:00.123456Z  INFO cafe::babe, then bad::add.",
        ] {
            assert!(
                matches!(hide_public_addresses(line), Cow::Borrowed(_)),
                "{line}"
            );
        }
    }

    #[test]
    fn the_name_of_a_module_is_not_an_address() {
        // `e::` in `core::`, `::a` in `::api` and `f::` in `buf::` are addresses to the parser,
        // and the targets of log lines are made of such pieces.
        for line in [
            "2026-10-15T08:00:00.123456Z  WARN baton_core::node: a line",
            "2026-10-15T08:00:00.123456Z  INFO baton_ffi::api::startup::tests: a line",
            "2026-10-15T08:00:00.123456Z ERROR baton_ffi::logbuf::panics: panicked at a.rs:1:1: x",
            "2026-10-15T08:00:00.123456Z  INFO iroh_relay::client::conn: connected",
            "2026-10-15T08:00:00.123456Z DEBUG quinn_proto::connection: closed",
            "std::io::Error and core::fmt::Debug and tokio::net::UdpSocket",
            "name::a",
        ] {
            assert!(
                matches!(hide_public_addresses(line), Cow::Borrowed(_)),
                "{line}"
            );
        }
        // A public address next to a word is still found when something separates them.
        assert_eq!(
            hide_public_addresses("baton_core::node: peer:8.8.8.8:53 and via [2606:4700::1]:2"),
            "baton_core::node: peer:public IPv4 (hidden):53 and via public IPv6 (hidden):2"
        );
    }

    #[test]
    fn a_number_with_a_leading_zero_or_a_port_too_big_is_left_alone() {
        // These do not parse as addresses, and are not guessed at.
        for line in ["08.8.8.8", "8.8.8.8:99999", "8.8.8"] {
            assert!(
                matches!(hide_public_addresses(line), Cow::Borrowed(_)),
                "{line}"
            );
        }
    }
}
