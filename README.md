# Link Set

This crate manages a set of connections (links) between two endpoints. The link
set will make sure that messages sent across the links are received in order.
The set will also resend unreceived messages. This is important even when using
links backed by TCP because one link may time out after a message has been sent.
Or to links could transport messages at different speeds and deliver later
messages first.

The purpose of this crate is to allow devices in a peer to peer setting to send
messages across connections with different protocols as needed. It also helps
reestablish the connection when necessary. However, it does not split traffic
across connections to increase bandwidth.

## Usage

A Link set sends messages of a given type. The type must implement the
LinkSetSendable trait. The trait has two methods, one to convert to bytes, and
one to convert from bytes. The crate has a configuration option `serde` that
implements this trait for all types that implement the serde traits `Serialize`
and `DeserializeOwned`. This implementation uses the bincode format internally.

Example Implementation
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct Data(String);

impl LinkSetSendable for Data {
    type E = std::string::FromUtf8Error;

    fn to_bytes(self) -> Result<Vec<u8>, Self::E> {
        Ok(self.0.as_bytes().to_vec())
    }

    fn from_bytes(bytes: Vec<u8>) -> Result<Self, Self::E>
    where
        Self: Sized,
    {
        Ok(Data(String::from_utf8(bytes)?))
    }
}
```

The next step is to initialize the link set.
```rust
// Annotated with the `Data` type from above
let link_set = LinkSet::<Data>::new(); 
```

There are several configuration options that change the way the link set behaves.

- Auto connect determines the behavior when all the links in the link set close.
  If it is set to true, it will try to reconnect, if it is false, the link set
  will become disconnected
- Reconnection timeout allows a link set to try to reconnect without emitting a
  disconnected message to the user. This kind of reconnection will appear as if
  it never disconnected.
- Grace period timeout causes a link set to allow incoming reconnections (as
  described above) without needing to have the means to reconnect itself. This
  is useful in 'server' contexts where the program might not know the address of
  the other side.

Examples
```rust
link_set.set_auto_connect(true).await;

// (Some(Duration::ZERO) = skip, None = try forever)
link_set.set_reconnection_timeout(Some(Duration::from_secs(30))).await;

// (Some(Duration::ZERO) = skip, None = try forever)
link_set.set_grace_period_timeout(Some(Duration::from_secs(30))).await;
```

There are also several other commands required to use the link set

- The add connector function lets the link set establish new outgoing links when
  it determines that it needs a new link. A link set with no connectors will
  have to rely on links supplied through the add link function described below.
  A connector is provided an address in the form of a string, and can return a
  link if it was able to connect.
- The add address function works in combination with the add connector function.
  It provides an address to the link set to try with its connectors when
  attempting to connect. There is also a variant called try address, in which the address is only used once with each connector before being discarded.
- The add link function adds an already active link to the link set. This will
  cause the link set to become connected if it wasn't already.

Examples
``` rust
link_set.add_connector(|addr| async { Ok(some_link) });

link_set.add_addr("some addr".to_owned()).await;

// this will cause the link set to connect if it was not already
link_set.add_link(some_link).await;
```

And of course finally, there are the send and recv functions.

``` rust
link_set.send(Data(String::from("Test Message"))).await;

let msg = link_set.recv().await;
// msg will be a Result<LinkSetMessage<M>, LinkError>, where M is the message type
// LinkSetMessages can be Disconnected, Connected(Epoch), or Message(M, Epoch)
```
