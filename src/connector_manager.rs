use std::collections::HashMap;
use std::fmt::Debug;
use std::future::pending;
use std::ops::ControlFlow::{Break, Continue};
use std::time::Duration;

use futures::future::Either;
use futures::pin_mut;
use tokio::select;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{trace, warn};

use crate::debug::{ConnectionManagerDebugCommand, ConnectionManagerDebugReplySnapshot};
use crate::link_set::controller::{LinkSetControl, LinkSetControlCommand};
use crate::links::Address;
use crate::links::connector::PinnedLinkConnector;
use crate::{LinkSetError, LinkSetResult};

enum ConnectorManagerControl {
    AddAddr(Address),
    TryAddr(Address),
    AddConnector(Box<dyn PinnedLinkConnector>),
}

pub(crate) struct ConnectorManager {
    tx: Sender<ConnectorManagerControl>,
    stop_tx: oneshot::Sender<()>,
    debug_tx: Sender<ConnectionManagerDebugCommand>,
    handle: JoinHandle<(
        HashMap<String, Box<dyn PinnedLinkConnector>>,
        Vec<(Address, bool)>,
    )>,
}

impl ConnectorManager {
    pub fn start(
        mut to_core: Sender<LinkSetControl>,
        // addrs is a list of addresses, and a flag to indicate if they are permanent
        mut addrs: Vec<(Address, bool)>,
        // conns is a list of connectors that can be used
        mut conns: HashMap<String, Box<dyn PinnedLinkConnector>>,
    ) -> Self {
        let (tx, mut rx) = channel(10);
        let (stop_tx, stop_rx) = oneshot::channel();
        let (debug_tx, mut debug_rx) = channel(10);

        let handle = tokio::task::spawn(async move {
            let mut addr_index = 0;
            trace!("Connector started");
            select! {
                _ = async {
                    loop {
                        // process rx
                        process_rx(&mut rx, &mut conns, &mut addrs, &mut debug_rx).await;

                        // process for each addr, each conn
                        process_connecting(
                            &mut to_core,
                            &mut conns,
                            &mut addrs,
                            &mut addr_index,
                            &mut debug_rx,
                        )
                        .await;
                    }
                } => {},
                _ = stop_rx => {

                }
            }
            (conns, addrs)
        });

        Self {
            tx,
            stop_tx,
            debug_tx,
            handle,
        }
    }

    pub async fn add_addr(&self, addr: Address) {
        let _ = self.tx.send(ConnectorManagerControl::AddAddr(addr)).await;
    }

    pub async fn try_addr(&self, addr: Address) {
        let _ = self.tx.send(ConnectorManagerControl::TryAddr(addr)).await;
    }

    pub async fn add_connector(&self, conn: Box<dyn PinnedLinkConnector>) {
        let _ = self
            .tx
            .send(ConnectorManagerControl::AddConnector(conn))
            .await;
    }

    pub async fn debug_request_snapshot(
        &self,
    ) -> LinkSetResult<ConnectionManagerDebugReplySnapshot> {
        let (tx, rx) = oneshot::channel();
        let cmd = ConnectionManagerDebugCommand::Snapshot(tx);
        self.debug_tx
            .send(cmd)
            .await
            .map_err(|_| LinkSetError::Closed)?;
        rx.await.map_err(|_| LinkSetError::Closed)
    }

    pub async fn cancel(
        self,
    ) -> LinkSetResult<(
        HashMap<String, Box<dyn PinnedLinkConnector>>,
        Vec<(Address, bool)>,
    )> {
        drop(self.tx);
        let _ = self.stop_tx.send(());
        self.handle
            .await
            .map_err(|e| LinkSetError::TaskTerminated(Box::new(e)))
    }
}

impl Debug for ConnectorManagerControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddAddr(arg0) => f.debug_tuple("AddAddr").field(arg0).finish(),
            Self::TryAddr(arg0) => f.debug_tuple("TryAddr").field(arg0).finish(),
            Self::AddConnector(_) => f.debug_tuple("AddConnector").field(&"<Connector>").finish(),
        }
    }
}

