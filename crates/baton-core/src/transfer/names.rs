// SPDX-License-Identifier: Apache-2.0

//! The names that a received transfer is written under.
//!
//! The paths in the list of a transfer say what is inside what, and the receiver chooses what
//! every entry is called (ADR 0008, "Names are the receiver's to choose"). A name comes from
//! another device, so the receiver does not take it as it is:
//!
//! - What the file system cannot hold, and what only disguises a name, becomes `_`. That is every
//!   control character, and the characters that hide text or change its order (`U+202E`, the
//!   zero-width space and the rest of [`is_invisible`]), except the joiners, the variation
//!   selectors and the Mongolian vowel separator that ordinary writing and emoji need, and the
//!   tags that make the flags of England, Scotland and Wales. Where the rules are Windows's (see
//!   [`Rules`]) it is also `< > : " \ | ? *`, a dot or a space at the end of a name, and the names
//!   that Windows keeps for devices, which get an `_` after the device word (`CON.txt` becomes
//!   `CON_.txt`). Where they are Apple's it is `:`, which the Finder shows as `/`.
//! - Two names in one folder that the file system takes for one name get a number before the
//!   extension, and the later one in the list gets it: `Notes.txt` and `notes.txt` become
//!   `Notes.txt` and `notes 2.txt` where case is ignored. A name that clashes with nothing keeps
//!   the name it was given, even when it looks like one that a number would have made: a file
//!   that was called `notes 2.txt` is not renamed to make room. Nor is a name that needed no
//!   cleaning: it goes before the names that cleaning made into the same name, so `a_b` stays
//!   `a_b` and `a:b` becomes `a_b 2` where the colon is replaced.
//!
//! [`plan`] chooses the name of every entry of a list. [`numbered`] is the numbering on its own,
//! for the code that moves the items of a transfer into place and finds a name taken there.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
};

use baton_proto::{
    device::{Platform, is_invisible},
    limits::MAX_FILE_NAME_BYTES,
    transfer::{list::ListEntry, path},
};
use icu_normalizer::DecomposingNormalizer;

/// What takes the place of a character that cannot be in a name.
const REPLACEMENT: char = '_';

/// The most bytes that a number takes in a name: the space before it and `4294967295`.
const MAX_NUMBER_BYTES: usize = 1 + 10;

/// What a volume is like, as far as the names of what is written to it go.
///
/// Two things about it depend on the volume and not only on the system: whether names that
/// differ only in case are one name, and whether names that differ only in the form of their
/// Unicode text (`é` as one character, or as an `e` and an accent) are. A Mac can have a volume
/// that tells upper and lower case apart, and a card can be formatted so that it does not. The
/// receiver finds out what its destination is like and says so with [`Rules::new`];
/// [`Rules::usual`] is the guess for when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    /// The system whose rules say which characters a name cannot have.
    ///
    /// A Windows host holds every name to the Windows list whatever this says. A `:` or a `\` in a
    /// name there is not part of one name when the name is joined to a folder, so a plan made on
    /// a Windows host can differ from one made elsewhere with the same rules. That does not
    /// matter, because a transfer is received by one device from start to end: on one device the
    /// same list and the same rules always give the same plan.
    pub platform: Platform,
    /// Names that differ only in case are one name.
    pub ignores_case: bool,
    /// Names that differ only in the form of their Unicode text are one name.
    pub ignores_normalisation: bool,
}

impl Rules {
    /// The rules for a volume that is like this on `platform`.
    pub const fn new(platform: Platform, ignores_case: bool, ignores_normalisation: bool) -> Self {
        Self {
            platform,
            ignores_case,
            ignores_normalisation,
        }
    }

    /// What a volume is usually like on the system that this is built for: case is ignored on a
    /// Mac and on Windows but not on an iPhone or an iPad, and the form of Unicode text is
    /// ignored on every Apple system.
    pub const fn usual() -> Self {
        let platform = Platform::current();
        Self::new(
            platform,
            matches!(platform, Platform::MacOs | Platform::Windows),
            Self::is_apple(platform),
        )
    }

    const fn is_apple(platform: Platform) -> bool {
        matches!(platform, Platform::MacOs | Platform::Ios | Platform::IpadOs)
    }
}

/// Whether a name is held to the Windows list under `rules`: when the rules are Windows's, and on
/// a Windows host always.
const fn windows_rules(rules: Rules) -> bool {
    cfg!(windows) || matches!(rules.platform, Platform::Windows)
}

/// What the file system compares names by: names with the same key are one name to it.
///
/// Case is folded by making the text lower case, upper case and lower case again, which makes `ß`
/// and `SS` the same, as full case folding does, and `ẞ` too: the capital sharp s is its own upper
/// case, so it is only the lower case that takes it to `ß`. A key that matches too much only gives
/// a name a number it did not need. One that matches too little is caught where the names are
/// written, and fails that entry and no other: a top-level item is moved into place by a move that
/// refuses to replace what it finds, and inside a folder the writer makes files with `create_new`
/// and folders with `create_dir`, never `create_dir_all`, which would take two folders that the
/// plan keeps apart for one.
fn key(name: &str, rules: Rules) -> String {
    let folded: String = if rules.ignores_case {
        name.chars()
            .flat_map(char::to_lowercase)
            .flat_map(char::to_uppercase)
            .flat_map(char::to_lowercase)
            .collect()
    } else {
        name.to_owned()
    };
    if rules.ignores_normalisation {
        DecomposingNormalizer::new_nfd()
            .normalize(&folded)
            .into_owned()
    } else {
        folded
    }
}

/// What the numbers of a name are counted under: names that are given numbers from one count.
///
/// For a name that is not long that is the name, and `notes.txt` and `Notes.TXT` have one family
/// where case is ignored. For names that are long it is what is left of them once they are cut
/// to take a number, so that names that differ only after the cut do not each walk past the same
/// names that are taken.
fn family(name: &str, rules: Rules) -> String {
    key(&numbered(name, 0), rules)
}

