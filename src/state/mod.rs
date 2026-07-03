use std::pin::Pin;

use crate::{
    link_set::controller::LinkSetControlCommand,
    protocol::LinkProtocol,
    state::{common::CommonState, states::States},
};

pub(crate) mod common;
pub(crate) mod states;
pub(crate) mod transition;

pub(crate) trait State {
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

    fn timer(self: Box<Self>, common: &mut CommonState) -> impl Future<Output = States> + Send;
}

pub(crate) trait PinnedState {
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

impl<T: State + Send + 'static> PinnedState for T {
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
