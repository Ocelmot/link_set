use tracing::{error, trace};

use crate::{
    epoch::{Epoch, opt_epoch_increment},
    link_set::controller::{LinkSetControlCommand, LinkSetMessageInner},
    links::connector::PinnedLinkConnector,
    protocol::LinkProtocol,
    state::{
        state::{CommonState, CoreStateState},
        states::{
            StateTransitionFromAsync, States,
            connected::Connected, connecting::Connecting, epoch_mismatch::EpochMismatch,
            grace_period::GracePeriod, reconnecting::Reconnecting, to_state, to_state_param, to_state_param_async,
        },
    },
};

pub(crate) struct Disconnected {
    /// Connections available to the connection manager to connect
    pub(crate) conns: Vec<Box<dyn PinnedLinkConnector>>,
    /// Addresses to be copied to the connection manager when time to connect
    pub(crate) addrs: Vec<(String, bool)>,

    /// The next allowable epoch (None means any would be ok)
    pub(crate) epoch: Option<Epoch>,
}

impl Disconnected {
    pub fn new() -> Self {
        Disconnected {
            conns: Vec::new(),
            addrs: Vec::new(),
            epoch: None,
        }
    }
}

impl CoreStateState for Disconnected {
    async fn ctrl_msg(
        mut self: Box<Self>,
        common: &mut CommonState,
        msg: LinkSetControlCommand,
    ) -> States {
        match msg {
            LinkSetControlCommand::Connect => to_state::<Connecting, _>(self, common),
            LinkSetControlCommand::Disconnect => self.into(),
            LinkSetControlCommand::AddConnector(conn) => {
                trace!("LinkSetCore adding connector");
                self.conns.push(conn);
                self.into()
            }
            LinkSetControlCommand::AddAddress { addr, reuse } => {
                self.addrs.push((addr, reuse));
                self.into()
            }
            LinkSetControlCommand::AddLink(link) => match common.wrap_link(link) {
                Ok(link) => to_state_param_async::<EpochMismatch, _, _>(self, common, link).await,
                Err(err) => {
                    trace!("Failed to wrap link: {}", err);
                    self.into()
                }
            },
            LinkSetControlCommand::Message(data, epoch) => {
                // message should trigger connection, if we are able to connect,
                // otherwise discard if message has an epoch, it cannot be
                // correct since we are disconnected
                if !self.conns.is_empty() && !self.addrs.is_empty() && epoch.is_none() {
                    to_state_param::<Connecting, _, _>(self, common, data)
                } else {
                    self.into()
                }
            }
        }
    }

    async fn link_msg(
        self: Box<Self>,
        common: &mut CommonState,
        _id: u64,
        _msg: LinkProtocol,
    ) -> States {
        // There should be no incoming messages while disconnected, clear the
        // readers as a precaution
        common.clear_readers();
        self.into()
    }

    async fn timer(self: Box<Self>, common: &mut CommonState) -> States {
        // Disconnected should never need a timer, so disable it if it occurs
        common.get_timer().clear();
        self.into()
    }
}

impl StateTransitionFromAsync<Connecting> for Disconnected {
    async fn transition_from(
        old_state: Box<Connecting>,
        common: &mut CommonState,
    ) -> Box<Disconnected> {
        let (conns, addrs) = match old_state.connector.cancel().await {
            Ok((conns, addrs)) => (conns, addrs),
            Err(_) => {
                error!("Panic occurred in connector manager");
                // Should probably close down here so it can be handled properly by the user
                (Vec::new(), Vec::new())
            }
        };
        common.get_timer().clear();
        Box::new(Disconnected {
            conns,
            addrs,
            epoch: old_state.epoch,
        })
    }
}

impl StateTransitionFromAsync<EpochMismatch> for Disconnected {
    async fn transition_from(
        old_state: Box<EpochMismatch>,
        common: &mut CommonState,
    ) -> Box<Disconnected> {
        // Since this is moving from a state where the epoch was transmitted to
        // the other side, increment it
        common.get_timer().clear();
        Box::new(Disconnected {
            conns: old_state.conns,
            addrs: old_state.addrs,
            epoch: opt_epoch_increment(old_state.epoch),
        })
    }
}

impl StateTransitionFromAsync<Connected> for Disconnected {
    async fn transition_from(old_state: Box<Connected>, common: &mut CommonState) -> Box<Disconnected> {
        let _ = common.get_to_ctrl().send(LinkSetMessageInner::Disconnected).await;
        common.get_timer().clear();
        Box::new(Disconnected {
            conns: old_state.conns,
            addrs: old_state.addrs,
            epoch: Some(old_state.epoch.increment()),
        })
    }
}

impl StateTransitionFromAsync<Reconnecting> for Disconnected {
    async fn transition_from(old_state: Box<Reconnecting>, common: &mut CommonState) -> Box<Self> {
        let (conns, addrs) = match old_state.connector.cancel().await {
            Ok((conns, addrs)) => (conns, addrs),
            Err(_) => {
                error!("Panic occurred in connector manager");
                // Should probably close down here so it can be handled properly by the user
                (Vec::new(), Vec::new())
            }
        };
        let _ = common.get_to_ctrl().send(LinkSetMessageInner::Disconnected).await;
        common.get_timer().clear();
        Box::new(Disconnected {
            conns,
            addrs,
            epoch: Some(old_state.epoch.increment()),
        })
    }
}

impl StateTransitionFromAsync<GracePeriod> for Disconnected {
    async fn transition_from(old_state: Box<GracePeriod>, common: &mut CommonState) -> Box<Self> {
        common.get_timer().clear();
        let _ = common.get_to_ctrl().send(LinkSetMessageInner::Disconnected).await;
        Box::new(Disconnected {
            conns: old_state.conns,
            addrs: old_state.addrs,
            epoch: Some(old_state.epoch.increment()),
        })
    }
}
