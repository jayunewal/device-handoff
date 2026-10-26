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

// File transfer (docs/protocol.md section 8).

/// Most top-level items of a transfer that an offer shows by name.
pub const MAX_PREVIEW_ITEMS: usize = 16;

/// Longest name of one file or folder, in bytes. Most file systems allow no more.
pub const MAX_FILE_NAME_BYTES: usize = 255;

/// Longest relative path in a transfer, in bytes.
pub const MAX_PATH_BYTES: usize = 1024;

/// Most parts (names joined by `/`) in a relative path.
pub const MAX_PATH_DEPTH: usize = 64;

/// Most files in one transfer.
pub const MAX_TRANSFER_FILES: u32 = 100_000;

/// Most folders in one transfer.
pub const MAX_TRANSFER_FOLDERS: u32 = 100_000;

/// Largest file in a transfer, in bytes (1 PiB). It keeps the number of spans of a file well
/// inside 32 bits and the sum of the sizes of a few files inside 64 bits.
pub const MAX_FILE_BYTES: u64 = 1 << 50;

/// Most entries in one frame of the list of files.
pub const MAX_LIST_FRAME_ENTRIES: usize = 1024;

/// Most bytes the whole list of files of one transfer may take in the form that is hashed (an
/// entry is 19 bytes and its path). A receiver holds the whole list before it writes anything,
/// so the list has a cap of its own. A transfer with a longer list is offered as several
/// transfers.
pub const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;

/// Most lanes (unidirectional streams) one transfer may use.
pub const MAX_LANES: u8 = 8;

/// Most files that a `Done` names one by one.
pub const MAX_FAILURES_LISTED: usize = 256;

// Window streaming (docs/protocol.md section 9).

/// Largest encoded picture of one frame of video, header excluded, in bytes.
pub const MAX_PICTURE_BYTES: usize = 8 * 1024 * 1024;

/// Longest title of a shared window, in bytes.
pub const MAX_STREAM_TITLE_BYTES: usize = 512;

/// Longest edge of a window that is offered, in pixels. A window can be larger than any
/// picture that is sent, because the source scales it down.
pub const MAX_WINDOW_EDGE: u32 = 16_384;

/// Longest edge of a picture, in pixels.
pub const MAX_PICTURE_EDGE: u32 = 3840;

/// Highest frame rate, in frames a second.
pub const MAX_FRAME_RATE: u16 = 120;

/// Most codecs that an accepted offer names.
pub const MAX_CODECS: usize = 8;

/// Largest message on a control stream of a session, header excluded.
pub const MAX_CONTROL_MESSAGE_BYTES: usize = 64 * 1024;

/// Largest packet of sound, header excluded, in bytes. AAC-LC allows 6144 bits in a packet for
/// each channel, which is 1536 bytes for two.
pub const MAX_SOUND_PACKET_BYTES: usize = 8 * 1024;

/// Lowest sampling rate of sound that is sent, in hertz.
pub const MIN_SOUND_RATE: u32 = 8_000;

/// Highest sampling rate of sound that is sent, in hertz.
pub const MAX_SOUND_RATE: u32 = 96_000;

/// Most channels of sound that are sent.
pub const MAX_SOUND_CHANNELS: u8 = 2;

/// Fewest samples of each channel that one packet of sound holds. It bounds how many packets a
/// second there can be.
pub const MIN_SOUND_PACKET_SAMPLES: u16 = 256;

/// Most samples of each channel that one packet of sound holds.
pub const MAX_SOUND_PACKET_SAMPLES: u16 = 2048;

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
    /// A field is within the size limits but makes no sense, for example a total that its
    /// parts do not add up to.
    #[error("{field} is not valid: {reason}")]
    Invalid {
        /// Name of the field.
        field: &'static str,
        /// What is wrong, for logs.
        reason: &'static str,
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
