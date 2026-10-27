// SPDX-License-Identifier: Apache-2.0

//! `include/baton_stream_abi.h` is what the platform is written against, and `src/abi.rs` is what
//! the Rust side is written against. Nothing makes them the same but these tests, which read the
//! header and hold it to the Rust side:
//!
//! - the numbers it defines, and no others;
//! - every struct: the fields, their types, and the offsets and sizes that follow from them by the
//!   rules of a 64-bit C compiler, and the `BATON_STATIC_ASSERT` lines that say the same;
//! - the function types, and the functions that the library exports;
//! - that a C compiler and a C++ compiler take the header, wherever there is one.
//!
//! The header is read by a small parser that knows only the shapes the header uses. A shape it
//! does not know stops it with a message, so that whoever adds one teaches the test as well.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    collections::BTreeMap,
    env,
    ffi::c_void,
    fs,
    mem::{align_of, offset_of, size_of},
    path::{Path, PathBuf},
    process::Command,
};

use baton_proto::limits::{
    MAX_CODECS, MAX_FRAME_RATE, MAX_NAME_BYTES, MAX_PICTURE_BYTES, MAX_PICTURE_EDGE,
    MAX_SOUND_CHANNELS, MAX_SOUND_PACKET_BYTES, MAX_SOUND_PACKET_SAMPLES, MAX_SOUND_RATE,
    MAX_STREAM_TITLE_BYTES, MAX_WINDOW_EDGE, MIN_SOUND_PACKET_SAMPLES, MIN_SOUND_RATE,
};
use baton_stream_abi::{
    abi::{
        self, BatonBytes, BatonDecoderState, BatonFormat, BatonFrame, BatonFront, BatonGeometry,
        BatonLimits, BatonMessage, BatonPicked, BatonPicture, BatonPlay, BatonPost, BatonRect,
        BatonScreen, BatonSharerVTable, BatonSound, BatonSoundPacket, BatonSourceMessage,
        BatonSurroundings, BatonTakes, BatonViewerVTable, BatonWindow, CancelPickFn, ControlFn,
        DecodeFn, EndedFn, HearFn, LeaveFn, LiftFn, LimitsFn, OpenFn, PickFn, PlayFn, PostFn,
        PrepareFn, ReleaseFn, ScreenFn, ShowFn, SoundFn, SourceFn, StartFn, StateFn, StoppedFn,
        SurroundingsFn,
    },
    exports::{
        baton_stream_abi_version, baton_stream_link_decode_failed, baton_stream_pick_done,
        baton_stream_register_sharer, baton_stream_register_viewer, baton_stream_source_begin,
        baton_stream_source_cancel, baton_stream_source_changed, baton_stream_source_clock,
        baton_stream_source_end, baton_stream_source_new_epoch, baton_stream_source_pause,
        baton_stream_source_recovery_pending, baton_stream_source_resume, baton_stream_source_sent,
        baton_stream_source_sound, baton_stream_source_sound_begin,
        baton_stream_source_sound_silent,
    },
};

// What the header says.

/// A function, or the type of a function.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CFunction {
    name: String,
    ret: String,
    params: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CField {
    name: String,
    c_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CStruct {
    name: String,
    fields: Vec<CField>,
}

/// One `BATON_STATIC_ASSERT` line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Assert {
    Size {
        of: String,
        bytes: usize,
        message: String,
    },
    Offset {
        of: String,
        field: String,
        bytes: usize,
        message: String,
    },
}

#[derive(Debug, Default)]
struct Parsed {
    /// The numbers it defines.
    numbers: BTreeMap<String, i64>,
    /// The macros that take arguments.
    macros: Vec<String>,
    structs: Vec<CStruct>,
    fn_types: Vec<CFunction>,
    prototypes: Vec<CFunction>,
    asserts: Vec<Assert>,
}

fn header_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("include/baton_stream_abi.h")
}

/// The header as it is in git. On Windows a checkout may give the file CRLF line breaks, which
/// say nothing about the header, so they are read as the line breaks of git.
fn header() -> String {
    fs::read_to_string(header_path())
        .unwrap()
        .replace("\r\n", "\n")
}

/// `text` with every run of white space as one space, and nothing at either end.
fn squeeze(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` without its comments. A comment becomes one space, and the line breaks in it stay where
/// they were.
fn without_comments(text: &str) -> String {
    assert!(
        !text.contains("//"),
        "the header has a // comment; it uses /* */ only"
    );
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let end = start
            + rest[start..]
                .find("*/")
                .expect("a comment that is never closed")
            + 2;
        out.extend(rest[start..end].chars().filter(|c| *c == '\n'));
        out.push(' ');
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Splits `uint32_t size` or `const uint8_t *ptr` into the type and the name.
fn declaration(text: &str) -> (String, String) {
    let start = text
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map_or(0, |at| at + 1);
    let (head, name) = text.split_at(start);
    assert!(
        !head.trim().is_empty() && !name.is_empty(),
        "no type or no name in `{text}`"
    );
    (head.trim_end().to_owned(), name.to_owned())
}

/// The types of a list of parameters.
fn param_types(params: &str) -> Vec<String> {
    let params = params.trim();
    if params == "void" {
        return Vec::new();
    }
    params
        .split(',')
        .map(|param| declaration(param.trim()).0)
        .collect()
}

/// The statements of the code that is left once the preprocessor lines are gone: the text up to
/// each `;` that is not inside braces.
fn statements(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0_usize;
    for c in code.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.checked_sub(1).expect("a } that was never opened"),
            ';' if depth == 0 => {
                out.push(squeeze(&current));
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    assert!(
        current.trim().is_empty(),
        "text after the last statement: {current}"
    );
    out
}

fn parse_struct(rest: &str) -> CStruct {
    let (name, rest) = rest.split_once(" {").expect("a struct with no body");
    let (body, alias) = rest
        .rsplit_once('}')
        .expect("a struct that is never closed");
    assert_eq!(alias.trim(), name, "a struct whose two names differ");
    let fields = body
        .split(';')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(|field| {
            let (c_type, name) = declaration(field);
            CField { name, c_type }
        })
        .collect();
    CStruct {
        name: name.to_owned(),
        fields,
    }
}

fn parse_fn_type(typedef: &str) -> CFunction {
    let (returns, after) = typedef
        .split_once(" (*")
        .unwrap_or_else(|| panic!("a typedef that is not a struct or a function type: {typedef}"));
    let (name, params) = after
        .split_once(")(")
        .unwrap_or_else(|| panic!("a function type with no parameters: {after}"));
    let params = params
        .strip_suffix(')')
        .expect("a function type that is not closed");
    CFunction {
        name: name.to_owned(),
        ret: returns.to_owned(),
        params: param_types(params),
    }
}

fn parse_prototype(statement: &str) -> CFunction {
    let (head, params) = statement
        .split_once('(')
        .unwrap_or_else(|| panic!("a statement that the test does not read: {statement}"));
    let params = params
        .strip_suffix(')')
        .expect("a prototype that is not closed");
    let (ret, name) = declaration(head);
    CFunction {
        name,
        ret,
        params: param_types(params),
    }
}

fn parse_assert(rest: &str) -> Assert {
    let inner = rest
        .strip_suffix(')')
        .expect("an assert that is not closed");
    let (condition, message) = inner
        .rsplit_once(", \"")
        .unwrap_or_else(|| panic!("an assert with no message: {rest}"));
    let message = message
        .strip_suffix('"')
        .expect("a message that is not closed")
        .to_owned();
    if let Some(of) = condition.strip_prefix("sizeof(") {
        let (of, bytes) = of
            .split_once(") == ")
            .unwrap_or_else(|| panic!("a sizeof that the test does not read: {condition}"));
        Assert::Size {
            of: of.to_owned(),
            bytes: bytes.parse().unwrap(),
            message,
        }
    } else if let Some(of) = condition.strip_prefix("offsetof(") {
        let (of, rest) = of
            .split_once(", ")
            .unwrap_or_else(|| panic!("an offsetof that the test does not read: {condition}"));
        let (field, bytes) = rest
            .split_once(") == ")
            .unwrap_or_else(|| panic!("an offsetof that the test does not read: {condition}"));
        Assert::Offset {
            of: of.to_owned(),
            field: field.to_owned(),
            bytes: bytes.parse().unwrap(),
            message,
        }
    } else {
        panic!("an assert that the test does not read: {condition}");
    }
}

fn define(parsed: &mut Parsed, rest: &str) {
    let (name, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    if let Some((name, _)) = name.split_once('(') {
        parsed.macros.push(name.to_owned());
        return;
    }
    let value = value.trim();
    if value.is_empty() {
        // The include guard.
        return;
    }
    let number = value
        .trim_matches(|c| c == '(' || c == ')')
        .trim_end_matches('u');
    let number = number
        .parse()
        .unwrap_or_else(|_| panic!("{name} is not a number: {value}"));
    parsed.numbers.insert(name.to_owned(), number);
}

/// What the header declares, for a C compiler and not a C++ one.
fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut code = String::new();
    // For each `#if` that is open: whether its lines are read, and whether it is a block for C++.
    let mut open: Vec<(bool, bool)> = Vec::new();
    for line in without_comments(text).lines() {
        let line = line.trim();
        let reading = open.iter().all(|(read, _)| *read);
        let Some(directive) = line.strip_prefix('#') else {
            if reading {
                code.push_str(line);
                code.push('\n');
            }
            continue;
        };
        let directive = directive.trim_start();
        let (word, rest) = directive
            .split_once(char::is_whitespace)
            .unwrap_or((directive, ""));
        let rest = rest.trim();
        match word {
            // The include guard.
            "ifndef" => open.push((true, false)),
            "ifdef" => {
                assert_eq!(rest, "__cplusplus", "an #ifdef that the test does not read");
                open.push((false, true));
            }
            "else" => {
                let last = open.last_mut().expect("an #else with no #if");
                assert!(last.1, "an #else in a block that is not for C++");
                last.0 = !last.0;
            }
            "endif" => {
                open.pop().expect("an #endif with no #if");
            }
            "include" => {
                assert!(
                    !reading || rest == "<stddef.h>" || rest == "<stdint.h>",
                    "an #include that the test does not read: {rest}"
                );
            }
            "define" if reading => define(&mut parsed, rest),
            "define" => {}
            other => panic!("the header has a directive that the test does not read: #{other}"),
        }
    }
    assert!(open.is_empty(), "an #if that is never closed");
    for statement in statements(&code) {
        if let Some(rest) = statement.strip_prefix("typedef struct ") {
            parsed.structs.push(parse_struct(rest));
        } else if let Some(rest) = statement.strip_prefix("typedef ") {
            parsed.fn_types.push(parse_fn_type(rest));
        } else if let Some(rest) = statement.strip_prefix("BATON_STATIC_ASSERT(") {
            parsed.asserts.push(parse_assert(rest));
        } else {
            parsed.prototypes.push(parse_prototype(&statement));
        }
    }
    parsed
}

