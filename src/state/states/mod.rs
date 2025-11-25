use crate::state::{
    state::{CommonState, PinnedCoreStateState},
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

// Create a new state from an old state
pub(crate) trait StateTransitionFrom<F> {
    fn transition_from(old_state: Box<F>, common: &mut CommonState) -> Box<Self>;
}

pub(crate) fn to_state<T, F>(from: Box<F>, common: &mut CommonState) -> States
where
    T: StateTransitionFrom<F>,
    Box<T>: Into<States>,
{
    T::transition_from(from, common).into()
}

// Create a new state from an old state (Async)
pub(crate) trait StateTransitionFromAsync<F> {
    fn transition_from(
        old_state: Box<F>,
        common: &mut CommonState,
    ) -> impl Future<Output = Box<Self>>;
}

pub(crate) async fn to_state_async<T, F>(from: Box<F>, common: &mut CommonState) -> States
where
    T: StateTransitionFromAsync<F>,
    Box<T>: Into<States>,
{
    T::transition_from(from, common).await.into()
}

// Create a new state from an old state and some parameters
pub(crate) trait StateTransitionWithParam<F, P> {
    fn transition_from(old_state: Box<F>, common: &mut CommonState, param: P) -> Box<Self>;
}

pub(crate) fn to_state_param<T, F, P>(from: Box<F>, common: &mut CommonState, param: P) -> States
where
    T: StateTransitionWithParam<F, P>,
    Box<T>: Into<States>,
{
    T::transition_from(from, common, param).into()
}

// Create a new state from an old state and some parameters (async)
pub(crate) trait StateTransitionWithParamAsync<F, P> {
    fn transition_from(
        old_state: Box<F>,
        common: &mut CommonState,
        param: P,
    ) -> impl Future<Output = Box<Self>>;
}

pub(crate) async fn to_state_param_async<T, F, P>(
    from: Box<F>,
    common: &mut CommonState,
    param: P,
) -> States
where
    T: StateTransitionWithParamAsync<F, P>,
    Box<T>: Into<States>,
{
    T::transition_from(from, common, param).await.into()
}

pub(crate) enum States {
    Disconnected(Box<Disconnected>),
    Connecting(Box<Connecting>),
    EpochMismatch(Box<EpochMismatch>),
    Connected(Box<Connected>),
    Reconnecting(Box<Reconnecting>),
    GracePeriod(Box<GracePeriod>),
}

impl States {
    pub fn to_boxed(self) -> Box<dyn PinnedCoreStateState> {
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
