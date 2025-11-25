use tracing::trace;

use crate::{
    connector_manager::ConnectorManager,
    epoch::Epoch,
    link_set::controller::{LinkSetControlCommand, LinkSetMessageInner},
    message_manager::MessageManager,
    protocol::LinkProtocol,
    state::{
        state::{CommonState, CoreStateState},
        states::{
            StateTransitionFrom, States, connected::Connected, connecting::Connecting,
            disconnected::Disconnected, to_state, to_state_async, to_state_param_async,
        },
    },
};

pub(crate) struct Reconnecting {
    pub(crate) msg_mgr: MessageManager,
    pub(crate) epoch: Epoch,
    pub(crate) connector: ConnectorManager,
}

impl CoreStateState for Reconnecting {
    async fn ctrl_msg(
        mut self: Box<Self>,
        common: &mut crate::state::state::CommonState,
        msg: LinkSetControlCommand,
    ) -> States {
        match msg {
            LinkSetControlCommand::Connect => self.into(),
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
            LinkSetControlCommand::AddLink(link) => {
                if let Ok(link) = common.wrap_link(link) {
                    to_state_param_async::<Connected, _, _>(self, common, link).await
                } else {
                    self.into()
                }
            }
            LinkSetControlCommand::Message(data, epoch) => {
                if let Some(epoch) = epoch {
                    if self.epoch != epoch {
                        return self.into();
                    }
                }
                self.msg_mgr.insert_msg(data);
                self.into()
            }
        }
    }

    async fn link_msg(
        mut self: Box<Self>,
        common: &mut crate::state::state::CommonState,
        _id: u64,
        msg: crate::protocol::LinkProtocol,
    ) -> States {
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
                        // a mismatch has occurred, and we are disconnected,
                        // goto Connecting
                        to_state::<Connecting, _>(self, common)
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

    async fn timer(self: Box<Self>, common: &mut CommonState) -> States {
        // reconnecting period has finished, move to next state
        if common.auto_connect() {
            to_state::<Connecting, _>(self, common)
        } else {
            to_state_async::<Disconnected, _>(self, common).await
        }
    }
}

impl StateTransitionFrom<Connected> for Reconnecting {
    fn transition_from(old_state: Box<Connected>, common: &mut CommonState) -> Box<Self> {
        if let Some(timeout) = common.reconnecting_timeout().clone() {
            common.get_timer().set_deadline_from_now(timeout);
        } else {
            common.get_timer().clear();
        }

        let connector = ConnectorManager::start(
            common.get_to_core().clone(),
            old_state.addrs,
            old_state.conns,
        );
        Box::new(Reconnecting {
            connector,
            msg_mgr: old_state.msg_mgr,
            epoch: old_state.epoch,
        })
    }
}
