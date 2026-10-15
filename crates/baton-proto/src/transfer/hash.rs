// SPDX-License-Identifier: Apache-2.0

//! Spans, and the hash of a file made from them in any order.
//!
//! A file is cut into **spans** of [`SPAN_LEN`] bytes (1 MiB; the last one is shorter). BLAKE3
//! hashes a file as a tree of 1 KiB chunks, and a span that starts on a multiple of 1 MiB is a
//! whole subtree of that tree, or, for the last span, the subtree along its right edge. So each
//! span can be hashed on its own, at its place in the file, in whatever order the spans are read
//! or arrive, and the values of the spans can be merged at the end into the hash of the whole
//! file. That hash is the ordinary BLAKE3 hash of the file, the one `b3sum` prints.
//!
//! The merge follows BLAKE3's own rule for the shape of the tree: the left child of a node holds
//! the largest power of two of leaves that is smaller than the node's number of leaves, and the
//! right child holds the rest. Spans are 1024 chunks each, a power of two, so the rule gives the
//! same tree over spans as over chunks. A file of one span has no parent node at all, so the value
//! of its only span is the root hash itself.
//!
//! The functions here do no I/O. They can run on any thread and take the bytes of one span at a
//! time, so memory stays bounded by the spans in flight and not by the size of the file.

use std::collections::{BTreeMap, btree_map::Entry};

use blake3::{
    Hash, Hasher,
    hazmat::{HasherExt, Mode, merge_subtrees_non_root, merge_subtrees_root},
};

use crate::limits::MAX_FILE_BYTES;

/// Length of a span in bytes. A file is cut into spans of this length, and the last is shorter.
///
/// It is a multiple of BLAKE3's chunk length and a power of two of chunks, which is what makes
/// a span a subtree. Changing it changes the wire format.
pub const SPAN_LEN: u64 = 1 << 20;

/// What hashing one span gives: its chaining value, or, for a file of one span, the file's hash.
pub type SpanValue = [u8; blake3::OUT_LEN];

/// Why a span could not be hashed or added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HashError {
    /// The file is larger than a transfer may carry.
    #[error("a file of {size} bytes is larger than the limit")]
    TooLarge {
        /// The claimed size of the file.
        size: u64,
    },
    /// The file has no span with this number.
    #[error("the file has no span {index}")]
    NoSuchSpan {
        /// Number of the span.
        index: u32,
    },
    /// The bytes given are not as many as the span has.
    #[error("span {index} has {expected} bytes and {got} were given")]
    WrongLength {
        /// Number of the span.
        index: u32,
        /// Length the span has.
        expected: u32,
        /// Length that was given.
        got: usize,
    },
    /// The span was added before.
    #[error("span {index} was already added")]
    Duplicate {
        /// Number of the span.
        index: u32,
    },
    /// The span has not been added.
    #[error("span {index} has not been added")]
    Missing {
        /// Number of the span.
        index: u32,
    },
}

/// How many spans a file of `size` bytes has. An empty file has none.
pub const fn span_count(size: u64) -> u64 {
    size.div_ceil(SPAN_LEN)
}

/// The length of span `index` of a file of `size` bytes, or `None` if the file has no such span.
pub fn span_size(size: u64, index: u32) -> Option<u32> {
    span_size_with(SPAN_LEN, size, index)
}

/// Hashes one span, given the size of the file it belongs to, its number and its bytes.
///
/// The bytes must be exactly [`span_size`] long. The value does not depend on the other spans,
/// so any thread can compute it as soon as it has read the span.
pub fn hash_span(size: u64, index: u32, data: &[u8]) -> Result<SpanValue, HashError> {
    hash_span_with(SPAN_LEN, size, index, data)
}

/// The hash of one file, collected from the values of its spans in any order.
///
/// It keeps one value of 32 bytes for each span that was added, and nothing for a span that
/// was not, so a file that claims to be huge costs nothing until its data arrives.
#[derive(Debug, Clone)]
pub struct FileHasher {
    span_len: u64,
    size: u64,
    values: BTreeMap<u32, SpanValue>,
}

impl FileHasher {
    /// Starts the hash of a file of `size` bytes.
    pub fn new(size: u64) -> Result<Self, HashError> {
        Self::with_span_len(SPAN_LEN, size)
    }

    fn with_span_len(span_len: u64, size: u64) -> Result<Self, HashError> {
        if size > MAX_FILE_BYTES {
            return Err(HashError::TooLarge { size });
        }
        Ok(Self {
            span_len,
            size,
            values: BTreeMap::new(),
        })
    }

