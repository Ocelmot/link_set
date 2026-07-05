use std::{cmp::Ordering, collections::HashMap, time::Duration};

use tokio::time::Instant;
use tracing::{error, trace, warn};

const EPOCH_MISMATCH_TIMER_INTERVAL: Duration = Duration::from_secs(2);

use crate::{
    debug::{DebugCommand, DebugReplySnapshot, LinkDescription},
    epoch::{Epoch, compare_epochs},
    link_set::controller::{LinkSetControlCommand, LinkSetMessageInner},
    links::{Address, LinkEntry, connector::PinnedLinkConnector, link_manager::LinkManager},
    protocol::LinkProtocol,
    state::{
        State,
        common::CommonState,
        states::{
            States, connected::Connected, connecting::Connecting, disconnected::Disconnected,
        },
        transition::{
            StateTransitionFromAsync, StateTransitionWithParamAsync, to_state_async,
            to_state_param_async,
        },
    },
};

pub(crate) struct EpochMismatch {
    /// Connections available to the connection manager to connect
    pub(crate) conns: HashMap<String, Box<dyn PinnedLinkConnector>>,
    /// Addresses to be copied to the connection manager when time to connect
    pub(crate) addrs: Vec<(Address, bool)>,

    pub(crate) links: LinkManager,
    pub(crate) queued_msgs: Vec<Vec<u8>>,
    pub(crate) epoch: Option<Epoch>,
    pub(crate) last_ping: Instant,
}

