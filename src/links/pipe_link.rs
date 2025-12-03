use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
    u32,
};

use rand::{distributions::Bernoulli, prelude::Distribution, thread_rng};
use thiserror::Error;
use tokio::{
    select,
    sync::mpsc::{Receiver, Sender, channel, error::TrySendError},
    time::{Instant, sleep_until},
};
use tracing::{debug, trace};

use crate::{
    deadline::Deadline,
    links::link::{Link, LinkReader},
    protocol::LinkProtocol,
};

/// A PipeLinkBuilder sets the configuration for a pair of [PipeLink]s.
///
/// This allows a pair of connected PipeLinks to be created with additional
/// settings that may be useful for testing. E.g. the link could expire and
/// close after a certain period of time.
#[derive(Debug, Clone, Copy)]
pub struct PipeLinkBuilder {
    max_size: u32,
    reliability: f64,
    expiration: Option<Duration>,
    latency: Option<Duration>,
}

impl PipeLinkBuilder {
    /// Create a new PipeLinkBuilder with the default characteristics
    pub fn new() -> Self {
        Self {
            max_size: u32::MAX,
            reliability: 1.0,
            expiration: None,
            latency: None,
        }
    }

    /// Constrains the max size of a slice that can be sent through the PipeLink
    pub fn max_size(mut self, max_size: u32) -> Self {
        self.max_size = max_size;
        self
    }

    /// Causes the PipeLink to send items sent through it with the given
    /// probability. Probability is [0.0, 1.0], 1.0 indicates the message will
    /// always be sent.
    pub fn reliability(mut self, reliability: f64) -> Self {
        self.reliability = reliability;
        self
    }

    /// Causes the PipeLink to send items sent through it with the given
    /// probability. Probability is [0.0, 1.0], 1.0 indicates the message will
    /// always be sent.
    pub fn set_reliability(&mut self, reliability: f64) {
        self.reliability = reliability;
    }

    /// Causes the PipeLink to expire after a certain period of time.
    pub fn expiration(mut self, expiration: Option<Duration>) -> Self {
        self.expiration = expiration;
        self
    }

    /// Causes the PipeLink to add the given latency to each sent message
    pub fn latency(mut self, latency: Option<Duration>) -> Self {
        self.latency = latency;
        self
    }

    /// Create a linked pair of PipeLinks with the builder's current
    /// configuration.
    pub fn create_pair(&self) -> (PipeLink, PipeLink) {
        let (tx1, rx1) = channel(1000);
        let (tx2, rx2) = channel(1000);
        let (canceler, c1, c2) = PipeLinkCanceler::new();

        let expiration = if let Some(expiration) = self.expiration {
            Deadline::new_deadline(Instant::now() + expiration)
        } else {
            Deadline::new()
        };

        let id = rand::random();
        let first = PipeLink {
            id,
            tx: tx1,
            reader: Some(PipeLinkReader {
                id,
                rx: rx2,
                peeked: None,
                expiration,
                latency: self.latency,
                cancel_rx: c1,
            }),
            max_size: self.max_size,
            canceler: canceler.clone(),
            reliability: Bernoulli::new(self.reliability).unwrap(),
        };

        let expiration = if let Some(expiration) = self.expiration {
            Deadline::new_deadline(Instant::now() + expiration)
        } else {
            Deadline::new()
        };

        let id = rand::random();
        let second = PipeLink {
            id,
            tx: tx2,
            reader: Some(PipeLinkReader {
                id,
                rx: rx1,
                peeked: None,
                expiration,
                latency: self.latency,
                cancel_rx: c2,
            }),
            max_size: self.max_size,
            canceler,
            reliability: Bernoulli::new(self.reliability).unwrap(),
        };

        (first, second)
    }
}

/// Pipe link is an implementation of [Link] backed by a pair of mpsc channels.
/// This is primarily useful for testing the rest of the implementation.
pub struct PipeLink {
    id: u32,
    tx: Sender<(LinkProtocol, Instant)>,
    reader: Option<PipeLinkReader>,
    max_size: u32,
    canceler: PipeLinkCanceler,
    reliability: Bernoulli,
}

