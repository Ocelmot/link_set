use std::collections::HashMap;

use tracing::{error, trace, warn};

use crate::{
    debug::{DebugCommand, DebugReplySnapshot},
    epoch::{Epoch, opt_epoch_increment},
    link_set::controller::{LinkSetControlCommand, LinkSetMessageInner},
    links::{Address, connector::PinnedLinkConnector},
    protocol::LinkProtocol,
    state::{
        State,
        common::CommonState,
        states::{
            States, connected::Connected, connecting::Connecting, epoch_mismatch::EpochMismatch,
            grace_period::GracePeriod, reconnecting::Reconnecting,
        },
        transition::{StateTransitionFromAsync, to_state_async, to_state_param_async},
    },
};

pub(crate) struct Disconnected {
    /// Connections available to the connection manager to connect
    pub(crate) conns: HashMap<String, Box<dyn PinnedLinkConnector>>,
    /// Addresses to be copied to the connection manager when time to connect
    pub(crate) addrs: Vec<(Address, bool)>,

    /// The next allowable epoch (None means any would be ok)
    pub(crate) epoch: Option<Epoch>,
}

impl Disconnected {
    pub fn new() -> Self {
        Disconnected {
            conns: HashMap::new(),
            addrs: Vec::new(),
            epoch: None,
        }
    }
}

impl State for Disconnected {
    async fn ctrl_msg(
        mut self: Box<Self>,
        common: &mut CommonState,
        msg: LinkSetControlCommand,
    ) -> States {
        match msg {
            LinkSetControlCommand::Connect => to_state_async::<Connecting, _>(self, common).await,
            LinkSetControlCommand::Disconnect => self.into(),
            LinkSetControlCommand::AddConnector(conn) => {
                trace!("LinkSetCore adding connector");
                let scheme = conn.scheme();
                if self.conns.insert(scheme.to_string(), conn).is_some() {
                    warn!(
                        "Connector list already contained connector for scheme `{scheme}`! The connector has been replaced."
                    );
                }

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
                if common.auto_connect() && epoch.is_none() {
                    if self.conns.is_empty() {
                        warn!(
                            "Attempting connection with no connectors. Make sure to add a connector."
                        );
                    }
                    to_state_param_async::<Connecting, _, _>(self, common, data).await
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
        common.get_readers_mut().clear();
        self.into()
    }

    async fn timer(self: Box<Self>, common: &mut CommonState) -> States {
        // Disconnected should never need a timer, so disable it if it occurs
        common.get_timer().clear();
        self.into()
    }

    async fn debug(self: Box<Self>, _common: &mut CommonState, cmd: DebugCommand) -> States {
        let epoch = self.epoch;
        let addrs = self.addrs.iter().map(|a|a.0.clone()).collect();
        let conns = self.conns.iter().map(|c|c.1.scheme()).collect();
        let states = States::from(self);
        match cmd {
            DebugCommand::Snapshot(sender) => {
                let reply = DebugReplySnapshot {
                    state_name: states.get_name().to_owned(),
                    epoch,
                    addrs,
                    connectors: conns,
                    links: vec![],
                };
                let _ = sender.send(reply);
            }
            DebugCommand::EvictLink(sender, _id ) => {
                let _ = sender.send(false); // disconnected has no links
            }
        }
        states
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
                (HashMap::new(), Vec::new())
            }
        };
        common.get_timer().clear();
        let _ = common
            .get_to_ctrl()
            .send(LinkSetMessageInner::AttemptingConnection(false))
            .await;
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
    async fn transition_from(
        old_state: Box<Connected>,
        common: &mut CommonState,
    ) -> Box<Disconnected> {
        let _ = common
            .get_to_ctrl()
            .send(LinkSetMessageInner::Disconnected)
            .await;
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
                (HashMap::new(), Vec::new())
            }
        };
        let _ = common
            .get_to_ctrl()
            .send(LinkSetMessageInner::AttemptingConnection(false))
            .await;
        let _ = common
            .get_to_ctrl()
            .send(LinkSetMessageInner::Disconnected)
            .await;
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
        let _ = common
            .get_to_ctrl()
            .send(LinkSetMessageInner::Disconnected)
            .await;
        Box::new(Disconnected {
            conns: old_state.conns,
            addrs: old_state.addrs,
            epoch: Some(old_state.epoch.increment()),
        })
    }
}
