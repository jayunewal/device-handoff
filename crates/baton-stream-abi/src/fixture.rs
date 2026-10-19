// SPDX-License-Identifier: Apache-2.0

//! Pictures from a real encoder (x264) for the tests, and the means to take them apart and put
//! them together again.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// Three pictures from a real encoder (x264): a key frame, a delta frame and a key frame,
/// each 64 by 48 pixels, with the delimiters and the parameter sets that it writes.
pub(crate) const ENCODER_OUTPUT: &[u8] =
    include_bytes!("../../baton-stream/tests/fixtures/encoder-three-pictures.h264");
pub(crate) const WIDTH: u32 = 64;
pub(crate) const HEIGHT: u32 = 48;

pub(crate) const SLICE: u8 = 1;
pub(crate) const IDR: u8 = 5;
pub(crate) const SEI: u8 = 6;
pub(crate) const SPS: u8 = 7;
pub(crate) const PPS: u8 = 8;
pub(crate) const START: [u8; 4] = [0, 0, 0, 1];

/// The pictures of the stream, split before each delimiter.
pub(crate) fn pictures() -> Vec<Vec<u8>> {
    let mut starts: Vec<usize> = ENCODER_OUTPUT
        .windows(5)
        .enumerate()
        .filter(|(_, window)| *window == [0, 0, 0, 1, 9])
        .map(|(at, _)| at)
        .collect();
    starts.push(ENCODER_OUTPUT.len());
    starts
        .windows(2)
        .map(|pair| ENCODER_OUTPUT[pair[0]..pair[1]].to_vec())
        .collect()
}

/// The units of a picture, without their start codes (an encoder writes three bytes for some
/// and four for others).
pub(crate) fn units(picture: &[u8]) -> Vec<Vec<u8>> {
    let starts: Vec<usize> = (0..picture.len().saturating_sub(2))
        .filter(|at| picture[*at..*at + 3] == [0, 0, 1])
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(number, at)| {
            let end = starts.get(number + 1).map_or(picture.len(), |next| {
                // The zero before a start code of four bytes is not the unit's.
                if *next > 0 && picture[*next - 1] == 0 {
                    next - 1
                } else {
                    *next
                }
            });
            picture[at + 3..end].to_vec()
        })
        .collect()
}

pub(crate) fn join(units: &[Vec<u8>]) -> Vec<u8> {
    units
        .iter()
        .flat_map(|unit| START.iter().chain(unit))
        .copied()
        .collect()
}

pub(crate) fn kind(unit: &[u8]) -> u8 {
    unit[0] & 0x1f
}

/// The picture without the units of these kinds.
pub(crate) fn without(picture: &[u8], kinds: &[u8]) -> Vec<u8> {
    let kept: Vec<Vec<u8>> = units(picture)
        .into_iter()
        .filter(|unit| !kinds.contains(&kind(unit)))
        .collect();
    join(&kept)
}

/// The picture with `unit` in front of its first unit.
pub(crate) fn with_first(picture: &[u8], unit: &[u8]) -> Vec<u8> {
    let mut all = vec![unit.to_vec()];
    all.extend(units(picture));
    join(&all)
}

/// The kinds of the units in a sample in AVCC form.
pub(crate) fn avcc_kinds(mut sample: &[u8]) -> Vec<u8> {
    let mut kinds = Vec::new();
    while !sample.is_empty() {
        let (length, rest) = sample.split_at(4);
        let length = u32::from_be_bytes(length.try_into().unwrap()) as usize;
        let (unit, rest) = rest.split_at(length);
        kinds.push(kind(unit));
        sample = rest;
    }
    kinds
}

/// Bits, the first of a byte first, as a unit of H.264 is written.
struct Bits {
    bytes: Vec<u8>,
    count: u32,
}

impl Bits {
    /// The `count` lowest bits of `bits`, the highest first.
    fn put(&mut self, bits: u32, count: u32) {
        for shift in (0..count).rev() {
            if self.count.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = u8::try_from((bits >> shift) & 1).unwrap();
            *self.bytes.last_mut().unwrap() |= bit << (7 - self.count % 8);
            self.count += 1;
        }
    }

    /// A number in the Exp-Golomb form that the standard calls ue(v).
    fn ue(&mut self, number: u32) {
        let value = number + 1;
        let length = 32 - value.leading_zeros();
        self.put(0, length - 1);
        self.put(value, length);
    }
}

/// The bytes of a unit with a 3 put after two zeros that a byte of 3 or less follows, so that no
/// start code can appear in it.
fn prevented(bytes: &[u8]) -> Vec<u8> {
    let mut written = Vec::new();
    let mut zeros = 0;
    for byte in bytes {
        if zeros >= 2 && *byte <= 3 {
            written.push(3);
            zeros = 0;
        }
        written.push(*byte);
        zeros = if *byte == 0 { zeros + 1 } else { 0 };
    }
    written
}

/// A sequence parameter set (a unit, without a start code) for a progressive picture of `blocks`
/// macroblocks of 16 pixels across and down, in the main profile, with `crop` taken off its right
/// and bottom edges in units of two pixels.
pub(crate) fn sps(blocks: (u32, u32), crop: (u32, u32)) -> Vec<u8> {
    let mut bits = Bits {
        bytes: Vec::new(),
        count: 0,
    };
    bits.put(77, 8);
    bits.put(0, 8);
    bits.put(30, 8);
    // The number of the set.
    bits.ue(0);
    // The bits of the frame number: 4. The order of display is the order of decoding.
    bits.ue(0);
    bits.ue(2);
    // One picture to refer to, and no gaps in the numbers of frames.
    bits.ue(1);
    bits.put(0, 1);
    bits.ue(blocks.0 - 1);
    bits.ue(blocks.1 - 1);
    // Only frames, and the inference of direct prediction that the main profile needs.
    bits.put(1, 1);
    bits.put(1, 1);
    let cropped = crop != (0, 0);
    bits.put(u32::from(cropped), 1);
    if cropped {
        bits.ue(0);
        bits.ue(crop.0);
        bits.ue(0);
        bits.ue(crop.1);
    }
    // No video usability information, then the stop bit.
    bits.put(0, 1);
    bits.put(1, 1);
    let mut unit = vec![0x67];
    unit.extend(prevented(&bits.bytes));
    unit
}
