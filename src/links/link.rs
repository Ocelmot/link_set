use crate::{LinkSetError, LinkSetResult};
use std::{future::Future, pin::Pin};

/// The Link trait encapsulates different ways of connecting two endpoints, so
/// they can be used with [LinkSet]
pub trait Link: Send + Sync {
    /// Send a [Vec<u8>] to the other side of the network.
    ///
    /// This must be read by the other side with the same chunk boundaries. Ten
    /// bytes sent may not become two five byte reads. However, order of arrival
    /// or guarantee of delivery are not required.
    #[allow(refining_impl_trait)]
    fn send(
        &mut self,
        msg: Vec<u8>,
    ) -> impl Future<Output = Result<(), impl std::error::Error + Send + Sync + 'static>> + Send;

    /// Receive a [LinkProtocol] from the other side.
    ///
    /// This must be read with the same chunk boundaries as were sent by the
    /// other side. Ten bytes sent may not become two five byte reads. However,
    /// order of arrival or guarantee of delivery are not required.
    #[allow(refining_impl_trait)]
    fn recv(
        &mut self,
    ) -> impl Future<Output = Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static>> + Send;

    /// Returns a Receiver of [Vec<u8>]s that will fill with items from the
    /// Link.
    #[allow(refining_impl_trait)]
    fn take_reader(
        &mut self,
    ) -> Result<impl LinkReader + 'static, impl std::error::Error + Send + Sync + 'static>;

    /// returns the maximum size of data that can be sent through this Link
    ///
    /// The [LinkSet] will automatically break up larger messages during the
    /// serialization and deserialization process to overcome this limit. The
    /// LinkSet will not send Vec<u8>s larger than this amount through this
    /// link.
    fn max_size(&self) -> u32;

    /// Hint to indicate that this link can no longer carry data. This is not
    /// required, and if a link has a high enough latency, the LinkSet might
    /// decide its closed anyway.
    fn is_closed(&mut self) -> bool;
}

/// Something returned from Link's take_reader() function
pub trait LinkReader: Send + Sync {
    #[allow(refining_impl_trait)]
    fn read(
        &mut self,
    ) -> impl Future<Output = Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static>>
    + Send
    + Sync;
}

/// A wrapped version of LinkReader that returns Pin<Box<dyn Future>> instead of
/// impl Future to make it dyn-compatible
pub trait PinnedLinkReader: private::LinkReaderSeal + Send + Sync {
    fn read<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = LinkSetResult<Vec<u8>>> + Send + Sync + 'a>>;
}

/// All LinkReaders should be able to be converted into a pinned version to be
/// returned from the PinnedLink
impl<T: LinkReader + Send + Sync> PinnedLinkReader for T {
    fn read<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = LinkSetResult<Vec<u8>>> + Send + Sync + 'a>> {
        Box::pin(async {
            self.read()
                .await
                .map_err(|e| LinkSetError::LinkError(Box::new(e)))
        })
    }
}

/// This trait is a wrapper around [Link] to allow it to be a trait object.
/// Implement [Link] instead of this trait.
///
/// There is a blanket implementation of PinnedLink for all types that implement
/// Link, that pins the returned futures.
#[allow(dead_code)]
pub trait PinnedLink: private::LinkSeal + Send + Sync {
    /// Wrapper around [Link::send]
    fn send(&mut self, msg: Vec<u8>) -> Pin<Box<dyn Future<Output = LinkSetResult> + '_ + Send>>;
    /// Wrapper around [Link::recv]
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = LinkSetResult<Vec<u8>>> + '_ + Send>>;
    /// Wrapper around [Link::take_reader]
    fn take_reader(&mut self) -> LinkSetResult<Box<dyn PinnedLinkReader + 'static>>;

    /// Wrapper around [Link::max_size]
    fn max_size(&self) -> u32;
    /// Wrapper around [Link::is_closed]
    fn is_closed(&mut self) -> bool;
}

/// Any implementation of link should be able to be wrapped into a pinned link
impl<T: Link> PinnedLink for T {
    fn send(&mut self, msg: Vec<u8>) -> Pin<Box<dyn Future<Output = LinkSetResult> + '_ + Send>> {
        Box::pin(async {
            self.send(msg)
                .await
                .map_err(|e| LinkSetError::LinkError(Box::new(e)))
        })
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = LinkSetResult<Vec<u8>>> + '_ + Send>> {
        Box::pin(async {
            self.recv()
                .await
                .map_err(|e| LinkSetError::LinkError(Box::new(e)))
        })
    }

    fn take_reader(&mut self) -> LinkSetResult<Box<dyn PinnedLinkReader + 'static>> {
        Ok(Box::new(
            self.take_reader()
                .map_err(|e| LinkSetError::LinkError(Box::new(e)))?,
        ))
    }

    fn max_size(&self) -> u32 {
        self.max_size()
    }

    fn is_closed(&mut self) -> bool {
        self.is_closed()
    }
}

/// Convert a PinnedLink into a Box<dyn PinnedLink>
impl<PL: PinnedLink + 'static> From<PL> for Box<dyn PinnedLink> {
    fn from(value: PL) -> Self {
        Box::new(value)
    }
}

pub(crate) mod private {
    pub trait LinkSeal {}
    pub trait LinkReaderSeal {}

    impl<L: super::Link> LinkSeal for L {}
    impl<L: super::LinkReader> LinkReaderSeal for L {}
}
