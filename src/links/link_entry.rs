use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use tokio::sync::mpsc::channel;
use tokio_stream::wrappers::ReceiverStream;
use tracing::trace;

use crate::{LinkSetResult, links::link::PinnedLink, protocol::LinkProtocol};

/// The last N historical samples to keep when calculating latency
static HISTORY_LEN: usize = 6;
/// The latency to fill the buffer when creating a new link
static DEFAULT_LATENCY: Duration = Duration::from_millis(100);

pub(crate) static MAX_LATENCY: Duration = Duration::from_secs(2);

/// This holds the link in the link manager. It adds metadata like latency, as
/// well as handling the serialization and deserialization to and from the
/// Link's Vec<u8>
pub(crate) struct LinkEntry {
    /// Identify this link from the others
    id: u64,
    /// The wrapped link
    link: Box<dyn PinnedLink>,
    /// The time the last un-received ping was sent.
    ping: Option<Instant>,
    /// Queue of recent latencies
    recent: VecDeque<Duration>,
}

impl LinkEntry {
    // LinkEntry takes 49b, use 56 to give extra space.
    pub(crate) const OVERHEAD: u32 = 56;

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

    pub fn scheme(&self) -> &'static str {
        self.link.scheme()
    }

    pub async fn send_ping(&mut self) -> LinkSetResult {
        // end the previous ping if there was one.
        self.end_ping();
        self.ping = Some(Instant::now());
        self.send(LinkProtocol::Ping).await
    }

    pub async fn send_pong(&mut self) -> LinkSetResult {
        self.send(LinkProtocol::Pong).await
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
        if self.recent.is_empty() {
            // If connection has not been used, assume default latency
            return DEFAULT_LATENCY;
        }
        self.recent.iter().sum::<Duration>() / self.recent.len() as u32
    }

    pub fn take_reader(&mut self) -> LinkSetResult<ReceiverStream<(u64, LinkProtocol)>> {
        let mut reader = self.link.take_reader()?;
        let (tx, rx) = channel(10);
        let id = self.id;

        tokio::spawn(async move {
            loop {
                let Ok(proto) = reader.read().await else {
                    return;
                };
                let mut data = VecDeque::from(proto);
                let Ok(proto) = LinkProtocol::deserialize(&mut data) else{
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

    pub(crate) async fn send(&mut self, msg: LinkProtocol) -> LinkSetResult {
        self.link.send(msg.serialize()).await
    }

    pub(crate) fn max_size(&self) -> u32 {
        // minus the overhead for the serialization
        self.link.max_size().saturating_sub(Self::OVERHEAD)
    }

    pub(crate) fn is_closed(&mut self) -> bool {
        self.link.is_closed()
    }
}
