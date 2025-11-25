
use crate::{LinkResult, protocol::LinkProtocol};
use std::{future::Future, pin::Pin};

/// The Link trait encapsulates different ways of connecting two endpoints, so
/// they can be used with [LinkSet]
pub trait Link: Send + Sync {


    /// Send a [LinkProtocol] to the other side of the network.
    fn send(&mut self, msg: LinkProtocol) -> impl Future<Output = LinkResult> + Send;
    /// Receive a [LinkProtocol] from the other side.
    fn recv(&mut self) -> impl Future<Output = LinkResult<LinkProtocol>> + Send;
    /// Returns a Receiver of [LinkProtocol]s that will fill with items from the
    /// Link.
    // fn take_reader(&mut self) -> LinkResult<Receiver<LinkProtocol>>;
	fn take_reader(&mut self) -> LinkResult<impl LinkReader+ 'static>;

    /// returns the maximum size of data that can be sent through this Link
    ///
    /// The [LinkSet] will automatically break up larger messages during the
    /// serialization and deserialization process to overcome this limit.
    fn max_size(&self) -> u32;

    /// Should the [LinkSet] remove this link. I.E. it will not be able to send
    /// any more data.
    fn is_closed(&mut self) -> bool;
}


/// Something returned from Link's take_reader() function
pub trait LinkReader: Send + Sync{
	fn read(&mut self) -> impl Future<Output=LinkResult<LinkProtocol>> + Send + Sync;
}

/// A wrapped version of LinkReader that returns Pin<Box<dyn Future>> instead of
/// impl Future to make it dyn-compatible
pub trait PinnedLinkReader: private::LinkReaderSeal + Send + Sync{
	fn read<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = LinkResult<LinkProtocol>> + Send + Sync + 'a>> ;
}

/// All LinkReaders should be able to be converted into a pinned version to be
/// returned from the PinnedLink
impl<T: LinkReader + Send + Sync> PinnedLinkReader for T {
	fn read<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = LinkResult<LinkProtocol>> + Send + Sync + 'a>> {
		Box::pin(async{self.read().await})
	}
}


/// This trait is a wrapper around [Link] to allow it to be a trait object. Implement [Link] instead of this trait.
///
/// There is a blanket implementation of PinnedLink for all types that implement Link, that pins the returned futures.
#[allow(dead_code)]
pub trait PinnedLink: private::LinkSeal + Send + Sync {
    /// Wrapper around [Link::send]
    fn send(&mut self, msg: LinkProtocol) -> Pin<Box<dyn Future<Output = LinkResult> + '_ + Send>>;
    /// Wrapper around [Link::recv]
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = LinkResult<LinkProtocol>> + '_ + Send>>;
    /// Wrapper around [Link::take_reader]
    // fn take_reader(&mut self) -> LinkResult<Receiver<LinkProtocol>>;
	fn take_reader(&mut self) -> LinkResult<Box<dyn PinnedLinkReader + 'static>>;

    /// Wrapper around [Link::max_size]
    fn max_size(&self) -> u32;
    /// Wrapper around [Link::is_closed]
    fn is_closed(&mut self) -> bool;
}

/// Any implementation of link should be able to be wrapped into a pinned link
impl<T: Link> PinnedLink for T {

    fn send(&mut self, msg: LinkProtocol) -> Pin<Box<dyn Future<Output = LinkResult> + '_ + Send>> {
        Box::pin(async { self.send(msg).await })
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = LinkResult<LinkProtocol>> + '_ + Send>> {
        Box::pin(async { self.recv().await })
    }

    fn take_reader(&mut self) -> LinkResult<Box<dyn PinnedLinkReader + 'static>> {
        Ok(Box::new(self.take_reader()?))
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
