// SPDX-License-Identifier: Apache-2.0

//! The size of the pictures that a sequence parameter set describes.
//!
//! A viewer says how large a picture it takes, and the source says in its `Config` what it
//! sends, but a decoder allocates by what the parameter sets in a key frame say, and those come
//! from a paired device that may be broken. A viewer therefore reads the size from them before it
//! hands them to a decoder, and refuses a picture that is larger than it said it takes.

use super::{NAL_SPS, kind};

/// The profiles whose sequence parameter set says how the chroma is sampled and how many bits a
/// sample has (H.264, 7.3.2.1.1). The others are 4:2:0 at 8 bits.
const WITH_CHROMA_FORMAT: [u32; 13] =
    [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135];

/// The most reference frames a sequence parameter set of picture order type 1 lists the offsets
/// of in its cycle.
const MAX_OFFSETS: u32 = 255;

/// The highest value of a bit depth, minus 8: samples have from 8 to 14 bits.
const MAX_DEPTH_MINUS_8: u32 = 6;

/// A reader of bits, the most significant first. A read at the end of the data, or one that
/// could not hold a number of 32 bits, gives `None`.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.at / 8)?;
        let bit = (byte >> (7 - self.at % 8)) & 1;
        self.at += 1;
        Some(u32::from(bit))
    }

    fn flag(&mut self) -> Option<bool> {
        self.bit().map(|bit| bit == 1)
    }

    /// A number of `count` bits, at most 32.
    fn bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0_u64;
        for _ in 0..count {
            value = (value << 1) | u64::from(self.bit()?);
        }
        u32::try_from(value).ok()
    }

    /// An unsigned number in Exp-Golomb code, `ue(v)`: as many zeros as the number has bits
    /// after its first one, the one, and those bits. The number is the value of the bits from
    /// the one on, less one.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0_u32;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let rest = self.bits(zeros)?;
        (1_u32 << zeros).checked_sub(1)?.checked_add(rest)
    }

    /// A signed number in Exp-Golomb code, `se(v)`: the numbers 0, 1, -1, 2, -2 and so on take
    /// the codes of 0, 1, 2, 3, 4.
    fn se(&mut self) -> Option<i64> {
        let code = self.ue()?;
        let magnitude = i64::from(code.div_ceil(2));
        Some(if code & 1 == 1 { magnitude } else { -magnitude })
    }

    /// A scaling list of `size` entries, which is read to get past it. Each entry says by how much
    /// the scale changes from the entry before, and a list ends early when the scale becomes zero:
    /// the entries that are left take the last scale and say nothing.
    fn skip_scaling_list(&mut self, size: u32) -> Option<()> {
        let mut scale = 8_i64;
        for _ in 0..size {
            if scale != 0 {
                scale = (scale + self.se()? + 256).rem_euclid(256);
            }
        }
        Some(())
    }
}

