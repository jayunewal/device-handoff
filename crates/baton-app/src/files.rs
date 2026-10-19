// SPDX-License-Identifier: Apache-2.0

//! Files between devices: taking them after the person says yes, and sending them.
//!
//! A device takes files only when the app gives it a folder for them ([`Options::receive_dir`]).
//! Every transfer is put to the person first, with who is asking and the names of what is offered,
//! and they have two minutes. The files are put in the folder when the transfer has ended and
//! each has been checked, and not before (ADR 0008), so the folder never holds a file that is half
//! there.
//!
//! A transfer that this device sends is a [`FileSend`]: it shows how far the transfer has got,
//! can be given up, and says how it ended.

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex},
};

use baton_core::{
    Ending, PairedDevice, TransferHandler, Verdict,
    transfer::{
        destination::Destination,
        send::{Outcome, Progress, Sending, Stage},
    },
    util::BoxFuture,
};
use baton_proto::{
    device::sanitize_display_name,
    id::{DeviceId, TransferId},
    transfer::{Done, Offer},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{
    Error, Event, Result,
    app::{Questions, locked},
    model::{FileProgress, FilesSent, PreviewName, TransferEnd, TransferRequest, TransferResult},
};

/// What this device does with the transfers that other devices offer: it asks the person, and it
/// tells the screen how each one ended.
pub(crate) struct Files {
    destination: Destination,
    questions: Arc<Questions>,
    /// The transfers the person said yes to that have not ended, by the device that offered them
    /// and the id they have on the wire.
    taken: Mutex<HashMap<(DeviceId, TransferId), Taken>>,
}

/// What is remembered of a transfer from the time the person said yes to the time it ends.
struct Taken {
    request: u64,
    files: u32,
    bytes: u64,
    preview: Vec<PreviewName>,
}

impl Files {
    pub(crate) fn new(destination: Destination, questions: Arc<Questions>) -> Self {
        Self {
            destination,
            questions,
            taken: Mutex::new(HashMap::new()),
        }
    }
}

impl TransferHandler for Files {
    fn destination(&self) -> Option<Destination> {
        Some(self.destination.clone())
    }

    fn offered<'a>(&'a self, from: &'a PairedDevice, offer: &'a Offer) -> BoxFuture<'a, Verdict> {
        Box::pin(async move {
            // The names were written by the other device, so they are cleaned the way a device
            // name is before anyone is shown them.
            let preview: Vec<PreviewName> = offer
                .preview
                .iter()
                .map(|item| PreviewName {
                    name: sanitize_display_name(&item.name),
                    folder: item.folder,
                    files: item.files,
                    bytes: item.bytes,
                })
                .collect();
            let summary = offer.summary;
            let (request, taken) = Arc::clone(&self.questions)
                .ask_transfer(|id| TransferRequest {
                    id,
                    device_id: from.id.to_string(),
                    device_name: from.display_name().to_owned(),
                    items: summary.items,
                    files: summary.files,
                    folders: summary.folders,
                    bytes: summary.bytes,
                    preview: preview.clone(),
                })
                .await;
            if !taken {
                return Verdict::Refuse;
            }
            locked(&self.taken).insert(
                (from.id, offer.id),
                Taken {
                    request,
                    files: summary.files,
                    bytes: summary.bytes,
                    preview,
                },
            );
            Verdict::Accept
        })
    }

    fn ended(&self, from: &PairedDevice, id: TransferId, ending: Ending) {
        let Some(taken) = locked(&self.taken).remove(&(from.id, id)) else {
            return;
        };
        let result = match ending {
            Ending::Done(done) => TransferResult::Received {
                failed: failed_files(&done),
            },
            Ending::Dropped => TransferResult::Dropped,
        };
        self.questions.emit(Event::TransferEnded(TransferEnd {
            request: taken.request,
            device_id: from.id.to_string(),
            device_name: from.display_name().to_owned(),
            files: taken.files,
            bytes: taken.bytes,
            preview: taken.preview,
            result,
        }));
    }
}

