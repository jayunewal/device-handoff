// SPDX-License-Identifier: Apache-2.0

//! Which machine Baton is running on, so that a copied identity can be noticed.
//!
//! A device identity must belong to one machine. Migration tools, backup restores and file
//! sync copy whole home directories, and with them `identity.toml`. Two machines that answer
//! to the same key each receive what was meant for the other, and neither can tell. So the
//! identity file records which machine made it, and Baton stops with a clear message when it
//! finds the file on another one.
//!
//! The machine is told apart by the identifier the operating system keeps for itself:
//! `/etc/machine-id` on Linux, `IOPlatformUUID` on macOS and `MachineGuid` on Windows. That
//! identifier is not stored. What is stored is a hash of it made for Baton alone, so the
//! file does not reveal a value that other software also uses to recognise the machine.
//!
//! This is a safeguard, not a security boundary. It catches the ordinary ways an identity
//! gets copied; someone who can read the key file can still copy the key and edit the file.
//! Where no identifier can be found (a container without `/etc/machine-id`, a sandboxed app,
//! a phone) nothing is checked, and the check fails open rather than lock a person out.
//! Moving the key into a keystore whose items cannot leave the device would make the
//! problem go away; see `docs/security.md`.

use std::sync::OnceLock;

use data_encoding::HEXLOWER;

/// Domain separation, so the hash is specific to Baton and to this use.
const CONTEXT: &str = "baton 2026-10-14 machine binding v1";

/// Longest identifier accepted. A UUID with dashes is 36 characters.
const MAX_RAW_LEN: usize = 64;

/// Length of the stored form: a 32-byte hash in hexadecimal.
const STORED_LEN: usize = 64;

/// A stand-in for the machine: a hash of the identifier the operating system keeps for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MachineId(String);

impl MachineId {
    /// Makes one from the operating system's identifier, or `None` if that is not usable.
    pub(crate) fn from_raw(raw: &str) -> Option<Self> {
        let raw = clean(raw)?;
        let digest = blake3::derive_key(CONTEXT, raw.as_bytes());
        Some(Self(HEXLOWER.encode(&digest)))
    }

    /// Reads one back from a file. Anything but a hash of the right form is refused.
    pub(crate) fn parse(stored: &str) -> Option<Self> {
        let usable = stored.len() == STORED_LEN
            && stored
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        usable.then(|| Self(stored.to_owned()))
    }

    /// The form that is written to the identity file.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The machine Baton is running on, if the operating system will say.
///
/// The answer is worked out the first time and kept, because the machine does not change while
/// Baton runs. On a Mac and on Windows it comes from a program that Baton starts, and for a
/// moment a program that is starting holds a copy of every file that Baton has open, the lock
/// on the folder of a running Baton included. A folder that was let go of in that moment would
/// still be locked, and a Baton started on it again would be told that it is already running.
/// Asked once, that moment comes while the first Baton of the process is starting. No other has
/// finished starting by then, because each waits here for the answer, so there is no folder yet
/// that could be stopped and started again.
pub(crate) fn current() -> Option<MachineId> {
    static CURRENT: OnceLock<Option<MachineId>> = OnceLock::new();
    CURRENT.get_or_init(read).clone()
}

/// How often the operating system has been asked, for the test that it is asked only once.
#[cfg(test)]
static READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Asks the operating system which machine this is.
fn read() -> Option<MachineId> {
    #[cfg(test)]
    READS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    MachineId::from_raw(&raw()?)
}

/// Accepts an identifier that can tell machines apart, in a normal form.
///
/// An identifier that is all zeros is what some virtual machines and broken firmware report
/// for every machine, so it would make every machine look the same.
fn clean(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let usable = !raw.is_empty()
        && raw.len() <= MAX_RAW_LEN
        && raw.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        && raw.bytes().any(|b| b != b'0' && b != b'-');
    usable.then(|| raw.to_ascii_lowercase())
}

#[cfg(target_os = "linux")]
fn raw() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .into_iter()
        .find_map(|path| {
            std::fs::read_to_string(path)
                .ok()
                .filter(|text| clean(text).is_some())
        })
}