/// The bytes of a NAL unit without its emulation prevention: the 3 after two zeros is left out.
fn unescaped(unit: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(unit.len());
    let mut zeros = 0_u32;
    for byte in unit {
        if zeros >= 2 && *byte == 3 {
            zeros = 0;
            continue;
        }
        out.push(*byte);
        zeros = if *byte == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// The size in pixels, width and height, of the pictures that the sequence parameter set `sps`
/// describes, with its cropping taken off. `sps` is the NAL unit with its header byte, as
/// [`ForDecoder::sps`](super::ForDecoder::sps) has it.
///
/// This reads a set that comes from another device, and says `None` for one that is not a
/// sequence parameter set, one that is cut short, one whose size is nothing or too large for 32
/// bits, and one with a number in it that the standard does not allow and that the reading goes
/// by: the chroma format, a bit depth, the picture order type, the number of offsets of type 1,
/// and a number of more than 32 bits. It reads as far as the cropping and checks nothing else
/// on the way, so a set that is wrong in another way is for the decoder to refuse.
#[must_use]
pub fn picture_size(sps: &[u8]) -> Option<(u32, u32)> {
    if kind(sps) != Some(NAL_SPS) {
        return None;
    }
    let rbsp = unescaped(sps.get(1..)?);
    let mut bits = Bits { data: &rbsp, at: 0 };
    let profile = bits.bits(8)?;
    // The constraint flags, and the level.
    bits.bits(16)?;
    // The number of the set.
    bits.ue()?;
    let mut chroma = 1;
    if WITH_CHROMA_FORMAT.contains(&profile) {
        chroma = bits.ue()?;
        if chroma > 3 {
            return None;
        }
        if chroma == 3 {
            // Whether the three colour planes are coded apart. The cropping counts the same
            // either way.
            bits.bit()?;
        }
        // The bit depths of the luma and the chroma samples.
        if bits.ue()? > MAX_DEPTH_MINUS_8 || bits.ue()? > MAX_DEPTH_MINUS_8 {
            return None;
        }
        // Whether the transform is bypassed when the quantiser is zero.
        bits.bit()?;
        if bits.flag()? {
            let lists = if chroma == 3 { 12 } else { 8 };
            for list in 0..lists {
                if bits.flag()? {
                    bits.skip_scaling_list(if list < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    // The number of bits of the frame number.
    bits.ue()?;
    match bits.ue()? {
        0 => {
            // The number of bits of the picture order.
            bits.ue()?;
        }
        1 => {
            bits.bit()?;
            bits.se()?;
            bits.se()?;
            let offsets = bits.ue()?;
            if offsets > MAX_OFFSETS {
                return None;
            }
            for _ in 0..offsets {
                bits.se()?;
            }
        }
        2 => {}
        _ => return None,
    }
    // The most reference frames, and whether the numbers of frames may have gaps.
    bits.ue()?;
    bits.bit()?;
    let width_in_blocks = u64::from(bits.ue()?) + 1;
    let height_in_units = u64::from(bits.ue()?) + 1;
    let frames_only = bits.flag()?;
    if !frames_only {
        // Whether a frame may be coded as a pair of fields in one block row.
        bits.bit()?;
    }
    // Whether the motion of 8 by 8 blocks is inferred directly.
    bits.bit()?;
    let [left, right, top, bottom] = if bits.flag()? {
        [bits.ue()?, bits.ue()?, bits.ue()?, bits.ue()?].map(u64::from)
    } else {
        [0; 4]
    };

    // A unit of the height is a row of blocks of a frame, or of a field when the frame may be
    // coded as two. The cropping counts in the units of the samples of the chroma: two pixels
    // across for 4:2:0 and 4:2:2, and two pixels down for 4:2:0.
    let fields: u64 = if frames_only { 1 } else { 2 };
    let (unit_x, unit_y): (u64, u64) = match chroma {
        1 => (2, 2 * fields),
        2 => (2, fields),
        _ => (1, fields),
    };
    let width = width_in_blocks
        .checked_mul(16)?
        .checked_sub(unit_x.checked_mul(left.checked_add(right)?)?)?;
    let height = height_in_units
        .checked_mul(16 * fields)?
        .checked_sub(unit_y.checked_mul(top.checked_add(bottom)?)?)?;
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// The sequence parameter sets that x264 wrote for pictures of several sizes and kinds, as
    /// they come off the wire, with the size that `ffprobe` reads from the same stream. They are
    /// the evidence that this reads what a real encoder writes, and not only what the tests here
    /// write.
    const ENCODER_SETS: [(&str, &str, (u32, u32)); 13] = [
        (
            "main, 64 by 48",
            "6764000aacd9447b011000000300100000030020f1225960",
            (64, 48),
        ),
        (
            "high, 1920 by 1080, cropped at the bottom",
            "67640028acd940780227e5c044000003000400000300083c60c658",
            (1920, 1080),
        ),
        (
            "high, 1280 by 720",
            "6764001facd9405005bb011000000300100000030020f1831960",
            (1280, 720),
        ),
        (
            "high, 1366 by 768, cropped at the side",
            "67640020acd94056061e6f011000000300100000030020f1831960",
            (1366, 768),
        ),
        (
            "high, 3840 by 2160",
            "67640033acd9403c0043ec044000000300400000030083c60c6580",
            (3840, 2160),
        ),
        (
            "high, 2796 by 1290",
            "67640032acd9402bc0a3ee4c044000000300400000030083c60c6580",
            (2796, 1290),
        ),
        (
            "high, 1002 by 554",
            "6764001facd940fc11f924c044000003000400000300083c60c658",
            (1002, 554),
        ),
        (
            "high 4:2:2, 320 by 240",
            "677a000cbcd94141fb011000000300100000030020f1429960",
            (320, 240),
        ),
        (
            "high 4:4:4, 320 by 240",
            "67f4000c919b28283f6022000003000200000300041e28532c",
            (320, 240),
        ),
        (
            "high 4:4:4, 326 by 242, cropped",
            "67f4000c919b282a10f171f80880000003008000000301078a14cb",
            (326, 242),
        ),
        (
            "monochrome, 320 by 240",
            "6764000cf3650507ec05b2000003000200000300041e28532c",
            (320, 240),
        ),
        (
            "constrained baseline, 176 by 144",
            "6742c00ad902c4ec044000000300400000030083c48992",
            (176, 144),
        ),
        (
            "interlaced, 320 by 240",
            "67640015acd941410fcb808800000300080000030010f8a14cb0",
            (320, 240),
        ),
    ];

    /// Sets with scaling matrices, which no encoder here writes in the sequence parameter set
    /// (x264 sends its matrices in the picture parameter set). They were written by hand, to the
    /// syntax of the standard, put in front of the picture parameter set and the key frame of an
    /// x264 stream, and `ffprobe` read the size from them. The lists are 4 by 4 or 8 by 8, there
    /// are eight of them or twelve in 4:4:4, and a list that does not end early ends when its
    /// scale becomes zero, which a reader that lost count of the lists would not get past.
    const WRITTEN_BY_HAND: [(&str, &str, (u32, u32)); 5] = [
        (
            "high, a 4 by 4 list, one that ends at once, an 8 by 8 list that ends early",
            "6764000cad94ca208c52492490211a492492492492492492492492492490186d0283f2",
            (320, 240),
        ),
        (
            "high, every list sent, one of them ending in the middle and two at once",
            "6764000cada4924924924942321081dffffcc6318c6318c6318c631adb6db7142850a142850a142850a142850a142850a142850a142850a142850a142850a142850a142850a142850a142850a142850a142850a142850a846d0283f2",
            (320, 240),
        ),
        (
            "high 4:2:2, eight lists, two of them cut short",
            "677a000cbda4920dc59d210842108421084210842924924924924924924924924924924924924924924924924b40a0fc80",
            (320, 240),
        ),
        (
            "high 4:4:4, twelve lists",
            "67f4000c91b49249249249242290840ed24924924924a9249249249249249249249249249249249249249249249248466318c6318c6318c6318c6318c6318c6318c6318c6318c6318c6318c6318c6318c6318c6318c6318c94a52da0507e40",
            (320, 240),
        ),
        (
            "high 4:4:4, twelve lists, cropped",
            "67f4000c91a090842108421084210842029249249025b40a0fe88cc8",
            (316, 233),
        ),
    ];

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn the_sets_of_a_real_encoder_give_the_size_that_a_decoder_reads_from_them() {
        for (what, set, size) in ENCODER_SETS {
            assert_eq!(picture_size(&hex(set)), Some(size), "{what}");
        }
    }

    #[test]
    fn the_sets_with_scaling_lists_give_the_size_that_a_decoder_reads_from_them() {
        for (what, set, size) in WRITTEN_BY_HAND {
            assert_eq!(picture_size(&hex(set)), Some(size), "{what}");
        }
    }

    // A writer of the fields of a set, to make sets that no encoder here makes.

    #[derive(Default)]
    struct Writer {
        bytes: Vec<u8>,
        used: u32,
    }

    impl Writer {
        fn bit(&mut self, bit: bool) {
            if self.used == 0 {
                self.bytes.push(0);
            }
            if bit {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> self.used;
            }
            self.used = (self.used + 1) % 8;
        }

        fn bits(&mut self, value: u32, count: u32) {
            for at in (0..count).rev() {
                self.bit((value >> at) & 1 == 1);
            }
        }

        fn ue(&mut self, value: u32) {
            let code = u64::from(value) + 1;
            let length = 64 - code.leading_zeros();
            for _ in 1..length {
                self.bit(false);
            }
            for at in (0..length).rev() {
                self.bit((code >> at) & 1 == 1);
            }
        }

        fn se(&mut self, value: i32) {
            let code = if value > 0 {
                value.unsigned_abs() * 2 - 1
            } else {
                value.unsigned_abs() * 2
            };
            self.ue(code);
        }

        /// The bytes with the bit that ends a payload and the zeros that fill the last byte.
        fn finish(mut self) -> Vec<u8> {
            self.bit(true);
            self.bytes
        }
    }

    /// The bytes of a NAL unit of `kind` for `payload`, with emulation prevention.
    fn escaped(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut unit = vec![0x60 | kind];
        let mut zeros = 0;
        for byte in payload {
            if zeros >= 2 && *byte <= 3 {
                unit.push(3);
                zeros = 0;
            }
            unit.push(*byte);
            zeros = if *byte == 0 { zeros + 1 } else { 0 };
        }
        unit
    }

    #[derive(Debug, Clone)]
    enum Order {
        Lsb(u32),
        Offsets(Vec<i32>),
        None,
        /// A type that the standard does not have: the number is written and nothing after it.
        Other(u32),
    }

    /// The fields of a set that decide its size, and the fields in between that have to be
    /// read to get to them.
    #[derive(Debug, Clone)]
    struct Fields {
        profile: u32,
        chroma: u32,
        separate_planes: bool,
        /// The bit depths of the luma and the chroma samples, less 8.
        depths: [u32; 2],
        /// The lists of the scaling matrix, if it is sent: for each list, whether it is sent and
        /// the steps it takes.
        scaling: Option<Vec<Option<Vec<i32>>>>,
        /// The number of bits of the frame number, less 4. Nothing here reads it.
        frame_bits: u32,
        order: Order,
        width_in_blocks: u32,
        height_in_units: u32,
        frames_only: bool,
        crop: Option<[u32; 4]>,
    }

    impl Fields {
        fn high(&self) -> bool {
            WITH_CHROMA_FORMAT.contains(&self.profile)
        }

        /// The size that the standard gives for these fields, worked out as it states it.
        fn size(&self) -> Option<(u32, u32)> {
            let chroma = if self.high() { self.chroma } else { 1 };
            let (sub_width, sub_height) = match chroma {
                1 => (2, 2),
                2 => (2, 1),
                _ => (1, 1),
            };
            let (unit_x, unit_y) = if chroma == 0 || (chroma == 3 && self.separate_planes) {
                (1, 2 - u32::from(self.frames_only))
            } else {
                (sub_width, sub_height * (2 - u32::from(self.frames_only)))
            };
            let [left, right, top, bottom] = self.crop.unwrap_or_default();
            let width =
                i64::from(self.width_in_blocks) * 16 - i64::from(unit_x) * i64::from(left + right);
            let height =
                i64::from(2 - u32::from(self.frames_only)) * i64::from(self.height_in_units) * 16
                    - i64::from(unit_y) * i64::from(top + bottom);
            (width > 0 && height > 0).then(|| {
                (
                    u32::try_from(width).unwrap(),
                    u32::try_from(height).unwrap(),
                )
            })
        }

        fn rbsp(&self) -> Vec<u8> {
            let mut w = Writer::default();
            w.bits(self.profile, 8);
            // Constraint flags and the level.
            w.bits(0, 8);
            w.bits(40, 8);
            w.ue(0);
            if self.high() {
                w.ue(self.chroma);
                if self.chroma == 3 {
                    w.bit(self.separate_planes);
                }
                w.ue(self.depths[0]);
                w.ue(self.depths[1]);
                w.bit(false);
                match &self.scaling {
                    None => w.bit(false),
                    Some(lists) => {
                        w.bit(true);
                        for (index, list) in lists.iter().enumerate() {
                            match list {
                                None => w.bit(false),
                                Some(steps) => {
                                    w.bit(true);
                                    let size = if index < 6 { 16 } else { 64 };
                                    let (mut last, mut next) = (8_i32, 8_i32);
                                    let mut steps = steps.iter();
                                    for _ in 0..size {
                                        if next != 0 {
                                            let step = steps.next().copied().unwrap_or(0);
                                            w.se(step);
                                            next = (last + step + 256).rem_euclid(256);
                                        }
                                        if next != 0 {
                                            last = next;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            w.ue(self.frame_bits);
            match &self.order {
                Order::Lsb(bits) => {
                    w.ue(0);
                    w.ue(*bits);
                }
                Order::Offsets(offsets) => {
                    w.ue(1);
                    w.bit(false);
                    w.se(-3);
                    w.se(5);
                    w.ue(u32::try_from(offsets.len()).unwrap());
                    for offset in offsets {
                        w.se(*offset);
                    }
                }
                Order::None => w.ue(2),
                Order::Other(kind) => w.ue(*kind),
            }
            w.ue(3);
            w.bit(false);
            w.ue(self.width_in_blocks - 1);
            w.ue(self.height_in_units - 1);
            w.bit(self.frames_only);
            if !self.frames_only {
                w.bit(true);
            }
            w.bit(true);
            match self.crop {
                None => w.bit(false),
                Some(crop) => {
                    w.bit(true);
                    for edge in crop {
                        w.ue(edge);
                    }
                }
            }
            // No video usability information.
            w.bit(false);
            w.finish()
        }

        fn unit(&self) -> Vec<u8> {
            escaped(7, &self.rbsp())
        }
    }

    fn fields() -> impl Strategy<Value = Fields> {
        let lists = |chroma: u32| {
            let count = if chroma == 3 { 12 } else { 8 };
            proptest::option::of(proptest::collection::vec(
                proptest::option::of(proptest::collection::vec(-128..=127_i32, 0..70)),
                count,
            ))
        };
        (
            prop_oneof![
                Just(66_u32),
                Just(77),
                Just(88),
                Just(100),
                Just(110),
                Just(122),
                Just(244),
                Just(44),
                Just(118),
                Just(128),
            ],
            0..=3_u32,
            any::<bool>(),
            prop_oneof![
                (0..40_u32).prop_map(Order::Lsb),
                proptest::collection::vec(-20..20_i32, 0..6).prop_map(Order::Offsets),
                Just(Order::None),
            ],
            (1..=300_u32, 1..=200_u32, any::<bool>()),
            proptest::option::of(proptest::array::uniform4(0..9_u32)),
            (
                proptest::array::uniform2(0..=MAX_DEPTH_MINUS_8),
                prop_oneof![0..=12_u32, Just(u32::MAX - 1)],
            ),
        )
            .prop_flat_map(
                move |(profile, chroma, separate, order, (w, h, only), crop, (depths, bits))| {
                    lists(chroma).prop_map(move |scaling| Fields {
                        profile,
                        chroma,
                        separate_planes: separate,
                        depths,
                        scaling,
                        frame_bits: bits,
                        order: order.clone(),
                        width_in_blocks: w,
                        height_in_units: h,
                        frames_only: only,
                        crop,
                    })
                },
            )
    }

    proptest! {
        #[test]
        fn the_size_is_what_the_fields_say(fields in fields()) {
            prop_assert_eq!(picture_size(&fields.unit()), fields.size());
        }

        #[test]
        fn a_set_that_is_cut_short_gives_the_size_or_nothing(fields in fields()) {
            let unit = fields.unit();
            let size = picture_size(&unit);
            for length in 0..unit.len() {
                let cut = picture_size(&unit[..length]);
                prop_assert!(cut.is_none() || cut == size, "cut at {length}");
            }
        }

        #[test]
        fn what_is_not_a_set_does_not_panic(body in proptest::collection::vec(any::<u8>(), 0..80)) {
            let mut unit = vec![0x67];
            unit.extend_from_slice(&body);
            let _ = picture_size(&unit);
            let _ = picture_size(&body);
        }

        #[test]
        fn emulation_prevention_is_taken_out(payload in proptest::collection::vec(
            prop_oneof![Just(0_u8), Just(1), Just(2), Just(3), any::<u8>()],
            0..64,
        )) {
            let unit = escaped(7, &payload);
            prop_assert_eq!(unescaped(&unit[1..]), payload);
        }
    }

    fn base() -> Fields {
        Fields {
            profile: 100,
            chroma: 1,
            separate_planes: false,
            depths: [0, 0],
            scaling: None,
            frame_bits: 4,
            order: Order::Lsb(4),
            width_in_blocks: 120,
            height_in_units: 68,
            frames_only: true,
            crop: Some([0, 0, 0, 4]),
        }
    }

    #[test]
    fn a_set_with_the_cropping_of_a_full_hd_picture_gives_1920_by_1080() {
        assert_eq!(picture_size(&base().unit()), Some((1920, 1080)));
    }

    #[test]
    fn the_cropping_counts_in_units_of_the_chroma_samples() {
        let size = |chroma, frames_only, crop| {
            let fields = Fields {
                chroma,
                frames_only,
                crop: Some(crop),
                width_in_blocks: 4,
                height_in_units: 4,
                ..base()
            };
            picture_size(&fields.unit())
        };
        // 64 by 64 blocks of pixels, with two on each side taken off.
        assert_eq!(size(1, true, [2, 2, 2, 2]), Some((56, 56)));
        assert_eq!(size(2, true, [2, 2, 2, 2]), Some((56, 60)));
        assert_eq!(size(3, true, [2, 2, 2, 2]), Some((60, 60)));
        assert_eq!(size(0, true, [2, 2, 2, 2]), Some((60, 60)));
        // A frame that may be coded as two fields has twice the rows to a unit.
        assert_eq!(size(1, false, [2, 2, 2, 2]), Some((56, 112)));
        assert_eq!(size(2, false, [2, 2, 2, 2]), Some((56, 120)));
        assert_eq!(size(3, false, [2, 2, 2, 2]), Some((60, 120)));
    }

    #[test]
    fn colour_planes_that_are_apart_are_cropped_like_a_picture_with_no_colour() {
        let fields = Fields {
            chroma: 3,
            separate_planes: true,
            crop: Some([1, 1, 1, 1]),
            width_in_blocks: 4,
            height_in_units: 4,
            ..base()
        };
        assert_eq!(picture_size(&fields.unit()), Some((62, 62)));
    }

    #[test]
    fn a_set_that_crops_everything_away_or_more_has_no_size() {
        for crop in [[8, 0, 0, 0], [0, 0, 16, 0], [5, 5, 0, 0], [0, 0, 9, 9]] {
            let fields = Fields {
                width_in_blocks: 1,
                height_in_units: 1,
                crop: Some(crop),
                ..base()
            };
            assert_eq!(picture_size(&fields.unit()), None, "{crop:?}");
        }
    }

    #[test]
    fn a_picture_wider_than_32_bits_has_no_size() {
        // The largest number of blocks that the code can hold.
        let fields = Fields {
            width_in_blocks: u32::MAX,
            crop: None,
            ..base()
        };
        assert_eq!(picture_size(&fields.unit()), None);
    }

    #[test]
    fn what_is_not_a_sequence_parameter_set_has_no_size() {
        let unit = base().unit();
        let with_header = |header: u8| {
            let mut unit = unit.clone();
            unit[0] = header;
            unit
        };
        assert!(picture_size(&with_header(0x67)).is_some());
        // A picture parameter set, a subset sequence parameter set, a slice, and a unit with its
        // forbidden bit set.
        for header in [0x68, 0x6f, 0x65, 0xe7] {
            assert_eq!(picture_size(&with_header(header)), None, "{header:#x}");
        }
        assert_eq!(picture_size(&[]), None);
        assert_eq!(picture_size(&[0x67]), None);
    }

    #[test]
    fn numbers_that_the_standard_does_not_allow_are_refused() {
        // A chroma format that does not exist.
        let fields = Fields {
            chroma: 4,
            ..base()
        };
        assert_eq!(picture_size(&fields.unit()), None);
        // A picture order type that does not exist. The rest of the set is whole, so a reader
        // that went on would find the size.
        let no_such_type = Fields {
            order: Order::Other(3),
            ..base()
        };
        assert_eq!(picture_size(&no_such_type.unit()), None);
        // More offsets in the cycle of picture order type 1 than the standard allows.
        let too_many = Fields {
            order: Order::Offsets(vec![1; 256]),
            ..base()
        };
        assert_eq!(picture_size(&too_many.unit()), None);
        let enough = Fields {
            order: Order::Offsets(vec![1; 255]),
            ..base()
        };
        assert_eq!(picture_size(&enough.unit()), Some((1920, 1080)));
    }

    #[test]
    fn a_bit_depth_beyond_14_bits_is_refused() {
        let size = |depths| picture_size(&Fields { depths, ..base() }.unit());
        assert_eq!(size([6, 6]), Some((1920, 1080)));
        assert_eq!(size([7, 0]), None);
        assert_eq!(size([0, 7]), None);
    }

    #[test]
    fn the_largest_number_that_a_code_holds_is_read_and_one_more_is_refused() {
        // 31 zeros, a one and 31 bits: 4294967294, in a field that nothing here reads.
        let largest = Fields {
            frame_bits: u32::MAX - 1,
            ..base()
        };
        assert_eq!(picture_size(&largest.unit()), Some((1920, 1080)));
        // 32 zeros: a number that does not fit in 32 bits.
        let one_more = Fields {
            frame_bits: u32::MAX,
            ..base()
        };
        assert_eq!(picture_size(&one_more.unit()), None);
    }

    #[test]
    fn a_number_that_does_not_end_is_refused() {
        // 48 zeros, and then enough ones.
        let unit = [0x67, 66, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(picture_size(&unit), None);
    }

    #[test]
    fn scaling_lists_are_read_through_to_the_size() {
        // The scaling matrix of a set in 4:4:4 has twelve lists, one of which ends early.
        let fields = Fields {
            profile: 244,
            chroma: 3,
            scaling: Some(
                (0..12)
                    .map(|index| match index % 3 {
                        0 => None,
                        1 => Some(vec![-8, 3, 2, 0, 5]),
                        // The first step makes the next scale zero, which ends the list.
                        _ => Some(vec![-8]),
                    })
                    .collect(),
            ),
            ..base()
        };
        // The four rows that are cropped at the bottom count for one row each in 4:4:4.
        assert_eq!(picture_size(&fields.unit()), Some((1920, 1084)));
    }
}
