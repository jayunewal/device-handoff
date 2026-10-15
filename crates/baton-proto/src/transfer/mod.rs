// SPDX-License-Identifier: Apache-2.0

//! File transfer: the requests, the list of files, the lanes and the hash.
//!
//! The design is recorded in `docs/adr/0008-file-transfer.md` and specified in section 8 of
//! `docs/protocol.md`. This module holds what two devices must agree on, and nothing that
//! touches a disk or a network, so that it can be tested and fuzzed on its own.
//!
//! - This module: the requests and replies that travel on ordinary streams. The sender
//!   makes an [`Offer`], and the receiver, or the person using it, decides once. The sender
//!   later asks for the outcome with [`Finish`], or gives up with [`Cancel`].
//! - [`path`]: the rules for the paths in a transfer.
//! - [`list`]: the list of files, and the check of it against the offer that was accepted.
//! - [`lane`]: the header of a lane and the heads of its frames.
//! - [`hash`]: spans, and the BLAKE3 hash of a file made from them in any order.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    id::TransferId,
    limits::{
        LimitError, MAX_FAILURES_LISTED, MAX_FILE_BYTES, MAX_LANES, MAX_PREVIEW_ITEMS,
        MAX_TRANSFER_FILES, MAX_TRANSFER_FOLDERS,
    },
    message::DeclineReason,
};

pub mod hash;
pub mod lane;
pub mod list;
pub mod path;

/// A request about a transfer.
///
/// The sender makes it on a bidirectional stream like any other request, and only to a device
/// that advertised [`capability::TRANSFER_FILES`](crate::message::capability::TRANSFER_FILES).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferRequest {
    /// Ask whether the receiver will take a set of files and folders.
    Offer(Offer),
    /// Say that everything has been sent, and ask for the outcome.
    Finish(Finish),
    /// Give the transfer up.
    Cancel(Cancel),
}

impl TransferRequest {
    /// The transfer the request is about.
    pub const fn id(&self) -> TransferId {
        match self {
            Self::Offer(offer) => offer.id,
            Self::Finish(finish) => finish.id,
            Self::Cancel(cancel) => cancel.id,
        }
    }
}

/// The answer to a [`TransferRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferReply {
    /// Answers [`TransferRequest::Offer`]: the receiver takes the transfer.
    Accepted(Accepted),
    /// Answers [`TransferRequest::Offer`]: the receiver does not take it. A device that takes
    /// no part in transfers answers every request this way, with
    /// [`Unsupported`](DeclineReason::Unsupported).
    Declined(Declined),
    /// Answers [`TransferRequest::Finish`]: the outcome.
    Done(Done),
    /// Answers [`TransferRequest::Cancel`].
    Cancelled(Cancelled),
}

impl TransferReply {
    /// The transfer the reply is about.
    pub const fn id(&self) -> TransferId {
        match self {
            Self::Accepted(accepted) => accepted.id,
            Self::Declined(declined) => declined.id,
            Self::Done(done) => done.id,
            Self::Cancelled(cancelled) => cancelled.id,
        }
    }
}

/// What the sender wants to send: how much of it there is, and the first few names.
///
/// This is all the person is shown before they decide, so it is also all that has to be
/// right: the list that follows is checked against it ([`list::ListChecker`]). The hash of the
/// list pins down the rest of it, names and times included, so that a list that arrives later
/// cannot be another one than the one that was offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Chosen at random by the sender. A retry reuses it.
    pub id: TransferId,
    /// How much there is.
    pub summary: Summary,
    /// The first top-level items, in the order of the list: the first
    /// `min(summary.items, `[`MAX_PREVIEW_ITEMS`]`)` of them.
    pub preview: Vec<PreviewItem>,
    /// The hash of the whole list of files, as [`list::ListHasher`] makes it. A repeated offer
    /// is the same offer only if this is the same too.
    pub list_hash: list::ListHash,
}

/// The size of a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// The files and folders that are not inside another folder of the transfer. There is at
    /// least one.
    pub items: u32,
    /// All files, in every folder.
    pub files: u32,
    /// All folders, in every folder.
    pub folders: u32,
    /// The sizes of all the files added up.
    pub bytes: u64,
}

