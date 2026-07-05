use crate::{
    LinkSetError, LinkSetResult,
    epoch::Epoch,
    links::{LinkEntry, link_entry::MAX_LATENCY},
    protocol::LinkProtocol,
    slice_manager::SliceManager,
};

pub(crate) struct LinkManager {
    links: Vec<LinkEntry>,
}

impl LinkManager {
    pub fn new() -> Self {
        Self { links: Vec::new() }
    }

    pub fn with_link(link: LinkEntry) -> Self {
        Self { links: vec![link] }
    }

    pub fn link_entries(&self) -> impl Iterator<Item = &LinkEntry> {
        self.links.iter()
    }

    pub fn add_link(&mut self, link: LinkEntry) {
        self.links.insert(0, link)
    }

    /// Used for debugging, normal use shouldnt need to manually remove links
    pub fn evict_link(&mut self, id: u64) -> bool {
        let mut found_link = false;
        self.links.retain(|e|{
            if e.id() == id {
                found_link = true;
                false
            } else {
                true
            }
        });
        found_link
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    async fn send_proto(&mut self, proto: LinkProtocol) -> LinkSetResult {
        loop {
            if let Some(link) = self.links.first_mut() {
                if link.is_closed() || link.latency() > MAX_LATENCY {
                    self.links.remove(0);
                    continue;
                }

                break link.send(proto).await;
            } else {
                // links is empty
                break LinkSetResult::Err(LinkSetError::Closed);
            }
        }
    }

    /// Send A reset with the associated parameters across the link
    pub(crate) async fn reset(&mut self, epoch: Option<Epoch>, request: bool) -> LinkSetResult {
        let proto = LinkProtocol::Reset { epoch, request };
        self.send_proto(proto).await
    }

    pub(crate) async fn ack(&mut self, epoch: Epoch, ack: (u64, u64)) -> LinkSetResult {
        let proto = LinkProtocol::Ack {
            epoch: epoch.to_int(),
            seq: ack.0,
            last_index: ack.1,
        };
        self.send_proto(proto).await
    }

    /// Get the remaining slices from the slice manager, and send them across the link
    pub async fn send_slices(&mut self, slice_mgr: &SliceManager) -> LinkSetResult {
        loop {
            if let Some(link) = self.links.first_mut() {
                if link.is_closed() || link.latency() > MAX_LATENCY {
                    self.links.remove(0);
                    continue;
                }
                let size = link.max_size();
                let protos = slice_mgr.get_slices(size);
                for proto in protos {
                    link.send(proto).await?;
                }
                break Ok(());
            } else {
                // links is empty
                break LinkSetResult::Err(LinkSetError::Closed);
            }
        }
    }

    // Ping mechanisms
    /// Ping all links in the manager
    pub async fn ping_all(&mut self) {
        for link in &mut self.links {
            let _ = link.send_ping().await;
        }
    }

    // Reply with a pong message along the specified link
    pub async fn pong(&mut self, id: u64) {
        if let Some(link) = self
            .links
            .iter_mut()
            .filter(|x| x.id() == id)
            .take(1)
            .next()
        {
            let _ = link.send_pong().await;
        }
    }

    // stop the ping timer for the specified link (when recv pong message)
    pub fn end_ping(&mut self, id: u64) {
        if let Some(link) = self
            .links
            .iter_mut()
            .filter(|x| x.id() == id)
            .take(1)
            .next()
        {
            link.end_ping();
        }
    }

    // Upkeep
    pub(crate) fn upkeep(&mut self) {
        self.links.retain_mut(|link| {
            if link.is_closed() {
                return false;
            }
            link.latency() < MAX_LATENCY
        });
    }
}
