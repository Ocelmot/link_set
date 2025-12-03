use futures::StreamExt;
use rand::random;
use tokio::{
    select,
    sync::mpsc::{Receiver, Sender, channel},
};
use tracing::{Instrument, info, trace, trace_span};

use crate::{
    LinkSetResult,
    link_set::controller::{LinkSetControl, LinkSetMessageInner},
    state::state::CoreState,
};

pub(crate) fn start_core() -> (Sender<LinkSetControl>, Receiver<LinkSetMessageInner>) {
    trace!("LinkSetCore starting");
    let (to_core, mut from_ctrl) = channel(10);
    let (to_ctrl, from_core) = channel(10);

    // init
    let span = trace_span!("LinkSet", ID = random::<u16>());
    let mut core = CoreState::new(to_core.clone(),to_ctrl);

    // start loop
    tokio::spawn(
        async move {
            loop {
                trace!("LinkSetCore processing");
                trace!("Readers: {}", core.get_readers().len());

                let old_state_name = core.get_state_name();
                let (readers, timer) = core.get_refs();
                select! {
                    // get control messages
                    ctrl_msg = from_ctrl.recv() => {
                        if let Some(ctrl_msg) = ctrl_msg {
                            core = core.ctrl_msg(ctrl_msg).await;
                        }else {
                            // The handle has been dropped, quit
                            // If the state needs a shutdown chance, do that here
                            return Ok(());
                        }
                    },

                    // get link messages
                    link_msg = readers.next(), if !readers.is_empty() => {
                        if let Some((id, msg)) = link_msg {
                            core = core.link_msg(id, msg).await;
                        }else{
                            // Although there are no readers, more connections
                            // could be established later. Ignore
                            info!("readers got None");
                        }
                    },

                    // respond to timer
                    _ = timer, if timer.has_deadline() => {
                        core = core.timer().await;
                    }

                }

                if old_state_name != core.get_state_name() {
                    trace!("Transitioning from {:?} to {:?}", old_state_name, core.get_state_name());
                }
                
            }

            #[allow(unreachable_code)]
            LinkSetResult::Ok(())
        }
        .instrument(span),
    );

    (to_core, from_core)
}