#[cfg(target_os = "macos")]
fn raw() -> Option<String> {
    let output = run("/usr/sbin/ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])?;
    parse_ioreg(&output)
}

#[cfg(windows)]
fn raw() -> Option<String> {
    // The system folder is named by the system, not looked up on the search path.
    let root = std::env::var_os("SystemRoot")?;
    let reg = std::path::Path::new(&root).join("System32").join("reg.exe");
    let output = run(
        reg,
        &[
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
            "/reg:64",
        ],
    )?;
    parse_reg(&output)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn raw() -> Option<String> {
    None
}

/// Runs a system tool and returns what it printed, if it worked.
#[cfg(any(target_os = "macos", windows))]
fn run(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Option<String> {
    use std::process::{Command, Stdio};
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Finds the hardware UUID in the output of `ioreg -rd1 -c IOPlatformExpertDevice`, which
/// has a line like `    "IOPlatformUUID" = "5A5E7E4B-2D5C-5B7C-8F1E-0123456789AB"`.
#[cfg(any(target_os = "macos", test))]
fn parse_ioreg(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("\"IOPlatformUUID\"")?;
        let (_, value) = rest.split_once('=')?;
        Some(value.trim().trim_matches('"').to_owned())
    })
}

/// Finds the machine GUID in the output of `reg query` for `MachineGuid`, which has a line
/// like `    MachineGuid    REG_SZ    4c4c4544-0042-4210-8039-b6c04f453032`.
#[cfg(any(windows, test))]
fn parse_reg(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        if words.next()? != "MachineGuid" || words.next()? != "REG_SZ" {
            return None;
        }
        words.next().map(str::to_owned)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(raw: &str) -> String {
        MachineId::from_raw(raw).unwrap().as_str().to_owned()
    }

    #[test]
    fn the_stored_form_is_a_hash_made_for_baton() {
        // Computed independently with the Python `blake3` package:
        // blake3(raw, derive_key_context="baton 2026-10-14 machine binding v1").
        assert_eq!(
            stored("0123456789abcdef0123456789abcdef"),
            "ca28632710423ebd1ad95bf0a569ffe6711bee790a5523b089af08a7a340a763"
        );
        assert_eq!(
            stored("5a5e7e4b-2d5c-5b7c-8f1e-0123456789ab"),
            "0142fe4e51da5b1bc9d626d54384590b17c53bd6de87d60b11f7f3ef1b3aab0d"
        );
        assert_eq!(
            stored("4c4c4544-0042-4210-8039-b6c04f453032"),
            "692adb7a07aef0c0e5f276144758c8c023eb129c96388dbebf3ef9e6bc684324"
        );
    }

    #[test]
    fn the_raw_identifier_does_not_appear_in_the_stored_form() {
        let raw = "0123456789abcdef0123456789abcdef";
        assert!(!stored(raw).contains("0123456789abcdef"));
        assert_eq!(stored(raw).len(), STORED_LEN);
    }

    #[test]
    fn case_and_surrounding_space_do_not_matter() {
        let plain = stored("5a5e7e4b-2d5c-5b7c-8f1e-0123456789ab");
        assert_eq!(stored("5A5E7E4B-2D5C-5B7C-8F1E-0123456789AB"), plain);
        assert_eq!(stored("  5a5e7e4b-2d5c-5b7c-8f1e-0123456789ab\n"), plain);
    }

    #[test]
    fn identifiers_that_cannot_tell_machines_apart_are_not_used() {
        for raw in [
            "",
            "   ",
            "uninitialized",
            "00000000-0000-0000-0000-000000000000",
            "00000000000000000000000000000000",
            "not a machine id",
            "5a5e7e4b-2d5c-5b7c-8f1e-0123456789ab\u{0}",
            &"a".repeat(MAX_RAW_LEN + 1),
        ] {
            assert!(MachineId::from_raw(raw).is_none(), "{raw:?}");
        }
        assert!(MachineId::from_raw(&"a".repeat(MAX_RAW_LEN)).is_some());
    }

    #[test]
    fn different_identifiers_give_different_values() {
        assert_ne!(
            stored("0123456789abcdef0123456789abcdef"),
            stored("0123456789abcdef0123456789abcdee")
        );
    }

    #[test]
    fn a_stored_value_must_be_a_hash_of_the_right_form() {
        let good = stored("0123456789abcdef0123456789abcdef");
        assert_eq!(MachineId::parse(&good).unwrap().as_str(), good);
        for bad in [
            "",
            "ca28",
            &good.to_uppercase(),
            &format!("{good}0"),
            &good.replacen('c', "g", 1),
            &good[..63],
        ] {
            assert!(MachineId::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn the_hardware_uuid_is_found_in_ioreg_output() {
        let output = "\
+-o Root  <class IORegistryEntry, id 0x100000100, retain 20>
  +-o MacBookPro18,1  <class IOPlatformExpertDevice, id 0x10000020f, registered, matched, active, busy 0 (95 ms), retain 41>
    {
      \"IOPlatformSerialNumber\" = \"C02XXXXXXXX\"
      \"IOPlatformUUID\" = \"5A5E7E4B-2D5C-5B7C-8F1E-0123456789AB\"
      \"manufacturer\" = <\"Apple Inc.\">
    }
";
        assert_eq!(
            parse_ioreg(output).as_deref(),
            Some("5A5E7E4B-2D5C-5B7C-8F1E-0123456789AB")
        );
        assert_eq!(parse_ioreg(""), None);
        assert_eq!(parse_ioreg("\"IOPlatformUUID\"\n"), None);
        assert_eq!(parse_ioreg("\"IOPlatformSerialNumber\" = \"C02\"\n"), None);
    }

    #[test]
    fn the_machine_guid_is_found_in_reg_output() {
        let output = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Cryptography\r\n    MachineGuid    REG_SZ    4c4c4544-0042-4210-8039-b6c04f453032\r\n\r\n";
        assert_eq!(
            parse_reg(output).as_deref(),
            Some("4c4c4544-0042-4210-8039-b6c04f453032")
        );
        assert_eq!(parse_reg(""), None);
        assert_eq!(parse_reg("    MachineGuid    REG_DWORD    5\n"), None);
        assert_eq!(parse_reg("    MachineGuid    REG_SZ\n"), None);
        assert_eq!(parse_reg("    Version    REG_SZ    1\n"), None);
    }

    #[test]
    fn the_machine_gives_the_same_answer_every_time() {
        assert_eq!(current(), current());
    }

    #[test]
    fn the_operating_system_is_asked_only_once() {
        // Another test may have asked already. However often the machine is needed, the system
        // has been asked once in all.
        for _ in 0..4 {
            let _ = current();
        }
        assert_eq!(READS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // The macOS and Windows runners are real machines with the identifiers above, so the
    // calls that read them are proved here, where they can run.
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn the_machine_can_be_identified() {
        assert!(current().is_some());
    }
}
