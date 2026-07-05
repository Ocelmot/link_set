use futures::StreamExt;
use rand::random;
use tokio::{
    select,
    sync::mpsc::{Receiver, Sender, channel},
};
use tracing::{Instrument, info, trace, trace_span};

use crate::{
    LinkSetResult,
    debug::DebugCommand,
    link_set::controller::{LinkSetControl, LinkSetControlConfig, LinkSetMessageInner},
    protocol::LinkProtocol,
    state::{common::CommonState, states::States},
};

pub(crate) fn start_core(
    mut debug_channel: Option<Receiver<DebugCommand>>,
) -> (Sender<LinkSetControl>, Receiver<LinkSetMessageInner>) {
    trace!("LinkSetCore starting");
    let (to_core, mut from_ctrl) = channel(10);
    let (to_ctrl, from_core) = channel(10);

    // init
    let span = trace_span!("LinkSet", ID = random::<u16>());
    let mut common = CommonState::new(to_core.clone(), to_ctrl);
    let mut state = States::new();

    // start loop
    tokio::spawn(
        async move {
            loop {
                trace!("LinkSetCore processing");
                trace!("Readers: {}", common.get_readers_mut().len());

                let old_state_name = state.get_name();
                let (readers, timer) = common.get_await_items();
                select! {
                    // get control messages
                    ctrl_msg = from_ctrl.recv() => {
                        if let Some(ctrl_msg) = ctrl_msg {
                            state = handle_ctrl_msg(&mut common, state, ctrl_msg).await;
                        }else {
                            // The handle has been dropped, quit
                            // If the state needs a shutdown chance, do that here
                            return Ok(());
                        }
                    },

                    // get link messages
                    link_msg = readers.next(), if !readers.is_empty() => {
                        if let Some((id, msg)) = link_msg {
                            state = handle_link_msg(&mut common, state, id, msg).await;
                        }else{
                            // Although there are no readers, more connections
                            // could be established later. Ignore
                            info!("readers got None");
                        }
                    },

                    // respond to timer
                    _ = timer, if timer.has_deadline() => {
                        state = handle_timer(&mut common, state).await;
                    }

                    // Respond to debug message
                    cmd = recv_opt(&mut debug_channel) => {
                        state = handle_debug(&mut common, state, cmd).await;
                    }
                }

                if old_state_name != state.get_name() {
                    trace!(
                        "Transitioning from {:?} to {:?}",
                        old_state_name,
                        state.get_name()
                    );
                }
            }

            #[allow(unreachable_code)]
            LinkSetResult::Ok(())
        }
        .instrument(span),
    );

    (to_core, from_core)
}

/// Takes the current state and LinkSetControl message, dispatches it to the
/// state and returns the new state
async fn handle_ctrl_msg(common: &mut CommonState, state: States, msg: LinkSetControl) -> States {
    match msg {
        LinkSetControl::Command(cmd) => {
            let boxed = state.into_boxed_inner();
            boxed.ctrl_msg(common, cmd).await
        }
        LinkSetControl::Config(conf) => {
            match conf {
                LinkSetControlConfig::AutoConnect(connect) => {
                    common.set_auto_connect(connect);
                }
                LinkSetControlConfig::ConnectTimeout(timeout) => {
                    common.set_connecting_timeout(timeout);
                }
                LinkSetControlConfig::ReconnectTimeout(timeout) => {
                    common.set_reconnecting_timeout(timeout);
                }
                LinkSetControlConfig::GracePeriod(timeout) => {
                    common.set_grace_period_timeout(timeout);
                }
            }
            state
        }
    }
}

/// Takes the current state and a LinkProtocol and its id and passes it into the
/// current state. Returns the new state
async fn handle_link_msg(
    common: &mut CommonState,
    state: States,
    id: u64,
    msg: LinkProtocol,
) -> States {
    let boxed = state.into_boxed_inner();
    boxed.link_msg(common, id, msg).await
}

/// Takes the current state and calls its timer function. Returns the new state.
async fn handle_timer(common: &mut CommonState, state: States) -> States {
    let boxed = state.into_boxed_inner();
    boxed.timer(common).await
}

async fn handle_debug(common: &mut CommonState, state: States, cmd: DebugCommand) -> States {
    let boxed = state.into_boxed_inner();
    boxed.debug(common, cmd).await
}

async fn recv_opt(opt_receiver: &mut Option<Receiver<DebugCommand>>) -> DebugCommand {
    match opt_receiver {
        Some(receiver) => {
            match receiver.recv().await {
                Some(cmd) => cmd,
                None => {
                    *opt_receiver = None;
                    std::future::pending().await
                },
            }
        },
        None => std::future::pending().await,
    }

}