impl PipeLink {
    /// Create a pair of PipeLinks that are linked to each other.
    pub fn create_pair() -> (PipeLink, PipeLink) {
        let (tx1, rx1) = channel(1000);
        let (tx2, rx2) = channel(1000);
        let (canceler, c1, c2) = PipeLinkCanceler::new();

        let id = rand::random();
        let first = PipeLink {
            id,
            tx: tx1,
            reader: Some(PipeLinkReader {
                id,
                rx: rx2,
                peeked: None,
                expiration: Deadline::new(),
                latency: None,
                cancel_rx: c1,
            }),
            max_size: u32::MAX,
            canceler: canceler.clone(),
            reliability: Bernoulli::new(1.0).unwrap(),
        };

        let id = rand::random();
        let second = PipeLink {
            id,
            tx: tx2,
            reader: Some(PipeLinkReader {
                id,
                rx: rx1,
                peeked: None,
                expiration: Deadline::new(),
                latency: None,
                cancel_rx: c2,
            }),
            max_size: u32::MAX,
            canceler,
            reliability: Bernoulli::new(1.0).unwrap(),
        };

        (first, second)
    }

    /// Get the randomly assigned id for this pipe end
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn get_canceler(&self) -> PipeLinkCanceler {
        self.canceler.clone()
    }
}

pub type PipeLinkResult<T=()> =  Result<T, PipeLinkError>;

#[derive(Debug, Error)]
pub enum PipeLinkError {
    #[error("The receiver hs already been taken")]
    ReceiverTaken,

    #[error("The link has closed")]
    Closed
}

impl Link for PipeLink {
    #[allow(refining_impl_trait)]
    async fn send(&mut self, msg: LinkProtocol) -> Result<(), PipeLinkError> {
        let reliability_success = {
            let mut rng = thread_rng();
            self.reliability.sample(&mut rng)
        };

        if reliability_success {
            debug!("Pipe {}: send msg {:?}", self.id, msg);
            self.tx
                .send((msg, Instant::now()))
                .await
                .map_err(|_| PipeLinkError::Closed)?;
        } else {
            debug!("Pipe {}: send (dropped) msg {:?}", self.id, msg);
        }
        Ok(())
    }

    #[allow(refining_impl_trait)]
    async fn recv(&mut self) -> PipeLinkResult<LinkProtocol> {
        let reader = self.reader.as_mut().ok_or(PipeLinkError::ReceiverTaken)?;
        reader.read().await
    }

    fn take_reader(&mut self) -> Result<impl LinkReader + 'static, impl std::error::Error + std::marker::Send + Sync + 'static> {
        self.reader.take().ok_or(PipeLinkError::Closed)
    }

    fn max_size(&self) -> u32 {
        self.max_size
    }

    fn is_closed(&mut self) -> bool {
        self.tx.is_closed()
    }
}

#[derive(Debug, Clone)]
pub struct PipeLinkCanceler {
    a: Sender<()>,
    b: Sender<()>,
}

impl PipeLinkCanceler {
    pub(crate) fn new() -> (Self, Receiver<()>, Receiver<()>) {
        let (a_tx, a_rx) = channel(1);
        let (b_tx, b_rx) = channel(1);

        (Self { a: a_tx, b: b_tx }, a_rx, b_rx)
    }

    pub fn cancel(self) {
        let a = self.a.try_send(());
        let b = self.b.try_send(());
        trace!("a = {:?}, b = {:?}", a, b);
    }
}

