use tokio::sync::{mpsc::Sender, oneshot};

use crate::{Epoch, LinkSetError, LinkSetResult, links::Address};

pub(crate) enum DebugCommand {
    Snapshot(oneshot::Sender<DebugReplySnapshot>),
    EvictLink(oneshot::Sender<bool>, u64),
}

pub struct DebugReplySnapshot {
    pub state_name: String,
    pub epoch: Option<Epoch>,
    pub addrs: Vec<Address>,
    pub connectors: Vec<&'static str>,
    pub links: Vec<LinkDescription>
}

pub struct LinkDescription{
    pub id: u64,
    pub scheme: String,
}

pub(crate) struct ConnectionManagerDebugReplySnapshot{
    /// List of Addresses being handled
    pub addrs: Vec<Address>,
    /// List of schemes of connectors
    pub connectors: Vec<&'static str>
}

pub struct DebugHandle {
    sender: Sender<DebugCommand>,
}

impl DebugHandle {
    pub(crate) fn new(sender: Sender<DebugCommand>) -> Self {
        Self { sender }
    }

    /// Get a [DebugReplySnapshot] of the current state of the LinkSet
    pub async fn snapshot(&self) -> LinkSetResult<DebugReplySnapshot> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = DebugCommand::Snapshot(reply_tx);
        self.sender
            .send(cmd)
            .await
            .map_err(|_| LinkSetError::Terminated)?;
        reply_rx.await.map_err(|_| LinkSetError::Terminated)
    }

    /// Remove the link with the given id.
    /// Returns a bool, true if successful, false if inapplicable.
    pub async fn evict_link(&self, id: u64) -> LinkSetResult<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = DebugCommand::EvictLink(reply_tx, id);
        self.sender
            .send(cmd)
            .await
            .map_err(|_| LinkSetError::Terminated)?;
        reply_rx.await.map_err(|_| LinkSetError::Terminated)
    }
}
