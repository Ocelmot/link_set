use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;
use tracing::trace;

use crate::{LinkResult, links::link::PinnedLink, protocol::LinkProtocol};

/// The last N historical samples to keep when calculating latency
static HISTORY_LEN: usize = 6;
/// The latency to fill the buffer when creating a new link
static DEFAULT_LATENCY: Duration = Duration::from_millis(100);

pub(crate) static MAX_LATENCY: Duration = Duration::from_secs(2);

pub(crate) struct WrappedLink {
    /// Identify this link from the others
    id: u64,
    /// The wrapped link
    link: Box<dyn PinnedLink>,
    /// The time the last un-received ping was sent.
    ping: Option<Instant>,
    /// Queue of recent latencies
    recent: VecDeque<Duration>,
}

impl WrappedLink {
    pub fn new<L: Into<Box<dyn PinnedLink>>>(link: L, id: u64) -> Self {
        let link = link.into();
        let mut recent = VecDeque::with_capacity(HISTORY_LEN + 1);
        for _ in 0..HISTORY_LEN {
            recent.push_back(DEFAULT_LATENCY);
        }

        Self {
            id,
            link,
            ping: None,
            recent,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub async fn send_ping(&mut self) -> LinkResult {
        // end the previous ping if there was one.
        self.end_ping();
        self.ping = Some(Instant::now());
        self.link.send(LinkProtocol::Ping).await
    }

    pub async fn send_pong(&mut self) -> LinkResult {
        self.link.send(LinkProtocol::Pong).await
    }

    pub fn end_ping(&mut self) {
        if let Some(last_ping) = self.ping.take() {
            self.add_latency(last_ping.elapsed());
        }
    }

    fn add_latency(&mut self, latency: Duration) {
        trace!("Adding latency: {}ms", latency.as_millis());
        self.recent.push_front(latency);
        self.recent.truncate(HISTORY_LEN);
    }

    pub fn latency(&self) -> Duration {
        if self.recent.len() == 0 {
            // If connection has not been used, assume default latency
            return DEFAULT_LATENCY;
        }
        self.recent.iter().sum::<Duration>() / self.recent.len() as u32
    }

    pub fn take_reader(&mut self) -> LinkResult<ReceiverStream<(u64, LinkProtocol)>> {
        let mut reader = self.link.take_reader()?;
        let (tx, rx) = channel(10);
        let id = self.id;

        tokio::spawn(async move {
            loop {
                let Ok(proto) = reader.read().await else {
                    return;
                };
                if tx.send((id, proto)).await.is_err() {
                    return;
                }
            }
        });
        let x = ReceiverStream::new(rx);
        Ok(x)
    }

    pub(crate) async fn send(&mut self, msg: LinkProtocol) -> LinkResult {
        self.link.send(msg).await
    }

    pub(crate) async fn recv(&mut self) -> LinkResult<LinkProtocol> {
        self.link.recv().await
    }

    pub(crate) fn max_size(&self) -> u32 {
        self.link.max_size()
    }

    pub(crate) fn is_closed(&mut self) -> bool {
        self.link.is_closed()
    }
}