// What a 64-bit C compiler makes of the structs.

#[derive(Debug)]
struct CompiledField {
    name: String,
    c_type: String,
    offset: usize,
    size: usize,
}

#[derive(Debug)]
struct Compiled {
    name: String,
    size: usize,
    align: usize,
    fields: Vec<CompiledField>,
}

/// The size and alignment of a type of the header.
fn size_align(c_type: &str, known: &BTreeMap<String, (usize, usize)>) -> (usize, usize) {
    match c_type {
        "uint32_t" | "int32_t" | "float" => (4, 4),
        "uint64_t" | "int64_t" | "size_t" | "double" => (8, 8),
        pointer if pointer.ends_with('*') => (8, 8),
        other => *known
            .get(other)
            .unwrap_or_else(|| panic!("the test does not know the size of the type {other}")),
    }
}

/// The layout of each struct, worked out from its fields by the rules of a 64-bit C compiler:
/// each field at the next multiple of its own alignment, and the struct as large as its
/// alignment makes it.
fn compile(parsed: &Parsed) -> Vec<Compiled> {
    let mut known: BTreeMap<String, (usize, usize)> = parsed
        .fn_types
        .iter()
        .map(|function| (function.name.clone(), (8, 8)))
        .collect();
    let mut out = Vec::new();
    for c_struct in &parsed.structs {
        let mut at = 0;
        let mut align = 1;
        let mut fields = Vec::new();
        for field in &c_struct.fields {
            let (size, field_align) = size_align(&field.c_type, &known);
            at = usize::next_multiple_of(at, field_align);
            fields.push(CompiledField {
                name: field.name.clone(),
                c_type: field.c_type.clone(),
                offset: at,
                size,
            });
            at += size;
            align = align.max(field_align);
        }
        let size = usize::next_multiple_of(at, align);
        known.insert(c_struct.name.clone(), (size, align));
        out.push(Compiled {
            name: c_struct.name.clone(),
            size,
            align,
            fields,
        });
    }
    out
}

// What the library has.

struct RustField {
    name: &'static str,
    c_type: &'static str,
    offset: usize,
    size: usize,
}

struct RustStruct {
    name: &'static str,
    size: usize,
    align: usize,
    fields: Vec<RustField>,
}

/// The layout of a struct of `abi`, with the C type that each of its fields is written as in the
/// header. The struct is named with every field, and each has the Rust type that is said, or this
/// does not compile.
macro_rules! rust_struct {
    ($ty:ident { $($field:ident: $rust:ty = $c:literal),+ $(,)? }) => {{
        #[allow(dead_code)]
        fn every_field_is_listed(value: &$ty) {
            let $ty { $($field: _),+ } = value;
        }
        $( let _: fn(&$ty) -> &$rust = |value| &value.$field; )+
        RustStruct {
            name: stringify!($ty),
            size: size_of::<$ty>(),
            align: align_of::<$ty>(),
            fields: vec![$(RustField {
                name: stringify!($field),
                c_type: $c,
                offset: offset_of!($ty, $field),
                size: size_of::<$rust>(),
            }),+],
        }
    }};
}

/// Every struct of the header, in the order of the header.
fn rust_structs() -> Vec<RustStruct> {
    let mut all = structs_for_showing();
    all.extend(structs_for_sharing());
    all.extend(structs_for_sound());
    all.extend(tables_of_functions());
    all
}

/// The structs for showing a window, and the ones that both halves use.
fn structs_for_showing() -> Vec<RustStruct> {
    vec![
        rust_struct!(BatonBytes {
            ptr: *const u8 = "const uint8_t *",
            len: usize = "size_t",
        }),
        rust_struct!(BatonLimits {
            size: u32 = "uint32_t",
            max_width: u32 = "uint32_t",
            max_height: u32 = "uint32_t",
            max_fps: u32 = "uint32_t",
            codecs: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
        }),
        rust_struct!(BatonWindow {
            size: u32 = "uint32_t",
            width: u32 = "uint32_t",
            height: u32 = "uint32_t",
            control: u32 = "uint32_t",
            title: BatonBytes = "BatonBytes",
            app: BatonBytes = "BatonBytes",
        }),
        rust_struct!(BatonScreen {
            size: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
            context: *mut c_void = "void *",
            id: i64 = "int64_t",
        }),
        rust_struct!(BatonMessage {
            size: u32 = "uint32_t",
            cap: u32 = "uint32_t",
            len: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
            text: *mut u8 = "uint8_t *",
        }),
        rust_struct!(BatonFrame {
            size: u32 = "uint32_t",
            flags: u32 = "uint32_t",
            number: u64 = "uint64_t",
            depends_on: u64 = "uint64_t",
            capture_time: u64 = "uint64_t",
            chain: u32 = "uint32_t",
            epoch: u32 = "uint32_t",
            sample: BatonBytes = "BatonBytes",
            sets: *const BatonBytes = "const BatonBytes *",
            set_count: usize = "size_t",
            sequence_sets: usize = "size_t",
        }),
        rust_struct!(BatonSourceMessage {
            size: u32 = "uint32_t",
            kind: u32 = "uint32_t",
            epoch: u32 = "uint32_t",
            codec: u32 = "uint32_t",
            width: u32 = "uint32_t",
            height: u32 = "uint32_t",
            fps: u32 = "uint32_t",
            reason: u32 = "uint32_t",
            count: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
        }),
        rust_struct!(BatonDecoderState {
            size: u32 = "uint32_t",
            decode_us: u32 = "uint32_t",
            thermal: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
        }),
    ]
}

