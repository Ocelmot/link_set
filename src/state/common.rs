use std::time::Duration;

use futures::stream::SelectAll;
use tokio::sync::mpsc::Sender;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    LinkSetResult,
    deadline::Deadline,
    link_set::controller::{LinkSetControl, LinkSetMessageInner},
    links::{LinkEntry, link::PinnedLink},
    protocol::LinkProtocol,
};

pub(crate) struct CommonState {
    to_core: Sender<LinkSetControl>,
    to_ctrl: Sender<LinkSetMessageInner>,

    next_link_id: u64,
    readers: SelectAll<ReceiverStream<(u64, LinkProtocol)>>,

    /// Timer for states
    timer: Deadline,

    /// Automatically attempt to reconnect when the link set disconnects
    auto_connect: bool,

    /// Timeout for Connecting state (Some(ZERO) = immediately disconnect,
    /// None = try forever)
    connecting_timeout: Option<Duration>,

    /// how long should the reconnection attempt last (Some(ZERO) = skip, None =
    /// try forever)
    reconnecting_timeout: Option<Duration>,

    /// Allow incoming reconnection attempts for duration (Some(ZERO) = skip,
    /// None = try forever)
    grace_period_timeout: Option<Duration>,
}

impl CommonState {
    pub(crate) fn new(
        to_core: Sender<LinkSetControl>,
        to_ctrl: Sender<LinkSetMessageInner>,
    ) -> Self {
        Self {
            to_core,
            to_ctrl,

            next_link_id: 0,
            readers: SelectAll::new(),

            timer: Deadline::new(),

            auto_connect: true,
            connecting_timeout: Some(Duration::from_secs(60)),
            reconnecting_timeout: Some(Duration::ZERO),
            grace_period_timeout: Some(Duration::ZERO),
        }
    }

    pub(crate) fn get_await_items(
        &mut self,
    ) -> (
        &mut SelectAll<ReceiverStream<(u64, LinkProtocol)>>,
        &mut Deadline,
    ) {
        (&mut self.readers, &mut self.timer)
    }

    pub(crate) fn get_readers_mut(
        &mut self,
    ) -> &mut SelectAll<ReceiverStream<(u64, LinkProtocol)>> {
        &mut self.readers
    }

    pub(crate) fn get_timer(&mut self) -> &mut Deadline {
        &mut self.timer
    }

    pub(crate) fn get_to_core(&self) -> &Sender<LinkSetControl> {
        &self.to_core
    }

    pub(crate) fn get_to_ctrl(&self) -> &Sender<LinkSetMessageInner> {
        &self.to_ctrl
    }

    pub(crate) fn auto_connect(&self) -> bool {
        self.auto_connect
    }
    pub(crate) fn set_auto_connect(&mut self, auto_connect: bool) {
        self.auto_connect = auto_connect;
    }

    pub(crate) fn connecting_timeout(&self) -> &Option<Duration> {
        &self.connecting_timeout
    }
    pub(crate) fn set_connecting_timeout(&mut self, connecting_timeout: Option<Duration>) {
        self.connecting_timeout = connecting_timeout
    }

    pub(crate) fn reconnecting_timeout(&self) -> &Option<Duration> {
        &self.reconnecting_timeout
    }
    pub(crate) fn set_reconnecting_timeout(&mut self, reconnecting_timeout: Option<Duration>) {
        self.reconnecting_timeout = reconnecting_timeout
    }

    pub(crate) fn grace_period_timeout(&self) -> &Option<Duration> {
        &self.grace_period_timeout
    }
    pub(crate) fn set_grace_period_timeout(&mut self, grace_period_timeout: Option<Duration>) {
        self.grace_period_timeout = grace_period_timeout
    }

    pub(crate) fn wrap_link(&mut self, link: Box<dyn PinnedLink>) -> LinkSetResult<LinkEntry> {
        let id = self.next_link_id;
        self.next_link_id += 1;
        if link.max_size() <= LinkEntry::OVERHEAD {
            return Err(crate::LinkSetError::Implementation {
                size: link.max_size(),
                required_size: LinkEntry::OVERHEAD,
            });
        }
        let mut wrapped = LinkEntry::new(link, id);
        let reader = wrapped.take_reader()?;
        self.readers.push(reader);
        Ok(wrapped)
    }
}
