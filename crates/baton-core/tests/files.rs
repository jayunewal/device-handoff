// SPDX-License-Identifier: Apache-2.0

//! Sending files between two devices through the public interface, as an application does it: walk
//! what the person chose, hand it to the node, watch the transfer, and take its outcome.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use baton_core::{
    Ending, Error, Identity, PairedDevice, TransferHandler, Verdict,
    transfer::{
        destination::Destination,
        send::{Outcome, Selection, Stage},
    },
    util::BoxFuture,
};
use baton_proto::{
    DeviceId, TransferId,
    message::DeclineReason,
    transfer::{Offer, Summary},
};
use common::{Device, Spec, device, eventually};

/// The application on the receiving device: it puts what it receives in a folder, says yes to every
/// offer, and keeps what it was asked and how each transfer ended.
struct Takes {
    destination: Destination,
    offers: Mutex<Vec<Offer>>,
    endings: Mutex<Vec<(TransferId, Ending)>>,
}

impl TransferHandler for Takes {
    fn destination(&self) -> Option<Destination> {
        Some(self.destination.clone())
    }

    fn offered<'a>(&'a self, _from: &'a PairedDevice, offer: &'a Offer) -> BoxFuture<'a, Verdict> {
        self.offers.lock().unwrap().push(offer.clone());
        Box::pin(async { Verdict::Accept })
    }

    fn ended(&self, _from: &PairedDevice, id: TransferId, ending: Ending) {
        self.endings.lock().unwrap().push((id, ending));
    }
}

/// A device that takes files into a folder, and the folder.
struct Taker {
    device: Device,
    takes: Arc<Takes>,
    folder: tempfile::TempDir,
}

async fn start_taker() -> Taker {
    let folder = tempfile::tempdir().unwrap();
    let identity = Identity::generate();
    let takes = Arc::new(Takes {
        destination: Destination::open(folder.path(), &identity.device_id()).unwrap(),
        offers: Mutex::default(),
        endings: Mutex::default(),
    });
    let device = Spec::new("Laptop")
        .with_transfers(Arc::clone(&takes) as Arc<dyn TransferHandler>)
        .start_with(tempfile::tempdir().unwrap(), &identity)
        .await;
    Taker {
        device,
        takes,
        folder,
    }
}

/// The path of each file under `root`, with its content.
fn files_under(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(Result::unwrap).collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with(".baton") {
                continue;
            }
            if path.is_dir() {
                visit(&path, root, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out
}

#[tokio::test]
async fn a_folder_and_a_file_walked_and_sent_arrive_in_the_folder_of_the_other_device() {
    let taker = start_taker().await;
    let sender = device("Phone").await;
    sender.pair_with(&taker.device).await;
    let source = tempfile::tempdir().unwrap();
    fs::create_dir_all(source.path().join("trip/photos")).unwrap();
    fs::write(source.path().join("trip/notes.txt"), b"packing list").unwrap();
    fs::write(source.path().join("trip/photos/one.jpg"), vec![7; 3 << 20]).unwrap();
    fs::write(source.path().join("top.bin"), b"top").unwrap();
    let chosen = [source.path().join("trip"), source.path().join("top.bin")];

    let selection = Selection::walk(&chosen).unwrap();
    let summary = selection.summary();
    assert_eq!((summary.files, summary.folders), (3, 2));
    let sending = sender.node.send_files(&taker.device.id, selection).unwrap();
    let mut progress = sending.progress();
    let outcome = sending.outcome().await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    assert_eq!((sent.files, sent.bytes), (3, 12 + (3 << 20) + 3));
    assert_eq!(
        files_under(taker.folder.path()),
        [
            (PathBuf::from("top.bin"), b"top".to_vec()),
            (PathBuf::from("trip/notes.txt"), b"packing list".to_vec()),
            (PathBuf::from("trip/photos/one.jpg"), vec![7; 3 << 20]),
        ]
    );
    // The application at the other end was asked once, and saw what the walk said.
    let offers = taker.takes.offers.lock().unwrap().clone();
    assert_eq!(offers.len(), 1);
    assert_eq!(
        offers[0].summary,
        Summary {
            items: 2,
            files: 3,
            folders: 2,
            bytes: 12 + (3 << 20) + 3,
        }
    );
    // The screen that watches sees the transfer over.
    let last = progress.borrow_and_update();
    assert_eq!((last.stage, last.files), (Stage::Over, 3));
    drop(last);
    eventually("the application to hear the end", || {
        taker.takes.endings.lock().unwrap().len() == 1
    })
    .await;
    assert!(matches!(
        taker.takes.endings.lock().unwrap()[0].1,
        Ending::Done(_)
    ));
}

#[tokio::test]
async fn a_device_that_takes_no_files_declines_what_is_sent_to_it() {
    // `common::pair` starts devices that have no handler for files.
    let (receiver, sender) = common::pair().await;
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("a.txt"), b"hello").unwrap();
    let selection = Selection::walk(&[source.path().join("a.txt")]).unwrap();

    let sending = sender.node.send_files(&receiver.id, selection).unwrap();

    assert_eq!(
        sending.outcome().await,
        Outcome::Declined(DeclineReason::Unsupported)
    );
}

#[tokio::test]
async fn files_cannot_be_sent_to_a_device_that_is_not_paired() {
    let sender = device("Phone").await;
    let stranger = DeviceId::from_bytes([9; DeviceId::LEN]);
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("a.txt"), b"hello").unwrap();
    let selection = Selection::walk(&[source.path().join("a.txt")]).unwrap();

    let error = sender.node.send_files(&stranger, selection).unwrap_err();

    assert!(matches!(error, Error::NotPaired(id) if id == stranger));
}