/// The structs for sharing a window.
fn structs_for_sharing() -> Vec<RustStruct> {
    vec![
        rust_struct!(BatonTakes {
            size: u32 = "uint32_t",
            max_width: u32 = "uint32_t",
            max_height: u32 = "uint32_t",
            max_fps: u32 = "uint32_t",
            codecs: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
        }),
        rust_struct!(BatonFormat {
            size: u32 = "uint32_t",
            codec: u32 = "uint32_t",
            width: u32 = "uint32_t",
            height: u32 = "uint32_t",
            fps: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
        }),
        rust_struct!(BatonPicked {
            size: u32 = "uint32_t",
            width: u32 = "uint32_t",
            height: u32 = "uint32_t",
            kind: u32 = "uint32_t",
            context: *mut c_void = "void *",
            title: BatonBytes = "BatonBytes",
            app: BatonBytes = "BatonBytes",
        }),
        rust_struct!(BatonRect {
            x: f64 = "double",
            y: f64 = "double",
            width: f64 = "double",
            height: f64 = "double",
        }),
        rust_struct!(BatonGeometry {
            window_width: f64 = "double",
            window_height: f64 = "double",
            picture_width: u32 = "uint32_t",
            picture_height: u32 = "uint32_t",
            content: BatonRect = "BatonRect",
        }),
        rust_struct!(BatonPicture {
            size: u32 = "uint32_t",
            flags: u32 = "uint32_t",
            capture_time: u64 = "uint64_t",
            length_size: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
            sample: BatonBytes = "BatonBytes",
            sets: *const BatonBytes = "const BatonBytes *",
            set_count: usize = "size_t",
            geometry: BatonGeometry = "BatonGeometry",
        }),
        rust_struct!(BatonFront {
            rect: BatonRect = "BatonRect",
            flags: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
        }),
        rust_struct!(BatonSurroundings {
            size: u32 = "uint32_t",
            locked: u32 = "uint32_t",
            on_screen: u32 = "uint32_t",
            keyboard: u32 = "uint32_t",
            window: BatonRect = "BatonRect",
            fronts: *mut BatonFront = "BatonFront *",
            front_cap: u32 = "uint32_t",
            front_count: u32 = "uint32_t",
        }),
        rust_struct!(BatonPost {
            size: u32 = "uint32_t",
            kind: u32 = "uint32_t",
            x: f64 = "double",
            y: f64 = "double",
            action: u32 = "uint32_t",
            button: u32 = "uint32_t",
            dx: f32 = "float",
            dy: f32 = "float",
            modifiers: u32 = "uint32_t",
            code: u32 = "uint32_t",
            down: u32 = "uint32_t",
            repeat: u32 = "uint32_t",
            text: BatonBytes = "BatonBytes",
        }),
    ]
}

/// The structs for the sound of a display, on both sides.
fn structs_for_sound() -> Vec<RustStruct> {
    vec![
        rust_struct!(BatonSound {
            size: u32 = "uint32_t",
            codec: u32 = "uint32_t",
            rate: u32 = "uint32_t",
            channels: u32 = "uint32_t",
            samples: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
        }),
        rust_struct!(BatonPlay {
            size: u32 = "uint32_t",
            flags: u32 = "uint32_t",
            number: u64 = "uint64_t",
            capture_time: u64 = "uint64_t",
            lost_before: u64 = "uint64_t",
            packet: BatonBytes = "BatonBytes",
        }),
        rust_struct!(BatonSoundPacket {
            size: u32 = "uint32_t",
            reserved: u32 = "uint32_t",
            capture_time: u64 = "uint64_t",
            bytes: BatonBytes = "BatonBytes",
        }),
    ]
}

/// The tables of functions that the platform registers.
fn tables_of_functions() -> Vec<RustStruct> {
    vec![
        rust_struct!(BatonViewerVTable {
            version: u32 = "uint32_t",
            size: u32 = "uint32_t",
            context: *mut c_void = "void *",
            limits: Option<LimitsFn> = "BatonLimitsFn",
            open: Option<OpenFn> = "BatonOpenFn",
            decode: Option<DecodeFn> = "BatonDecodeFn",
            show: Option<ShowFn> = "BatonShowFn",
            source: Option<SourceFn> = "BatonSourceFn",
            state: Option<StateFn> = "BatonStateFn",
            ended: Option<EndedFn> = "BatonEndedFn",
            release: Option<ReleaseFn> = "BatonReleaseFn",
            sound: Option<SoundFn> = "BatonSoundFn",
            play: Option<PlayFn> = "BatonPlayFn",
            stopped: Option<StoppedFn> = "BatonStoppedFn",
        }),
        rust_struct!(BatonSharerVTable {
            version: u32 = "uint32_t",
            size: u32 = "uint32_t",
            context: *mut c_void = "void *",
            control: Option<ControlFn> = "BatonControlFn",
            pick: Option<PickFn> = "BatonPickFn",
            cancel_pick: Option<CancelPickFn> = "BatonCancelPickFn",
            prepare: Option<PrepareFn> = "BatonPrepareFn",
            start: Option<StartFn> = "BatonStartFn",
            ended: Option<EndedFn> = "BatonEndedFn",
            release: Option<ReleaseFn> = "BatonReleaseFn",
            surroundings: Option<SurroundingsFn> = "BatonSurroundingsFn",
            post: Option<PostFn> = "BatonPostFn",
            screen: Option<ScreenFn> = "BatonScreenFn",
            lift: Option<LiftFn> = "BatonLiftFn",
            hear: Option<HearFn> = "BatonHearFn",
            leave: Option<LeaveFn> = "BatonLeaveFn",
        }),
    ]
}

fn function(name: &str, ret: &str, params: &[&str]) -> CFunction {
    CFunction {
        name: name.to_owned(),
        ret: ret.to_owned(),
        params: params.iter().map(|param| (*param).to_owned()).collect(),
    }
}

/// A function type of `abi` as the header spells it. The alias is the type that is said, in both
/// directions, or this does not compile: a safe function does not pass for an unsafe one.
macro_rules! rust_fn_type {
    ($alias:ident as $name:literal: $ret:literal $params:tt is $rust:ty) => {{
        #[allow(dead_code)]
        fn is_the_alias(value: $rust) -> $alias {
            value
        }
        #[allow(dead_code)]
        fn is_the_type(value: $alias) -> $rust {
            value
        }
        function($name, $ret, &$params)
    }};
}