/// Whether `c` is one of the invisible characters that ordinary writing needs, and that is
/// therefore kept: the zero-width joiner and non-joiner (emoji sequences, Persian, the scripts of
/// India), the variation selectors (what makes an emoji an emoji, and which form of a Han
/// character) and the Mongolian ones, which are the free variation selectors and the vowel
/// separator that is written inside words.
const fn needed_by_text(c: char) -> bool {
    matches!(
        c,
        '\u{180B}'..='\u{180F}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

/// The flags of England, Scotland and Wales: a black flag (`U+1F3F4`), the tag characters that
/// spell `gbeng`, `gbsct` or `gbwls`, and the tag that ends them (`U+E007F`).
///
/// Tag characters draw nothing and can carry text that nobody sees, so they are replaced wherever
/// they are. These three sequences are the exception, because they are how emoji spells these
/// flags and the only text that they can carry is the name of the flag that they show.
const SUBDIVISION_FLAGS: [&str; 3] = [
    "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}",
    "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
    "\u{1F3F4}\u{E0067}\u{E0062}\u{E0077}\u{E006C}\u{E0073}\u{E007F}",
];

/// Whether a name may have `c` in it under `rules`.
fn allowed(c: char, rules: Rules) -> bool {
    // The line and paragraph separators are not control characters, but a name that has one in it
    // breaks lines in whatever shows it.
    if c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}')
        || (is_invisible(c) && !needed_by_text(c))
    {
        return false;
    }
    if windows_rules(rules) {
        !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    } else if Rules::is_apple(rules.platform) {
        c != ':'
    } else {
        // Android is here too. Its shared storage allows fewer characters than other Unix
        // systems do, and the Android receiver, which is not built yet, will say which.
        true
    }
}

/// Whether `stem` is one of the names that Windows keeps for devices, in any case: `CON`, `PRN`,
/// `AUX`, `NUL`, `CONIN$`, `CONOUT$`, and `COM` and `LPT` with a digit (also a superscript one).
fn is_device(stem: &str) -> bool {
    let stem = stem.trim_end_matches(' ').to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|digit| {
            matches!(
                digit,
                "0" | "1"
                    | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "9"
                    | "\u{B9}"
                    | "\u{B2}"
                    | "\u{B3}"
            )
        })
    })
}

/// Where the device word ends, if `name` starts with one: Windows takes `CON.txt` and `CON .txt`
/// for the device too, so what counts is the name up to the first dot.
fn device_end(name: &str) -> Option<usize> {
    let stem = name.split('.').next().unwrap_or(name);
    is_device(stem).then(|| stem.trim_end_matches(' ').len())
}

/// Turns a dot or a space at the end of `name` into `_`. Windows does not keep them: `report.`
/// would be `report`, which another name of the transfer can be.
fn replace_trailing(name: &mut String) {
    let kept = name.trim_end_matches(['.', ' ']).len();
    let trailing = name.len() - kept;
    name.truncate(kept);
    name.extend(std::iter::repeat_n(REPLACEMENT, trailing));
}

/// The part of `text` that is at most `max` bytes long and ends at a character.
fn cut(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// `name` with every character that it cannot have replaced by `_`, and the flags that
/// [`SUBDIVISION_FLAGS`] lists kept whole.
fn replace_characters(name: &str, rules: Rules) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(c) = rest.chars().next() {
        if let Some(flag) = SUBDIVISION_FLAGS
            .iter()
            .find(|flag| rest.starts_with(**flag))
        {
            out.push_str(flag);
            rest = &rest[flag.len()..];
        } else {
            out.push(if allowed(c, rules) { c } else { REPLACEMENT });
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// A name that is safe to create on a volume with these rules: what cannot be in it is replaced
/// by `_`, as the module docs say, and nothing else is changed.
///
/// Every replacement takes the place of one character, so a name that is made of nothing but what
/// is replaced is not empty afterwards, and a name that was within [`MAX_FILE_NAME_BYTES`] still
/// is. (Only the `_` that a device name gets adds a byte, and then the end of the name is cut.)
/// Cleaning a name that is clean changes nothing.
pub fn clean(name: &str, rules: Rules) -> String {
    let mut name = replace_characters(name, rules);
    if windows_rules(rules) {
        replace_trailing(&mut name);
        if let Some(end) = device_end(&name) {
            name.insert(end, REPLACEMENT);
            // The cut can leave a dot or a space at the end, and never touches the device word,
            // which is at the start.
            let kept = cut(&name, MAX_FILE_NAME_BYTES).len();
            name.truncate(kept);
            replace_trailing(&mut name);
        }
    }
    name
}

/// Where the extension of `name` starts, which is where a number goes.
///
/// It is the last dot, unless that is the first character (`.bashrc` has no extension), or the
/// last one, or what follows it is long or has a space in it (`Mr. Smith's notes` has none).
/// `.tar` before the last dot belongs to the extension: `archive.tar.gz`.
fn extension_start(name: &str) -> usize {
    let Some(dot) = name.rfind('.') else {
        return name.len();
    };
    let after = &name[dot + 1..];
    if dot == 0
        || after.is_empty()
        || after.chars().count() > 16
        || after.contains(char::is_whitespace)
    {
        return name.len();
    }
    // A dot is one byte, so both of these cut at a character.
    let before = &name[..dot];
    match before.rfind('.') {
        Some(tar) if tar > 0 && before[tar + 1..].eq_ignore_ascii_case("tar") => tar,
        _ => dot,
    }
}

/// `name` with `number` before its extension, which is how a name that meets another gets a name
/// of its own: `notes.txt` is `notes 2.txt`, `archive.tar.gz` is `archive 2.tar.gz` and `Makefile`
/// is `Makefile 2`. Numbers start at 2.
///
/// A name that is long is cut at the end of what comes before the number, so that the result has
/// at most [`MAX_FILE_NAME_BYTES`] bytes, which is what every file system that Baton runs on
/// allows. It is cut for the longest number there is, so the same name is always cut in the same
/// place, whatever the number.
pub fn numbered(name: &str, number: u32) -> String {
    let (stem, extension) = name.split_at(extension_start(name));
    let room = MAX_FILE_NAME_BYTES.saturating_sub(MAX_NUMBER_BYTES + extension.len());
    format!("{} {number}{extension}", cut(stem, room))
}

/// Why a list could not be planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// An entry is not inside a folder that was listed before it. A list that
    /// [`baton_proto::transfer::list::ListChecker`] accepted never has this.
    #[error("entry {index} is not inside a folder that was listed before it")]
    MissingParent {
        /// The position of the entry in the list.
        index: usize,
    },
    /// Every number up to the largest is taken. That takes four billion names that clash, which
    /// no list has; it is an error so that the search for a number always ends.
    #[error("no number is left to give entry {index} a name of its own")]
    NoFreeNumber {
        /// The position of the entry in the list.
        index: usize,
    },
}

/// One entry of a plan.
struct Planned {
    /// The position of the folder that the entry is in, if it is in one.
    parent: Option<usize>,
    /// The name that the entry is written under, without its folders.
    name: String,
    /// Whether another name of its folder was taken first. Such a name waits for a number.
    clashes: bool,
}

/// The names of the entries of a list on the receiver's disk.
///
/// Like a list entry, it is shown by its size in a log: what people call their files is theirs.
pub struct Plan {
    items: Vec<Planned>,
}

impl fmt::Debug for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Plan")
            .field("entries", &self.items.len())
            .finish()
    }
}