    /// The size of the file in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The number of spans the file has.
    pub fn span_count(&self) -> u32 {
        // At most 2^30 for the largest file, so this never saturates.
        u32::try_from(self.size.div_ceil(self.span_len)).unwrap_or(u32::MAX)
    }

    /// Whether span `index` has been added.
    pub fn has(&self, index: u32) -> bool {
        self.values.contains_key(&index)
    }

    /// How many spans have been added.
    pub fn spans_held(&self) -> usize {
        self.values.len()
    }

    /// Whether every span has been added.
    pub fn is_complete(&self) -> bool {
        usize::try_from(self.span_count()).is_ok_and(|count| self.values.len() == count)
    }

    /// Adds the value of span `index`, computed by [`hash_span`] on any thread.
    pub fn insert(&mut self, index: u32, value: SpanValue) -> Result<(), HashError> {
        span_size_with(self.span_len, self.size, index).ok_or(HashError::NoSuchSpan { index })?;
        match self.values.entry(index) {
            Entry::Occupied(_) => Err(HashError::Duplicate { index }),
            Entry::Vacant(slot) => {
                slot.insert(value);
                Ok(())
            }
        }
    }

    /// Hashes the bytes of span `index` and adds the value.
    pub fn add_span(&mut self, index: u32, data: &[u8]) -> Result<(), HashError> {
        // A repeat is refused before it is hashed, so repeating a span costs nothing.
        if self.has(index) {
            return Err(HashError::Duplicate { index });
        }
        let value = hash_span_with(self.span_len, self.size, index, data)?;
        self.insert(index, value)
    }

    /// The hash of the whole file, once every span has been added.
    pub fn finish(&self) -> Result<Hash, HashError> {
        match self.span_count() {
            0 => Ok(blake3::hash(b"")),
            1 => self.value(0).map(Hash::from_bytes),
            count => {
                let left_count = left_spans(count);
                let left = self.subtree(0, left_count)?;
                let right = self.subtree(left_count, count - left_count)?;
                Ok(merge_subtrees_root(&left, &right, Mode::Hash))
            }
        }
    }

    fn value(&self, index: u32) -> Result<SpanValue, HashError> {
        self.values
            .get(&index)
            .copied()
            .ok_or(HashError::Missing { index })
    }

    /// The chaining value of the node over `count` spans that start at `first`.
    fn subtree(&self, first: u32, count: u32) -> Result<SpanValue, HashError> {
        if count == 1 {
            return self.value(first);
        }
        let left_count = left_spans(count);
        let left = self.subtree(first, left_count)?;
        let right = self.subtree(first + left_count, count - left_count)?;
        Ok(merge_subtrees_non_root(&left, &right, Mode::Hash))
    }
}

/// The number of leaves in the left child of a node that has `count` (at least two) leaves:
/// the largest power of two below `count`.
fn left_spans(count: u32) -> u32 {
    1 << (u32::BITS - 1 - count.saturating_sub(1).max(1).leading_zeros())
}

fn span_size_with(span_len: u64, size: u64, index: u32) -> Option<u32> {
    let start = u64::from(index).checked_mul(span_len)?;
    let left = size.checked_sub(start).filter(|left| *left > 0)?;
    u32::try_from(left.min(span_len)).ok()
}

