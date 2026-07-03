use crate::state::{common::CommonState, states::States};

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