impl Plan {
    /// How many entries there are, which is how many the list has.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the list was empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The name that entry number `index` of the list is written under, without the folders it is
    /// in, or `None` if the list has no such entry.
    pub fn name(&self, index: usize) -> Option<&str> {
        self.items.get(index).map(|item| item.name.as_str())
    }

    /// Where entry number `index` goes, relative to the folder that the transfer is put in: the
    /// names of the folders it is in, from the outermost, and its own name. `None` if the list
    /// has no such entry.
    pub fn path(&self, index: usize) -> Option<PathBuf> {
        let mut names = Vec::new();
        let mut at = Some(index);
        while let Some(index) = at {
            let item = self.items.get(index)?;
            names.push(item.name.as_str());
            at = item.parent;
        }
        Some(names.iter().rev().collect())
    }
}

/// The names that are taken in one folder.
#[derive(Default)]
struct Group {
    /// The keys of the names that are taken.
    taken: HashSet<String>,
    /// For each family of names, the number to try next. Without it a long run of names that
    /// clash would try the numbers that the ones before it took again.
    next: HashMap<String, u32>,
}

impl Group {
    /// Takes the first of `name 2`, `name 3` and so on that is free, or says that none is.
    fn number(&mut self, name: &str, rules: Rules) -> Option<String> {
        let family = family(name, rules);
        let mut number = self.next.get(&family).copied().unwrap_or(2);
        loop {
            let candidate = numbered(name, number);
            if self.taken.insert(key(&candidate, rules)) {
                self.next.insert(family, number.saturating_add(1));
                return Some(candidate);
            }
            number = number.checked_add(1)?;
        }
    }
}