/// A top-level item of a transfer, shown to the person who decides.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewItem {
    /// The name of a file or folder: one name, not a path. See [`path::check_name`].
    pub name: String,
    /// Whether it is a folder.
    #[serde(default, skip_serializing_if = "is_default")]
    pub folder: bool,
    /// For a folder, how many files are in it, in every folder inside it. 0 for a file.
    #[serde(default, skip_serializing_if = "is_default")]
    pub files: u32,
    /// The size of the file, or of all the files in the folder added up.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bytes: u64,
}

/// For `skip_serializing_if`: a field that has its default (`false`, 0) is left out of the
/// message, and a reader that finds it absent takes the same value.
fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

// The names are what people call their files. See the same note on `list::ListEntry`.
impl fmt::Debug for PreviewItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreviewItem")
            .field("name_bytes", &self.name.len())
            .field("folder", &self.folder)
            .field("files", &self.files)
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// The receiver takes the transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted {
    /// The id of the offer this answers.
    pub id: TransferId,
    /// How many lanes the sender may open for this transfer, between 1 and
    /// [`MAX_LANES`]. They are numbered from 0.
    pub lanes: u8,
}

/// The receiver does not take the transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declined {
    /// The id of the offer this answers.
    pub id: TransferId,
    /// Why not.
    pub reason: DeclineReason,
}

/// The sender has sent everything it will send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finish {
    /// The transfer that ends.
    pub id: TransferId,
    /// How many files the sender sent in full: those of the list, less the ones it skipped.
    pub files: u32,
    /// How many bytes those files add up to.
    pub bytes: u64,
}

/// The outcome of a transfer, answering [`Finish`] once the receiver has dealt with every file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    /// The transfer that ended.
    pub id: TransferId,
    /// The files that were not put in place, in the order of the list: the first
    /// [`MAX_FAILURES_LISTED`] of them at most. A file that is not named here is in place if
    /// [`more_failed`](Self::more_failed) is 0, or if it comes before the last one named.
    pub failed: Vec<Failure>,
    /// How many more files failed than are named in `failed`. They all come after the last
    /// one named, and there are none unless [`MAX_FAILURES_LISTED`] are named. Which of the
    /// files after the last one named are in place cannot be told, so a sender treats them
    /// all as not in place.
    #[serde(default, skip_serializing_if = "is_default")]
    pub more_failed: u32,
}

/// A file that was not put in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    /// Position of the file in the list.
    pub file: u32,
    /// Why it was not put in place.
    pub reason: FailReason,
}

/// Why a file was not put in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailReason {
    /// Not all of it arrived.
    Incomplete,
    /// What arrived did not match the hash the sender gave.
    Mismatch,
    /// There was no room for it.
    NoSpace,
    /// The receiver could not write it.
    WriteFailed,
    /// The sender said it would not send it.
    Skipped,
    /// Any reason this version does not know about.
    #[serde(other)]
    Other,
}

/// The sender gives the transfer up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cancel {
    /// The transfer that ends.
    pub id: TransferId,
}

/// The receiver has dropped the transfer and what it held for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cancelled {
    /// The transfer that ended. A receiver that never had it, or has forgotten it, answers
    /// the same way: there is nothing left to cancel.
    pub id: TransferId,
}

fn too_many(field: &'static str, count: u32, max: u32) -> LimitError {
    LimitError::TooMany {
        field,
        count: usize::try_from(count).unwrap_or(usize::MAX),
        max: usize::try_from(max).unwrap_or(usize::MAX),
    }
}

fn invalid(field: &'static str, reason: &'static str) -> LimitError {
    LimitError::Invalid { field, reason }
}

impl PreviewItem {
    fn validate(&self) -> Result<(), LimitError> {
        path::check_name(&self.name).map_err(|e| e.into_limit("preview name"))?;
        if !self.folder && self.files != 0 {
            return Err(invalid("preview", "a file has files in it"));
        }
        if self.folder && self.files == 0 && self.bytes != 0 {
            return Err(invalid("preview", "a folder without files has a size"));
        }
        Ok(())
    }
}

