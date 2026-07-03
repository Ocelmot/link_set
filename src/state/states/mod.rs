use crate::state::{
    PinnedState,
    states::{
        connected::Connected, connecting::Connecting, disconnected::Disconnected,
        epoch_mismatch::EpochMismatch, grace_period::GracePeriod, reconnecting::Reconnecting,
    },
};

pub(crate) mod connected;
pub(crate) mod connecting;
pub(crate) mod disconnected;
pub(crate) mod epoch_mismatch;
pub(crate) mod grace_period;
pub(crate) mod reconnecting;

pub(crate) enum States {
    Disconnected(Box<Disconnected>),
    Connecting(Box<Connecting>),
    EpochMismatch(Box<EpochMismatch>),
    Connected(Box<Connected>),
    Reconnecting(Box<Reconnecting>),
    GracePeriod(Box<GracePeriod>),
}

impl States {
    pub fn into_boxed_inner(self) -> Box<dyn PinnedState> {
        match self {
            Self::Disconnected(disconnected) => disconnected,
            Self::Connecting(connecting) => connecting,
            Self::EpochMismatch(epoch_mismatch) => epoch_mismatch,
            Self::Connected(connected) => connected,
            Self::Reconnecting(reconnecting) => reconnecting,
            Self::GracePeriod(grace_period) => grace_period,
        }
    }

    pub fn get_name(&self) -> &'static str {
        match self {
            Self::Disconnected(_) => "Disconnected",
            Self::Connecting(_) => "Connecting",
            Self::EpochMismatch(_) => "EpochMismatch",
            Self::Connected(_) => "Connected",
            Self::Reconnecting(_) => "Reconnecting",
            Self::GracePeriod(_) => "GracePeriod",
        }
    }
}

impl States {
    pub(crate) fn new() -> Self {
        Self::Disconnected(Box::new(Disconnected::new()))
    }
}

impl From<Box<Disconnected>> for States {
    fn from(value: Box<Disconnected>) -> Self {
        Self::Disconnected(value)
    }
}

impl From<Box<Connecting>> for States {
    fn from(value: Box<Connecting>) -> Self {
        Self::Connecting(value)
    }
}

impl From<Box<EpochMismatch>> for States {
    fn from(value: Box<EpochMismatch>) -> Self {
        Self::EpochMismatch(value)
    }
}

impl From<Box<Connected>> for States {
    fn from(value: Box<Connected>) -> Self {
        Self::Connected(value)
    }
}

impl From<Box<Reconnecting>> for States {
    fn from(value: Box<Reconnecting>) -> Self {
        Self::Reconnecting(value)
    }
}

impl From<Box<GracePeriod>> for States {
    fn from(value: Box<GracePeriod>) -> Self {
        Self::GracePeriod(value)
    }
}
