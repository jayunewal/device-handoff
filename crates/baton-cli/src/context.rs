// SPDX-License-Identifier: Apache-2.0

//! The state a command works on, opened only as far as the command needs it.
//!
//! Reading the inbox or the list of devices needs nothing but the files, so those work
//! while another window is running `baton listen`. Anything that starts the network or
//! changes the list of devices takes the folder's lock first, and fails with a clear
//! message if another process has it: two programs answering to one device identity
//! would fight over its relay connection and its addresses.

use std::{path::PathBuf, sync::Arc};

use baton_core::{
    Config, DataDir, DirLock, Identity, Inbox, InboxHandler, Node, PairedDevice, TrustStore,
};
use baton_proto::DeviceId;

use crate::error::CliError;

/// A Baton state folder, and what has been done with it so far.
pub struct Context {
    dir: DataDir,
    config: Config,
    inbox: Inbox,
    lock: Option<DirLock>,
    trust: Option<TrustStore>,
    node: Option<Arc<Node>>,
}

impl Context {
    /// Opens the folder `home`, or the default one (`$BATON_HOME`, or a folder in the
    /// person's user profile). It is created if it is not there.
    pub fn open(home: Option<PathBuf>) -> Result<Self, CliError> {
        let root = match home {
            Some(home) => home,
            None => DataDir::default_root()?,
        };
        let dir = DataDir::open(root)?;
        let config = Config::load(&dir.config_file())?;
        let inbox = Inbox::load(dir.inbox_file(), config.handoff.max_inbox_items)?;
        Ok(Self {
            dir,
            config,
            inbox,
            lock: None,
            trust: None,
            node: None,
        })
    }

    /// The folder.
    pub fn dir(&self) -> &DataDir {
        &self.dir
    }

    /// The settings.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The inbox. Reading it and changing it are safe at any time, from any process.
    pub fn inbox(&self) -> &Inbox {
        &self.inbox
    }

    /// Takes the folder's lock, unless this process already holds it.
    fn hold(&mut self) -> Result<(), CliError> {
        if self.lock.is_none() {
            self.lock = Some(self.dir.lock()?);
        }
        Ok(())
    }

    /// This device's id: what is on disk, or a new identity if it has none yet. Making
    /// one needs the folder's lock; reading one does not.
    pub fn device_id(&mut self) -> Result<DeviceId, CliError> {
        if let Some(identity) = Identity::load(&self.dir.identity_file())? {
            return Ok(identity.device_id());
        }
        self.hold()?;
        let identity = Identity::load_or_create(&self.dir.identity_file())?;
        Ok(identity.device_id())
    }

    /// The paired devices: the running list if this process has one, otherwise the file as
    /// it is now. Reading the file is safe while another window is running.
    pub fn devices(&self) -> Result<Vec<PairedDevice>, CliError> {
        match &self.trust {
            Some(trust) => Ok(trust.list()),
            None => Ok(TrustStore::load(self.dir.trust_file())?.list()),
        }
    }

    /// The list of paired devices, to change it. Takes the folder's lock.
    pub fn trust_for_update(&mut self) -> Result<TrustStore, CliError> {
        self.hold()?;
        // Loaded only now that nobody else can change the file, so that a change made by
        // a process that has just finished is not overwritten by an older copy.
        if self.trust.is_none() {
            self.trust = Some(TrustStore::load(self.dir.trust_file())?);
        }
        self.trust
            .clone()
            .ok_or_else(|| CliError::new("the list of paired devices could not be opened"))
    }

    /// The running node, started on first use. Whatever it is handed is kept in the
    /// inbox.
    pub async fn node(&mut self) -> Result<Arc<Node>, CliError> {
        if let Some(node) = &self.node {
            return Ok(Arc::clone(node));
        }
        self.hold()?;
        let identity = Identity::load_or_create(&self.dir.identity_file())?;
        let trust = self.trust_for_update()?;
        let node = Node::builder(&identity, trust, self.config.clone())
            .handoff_handler(InboxHandler::new(self.inbox.clone()))
            .bind()
            .await?;
        let node = Arc::new(node);
        self.node = Some(Arc::clone(&node));
        Ok(node)
    }

    /// Stops the node, if one was started, and tells the devices connected to it.
    pub async fn shutdown(&mut self) {
        if let Some(node) = self.node.take() {
            node.shutdown().await;
        }
    }
}