impl Offer {
    /// Checks the offer for what can be seen without the list: every field is within its
    /// limit, the parts add up, and the preview is the first top-level items.
    pub(crate) fn validate(&self) -> Result<(), LimitError> {
        let Summary {
            items,
            files,
            folders,
            bytes,
        } = self.summary;
        if items == 0 {
            return Err(LimitError::Empty {
                field: "transfer items",
            });
        }
        if files > MAX_TRANSFER_FILES {
            return Err(too_many("transfer files", files, MAX_TRANSFER_FILES));
        }
        if folders > MAX_TRANSFER_FOLDERS {
            return Err(too_many("transfer folders", folders, MAX_TRANSFER_FOLDERS));
        }
        if u64::from(items) > u64::from(files) + u64::from(folders) {
            return Err(invalid("transfer items", "more than files and folders"));
        }
        if u128::from(bytes) > u128::from(files) * u128::from(MAX_FILE_BYTES) {
            return Err(invalid("transfer bytes", "more than the files can hold"));
        }

        let shown = usize::try_from(items).map_or(MAX_PREVIEW_ITEMS, |n| n.min(MAX_PREVIEW_ITEMS));
        if self.preview.len() != shown {
            return Err(invalid("preview", "is not the first top-level items"));
        }
        let (mut in_files, mut in_folders, mut in_bytes) = (0_u64, 0_u64, 0_u128);
        let mut before: Option<&str> = None;
        for item in &self.preview {
            item.validate()?;
            // The list is in the order of its names, and so is the preview, with no name twice.
            // An offer that shows `a.txt` twice could never match a list, and a person would be
            // asked about it all the same.
            if before.is_some_and(|name| name >= item.name.as_str()) {
                return Err(invalid("preview", "is not in the order of the list"));
            }
            before = Some(item.name.as_str());
            if item.folder {
                in_folders += 1;
                in_files += u64::from(item.files);
            } else {
                in_files += 1;
            }
            in_bytes += u128::from(item.bytes);
        }
        if in_files > u64::from(files)
            || in_folders > u64::from(folders)
            || in_bytes > u128::from(bytes)
        {
            return Err(invalid("preview", "shows more than the transfer holds"));
        }
        Ok(())
    }
}

impl Finish {
    fn validate(&self) -> Result<(), LimitError> {
        if self.files > MAX_TRANSFER_FILES {
            return Err(too_many("finished files", self.files, MAX_TRANSFER_FILES));
        }
        if u128::from(self.bytes) > u128::from(self.files) * u128::from(MAX_FILE_BYTES) {
            return Err(invalid("finished bytes", "more than the files can hold"));
        }
        Ok(())
    }
}

impl TransferRequest {
    pub(crate) fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Offer(offer) => offer.validate(),
            Self::Finish(finish) => finish.validate(),
            Self::Cancel(_) => Ok(()),
        }
    }
}

