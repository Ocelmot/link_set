use std::{
    collections::VecDeque, io::{Read, Write}, mem, net::{Ipv4Addr, Ipv6Addr, SocketAddr::{self, V4, V6}, SocketAddrV4, SocketAddrV6}, sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    }, time::Duration,
};

use crate::links::{AddressRepr, Link, LinkReader};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, ErrorKind},
    net::{
        TcpListener, TcpStream, ToSocketAddrs,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::mpsc::{Receiver, channel},
};
use tracing::{debug, error};

static TCP_SCHEME: &'static str = "tcp";

pub type TcpLinkResult<T = ()> = Result<T, TcpLinkError>;

#[derive(Debug, thiserror::Error)]
pub enum TcpLinkError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("The receiver has already been taken")]
    ReceiverTaken,

    #[error("Invalid Address format")]
    InvalidAddr,

    #[error("The link has closed")]
    Closed,
}


impl From<SocketAddr> for AddressRepr {
    fn from(value: SocketAddr) -> Self {
        let bytes = match value {
            V4(addr) => {
                let mut bytes = Vec::with_capacity(6);
                bytes.extend_from_slice(&(addr.ip()).octets());
                bytes.extend_from_slice(&addr.port().to_be_bytes());
                bytes
            },
            V6(addr) => {
                let mut bytes = Vec::with_capacity(18);
                bytes.extend_from_slice(&(addr.ip()).octets());
                bytes.extend_from_slice(&addr.port().to_be_bytes());
                bytes
            }
        };

        AddressRepr::Bytes(bytes)
    }
}

/// An implementation of [Link] that uses TCP as its underlying transport
/// mechanism.
pub struct TcpLink {
    socket_reader: Option<OwnedReadHalf>,
    socket_writer: OwnedWriteHalf,
    read_len: Option<u32>,
    read_buffer: VecDeque<u8>,
    is_closed: Arc<AtomicBool>,
}

impl TcpLink {
    /// Sets up a listener for new incoming TcpLinks.
    ///
    /// New TcpLinks will be returned through the returned channel.
    pub async fn listen<A: ToSocketAddrs>(listen_addr: A) -> TcpLinkResult<Receiver<Self>> {
        let (tx, rx) = channel(50);
        let listener = TcpListener::bind(listen_addr).await?;

        // listen for connections,
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((socket, _)) => {
                        let (socket_reader, socket_writer) = socket.into_split();
                        let tcp_link = Self {
                            socket_reader: Some(socket_reader),
                            socket_writer,
                            read_len: None,
                            read_buffer: VecDeque::new(),
                            is_closed: Arc::new(AtomicBool::new(false)),
                        };

                        // emit Link on channel,
                        tx.send(tcp_link).await.map_err(|_| TcpLinkError::Closed)?;
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            ErrorKind::ConnectionAborted
                                | ErrorKind::ConnectionReset
                                | ErrorKind::ConnectionRefused
                        ) =>
                    {
                        continue;
                    }
                    Err(e) => {
                        error!("Accept failed: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }

            #[allow(unreachable_code)]
            TcpLinkResult::Ok(())
        });
        Ok(rx)
    }

    pub async fn connect_address(addr: AddressRepr) -> TcpLinkResult<Self> {
        match addr {
            AddressRepr::String(addr) => TcpLink::connect(addr).await,
            AddressRepr::Bytes(bytes) => {
                let sockaddr = match bytes.len() {
                    6 => {
                        let ip: &[u8; 4] = bytes[0..4].try_into().unwrap();
                        let port = u16::from_be_bytes(bytes[4..6].try_into().unwrap());

                        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from_octets(*ip), port))
                    },
                    18 => {
                        let ip: &[u8; 16] = bytes[0..16].try_into().unwrap();
                        let port = u16::from_be_bytes(bytes[16..18].try_into().unwrap());

                        SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from_octets(*ip), port, 0, 0))
                    }
                    _ => {
                        return Err(TcpLinkError::InvalidAddr);
                    }
                };

                TcpLink::connect(sockaddr).await
            },
        }
    }

    /// Establishes a connection to a process that has called [TcpLink::listen].
    pub async fn connect<A: ToSocketAddrs>(addr: A) -> TcpLinkResult<Self> {
        // Connect socket
        let socket = TcpStream::connect(addr).await?;
        let (socket_reader, socket_writer) = socket.into_split();

        // Return the connected TCPLink
        let read_len = None;
        let read_buffer = VecDeque::new();
        Ok(TcpLink {
            socket_reader: Some(socket_reader),
            socket_writer,
            read_len,
            read_buffer,

            is_closed: Arc::new(AtomicBool::new(false)),
        })
    }

    async fn read_chunk(&mut self) -> TcpLinkResult<Vec<u8>> {
        recv_chunk(
            self.socket_reader
                .as_mut()
                .ok_or(TcpLinkError::ReceiverTaken)?,
            &mut self.read_len,
            &mut self.read_buffer,
        )
        .await
    }

    async fn write_chunk(&mut self, data: &[u8]) -> TcpLinkResult {
        debug!("chunk write with len {}", data.len());
        self.socket_writer.write_u32(data.len() as u32).await?;
        self.socket_writer.write_all(data).await?;
        Ok(())
    }
}