/// Chooses the path of every entry of a list.
///
/// `entries` are in the order of a list that the checker accepted: an entry is inside a folder
/// that comes before it. Each name is [cleaned](clean), and then numbered if the file system
/// takes it for one name with another in its folder under `rules`. The first of such names keeps
/// the name it has and each later one, in the order of the list, gets the first number that is
/// free, which is a number that no other name of the folder has taken or already is. A name that
/// needed no cleaning counts as the first of its kind before the names that cleaning changed, so
/// that `a_b` keeps its name when `a:b` is cleaned into it. What is inside a folder goes inside
/// the planned folder, whatever it ended up called.
///
/// On one device the same list and the same rules always give the same plan, which a transfer
/// that is resumed relies on to find what it has already written.
pub fn plan(entries: &[ListEntry], rules: Rules) -> Result<Plan, PlanError> {
    let mut folders: HashMap<&str, usize> = HashMap::new();
    let mut groups: HashMap<Option<usize>, Group> = HashMap::new();
    let mut items = Vec::with_capacity(entries.len());
    // Whether cleaning changed the name of each entry.
    let mut changed = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let (folder, given) = path::split_name(&entry.path);
        let parent = if folder.is_empty() {
            None
        } else {
            let found = folders.get(folder).copied();
            Some(found.ok_or(PlanError::MissingParent { index })?)
        };
        let name = clean(given, rules);
        changed.push(name != given);
        if entry.folder {
            folders.insert(&entry.path, index);
        }
        items.push(Planned {
            parent,
            name,
            clashes: false,
        });
    }
    // The names that cleaning left alone take their place first, and then the names that it
    // changed, each in the order of the list: `a_b` is not given a number because `a:b` came
    // before it and became `a_b` too.
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&index| changed[index]);
    for index in order {
        let item = &mut items[index];
        let taken = &mut groups.entry(item.parent).or_default().taken;
        item.clashes = !taken.insert(key(&item.name, rules));
    }
    // Every name that does not clash is taken by now, so a number never takes the name of
    // another entry of the list.
    for (index, item) in items.iter_mut().enumerate() {
        if item.clashes {
            let group = groups.entry(item.parent).or_default();
            item.name = group
                .number(&item.name, rules)
                .ok_or(PlanError::NoFreeNumber { index })?;
        }
    }
    Ok(Plan { items })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;

    use super::*;

    const WINDOWS: Rules = Rules::new(Platform::Windows, true, false);
    const MAC: Rules = Rules::new(Platform::MacOs, true, true);
    const IPHONE: Rules = Rules::new(Platform::Ios, false, true);
    const LINUX: Rules = Rules::new(Platform::Linux, false, false);

    fn file(path: &str) -> ListEntry {
        ListEntry {
            path: path.into(),
            folder: false,
            size: 1,
            modified_ms: None,
        }
    }

    fn folder(path: &str) -> ListEntry {
        ListEntry {
            path: path.into(),
            folder: true,
            size: 0,
            modified_ms: None,
        }
    }

    /// The path of every entry as text, with `/` between the names on every system.
    fn planned(entries: &[ListEntry], rules: Rules) -> Vec<String> {
        let plan = plan(entries, rules).unwrap();
        (0..plan.len())
            .map(|index| {
                let path = plan.path(index).unwrap();
                let names: Vec<_> = path
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect();
                names.join("/")
            })
            .collect()
    }

    #[test]
    fn ordinary_names_are_not_touched_on_any_system() {
        for rules in [WINDOWS, MAC, IPHONE, LINUX] {
            for name in [
                "report.pdf",
                "Holiday 2026",
                "IMG_0001.jpeg",
                "日本語のファイル.txt",
                "Käse – 🧀.txt",
                ".hidden",
                "a b  c",
                "no extension",
                "dots.in.the.name.txt",
                "emoji ❤️.png",
                "family 👨\u{200D}👩\u{200D}👧.jpg",
                "می\u{200C}خواهم.txt",
                "漢\u{E0100}字.txt",
                "\u{1F1E9}\u{1F1EA}.txt",
                // The Mongolian word for black, which has the vowel separator inside it.
                "\u{182C}\u{1820}\u{1837}\u{180E}\u{1820}.txt",
                // The flags of England, Scotland and Wales.
                "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F} trip.jpg",
                "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F} trip.jpg",
                "trip \u{1F3F4}\u{E0067}\u{E0062}\u{E0077}\u{E006C}\u{E0073}\u{E007F}",
            ] {
                assert_eq!(clean(name, rules), name, "{name:?} on {rules:?}");
            }
        }
    }

    #[test]
    fn tag_characters_are_kept_only_in_the_flags_that_use_them() {
        let england = SUBDIVISION_FLAGS[0];
        let scotland = SUBDIVISION_FLAGS[1];
        for rules in [WINDOWS, MAC, IPHONE, LINUX] {
            for flag in SUBDIVISION_FLAGS {
                // Next to each other, and between other characters.
                let name = format!("a{flag}{flag}b");
                assert_eq!(clean(&name, rules), name);
                // A tag before or after a flag is not part of it.
                assert_eq!(
                    clean(&format!("\u{E0041}{flag}\u{E0041}"), rules),
                    format!("_{flag}_")
                );
            }
            // Text in tags after the black flag is not a flag of the three.
            let hidden = "\u{1F3F4}\u{E0069}\u{E0067}\u{E006E}\u{E006F}\u{E0072}\u{E0065}\u{E007F}";
            assert_eq!(clean(hidden, rules), "\u{1F3F4}_______");
            // Tags without the black flag in front of them, and a flag with its end cut off.
            let loose = england.trim_start_matches('\u{1F3F4}');
            assert_eq!(clean(loose, rules), "______");
            let cut_short = scotland.trim_end_matches('\u{E007F}');
            assert_eq!(clean(cut_short, rules), "\u{1F3F4}_____");
            // A flag that goes on is the flag and then tags that are replaced.
            assert_eq!(
                clean(&format!("{england}\u{E0067}\u{E007F}"), rules),
                format!("{england}__")
            );
            // The tag that ends a flag is replaced on its own like any other.
            assert_eq!(clean("\u{E007F}", rules), "_");
        }
    }

    #[test]
    fn control_and_disguising_characters_are_replaced_everywhere() {
        for rules in [WINDOWS, MAC, IPHONE, LINUX] {
            assert_eq!(clean("a\u{1}b\nc\td\u{7f}e\u{85}f", rules), "a_b_c_d_e_f");
            assert_eq!(clean("a\u{2028}b\u{2029}c", rules), "a_b_c");
            // The text-direction controls that make `invoice` U+202E `fdp.app` look like a PDF.
            assert_eq!(clean("invoice\u{202E}fdp.app", rules), "invoice_fdp.app");
            assert_eq!(
                clean("a\u{2066}b\u{2069}c\u{200E}d\u{061C}e", rules),
                "a_b_c_d_e"
            );
            for c in [
                '\u{200B}',
                '\u{2060}',
                '\u{FEFF}',
                '\u{00AD}',
                '\u{3164}',
                '\u{FFA0}',
                '\u{FFF9}',
                '\u{E0041}',
                '\u{E007F}',
                '\u{034F}',
                '\u{206A}',
                '\u{1D173}',
            ] {
                assert_eq!(
                    clean(&format!("a{c}b"), rules),
                    "a_b",
                    "U+{:04X}",
                    u32::from(c)
                );
            }
        }
    }

    #[test]
    fn what_windows_does_not_allow_is_replaced_under_the_windows_rules() {
        assert_eq!(clean(r#"a<b>c:d"e\f|g?h*i"#, WINDOWS), "a_b_c_d_e_f_g_h_i");
        assert_eq!(clean("a/b", WINDOWS), "a_b");
    }

    #[test]
    #[cfg(not(windows))]
    fn those_characters_are_kept_elsewhere_except_the_colon_on_apple_systems() {
        let text = r#"a<b>c"d\e|f?g*h"#;
        assert_eq!(clean(text, LINUX), text);
        assert_eq!(clean(text, MAC), text);
        assert_eq!(clean("a:b\\c", LINUX), "a:b\\c");
        for rules in [MAC, IPHONE] {
            assert_eq!(clean("a:b\\c", rules), "a_b\\c");
        }
    }

    #[test]
    #[cfg(windows)]
    fn a_windows_host_holds_every_name_to_the_windows_list() {
        for rules in [MAC, IPHONE, LINUX] {
            assert_eq!(clean("a:b\\c", rules), "a_b_c");
            assert_eq!(clean("CON", rules), "CON_");
            assert_eq!(clean("report.", rules), "report_");
        }
    }

    #[test]
    fn a_name_on_windows_cannot_end_in_a_dot_or_a_space() {
        assert_eq!(clean("report.", WINDOWS), "report_");
        assert_eq!(clean("report ", WINDOWS), "report_");
        assert_eq!(clean("a. . ", WINDOWS), "a____");
        assert_eq!(clean("...", WINDOWS), "___");
        assert_eq!(clean(" ", WINDOWS), "_");
        // Inside a name they are fine, and so is a dot or a space at the start.
        assert_eq!(clean(" a. b.c", WINDOWS), " a. b.c");
    }

    #[test]
    #[cfg(not(windows))]
    fn a_dot_or_a_space_at_the_end_is_kept_elsewhere() {
        for rules in [MAC, IPHONE, LINUX] {
            assert_eq!(clean("report. ", rules), "report. ");
            assert_eq!(clean("report.", rules), "report.");
        }
    }

    #[test]
    fn the_names_that_windows_keeps_for_devices_get_an_underscore() {
        for (name, clean_name) in [
            ("CON", "CON_"),
            ("con", "con_"),
            ("Nul.txt", "Nul_.txt"),
            ("aux.tar.gz", "aux_.tar.gz"),
            ("PRN", "PRN_"),
            ("COM0", "COM0_"),
            ("COM1", "COM1_"),
            ("com9.log", "com9_.log"),
            ("LPT0", "LPT0_"),
            ("LPT3", "LPT3_"),
            ("lpt\u{B2}", "lpt\u{B2}_"),
            ("COM\u{B9}.txt", "COM\u{B9}_.txt"),
            ("CONIN$", "CONIN$_"),
            ("conout$.x", "conout$_.x"),
            ("CON .txt", "CON_ .txt"),
        ] {
            assert_eq!(clean(name, WINDOWS), clean_name, "{name:?}");
        }
        // These only look like them.
        for name in [
            "console",
            "com10",
            "com",
            "lpt",
            "auxiliary",
            "nul_",
            "con_",
            "my.con",
            "CON_.txt",
            "xcon",
            "CON x",
        ] {
            assert_eq!(clean(name, WINDOWS), name, "{name:?}");
        }
    }

    #[test]
    #[cfg(not(windows))]
    fn the_names_of_devices_are_names_like_any_other_elsewhere() {
        for rules in [MAC, IPHONE, LINUX] {
            assert_eq!(clean("CON", rules), "CON");
            assert_eq!(clean("com1.txt", rules), "com1.txt");
        }
    }

    #[test]
    fn a_device_name_at_the_length_limit_still_fits() {
        // The `_` after `CON` makes this one byte too long, and the cut leaves a dot at the end.
        let name = format!("CON.{}.x", "x".repeat(MAX_FILE_NAME_BYTES - 6));
        assert_eq!(name.len(), MAX_FILE_NAME_BYTES);
        let cleaned = clean(&name, WINDOWS);
        assert_eq!(cleaned.len(), MAX_FILE_NAME_BYTES);
        assert!(cleaned.starts_with("CON_.x"), "{cleaned:?}");
        assert!(cleaned.ends_with("x_"), "{cleaned:?}");
        assert_eq!(clean(&cleaned, WINDOWS), cleaned);
        // A name of the same length that is not a device is not shortened.
        let name = "y".repeat(MAX_FILE_NAME_BYTES);
        assert_eq!(clean(&name, WINDOWS), name);
        // And the cut is at a character: this one is cut in the middle of the last `é`.
        let name = format!("CON.x{}", "é".repeat(125));
        assert_eq!(name.len(), MAX_FILE_NAME_BYTES);
        let cleaned = clean(&name, WINDOWS);
        assert_eq!(cleaned.len(), MAX_FILE_NAME_BYTES - 1);
        assert!(cleaned.starts_with("CON_.x"), "{cleaned:?}");
        assert!(cleaned.ends_with('é'), "{cleaned:?}");
    }

    #[test]
    fn cleaning_a_name_twice_is_cleaning_it_once() {
        for rules in [WINDOWS, MAC, IPHONE, LINUX] {
            for name in [
                "CON.",
                "a\u{202E}b ",
                "x:y",
                "con .txt",
                "...",
                "COM1 ",
                "a. ",
                "NUL\u{7}",
            ] {
                let once = clean(name, rules);
                assert_eq!(clean(&once, rules), once, "{name:?} on {rules:?}");
            }
        }
    }

    #[test]
    fn a_number_goes_before_the_extension() {
        for (name, number, result) in [
            ("notes.txt", 2, "notes 2.txt"),
            ("notes.txt", 13, "notes 13.txt"),
            ("notes", 2, "notes 2"),
            ("Makefile", 3, "Makefile 3"),
            ("archive.tar.gz", 2, "archive 2.tar.gz"),
            ("ARCHIVE.TAR.BZ2", 2, "ARCHIVE 2.TAR.BZ2"),
            ("a.b.c", 2, "a.b 2.c"),
            (".bashrc", 2, ".bashrc 2"),
            (".tar.gz", 2, ".tar 2.gz"),
            ("report.", 2, "report. 2"),
            ("Mr. Smith's notes", 2, "Mr. Smith's notes 2"),
            (
                "photo.a-very-long-extension-indeed",
                2,
                "photo.a-very-long-extension-indeed 2",
            ),
            ("日本語.txt", 2, "日本語 2.txt"),
        ] {
            assert_eq!(numbered(name, number), result, "{name:?} {number}");
        }
    }

    #[test]
    fn a_number_never_makes_a_name_longer_than_a_file_system_allows() {
        let accents = "é".repeat(125); // 250 bytes
        for name in [
            format!("{accents}.txt"),
            format!("{}x", "é".repeat(127)),
            "x".repeat(MAX_FILE_NAME_BYTES),
            format!("{}.tar.gz", "y".repeat(MAX_FILE_NAME_BYTES - 7)),
        ] {
            assert!(name.len() <= MAX_FILE_NAME_BYTES, "{} bytes", name.len());
            for number in [2, 10, 99_999, u32::MAX] {
                let result = numbered(&name, number);
                assert!(
                    result.len() <= MAX_FILE_NAME_BYTES,
                    "{} bytes",
                    result.len()
                );
                assert!(result.contains(&format!(" {number}")));
            }
        }
        // The extension and the number stay whole, and the stem gives way at a character.
        let result = numbered(&format!("{accents}.txt"), 2);
        assert!(result.ends_with(" 2.txt"), "{result:?}");
        assert!(result.starts_with("éé"));
        // The same name is cut in the same place whatever the number.
        let long = "z".repeat(MAX_FILE_NAME_BYTES);
        let cut_at = |number| numbered(&long, number).split(' ').next().unwrap().len();
        assert_eq!(cut_at(2), cut_at(u32::MAX));
    }

    #[test]
    fn names_that_the_file_system_takes_for_one_are_numbered_in_list_order() {
        let entries = [file("Notes.txt"), file("notes.txt"), file("NOTES.TXT")];
        assert_eq!(
            planned(&entries, MAC),
            ["Notes.txt", "notes 2.txt", "NOTES 3.TXT"]
        );
        // Where case counts they are three names.
        assert_eq!(
            planned(&entries, LINUX),
            ["Notes.txt", "notes.txt", "NOTES.TXT"]
        );
        assert_eq!(planned(&entries, IPHONE), planned(&entries, LINUX));
    }

    #[test]
    fn a_name_that_the_sender_chose_keeps_it_when_a_number_would_take_it() {
        // `notes 2.txt` is a name of its own, so the numbers step over it and it is not renamed.
        let entries = [
            file("Notes.txt"),
            file("notes 2.txt"),
            file("notes.txt"),
            file("NOTES.TXT"),
        ];
        assert_eq!(
            planned(&entries, MAC),
            ["Notes.txt", "notes 2.txt", "notes 3.txt", "NOTES 4.TXT"]
        );
        // It does not matter whether the name that a number would take comes before or after.
        let entries = [file("a"), file("A"), file("a 2")];
        assert_eq!(planned(&entries, MAC), ["a", "A 3", "a 2"]);
        let entries = [file("A"), file("a 2"), file("a")];
        assert_eq!(planned(&entries, MAC), ["A", "a 2", "a 3"]);
    }

    #[test]
    fn a_folder_from_an_iphone_can_hold_names_that_a_mac_takes_for_one() {
        let entries = [
            folder("Photos"),
            file("Photos/IMG.JPG"),
            file("Photos/img.jpg"),
            file("Photos/ß.txt"),
            file("Photos/ss.txt"),
        ];
        assert_eq!(
            planned(&entries, MAC),
            [
                "Photos",
                "Photos/IMG.JPG",
                "Photos/img 2.jpg",
                "Photos/ß.txt",
                "Photos/ss 2.txt"
            ]
        );
        assert_eq!(
            planned(&entries, IPHONE),
            [
                "Photos",
                "Photos/IMG.JPG",
                "Photos/img.jpg",
                "Photos/ß.txt",
                "Photos/ss.txt"
            ]
        );
    }

    #[test]
    fn the_capital_sharp_s_is_the_same_name_as_the_small_one_and_as_ss() {
        let entries = [file("ß.txt"), file("ẞ.txt"), file("ss.txt"), file("SS.txt")];
        assert_eq!(
            planned(&entries, MAC),
            ["ß.txt", "ẞ 2.txt", "ss 3.txt", "SS 4.txt"]
        );
        // Where case counts they are four names.
        assert_eq!(
            planned(&entries, IPHONE),
            ["ß.txt", "ẞ.txt", "ss.txt", "SS.txt"]
        );
        // The other way round, the first of them keeps its name.
        let entries = [file("ẞ"), file("ss"), file("ß")];
        assert_eq!(planned(&entries, MAC), ["ẞ", "ss 2", "ß 3"]);
    }

    #[test]
    fn no_character_gives_a_different_key_in_another_case() {
        let case_only = Rules::new(Platform::Linux, true, false);
        for c in '\0'..=char::MAX {
            if c.to_lowercase().eq([c]) && c.to_uppercase().eq([c]) {
                continue;
            }
            let key_of = |text: String| key(&text, case_only);
            let own = key_of(c.to_string());
            assert_eq!(
                own,
                key_of(c.to_lowercase().collect()),
                "U+{:04X} and its lower case",
                u32::from(c)
            );
            assert_eq!(
                own,
                key_of(c.to_uppercase().collect()),
                "U+{:04X} and its upper case",
                u32::from(c)
            );
        }
    }

    #[test]
    fn names_that_differ_only_in_the_form_of_their_text_clash_where_the_volume_normalises() {
        let composed = "caf\u{E9}.txt";
        let decomposed = "cafe\u{301}.txt";
        let entries = [file(composed), file(decomposed)];
        assert_eq!(
            planned(&entries, MAC),
            [composed.to_owned(), "cafe\u{301} 2.txt".to_owned()]
        );
        assert_eq!(
            planned(&entries, LINUX),
            [composed.to_owned(), decomposed.to_owned()]
        );
        // Case and form together.
        let entries = [file("CAF\u{C9}"), file("cafe\u{301}")];
        assert_eq!(
            planned(&entries, MAC),
            ["CAF\u{C9}".to_owned(), "cafe\u{301} 2".to_owned()]
        );
        let case_only = Rules::new(Platform::MacOs, true, false);
        assert_eq!(planned(&entries, case_only), ["CAF\u{C9}", "cafe\u{301}"]);
    }

    #[test]
    fn names_that_clean_to_one_name_are_numbered_too() {
        let entries = [file("a:b"), file("a?b"), file("a*b")];
        assert_eq!(planned(&entries, WINDOWS), ["a_b", "a_b 2", "a_b 3"]);
        let entries = [file("x\u{202E}y"), file("x\u{202D}y")];
        assert_eq!(planned(&entries, LINUX), ["x_y", "x_y 2"]);
    }

    #[test]
    fn a_name_that_needed_no_cleaning_is_not_given_up_for_one_that_did() {
        // `a_b` was called that, and `a:b` has to change anyway, so `a:b` is the one that waits.
        let entries = [file("a:b"), file("a_b"), file("a?b")];
        assert_eq!(planned(&entries, WINDOWS), ["a_b 2", "a_b", "a_b 3"]);
        let entries = [file("x\u{202E}y"), file("x_y")];
        assert_eq!(planned(&entries, LINUX), ["x_y 2", "x_y"]);
        // Among the names of one kind the list decides, and the numbers step over what is taken.
        let entries = [
            file("a:b"),
            file("a_b 2"),
            file("a?b"),
            file("A_B"),
            file("a_b"),
        ];
        assert_eq!(
            planned(&entries, WINDOWS),
            ["a_b 3", "a_b 2", "a_b 4", "A_B", "a_b 5"]
        );
        // It is the same inside a folder, and in a folder of its own.
        let entries = [
            folder("d"),
            file("d/a:b"),
            file("d/a_b"),
            folder("e"),
            file("e/a:b"),
        ];
        assert_eq!(
            planned(&entries, WINDOWS),
            ["d", "d/a_b 2", "d/a_b", "e", "e/a_b"]
        );
    }

    #[test]
    fn what_is_inside_a_folder_goes_inside_the_planned_folder() {
        let entries = [
            folder("CON"),
            file("CON/a.txt"),
            folder("CON/sub"),
            file("CON/sub/b.txt"),
            folder("con"),
            file("con/a.txt"),
            file("z.txt"),
        ];
        assert_eq!(
            planned(&entries, WINDOWS),
            [
                "CON_",
                "CON_/a.txt",
                "CON_/sub",
                "CON_/sub/b.txt",
                // The second folder is `con_` to a file system that ignores case, which is
                // the name of the first one.
                "con_ 2",
                "con_ 2/a.txt",
                "z.txt",
            ]
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn a_device_name_is_the_name_of_a_folder_like_any_other_elsewhere() {
        let entries = [
            folder("CON"),
            file("CON/a.txt"),
            folder("con"),
            file("con/a.txt"),
        ];
        assert_eq!(
            planned(&entries, LINUX),
            ["CON", "CON/a.txt", "con", "con/a.txt"]
        );
    }

    #[test]
    fn the_same_name_in_two_folders_is_not_a_clash() {
        let entries = [
            folder("a"),
            file("a/x"),
            folder("b"),
            folder("b/a"),
            file("b/a/x"),
            file("b/x"),
        ];
        assert_eq!(
            planned(&entries, MAC),
            ["a", "a/x", "b", "b/a", "b/a/x", "b/x"]
        );
    }

    #[test]
    fn a_file_and_a_folder_can_clash_and_the_later_one_gets_the_number() {
        let entries = [folder("Notes"), file("Notes/a"), file("notes")];
        assert_eq!(planned(&entries, MAC), ["Notes", "Notes/a", "notes 2"]);
    }

    #[test]
    fn a_list_that_is_in_order_and_clean_is_planned_as_it_is() {
        let entries = [
            folder("Holiday"),
            folder("Holiday/day 1"),
            file("Holiday/day 1/a.jpg"),
            file("Holiday/notes.txt"),
            file("report.pdf"),
        ];
        let paths: Vec<_> = entries.iter().map(|entry| entry.path.clone()).collect();
        for rules in [WINDOWS, MAC, IPHONE, LINUX] {
            assert_eq!(planned(&entries, rules), paths);
        }
        let plan = plan(&entries, MAC).unwrap();
        assert_eq!(plan.len(), 5);
        assert!(!plan.is_empty());
        assert_eq!(plan.name(2), Some("a.jpg"));
        assert_eq!(
            plan.path(2),
            Some(["Holiday", "day 1", "a.jpg"].iter().collect::<PathBuf>())
        );
        assert_eq!(plan.name(5), None);
        assert_eq!(plan.path(5), None);
        // A log shows how many entries there are and nothing that anyone called anything.
        assert_eq!(format!("{plan:?}"), "Plan { entries: 5 }");
        assert!(super::plan(&[], MAC).unwrap().is_empty());
    }

    #[test]
    fn an_entry_that_is_not_inside_a_listed_folder_cannot_be_planned() {
        for entries in [
            vec![file("a/x")],
            vec![folder("a"), file("b/x")],
            vec![folder("a"), folder("a/b"), file("a/c/x")],
            // A file is not a folder.
            vec![file("a"), file("a/x")],
            // The folder has to come first.
            vec![file("a/x"), folder("a")],
        ] {
            let error = plan(&entries, MAC).unwrap_err();
            assert!(
                matches!(error, PlanError::MissingParent { .. }),
                "{error:?}"
            );
        }
        assert_eq!(
            plan(&[folder("a"), file("b/x")], MAC).unwrap_err(),
            PlanError::MissingParent { index: 1 }
        );
    }

    #[test]
    fn no_number_is_left_after_the_largest() {
        let name = "notes.txt";
        let mut group = Group::default();
        group.taken.insert(key(name, MAC));
        group.taken.insert(key(&numbered(name, u32::MAX), MAC));
        group.next.insert(family(name, MAC), u32::MAX);
        assert_eq!(group.number(name, MAC), None);
        // The largest number itself is still given when it is free.
        let mut group = Group::default();
        group.taken.insert(key(name, MAC));
        group.next.insert(family(name, MAC), u32::MAX);
        assert_eq!(group.number(name, MAC), Some(numbered(name, u32::MAX)));
    }

    #[test]
    fn long_names_that_are_cut_to_the_same_stem_count_their_numbers_together() {
        let stem = "a".repeat(MAX_FILE_NAME_BYTES - 1);
        let (b, c) = (format!("{stem}b"), format!("{stem}c"));
        assert_ne!(key(&b, MAC), key(&c, MAC));
        assert_eq!(numbered(&b, 2), numbered(&c, 2));
        assert_eq!(family(&b, MAC), family(&c, MAC));
        // Names that are not cut keep their own count, and case does not matter where it is
        // ignored.
        assert_ne!(family("b.txt", MAC), family("c.txt", MAC));
        assert_ne!(family("a.txt", MAC), family("a.png", MAC));
        assert_eq!(family("b.txt", MAC), family("B.TXT", MAC));
        assert_ne!(family("b.txt", LINUX), family("B.TXT", LINUX));
    }

    #[test]
    fn many_names_that_clash_with_each_other_are_planned_quickly() {
        // 12 letters, each in two cases: 4096 names that are one name where case is ignored.
        let letters: Vec<char> = "abcdefghijkl".chars().collect();
        let mut names: Vec<String> = (0..1_u32 << letters.len())
            .map(|bits| {
                letters
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        if (bits >> i) & 1 == 1 {
                            c.to_ascii_uppercase()
                        } else {
                            *c
                        }
                    })
                    .collect()
            })
            .collect();
        names.sort();
        let entries: Vec<ListEntry> = names.iter().map(|name| file(name)).collect();
        let start = std::time::Instant::now();
        let paths = planned(&entries, MAC);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        let keys: HashSet<String> = paths.iter().map(|name| key(name, MAC)).collect();
        assert_eq!(keys.len(), entries.len());
        assert_eq!(
            paths.iter().filter(|name| name.contains(' ')).count(),
            entries.len() - 1
        );
    }

    #[test]
    fn the_usual_rules_are_those_of_the_system_this_is_built_for() {
        let usual = Rules::usual();
        assert_eq!(usual.platform, Platform::current());
        let (case, normalisation) = if cfg!(target_os = "macos") {
            (true, true)
        } else if cfg!(target_os = "windows") {
            (true, false)
        } else if cfg!(target_os = "ios") {
            (false, true)
        } else {
            (false, false)
        };
        assert_eq!(usual.ignores_case, case);
        assert_eq!(usual.ignores_normalisation, normalisation);
    }

    /// Bits of names that are trouble on some system or other.
    const PIECES: &[&str] = &[
        "a",
        "A",
        "b",
        "B",
        "ß",
        "ẞ",
        "SS",
        "ss",
        "é",
        "e\u{301}",
        "É",
        ":",
        "\\",
        "<",
        "?",
        ".",
        " ",
        ".txt",
        ".tar",
        ".gz",
        "con",
        "CON",
        "COM1",
        "nul",
        "_",
        " 2",
        "2",
        "\u{202E}",
        "\u{200D}",
        "\u{FE0F}",
        "\u{180E}",
        "\u{7}",
        "日",
        "\u{200B}",
        // A black flag, a tag that is no flag, and a whole flag (Scotland).
        "\u{1F3F4}",
        "\u{E0067}",
        "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
    ];

    /// Whether every character of `name`, but those in flags that are kept, is one that `rules`
    /// allow.
    fn has_only_allowed_characters(name: &str, rules: Rules) -> bool {
        let mut rest = name.to_owned();
        for flag in SUBDIVISION_FLAGS {
            rest = rest.replace(flag, "");
        }
        rest.chars().all(|c| allowed(c, rules))
    }

    fn name() -> impl Strategy<Value = String> {
        proptest::collection::vec(proptest::sample::select(PIECES), 1..6)
            .prop_map(|pieces| pieces.concat())
            .prop_filter("a name that the wire accepts", |name| {
                path::check_name(name).is_ok()
            })
    }

    /// A file, or a folder with entries in it.
    #[derive(Debug, Clone)]
    struct Node {
        name: String,
        children: Option<Vec<Node>>,
    }

    fn nodes(depth: u32) -> BoxedStrategy<Vec<Node>> {
        let leaf = name().prop_map(|name| Node {
            name,
            children: None,
        });
        let entry = if depth == 0 {
            leaf.boxed()
        } else {
            prop_oneof![
                3 => leaf,
                1 => (name(), nodes(depth - 1)).prop_map(|(name, children)| Node {
                    name,
                    children: Some(children),
                }),
            ]
            .boxed()
        };
        proptest::collection::vec(entry, 0..8)
            .prop_map(|nodes| {
                // The names in a folder are different and in the order of a list.
                let mut seen = BTreeSet::new();
                let mut nodes: Vec<Node> = nodes
                    .into_iter()
                    .filter(|node| seen.insert(node.name.clone()))
                    .collect();
                nodes.sort_by(|a, b| a.name.cmp(&b.name));
                nodes
            })
            .boxed()
    }

    /// The entries of a tree in the order of a list, each with the position of its folder.
    fn flatten(
        nodes: &[Node],
        prefix: &str,
        parent: Option<usize>,
        out: &mut Vec<(ListEntry, Option<usize>)>,
    ) {
        for node in nodes {
            let path = if prefix.is_empty() {
                node.name.clone()
            } else {
                format!("{prefix}/{}", node.name)
            };
            let index = out.len();
            match &node.children {
                Some(children) => {
                    out.push((folder(&path), parent));
                    flatten(children, &path, Some(index), out);
                }
                None => out.push((file(&path), parent)),
            }
        }
    }

    fn rules() -> impl Strategy<Value = Rules> {
        (
            proptest::sample::select(vec![
                Platform::Windows,
                Platform::MacOs,
                Platform::Ios,
                Platform::IpadOs,
                Platform::Linux,
                Platform::Android,
                Platform::Other,
            ]),
            any::<bool>(),
            any::<bool>(),
        )
            .prop_map(|(platform, case, normalisation)| Rules::new(platform, case, normalisation))
    }

    proptest! {
        #[test]
        fn every_plan_is_in_order_unique_and_valid(tree in nodes(3), rules in rules()) {
            let mut flat = Vec::new();
            flatten(&tree, "", None, &mut flat);
            let entries: Vec<ListEntry> = flat.iter().map(|(entry, _)| entry.clone()).collect();
            prop_assert!(entries
                .windows(2)
                .all(|pair| path::compare(&pair[0].path, &pair[1].path).is_lt()));

            let first = plan(&entries, rules).unwrap();
            prop_assert_eq!(first.len(), entries.len());
            // The same list and rules give the same plan.
            let again = plan(&entries, rules).unwrap();
            for index in 0..entries.len() {
                prop_assert_eq!(first.name(index), again.name(index));
            }

            let mut seen_in: HashMap<Option<usize>, HashSet<String>> = HashMap::new();
            let mut group_sizes: HashMap<Option<usize>, u32> = HashMap::new();
            for (_, parent) in &flat {
                *group_sizes.entry(*parent).or_default() += 1;
            }
            for (index, (entry, parent)) in flat.iter().enumerate() {
                let own = first.name(index).unwrap();
                let path = first.path(index).unwrap();
                // Its path is the path of its folder and its own name.
                let expected = match parent {
                    Some(parent) => first.path(*parent).unwrap().join(own),
                    None => PathBuf::from(own),
                };
                prop_assert_eq!(&path, &expected);
                prop_assert_eq!(path.components().count(), entry.path.split('/').count());

                // Two names that the file system takes for one are never made.
                let fresh = seen_in.entry(*parent).or_default().insert(key(own, rules));
                prop_assert!(fresh, "{own:?}");

                // A name is the one that cleaning left or a number of it.
                let cleaned = clean(path::split_name(&entry.path).1, rules);
                if own != cleaned {
                    let most = group_sizes[parent] + 2;
                    prop_assert!(
                        (2..=most).any(|n| numbered(&cleaned, n) == own),
                        "{own:?} is not a number of {cleaned:?}"
                    );
                }

                // And it is a name that can be made.
                prop_assert!(!own.is_empty() && own != "." && own != "..");
                prop_assert!(own.len() <= MAX_FILE_NAME_BYTES);
                prop_assert!(has_only_allowed_characters(own, rules), "{own:?}");
                if windows_rules(rules) {
                    prop_assert!(!own.ends_with(['.', ' ']), "{own:?}");
                    prop_assert!(device_end(own).is_none(), "{own:?}");
                }
            }
        }

        #[test]
        fn the_first_name_of_a_kind_keeps_its_name(tree in nodes(2), rules in rules()) {
            let mut flat = Vec::new();
            flatten(&tree, "", None, &mut flat);
            let entries: Vec<ListEntry> = flat.iter().map(|(entry, _)| entry.clone()).collect();
            let planned = plan(&entries, rules).unwrap();
            // A name that needed no cleaning is first of its kind before one that did.
            let mut seen: HashSet<(Option<usize>, String)> = HashSet::new();
            for changed in [false, true] {
                for (index, (entry, parent)) in flat.iter().enumerate() {
                    let given = path::split_name(&entry.path).1;
                    let cleaned = clean(given, rules);
                    if (cleaned != given) == changed
                        && seen.insert((*parent, key(&cleaned, rules)))
                    {
                        prop_assert_eq!(planned.name(index), Some(cleaned.as_str()));
                    }
                }
            }
        }

        #[test]
        fn cleaning_is_idempotent_and_keeps_what_is_fine(name in name(), rules in rules()) {
            let once = clean(&name, rules);
            prop_assert_eq!(clean(&once, rules), once.clone());
            let stays = has_only_allowed_characters(&name, rules)
                && !(windows_rules(rules)
                    && (name.ends_with(['.', ' ']) || device_end(&name).is_some()));
            if stays {
                prop_assert_eq!(once, name);
            }
        }

        #[test]
        fn a_number_fits_and_keeps_the_extension(name in name(), number in 2_u32..) {
            let result = numbered(&name, number);
            prop_assert!(result.len() <= MAX_FILE_NAME_BYTES);
            let suffix = format!(" {number}");
            prop_assert!(result.contains(&suffix));
            let start = extension_start(&name);
            prop_assert!(result.ends_with(&name[start..]));
        }
    }
}
