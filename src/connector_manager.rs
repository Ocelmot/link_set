use std::fmt::Debug;
use std::time::Duration;

use tokio::select;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::trace;

use crate::link_set::controller::{LinkSetControl, LinkSetControlCommand};
use crate::links::connector::PinnedLinkConnector;
use crate::{LinkError, LinkResult};

enum ConnectorManagerControl {
    AddAddr(String),
    TryAddr(String),
    AddConnector(Box<dyn PinnedLinkConnector>),
}

pub(crate) struct ConnectorManager {
    tx: Sender<ConnectorManagerControl>,
    stop_tx: oneshot::Sender<()>,
    handle: JoinHandle<(Vec<Box<dyn PinnedLinkConnector>>, Vec<(String, bool)>)>,
}

impl ConnectorManager {
    pub fn start(
        mut to_core: Sender<LinkSetControl>,
        // addrs is a list of addresses, and a flag to indicate if they are permanent
        mut addrs: Vec<(String, bool)>,
        // conns is a list of connectors that can be used
        mut conns: Vec<Box<dyn PinnedLinkConnector>>,
    ) -> Self {
        let (tx, mut rx) = channel(10);
        let (stop_tx, stop_rx) = oneshot::channel();

        let handle = tokio::task::spawn(async move {
            let mut addr_index = 0;
            let mut conn_index = 0;
            trace!("Connector started");
            select! {
                _ = async {
                    loop {
                        // process rx
                        process_rx(&mut rx, &mut conns, &mut addrs).await;

                        // process for each addr, each conn
                        process_connecting(
                            &mut to_core,
                            &mut conns,
                            &mut addrs,
                            &mut conn_index,
                            &mut addr_index,
                        )
                        .await;
                    }
                } => {},
                _ = stop_rx => {

                }
            }
            return (conns, addrs);
        });

        Self {
            tx,
            stop_tx,
            handle,
        }
    }

    pub async fn add_addr(&self, addr: String) {
        let _ = self.tx.send(ConnectorManagerControl::AddAddr(addr)).await;
    }

    pub async fn try_addr(&self, addr: String) {
        let _ = self.tx.send(ConnectorManagerControl::TryAddr(addr)).await;
    }

    pub async fn add_connector(&self, conn: Box<dyn PinnedLinkConnector>) {
        let _ = self
            .tx
            .send(ConnectorManagerControl::AddConnector(conn))
            .await;
    }

    pub async fn cancel(
        self,
    ) -> LinkResult<(Vec<Box<dyn PinnedLinkConnector>>, Vec<(String, bool)>)> {
        drop(self.tx);
        let _ = self.stop_tx.send(());
        self.handle
            .await
            .map_err(|e| LinkError::TaskTerminated(Box::new(e)))
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
    conns: &mut Vec<Box<dyn PinnedLinkConnector>>,
    addrs: &mut Vec<(String, bool)>,
) {
    loop {
        let ctrl = if addrs.len() == 0 || conns.len() == 0 {
            // wait for rx to have a message and start over
            trace!(
                "Connector waiting for message, addrs: {}, conns: {}",
                addrs.len(),
                conns.len()
            );
            match rx.recv().await {
                Some(ctrl) => Some(ctrl),
                None => return,
            }
        } else {
            trace!(
                "Connector checking for message, addrs: {}, conns: {}",
                addrs.len(),
                conns.len()
            );
            match rx.try_recv() {
                Ok(ctrl) => Some(ctrl),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        };

        trace!("Got control message: {:?}", ctrl);
        match ctrl {
            Some(ConnectorManagerControl::AddAddr(add_addr)) => {
                for (addr, _) in &mut *addrs {
                    if *addr == add_addr {
                        // skip addrs we already have in the list
                        continue;
                    }
                }
                addrs.push((add_addr, true))
            }
            Some(ConnectorManagerControl::TryAddr(try_addr)) => {
                for (addr, _) in &mut *addrs {
                    if *addr == try_addr {
                        // skip addrs we already have in the list
                        continue;
                    }
                }
                addrs.push((try_addr, false))
            }
            Some(ConnectorManagerControl::AddConnector(conn)) => conns.push(conn),
            None => break,
        }
    }
}

async fn process_connecting(
    to_core: &mut Sender<LinkSetControl>,
    conns: &mut Vec<Box<dyn PinnedLinkConnector>>,
    addrs: &mut Vec<(String, bool)>,
    conn_index: &mut usize,
    addr_index: &mut usize,
) {
    if let Some((addr, retain_addr)) = addrs.get(*addr_index) {
        if let Some(conn) = conns.get_mut(*conn_index) {
            trace!("Connector attempting to connect to {addr} with connector index {conn_index}");
            let res = conn.connect(addr.clone()).await;
            match res {
                Ok(link) => {
                    trace!("Connector made connection, adding link");
                    let _ = to_core
                        .send(LinkSetControl::Command(LinkSetControlCommand::AddLink(
                            link,
                        )))
                        .await;
                }
                Err(e) => {
                    // if there is an error, we will just try again later or with another connector
                    trace!("Connector did not make a connection: {}", e);
                }
            }
            *conn_index += 1;
        } else {
            *conn_index = 0;
            if *retain_addr {
                *addr_index += 1;
            } else {
                addrs.remove(*addr_index);
            }
        }
    } else {
        trace!("Address/Connector list finished, sleeping");
        *addr_index = 0;
        sleep(Duration::from_secs(5)).await;
    }
}