fn rust_fn_types() -> Vec<CFunction> {
    vec![
        rust_fn_type!(LimitsFn as "BatonLimitsFn": "int32_t" ["void *", "BatonLimits *"]
            is unsafe extern "C" fn(*mut c_void, *mut BatonLimits) -> i32),
        rust_fn_type!(OpenFn as "BatonOpenFn": "int32_t"
            ["void *", "const BatonWindow *", "uint64_t", "BatonScreen *", "BatonMessage *"]
            is unsafe extern "C" fn(
                *mut c_void,
                *const BatonWindow,
                u64,
                *mut BatonScreen,
                *mut BatonMessage,
            ) -> i32),
        rust_fn_type!(DecodeFn as "BatonDecodeFn": "int32_t" ["void *", "const BatonFrame *"]
            is unsafe extern "C" fn(*mut c_void, *const BatonFrame) -> i32),
        rust_fn_type!(ShowFn as "BatonShowFn": "void" ["void *", "uint64_t"]
            is unsafe extern "C" fn(*mut c_void, u64)),
        rust_fn_type!(SourceFn as "BatonSourceFn": "void"
            ["void *", "const BatonSourceMessage *"]
            is unsafe extern "C" fn(*mut c_void, *const BatonSourceMessage)),
        rust_fn_type!(StateFn as "BatonStateFn": "void" ["void *", "BatonDecoderState *"]
            is unsafe extern "C" fn(*mut c_void, *mut BatonDecoderState)),
        rust_fn_type!(EndedFn as "BatonEndedFn": "void" ["void *"]
            is unsafe extern "C" fn(*mut c_void)),
        rust_fn_type!(ReleaseFn as "BatonReleaseFn": "void" ["void *"]
            is unsafe extern "C" fn(*mut c_void)),
        rust_fn_type!(SoundFn as "BatonSoundFn": "void" ["void *", "const BatonSound *"]
            is unsafe extern "C" fn(*mut c_void, *const BatonSound)),
        rust_fn_type!(PlayFn as "BatonPlayFn": "void" ["void *", "const BatonPlay *"]
            is unsafe extern "C" fn(*mut c_void, *const BatonPlay)),
        rust_fn_type!(StoppedFn as "BatonStoppedFn": "void" ["void *", "uint32_t"]
            is unsafe extern "C" fn(*mut c_void, u32)),
        rust_fn_type!(ControlFn as "BatonControlFn": "uint32_t" ["void *"]
            is unsafe extern "C" fn(*mut c_void) -> u32),
        rust_fn_type!(PickFn as "BatonPickFn": "int32_t" ["void *", "uint64_t", "BatonMessage *"]
            is unsafe extern "C" fn(*mut c_void, u64, *mut BatonMessage) -> i32),
        rust_fn_type!(CancelPickFn as "BatonCancelPickFn": "void" ["void *", "uint64_t"]
            is unsafe extern "C" fn(*mut c_void, u64)),
        rust_fn_type!(PrepareFn as "BatonPrepareFn": "int32_t"
            ["void *", "const BatonTakes *", "BatonFormat *"]
            is unsafe extern "C" fn(*mut c_void, *const BatonTakes, *mut BatonFormat) -> i32),
        rust_fn_type!(StartFn as "BatonStartFn": "int32_t"
            ["void *", "uint64_t", "BatonMessage *"]
            is unsafe extern "C" fn(*mut c_void, u64, *mut BatonMessage) -> i32),
        rust_fn_type!(SurroundingsFn as "BatonSurroundingsFn": "int32_t"
            ["void *", "BatonSurroundings *"]
            is unsafe extern "C" fn(*mut c_void, *mut BatonSurroundings) -> i32),
        rust_fn_type!(PostFn as "BatonPostFn": "int32_t" ["void *", "const BatonPost *"]
            is unsafe extern "C" fn(*mut c_void, *const BatonPost) -> i32),
        rust_fn_type!(ScreenFn as "BatonScreenFn": "int32_t"
            ["void *", "uint32_t", "BatonPicked *", "BatonMessage *"]
            is unsafe extern "C" fn(*mut c_void, u32, *mut BatonPicked, *mut BatonMessage) -> i32),
        rust_fn_type!(LiftFn as "BatonLiftFn": "void" ["void *", "uint32_t"]
            is unsafe extern "C" fn(*mut c_void, u32)),
        rust_fn_type!(HearFn as "BatonHearFn": "void" ["void *", "uint32_t"]
            is unsafe extern "C" fn(*mut c_void, u32)),
        rust_fn_type!(LeaveFn as "BatonLeaveFn": "void" ["void *"]
            is unsafe extern "C" fn(*mut c_void)),
    ]
}

/// The functions that the library exports, as the header declares them.
fn rust_exports() -> Vec<CFunction> {
    exports_by_name();
    vec![
        function("baton_stream_abi_version", "uint32_t", &[]),
        function(
            "baton_stream_register_viewer",
            "int32_t",
            &["const BatonViewerVTable *"],
        ),
        function("baton_stream_link_decode_failed", "int32_t", &["uint64_t"]),
        function(
            "baton_stream_register_sharer",
            "int32_t",
            &["const BatonSharerVTable *"],
        ),
        function(
            "baton_stream_pick_done",
            "int32_t",
            &["uint64_t", "const BatonPicked *", "const BatonBytes *"],
        ),
        function(
            "baton_stream_source_clock",
            "int32_t",
            &["uint64_t", "uint64_t *"],
        ),
        function(
            "baton_stream_source_begin",
            "int32_t",
            &["uint64_t", "uint32_t *"],
        ),
        function("baton_stream_source_cancel", "int32_t", &["uint64_t"]),
        function(
            "baton_stream_source_recovery_pending",
            "int32_t",
            &["uint64_t", "uint32_t *"],
        ),
        function(
            "baton_stream_source_sent",
            "int32_t",
            &[
                "uint64_t",
                "const BatonPicture *",
                "uint32_t *",
                "uint64_t *",
            ],
        ),
        function(
            "baton_stream_source_changed",
            "int32_t",
            &["uint64_t", "uint64_t"],
        ),
        function(
            "baton_stream_source_new_epoch",
            "int32_t",
            &["uint64_t", "const BatonFormat *"],
        ),
        function(
            "baton_stream_source_pause",
            "int32_t",
            &["uint64_t", "uint32_t"],
        ),
        function("baton_stream_source_resume", "int32_t", &["uint64_t"]),
        function(
            "baton_stream_source_sound_begin",
            "int32_t",
            &["uint64_t", "const BatonSound *"],
        ),
        function(
            "baton_stream_source_sound_silent",
            "int32_t",
            &["uint64_t", "uint32_t"],
        ),
        function(
            "baton_stream_source_sound",
            "int32_t",
            &[
                "uint64_t",
                "const BatonSoundPacket *",
                "uint32_t *",
                "uint64_t *",
            ],
        ),
        function(
            "baton_stream_source_end",
            "int32_t",
            &["uint64_t", "const BatonBytes *"],
        ),
    ]
}

/// Each function that the library exports, named, so that one whose signature changes stops this
/// from compiling. Nothing is called.
fn exports_by_name() {
    let _: extern "C" fn() -> u32 = baton_stream_abi_version;
    let _: unsafe extern "C" fn(*const BatonViewerVTable) -> i32 = baton_stream_register_viewer;
    let _: extern "C" fn(u64) -> i32 = baton_stream_link_decode_failed;
    let _: unsafe extern "C" fn(*const BatonSharerVTable) -> i32 = baton_stream_register_sharer;
    let _: unsafe extern "C" fn(u64, *const BatonPicked, *const BatonBytes) -> i32 =
        baton_stream_pick_done;
    let _: unsafe extern "C" fn(u64, *mut u64) -> i32 = baton_stream_source_clock;
    let _: unsafe extern "C" fn(u64, *mut u32) -> i32 = baton_stream_source_begin;
    let _: extern "C" fn(u64) -> i32 = baton_stream_source_cancel;
    let _: unsafe extern "C" fn(u64, *mut u32) -> i32 = baton_stream_source_recovery_pending;
    let _: unsafe extern "C" fn(u64, *const BatonPicture, *mut u32, *mut u64) -> i32 =
        baton_stream_source_sent;
    let _: extern "C" fn(u64, u64) -> i32 = baton_stream_source_changed;
    let _: unsafe extern "C" fn(u64, *const BatonFormat) -> i32 = baton_stream_source_new_epoch;
    let _: extern "C" fn(u64, u32) -> i32 = baton_stream_source_pause;
    let _: extern "C" fn(u64) -> i32 = baton_stream_source_resume;
    let _: unsafe extern "C" fn(u64, *const BatonSound) -> i32 = baton_stream_source_sound_begin;
    let _: extern "C" fn(u64, u32) -> i32 = baton_stream_source_sound_silent;
    let _: unsafe extern "C" fn(u64, *const BatonSoundPacket, *mut u32, *mut u64) -> i32 =
        baton_stream_source_sound;
    let _: unsafe extern "C" fn(u64, *const BatonBytes) -> i32 = baton_stream_source_end;
}

/// The numbers of `abi` by the name that the header gives them. `ABI_VERSION` is the one whose
/// name differs.
macro_rules! numbers {
    ($($name:ident),+ $(,)?) => {
        vec![
            ("BATON_STREAM_ABI_VERSION".to_owned(), i64::from(abi::ABI_VERSION)),
            $((concat!("BATON_", stringify!($name)).to_owned(), i64::from(abi::$name))),+
        ]
    };
}

