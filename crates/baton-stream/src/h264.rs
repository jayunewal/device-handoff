// SPDX-License-Identifier: Apache-2.0

//! H.264 pictures on the wire, and in the encoder and the decoder of a platform.
//!
//! A picture on the wire is H.264 in Annex B form (`docs/protocol.md`, section 9): NAL units,
//! each behind a start code, with the parameter sets in front of every key frame. The encoder
//! and the decoder of an Apple device work with the AVCC form instead: each NAL unit behind its
//! length, and the parameter sets kept apart in a description of the format. This module turns
//! one into the other, so that the platform code only hands over the bytes it has and the
//! conversions can be tested on any computer.
//!
//! - [`avcc_to_annex_b`] is for the source: a sample from the encoder becomes a picture for the
//!   wire, and a key frame gets the parameter sets of the format in front of it.
//! - [`annex_b_to_avcc`] is for the viewer: a picture from the wire becomes a sample for the
//!   decoder, and the parameter sets come out of it for the description of the format.
//!
//! The NAL units themselves are not changed: both forms hold them with their emulation
//! prevention bytes in, so only what lies between them differs. The picture comes from a paired
//! device, which may be broken, so every length is checked and nothing here panics. Each
//! function finds out whether it can convert before it writes, so an error leaves the output as
//! it was.

/// A slice of a picture that is not an IDR picture.
pub const NAL_SLICE: u8 = 1;
/// A slice of an IDR picture, which can be decoded alone.
pub const NAL_IDR: u8 = 5;
/// Supplemental enhancement information.
pub const NAL_SEI: u8 = 6;
/// A sequence parameter set.
pub const NAL_SPS: u8 = 7;
/// A picture parameter set.
pub const NAL_PPS: u8 = 8;
/// An access unit delimiter.
pub const NAL_AUD: u8 = 9;
/// Filler data.
pub const NAL_FILLER: u8 = 12;

/// The most NAL units that a picture may hold. A picture of a window has one slice or a few,
/// and its parameter sets; a picture with more is not one that an encoder of ours made.
pub const MAX_UNITS: usize = 256;

/// The length of the length field that the output of [`annex_b_to_avcc`] has. The description of
/// the format that the platform makes from the parameter sets says so.
pub const AVCC_LENGTH_SIZE: usize = 4;

/// Why a picture was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// AVCC has the length of a NAL unit in 1, 2 or 4 bytes.
    #[error("a length field of {0} bytes is not one that AVCC uses")]
    LengthSize(usize),
    /// A length reaches beyond the end of the sample, or the sample ends inside a length.
    #[error("a NAL unit is cut short")]
    Truncated,
    /// A NAL unit with no byte in it.
    #[error("a NAL unit is empty")]
    Empty,
    /// The first bit of a NAL unit is always zero. One that is not is not H.264.
    #[error("a NAL unit has its forbidden bit set")]
    Forbidden,
    /// Bytes other than zeros come before the first start code.
    #[error("the picture does not begin with a start code")]
    NoStartCode,
    /// Nothing is left that a decoder or the wire takes: no NAL unit, or only parameter sets,
    /// delimiters and filler.
    #[error("the picture holds no NAL unit that is of use")]
    NoUnits,
    /// The picture has more NAL units than [`MAX_UNITS`].
    #[error("the picture holds more than {MAX_UNITS} NAL units")]
    TooManyUnits,
    /// A NAL unit of 4 GiB or more, which no length field of AVCC can hold.
    #[error("a NAL unit is too long for its length")]
    TooLarge,
    /// A NAL unit that holds a start code, or the zeros that could make one, or ends in a zero.
    /// On the wire it could not be told from the NAL units around it. An encoder keeps to this
    /// with emulation prevention.
    #[error("a NAL unit could be taken for more than one")]
    Ambiguous,
    /// A parameter set that is not one: empty, or not of the kind its place says.
    #[error("a parameter set is not valid")]
    ParameterSet,
    /// A key frame is to be written, and the parameter sets given lack a sequence parameter set
    /// or a picture parameter set.
    #[error("a key frame needs a sequence parameter set and a picture parameter set")]
    NoParameterSets,
}

/// The kind of a NAL unit: the low five bits of its first byte. A unit with nothing in it, or
/// with its forbidden bit set, has none.
fn kind(unit: &[u8]) -> Option<u8> {
    let first = *unit.first()?;
    (first & 0x80 == 0).then_some(first & 0x1f)
}

/// Whether a NAL unit can be written to the wire as it is and read back as one unit. It holds
/// none of the byte sequences `0 0 0`, `0 0 1` and `0 0 2`, which emulation prevention rules out,
/// so that no start code is inside it, and it does not end in a zero, which would be taken for
/// one of the zeros that come before the next start code.
fn is_clear(unit: &[u8]) -> bool {
    unit.last() != Some(&0)
        && !unit
            .windows(3)
            .any(|window| matches!(window, [0, 0, 0..=2]))
}

