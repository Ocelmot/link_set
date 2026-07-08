use std::collections::{HashMap, HashSet};

use std::hash::Hash;

use std::mem;
use std::sync::Arc;
use std::time::Duration;

use tokio::select;
use tokio::sync::mpsc::Sender;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::debug;

use crate::debug::ConnectionManagerDebugReplySnapshot;
use crate::link_set::controller::LinkSetControl;
use crate::link_set::controller::LinkSetControlCommand::AddLink;
use crate::links::Address;
use crate::links::connector::PinnedLinkConnector;

pub(crate) const RETRIES_MAX: u16 = 10;

pub(crate) type AddressSet = HashSet<ConnectorManagerAddress>;
pub(crate) type ConnectorSet = HashMap<String, Arc<dyn PinnedLinkConnector + 'static>>;

pub(crate) struct ConnectorManagerAddress {
    addr: Address,
    repeat: watch::Sender<bool>,
    count: Arc<Semaphore>,
}

impl ConnectorManagerAddress {
    pub(crate) fn new(addr: Address) -> Self {
        Self {
            addr,
            repeat: watch::Sender::new(false),
            count: Arc::new(Semaphore::new(0)),
        }
    }

    pub(crate) fn addr(&self) -> &Address {
        &self.addr
    }

    pub(crate) fn subscribe_repeat(&self) -> watch::Receiver<bool> {
        self.repeat.subscribe()
    }

    pub(crate) fn semaphore(&self) -> Arc<Semaphore> {
        self.count.clone()
    }

    pub(crate) fn add_count(&self, count: u16) {
        let existing = self.count.available_permits();
        let room = (RETRIES_MAX as usize).saturating_sub(existing);
        self.count.add_permits(std::cmp::min(count as usize, room));
    }

    pub(crate) fn repeat(&self) {
        self.set_repeat(true);
    }

    pub(crate) fn merge(&self, other: Self) {
        other.count.close();
        let other_repeat = other.repeat.send_replace(false);

        let permits = other.count.available_permits();
        self.add_count(usize::min(permits, u16::MAX as usize) as u16);
        if other_repeat {
            self.set_repeat(true);
        }
    }

    fn set_repeat(&self, value: bool) {
        self.repeat.send_if_modified(|v| {
            if value != *v {
                *v = value;
                true
            } else {
                false
            }
        });
    }
}

impl PartialEq for ConnectorManagerAddress {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl Eq for ConnectorManagerAddress {}

impl Hash for ConnectorManagerAddress {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.addr.hash(state);
    }
}

impl From<Address> for ConnectorManagerAddress {
    fn from(value: Address) -> Self {
        ConnectorManagerAddress::new(value)
    }
}

pub(crate) struct ConnectorManager {
    tx: Sender<LinkSetControl>,
    conns: ConnectorSet,
    handles: HashMap<ConnectorManagerAddress, Option<JoinHandle<()>>>,
}

impl ConnectorManager {
    pub fn start(
        tx: Sender<LinkSetControl>,
        // addrs is a list of addresses containing the count of usages
        addrs: AddressSet,
        // conns is a list of connectors that can be used
        conns: ConnectorSet,
    ) -> Self {
        let mut handles = HashMap::new();
        for addr in addrs {
            let handle = conns
                .get(addr.addr().scheme())
                .map(|conn| create_task(tx.clone(), &addr, &conn));

            handles.insert(addr, handle);
        }

        Self { tx, conns, handles }
    }

    pub async fn add_addr(&mut self, addr: ConnectorManagerAddress) {
        if self.handles.contains_key(&addr) {
            let (k, _v) = self
                .handles
                .get_key_value(&addr)
                .expect("handles contains_key()");
            k.merge(addr);
        } else {
            let handle = self
                .conns
                .get(addr.addr().scheme())
                .map(|conn| create_task(self.tx.clone(), &addr, &conn));

            self.handles.insert(addr, handle);
        }
    }