/// Reads out the next chunk from the TCP socket. State is kept in the passed in
/// buffer and length.
async fn recv_chunk(
    reader: &mut OwnedReadHalf,
    length: &mut Option<u32>,
    buffer: &mut VecDeque<u8>,
) -> TcpLinkResult<Vec<u8>> {
    let mut buf = [0u8; 1024];

    let len = match length {
        Some(len) => *len,
        None => {
            loop {
                // trace!("starting chunk read...");
                if buffer.len() >= 4 {
                    let mut len_bytes = [0u8; 4];
                    buffer.read_exact(&mut len_bytes)?;
                    let len = u32::from_be_bytes(len_bytes);
                    // debug!("chunk read with len {}", len);
                    length.replace(len);
                    break len;
                }

                let x = reader.read(&mut buf).await?;
                if x == 0 {
                    return Err(TcpLinkError::Closed);
                }
                buffer.write_all(&buf[..x])?;
            }
        }
    };

    loop {
        // if we have enough data in the read buffer, return it
        if len <= buffer.len() as u32 {
            let mut data = Vec::with_capacity(len as usize);
            buffer.take(len.into()).read_to_end(&mut data)?;

            *length = None;

            return Ok(data);
        }

        // otherwise, read some more
        let x = reader.read(&mut buf).await?;
        if x == 0 {
            return Err(TcpLinkError::Closed);
        }
        buffer.write_all(&buf[..x])?;
    }
}

/// The reader part of the [TcpLink]
///
/// Can be taken out of the originating TcpLink. This can make receiving code
/// easier to manage.
pub struct TcpLinkReader(Receiver<Vec<u8>>);

impl LinkReader for TcpLinkReader {
    async fn read(
        &mut self,
    ) -> Result<Vec<u8>, impl std::error::Error + std::marker::Send + Sync + 'static> {
        self.0.recv().await.ok_or(TcpLinkError::Closed)
    }
}

impl Link for TcpLink {
    fn scheme() -> &'static str {
        TCP_SCHEME
    }

    async fn send(
        &mut self,
        msg: Vec<u8>,
    ) -> Result<(), impl std::error::Error + Send + Sync + 'static> {
        debug!("Sending msg {:?}", msg);
        self.write_chunk(&msg).await.inspect_err(|_| {
            self.is_closed.store(true, Ordering::SeqCst);
        })
    }

    async fn recv(&mut self) -> Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static> {
        let data = self.read_chunk().await.inspect_err(|e| {
            if matches!(e, TcpLinkError::ReceiverTaken) {
                return;
            } else {
                self.is_closed.store(true, Ordering::SeqCst);
            }
        });

        debug!("Receiving msg: {:?}", data);
        data
    }

    /// Splits the read portion and write portion of this TCP Link, returning a
    /// TcpLinkReader from which incoming messages can be received.
    fn take_reader(
        &mut self,
    ) -> Result<
        impl LinkReader + 'static,
        impl std::error::Error + std::marker::Send + Sync + 'static,
    > {
        let mut reader = self
            .socket_reader
            .take()
            .ok_or(TcpLinkError::ReceiverTaken)?;
        let mut read_len = self.read_len.take();
        let mut read_buffer = mem::take(&mut self.read_buffer);
        let is_closed = self.is_closed.clone();
        let (tx, rx) = channel(10);
        tokio::task::spawn(async move {
            loop {
                let chunk = recv_chunk(&mut reader, &mut read_len, &mut read_buffer).await;
                let chunk = chunk.inspect_err(|_| {
                    is_closed.store(true, Ordering::SeqCst);
                })?;

                debug!("Receiving msg (taken reader): {:?}", chunk);
                tx.send(chunk).await.map_err(|_| {
                    is_closed.store(true, Ordering::SeqCst);
                    TcpLinkError::Closed
                })?;
            }
            #[allow(unreachable_code)]
            TcpLinkResult::Ok(())
        });

        TcpLinkResult::Ok(TcpLinkReader(rx))
    }

    fn max_size(&self) -> u32 {
        let mut limit = u32::MAX;
        limit -= 4; // for the chunk length indicator
        limit
    }

    fn is_closed(&mut self) -> bool {
        self.is_closed.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn listen_connect_test() {
        let mut listener = TcpLink::listen("127.0.0.1:1940").await.expect("listener should succeed");

        let mut connect_link = TcpLink::connect("127.0.0.1:1940").await.expect("connect should succeed");
        let mut listen_link = listener.recv().await.expect("listener should get new link");

        let data = b"test_data".to_vec();
        
        connect_link.send(data.clone()).await.expect("send should succeed");

        let recvd_msg = listen_link.recv().await.expect("recv should succeed");

        assert_eq!(data, recvd_msg);
    }

}