fn hash_span_with(
    span_len: u64,
    size: u64,
    index: u32,
    data: &[u8],
) -> Result<SpanValue, HashError> {
    let expected = span_size_with(span_len, size, index).ok_or(HashError::NoSuchSpan { index })?;
    if usize::try_from(expected).ok() != Some(data.len()) {
        return Err(HashError::WrongLength {
            index,
            expected,
            got: data.len(),
        });
    }
    let mut hasher = Hasher::new();
    // The offset is a multiple of the span length, which is a multiple of the chunk length, and
    // it is below the size of the file, so it cannot overflow.
    hasher.set_input_offset(u64::from(index) * span_len);
    hasher.update(data);
    if size <= span_len {
        // The only span is the whole file, which makes it the root of the tree.
        Ok(*hasher.finalize().as_bytes())
    } else {
        Ok(hasher.finalize_non_root())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes that differ from place to place, so that a span hashed at the wrong place or
    /// in the wrong order gives a different hash.
    fn data(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[3]
            })
            .collect()
    }

    /// The numbers 0..count in an order that depends on `seed`.
    fn shuffled(count: u32, seed: u64) -> Vec<u32> {
        let mut order: Vec<u32> = (0..count).collect();
        let mut state = seed | 1;
        for i in (1..order.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let j = usize::try_from(state % (i as u64 + 1)).unwrap();
            order.swap(i, j);
        }
        order
    }

    fn spans_of(content: &[u8], span_len: u64) -> Vec<&[u8]> {
        content.chunks(usize::try_from(span_len).unwrap()).collect()
    }

    fn hash_in_order(span_len: u64, content: &[u8], order: &[u32]) -> Hash {
        let size = content.len() as u64;
        let spans = spans_of(content, span_len);
        let mut hasher = FileHasher::with_span_len(span_len, size).unwrap();
        for &index in order {
            let bytes = spans[usize::try_from(index).unwrap()];
            let value = hash_span_with(span_len, size, index, bytes).unwrap();
            hasher.insert(index, value).unwrap();
        }
        assert!(hasher.is_complete());
        hasher.finish().unwrap()
    }

    #[test]
    fn the_left_child_holds_the_largest_power_of_two_below_the_count() {
        for (count, left) in [
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 4),
            (8, 4),
            (9, 8),
            (16, 8),
            (17, 16),
            ((1 << 30) + 1, 1 << 30),
        ] {
            assert_eq!(left_spans(count), left, "{count}");
        }
    }

    #[test]
    fn span_sizes_follow_the_size_of_the_file() {
        assert_eq!(span_count(0), 0);
        assert_eq!(span_count(1), 1);
        assert_eq!(span_count(SPAN_LEN), 1);
        assert_eq!(span_count(SPAN_LEN + 1), 2);
        assert_eq!(span_count(MAX_FILE_BYTES), 1 << 30);

        let span = u32::try_from(SPAN_LEN).unwrap();
        assert_eq!(span_size(0, 0), None);
        assert_eq!(span_size(1, 0), Some(1));
        assert_eq!(span_size(1, 1), None);
        assert_eq!(span_size(SPAN_LEN, 0), Some(span));
        assert_eq!(span_size(SPAN_LEN, 1), None);
        assert_eq!(span_size(SPAN_LEN + 5, 0), Some(span));
        assert_eq!(span_size(SPAN_LEN + 5, 1), Some(5));
        assert_eq!(span_size(SPAN_LEN + 5, 2), None);
        assert_eq!(span_size(3 * SPAN_LEN, 2), Some(span));
        assert_eq!(span_size(MAX_FILE_BYTES, (1 << 30) - 1), Some(span));
        assert_eq!(span_size(MAX_FILE_BYTES, 1 << 30), None);
        assert_eq!(span_size(u64::MAX, u32::MAX), Some(span));
    }

    #[test]
    fn every_shape_of_tree_gives_the_hash_of_the_whole_file() {
        // Spans of one, two and four chunks are as good as spans of 1 MiB for the shape of the
        // tree, and cheap enough to try every count up to 35 and every kind of end.
        for span_len in [1024_u64, 2048, 4096] {
            for spans in 0_u64..=35 {
                for tail in [0, 1, 1023, 1024, 1025, span_len - 1] {
                    if tail >= span_len {
                        continue;
                    }
                    let size = spans * span_len + tail;
                    let content = data(usize::try_from(size).unwrap(), size + span_len);
                    let count = u32::try_from(size.div_ceil(span_len)).unwrap();
                    let expected = blake3::hash(&content);
                    let in_order: Vec<u32> = (0..count).collect();
                    let reversed: Vec<u32> = in_order.iter().rev().copied().collect();
                    for order in [in_order, reversed, shuffled(count, size + 7)] {
                        assert_eq!(
                            hash_in_order(span_len, &content, &order),
                            expected,
                            "span {span_len}, {size} bytes, order {order:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_real_span_length_gives_the_hash_of_the_whole_file_too() {
        let span = usize::try_from(SPAN_LEN).unwrap();
        for size in [
            0,
            1,
            span - 1,
            span,
            span + 1,
            2 * span,
            3 * span + 17,
            5 * span + 1,
        ] {
            let content = data(size, 99);
            let count = u32::try_from(span_count(size as u64)).unwrap();
            let mut hasher = FileHasher::new(size as u64).unwrap();
            for index in shuffled(count, size as u64 + 3) {
                let start = usize::try_from(index).unwrap() * span;
                let bytes = content.get(start..(start + span).min(size)).unwrap();
                hasher.add_span(index, bytes).unwrap();
            }
            assert_eq!(hasher.finish().unwrap(), blake3::hash(&content), "{size}");
        }
    }

    #[test]
    fn the_hash_of_a_known_file_is_the_one_b3sum_prints() {
        // Files of the byte pattern 0, 1, 2, ..., 250, 0, 1, ... The values come from the portable
        // C implementation of BLAKE3 hashing each file in one piece, not from the Rust crate that
        // the code uses, and they are listed in docs/protocol.md section 7.
        for (size, expected) in [
            (
                0,
                "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            ),
            (
                1,
                "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
            ),
            // One span, exactly.
            (
                1 << 20,
                "74cb441fd087764ca9c3694da742ebe30cbeb3060a17009ca81825c7a8d10343",
            ),
            // Two spans, the second of one byte.
            (
                (1 << 20) + 1,
                "2f053cd7472cf0cd2f9adaf45c1180255b91b9a865404a63671a0ee5f792ed33",
            ),
            // Three spans, the last of three bytes.
            (
                2 * (1 << 20) + 3,
                "891dfce8b8f6360c474d71183da59574306e74157a5ce6cbcd12933cc78c819f",
            ),
            // Six spans: four on the left of the root and two on the right.
            (
                5 * (1 << 20) + 1,
                "a2eed482c537df008175a978a474f0a2f257d3ef11f7ac1f6fa9f5e49af4d345",
            ),
        ] {
            let content: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let mut hasher = FileHasher::new(size).unwrap();
            // Backwards, so that no span is added in the order of the file.
            for (index, bytes) in content.chunks(1 << 20).enumerate().rev() {
                hasher
                    .add_span(u32::try_from(index).unwrap(), bytes)
                    .unwrap();
            }
            assert_eq!(
                hasher.finish().unwrap().to_hex().as_str(),
                expected,
                "{size}"
            );
        }
    }

    #[test]
    fn a_span_that_is_not_there_or_is_the_wrong_length_is_refused() {
        let size = 2 * SPAN_LEN + 5;
        let mut hasher = FileHasher::new(size).unwrap();
        assert_eq!(hasher.span_count(), 3);
        assert_eq!(
            hasher.add_span(3, &[0; 5]),
            Err(HashError::NoSuchSpan { index: 3 })
        );
        assert_eq!(
            hasher.add_span(2, &[0; 4]),
            Err(HashError::WrongLength {
                index: 2,
                expected: 5,
                got: 4
            })
        );
        assert_eq!(
            hasher.add_span(0, &[0; 10]),
            Err(HashError::WrongLength {
                index: 0,
                expected: 1 << 20,
                got: 10
            })
        );
        assert_eq!(
            hasher.insert(9, [0; 32]),
            Err(HashError::NoSuchSpan { index: 9 })
        );
        assert_eq!(hasher.spans_held(), 0);
    }

    #[test]
    fn a_span_added_twice_and_a_missing_span_are_refused() {
        let mut hasher = FileHasher::new(SPAN_LEN + 1).unwrap();
        hasher.add_span(1, &[7]).unwrap();
        assert_eq!(
            hasher.add_span(1, &[7]),
            Err(HashError::Duplicate { index: 1 })
        );
        assert_eq!(
            hasher.insert(1, [0; 32]),
            Err(HashError::Duplicate { index: 1 })
        );
        assert!(!hasher.is_complete());
        assert_eq!(hasher.finish(), Err(HashError::Missing { index: 0 }));
        hasher.add_span(0, &vec![0; 1 << 20]).unwrap();
        assert!(hasher.is_complete());
        assert!(hasher.finish().is_ok());
    }

    #[test]
    fn a_file_that_claims_to_be_huge_costs_nothing_until_its_data_arrives() {
        let mut hasher = FileHasher::new(MAX_FILE_BYTES).unwrap();
        assert_eq!(hasher.span_count(), 1 << 30);
        assert_eq!(hasher.spans_held(), 0);
        hasher.insert((1 << 30) - 1, [1; 32]).unwrap();
        assert_eq!(hasher.spans_held(), 1);
        assert!(!hasher.is_complete());
        assert_eq!(
            hasher.insert(1 << 30, [1; 32]),
            Err(HashError::NoSuchSpan { index: 1 << 30 })
        );
        assert_eq!(
            FileHasher::new(MAX_FILE_BYTES + 1).unwrap_err(),
            HashError::TooLarge {
                size: MAX_FILE_BYTES + 1
            }
        );
    }

    #[test]
    fn an_empty_file_has_the_hash_of_nothing_and_no_spans() {
        let hasher = FileHasher::new(0).unwrap();
        assert_eq!(hasher.span_count(), 0);
        assert!(hasher.is_complete());
        assert_eq!(hasher.finish().unwrap(), blake3::hash(b""));
        assert_eq!(
            hash_span(0, 0, &[]),
            Err(HashError::NoSuchSpan { index: 0 })
        );
    }
}
