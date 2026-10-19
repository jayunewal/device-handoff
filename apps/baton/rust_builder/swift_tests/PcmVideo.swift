// SPDX-License-Identifier: Apache-2.0

// A short H.264 video with a known answer, for the tests of a decoder. Its pictures are made of
// I_PCM macroblocks, which hold the samples as they are, and its deblocking filter is off, so
// whatever decodes it must give back the samples written here, to the last bit. No encoder is
// needed to make it, and no file is kept: the bytes are written by the code below, in the order
// of the syntax in the standard (ITU-T H.264, clauses 7.3.2.1.1, 7.3.2.2, 7.3.3 and 7.3.5).
//
// It is Baseline profile with one slice to a picture and no picture order, so the pictures come
// out in the order they are decoded. Two kinds of picture follow each other: a key frame that
// holds a picture, and a frame that is all skipped macroblocks, which shows the picture before.

import Foundation

/// Writes bits, the first of a value first, as the syntax of H.264 does.
struct BitWriter {
  private var bits: [Bool] = []

  mutating func put(_ count: Int, _ value: Int) {
    for shift in stride(from: count - 1, through: 0, by: -1) {
      bits.append((value >> shift) & 1 == 1)
    }
  }

  /// An unsigned number as Exp-Golomb code: ue(v) of the standard.
  mutating func ue(_ value: Int) {
    let coded = value + 1
    let width = Int.bitWidth - coded.leadingZeroBitCount
    put(width - 1, 0)
    put(width, coded)
  }

  mutating func alignWithZeros() {
    while bits.count % 8 != 0 { bits.append(false) }
  }

  /// A signed number as Exp-Golomb code: se(v) of the standard.
  mutating func se(_ value: Int) {
    ue(value > 0 ? 2 * value - 1 : -2 * value)
  }

  /// The bytes, after the bit that ends the data and the zeros that fill the last byte.
  mutating func finish() -> [UInt8] {
    bits.append(true)
    alignWithZeros()
    return stride(from: 0, to: bits.count, by: 8).map { start in
      bits[start..<(start + 8)].reduce(UInt8(0)) { ($0 << 1) | ($1 ? 1 : 0) }
    }
  }
}

/// A picture as three planes of samples. The chroma planes are half as wide and high.
struct TestPicture: Equatable {
  var width: Int
  var height: Int
  var luma: [UInt8]
  var blue: [UInt8]
  var red: [UInt8]

  /// A picture where every sample differs from the ones around it, and from the same sample of
  /// the picture with another `number`. The values stay between 16 and 223.
  init(number: Int, width: Int, height: Int) {
    self.width = width
    self.height = height
    luma = TestPicture.plane(
      width: width, height: height, base: 16, range: 200, steps: (5, 3, 40), number: number)
    blue = TestPicture.plane(
      width: width / 2, height: height / 2, base: 32, range: 192, steps: (7, 1, 30),
      number: number)
    red = TestPicture.plane(
      width: width / 2, height: height / 2, base: 64, range: 160, steps: (1, 9, 50),
      number: number)
  }

  /// The sample at (x, y) is `base` and a number below `range` that grows with x, with y and with
  /// the number of the picture, by the steps given in that order.
  private static func plane(
    width: Int, height: Int, base: Int, range: Int, steps: (x: Int, y: Int, number: Int),
    number: Int
  ) -> [UInt8] {
    var samples: [UInt8] = []
    samples.reserveCapacity(width * height)
    for y in 0..<height {
      for x in 0..<width {
        let grown: Int = x * steps.x + y * steps.y + number * steps.number
        samples.append(UInt8(base + grown % range))
      }
    }
    return samples
  }

  /// The rows of the top left `width` by `height` samples, as a decoder that makes two planes
  /// writes them: one plane for the luma, and one for the chroma, with a blue and a red sample
  /// side by side.
  func rows(width shown: Int, height rows: Int) -> (luma: [[UInt8]], chroma: [[UInt8]]) {
    let luma = (0..<rows).map { y in Array(self.luma[(y * width)..<(y * width + shown)]) }
    let chroma = (0..<(rows / 2)).map { y -> [UInt8] in
      let start = y * (width / 2)
      return (0..<(shown / 2)).flatMap { x in [blue[start + x], red[start + x]] }
    }
    return (luma, chroma)
  }
}

/// A video of `macroblocksWide` by `macroblocksHigh` macroblocks of 16 by 16 samples, of which the
/// top left `width` by `height` samples are shown: the edge on the right and at the bottom is cut
/// by `cropRight` and `cropBottom` units of two samples, as an encoder does with a size that is
/// not a multiple of 16.
struct PcmVideo {
  var macroblocksWide: Int
  var macroblocksHigh: Int
  var cropRight = 0
  var cropBottom = 0

  var codedWidth: Int { macroblocksWide * 16 }
  var codedHeight: Int { macroblocksHigh * 16 }
  var width: Int { codedWidth - 2 * cropRight }
  var height: Int { codedHeight - 2 * cropBottom }

  func picture(_ number: Int) -> TestPicture {
    TestPicture(number: number, width: codedWidth, height: codedHeight)
  }

