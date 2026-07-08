use std::time::Duration;

use tokio::time::Instant;
use tracing::{trace, warn};

use crate::{
    connector_manager::{AddressSet, ConnectorSet},
    debug::{DebugCommand, DebugReplySnapshot, LinkDescription},
    epoch::Epoch,
    link_set::controller::{LinkSetControlCommand, LinkSetMessageInner},
    links::{LinkEntry, link_manager::LinkManager},
    message_manager::MessageManager,
    protocol::LinkProtocol,
    state::{
        State,
        common::CommonState,
        states::{
            States, disconnected::Disconnected, epoch_mismatch::EpochMismatch,
            grace_period::GracePeriod, reconnecting::Reconnecting,
        },
        transition::{StateTransitionWithParamAsync, to_state, to_state_async},
    },
};

const CONNECTED_TIMER_INTERVAL: Duration = Duration::from_secs(5);
const PING_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct Connected {
    /// Connections available to the connection manager to connect
    pub(crate) conns: ConnectorSet,
    /// Addresses to be copied to the connection manager when time to connect
    pub(crate) addrs: AddressSet,

    pub(crate) links: LinkManager,

    pub(crate) msg_mgr: MessageManager,

    pub(crate) epoch: Epoch,

    pub(crate) last_ping: Instant,
}

impl Connected {
    /// Determine how the connected state should transition if it detects it is
    /// no longer connected
    async fn determine_state(self: Box<Self>, common: &mut CommonState) -> States {
        if self.links.is_empty() {
            // if reconnecting is enabled, go to reconnecting state
            // otherwise, if grace period is enabled, go there
            // else go to disconnected

            if let Some(Duration::ZERO) = common.reconnecting_timeout() {
                // Skip reconnecting
            } else {
                return to_state_async::<Reconnecting, _>(self, common).await;
            }

            if let Some(Duration::ZERO) = common.grace_period_timeout() {
                // Skip grace period
            } else {
                return to_state::<GracePeriod, _>(self, common);
            }

            // Otherwise, disconnect
            to_state_async::<Disconnected, _>(self, common).await
        } else {
            self.into()
        }
    }
}