/// What a picture that was made for the wire holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Whether it has a slice of an IDR picture, so that a decoder can start at it. The key flag
    /// of the header of its frame says so.
    pub idr: bool,
    /// How many NAL units it has, parameter sets included.
    pub units: usize,
}

/// A picture on the wire, taken apart for the decoder of a platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForDecoder<'a> {
    /// The sequence parameter sets of the picture, without start codes, in the order they came.
    pub sps: Vec<&'a [u8]>,
    /// The picture parameter sets, likewise.
    pub pps: Vec<&'a [u8]>,
    /// Whether it has a slice of an IDR picture.
    pub idr: bool,
}

impl ForDecoder<'_> {
    /// The parameter sets in the order that the description of a format takes them: the
    /// sequence parameter sets, then the picture parameter sets.
    pub fn parameter_sets(&self) -> impl Iterator<Item = &[u8]> {
        self.sps.iter().chain(&self.pps).copied()
    }
}

/// Splits a sample in AVCC form into its NAL units.
fn avcc_units(sample: &[u8], length_size: usize) -> Result<Vec<&[u8]>, Error> {
    if !matches!(length_size, 1 | 2 | 4) {
        return Err(Error::LengthSize(length_size));
    }
    let mut units = Vec::new();
    let mut rest = sample;
    while !rest.is_empty() {
        let (prefix, after) = rest.split_at_checked(length_size).ok_or(Error::Truncated)?;
        let length = prefix
            .iter()
            .fold(0_usize, |length, byte| (length << 8) | usize::from(*byte));
        let (unit, after) = after.split_at_checked(length).ok_or(Error::Truncated)?;
        if kind(unit).is_none() {
            return Err(if unit.is_empty() {
                Error::Empty
            } else {
                Error::Forbidden
            });
        }
        if units.len() == MAX_UNITS {
            return Err(Error::TooManyUnits);
        }
        units.push(unit);
        rest = after;
    }
    Ok(units)
}

/// Where the next start code is: the position of the three bytes `0 0 1`.
fn start_code(bytes: &[u8]) -> Option<usize> {
    bytes.windows(3).position(|window| window == [0, 0, 1])
}

/// Splits a picture in Annex B form into its NAL units. A start code is `0 0 1`, and a fourth
/// byte of zero in front of it, or more, belongs to the NAL unit before. The NAL units that have
/// nothing in them are left out.
fn annex_b_units(picture: &[u8]) -> Result<Vec<&[u8]>, Error> {
    let first = start_code(picture).ok_or(Error::NoStartCode)?;
    if picture
        .get(..first)
        .is_none_or(|before| before.iter().any(|byte| *byte != 0))
    {
        return Err(Error::NoStartCode);
    }
    let mut units = Vec::new();
    let mut rest = picture.get(first + 3..).unwrap_or_default();
    loop {
        let (unit, next) = match start_code(rest) {
            Some(at) => (rest.get(..at), rest.get(at + 3..)),
            None => (Some(rest), None),
        };
        // Zeros at the end of a NAL unit are those of the next start code, or filling. A unit
        // never ends in one itself, because the last byte of a unit is not zero.
        let unit = unit.unwrap_or_default();
        let end = unit
            .iter()
            .rposition(|byte| *byte != 0)
            .map_or(0, |at| at + 1);
        let unit = unit.get(..end).unwrap_or_default();
        if !unit.is_empty() {
            if kind(unit).is_none() {
                return Err(Error::Forbidden);
            }
            if units.len() == MAX_UNITS {
                return Err(Error::TooManyUnits);
            }
            units.push(unit);
        }
        match next {
            Some(next) => rest = next,
            None => break,
        }
    }
    Ok(units)
}

/// Makes the sample that a decoder takes from a picture in Annex B form, which is what comes
/// off the wire.
///
/// The NAL units come out in AVCC form with a length of [`AVCC_LENGTH_SIZE`] bytes, appended to
/// `out`. The parameter sets and the delimiters and filler of the picture are not in it: the
/// parameter sets are what the platform makes the description of the format from, and the
/// other two say nothing the decoder needs. A picture with no key frame has no parameter sets,
/// and the platform keeps the description it has.
///
/// # Errors
///
/// The picture does not begin with a start code, has a NAL unit with its forbidden bit set, has
/// more NAL units than [`MAX_UNITS`], or has none that the decoder takes.
pub fn annex_b_to_avcc<'a>(picture: &'a [u8], out: &mut Vec<u8>) -> Result<ForDecoder<'a>, Error> {
    let mut taken = ForDecoder {
        sps: Vec::new(),
        pps: Vec::new(),
        idr: false,
    };
    let mut for_decoder = Vec::new();
    for unit in annex_b_units(picture)? {
        match kind(unit) {
            Some(NAL_SPS) => taken.sps.push(unit),
            Some(NAL_PPS) => taken.pps.push(unit),
            Some(NAL_AUD | NAL_FILLER) => {}
            other => {
                taken.idr |= other == Some(NAL_IDR);
                let length = u32::try_from(unit.len()).map_err(|_| Error::TooLarge)?;
                for_decoder.push((length, unit));
            }
        }
    }
    if for_decoder.is_empty() {
        return Err(Error::NoUnits);
    }
    for (length, unit) in for_decoder {
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(unit);
    }
    Ok(taken)
}

