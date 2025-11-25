use tracing::trace;

use crate::{
    connector_manager::ConnectorManager,
    epoch::Epoch,
    link_set::controller::{LinkSetControlCommand, LinkSetMessageInner},
    protocol::LinkProtocol,
    state::{
        state::{CommonState, CoreStateState},
        states::{
            StateTransitionFrom, StateTransitionFromAsync, StateTransitionWithParam, States, connected::Connected, disconnected::Disconnected, epoch_mismatch::EpochMismatch, grace_period::GracePeriod, reconnecting::Reconnecting, to_state_async, to_state_param_async
        },
    },
};

pub(crate) struct Connecting {
    pub(crate) connector: ConnectorManager,
    pub(crate) queued_msgs: Vec<Vec<u8>>,
    pub(crate) epoch: Option<Epoch>,
}

impl CoreStateState for Connecting {
    async fn ctrl_msg(
        mut self: Box<Self>,
        common: &mut CommonState,
        msg: LinkSetControlCommand,
    ) -> States {
        match msg {
            LinkSetControlCommand::Connect => self.into(), // already connecting
            LinkSetControlCommand::Disconnect => {
                to_state_async::<Disconnected, _>(self, common).await
            }
            LinkSetControlCommand::AddConnector(conn) => {
                self.connector.add_connector(conn).await;
                self.into()
            }
            LinkSetControlCommand::AddAddress { addr, reuse } => {
                if reuse {
                    self.connector.add_addr(addr).await;
                } else {
                    self.connector.try_addr(addr).await;
                }
                self.into()
            }
            LinkSetControlCommand::AddLink(link) => match common.wrap_link(link) {
                Ok(link) => to_state_param_async::<EpochMismatch, _, _>(self, common, link).await,
                Err(err) => {
                    trace!("Failed to wrap link: {}", err);
                    self.into()
                }
            },
            LinkSetControlCommand::Message(msg, epoch) => {
                // since there is no current epoch, only messages with no epoch
                // could be queued for later
                if epoch.is_none() {
                    self.queued_msgs.push(msg);
                }
                self.into()
            }
        }
    }

    async fn link_msg(
        self: Box<Self>,
        _common: &mut CommonState,
        _id: u64,
        _msg: LinkProtocol,
    ) -> States {
        // There is nothing to do with any input
        self.into()
    }

    async fn timer(self: Box<Self>, common: &mut CommonState) -> States {
        // The Connecting timer indicates that the connection attempt has timed
        // out and become disconnected
        common.get_timer().clear();
        to_state_async::<Disconnected, _>(self, common).await
    }
}

impl StateTransitionFrom<Disconnected> for Connecting {
    fn transition_from(old_state: Box<Disconnected>, common: &mut CommonState) -> Box<Connecting> {
        let to_core = common.get_to_core().clone();

        // get timeout from common, set timer to new interval
        common.get_timer().clear();
        Box::new(Connecting {
            connector: ConnectorManager::start(to_core, old_state.addrs, old_state.conns),
            queued_msgs: Vec::new(),
            epoch: old_state.epoch,
        })
    }
}

impl StateTransitionWithParam<Disconnected, Vec<u8>> for Connecting {
    fn transition_from(
        old_state: Box<Disconnected>,
        common: &mut CommonState,
        msg: Vec<u8>,
    ) -> Box<Connecting> {
        let to_core = common.get_to_core().clone();
        Box::new(Connecting {
            connector: ConnectorManager::start(to_core, old_state.addrs, old_state.conns),
            queued_msgs: vec![msg],
            epoch: old_state.epoch,
        })
    }
}

impl StateTransitionFrom<EpochMismatch> for Connecting {
    fn transition_from(old_state: Box<EpochMismatch>, common: &mut CommonState) -> Box<Self> {
        let to_core = common.get_to_core().clone();
        Box::new(Connecting {
            connector: ConnectorManager::start(to_core, old_state.addrs, old_state.conns),
            queued_msgs: Vec::new(),
            epoch: old_state.epoch,
        })
    }
}

impl StateTransitionFrom<Connected> for Connecting {
    fn transition_from(old_state: Box<Connected>, common: &mut CommonState) -> Box<Self> {
        let to_core = common.get_to_core().clone();
        Box::new(Connecting {
            connector: ConnectorManager::start(to_core, old_state.addrs, old_state.conns),
            queued_msgs: Vec::new(),
            epoch: Some(old_state.epoch.increment()),
        })
    }
}

impl StateTransitionFrom<Reconnecting> for Connecting {
    fn transition_from(old_state: Box<Reconnecting>, _common: &mut CommonState) -> Box<Self> {
        Box::new(Connecting {
            connector: old_state.connector,
            queued_msgs: Vec::new(),
            epoch: Some(old_state.epoch.increment()),
        })
    }
}

impl StateTransitionFromAsync<GracePeriod> for Connecting {
    async fn transition_from(old_state: Box<GracePeriod>, common: &mut CommonState) -> Box<Self> {
        let _ = common.get_to_ctrl().send(LinkSetMessageInner::Disconnected).await;
        let to_core = common.get_to_core().clone();
        Box::new(Connecting {
            connector: ConnectorManager::start(to_core, old_state.addrs, old_state.conns),
            queued_msgs: Vec::new(),
            epoch: Some(old_state.epoch.increment()),
        })
    }
}