fn rust_numbers() -> Vec<(String, i64)> {
    numbers![
        OK,
        E_ARGUMENT,
        E_REGISTERED,
        E_PLATFORM,
        E_UNKNOWN_LINK,
        E_REENTRANT,
        E_PANIC,
        CODEC_H264,
        PICKED_WINDOW,
        PICKED_DISPLAY,
        SCREEN_NEEDS_PERMISSION,
        SCREEN_UNAVAILABLE,
        SCREEN_NEEDS_INPUT_PERMISSION,
        SCREEN_PRIVATE,
        LIFT_SCREEN,
        LIFT_INPUT,
        FRAME_KEY,
        FRAME_RECOVERY,
        FRAME_REFERENCED,
        SOURCE_CONFIG,
        SOURCE_PAUSE,
        SOURCE_RESUME,
        SOURCE_NOTICE,
        PAUSE_OTHER,
        PAUSE_LOCKED,
        PAUSE_HIDDEN,
        DROPPED_OTHER,
        DROPPED_MALFORMED,
        DROPPED_CONTROL_OFF,
        DROPPED_PAUSED,
        DROPPED_LOCKED,
        DROPPED_NOT_ON_SCREEN,
        DROPPED_TOO_FAST,
        DROPPED_FRAME_UNKNOWN,
        DROPPED_PICTURE_OUT_OF_DATE,
        DROPPED_WINDOW_RESIZED,
        DROPPED_OUTSIDE_WINDOW,
        DROPPED_COVERED,
        DROPPED_NO_KEYBOARD,
        DROPPED_APP_WINDOW_IN_FRONT,
        DROPPED_TOO_MANY_HELD,
        DROPPED_NOT_PRESSED,
        THERMAL_NOMINAL,
        THERMAL_FAIR,
        THERMAL_SERIOUS,
        THERMAL_CRITICAL,
        THERMAL_OTHER,
        MESSAGE_MAX,
        BEGIN_NOT_NOW,
        BEGIN_KEY,
        BEGIN_RECOVERY,
        BEGIN_DELTA,
        SETS_MAX,
        SET_BYTES_MAX,
        SENT,
        SENT_STALE,
        SENT_TOO_LARGE,
        SENT_OVER,
        SOUND_CODEC_AAC,
        PLAY_FIRST,
        SILENT_OFF,
        SILENT_WINDOW,
        SILENT_NO_PERMISSION,
        SILENT_FAILED,
        SILENT_OTHER,
        SOUND_SENT,
        SOUND_DROPPED,
        SOUND_TOO_LARGE,
        SOUND_OVER,
        KEYBOARD_ELSEWHERE,
        KEYBOARD_SHARED,
        KEYBOARD_UNKNOWN,
        FRONT_SAME_APPLICATION,
        FRONT_MENU,
        FRONTS_MAX,
        POST_POINTER,
        POST_KEY,
        POINTER_MOVE,
        POINTER_DOWN,
        POINTER_UP,
        POINTER_SCROLL,
        BUTTON_LEFT,
        BUTTON_RIGHT,
        BUTTON_MIDDLE,
        MOD_SHIFT,
        MOD_CONTROL,
        MOD_OPTION,
        MOD_COMMAND,
        MOD_CAPS_LOCK,
    ]
}

// The tests.

#[test]
fn the_numbers_of_the_header_are_the_numbers_of_the_library() {
    let parsed = parse(&header());
    let ours = rust_numbers();
    let theirs: Vec<(String, i64)> = parsed
        .numbers
        .iter()
        .map(|(name, number)| (name.clone(), *number))
        .collect();
    let mut ours_sorted = ours.clone();
    ours_sorted.sort();
    assert_eq!(
        theirs, ours_sorted,
        "the numbers that the header defines, and the numbers of `abi`"
    );

    // A number of `abi` that the list above leaves out would be one the header could not have.
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/abi.rs")).unwrap();
    for line in source.lines() {
        let Some(rest) = line.trim_start().strip_prefix("pub const ") else {
            continue;
        };
        let name = rest.split(':').next().unwrap();
        let header_name = if name == "ABI_VERSION" {
            "BATON_STREAM_ABI_VERSION".to_owned()
        } else {
            format!("BATON_{name}")
        };
        assert!(
            parsed.numbers.contains_key(&header_name),
            "`abi::{name}` has no number in the header, which would be {header_name}"
        );
    }
}

#[test]
fn the_structs_of_the_header_are_the_structs_of_the_library() {
    let compiled = compile(&parse(&header()));
    let ours = rust_structs();
    assert_eq!(
        compiled
            .iter()
            .map(|c_struct| c_struct.name.as_str())
            .collect::<Vec<_>>(),
        ours.iter()
            .map(|r_struct| r_struct.name)
            .collect::<Vec<_>>(),
        "the structs of the header and of `abi`, in order"
    );
    for (c_struct, r_struct) in compiled.iter().zip(&ours) {
        let name = r_struct.name;
        assert_eq!(
            c_struct.fields.len(),
            r_struct.fields.len(),
            "{name}: how many fields"
        );
        for (c_field, r_field) in c_struct.fields.iter().zip(&r_struct.fields) {
            let field = r_field.name;
            assert_eq!(c_field.name, field, "{name}: the fields, in order");
            assert_eq!(c_field.c_type, r_field.c_type, "{name}.{field}: the type");
            assert_eq!(c_field.size, r_field.size, "{name}.{field}: the size");
            assert_eq!(c_field.offset, r_field.offset, "{name}.{field}: the offset");
        }
        assert_eq!(c_struct.size, r_struct.size, "{name}: the size");
        assert_eq!(c_struct.align, r_struct.align, "{name}: the alignment");
    }
}

#[test]
fn the_static_asserts_of_the_header_say_what_the_structs_are() {
    let parsed = parse(&header());
    assert_eq!(parsed.macros, ["BATON_STATIC_ASSERT"]);

    // These are the lines that make a compiler that disagrees refuse to build, so they must say
    // what the declarations make, for every struct and every field and in the order of the file.
    let mut expected = vec![
        Assert::Size {
            of: "void *".to_owned(),
            bytes: 8,
            message: "the interface is for 64-bit targets".to_owned(),
        },
        Assert::Size {
            of: "size_t".to_owned(),
            bytes: 8,
            message: "the interface is for 64-bit targets".to_owned(),
        },
    ];
    for c_struct in compile(&parsed) {
        expected.push(Assert::Size {
            of: c_struct.name.clone(),
            bytes: c_struct.size,
            message: c_struct.name.clone(),
        });
        for field in &c_struct.fields {
            expected.push(Assert::Offset {
                of: c_struct.name.clone(),
                field: field.name.clone(),
                bytes: field.offset,
                message: format!("{}.{}", c_struct.name, field.name),
            });
        }
    }
    assert_eq!(parsed.asserts, expected);
}

#[test]
fn the_function_types_of_the_header_are_those_of_the_library() {
    assert_eq!(parse(&header()).fn_types, rust_fn_types());
}

#[test]
fn the_functions_of_the_header_are_the_functions_the_library_exports() {
    assert_eq!(parse(&header()).prototypes, rust_exports());

    // The library exports these and no others: every `no_mangle` is a prototype of the header.
    let exports =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/exports.rs")).unwrap();
    assert_eq!(
        exports.matches("#[unsafe(no_mangle)]").count(),
        rust_exports().len(),
        "a function is exported that the header does not declare, or the other way round"
    );
}

