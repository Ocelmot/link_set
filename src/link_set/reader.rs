use std::marker::PhantomData;

use tokio::sync::mpsc::Receiver;
use tracing::debug;

use crate::{
    LinkSetError, LinkSetMessage, LinkSetResult, LinkSetSendable, epoch::Epoch,
    link_set::controller::LinkSetMessageInner,
};

pub struct LinkSetReader<M: LinkSetSendable> {
    from_core: Receiver<LinkSetMessageInner>,
    epoch: Option<Epoch>,
    _phantom: PhantomData<M>,
}

impl<M: LinkSetSendable> LinkSetReader<M> {
    pub(crate) fn new(from_core: Receiver<LinkSetMessageInner>, epoch: Option<Epoch>) -> Self {
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
            .ok_or(LinkSetError::Terminated)?
            .try_into()?;

        if let LinkSetMessage::Connected(epoch) = &ret {
            self.epoch = Some(*epoch);
        }
        if let LinkSetMessage::Disconnected = &ret {
            self.epoch = None;
        }
        debug!("LinkSetReader emitted: {:?}", ret);
        Ok(ret)
    }

    pub fn epoch(&self) -> &Option<Epoch> {
        &self.epoch
    }
}
