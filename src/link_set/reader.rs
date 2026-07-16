use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
};

use tokio::sync::mpsc::Receiver;
use tracing::debug;

use crate::{
    LinkSetError, LinkSetMessage, LinkSetResult, LinkSetSendable, epoch::Epoch,
    link_set::controller::LinkSetMessageInner,
};

pub struct LinkSetReader<M: LinkSetSendable> {
    from_core: Receiver<LinkSetMessageInner>,
    epoch: Arc<Mutex<Option<Epoch>>>,
    _phantom: PhantomData<M>,
}

impl<M: LinkSetSendable> LinkSetReader<M> {
    pub(crate) fn new(
        from_core: Receiver<LinkSetMessageInner>,
        epoch: Arc<Mutex<Option<Epoch>>>,
    ) -> Self {
        Self {
            from_core,
            epoch,
            _phantom: PhantomData,
        }
    }

    pub async fn recv(&mut self) -> LinkSetResult<LinkSetMessage<M>> {
        let ret = self
            .from_core
            .recv()
            .await
            .ok_or_else(||{
                // Since the link set is terminated, there is no epoch any longer.
                match self.epoch.lock() {
                    Ok(mut lock) => *lock = None,
                    Err(poison) => *poison.into_inner() = None,
                }
                LinkSetError::Terminated
            })?
            .try_into()?;

        if let LinkSetMessage::Connected(epoch) = &ret {
            let mut mutex = self.epoch.lock().unwrap_or_else(|l| l.into_inner());
            *mutex = Some(*epoch);
        }
        if let LinkSetMessage::Disconnected = &ret {
            let mut mutex = self.epoch.lock().unwrap_or_else(|l| l.into_inner());
            *mutex = None;
        }
        debug!("LinkSetReader emitted: {:?}", ret);
        Ok(ret)
    }

    /// Reflects the epoch of the most recently received message.
    pub fn current_epoch(&self) -> Option<Epoch> {
        match self.epoch.lock() {
            Ok(lock) => {
                // Indicate the epoch is finished if the channel is closed and
                // empty
                if self.from_core.is_closed() && self.from_core.is_empty() {
                    None
                } else {
                    lock.clone()
                }
            }
            // Should never happen, lock is held for very little time, and there
            // are no panic points while it is held.
            Err(_) => None,
        }
    }
}

impl<M: LinkSetSendable> Drop for LinkSetReader<M>{
    fn drop(&mut self) {
        // When the reader is dropped, there is no more possibility of reading.
        // Effectively, there is no epoch any longer
        match self.epoch.lock() {
            Ok(mut lock) => *lock = None,
            Err(poison) => *poison.into_inner() = None,
        }
    }
}