#[test]
fn the_header_says_what_the_protocol_allows() {
    // The sentence is about the numbers of the protocol. If they change, the sentence is stale.
    let text = squeeze(&header().replace("\n *", " "));
    assert!(
        text.contains(&format!(
            "the largest picture is {MAX_PICTURE_EDGE} by {MAX_PICTURE_EDGE}, the most frames a second {MAX_FRAME_RATE}"
        )),
        "the header's numbers for the largest picture and the highest rate are not the protocol's"
    );
    assert_eq!(MAX_CODECS, 8);
    assert!(
        text.contains("there are one to eight codecs"),
        "the header's number of codecs is not the protocol's"
    );

    // What a window that is shared is held to.
    assert!(
        text.contains(&format!(
            "(an edge from 1 to {MAX_PICTURE_EDGE} pixels, a rate from 1 to {MAX_FRAME_RATE})"
        )),
        "the header's limits of the format of a picture are not the protocol's"
    );
    assert!(
        text.contains(&format!(
            "one larger than {MAX_WINDOW_EDGE} reads as {MAX_WINDOW_EDGE}"
        )),
        "the header's longest edge of a window or a display is not the protocol's"
    );
    assert!(
        text.contains(&format!(
            "They are cut to {MAX_STREAM_TITLE_BYTES} and {MAX_NAME_BYTES} bytes"
        )),
        "the header's longest title and application name are not the protocol's"
    );
    assert_eq!(MAX_PICTURE_BYTES, 8 * 1024 * 1024);
    assert!(
        text.contains("A sample of more than 8 MiB is not read"),
        "the header's largest picture is not the protocol's"
    );

    // What a form of sound and a packet of sound are held to.
    assert_eq!(MAX_SOUND_CHANNELS, 2);
    assert!(
        text.contains(&format!(
            "the sampling rate in hertz, from {MIN_SOUND_RATE} to {MAX_SOUND_RATE}, and channels is 1 or {MAX_SOUND_CHANNELS}"
        )),
        "the header's rates and channels of sound are not the protocol's"
    );
    assert!(
        text.contains(&format!(
            "one packet holds, from {MIN_SOUND_PACKET_SAMPLES} to {MAX_SOUND_PACKET_SAMPLES}"
        )),
        "the header's samples in a packet of sound are not the protocol's"
    );
    assert_eq!(MAX_SOUND_PACKET_BYTES, 8 * 1024);
    assert_eq!(
        text.matches("packet of more than 8 KiB is not read")
            .count(),
        2,
        "the header's largest packet of sound is not the protocol's"
    );
    // The sentences that the person is shown are cut to the length of every message.
    assert_eq!(
        text.matches(&format!(
            "at most {} bytes, cut to one line",
            abi::MESSAGE_MAX
        ))
        .count(),
        2,
        "the header's longest sentence is not the one of `abi`"
    );
}

#[test]
fn the_header_is_plain_text_that_is_not_wider_than_the_rest() {
    let text = header();
    assert!(text.is_ascii(), "the header is ASCII");
    assert!(text.starts_with("/* SPDX-License-Identifier: Apache-2.0 */\n"));
    assert!(text.ends_with("#endif /* BATON_STREAM_ABI_H */\n"));
    for (number, line) in text.lines().enumerate() {
        let number = number + 1;
        assert!(!line.contains('\t'), "line {number} has a tab");
        assert_eq!(line, line.trim_end(), "line {number} ends with a space");
        assert!(line.len() <= 100, "line {number} is wider than 100");
    }
}

// A C compiler and a C++ compiler.

/// What a platform does with the header: it declares its functions, builds the vtable, registers
/// it and calls the library. It is valid C11 and valid C++17, and is only compiled.
const PLATFORM: &str = r#"
#include "baton_stream_abi.h"
#include "baton_stream_abi.h"

static int32_t limits(void *context, BatonLimits *out) {
    (void)context;
    out->size = (uint32_t)sizeof *out;
    out->max_width = 1920;
    out->max_height = 1200;
    out->max_fps = 60;
    out->codecs = BATON_CODEC_H264;
    out->reserved = 0;
    return BATON_OK;
}

static int32_t open_screen(void *context, const BatonWindow *window, uint64_t link,
                           BatonScreen *screen, BatonMessage *message) {
    (void)context;
    (void)link;
    if (window->title.len > message->cap) {
        message->len = 0;
        return BATON_E_PLATFORM;
    }
    screen->size = (uint32_t)sizeof *screen;
    screen->context = NULL;
    screen->id = (int64_t)window->width;
    return BATON_OK;
}

static int32_t decode(void *screen, const BatonFrame *frame) {
    (void)screen;
    if ((frame->flags & BATON_FRAME_KEY) != 0 && frame->set_count < 2) {
        return BATON_E_ARGUMENT;
    }
    return frame->sample.len == 0 ? BATON_E_ARGUMENT : BATON_OK;
}

static void show(void *screen, uint64_t number) {
    (void)screen;
    (void)number;
}

static void source(void *screen, const BatonSourceMessage *message) {
    (void)screen;
    if (message->kind == BATON_SOURCE_NOTICE && message->reason == BATON_DROPPED_TOO_FAST) {
        (void)message->count;
    }
}

static void state(void *screen, BatonDecoderState *out) {
    (void)screen;
    out->size = (uint32_t)sizeof *out;
    out->decode_us = 1;
    out->thermal = BATON_THERMAL_NOMINAL;
    out->reserved = 0;
}

static void ended(void *screen) { (void)screen; }

static void release(void *screen) { (void)screen; }

static void sound(void *screen, const BatonSound *form) {
    (void)screen;
    if (form->codec == BATON_SOUND_CODEC_AAC && form->channels == 2 && form->samples == 1024) {
        (void)form->rate;
    }
}

static void play(void *screen, const BatonPlay *packet) {
    (void)screen;
    if ((packet->flags & BATON_PLAY_FIRST) != 0 || packet->lost_before > 0) {
        (void)packet->capture_time;
    }
    (void)packet->number;
    (void)packet->packet.len;
}

static void stopped(void *screen, uint32_t why) {
    (void)screen;
    if (why == BATON_SILENT_OFF || why == BATON_SILENT_WINDOW) {
        return;
    }
    (void)(why == BATON_SILENT_NO_PERMISSION || why == BATON_SILENT_FAILED ||
           why == BATON_SILENT_OTHER);
}

static int32_t register_viewer(void) {
    BatonViewerVTable table;
    table.version = BATON_STREAM_ABI_VERSION;
    table.size = (uint32_t)sizeof table;
    table.context = NULL;
    table.limits = limits;
    table.open = open_screen;
    table.decode = decode;
    table.show = show;
    table.source = source;
    table.state = state;
    table.ended = ended;
    table.release = release;
    table.sound = sound;
    table.play = play;
    table.stopped = stopped;
    (void)baton_stream_link_decode_failed(1);
    return baton_stream_register_viewer(&table);
}

static uint32_t control(void *context) {
    (void)context;
    return 1;
}

static int32_t pick(void *context, uint64_t request, BatonMessage *message) {
    (void)context;
    (void)request;
    if (message->cap == 0) {
        return BATON_E_PLATFORM;
    }
    return BATON_OK;
}

static void cancel_pick(void *context, uint64_t request) {
    (void)context;
    (void)request;
}

static int32_t prepare(void *picked, const BatonTakes *takes, BatonFormat *format) {
    (void)picked;
    if ((takes->codecs & BATON_CODEC_H264) == 0 || takes->max_width == 0) {
        return BATON_E_ARGUMENT;
    }
    format->size = (uint32_t)sizeof *format;
    format->codec = BATON_CODEC_H264;
    format->width = takes->max_width < 1920 ? takes->max_width : 1920;
    format->height = takes->max_height < 1080 ? takes->max_height : 1080;
    format->fps = takes->max_fps < 30 ? takes->max_fps : 30;
    format->reserved = 0;
    return BATON_OK;
}

static int32_t start(void *picked, uint64_t link, BatonMessage *message) {
    (void)picked;
    (void)message;
    return link == 0 ? BATON_E_PLATFORM : BATON_OK;
}

static int32_t surroundings(void *picked, BatonSurroundings *around) {
    (void)picked;
    if (around->front_cap == 0 || around->fronts == NULL) {
        return BATON_E_ARGUMENT;
    }
    around->locked = 0;
    around->on_screen = 1;
    around->keyboard = BATON_KEYBOARD_SHARED;
    around->window.x = 100.0;
    around->window.y = 50.0;
    around->window.width = 800.0;
    around->window.height = 600.0;
    around->fronts[0].rect = around->window;
    around->fronts[0].flags = BATON_FRONT_SAME_APPLICATION | BATON_FRONT_MENU;
    around->fronts[0].reserved = 0;
    around->front_count = 1;
    return BATON_OK;
}

