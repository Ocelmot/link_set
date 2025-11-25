use std::{pin::Pin, time::Duration};

use futures::stream::SelectAll;
use tokio::sync::mpsc::Sender;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    LinkResult,
    deadline::Deadline,
    link_set::controller::{LinkSetControl, LinkSetControlCommand, LinkSetControlConfig, LinkSetMessageInner},
    links::{WrappedLink, link::PinnedLink},
    protocol::LinkProtocol,
    state::states::States,
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

    /// Timeout for Connecting state (Some(ZERO) = move directly to connecting,
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
    fn new(to_core: Sender<LinkSetControl>, to_ctrl: Sender<LinkSetMessageInner>) -> Self {
        Self {
            to_core,
            to_ctrl,

            next_link_id: 0,
            readers: SelectAll::new(),

            timer: Deadline::new(),

            auto_connect: true,
            connecting_timeout: Some(Duration::from_secs(30)),
            reconnecting_timeout: Some(Duration::ZERO),
            grace_period_timeout: Some(Duration::ZERO),
        }
    }

    pub(crate) fn wrap_link(&mut self, link: Box<dyn PinnedLink>) -> LinkResult<WrappedLink> {
        let id = self.next_link_id;
        self.next_link_id = self.next_link_id + 1;
        let mut wrapped = WrappedLink::new(link, id);
        let reader = wrapped.take_reader()?;
        self.readers.push(reader);
        Ok(wrapped)
    }
    pub(crate) fn clear_readers(&mut self) {
        self.readers.clear();
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
}

pub(crate) struct CoreState {
    common: CommonState,
    state: States,
}

impl CoreState {
    pub(crate) fn new(to_core: Sender<LinkSetControl>, to_ctrl: Sender<LinkSetMessageInner>) -> Self {
        Self {
            common: CommonState::new(to_core, to_ctrl),
            state: States::new().into(),
        }
    }

    pub(crate) fn get_readers(&mut self) -> &mut SelectAll<ReceiverStream<(u64, LinkProtocol)>> {
        &mut self.common.readers
    }

    pub(crate) fn get_timer(&mut self) -> &mut Deadline {
        &mut self.common.timer
    }

    pub(crate) fn get_refs(&mut self) -> (&mut SelectAll<ReceiverStream<(u64, LinkProtocol)>>, &mut Deadline) {
        (&mut self.common.readers, &mut self.common.timer)
    }

    pub(crate) fn get_state_name(&self) -> &'static str  {
        self.state.get_name()
    }

    pub(crate) async fn ctrl_msg(self, msg: LinkSetControl) -> Self {
        match msg{
            LinkSetControl::Command(cmd) => self.ctrl_msg_cmd(cmd).await,
            LinkSetControl::Config(conf) => self.ctrl_msg_cfg(conf).await,
        }
    }

    pub(crate) async fn ctrl_msg_cmd(mut self, msg: LinkSetControlCommand) -> Self {
        let boxed = self.state.to_boxed();
        let state = boxed.ctrl_msg(&mut self.common, msg).await;
        Self {
            common: self.common,
            state,
        }
    }

    pub(crate) async fn ctrl_msg_cfg(mut self, msg: LinkSetControlConfig) -> Self {
        match msg {
            LinkSetControlConfig::AutoConnect(connect) => self.common.set_auto_connect(connect),
            LinkSetControlConfig::ReconnectTimeout(timeout) => self.common.set_reconnecting_timeout(timeout),
            LinkSetControlConfig::GracePeriod(timeout) => self.common.set_grace_period_timeout(timeout),
        }
        self
    }

    pub(crate) async fn link_msg(mut self, id: u64, msg: LinkProtocol) -> Self {
        let boxed = self.state.to_boxed();
        let state = boxed.link_msg(&mut self.common, id, msg).await;
        Self {
            common: self.common,
            state,
        }
    }

    pub(crate) async fn timer(mut self) -> Self {
        let boxed = self.state.to_boxed();
        let state = boxed.timer(&mut self.common).await;
        Self {
            common: self.common,
            state,
        }
    }
}

pub(crate) trait CoreStateState {
    fn ctrl_msg(
        self: Box<Self>,
        common: &mut CommonState,
        msg: LinkSetControlCommand,
    ) -> impl Future<Output = States> + Send;

    fn link_msg(
        self: Box<Self>,
        common: &mut CommonState,
        id: u64,
        msg: LinkProtocol,
    ) -> impl Future<Output = States> + Send;

    fn timer(
        self: Box<Self>,
        common: &mut CommonState,
    ) -> impl Future<Output = States> + Send;
}

pub(crate) trait PinnedCoreStateState {
    fn ctrl_msg(
        self: Box<Self>,
        common: &'_ mut CommonState,
        msg: LinkSetControlCommand,
    ) -> Pin<Box<dyn Future<Output = States> + Send + '_>>;

    fn link_msg(
        self: Box<Self>,
        common: &'_ mut CommonState,
        id: u64,
        msg: LinkProtocol,
    ) -> Pin<Box<dyn Future<Output = States> + Send + '_>>;

    fn timer<'a>(
        self: Box<Self>,
        common: &'a mut CommonState,
    ) -> Pin<Box<dyn Future<Output = States> + Send + 'a>>;
}

impl<T: CoreStateState + Send + 'static> PinnedCoreStateState for T {
    fn ctrl_msg(
        self: Box<Self>,
        common: &'_ mut CommonState,
        msg: LinkSetControlCommand,
    ) -> Pin<Box<dyn Future<Output = States> + Send + '_>> {
        Box::pin(async move { self.ctrl_msg(common, msg).await })
    }
    fn link_msg(
        self: Box<Self>,
        common: &'_ mut CommonState,
        id: u64,
        msg: LinkProtocol,
    ) -> Pin<Box<dyn Future<Output = States> + Send + '_>> {
        Box::pin(async move { self.link_msg(common, id, msg).await })
    }

    fn timer<'a>(
        self: Box<Self>,
        common: &'a mut CommonState,
    ) -> Pin<Box<dyn Future<Output = States> + Send + 'a>> {
        Box::pin(async move { self.timer(common).await })
    }
}
