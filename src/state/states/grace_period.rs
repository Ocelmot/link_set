use tracing::trace;

use crate::{
    epoch::Epoch,
    link_set::controller::{LinkSetControlCommand, LinkSetMessageInner},
    links::connector::PinnedLinkConnector,
    message_manager::MessageManager,
    protocol::LinkProtocol,
    state::{
        state::{CommonState, CoreStateState},
        states::{
            StateTransitionFrom, connected::Connected, connecting::Connecting,
            disconnected::Disconnected, to_state_async, to_state_param_async,
        },
    },
};

pub(crate) struct GracePeriod {
    /// Connections available to the connection manager to connect
    pub(crate) conns: Vec<Box<dyn PinnedLinkConnector>>,
    /// Addresses to be copied to the connection manager when time to connect
    pub(crate) addrs: Vec<(String, bool)>,

    pub(crate) msg_mgr: MessageManager,
    pub(crate) epoch: Epoch,
}

impl CoreStateState for GracePeriod {
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
                self.conns.push(conn);
                self.into()
            }
            LinkSetControlCommand::AddAddress { addr, reuse } => {
                self.addrs.push((addr, reuse));
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
}

impl StateTransitionFrom<Connected> for GracePeriod {
    fn transition_from(old_state: Box<Connected>, common: &mut CommonState) -> Box<Self> {
        if let Some(timeout) = common.grace_period_timeout().clone() {
            common.get_timer().set_deadline_from_now(timeout);
        } else {
            common.get_timer().clear();
        }

        Box::new(GracePeriod {
            conns: old_state.conns,
            addrs: old_state.addrs,
            msg_mgr: old_state.msg_mgr,
            epoch: old_state.epoch,
        })
    }
}