impl State for Connected {
    async fn ctrl_msg(
        mut self: Box<Self>,
        common: &mut CommonState,
        msg: LinkSetControlCommand,
    ) -> States {
        match msg {
            LinkSetControlCommand::Connect => self.into(),
            LinkSetControlCommand::Disconnect => {
                to_state_async::<Disconnected, _>(self, common).await
            }
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
            LinkSetControlCommand::AddAddress(addr) => {
                if let Some(old) = self.addrs.get(&addr) {
                    old.merge(addr);
                } else {
                    self.addrs.insert(addr);
                }
                self.into()
            }
            LinkSetControlCommand::AddLink(link) => {
                if let Ok(link) = common.wrap_link(link) {
                    self.links.add_link(link);
                } // Else: it was not valid for some reason
                self.into()
            }
            LinkSetControlCommand::Message(msg, epoch) => {
                // make sure the epoch matches the current state
                if let Some(epoch) = epoch
                    && self.epoch != epoch
                {
                    return self.into();
                }

                let seq = self.msg_mgr.insert_msg(msg);
                if let Some(slice_mgr) = self.msg_mgr.get_outgoing_slice(seq) {
                    let _ = self.links.send_slices(slice_mgr).await;
                }

                self.determine_state(common).await
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
                // only respond to requests
                if request {
                    if epoch == Some(self.epoch) {
                        // reply with our epoch
                        let _ = self.links.reset(Some(self.epoch), false).await;
                        self.determine_state(common).await
                    } else {
                        // a mismatch has occurred, goto proper state to solve it
                        to_state_async::<EpochMismatch, _>(self, common).await
                    }
                } else {
                    self.into()
                }
            }
            LinkProtocol::Ack {
                epoch,
                seq,
                last_index,
            } => {
                trace!("Got Ack for epoch {:?}", epoch);
                if epoch == self.epoch.to_int() {
                    self.msg_mgr.recv_ack((seq, last_index));
                }
                self.into()
            }
            LinkProtocol::MsgSlice {
                epoch,
                seq,
                seq_len,
                first_index,
                data,
            } => {
                trace!("got MsgSlice with epoch {:?}", epoch);
                if epoch == self.epoch.to_int() {
                    let should_ack = self.msg_mgr.recv(seq, first_index, seq_len, data);
                    if should_ack {
                        let ack = self.msg_mgr.last_ack();
                        let _ = self.links.ack(self.epoch, ack).await;
                    }

                    for msg in self.msg_mgr.take_recvd() {
                        trace!("Got recvd message");
                        let _ = common
                            .get_to_ctrl()
                            .send(LinkSetMessageInner::Message(msg, self.epoch))
                            .await;
                    }
                    self.determine_state(common).await
                } else {
                    self.into()
                }
            }

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
        if self.last_ping.elapsed() > PING_INTERVAL {
            self.links.ping_all().await;
        }

        self.links.upkeep();

        if !self.links.is_empty() {
            // resend earliest sequence
            if let Some(slice_mgr) = self.msg_mgr.get_outgoing_slices().take(1).next() {
                _ = self.links.send_slices(slice_mgr).await;
            }
        }

        self.determine_state(common).await
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
                let addrs = self.addrs.iter().map(|a| a.addr().clone()).collect();
                let conns = self.conns.iter().map(|c| c.1.scheme()).collect();
                let states = States::from(self);
                let reply = DebugReplySnapshot {
                    state_name: states.get_name().to_owned(),
                    epoch: Some(epoch),
                    addrs,
                    connectors: conns,
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

impl StateTransitionWithParamAsync<EpochMismatch, Epoch> for Connected {
    async fn transition_from(
        mut old_state: Box<EpochMismatch>,
        common: &mut CommonState,
        epoch: Epoch,
    ) -> Box<Connected> {
        let _ = common
            .get_to_ctrl()
            .send(LinkSetMessageInner::Connected(epoch))
            .await;
        let mut msg_mgr = MessageManager::new(epoch);
        for data in old_state.queued_msgs {
            trace!("sending queued message");
            let seq = msg_mgr.insert_msg(data);
            let slice_mgr = msg_mgr
                .get_outgoing_slice(seq)
                .expect("slice was just inserted");
            let _ = old_state.links.send_slices(slice_mgr).await;
        }

        common.get_timer().clear();
        common.get_timer().modify_repeat(CONNECTED_TIMER_INTERVAL);
        Box::new(Self {
            conns: old_state.conns,
            addrs: old_state.addrs,
            links: old_state.links,
            msg_mgr,

            epoch,
            last_ping: old_state.last_ping,
        })
    }
}

impl StateTransitionWithParamAsync<Reconnecting, LinkEntry> for Connected {
    async fn transition_from(
        old_state: Box<Reconnecting>,
        common: &mut CommonState,
        link: LinkEntry,
    ) -> Box<Self> {
        let (conns, addrs) = old_state.connector.cancel().await;

        let mut links = LinkManager::with_link(link);
        links.ping_all().await;

        let _ = common
            .get_to_ctrl()
            .send(LinkSetMessageInner::AttemptingConnection(false))
            .await;

        common.get_timer().clear();
        common.get_timer().modify_repeat(CONNECTED_TIMER_INTERVAL);
        Box::new(Self {
            conns,
            addrs,
            links,
            msg_mgr: old_state.msg_mgr,
            epoch: old_state.epoch,
            last_ping: Instant::now(),
        })
    }
}

impl StateTransitionWithParamAsync<GracePeriod, LinkEntry> for Connected {
    async fn transition_from(
        old_state: Box<GracePeriod>,
        common: &mut CommonState,
        link: LinkEntry,
    ) -> Box<Self> {
        let mut links = LinkManager::with_link(link);
        links.ping_all().await;

        common.get_timer().clear();
        common.get_timer().modify_repeat(CONNECTED_TIMER_INTERVAL);
        Box::new(Self {
            conns: old_state.conns,
            addrs: old_state.addrs,
            links,
            msg_mgr: old_state.msg_mgr,
            epoch: old_state.epoch,
            last_ping: Instant::now(),
        })
    }
}