    pub async fn add_connector(&mut self, conn: Arc<dyn PinnedLinkConnector>) {
        if self.conns.contains_key(conn.scheme()) {
            // Replace the connector in active tasks
            // First cancel all connectors
            let iter = self
                .handles
                .iter()
                .filter(|(addr, _)| addr.addr.scheme() == conn.scheme());
            for (_, handle) in iter {
                if let Some(handle) = handle {
                    handle.abort();
                }
            }

            // for each connector, await it
            for (addr, handle) in self
                .handles
                .iter_mut()
                .filter(|(addr, _)| addr.addr.scheme() == conn.scheme())
            {
                if let Some(handle) = handle.take() {
                    let _ = handle.await;
                }

                // Now there are no tasks for this scheme, it is safe to remake
                // them with the new connector
                let new_handle = create_task(self.tx.clone(), &addr, &conn);
                *handle = Some(new_handle);
            }
        } else {
            // new connector, start new tasks
            let iter = self
                .handles
                .iter_mut()
                .filter(|(addr, _)| addr.addr.scheme() == conn.scheme());

            for (addr, task) in iter {
                debug_assert!(task.is_none());
                *task = Some(create_task(self.tx.clone(), addr, &conn));
            }
        }
        self.conns.insert(conn.scheme().to_owned(), conn);
    }

    pub fn debug_request_snapshot(&self) -> ConnectionManagerDebugReplySnapshot {
        let addrs = self
            .handles
            .iter()
            .map(|(addr, _)| addr.addr.clone())
            .collect();
        let connectors = self.conns.iter().map(|(_, c)| c.scheme()).collect();
        ConnectionManagerDebugReplySnapshot { addrs, connectors }
    }

    pub async fn cancel(
        mut self,
    ) -> (
        ConnectorSet,
        AddressSet,
    ) {
        // First cancel all connectors
        for (_, handle) in &self.handles {
            if let Some(handle) = handle {
                handle.abort();
            }
        }

        // Collect the addrs
        let mut addrs = HashSet::new();

        let handles = mem::take(&mut self.handles);
        for (addr, mut handle) in handles.into_iter() {
            if let Some(handle) = handle.take() {
                let _ = handle.await;
            }

            addrs.insert(addr);
        }
        (mem::take(&mut self.conns), addrs)
    }
}

impl Drop for ConnectorManager {
    fn drop(&mut self) {
        // Normal use shouldn't have any entries to drop. They should be consumed
        // in cancel(). However, any task that gets through that will live
        // forever otherwise.
        for (_, handle) in &self.handles{
            if let Some(handle) = handle {
                handle.abort();
            }
        }
    }
}

fn create_task(
    tx: Sender<LinkSetControl>,
    addr: &ConnectorManagerAddress,
    conn: &Arc<dyn PinnedLinkConnector>,
) -> JoinHandle<()> {
    let address = addr.addr().addr().to_owned();
    let scheme = addr.addr().scheme().to_owned();
    let semaphore = addr.semaphore();
    let mut repeat = addr.subscribe_repeat();
    let connector = conn.clone();
    tokio::spawn(async move {
        loop {
            let permit = if *repeat.borrow() {
                None
            } else {
                select! {
                    res = semaphore.acquire() => match res {
                        Ok(permit) => Some(permit),
                        Err(_) => break, // semaphore closed
                    },
                    // When the repeat value changes, evaluate from the start
                    // where it will detect the change
                    res = repeat.changed() => match res {
                        Ok(_) => continue,
                        Err(_) => break, // sender dropped
                    }
                }
            };

            match connector.connect(address.clone()).await {
                Ok(link) => {
                    let result = tx.send(LinkSetControl::Command(AddLink(link))).await;
                    if result.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    debug!("Connector {scheme} failed to connect: {e}");
                }
            }

            if let Some(permit) = permit {
                permit.forget();
            }
            sleep(Duration::from_secs(5)).await;
        }
    })
}
