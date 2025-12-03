use std::pin::Pin;

use crate::{
    LinkSetResult,
    links::link::{Link, PinnedLink},
};

pub trait LinkConnector: Send + Sync + 'static{
    fn connect(&mut self, addr: String) -> impl Future<Output = LinkSetResult<impl Link + 'static>> + Send + Sync;
}

// Link connector is implemented for async functions that match its signature
impl<F: Send + Sync + 'static, Ret, L> LinkConnector for F
where
    F: FnMut(String) -> Ret,
    Ret: Future<Output = LinkSetResult<L>> + Send + Sync,
    L: Link + 'static,
{
    fn connect(&mut self, addr: String) -> impl Future<Output = LinkSetResult<impl Link + 'static>> {
        self(addr)
    }
}

// The pinned version of the LinkConnector trait. Wraps returned futures with pin and box
pub(crate) trait PinnedLinkConnector: Sync + Send {
    fn connect<'a>(
        &'a mut self,
        addr: String,
    ) -> Pin<Box<dyn Future<Output = LinkSetResult<Box<dyn PinnedLink + 'static>>> + Send + Sync + 'a>>;
}

// PinnedLinkConnector is implemented for all LinkConnectors
impl<LC: LinkConnector> PinnedLinkConnector for LC {
    fn connect<'a>(
        &'a mut self,
        addr: String,
    ) -> Pin<Box<dyn Future<Output = LinkSetResult<Box<dyn PinnedLink + 'static>>> + Send + Sync + 'a>> {
        let x = async {
            let link = self.connect(addr).await?;
            LinkSetResult::Ok(Box::new(link) as Box<dyn PinnedLink>)
        };
        Box::pin(x)
    }
}