/// How many files a receiver says it did not put in place, the ones it does not name included.
fn failed_files(done: &Done) -> u32 {
    u32::try_from(done.failed.len())
        .unwrap_or(u32::MAX)
        .saturating_add(done.more_failed)
}

/// A transfer that this device is sending.
///
/// Look at [`progress`](Self::progress) now and then to show how far it has got, give it up with
/// [`cancel`](Self::cancel), and wait for [`outcome`](Self::outcome). Letting go of it before it
/// ends gives the transfer up.
pub struct FileSend {
    progress: watch::Receiver<Progress>,
    cancel: CancellationToken,
    result: watch::Receiver<Option<Result<FilesSent>>>,
}

impl fmt::Debug for FileSend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileSend")
            .field("progress", &*self.progress.borrow())
            .finish_non_exhaustive()
    }
}

impl FileSend {
    /// Takes over a transfer that was just started to the device called `device`. `left_out` is
    /// how many things in what was chosen the walk left out.
    pub(crate) fn start(sending: Sending, device: String, left_out: u32) -> Self {
        let progress = sending.progress();
        let cancel = CancellationToken::new();
        let (done, result) = watch::channel(None);
        tokio::spawn(drive(sending, device, left_out, cancel.clone(), done));
        Self {
            progress,
            cancel,
            result,
        }
    }

    /// How far the transfer has got. This is cheap, and the latest is always there.
    pub fn progress(&self) -> FileProgress {
        FileProgress::from(*self.progress.borrow())
    }

    /// Gives the transfer up, and tells the other device if it can. [`outcome`](Self::outcome)
    /// then says [`ErrorKind::Cancelled`](crate::ErrorKind::Cancelled). It does nothing once the
    /// transfer is over.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Waits for the end of the transfer. Every call gets the same answer.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Declined`](crate::ErrorKind::Declined) if the other device did not take the
    /// files, [`ErrorKind::Unreachable`](crate::ErrorKind::Unreachable) if Baton gave up
    /// reaching it, [`ErrorKind::Cancelled`](crate::ErrorKind::Cancelled) if the transfer was
    /// given up with [`cancel`](Self::cancel), [`ErrorKind::Stopped`](crate::ErrorKind::Stopped)
    /// if the app stopped, and [`ErrorKind::NotPaired`](crate::ErrorKind::NotPaired) or
    /// [`ErrorKind::Other`](crate::ErrorKind::Other) otherwise.
    pub async fn outcome(&self) -> Result<FilesSent> {
        let mut result = self.result.clone();
        match result.wait_for(Option::is_some).await {
            Ok(ended) => ended.clone().unwrap_or_else(|| Err(Error::stopped())),
            // The task that drives the transfer is gone without an answer, which happens only
            // as the runtime shuts down.
            Err(_) => Err(Error::stopped()),
        }
    }
}

impl Drop for FileSend {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Waits for the end of `sending`, gives it up when `cancel` is cancelled, and says how it went.
async fn drive(
    sending: Sending,
    device: String,
    left_out: u32,
    cancel: CancellationToken,
    done: watch::Sender<Option<Result<FilesSent>>>,
) {
    let mut progress = sending.progress();
    let (files, bytes) = {
        let now = progress.borrow();
        (now.files, now.bytes)
    };
    // The outcome takes the transfer, so it is asked for after the transfer is over, and the
    // wait for the end is the progress, which says `Over` as the last thing the transfer does.
    loop {
        if progress.borrow().stage == Stage::Over {
            break;
        }
        tokio::select! {
            () = cancel.cancelled() => {
                sending.cancel();
                break;
            }
            changed = progress.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }
    let result = match sending.outcome().await {
        Outcome::Done(sent) => Ok(FilesSent {
            files,
            bytes,
            failed: failed_files(&sent.done),
            left_out,
        }),
        other => Err(Error::not_sent(&other, &device)),
    };
    // Nobody waiting is fine.
    let _ = done.send(Some(result));
}