  /// The units without their start codes, as they are in the parameter sets of a frame.
  var sequenceParameterSet: [UInt8] {
    var b = BitWriter()
    b.put(8, 66)  // profile_idc: Baseline
    b.put(8, 0xC0)  // constraint_set0_flag and constraint_set1_flag
    b.put(8, 30)  // level_idc
    b.ue(0)  // seq_parameter_set_id
    b.ue(0)  // log2_max_frame_num_minus4: frame_num has 4 bits
    b.ue(2)  // pic_order_cnt_type: the order of output is the order of decoding
    b.ue(1)  // max_num_ref_frames
    b.put(1, 0)  // gaps_in_frame_num_value_allowed_flag
    b.ue(macroblocksWide - 1)
    b.ue(macroblocksHigh - 1)
    b.put(1, 1)  // frame_mbs_only_flag
    b.put(1, 1)  // direct_8x8_inference_flag
    if cropRight != 0 || cropBottom != 0 {
      b.put(1, 1)  // frame_cropping_flag
      b.ue(0)
      b.ue(cropRight)
      b.ue(0)
      b.ue(cropBottom)
    } else {
      b.put(1, 0)
    }
    b.put(1, 0)  // vui_parameters_present_flag
    return PcmVideo.unit(referenceIndication: 3, kind: 7, payload: b.finish())
  }

  var pictureParameterSet: [UInt8] {
    var b = BitWriter()
    b.ue(0)  // pic_parameter_set_id
    b.ue(0)  // seq_parameter_set_id
    b.put(1, 0)  // entropy_coding_mode_flag: CAVLC
    b.put(1, 0)  // bottom_field_pic_order_in_frame_present_flag
    b.ue(0)  // num_slice_groups_minus1
    b.ue(0)  // num_ref_idx_l0_default_active_minus1
    b.ue(0)  // num_ref_idx_l1_default_active_minus1
    b.put(1, 0)  // weighted_pred_flag
    b.put(2, 0)  // weighted_bipred_idc
    b.se(0)  // pic_init_qp_minus26
    b.se(0)  // pic_init_qs_minus26
    b.se(0)  // chroma_qp_index_offset
    b.put(1, 1)  // deblocking_filter_control_present_flag
    b.put(1, 0)  // constrained_intra_pred_flag
    b.put(1, 0)  // redundant_pic_cnt_present_flag
    return PcmVideo.unit(referenceIndication: 3, kind: 8, payload: b.finish())
  }

  /// A key frame that holds picture `number`.
  func keyFrame(_ number: Int) -> [UInt8] {
    let picture = self.picture(number)
    var b = BitWriter()
    b.ue(0)  // first_mb_in_slice
    b.ue(7)  // slice_type: I, and every slice of the picture is
    b.ue(0)  // pic_parameter_set_id
    b.put(4, 0)  // frame_num
    b.ue(number)  // idr_pic_id: it differs between two key frames in a row
    b.put(1, 0)  // no_output_of_prior_pics_flag
    b.put(1, 0)  // long_term_reference_flag
    b.se(0)  // slice_qp_delta
    b.ue(1)  // disable_deblocking_filter_idc
    for row in 0..<macroblocksHigh {
      for column in 0..<macroblocksWide {
        b.ue(25)  // mb_type: I_PCM
        b.alignWithZeros()
        for y in 0..<16 {
          let start = (row * 16 + y) * codedWidth + column * 16
          for sample in picture.luma[start..<(start + 16)] { b.put(8, Int(sample)) }
        }
        for plane in [picture.blue, picture.red] {
          for y in 0..<8 {
            let start = (row * 8 + y) * (codedWidth / 2) + column * 8
            for sample in plane[start..<(start + 8)] { b.put(8, Int(sample)) }
          }
        }
      }
    }
    return PcmVideo.unit(referenceIndication: 3, kind: 5, payload: b.finish())
  }

  /// A frame that shows the picture before it, `frameNumber` frames after the last key frame.
  func repeatedFrame(_ frameNumber: Int) -> [UInt8] {
    var b = BitWriter()
    b.ue(0)  // first_mb_in_slice
    b.ue(5)  // slice_type: P, and every slice of the picture is
    b.ue(0)  // pic_parameter_set_id
    b.put(4, frameNumber)
    b.put(1, 0)  // num_ref_idx_active_override_flag
    b.put(1, 0)  // ref_pic_list_modification_flag_l0
    b.put(1, 0)  // adaptive_ref_pic_marking_mode_flag
    b.se(0)  // slice_qp_delta
    b.ue(1)  // disable_deblocking_filter_idc
    b.ue(macroblocksWide * macroblocksHigh)  // mb_skip_run: every macroblock
    return PcmVideo.unit(referenceIndication: 2, kind: 1, payload: b.finish())
  }

  /// A unit of the stream: its header byte and the data, with the byte 3 put into every place
  /// where two zeros would be followed by a byte that could be taken for the start of a code.
  static func unit(referenceIndication: Int, kind: Int, payload: [UInt8]) -> [UInt8] {
    var unit: [UInt8] = [UInt8(referenceIndication << 5 | kind)]
    var zeros = 0
    for byte in payload {
      if zeros >= 2 && byte <= 3 {
        unit.append(3)
        zeros = 0
      }
      unit.append(byte)
      zeros = byte == 0 ? zeros + 1 : 0
    }
    return unit
  }

  /// The unit as the sample of a frame holds it: behind its length, in four bytes.
  static func sample(_ unit: [UInt8]) -> [UInt8] {
    let length = UInt32(unit.count)
    return [UInt8(length >> 24), UInt8((length >> 16) & 0xFF), UInt8((length >> 8) & 0xFF), UInt8(length & 0xFF)]
      + unit
  }
}
