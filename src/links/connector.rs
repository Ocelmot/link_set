use std::pin::Pin;

use crate::{
    LinkSetResult,
    links::link::{Link, PinnedLink},
};

pub trait LinkConnector: Send + Sync + 'static{
    fn scheme(&self) -> &'static str;
    fn connect(&mut self, addr: String) -> impl Future<Output = Result<impl Link + 'static, impl std::error::Error + Send + Sync + 'static>> + Send + Sync;
}

// Link connector is implemented for async functions that match its signature
impl<F: Send + Sync + 'static, Ret, L, E> LinkConnector for (&'static str, F)
where
    F: FnMut(String) -> Ret,
    Ret: Future<Output = Result<L, E>> + Send + Sync,
    L: Link + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    fn scheme(&self) -> &'static str {self.0}
    fn connect(&mut self, addr: String) -> impl Future<Output = Result<impl Link + 'static, impl std::error::Error + Send + Sync + 'static>> {
        self.1(addr)
    }
}

// The pinned version of the LinkConnector trait. Wraps returned futures with pin and box
pub(crate) trait PinnedLinkConnector: Sync + Send {
    fn scheme(&self) -> &'static str;
    fn connect<'a>(
        &'a mut self,
        addr: String,
    ) ->  Pin<Box<dyn Future<Output = LinkSetResult<Box<dyn PinnedLink + 'static>>> + Send + Sync + 'a>> ;
}

// PinnedLinkConnector is implemented for all LinkConnectors
impl<LC: LinkConnector> PinnedLinkConnector for LC {
    fn scheme(&self) -> &'static str {self.scheme()}
    fn connect<'a>(
        &'a mut self,
        addr: String,
    ) -> Pin<Box<dyn Future<Output = LinkSetResult<Box<dyn PinnedLink + 'static>>> + Send + Sync + 'a>> {
        let x = async {
            let link = self.connect(addr).await.map_err(|e| crate::LinkSetError::LinkError(Box::new(e)))?;
            LinkSetResult::Ok(Box::new(link) as Box<dyn PinnedLink>)
        };
        Box::pin(x)
    }
}