/// The sequence parameter sets and then the picture parameter sets of those given, whatever
/// order they were given in.
fn in_order<'a>(parameter_sets: &[&'a [u8]]) -> Result<Vec<&'a [u8]>, Error> {
    let (mut sequence, mut picture) = (Vec::new(), Vec::new());
    for set in parameter_sets {
        match kind(set) {
            Some(NAL_SPS) if is_clear(set) => sequence.push(*set),
            Some(NAL_PPS) if is_clear(set) => picture.push(*set),
            _ => return Err(Error::ParameterSet),
        }
    }
    if sequence.is_empty() || picture.is_empty() {
        return Err(Error::NoParameterSets);
    }
    sequence.append(&mut picture);
    Ok(sequence)
}

/// Makes the picture for the wire from a sample that the encoder made, in AVCC form with
/// lengths of `length_size` bytes (1, 2 or 4), appended to `out`.
///
/// The encoder keeps the parameter sets in the description of its format and not in its samples,
/// so the platform passes them as `parameter_sets`, the sequence parameter sets and the picture
/// parameter sets in any order. A sample that has a slice of an IDR picture is a key frame, and
/// they are written in front of it, as every key frame on the wire holds them; any other sample
/// is written without them and `parameter_sets` is not looked at, so the platform may pass them
/// with every sample. Parameter sets, delimiters and filler that the sample has itself are not
/// copied.
///
/// # Errors
///
/// `length_size` is not one that AVCC uses, a length reaches beyond the end of the sample, a NAL
/// unit is empty, has its forbidden bit set or could be taken for more than one on the wire, the
/// sample holds more than [`MAX_UNITS`] of them or none that goes on the wire, or it is a key
/// frame and `parameter_sets` has an entry that is not a parameter set or lacks one of the two
/// kinds.
pub fn avcc_to_annex_b(
    sample: &[u8],
    length_size: usize,
    parameter_sets: &[&[u8]],
    out: &mut Vec<u8>,
) -> Result<Summary, Error> {
    let units: Vec<&[u8]> = avcc_units(sample, length_size)?
        .into_iter()
        .filter(|unit| !matches!(kind(unit), Some(NAL_SPS | NAL_PPS | NAL_AUD | NAL_FILLER)))
        .collect();
    if units.is_empty() {
        return Err(Error::NoUnits);
    }
    if !units.iter().all(|unit| is_clear(unit)) {
        return Err(Error::Ambiguous);
    }
    let idr = units.iter().any(|unit| kind(unit) == Some(NAL_IDR));
    let sets = if idr {
        in_order(parameter_sets)?
    } else {
        Vec::new()
    };
    let count = sets.len() + units.len();
    if count > MAX_UNITS {
        return Err(Error::TooManyUnits);
    }
    for unit in sets.into_iter().chain(units) {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(unit);
    }
    Ok(Summary { idr, units: count })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// A NAL unit of `kind` with `body` after its header byte, with the emulation prevention
    /// that an encoder applies, so that no start code can appear inside it.
    fn unit(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut unit = vec![0x60 | kind];
        let mut zeros = 0;
        for byte in body {
            if zeros >= 2 && *byte <= 3 {
                unit.push(3);
                zeros = 0;
            }
            unit.push(*byte);
            zeros = if *byte == 0 { zeros + 1 } else { 0 };
        }
        // A unit never ends in a zero.
        if unit.last() == Some(&0) {
            unit.push(3);
        }
        unit
    }

    fn annex_b(units: &[&[u8]]) -> Vec<u8> {
        let mut picture = Vec::new();
        for unit in units {
            picture.extend_from_slice(&[0, 0, 0, 1]);
            picture.extend_from_slice(unit);
        }
        picture
    }

    fn avcc(units: &[&[u8]], length_size: usize) -> Vec<u8> {
        let mut sample = Vec::new();
        for unit in units {
            let length = unit.len().to_be_bytes();
            sample.extend_from_slice(&length[length.len() - length_size..]);
            sample.extend_from_slice(unit);
        }
        sample
    }

    /// A sample, the length of its length field, the parameter sets given with it, and the
    /// error that it is refused with.
    type Case<'a> = (&'a [u8], usize, Vec<&'a [u8]>, Error);

    fn sps() -> Vec<u8> {
        unit(NAL_SPS, &[0x42, 0x00, 0x1f, 0xe9, 0x40, 0x5a, 0x1e])
    }

    fn pps() -> Vec<u8> {
        unit(NAL_PPS, &[0xce, 0x3c, 0x80])
    }

    fn idr() -> Vec<u8> {
        unit(NAL_IDR, &[0x88, 0x84, 0x00, 0x21, 0xff, 0xfe])
    }

    fn delta() -> Vec<u8> {
        unit(NAL_SLICE, &[0x9a, 0x01, 0x02, 0x03, 0x04])
    }

    #[test]
    fn a_key_frame_off_the_wire_gives_its_parameter_sets_and_a_sample() {
        let (sps, pps, idr) = (sps(), pps(), idr());
        let picture = annex_b(&[&sps, &pps, &idr]);
        let mut out = Vec::new();
        let taken = annex_b_to_avcc(&picture, &mut out).unwrap();
        assert_eq!(taken.sps, [&sps[..]]);
        assert_eq!(taken.pps, [&pps[..]]);
        assert!(taken.idr);
        assert_eq!(out, avcc(&[&idr], 4));
        assert_eq!(
            taken.parameter_sets().collect::<Vec<_>>(),
            [&sps[..], &pps[..]]
        );
    }

    #[test]
    fn a_delta_frame_has_no_parameter_sets_and_is_not_an_idr_picture() {
        let delta = delta();
        let picture = annex_b(&[&delta]);
        let mut out = vec![9, 9];
        let taken = annex_b_to_avcc(&picture, &mut out).unwrap();
        assert!(taken.sps.is_empty() && taken.pps.is_empty() && !taken.idr);
        // The sample is added to what was there.
        assert_eq!(out, [&[9, 9][..], &avcc(&[&delta], 4)].concat());
    }

    #[test]
    fn the_sets_of_a_picture_come_out_in_the_order_the_format_takes_them() {
        let (sps, pps, idr) = (sps(), pps(), idr());
        let other = unit(NAL_SPS, &[0x42, 0x00, 0x28]);
        let picture = annex_b(&[&pps, &sps, &idr, &other]);
        let taken = annex_b_to_avcc(&picture, &mut Vec::new()).unwrap();
        assert_eq!(
            taken.parameter_sets().collect::<Vec<_>>(),
            [&sps[..], &other[..], &pps[..]]
        );
    }

    #[test]
    fn start_codes_of_three_and_four_bytes_and_zeros_around_them_are_read() {
        let (sps, pps, idr) = (sps(), pps(), idr());
        let mut picture = vec![0, 0, 0, 0, 0, 1];
        picture.extend_from_slice(&sps);
        picture.extend_from_slice(&[0, 0, 1]);
        picture.extend_from_slice(&pps);
        // Filling between the units, and a fourth zero before a start code.
        picture.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1]);
        picture.extend_from_slice(&idr);
        picture.extend_from_slice(&[0, 0, 0]);
        let mut out = Vec::new();
        let taken = annex_b_to_avcc(&picture, &mut out).unwrap();
        assert_eq!(taken.sps, [&sps[..]]);
        assert_eq!(taken.pps, [&pps[..]]);
        assert_eq!(out, avcc(&[&idr], 4));
    }

    #[test]
    fn units_with_nothing_in_them_are_left_out() {
        let idr = idr();
        let mut picture = vec![0, 0, 1, 0, 0, 1];
        picture.extend_from_slice(&idr);
        let mut out = Vec::new();
        annex_b_to_avcc(&picture, &mut out).unwrap();
        assert_eq!(out, avcc(&[&idr], 4));
    }

    #[test]
    fn delimiters_and_filler_do_not_reach_the_decoder() {
        let (aud, filler, delta) = (unit(NAL_AUD, &[0xf0]), unit(NAL_FILLER, &[0xff]), delta());
        let picture = annex_b(&[&aud, &delta, &filler]);
        let mut out = Vec::new();
        annex_b_to_avcc(&picture, &mut out).unwrap();
        assert_eq!(out, avcc(&[&delta], 4));
    }

    #[test]
    fn supplemental_information_goes_on_to_the_decoder() {
        let (sei, idr) = (unit(NAL_SEI, &[1, 2, 3]), idr());
        let picture = annex_b(&[&sei, &idr]);
        let mut out = Vec::new();
        annex_b_to_avcc(&picture, &mut out).unwrap();
        assert_eq!(out, avcc(&[&sei, &idr], 4));
    }

    #[test]
    fn a_picture_that_does_not_begin_with_a_start_code_is_refused() {
        let mut out = vec![1];
        for picture in [
            &[][..],
            &[1, 2, 3][..],
            &[0, 0, 0][..],
            // Something before the start code.
            &[7, 0, 0, 1, 0x65, 1][..],
            &[0, 0, 0, 1][..],
        ] {
            let error = annex_b_to_avcc(picture, &mut out).unwrap_err();
            assert!(
                matches!(error, Error::NoStartCode | Error::NoUnits),
                "{picture:?}: {error}"
            );
        }
        assert_eq!(out, [1]);
    }

    #[test]
    fn a_unit_with_the_forbidden_bit_set_is_refused() {
        let mut out = Vec::new();
        let picture = annex_b(&[&[0xe5, 1, 2]]);
        assert_eq!(annex_b_to_avcc(&picture, &mut out), Err(Error::Forbidden));
        assert_eq!(out, Vec::<u8>::new());
    }

    #[test]
    fn a_picture_of_parameter_sets_alone_has_nothing_for_the_decoder() {
        let picture = annex_b(&[&sps(), &pps()]);
        let mut out = Vec::new();
        assert_eq!(annex_b_to_avcc(&picture, &mut out), Err(Error::NoUnits));
        assert_eq!(out, Vec::<u8>::new());
    }

    #[test]
    fn more_units_than_the_limit_are_refused() {
        let delta = delta();
        let units = vec![&delta[..]; MAX_UNITS];
        let mut out = Vec::new();
        annex_b_to_avcc(&annex_b(&units), &mut out).unwrap();
        out.clear();
        avcc_to_annex_b(&avcc(&units, 4), 4, &[], &mut out).unwrap();
        out.clear();
        let units = vec![&delta[..]; MAX_UNITS + 1];
        assert_eq!(
            annex_b_to_avcc(&annex_b(&units), &mut out),
            Err(Error::TooManyUnits)
        );
        assert_eq!(
            avcc_to_annex_b(&avcc(&units, 4), 4, &[], &mut out),
            Err(Error::TooManyUnits)
        );
        assert_eq!(out, Vec::<u8>::new());
    }

    #[test]
    fn a_sample_is_held_to_the_limit_whatever_is_left_out_of_it() {
        let (filler, delta) = (unit(NAL_FILLER, &[0xff]), delta());
        let mut units = vec![&filler[..]; MAX_UNITS - 1];
        units.push(&delta);
        let mut out = Vec::new();
        avcc_to_annex_b(&avcc(&units, 4), 4, &[], &mut out).unwrap();
        assert_eq!(out, annex_b(&[&delta]));
        out.clear();
        units.push(&filler);
        assert_eq!(
            avcc_to_annex_b(&avcc(&units, 4), 4, &[], &mut out),
            Err(Error::TooManyUnits)
        );
        assert_eq!(out, Vec::<u8>::new());
    }

    #[test]
    fn a_length_of_more_than_one_byte_is_read_from_all_of_its_bytes() {
        // Units of 255, 256, 300, 65 535, 65 536 and 70 000 bytes, with the shortest length
        // field that holds them and the longer ones.
        for (size, length_sizes) in [
            (255_usize, &[1, 2, 4][..]),
            (256, &[2, 4][..]),
            (300, &[2, 4][..]),
            (65_535, &[2, 4][..]),
            (65_536, &[4][..]),
            (70_000, &[4][..]),
        ] {
            let slice = unit(NAL_SLICE, &vec![0xaa; size - 1]);
            assert_eq!(slice.len(), size);
            for length_size in length_sizes {
                let mut out = Vec::new();
                let sample = avcc(&[&slice], *length_size);
                avcc_to_annex_b(&sample, *length_size, &[], &mut out).unwrap();
                assert_eq!(out, annex_b(&[&slice]), "{size} bytes in {length_size}");
            }
            let mut out = Vec::new();
            annex_b_to_avcc(&annex_b(&[&slice]), &mut out).unwrap();
            assert_eq!(out, avcc(&[&slice], 4), "{size} bytes");
        }
    }

    #[test]
    fn the_parameter_sets_of_a_key_frame_count_towards_the_limit() {
        let (sps, pps, idr, delta) = (sps(), pps(), idr(), delta());
        let mut units = vec![&delta[..]; MAX_UNITS - 3];
        units.push(&idr);
        let mut out = Vec::new();
        let summary = avcc_to_annex_b(&avcc(&units, 4), 4, &[&sps, &pps], &mut out).unwrap();
        assert_eq!(summary.units, MAX_UNITS);
        // The picture that was written is one that the other direction takes.
        annex_b_to_avcc(&out, &mut Vec::new()).unwrap();
        out.clear();
        units.push(&delta);
        assert_eq!(
            avcc_to_annex_b(&avcc(&units, 4), 4, &[&sps, &pps], &mut out),
            Err(Error::TooManyUnits)
        );
        assert_eq!(out, Vec::<u8>::new());
        // A sample that is not a key frame has no sets to count.
        let units = vec![&delta[..]; MAX_UNITS];
        avcc_to_annex_b(&avcc(&units, 4), 4, &[&sps, &pps], &mut out).unwrap();
    }

    #[test]
    fn a_key_frame_from_the_encoder_gets_its_parameter_sets_in_front() {
        let (sps, pps, idr) = (sps(), pps(), idr());
        for length_size in [1, 2, 4] {
            let sample = avcc(&[&idr], length_size);
            let mut out = Vec::new();
            let summary = avcc_to_annex_b(&sample, length_size, &[&sps, &pps], &mut out).unwrap();
            assert_eq!(
                summary,
                Summary {
                    idr: true,
                    units: 3
                }
            );
            assert_eq!(out, annex_b(&[&sps, &pps, &idr]));
        }
    }

    #[test]
    fn the_sequence_parameter_sets_go_first_whatever_order_they_come_in() {
        let (sps, pps, idr) = (sps(), pps(), idr());
        let other = unit(NAL_SPS, &[0x42, 0x00, 0x28]);
        let mut out = Vec::new();
        let sample = avcc(&[&idr], 4);
        avcc_to_annex_b(&sample, 4, &[&pps, &other, &sps], &mut out).unwrap();
        assert_eq!(out, annex_b(&[&other, &sps, &pps, &idr]));
    }

    #[test]
    fn a_delta_frame_from_the_encoder_is_the_same_units_behind_start_codes() {
        let (a, b) = (delta(), unit(NAL_SLICE, &[0x77, 0x88]));
        let mut out = Vec::new();
        let summary = avcc_to_annex_b(&avcc(&[&a, &b], 4), 4, &[], &mut out).unwrap();
        assert_eq!(
            summary,
            Summary {
                idr: false,
                units: 2
            }
        );
        assert_eq!(out, annex_b(&[&a, &b]));
    }

    #[test]
    fn the_sets_may_come_with_every_sample_but_only_a_key_frame_gets_them() {
        let (sps, pps, delta) = (sps(), pps(), delta());
        let mut out = Vec::new();
        let sets: [&[u8]; 2] = [&sps, &pps];
        avcc_to_annex_b(&avcc(&[&delta], 4), 4, &sets, &mut out).unwrap();
        assert_eq!(out, annex_b(&[&delta]));
        // Nothing is made of sets that are not used.
        out.clear();
        avcc_to_annex_b(&avcc(&[&delta], 4), 4, &[&[]], &mut out).unwrap();
        assert_eq!(out, annex_b(&[&delta]));
    }

    #[test]
    fn what_a_sample_holds_apart_from_the_slices_is_not_copied() {
        let (sps, pps, idr) = (sps(), pps(), idr());
        let (aud, filler) = (unit(NAL_AUD, &[0xf0]), unit(NAL_FILLER, &[0xff]));
        let sei = unit(NAL_SEI, &[1]);
        // The parameter sets that the sample brings are not the ones of the format.
        let own = unit(NAL_SPS, &[0x42, 0x00, 0x28]);
        let sample = avcc(&[&aud, &own, &pps, &sei, &idr, &filler], 4);
        let mut out = Vec::new();
        let summary = avcc_to_annex_b(&sample, 4, &[&sps, &pps], &mut out).unwrap();
        assert_eq!(out, annex_b(&[&sps, &pps, &sei, &idr]));
        assert_eq!(summary.units, 4);
        // A sample of nothing else has nothing for the wire.
        out.clear();
        let sample = avcc(&[&aud, &own, &pps, &filler], 4);
        assert_eq!(
            avcc_to_annex_b(&sample, 4, &[&sps, &pps], &mut out),
            Err(Error::NoUnits)
        );
        assert_eq!(out, Vec::<u8>::new());
    }

    #[test]
    fn a_key_frame_is_not_written_without_both_kinds_of_parameter_sets() {
        let (sps, pps, idr) = (sps(), pps(), idr());
        let sample = avcc(&[&idr], 4);
        let mut out = vec![5];
        let cases: [&[&[u8]]; 4] = [&[], &[&sps], &[&pps], &[&sps, &sps]];
        for sets in cases {
            assert_eq!(
                avcc_to_annex_b(&sample, 4, sets, &mut out),
                Err(Error::NoParameterSets),
                "{sets:?}"
            );
        }
        assert_eq!(out, [5]);
    }

    #[test]
    fn a_unit_that_the_wire_could_not_tell_from_its_neighbours_is_refused() {
        let (sps, pps) = (sps(), pps());
        let mut out = vec![5];
        // Start codes inside a unit, the zeros that lead up to one, and a zero at the end.
        let units: [&[u8]; 6] = [
            &[0x41, 0, 0, 1, 5],
            &[0x41, 0, 0, 0, 5],
            &[0x41, 0, 0, 2, 5],
            &[0x41, 5, 0],
            &[0x41, 5, 0, 0],
            &[0],
        ];
        for unit in units {
            assert_eq!(
                avcc_to_annex_b(&avcc(&[unit], 4), 4, &[], &mut out),
                Err(Error::Ambiguous),
                "{unit:?}"
            );
        }
        // The parameter sets are held to it too.
        let key = avcc(&[&idr()], 4);
        for set in [&[0x67, 0, 0, 1, 5][..], &[0x67, 5, 0][..]] {
            assert_eq!(
                avcc_to_annex_b(&key, 4, &[set, &pps], &mut out),
                Err(Error::ParameterSet),
                "{set:?}"
            );
            assert_eq!(
                avcc_to_annex_b(
                    &key,
                    4,
                    &[&sps, &[&[0x68][..], &set[1..]].concat()],
                    &mut out
                ),
                Err(Error::ParameterSet),
                "{set:?}"
            );
        }
        assert_eq!(out, [5]);
        // What emulation prevention leaves is fine: a zero that is followed by a three, and zeros
        // that are not three in a row.
        let escaped = unit(NAL_SLICE, &[0, 0, 0, 1, 0, 0, 2, 0, 1, 0]);
        assert!(escaped.windows(3).any(|window| window == [0, 0, 3]));
        avcc_to_annex_b(&avcc(&[&escaped], 4), 4, &[], &mut out).unwrap();
    }

    #[test]
    fn a_sample_that_is_not_valid_is_refused_and_leaves_the_output_as_it_was() {
        let (sps, pps, idr, delta) = (sps(), pps(), idr(), delta());
        let mut out = vec![5];
        let good = avcc(&[&delta], 4);
        let mut cut = good.clone();
        cut.pop();
        let key = avcc(&[&idr], 4);
        let cases: [Case<'_>; 10] = [
            (&good, 3, vec![], Error::LengthSize(3)),
            (&good, 0, vec![], Error::LengthSize(0)),
            (&cut, 4, vec![], Error::Truncated),
            (&[0, 0, 1], 4, vec![], Error::Truncated),
            (&[0, 0, 0, 0], 4, vec![], Error::Empty),
            (&[0, 0, 0, 1, 0x80], 4, vec![], Error::Forbidden),
            (&[], 4, vec![], Error::NoUnits),
            // A set that is empty, that says it is a slice, or that has its forbidden bit set.
            (&key, 4, vec![&sps, &pps, &[]], Error::ParameterSet),
            (&key, 4, vec![&sps, &pps, &delta], Error::ParameterSet),
            (&key, 4, vec![&sps, &pps, &[0xe7, 1]], Error::ParameterSet),
        ];
        for (sample, length_size, sets, expected) in cases {
            assert_eq!(
                avcc_to_annex_b(sample, length_size, &sets, &mut out),
                Err(expected),
                "{sample:?}"
            );
            assert_eq!(out, [5]);
        }
    }

    #[test]
    fn a_picture_survives_the_way_there_and_back() {
        let (sps, pps, idr) = (sps(), pps(), idr());
        let sample = avcc(&[&idr], 4);
        let mut wire = Vec::new();
        avcc_to_annex_b(&sample, 4, &[&sps, &pps], &mut wire).unwrap();
        let mut back = Vec::new();
        let taken = annex_b_to_avcc(&wire, &mut back).unwrap();
        assert_eq!(back, sample);
        assert_eq!(
            taken.parameter_sets().collect::<Vec<_>>(),
            [&sps[..], &pps[..]]
        );
    }

    /// Three pictures that a real encoder made (libx264, with `ffmpeg -f lavfi -i
    /// testsrc2=size=64x48:rate=30 -frames:v 3 -c:v libx264 -preset ultrafast -tune zerolatency
    /// -profile:v high -x264-params aud=1:keyint=2:bframes=0:repeat-headers=1:slices=2 -f h264`):
    /// a key frame, a delta frame and a key frame. Each begins with a delimiter. The first key frame
    /// has the parameter sets, a unit of supplemental information with the version of the encoder,
    /// and two slices of an IDR picture; the second has the parameter sets and the slices, and a
    /// delta frame has two slices.
    const ENCODER_OUTPUT: &[u8] = include_bytes!("../tests/fixtures/encoder-three-pictures.h264");

    /// The pictures of a stream, split before each delimiter.
    fn pictures(stream: &[u8]) -> Vec<&[u8]> {
        let delimiter = [0, 0, 0, 1, NAL_AUD];
        let mut starts: Vec<usize> = stream
            .windows(delimiter.len())
            .enumerate()
            .filter(|(_, window)| *window == delimiter)
            .map(|(at, _)| at)
            .collect();
        starts.push(stream.len());
        starts
            .windows(2)
            .map(|pair| &stream[pair[0]..pair[1]])
            .collect()
    }

    #[test]
    fn pictures_of_a_real_encoder_go_to_a_decoder_and_back() {
        let pictures = pictures(ENCODER_OUTPUT);
        assert_eq!(pictures.len(), 3);
        let expected: [(bool, &[u8]); 3] = [
            (true, &[NAL_SEI, NAL_IDR, NAL_IDR]),
            (false, &[NAL_SLICE, NAL_SLICE]),
            (true, &[NAL_IDR, NAL_IDR]),
        ];
        let mut sets: Vec<Vec<u8>> = Vec::new();
        for (picture, (key, kinds)) in pictures.into_iter().zip(expected) {
            let mut sample = Vec::new();
            let taken = annex_b_to_avcc(picture, &mut sample).unwrap();
            assert_eq!(taken.idr, key);
            let counts = (taken.sps.len(), taken.pps.len());
            assert_eq!(counts, if key { (1, 1) } else { (0, 0) });
            if key {
                sets = taken.parameter_sets().map(<[u8]>::to_vec).collect();
            }
            // The sample holds the units for the decoder, behind lengths of four bytes.
            let units = avcc_units(&sample, AVCC_LENGTH_SIZE).unwrap();
            let found: Vec<u8> = units.iter().filter_map(|unit| kind(unit)).collect();
            assert_eq!(found, kinds);
            // What the source writes for that sample is the picture without its delimiter.
            let given: Vec<&[u8]> = sets.iter().map(Vec::as_slice).collect();
            let mut wire = Vec::new();
            let summary = avcc_to_annex_b(&sample, AVCC_LENGTH_SIZE, &given, &mut wire).unwrap();
            assert_eq!(summary.idr, key);
            let before = annex_b_units(picture).unwrap();
            assert_eq!(kind(before[0]), Some(NAL_AUD));
            assert_eq!(annex_b_units(&wire).unwrap(), before[1..]);
        }
    }

    /// A unit as an encoder makes it: any kind that the decoder takes, and a body of bytes that
    /// are often zero, to meet the emulation prevention.
    fn arbitrary_unit() -> impl Strategy<Value = Vec<u8>> {
        (
            prop_oneof![
                Just(NAL_SLICE),
                Just(NAL_IDR),
                Just(NAL_SEI),
                Just(2),
                Just(3)
            ],
            proptest::collection::vec(
                prop_oneof![Just(0_u8), Just(1), Just(3), any::<u8>()],
                0..64,
            ),
        )
            .prop_map(|(kind, body)| unit(kind, &body))
    }

    proptest! {
        #[test]
        fn what_the_encoder_makes_comes_back_whole(
            units in proptest::collection::vec(arbitrary_unit(), 1..12),
            length_size in prop_oneof![Just(1_usize), Just(2), Just(4)],
        ) {
            let units: Vec<&[u8]> = units.iter().map(Vec::as_slice).collect();
            let sample = avcc(&units, length_size);
            let (sps, pps) = (sps(), pps());
            let mut wire = Vec::new();
            let summary = avcc_to_annex_b(&sample, length_size, &[&sps, &pps], &mut wire).unwrap();
            let key = units.iter().any(|unit| kind(unit) == Some(NAL_IDR));
            let sets = if key { 2 } else { 0 };
            prop_assert_eq!(summary.idr, key);
            prop_assert_eq!(summary.units, units.len() + sets);
            let mut back = Vec::new();
            let taken = annex_b_to_avcc(&wire, &mut back).unwrap();
            prop_assert_eq!(back, avcc(&units, 4));
            prop_assert_eq!(taken.sps.len() + taken.pps.len(), sets);
            prop_assert_eq!(taken.idr, key);
        }

        #[test]
        fn nothing_from_the_wire_makes_it_panic_or_grow_beyond_a_quarter_more(
            picture in proptest::collection::vec(
                prop_oneof![Just(0_u8), Just(0), Just(1), any::<u8>()], 0..400),
        ) {
            let mut out = Vec::new();
            if let Ok(taken) = annex_b_to_avcc(&picture, &mut out) {
                // Whatever comes out is a sample that reads back as units of the input.
                let units = avcc_units(&out, AVCC_LENGTH_SIZE).unwrap();
                prop_assert!(!units.is_empty());
                for each in units.iter().chain(&taken.sps).chain(&taken.pps) {
                    prop_assert!(picture.windows(each.len()).any(|window| window == *each));
                }
                prop_assert!(out.len() <= picture.len() + picture.len() / 4 + 4);
            } else {
                prop_assert!(out.is_empty());
            }
        }

        #[test]
        fn nothing_from_the_encoder_makes_it_panic_and_what_is_written_reads_back_as_written(
            sample in proptest::collection::vec(
                prop_oneof![Just(0_u8), Just(1), any::<u8>()], 0..300),
            length_size in 0_usize..6,
        ) {
            let (sps, pps) = (sps(), pps());
            let mut out = Vec::new();
            match avcc_to_annex_b(&sample, length_size, &[&sps, &pps], &mut out) {
                Ok(summary) => {
                    let kept: Vec<&[u8]> = avcc_units(&sample, length_size)
                        .unwrap()
                        .into_iter()
                        .filter(|unit| !matches!(kind(unit), Some(7 | 8 | 9 | 12)))
                        .collect();
                    let expected: Vec<&[u8]> = if summary.idr {
                        [&sps[..], &pps[..]].into_iter().chain(kept).collect()
                    } else {
                        kept
                    };
                    prop_assert_eq!(annex_b_units(&out).unwrap(), expected);
                }
                Err(_) => prop_assert!(out.is_empty()),
            }
        }
    }
}
