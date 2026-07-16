use std::sync::atomic::Ordering;

use tracing::{trace, warn};

use crate::{
    connector_manager::{AddressSet, ConnectorSet},
    debug::{DebugCommand, DebugReplySnapshot},
    epoch::Epoch,
    link_set::controller::{LinkSetControlCommand, LinkSetMessageInner},
    message_manager::MessageManager,
    protocol::LinkProtocol,
    state::{
        State,
        common::CommonState,
        states::{
            States, connected::Connected, connecting::Connecting, disconnected::Disconnected,
        },
        transition::{StateTransitionFrom, to_state_async, to_state_param_async},
    },
};

pub(crate) struct GracePeriod {
    /// Connections available to the connection manager to connect
    pub(crate) conns: ConnectorSet,
    /// Addresses to be copied to the connection manager when time to connect
    pub(crate) addrs: AddressSet,

    pub(crate) msg_mgr: MessageManager,
    pub(crate) epoch: Epoch,
}

impl State for GracePeriod {
    async fn ctrl_msg(
        mut self: Box<Self>,
        common: &mut CommonState,
        msg: LinkSetControlCommand,
    ) -> super::States {
        match msg {
            LinkSetControlCommand::Connect => self.into(), // already connected as best we can (maybe move to connecting?)
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
            LinkSetControlCommand::AddLink(link) => match common.wrap_link(link) {
                Ok(link) => to_state_param_async::<Connected, _, _>(self, common, link).await,
                Err(err) => {
                    trace!("Failed to wrap link: {}", err);
                    self.into()
                }
            },
            LinkSetControlCommand::Message(data, epoch) => {
                // make sure the epoch matches the current state
                if let Some(epoch) = epoch
                    && self.epoch != epoch
                {
                    return self.into();
                }

                self.msg_mgr.insert_msg(data);

                self.into()
            }
        }
    }

    async fn link_msg(
        mut self: Box<Self>,
        common: &mut crate::state::common::CommonState,
        _id: u64,
        msg: crate::protocol::LinkProtocol,
    ) -> super::States {
        // This is possible since the read half has been separated from
        // the link itself, and might drop at a different time

        match msg {
            LinkProtocol::Reset { epoch, request } => {
                // if a request for a mismatched epoch arrives, return to
                // EpochMismatch
                if request {
                    if epoch != Some(self.epoch) {
                        // cant reply, ignore
                        self.into()
                    } else {
                        // a mismatch has occurred, and we cannot reconnect,
                        // disconnect
                        to_state_async::<Disconnected, _>(self, common).await
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
                    self.msg_mgr.recv(seq, first_index, seq_len, data);

                    for msg in self.msg_mgr.take_recvd() {
                        trace!("Got recvd message");
                        let _ = common
                            .get_to_ctrl()
                            .send(LinkSetMessageInner::Message(msg, self.epoch))
                            .await;
                    }
                }
                self.into()
            }
            // Cannot send ping or pong since we have no active links. They
            // should soon timeout.
            LinkProtocol::Ping => self.into(),
            LinkProtocol::Pong => self.into(),
        }
    }

    async fn timer(self: Box<Self>, common: &mut CommonState) -> super::States {
        // if the timer is set it is because the grace period elapsed
        if common.auto_connect() {
            to_state_async::<Connecting, _>(self, common).await
        } else {
            to_state_async::<Disconnected, _>(self, common).await
        }
    }

    async fn debug(self: Box<Self>, _common: &mut CommonState, cmd: DebugCommand) -> States {
        let epoch = self.epoch;
        let addrs = self.addrs.iter().map(|a| a.addr().clone()).collect();
        let conns = self.conns.iter().map(|c| c.1.scheme()).collect();
        let states = States::from(self);
        match cmd {
            DebugCommand::Snapshot(sender) => {
                let reply = DebugReplySnapshot {
                    state_name: states.get_name().to_owned(),
                    epoch: Some(epoch),
                    addrs,
                    connectors: conns,
                    links: vec![],
                };
                let _ = sender.send(reply);
            }
            DebugCommand::EvictLink(sender, _id) => {
                let _ = sender.send(false); // disconnected has no links
            }
        }
        states
    }
}

impl StateTransitionFrom<Connected> for GracePeriod {
    fn transition_from(old_state: Box<Connected>, common: &mut CommonState) -> Box<Self> {
        if let Some(timeout) = common.grace_period_timeout().clone() {
            common.get_timer().modify_deadline_from_now(timeout);
        } else {
            common.get_timer().clear();
        }
        common.is_active().store(false, Ordering::Release);

        Box::new(GracePeriod {
            conns: old_state.conns,
            addrs: old_state.addrs,
            msg_mgr: old_state.msg_mgr,
            epoch: old_state.epoch,
        })
    }
}
