use crate::{LinkSetError, LinkSetResult, links::Link};

pub struct Peekable<L: Link> {
    link: L,
    peeked: Option<Vec<u8>>,
}

impl<L: Link> Peekable<L> {
    pub fn new(link: L) -> Self {
        Self { link, peeked: None }
    }

    pub async fn peek(&mut self) -> LinkSetResult<&Vec<u8>> {
        if self.peeked.is_none() {
            let new_data = self
                .link
                .recv()
                .await
                .map_err(|e| LinkSetError::LinkError(Box::new(e)))?;
            self.peeked = Some(new_data);
        }

        Ok(self
            .peeked
            .as_ref()
            .expect("empty peeked should have been filled"))
    }

    /// Returns the internal copy of the peeked value, if there is one.
    pub fn take_peeked(&mut self) -> Option<Vec<u8>> {
        self.peeked.take()
    }
}

impl<L: Link> Link for Peekable<L> {
    fn scheme() -> &'static str {
        L::scheme()
    }

    async fn send(
        &mut self,
        msg: Vec<u8>,
    ) -> Result<(), impl std::error::Error + Send + Sync + 'static> {
        self.link.send(msg).await
    }

    async fn recv(&mut self) -> Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static> {
        self.link.recv().await
    }

    fn take_reader(
        &mut self,
    ) -> Result<
        impl crate::links::LinkReader + 'static,
        impl std::error::Error + Send + Sync + 'static,
    > {
        self.link.take_reader()
    }

    fn max_size(&self) -> u32 {
        self.link.max_size()
    }

    fn is_closed(&mut self) -> bool {
        self.link.is_closed()
    }
}