static int32_t post_input(void *picked, const BatonPost *post) {
    (void)picked;
    if (post->kind == BATON_POST_POINTER) {
        if (post->action == BATON_POINTER_SCROLL) {
            return post->dy > 0.0f || post->dx > 0.0f ? BATON_OK : BATON_E_ARGUMENT;
        }
        if (post->action == BATON_POINTER_DOWN || post->action == BATON_POINTER_UP) {
            return post->button == BATON_BUTTON_LEFT ? BATON_OK : BATON_E_PLATFORM;
        }
        return post->action == BATON_POINTER_MOVE && post->x >= 0.0 && post->y >= 0.0
                   ? BATON_OK
                   : BATON_E_ARGUMENT;
    }
    if (post->kind == BATON_POST_KEY) {
        if ((post->modifiers & BATON_MOD_SHIFT) != 0 && post->text.len > 0) {
            return post->text.ptr != NULL ? BATON_OK : BATON_E_ARGUMENT;
        }
        return post->down == 1 || post->repeat == 0 ? BATON_OK : BATON_E_PLATFORM;
    }
    return BATON_E_ARGUMENT;
}

static int32_t screen(void *context, uint32_t flags, BatonPicked *picked, BatonMessage *message) {
    (void)context;
    if ((flags & ~BATON_SCREEN_PRIVATE) != 0) {
        return BATON_E_ARGUMENT;
    }
    if (picked->size < sizeof *picked || picked->title.len != 0 || message->cap == 0) {
        return BATON_E_ARGUMENT;
    }
    if (picked->width == 7) {
        return BATON_SCREEN_NEEDS_PERMISSION;
    }
    if (picked->width == 8) {
        return BATON_SCREEN_UNAVAILABLE;
    }
    if ((flags & BATON_SCREEN_PRIVATE) != 0 && picked->width == 9) {
        return BATON_SCREEN_NEEDS_INPUT_PERMISSION;
    }
    picked->width = 3024;
    picked->height = 1964;
    picked->kind = BATON_PICKED_DISPLAY;
    picked->context = picked;
    return BATON_OK;
}

static uint32_t lifted_now;

static void lift(void *picked, uint32_t lifted) {
    (void)picked;
    lifted_now = lifted & (BATON_LIFT_SCREEN | BATON_LIFT_INPUT);
}

static void window_ended(void *picked) { (void)picked; }

static void window_left(void *picked) { (void)picked; }

static void window_released(void *picked) { (void)picked; }

static uint32_t listening = 0;

static void hear(void *picked, uint32_t on) {
    (void)picked;
    listening = on;
}

static int32_t register_sharer(void) {
    BatonSharerVTable table;
    table.version = BATON_STREAM_ABI_VERSION;
    table.size = (uint32_t)sizeof table;
    table.context = NULL;
    table.control = control;
    table.pick = pick;
    table.cancel_pick = cancel_pick;
    table.prepare = prepare;
    table.start = start;
    table.ended = window_ended;
    table.release = window_released;
    table.surroundings = surroundings;
    table.post = post_input;
    table.screen = screen;
    table.lift = lift;
    table.hear = hear;
    table.leave = window_left;
    return baton_stream_register_sharer(&table);
}

int32_t register_platform(void) {
    int32_t status;
    if (baton_stream_abi_version() != BATON_STREAM_ABI_VERSION) {
        return BATON_E_ARGUMENT;
    }
    status = register_viewer();
    return status == BATON_OK ? register_sharer() : status;
}

/* What the picker of the system does when the person has chosen, given up, or it failed. */
int32_t answer_pick(uint64_t request, void *window, int32_t how) {
    static const char title[] = "Notes";
    static const char app[] = "Text editor";
    static const char failure[] = "The list of windows could not be shown.";
    BatonPicked picked;
    BatonBytes sentence;
    if (how == 0) {
        picked.size = (uint32_t)sizeof picked;
        picked.width = 1200;
        picked.height = 800;
        picked.kind = BATON_PICKED_WINDOW;
        picked.context = window;
        picked.title.ptr = (const uint8_t *)title;
        picked.title.len = sizeof title - 1;
        picked.app.ptr = (const uint8_t *)app;
        picked.app.len = sizeof app - 1;
        return baton_stream_pick_done(request, &picked, NULL);
    }
    if (how == 1) {
        return baton_stream_pick_done(request, NULL, NULL);
    }
    sentence.ptr = (const uint8_t *)failure;
    sentence.len = sizeof failure - 1;
    return baton_stream_pick_done(request, NULL, &sentence);
}

/* What a capture thread does for a window that is shared, with the link that start was given. */
int32_t capture(uint64_t link, uint32_t pending_picture) {
    static const uint8_t sequence_set[] = {0x67, 0x42, 0x00, 0x1f};
    static const uint8_t picture_set[] = {0x68, 0xce, 0x3c, 0x80};
    static const uint8_t slice[] = {0, 0, 0, 5, 0x65, 1, 2, 3, 4};
    static const char sorry[] = "The system stopped capturing the window.";
    uint64_t micros = 0;
    uint64_t number = 0;
    uint32_t kind = BATON_BEGIN_NOT_NOW;
    uint32_t pending = 0;
    uint32_t outcome = BATON_SENT_OVER;
    BatonBytes sets[2];
    BatonBytes sentence;
    BatonPicture picture;
    BatonFormat format;
    int32_t status = baton_stream_source_clock(link, &micros);
    if (status != BATON_OK) {
        return status;
    }
    status = baton_stream_source_recovery_pending(link, &pending);
    if (status != BATON_OK) {
        return status;
    }
    status = baton_stream_source_begin(link, &kind);
    if (status != BATON_OK || kind == BATON_BEGIN_NOT_NOW) {
        return status;
    }
    if (kind == BATON_BEGIN_DELTA && pending_picture == 0) {
        return baton_stream_source_cancel(link);
    }
    sets[0].ptr = sequence_set;
    sets[0].len = sizeof sequence_set;
    sets[1].ptr = picture_set;
    sets[1].len = sizeof picture_set;
    picture.size = (uint32_t)sizeof picture;
    picture.flags = BATON_FRAME_REFERENCED;
    picture.capture_time = micros;
    picture.length_size = 4;
    picture.reserved = 0;
    picture.sample.ptr = slice;
    picture.sample.len = sizeof slice;
    picture.sets = sets;
    picture.set_count = 2;
    picture.geometry.window_width = 800.0;
    picture.geometry.window_height = 600.0;
    picture.geometry.picture_width = 1600;
    picture.geometry.picture_height = 1200;
    picture.geometry.content.x = 0.0;
    picture.geometry.content.y = 0.0;
    picture.geometry.content.width = 1600.0;
    picture.geometry.content.height = 1200.0;
    status = baton_stream_source_changed(link, micros);
    if (status != BATON_OK) {
        return status;
    }
    status = baton_stream_source_sent(link, &picture, &outcome, &number);
    if (status != BATON_OK) {
        return status;
    }
    if (outcome == BATON_SENT_STALE || outcome == BATON_SENT_TOO_LARGE) {
        format.size = (uint32_t)sizeof format;
        format.codec = BATON_CODEC_H264;
        format.width = 1280;
        format.height = 720;
        format.fps = 30;
        format.reserved = 0;
        status = baton_stream_source_new_epoch(link, &format);
        if (status != BATON_OK) {
            return status;
        }
    }
    if (outcome == BATON_SENT_OVER) {
        return BATON_E_UNKNOWN_LINK;
    }
    if (number == 0) {
        (void)baton_stream_source_pause(link, BATON_PAUSE_LOCKED);
        (void)baton_stream_source_resume(link);
    }
    sentence.ptr = (const uint8_t *)sorry;
    sentence.len = sizeof sorry - 1;
    return baton_stream_source_end(link, pending_picture == 0 ? NULL : &sentence);
}

