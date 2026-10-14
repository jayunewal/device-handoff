// SPDX-License-Identifier: Apache-2.0

//! Size limits that are part of the protocol.
//!
//! Every length that comes from the network is checked against one of these
//! before anything is allocated or acted on. Raising a limit is a compatible
//! change; lowering one is not.

/// Largest frame accepted on the session protocol (header excluded).
pub const MAX_FRAME_LEN: usize = 2 * 1024 * 1024;

/// Largest frame accepted on the pairing protocol (header excluded).
///
/// Pairing messages are a name and a 32-byte proof, so this is generous.
pub const MAX_PAIR_FRAME_LEN: usize = 4 * 1024;

/// Longest device name, in bytes, as sent on the wire.
pub const MAX_NAME_BYTES: usize = 256;

/// Longest app version string, in bytes.
pub const MAX_VERSION_BYTES: usize = 64;

/// Most capabilities a device may advertise.
pub const MAX_CAPABILITIES: usize = 64;

/// Longest capability identifier, in bytes.
pub const MAX_CAPABILITY_BYTES: usize = 64;

/// Longest link, in bytes.
pub const MAX_URL_BYTES: usize = 8 * 1024;

/// Longest link title, in bytes.
pub const MAX_TITLE_BYTES: usize = 1024;

/// Largest clipboard text, in bytes.
pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 1024 * 1024;

/// Longest error message, in bytes.
pub const MAX_ERROR_MESSAGE_BYTES: usize = 1024;

/// A field that does not satisfy the protocol's limits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LimitError {
    /// A string or byte field is longer than allowed.
    #[error("{field} is {len} bytes, more than the limit of {max}")]
    TooLong {
        /// Name of the field.
        field: &'static str,
        /// Actual length in bytes.
        len: usize,
        /// Largest allowed length in bytes.
        max: usize,
    },
    /// A list has more entries than allowed.
    #[error("{field} has {count} entries, more than the limit of {max}")]
    TooMany {
        /// Name of the field.
        field: &'static str,
        /// Actual number of entries.
        count: usize,
        /// Largest allowed number of entries.
        max: usize,
    },
    /// A field that must carry something is empty.
    #[error("{field} must not be empty")]
    Empty {
        /// Name of the field.
        field: &'static str,
    },
}

/// Checks that `value` is not longer than `max` bytes.
pub(crate) fn check_len(field: &'static str, value: &str, max: usize) -> Result<(), LimitError> {
    if value.len() > max {
        return Err(LimitError::TooLong {
            field,
            len: value.len(),
            max,
        });
    }
    Ok(())
}

/// Checks that `value` is not empty and not longer than `max` bytes.
pub(crate) fn check_non_empty_len(
    field: &'static str,
    value: &str,
    max: usize,
) -> Result<(), LimitError> {
    if value.is_empty() {
        return Err(LimitError::Empty { field });
    }
    check_len(field, value, max)
}