pub struct PipeLinkReader {
    id: u32,
    rx: Receiver<(LinkProtocol, Instant)>,
    peeked: Option<(LinkProtocol, Instant)>,
    expiration: Deadline,
    latency: Option<Duration>,
    cancel_rx: Receiver<()>,
}
impl LinkReader for PipeLinkReader {
    #[allow(refining_impl_trait)]
    async fn read(&mut self) -> PipeLinkResult<LinkProtocol> {
        if let Some((msg, timestamp)) = &self.peeked {
            if let Some(latency) = self.latency {
                sleep_until(*timestamp + latency).await;
            }
            let msg = msg.clone();
            self.peeked = None;
            if self.expiration.is_elapsed() {
                debug!("Pipe {}: expired", self.id);
                self.rx.close();
                return Err(PipeLinkError::Closed);
            } else {
                debug!("Pipe {}: read msg {:?}", self.id, msg);
                return Ok(msg);
            }
        }

        select! {
            biased;
            _ = self.cancel_rx.recv() => {
                debug!("Pipe {}: canceled", self.id);
                self.rx.close();
                return Err(PipeLinkError::Closed)
            }
            _ = &mut self.expiration, if self.expiration.has_deadline() => {
                debug!("Pipe {}: expired", self.id);
                self.rx.close();
                return Err(PipeLinkError::Closed)
            }
            msg = self.rx.recv() => {
                let (msg, timestamp) = msg.ok_or(PipeLinkError::Closed)?;
                self.peeked = Some((msg.clone(), timestamp));
                if let Some(latency) = self.latency {
                    sleep_until(timestamp + latency).await;
                }
                self.peeked = None;
                debug!("Pipe {}: read msg {:?}", self.id, msg);
                return Ok(msg);
            },
        }
    }
}

/// Serves to simulate the listen/connect actions when establishing a connection
/// between [PipeLink]s.
///
/// This can be used to set up a fake network for testing or simulation
/// purposes.
#[derive(Debug, Clone)]
pub struct PipeLinkHub {
    link_builder: Arc<Mutex<PipeLinkBuilder>>,
    listen_channel_size: usize,
    listeners: Arc<Mutex<HashMap<String, Sender<PipeLink>>>>,
}