/* What the thread that records and encodes the sound of a display does, once hear said 1. */
int32_t record(uint64_t link, uint32_t allowed) {
    static const uint8_t access_unit[] = {0x21, 0x10, 0x04, 0x60};
    uint64_t micros = 0;
    uint64_t number = 0;
    uint32_t outcome = BATON_SOUND_OVER;
    BatonSound form;
    BatonSoundPacket packet;
    int32_t status;
    if (listening == 0) {
        return BATON_OK;
    }
    if (allowed == 0) {
        return baton_stream_source_sound_silent(link, BATON_SILENT_NO_PERMISSION);
    }
    form.size = (uint32_t)sizeof form;
    form.codec = BATON_SOUND_CODEC_AAC;
    form.rate = 48000;
    form.channels = 2;
    form.samples = 1024;
    form.reserved = 0;
    status = baton_stream_source_sound_begin(link, &form);
    if (status != BATON_OK) {
        return status;
    }
    status = baton_stream_source_clock(link, &micros);
    if (status != BATON_OK) {
        return status;
    }
    packet.size = (uint32_t)sizeof packet;
    packet.reserved = 0;
    packet.capture_time = micros;
    packet.bytes.ptr = access_unit;
    packet.bytes.len = sizeof access_unit;
    status = baton_stream_source_sound(link, &packet, &outcome, &number);
    if (status != BATON_OK) {
        return status;
    }
    if (outcome == BATON_SOUND_OVER) {
        return BATON_E_UNKNOWN_LINK;
    }
    if (outcome == BATON_SOUND_TOO_LARGE) {
        return baton_stream_source_sound_silent(link, BATON_SILENT_FAILED);
    }
    if (outcome == BATON_SOUND_DROPPED) {
        return baton_stream_source_sound_silent(link, BATON_SILENT_OTHER);
    }
    return outcome == BATON_SOUND_SENT ? BATON_OK : BATON_E_PLATFORM;
}
"#;

/// The first of `names` that runs, after the one that `variable` names.
fn find(variable: &str, names: &[&str]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for name in env::var(variable)
        .into_iter()
        .chain(names.iter().map(|n| (*n).to_owned()))
    {
        let runs = Command::new(&name)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        if runs && !found.contains(&name) {
            found.push(name);
        }
    }
    found
}

/// Runs `compiler` over `source`, against the headers in `include`, and gives what it said when
/// it refused.
fn compile_with(
    compiler: &str,
    standard: &str,
    include: &Path,
    source: &Path,
) -> Result<(), String> {
    let output = Command::new(compiler)
        .args([
            &format!("-std={standard}"),
            "-pedantic",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fsyntax-only",
            "-I",
        ])
        .arg(include)
        .arg(source)
        .output()
        .unwrap();
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{compiler} -std={standard}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// The compilers to try, for C and for C++. On Windows there are none that take these arguments.
/// On the machines that run CI there are, so a missing one is a failure and not a skipped test.
fn compilers(variable: &str, names: &[&str]) -> Option<Vec<String>> {
    if cfg!(windows) {
        return None;
    }
    let found = find(variable, names);
    if found.is_empty() {
        assert!(
            env::var_os("CI").is_none(),
            "no compiler was found ({names:?}), and this is CI"
        );
        eprintln!("no compiler was found ({names:?}); the header was not compiled");
        return None;
    }
    Some(found)
}

#[test]
fn a_c_compiler_takes_the_header_and_a_platform_that_uses_it() {
    let Some(found) = compilers("CC", &["cc", "gcc", "clang"]) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("platform.c");
    fs::write(&source, PLATFORM).unwrap();
    let include = header_path().parent().unwrap().to_owned();
    for compiler in found {
        compile_with(&compiler, "c11", &include, &source).unwrap();
    }
}

#[test]
fn a_cpp_compiler_takes_the_header_and_a_platform_that_uses_it() {
    let Some(found) = compilers("CXX", &["c++", "g++", "clang++"]) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("platform.cc");
    fs::write(&source, PLATFORM).unwrap();
    let include = header_path().parent().unwrap().to_owned();
    for compiler in found {
        compile_with(&compiler, "c++17", &include, &source).unwrap();
    }
}

#[test]
fn the_platform_sample_uses_every_function_of_the_header() {
    // A compiler that takes the sample says nothing about a function that the sample leaves out.
    let parsed = parse(&header());
    for prototype in &parsed.prototypes {
        assert!(
            PLATFORM.contains(&format!("{}(", prototype.name)),
            "the platform sample never calls {}",
            prototype.name
        );
    }
    for vtable in parsed
        .structs
        .iter()
        .filter(|c_struct| c_struct.name.ends_with("VTable"))
    {
        for field in &vtable.fields {
            if parsed
                .fn_types
                .iter()
                .any(|function| function.name == field.c_type)
            {
                assert!(
                    PLATFORM.contains(&format!("table.{} = ", field.name)),
                    "the platform sample never sets {}.{}",
                    vtable.name,
                    field.name
                );
            }
        }
    }
}

#[test]
fn a_compiler_refuses_a_header_whose_numbers_are_wrong() {
    // The test above is only worth something if a compiler does look at the numbers.
    let Some(found) = compilers("CC", &["cc", "gcc", "clang"]) else {
        return;
    };
    let text = header();
    for (right, wrong) in [
        (
            "offsetof(BatonLimits, codecs) == 16",
            "offsetof(BatonLimits, codecs) == 20",
        ),
        ("sizeof(BatonFrame) == 80", "sizeof(BatonFrame) == 72"),
        (
            "    uint32_t thermal;   /* BATON_THERMAL_ */\n    uint32_t reserved;\n}",
            "    uint32_t thermal;   /* BATON_THERMAL_ */\n}",
        ),
        ("sizeof(BatonPicture) == 112", "sizeof(BatonPicture) == 104"),
        (
            "offsetof(BatonSharerVTable, start) == 48",
            "offsetof(BatonSharerVTable, start) == 40",
        ),
        (
            "    size_t set_count;\n    BatonGeometry geometry;\n} BatonPicture;",
            "    size_t set_count;\n} BatonPicture;",
        ),
        ("sizeof(BatonPost) == 72", "sizeof(BatonPost) == 64"),
        (
            "offsetof(BatonSurroundings, front_cap) == 56",
            "offsetof(BatonSurroundings, front_cap) == 52",
        ),
        (
            "offsetof(BatonSharerVTable, post) == 80",
            "offsetof(BatonSharerVTable, post) == 72",
        ),
        (
            "offsetof(BatonSharerVTable, screen) == 88",
            "offsetof(BatonSharerVTable, screen) == 80",
        ),
        (
            "offsetof(BatonSharerVTable, lift) == 96",
            "offsetof(BatonSharerVTable, lift) == 88",
        ),
        (
            "offsetof(BatonSharerVTable, hear) == 104",
            "offsetof(BatonSharerVTable, hear) == 96",
        ),
        (
            "offsetof(BatonSharerVTable, leave) == 112",
            "offsetof(BatonSharerVTable, leave) == 104",
        ),
        (
            "sizeof(BatonSharerVTable) == 120",
            "sizeof(BatonSharerVTable) == 112",
        ),
        (
            "offsetof(BatonViewerVTable, play) == 88",
            "offsetof(BatonViewerVTable, play) == 80",
        ),
        (
            "sizeof(BatonViewerVTable) == 104",
            "sizeof(BatonViewerVTable) == 80",
        ),
        ("sizeof(BatonSound) == 24", "sizeof(BatonSound) == 20"),
        (
            "offsetof(BatonPlay, lost_before) == 24",
            "offsetof(BatonPlay, lost_before) == 20",
        ),
        (
            "    uint64_t capture_time;\n    BatonBytes bytes;\n} BatonSoundPacket;",
            "    uint64_t capture_time;\n} BatonSoundPacket;",
        ),
    ] {
        assert_eq!(text.matches(right).count(), 1, "the header has `{right}`");
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("baton_stream_abi.h"),
            text.replace(right, wrong),
        )
        .unwrap();
        let source = dir.path().join("platform.c");
        fs::write(&source, PLATFORM).unwrap();
        for compiler in &found {
            let result = compile_with(compiler, "c11", dir.path(), &source);
            assert!(
                result.is_err(),
                "{compiler} took a header with `{wrong}` instead of `{right}`"
            );
        }
    }
}