async fn process_rx(
    rx: &mut Receiver<ConnectorManagerControl>,
    conns: &mut HashMap<String, Box<dyn PinnedLinkConnector>>,
    addrs: &mut Vec<(Address, bool)>,
    debug_rx: &mut Receiver<ConnectionManagerDebugCommand>,
) {
    loop {
        let flow = or_debug(conns, addrs, debug_rx, async {
            if addrs.len() == 0 || conns.len() == 0 {
                // wait for rx to have a message and start over
                trace!(
                    "Connector waiting for message, addrs: {}, conns: {}",
                    addrs.len(),
                    conns.len()
                );
                match rx.recv().await {
                    Some(ctrl) => Continue(Some(ctrl)),
                    None => Break(()),
                }
            } else {
                trace!(
                    "Connector checking for message, addrs: {}, conns: {}",
                    addrs.len(),
                    conns.len()
                );
                match rx.try_recv() {
                    Ok(ctrl) => Continue(Some(ctrl)),
                    Err(TryRecvError::Empty) => Continue(None),
                    Err(TryRecvError::Disconnected) => Break(()),
                }
            }
        })
        .await;
        let Continue(ctrl) = flow else {
            return;
        };

        trace!("Got control message: {:?}", ctrl);
        match ctrl {
            Some(ConnectorManagerControl::AddAddr(add_addr)) => {
                let mut contains = false;
                for (addr, _) in &mut *addrs {
                    if *addr == add_addr {
                        // skip addrs we already have in the list
                        contains = true;
                        break;
                    }
                }
                if !contains {
                    addrs.push((add_addr, true));
                }
            }
            Some(ConnectorManagerControl::TryAddr(try_addr)) => {
                let mut contains = false;
                for (addr, _) in &mut *addrs {
                    if *addr == try_addr {
                        // skip addrs we already have in the list
                        contains = true;
                        break;
                    }
                }
                if !contains {
                    addrs.push((try_addr, false));
                }
            }
            Some(ConnectorManagerControl::AddConnector(conn)) => {
                let scheme = conn.scheme();
                if conns.insert(scheme.to_string(), conn).is_some() {
                    warn!(
                        "Connector list already contained connector for scheme `{scheme}`! The connector has been replaced."
                    );
                }
            }
            None => break,
        };
    }
}

async fn process_connecting(
    to_core: &mut Sender<LinkSetControl>,
    conns: &mut HashMap<String, Box<dyn PinnedLinkConnector>>,
    addrs: &mut Vec<(Address, bool)>,
    addr_index: &mut usize,
    debug_rx: &mut Receiver<ConnectionManagerDebugCommand>,
) {
    if let Some((addr, retain_addr)) = addrs.get(*addr_index) {
        if let Some(conn) = conns.get(addr.scheme()) {
            let scheme = conn.scheme();
            trace!("Connector attempting to connect to {addr} with connector scheme {scheme}");
            let res = or_debug(
                conns,
                addrs,
                debug_rx,
                conn.connect(addr.addr().to_string()),
            )
            .await;
            match res {
                Ok(link) => {
                    trace!("Connector made connection, adding link");
                    let _ = or_debug(
                        conns,
                        addrs,
                        debug_rx,
                        to_core.send(LinkSetControl::Command(LinkSetControlCommand::AddLink(
                            link,
                        ))),
                    )
                    .await;
                }
                Err(e) => {
                    // if there is an error, we will just try again later or with another connector
                    trace!("Connector did not make a connection: {}", e);
                }
            }

            if *retain_addr {
                *addr_index += 1;
            } else {
                addrs.remove(*addr_index);
            }
        } else {
            *addr_index += 1;
        }
    } else {
        trace!("Address/Connector list finished, sleeping");
        *addr_index = 0;
        sleep(Duration::from_secs(5)).await;
    }
}

async fn or_debug<F, O>(
    conns: &HashMap<String, Box<dyn PinnedLinkConnector>>,
    addrs: &[(Address, bool)],
    debug_rx: &mut Receiver<ConnectionManagerDebugCommand>,
    fut: F,
) -> O
where
    F: Future<Output = O>,
{
    pin_mut!(fut);

    loop {
        let debug_fut = async {
            let res = debug_rx.recv().await;
            match res {
                Some(item) => item,
                None => pending().await,
            }
        };
        pin_mut!(debug_fut);

        let either = futures::future::select(debug_fut, fut).await;

        match either {
            Either::Left((cmd, partial_fut)) => {
                fut = partial_fut;
                match cmd {
                    ConnectionManagerDebugCommand::Snapshot(sender) => {
                        let snapshot = ConnectionManagerDebugReplySnapshot {
                            addrs: addrs.iter().map(|x| x.0.clone()).collect(),
                            connectors: conns.iter().map(|(_, v)| v.scheme()).collect(),
                        };
                        let _ = sender.send(snapshot);
                    }
                }
            }
            Either::Right((out, _)) => {
                break out;
            }
        }
    }
}
