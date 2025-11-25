use std::{error::Error, marker::PhantomData, time::Duration};

use tokio::sync::mpsc::{Receiver, Sender};

use crate::{
    LinkError, LinkResult, LinkSetReader,
    core::start_core,
    epoch::Epoch,
    links::{
        connector::{LinkConnector, PinnedLinkConnector},
        link::PinnedLink,
    },
};

pub trait LinkSetSendable: Send + Sync + 'static {
    type E: Error + Send;
    fn to_bytes(self) -> Result<Vec<u8>, Self::E>;
    fn from_bytes(bytes: Vec<u8>) -> Result<Self, Self::E>
    where
        Self: Sized;
}

pub(crate) enum LinkSetControl {
    Command(LinkSetControlCommand),
    Config(LinkSetControlConfig),
}

pub(crate) enum LinkSetControlCommand {
    /// Cause the LinkSet to attempt to connect using stored addresses, if any.
    Connect,
    /// Cause the LinkSet to disconnect, clearing all active links it may contain
    Disconnect,
    /// Adds a new connector to the link set for the purposes of reestablishing
    /// a broken connection.
    AddConnector(Box<dyn PinnedLinkConnector>),
    
    /// Adds an address to the link set to use when connecting. The bool
    /// indicates if the address should be used multiple times, or just once
    AddAddress{addr: String, reuse: bool},
    /// Add a new link to the LinkSet (moves the set towards connected)
    AddLink(Box<dyn PinnedLink>),
    /// Send a message across the link set
    Message(Vec<u8>, Option<Epoch>),
}

pub(crate) enum LinkSetControlConfig {
/// Enables or disables the LinkSet auto reconnect feature
    AutoConnect(bool),

    /// The amount of time the system is willing to try reconnecting with the
    /// same epoch
    ReconnectTimeout(Option<Duration>),

    /// The amount of time the system is willing to wait for an incoming
    /// reconnection with the same epoch
    GracePeriod(Option<Duration>),
}

/// Indicates changes to the status of the LinkSet, and if a message was
/// received.
#[derive(Debug)]
pub(crate) enum LinkSetMessageInner {
    /// The LinkSet has disconnected
    Disconnected,

    /// The LinkSet successfully established a connection
    Connected(Epoch),

    /// The LinkSet has started or stopped reconnecting.
    ///
    /// A reconnection allows the connection to be reestablished within a
    /// certain time period without causing a Disconnected/Connected cycle.
    /// However, it may still be useful to know if the LinkSet is attempting a
    /// reconnect to provide it with new addresses to help its attempt.
    ///
    /// These messages are not sent unless the LinkSet's reconnect feature is
    /// enabled.
    Connecting(bool),

    /// A [Message] was received with the given epoch
    Message(Vec<u8>, Epoch),
}

/// Indicates changes to the status of the LinkSet, and if a message was
/// received.
#[derive(Debug)]
pub enum LinkSetMessage<M: LinkSetSendable> {
    /// The LinkSet has disconnected
    Disconnected,

    /// The LinkSet successfully established a connection
    Connected(Epoch),

    /// The LinkSet has started or stopped reconnecting.
    ///
    /// A reconnection allows the connection to be reestablished within a
    /// certain time period without causing a Disconnected/Connected cycle.
    /// However, it may still be useful to know if the LinkSet is attempting a
    /// reconnect to provide it with new addresses to help its attempt.
    ///
    /// These messages are not sent unless the LinkSet's reconnect feature is
    /// enabled.
    Connecting(bool),

    /// A [Message] was received with the given epoch
    Message(M, Epoch),
}

impl<M: LinkSetSendable> TryFrom<LinkSetMessageInner> for LinkSetMessage<M> {
    type Error = LinkError;
    fn try_from(value: LinkSetMessageInner) -> Result<Self, Self::Error> {
        match value {
            LinkSetMessageInner::Disconnected => Ok(LinkSetMessage::Disconnected),
            LinkSetMessageInner::Connected(epoch) => Ok(LinkSetMessage::Connected(epoch)),
            LinkSetMessageInner::Connecting(is_connecting) => {
                Ok(LinkSetMessage::Connecting(is_connecting))
            }
            LinkSetMessageInner::Message(bytes, epoch) => Ok(LinkSetMessage::Message(
                M::from_bytes(bytes)
                    .map_err(|e| LinkError::SendableDeserialization(Box::new(e)))?,
                epoch,
            )),
        }
    }
}

/// LinkSet manages a set of Links to another member of the network, and
/// transmits messages across them. It also manages connecting and reconnecting.
pub struct LinkSet<M: LinkSetSendable> {
    to_core: Sender<LinkSetControl>,
    from_core: Option<Receiver<LinkSetMessageInner>>,
    state: Option<Epoch>,
    _phantom: PhantomData<M>,
}