impl State for EpochMismatch {
    async fn ctrl_msg(
        mut self: Box<Self>,
        common: &mut CommonState,
        msg: LinkSetControlCommand,
    ) -> States {
        // EpochMismatch needs to have a target epoch that determines what kinds
        // of responses it sends to the other node. Each side will repeatedly
        // request its own epoch from the other side until the other side
        // returns an epoch that matches if the other side sends an epoch that
        // is greater than its own epoch, this node will adopt that as its
        // epoch. None epochs will always compare smaller than anything else,
        // and upon receiving none while our epoch is none, will increment to 1

        // Epoch type itself should implement this kind of comparison/updating.
        // Given another epoch, the Epoch should update itself accordingly. then
        // reply with the current epoch.

        // If all the links fail, move to the same state as if we were
        // connected. I.E. disconnected, reconnecting, grace_period

        match msg {
            LinkSetControlCommand::Connect => self.into(),
            LinkSetControlCommand::Disconnect => {
                to_state_async::<Disconnected, _>(self, common).await
            }
            LinkSetControlCommand::AddConnector(conn) => {
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
            LinkSetControlCommand::AddLink(link) => {
                if let Ok(link) = common.wrap_link(link) {
                    self.links.add_link(link);
                } // Else: it was not valid for some reason
                self.into()
            }
            LinkSetControlCommand::Message(msg, epoch) => {
                // At this state, we are still not connected, so the same rules
                // apply as for the Connecting state
                if epoch.is_none() {
                    self.queued_msgs.push(msg);
                }
                self.into()
            }
        }
    }

    async fn link_msg(
        mut self: Box<Self>,
        common: &mut CommonState,
        id: u64,
        msg: LinkProtocol,
    ) -> States {
        match msg {
            LinkProtocol::Reset { epoch, request } => {
                trace!(
                    "got reset with epoch {:?}, was request: {:?}",
                    epoch, request
                );
                match compare_epochs(&self.epoch, &epoch) {
                    Ordering::Greater => {
                        // if our epoch is greater than theirs, reply to them,
                        // but do not move to connected until they send
                        // confirmation
                        let _ = self.links.reset(self.epoch, false).await;

                        self.into()
                    }
                    Ordering::Equal => {
                        // if the epochs are equal, move to connected
                        if request {
                            let _ = self.links.reset(self.epoch, false).await;
                        }

                        let epoch = self
                            .epoch
                            .expect("two null opt<epochs> should not compare equally");
                        to_state_param_async::<Connected, _, _>(self, common, epoch).await
                    }
                    Ordering::Less => {
                        // If our epoch is less than theirs, we should adopt
                        // theirs and move to connected
                        self.epoch = epoch;
                        // if we compared less because both were none, start at one
                        let epoch = self.epoch.unwrap_or(Epoch::ONE);

                        // Since we have just changed our epoch, let them know.
                        let _ = self.links.reset(Some(epoch), false).await;

                        to_state_param_async::<Connected, _, _>(self, common, epoch).await
                    }
                }
            }

            // Since there is no active Epoch, Ack and MsgSlice have nothing to do
            LinkProtocol::Ack { .. } => self.into(),
            LinkProtocol::MsgSlice { .. } => self.into(),

            // Since there are connections, their upkeep still needs to be done
            LinkProtocol::Ping => {
                // reply over same channel
                self.links.pong(id).await;
                self.into()
            }
            LinkProtocol::Pong => {
                // register that the ping was received
                self.links.end_ping(id);
                self.into()
            }
        }
    }

    async fn timer(mut self: Box<Self>, common: &mut CommonState) -> States {
        // An epoch mismatch timer indicates that it is time to check that the
        // links are still active, and resend the epoch request

        // link upkeep should purge disconnected links.
        self.links.upkeep();
        self.links.ping_all().await;

        if self.links.is_empty() {
            if common.auto_connect() {
                return to_state_async::<Connecting, _>(self, common).await;
            } else {
                return to_state_async::<Disconnected, _>(self, common).await;
            }
        }

        let _ = self.links.reset(self.epoch, true).await;

        self.into()
    }

    async fn debug(mut self: Box<Self>, _common: &mut CommonState, cmd: DebugCommand) -> States {
        match cmd {
            DebugCommand::Snapshot(sender) => {
                let links = {
                    let mut descriptions = self
                        .links
                        .link_entries()
                        .map(|entry| LinkDescription {
                            id: entry.id(),
                            scheme: entry.scheme().to_string(),
                        })
                        .collect::<Vec<_>>();
                    descriptions.sort_by(|a, b| a.id.cmp(&b.id));
                    descriptions
                };
                let epoch = self.epoch;
                let addrs = self.addrs.iter().map(|a|a.0.clone()).collect();
                let conns = self.conns.iter().map(|c|c.1.scheme()).collect();
                let states = States::from(self);
                let reply = DebugReplySnapshot {
                    state_name: states.get_name().to_owned(),
                    addrs,
                    connectors: conns,
                    epoch,
                    links,
                };
                let _ = sender.send(reply);
                states
            }
            DebugCommand::EvictLink(sender, id) => {
                self.links.evict_link(id);
                let _ = sender.send(true);
                self.into()
            }
        }
    }
}

impl StateTransitionWithParamAsync<Disconnected, LinkEntry> for EpochMismatch {
    async fn transition_from(
        old_state: Box<Disconnected>,
        common: &mut CommonState,
        link: LinkEntry,
    ) -> Box<EpochMismatch> {
        let mut links = LinkManager::new();
        links.add_link(link);
        links.ping_all().await;
        let _ = links.reset(old_state.epoch, true).await;

        common
            .get_timer()
            .modify_repeat(EPOCH_MISMATCH_TIMER_INTERVAL);

        Box::new(EpochMismatch {
            conns: old_state.conns,
            addrs: old_state.addrs,
            links,
            queued_msgs: Vec::new(),
            epoch: old_state.epoch,
            last_ping: Instant::now(),
        })
    }
}

impl StateTransitionWithParamAsync<Connecting, LinkEntry> for EpochMismatch {
    async fn transition_from(
        old_state: Box<Connecting>,
        common: &mut CommonState,
        link: LinkEntry,
    ) -> Box<EpochMismatch> {
        let (conns, addrs) = match old_state.connector.cancel().await {
            Ok((conns, addrs)) => (conns, addrs),
            Err(_) => {
                error!("Panic occurred in connector manager");
                // Should probably close down here so it can be handled properly by the user
                (HashMap::new(), Vec::new())
            }
        };

        let mut links = LinkManager::new();
        links.add_link(link);
        links.ping_all().await;
        let _ = links.reset(old_state.epoch, true).await;

        let _ = common
            .get_to_ctrl()
            .send(LinkSetMessageInner::AttemptingConnection(false))
            .await;

        common
            .get_timer()
            .modify_repeat(EPOCH_MISMATCH_TIMER_INTERVAL);

        Box::new(EpochMismatch {
            conns,
            addrs,
            links,
            queued_msgs: old_state.queued_msgs,
            epoch: old_state.epoch,
            last_ping: Instant::now(),
        })
    }
}

impl StateTransitionFromAsync<Connected> for EpochMismatch {
    async fn transition_from(mut old_state: Box<Connected>, common: &mut CommonState) -> Box<Self> {
        let epoch = Some(old_state.epoch.increment());
        let _ = old_state.links.reset(epoch, true).await;

        common
            .get_timer()
            .modify_repeat(EPOCH_MISMATCH_TIMER_INTERVAL);

        Box::new(EpochMismatch {
            conns: old_state.conns,
            addrs: old_state.addrs,
            links: old_state.links,
            queued_msgs: Vec::new(),
            epoch,
            last_ping: old_state.last_ping,
        })
    }
}