impl PipeLinkHub {
    /// Create a new PipeLinkHub.
    ///
    /// The connections it returns will be generated with the given
    /// [PipeLinkBuilder] which can be given options to make the links timeout,
    /// drop messages, etc.
    pub fn new(link_builder: PipeLinkBuilder) -> Self {
        Self {
            link_builder: Arc::new(Mutex::new(link_builder)),
            listen_channel_size: 25,
            listeners: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn new_with_size(link_builder: PipeLinkBuilder, listen_channel_size: usize) -> Self {
        Self {
            link_builder: Arc::new(Mutex::new(link_builder)),
            listen_channel_size,
            listeners: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Returns a reference to the PipeLinkBuilder that is used to create new
    /// links. Can be used to change the properties of emitted [PipeLink]s
    pub fn modify_builder(
        &mut self,
        func: impl FnOnce(&mut MutexGuard<PipeLinkBuilder>) -> PipeLinkBuilder,
    ) {
        if let Ok(mut guard) = self.link_builder.lock() {
            let new_val = func(&mut guard);
            *guard = new_val;
        }
    }

    /// Returns a [Receiver<PipeLink>] that will "listen" at the given address.
    /// Connections to that address will cause a new PipeLink to be created and
    /// sent through the receiver.
    pub fn listen(&mut self, addr: String) -> Receiver<PipeLink> {
        let (tx, rx) = channel(self.listen_channel_size);
        // if the lock could not be acquired,the listener will close immediately
        if let Ok(mut listeners) = self.listeners.lock() {
            listeners.insert(addr, tx);
        }
        rx
    }

    /// Creates a connection to a listening address if there is one.
    pub fn connect(&mut self, dest_addr: &str) -> Option<PipeLink> {
        let mut listeners = self.listeners.lock().ok()?;
        let listener = listeners.get(dest_addr)?;

        let mut should_remove = false;
        let result = match listener.try_reserve() {
            Ok(permit) => {
                let (link1, link2) = {
                    let builder = self.link_builder.lock().ok()?;
                    builder.create_pair()
                };
                permit.send(link2);
                Some(link1)
            }
            Err(TrySendError::Closed(_)) => {
                should_remove = true;
                None
            }
            Err(TrySendError::Full(_)) => None,
        };

        if should_remove {
            listeners.remove(dest_addr);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::sleep;
    use tracing::trace;

    use crate::{LinkSetError, links::link::PinnedLink};

    use super::*;

    // Pipe link tests

    #[tokio::test]
    #[test_log::test]
    async fn pipe_links_send_recv() {
        let (mut a, mut b) = PipeLink::create_pair();
        let msg = LinkProtocol::Ping;
        PinnedLink::send(&mut a, msg.clone()).await.unwrap();

        let recvd_msg = PinnedLink::recv(&mut b).await.unwrap();
        assert_eq!(recvd_msg, msg, "Message was not the same when received");
    }

    #[tokio::test]
    #[test_log::test]
    async fn pipe_links_send_recv_canceled() {
        let (mut a, mut b) = PipeLink::create_pair();
        // cancel the link
        a.get_canceler().cancel();

        // send msg
        let msg = LinkProtocol::Ping;
        PinnedLink::send(&mut a, msg.clone()).await.unwrap();

        // test that it was not received
        let recvd_msg = PinnedLink::recv(&mut b).await;
        if let Err(LinkSetError::Terminated) = recvd_msg {
        } else {
            panic!("Link should expire");
        }
    }

    #[tokio::test]
    #[test_log::test]
    async fn pipe_links_taken_send_recv_canceled() {
        let (mut a, mut b) = PipeLink::create_pair();

        // cancel the link
        trace!("canceling link");
        a.get_canceler().cancel();

        trace!("taking reader");
        let mut reader = PinnedLink::take_reader(&mut b).unwrap();

        // send msg
        trace!("sending msg");
        let msg = LinkProtocol::Ping;
        let _ = PinnedLink::send(&mut a, msg.clone()).await;

        // test that it was not received
        trace!("reading");
        let recvd_msg = reader.read().await;
        if let Err(_) = recvd_msg {
        } else {
            panic!("Link should expire");
        }
    }

    #[tokio::test]
    #[test_log::test]
    async fn pipe_links_send_recv_reliability() {
        let reliability = 0.5;
        let (mut a, mut b) = PipeLinkBuilder::new()
            .reliability(reliability)
            .create_pair();

        // Send 100 msgs, then close the link
        let send_count = 1000;
        for _ in 0..send_count {
            let msg = LinkProtocol::Ping;
            PinnedLink::send(&mut a, msg)
                .await
                .expect("should be able to send");
        }
        drop(a);

        let mut recv_count = 0;
        while let Ok(_) = PinnedLink::recv(&mut b).await {
            recv_count += 1;
        }
        trace!("Sent: {send_count}, recvd: {recv_count}");
        let recv_ratio = recv_count as f64 / send_count as f64;

        assert!(
            (reliability - recv_ratio).abs() < 0.10,
            "reliability({recv_ratio}) too far from target ({reliability})"
        );
    }

    #[tokio::test]
    #[test_log::test]
    async fn pipe_links_send_recv_not_expired() {
        let (mut a, mut b) = PipeLinkBuilder::new()
            .expiration(Some(Duration::from_secs(2)))
            .create_pair();

        sleep(Duration::from_secs(1)).await; // wait for the links to expire
        let msg = LinkProtocol::Ping;
        PinnedLink::send(&mut a, msg.clone())
            .await
            .expect("should be able to send");

        let recvd_msg = PinnedLink::recv(&mut b)
            .await
            .expect("Link should not expire");
        assert_eq!(recvd_msg, msg, "Message was not the same when received");
    }

    #[tokio::test]
    #[test_log::test]
    async fn pipe_links_send_recv_expired() {
        let (mut a, mut b) = PipeLinkBuilder::new()
            .expiration(Some(Duration::from_secs(1)))
            .create_pair();

        sleep(Duration::from_secs(2)).await; // wait for the links to expire
        let msg = LinkProtocol::Ping;
        PinnedLink::send(&mut a, msg.clone())
            .await
            .expect("should be able to send");

        let recvd_msg = PinnedLink::recv(&mut b).await;
        if let Err(LinkSetError::Terminated) = recvd_msg {
        } else {
            panic!("Link should expire");
        }
    }

    #[tokio::test]
    #[test_log::test]
    async fn pipe_links_send_recv_latency() {
        let latency_ms = 2000;
        let (mut a, mut b) = PipeLinkBuilder::new()
            .latency(Some(Duration::from_millis(latency_ms)))
            .create_pair();

        let msg = LinkProtocol::Ping;
        PinnedLink::send(&mut a, msg.clone())
            .await
            .expect("should be able to send");
        // start timer
        let start = Instant::now();

        let recvd_msg = PinnedLink::recv(&mut b)
            .await
            .expect("Link should not expire");
        assert_eq!(recvd_msg, msg, "Message was not the same when received");

        let stop = start.elapsed();
        // make sure that expire is approx. the latency time
        assert!(
            stop > Duration::from_millis(latency_ms - 100),
            "Received too soon"
        );
        assert!(
            stop < Duration::from_millis(latency_ms + 100),
            "Received too late"
        );
    }

    // Pipe link hub tests
    #[tokio::test]
    #[test_log::test]
    async fn pipe_link_hub_send_recv() {
        let builder = PipeLinkBuilder::new();
        let mut hub = PipeLinkHub::new(builder);

        let addr = String::from("addr1");
        let mut listener = hub.listen(addr.clone());

        let mut b = hub.connect(&addr).expect("connection should establish");
        let mut a = listener
            .recv()
            .await
            .expect("listener should return connection");

        let msg = LinkProtocol::Ping;
        PinnedLink::send(&mut a, msg.clone()).await.unwrap();

        let recvd_msg = PinnedLink::recv(&mut b).await.unwrap();
        assert_eq!(recvd_msg, msg, "Message was not the same when received");
    }

    #[tokio::test]
    #[test_log::test]
    async fn pipe_link_hub_send_recv_two_links_one_addr() {
        let builder = PipeLinkBuilder::new();
        let mut hub = PipeLinkHub::new(builder);

        let addr1 = String::from("addr1");
        let mut listener = hub.listen(addr1.clone());

        let mut b = hub.connect(&addr1).expect("connection should establish");
        let mut a = listener
            .recv()
            .await
            .expect("listener should return connection");

        let mut c = hub.connect(&addr1).expect("connection should establish");
        let mut d = listener
            .recv()
            .await
            .expect("listener should return connection");

        // test that each of the links connects correctly
        let msg = LinkProtocol::Ping;
        PinnedLink::send(&mut a, msg.clone()).await.unwrap();

        let recvd_msg = PinnedLink::recv(&mut b).await.unwrap();
        assert_eq!(recvd_msg, msg, "Message was not the same when received");

        let msg2 = LinkProtocol::Pong;
        PinnedLink::send(&mut c, msg2.clone()).await.unwrap();

        let recvd_msg2 = PinnedLink::recv(&mut d).await.unwrap();
        assert_eq!(recvd_msg2, msg2, "Message 2 was not the same when received");
    }

    #[tokio::test]
    #[test_log::test]
    async fn pipe_link_hub_send_recv_two_links_two_addr() {
        let builder = PipeLinkBuilder::new();
        let mut hub = PipeLinkHub::new(builder);

        let addr1 = String::from("addr1");
        let mut listener1 = hub.listen(addr1.clone());
        let addr2 = String::from("addr2");
        let mut listener2 = hub.listen(addr2.clone());

        let mut b = hub.connect(&addr1).expect("connection should establish");
        let mut a = listener1
            .recv()
            .await
            .expect("listener should return connection");

        let mut c = hub.connect(&addr2).expect("connection should establish");
        let mut d = listener2
            .recv()
            .await
            .expect("listener should return connection");

        // test that each of the links connects correctly
        let msg = LinkProtocol::Ping;
        PinnedLink::send(&mut a, msg.clone()).await.unwrap();

        let recvd_msg = PinnedLink::recv(&mut b).await.unwrap();
        assert_eq!(recvd_msg, msg, "Message was not the same when received");

        let msg2 = LinkProtocol::Pong;
        PinnedLink::send(&mut c, msg2.clone()).await.unwrap();

        let recvd_msg2 = PinnedLink::recv(&mut d).await.unwrap();
        assert_eq!(recvd_msg2, msg2, "Message 2 was not the same when received");
    }
}