impl<M: LinkSetSendable> LinkSet<M> {
    /// Create a new LinkSet
    pub fn new() -> Self {
        let (to_core, from_core) = start_core();
        Self {
            to_core,
            from_core: Some(from_core),
            state: None,
            _phantom: PhantomData,
        }
    }

    /// Cause the LinkSet to attempt to connect using stored addresses, if any.
    pub async fn connect(&self) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Command(LinkSetControlCommand::Connect))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Cause the LinkSet to disconnect, clearing all active links it may contain
    pub async fn disconnect(&self) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Command(LinkSetControlCommand::Disconnect))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Adds a new connector to the link set for the purposes of reestablishing
    /// a broken connection.
    pub async fn add_connector<C: LinkConnector>(&self, conn: C) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Command(LinkSetControlCommand::AddConnector(
                Box::new(conn) as Box<dyn PinnedLinkConnector>
            )))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Enables or disables the LinkSet auto reconnect feature
    pub async fn set_auto_connect(&self, reconnect: bool) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Config(LinkSetControlConfig::AutoConnect(reconnect)))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Sets the reconnection timeout, the time the link will spend attempting a
    /// reconnection with the same epoch
    pub async fn set_reconnection_timeout(&self, timeout: Option<Duration>) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Config(LinkSetControlConfig::ReconnectTimeout(timeout)))
            .await
            .map_err(|_| LinkError::Terminated)
    }


    /// Sets the grace period timeout, the time the link set will wait for an
    /// incoming connection and still use the same epoch
    pub async fn set_grace_period_timeout(&self, grace_period: Option<Duration>) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Config(LinkSetControlConfig::GracePeriod(grace_period)))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Adds an address to the link set to use for auto reconnection
    pub async fn add_addr(&self, addr: String) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Command(LinkSetControlCommand::AddAddress { addr, reuse: true }))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Adds an address to the link set to try to use for auto reconnection one time
    pub async fn try_addr(&self, addr: String) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Command(LinkSetControlCommand::AddAddress  { addr, reuse: false }))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Add a new link to the LinkSet
    pub async fn add_link<L>(&self, link: L) -> LinkResult
    where
        L: Into<Box<dyn PinnedLink>> + 'static,
    {
        self.to_core
            .send(LinkSetControl::Command(LinkSetControlCommand::AddLink(link.into())))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Send a message across the link set
    pub async fn send(&self, msg: M) -> LinkResult {
        let msg = msg
            .to_bytes()
            .map_err(|e| LinkError::SendableSerialization(Box::new(e)))?;
        self.to_core
            .send(LinkSetControl::Command(LinkSetControlCommand::Message(msg, None)))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Send a message across the link set, using the epoch to prevent this
    /// message to be sent after a reconnection.
    pub async fn send_with_epoch(&self, msg: M, epoch: Epoch) -> LinkResult {
        let msg = msg
            .to_bytes()
            .map_err(|e| LinkError::SendableSerialization(Box::new(e)))?;
        self.to_core
            .send(LinkSetControl::Command(LinkSetControlCommand::Message(msg, Some(epoch))))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Send a message across the link set, using the epoch to prevent this
    /// message to be sent after a reconnection.
    pub async fn send_opt_epoch(&self, msg: M, epoch: Option<Epoch>) -> LinkResult {
        let msg = msg
            .to_bytes()
            .map_err(|e| LinkError::SendableSerialization(Box::new(e)))?;
        self.to_core
            .send(LinkSetControl::Command(LinkSetControlCommand::Message(msg, epoch)))
            .await
            .map_err(|_| LinkError::Terminated)
    }

    /// Get a message from the link set
    pub async fn recv(&mut self) -> LinkResult<LinkSetMessage<M>> {
        let ret = self
            .from_core
            .as_mut()
            .ok_or(LinkError::ReceiverTaken)?
            .recv()
            .await
            .ok_or(LinkError::Terminated)?
            .try_into()?;

        if let LinkSetMessage::Connected(epoch) = &ret {
            self.state = Some(*epoch);
        }
        if let LinkSetMessage::Disconnected = &ret {
            self.state = None;
        }
        Ok(ret)
    }

    /// Takes the receiver part of this LinkSet
    pub fn take_recv(&mut self) -> Option<LinkSetReader<M>> {
        let reader = self.from_core.take()?;
        let epoch = self.state.take();
        Some(LinkSetReader::<M>::new(reader, epoch))
    }

    /// Clones the sender part of this LinkSet, the clone will behave as if the receiver was taken.
    pub fn clone_sender(&self) -> Self {
        Self {
            to_core: self.to_core.clone(),
            from_core: None,
            state: None,
            _phantom: PhantomData,
        }
    }
}

impl<M: LinkSetSendable> ::core::fmt::Debug for LinkSet<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = if self.from_core.is_some() {
            // has recv
            match self.state {
                Some(epoch) => format!("Connected(epoch={})", epoch),
                None => String::from("Disconnected"),
            }
        } else {
            // recv was taken
            String::from("Taken")
        };

        write!(f, "LinkSet {{ status: {} }}", state)
    }
}