impl TransferReply {
    pub(crate) fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Accepted(accepted) => {
                if (1..=MAX_LANES).contains(&accepted.lanes) {
                    Ok(())
                } else {
                    Err(invalid("lanes", "is not between 1 and the most there are"))
                }
            }
            Self::Done(done) => {
                if done.failed.len() > MAX_FAILURES_LISTED {
                    return Err(LimitError::TooMany {
                        field: "failed files",
                        count: done.failed.len(),
                        max: MAX_FAILURES_LISTED,
                    });
                }
                // The files are named in the order of the list, so that a file that is not
                // named is known to be in place when it comes before the last one that is.
                if !done.failed.is_sorted_by(|a, b| a.file < b.file) {
                    return Err(invalid("failed files", "are not in the order of the list"));
                }
                // The rest are counted only once the names have run out, so that all of them
                // come after the last one named.
                if done.more_failed > 0 && done.failed.len() < MAX_FAILURES_LISTED {
                    return Err(invalid(
                        "more failed files",
                        "are counted before the names have run out",
                    ));
                }
                Ok(())
            }
            Self::Declined(_) | Self::Cancelled(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use ciborium::{Value, cbor};

    use super::*;
    use crate::{
        frame::{self, FrameError},
        message::{Request, Response},
    };

    fn id(byte: u8) -> TransferId {
        TransferId::from_bytes([byte; 16])
    }

    /// A list hash whose bytes are 00, 01, 02 and so on, which is easy to see in a dump.
    fn list_hash() -> list::ListHash {
        list::ListHash::from_bytes(std::array::from_fn(|i| u8::try_from(i).unwrap()))
    }

    fn offer() -> Offer {
        Offer {
            id: id(0x42),
            summary: Summary {
                items: 2,
                files: 3,
                folders: 1,
                bytes: 1_234,
            },
            preview: vec![
                PreviewItem {
                    name: "Holiday".into(),
                    folder: true,
                    files: 2,
                    bytes: 1_200,
                },
                PreviewItem {
                    name: "notes.txt".into(),
                    folder: false,
                    files: 0,
                    bytes: 34,
                },
            ],
            list_hash: list_hash(),
        }
    }

    fn all_requests() -> Vec<TransferRequest> {
        vec![
            TransferRequest::Offer(offer()),
            TransferRequest::Finish(Finish {
                id: id(1),
                files: 3,
                bytes: 1_234,
            }),
            TransferRequest::Cancel(Cancel { id: id(2) }),
        ]
    }

    fn all_replies() -> Vec<TransferReply> {
        vec![
            TransferReply::Accepted(Accepted {
                id: id(1),
                lanes: 4,
            }),
            TransferReply::Declined(Declined {
                id: id(2),
                reason: DeclineReason::NoSpace,
            }),
            TransferReply::Done(Done {
                id: id(3),
                failed: vec![
                    Failure {
                        file: 7,
                        reason: FailReason::Mismatch,
                    },
                    Failure {
                        file: 9,
                        reason: FailReason::Other,
                    },
                ],
                more_failed: 0,
            }),
            TransferReply::Done(Done {
                id: id(4),
                failed: vec![],
                more_failed: 0,
            }),
            TransferReply::Cancelled(Cancelled { id: id(5) }),
            // As many failures as a `Done` names, and more that it only counts.
            TransferReply::Done(Done {
                id: id(6),
                failed: failures(MAX_FAILURES_LISTED),
                more_failed: 5,
            }),
        ]
    }

    /// Files 0, 2, 4 and so on, which failed, in the order of the list.
    fn failures(count: usize) -> Vec<Failure> {
        (0..count)
            .map(|i| Failure {
                file: u32::try_from(i).unwrap() * 2,
                reason: FailReason::Incomplete,
            })
            .collect()
    }

    #[test]
    fn every_request_and_reply_survives_the_wire() {
        for request in all_requests() {
            let message = Request::Transfer(request);
            let frame = frame::encode(&message).unwrap();
            assert_eq!(frame::decode::<Request>(&frame).unwrap(), message);
        }
        for reply in all_replies() {
            let message = Response::Transfer(reply);
            let frame = frame::encode(&message).unwrap();
            assert_eq!(frame::decode::<Response>(&frame).unwrap(), message);
        }
    }

    #[test]
    fn requests_and_replies_know_which_transfer_they_are_about() {
        let ids: Vec<TransferId> = all_requests().iter().map(TransferRequest::id).collect();
        assert_eq!(ids, [id(0x42), id(1), id(2)]);
        let ids: Vec<TransferId> = all_replies().iter().map(TransferReply::id).collect();
        assert_eq!(ids, [id(1), id(2), id(3), id(4), id(5), id(6)]);
    }

    #[test]
    fn an_offer_has_a_known_wire_encoding() {
        // The exact CBOR is the protocol. Built here by hand, key by key, so that a change in
        // the order of the fields, or in how an absent field is left out, shows up.
        let expected = cbor!({
            "Transfer" => {
                "Offer" => {
                    "id" => Value::Bytes(vec![0x42; 16]),
                    "summary" => { "items" => 2, "files" => 3, "folders" => 1, "bytes" => 1234 },
                    "preview" => [
                        { "name" => "Holiday", "folder" => true, "files" => 2, "bytes" => 1200 },
                        { "name" => "notes.txt", "bytes" => 34 },
                    ],
                    "list_hash" => Value::Bytes((0..32).collect()),
                },
            },
        })
        .unwrap();
        let mut bytes = Vec::new();
        ciborium::into_writer(&expected, &mut bytes).unwrap();

        let frame = frame::encode(&Request::Transfer(TransferRequest::Offer(offer()))).unwrap();
        assert_eq!(frame.get(frame::HEADER_LEN..).unwrap(), bytes.as_slice());
        assert_eq!(
            data_encoding::HEXLOWER.encode(&frame),
            "000000c2a1685472616e73666572a1654f66666572a462696450424242424242424242424242424242426773756d6d617279a4656974656d73026566696c65730367666f6c64657273016562797465731904d2677072657669657782a4646e616d6567486f6c6964617966666f6c646572f56566696c6573026562797465731904b0a2646e616d65696e6f7465732e7478746562797465731822696c6973745f686173685820000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        );
    }

    #[test]
    fn fields_this_version_does_not_know_are_ignored() {
        let payload = cbor!({
            "Transfer" => {
                "Cancel" => { "id" => Value::Bytes(vec![7; 16]), "reason" => "later versions" },
            },
        })
        .unwrap();
        let mut bytes = Vec::new();
        ciborium::into_writer(&payload, &mut bytes).unwrap();
        assert_eq!(
            frame::decode_payload::<Request>(&bytes).unwrap(),
            Request::Transfer(TransferRequest::Cancel(Cancel { id: id(7) }))
        );
    }

    #[test]
    fn a_reason_this_version_does_not_know_is_other_and_data_is_an_error() {
        let decode = |payload: Value| {
            let mut bytes = Vec::new();
            ciborium::into_writer(&payload, &mut bytes).unwrap();
            frame::decode_payload::<Response>(&bytes)
        };
        let declined = |reason: Value| {
            cbor!({ "Transfer" => { "Declined" => { "id" => Value::Bytes(vec![7; 16]), "reason" => reason } } })
                .unwrap()
        };
        assert_eq!(
            decode(declined(Value::Text("FromTheFuture".into()))).unwrap(),
            Response::Transfer(TransferReply::Declined(Declined {
                id: id(7),
                reason: DeclineReason::Other
            }))
        );
        assert!(matches!(
            decode(declined(cbor!({ "RetryAfter" => 5 }).unwrap())),
            Err(FrameError::Decode(_))
        ));
        let done = |reason: &str| {
            cbor!({ "Transfer" => { "Done" => { "id" => Value::Bytes(vec![7; 16]),
                "failed" => [{ "file" => 1, "reason" => reason }] } } })
            .unwrap()
        };
        assert_eq!(
            decode(done("Mismatch")).unwrap(),
            Response::Transfer(TransferReply::Done(Done {
                id: id(7),
                failed: vec![Failure {
                    file: 1,
                    reason: FailReason::Mismatch
                }],
                more_failed: 0
            }))
        );
        assert!(matches!(
            decode(done("FromTheFuture")).unwrap(),
            Response::Transfer(TransferReply::Done(d)) if d.failed.first().is_some_and(|f| f.reason == FailReason::Other)
        ));
    }

    fn bad(change: impl FnOnce(&mut Offer)) -> Result<(), LimitError> {
        let mut offer = offer();
        change(&mut offer);
        offer.validate()
    }

    #[test]
    fn an_offer_that_adds_up_is_valid() {
        assert_eq!(offer().validate(), Ok(()));
        // A folder with nothing in it, offered by itself.
        let empty_folder = Offer {
            id: id(1),
            summary: Summary {
                items: 1,
                files: 0,
                folders: 1,
                bytes: 0,
            },
            preview: vec![PreviewItem {
                name: "empty".into(),
                folder: true,
                files: 0,
                bytes: 0,
            }],
            list_hash: list_hash(),
        };
        assert_eq!(empty_folder.validate(), Ok(()));
        // Seventeen top-level items: the preview stops at sixteen.
        let many = Offer {
            id: id(1),
            summary: Summary {
                items: 17,
                files: 17,
                folders: 0,
                bytes: 17,
            },
            preview: (0..16)
                .map(|i| PreviewItem {
                    name: format!("f{i:02}"),
                    folder: false,
                    files: 0,
                    bytes: 1,
                })
                .collect(),
            list_hash: list_hash(),
        };
        assert_eq!(many.validate(), Ok(()));
    }

    #[test]
    fn an_offer_needs_a_list_hash_that_is_32_bytes() {
        let decode = |hash: Option<Value>| {
            let mut fields = vec![
                (Value::Text("id".into()), Value::Bytes(vec![0x42; 16])),
                (
                    Value::Text("summary".into()),
                    cbor!({ "items" => 1, "files" => 1, "folders" => 0, "bytes" => 0 }).unwrap(),
                ),
                (
                    Value::Text("preview".into()),
                    cbor!([{ "name" => "a" }]).unwrap(),
                ),
            ];
            if let Some(hash) = hash {
                fields.push((Value::Text("list_hash".into()), hash));
            }
            let payload = cbor!({ "Transfer" => { "Offer" => Value::Map(fields) } }).unwrap();
            let mut bytes = Vec::new();
            ciborium::into_writer(&payload, &mut bytes).unwrap();
            frame::decode_payload::<Request>(&bytes)
        };
        assert!(decode(Some(Value::Bytes(vec![9; 32]))).is_ok());
        for bad in [
            None,
            Some(Value::Bytes(vec![9; 31])),
            Some(Value::Bytes(vec![9; 33])),
            Some(Value::Bytes(vec![])),
            Some(Value::Text("a".repeat(32))),
            Some(Value::Null),
        ] {
            assert!(
                matches!(decode(bad.clone()), Err(FrameError::Decode(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn an_offer_that_does_not_add_up_is_refused() {
        assert!(bad(|o| o.summary.items = 0).is_err());
        assert!(bad(|o| o.summary.files = MAX_TRANSFER_FILES + 1).is_err());
        assert!(bad(|o| o.summary.folders = MAX_TRANSFER_FOLDERS + 1).is_err());
        // More items than there are files and folders.
        assert!(bad(|o| o.summary.items = 5).is_err());
        // Bytes where there are no files, and more than files can hold.
        assert!(
            bad(|o| {
                o.summary.files = 0;
                o.summary.folders = 2;
                o.preview[0].files = 0;
                o.preview[0].bytes = 0;
                o.preview[1].folder = true;
                o.preview[1].bytes = 0;
            })
            .is_err()
        );
        assert!(bad(|o| o.summary.bytes = MAX_FILE_BYTES * 3 + 1).is_err());
        // A preview that is not the first sixteen items, or fewer, or more.
        assert!(
            bad(|o| {
                o.preview.pop();
            })
            .is_err()
        );
        assert!(bad(|o| o.preview.push(o.preview[0].clone())).is_err());
        // A preview that is not in the order of the list, or that shows a name twice.
        let order = Err(invalid("preview", "is not in the order of the list"));
        assert_eq!(bad(|o| o.preview.swap(0, 1)), order);
        assert_eq!(
            bad(|o| o.preview[1].name = o.preview[0].name.clone()),
            order
        );
        // A preview that shows more than the transfer holds.
        assert!(bad(|o| o.preview[0].files = 3).is_err());
        assert!(bad(|o| o.preview[1].bytes = 1_300).is_err());
        assert!(
            bad(|o| {
                o.preview[1].folder = true;
                o.summary.folders = 1;
                o.preview[0].folder = true;
            })
            .is_err()
        );
        // Names that are not names.
        for name in ["", "a/b", "..", ".", "a\0b", &"n".repeat(256)] {
            assert!(
                bad(|o| o.preview[1].name = name.into()).is_err(),
                "{name:?}"
            );
        }
        // A file with files in it, and a folder with a size but no files.
        assert!(bad(|o| o.preview[1].files = 1).is_err());
        assert!(
            bad(|o| {
                o.preview[0].files = 0;
            })
            .is_err()
        );
    }

    #[test]
    fn the_other_requests_and_replies_are_checked_too() {
        let finish = |files, bytes| {
            TransferRequest::Finish(Finish {
                id: id(1),
                files,
                bytes,
            })
            .validate()
        };
        assert_eq!(finish(0, 0), Ok(()));
        assert_eq!(finish(MAX_TRANSFER_FILES, MAX_FILE_BYTES), Ok(()));
        assert!(finish(MAX_TRANSFER_FILES + 1, 0).is_err());
        assert!(finish(1, MAX_FILE_BYTES + 1).is_err());
        assert!(finish(0, 1).is_err());

        let lanes = |lanes| TransferReply::Accepted(Accepted { id: id(1), lanes }).validate();
        assert!(lanes(0).is_err());
        assert_eq!(lanes(1), Ok(()));
        assert_eq!(lanes(MAX_LANES), Ok(()));
        assert!(lanes(MAX_LANES + 1).is_err());

        let failed = |count| {
            TransferReply::Done(Done {
                id: id(1),
                failed: failures(count),
                more_failed: 0,
            })
            .validate()
        };
        assert_eq!(failed(MAX_FAILURES_LISTED), Ok(()));
        assert!(failed(MAX_FAILURES_LISTED + 1).is_err());
    }

    // The files that a `Done` does not name are in place only if it can be told which they
    // are: the named ones are the first to fail, in the order of the list, and the rest, which
    // are only counted, all come after them.
    #[test]
    fn a_done_names_the_first_failures_in_order_and_counts_the_rest() {
        let done = |failed: Vec<Failure>, more_failed| {
            TransferReply::Done(Done {
                id: id(1),
                failed,
                more_failed,
            })
            .validate()
        };
        let at = |files: &[u32]| {
            files
                .iter()
                .map(|&file| Failure {
                    file,
                    reason: FailReason::Other,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(done(at(&[]), 0), Ok(()));
        assert_eq!(done(at(&[3, 9, 10]), 0), Ok(()));

        let order = Err(invalid("failed files", "are not in the order of the list"));
        assert_eq!(done(at(&[9, 3]), 0), order);
        assert_eq!(done(at(&[3, 3]), 0), order);
        assert_eq!(done(at(&[1, 2, 2]), 0), order);

        // More failures than are named: only once the names have run out.
        let early = Err(invalid(
            "more failed files",
            "are counted before the names have run out",
        ));
        assert_eq!(done(at(&[]), 1), early);
        assert_eq!(done(at(&[3]), 1), early);
        assert_eq!(done(failures(MAX_FAILURES_LISTED - 1), 1), early);
        assert_eq!(done(failures(MAX_FAILURES_LISTED), 1), Ok(()));
        assert_eq!(done(failures(MAX_FAILURES_LISTED), 5_000), Ok(()));
        // The order is checked on a full `Done` too.
        let mut full = failures(MAX_FAILURES_LISTED);
        full.swap(10, 11);
        assert_eq!(done(full, 5), order);
    }

    #[test]
    fn a_done_that_breaks_the_rules_is_refused_when_it_is_read() {
        let decode = |failed: Value, more_failed: u32| {
            let payload = cbor!({ "Transfer" => { "Done" => {
                "id" => Value::Bytes(vec![7; 16]),
                "failed" => failed,
                "more_failed" => more_failed,
            } } })
            .unwrap();
            let mut bytes = Vec::new();
            ciborium::into_writer(&payload, &mut bytes).unwrap();
            frame::decode_payload::<Response>(&bytes)
        };
        let two = |first: u32, second: u32| {
            cbor!([{ "file" => first, "reason" => "Other" }, { "file" => second, "reason" => "Other" }])
                .unwrap()
        };
        assert!(decode(two(1, 2), 0).is_ok());
        assert!(matches!(decode(two(2, 1), 0), Err(FrameError::Limit(_))));
        assert!(matches!(decode(two(2, 2), 0), Err(FrameError::Limit(_))));
        assert!(matches!(decode(two(1, 2), 3), Err(FrameError::Limit(_))));
    }

    #[test]
    fn a_message_over_its_limit_is_not_sent() {
        let mut message = offer();
        message.preview[1].name = "n".repeat(MAX_FILE_NAME_FOR_TEST + 1);
        assert!(matches!(
            frame::encode(&Request::Transfer(TransferRequest::Offer(message))),
            Err(FrameError::Limit(_))
        ));
    }

    const MAX_FILE_NAME_FOR_TEST: usize = crate::limits::MAX_FILE_NAME_BYTES;

    #[test]
    fn names_are_not_printed_in_a_log() {
        let mut message = offer();
        message.preview[0].name = "Tax returns 2025".into();
        let shown = format!("{:?}", Request::Transfer(TransferRequest::Offer(message)));
        assert!(!shown.contains("Tax"), "{shown}");
        assert!(shown.contains("name_bytes: 16"), "{shown}");
    }
}
