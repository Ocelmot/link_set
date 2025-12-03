use std::error::Error;

use thiserror::Error;

/// A [Result] from the link_set crate
pub type LinkSetResult<T = ()> = Result<T, LinkSetError>;

#[derive(Error, Debug)]
pub enum LinkSetError {
    #[error("deserialize: unexpected end of stream")]
    DeserializeEOF,
    #[error("deserialize: found invalid byte {0}")]
    DeserializeInvalid(u8),
    #[error("deserialize: invalid slice length: 0")]
    DeserializeInvalidLen,

	#[error("TaskTerminated error: {0}")]
    TaskTerminated(#[source] Box<dyn Error + Send + Sync>),

    #[error("LinkSetSendable serialization error: {0}")]
    SendableSerialization(#[source] Box<dyn Error + Send + Sync>),
    #[error("LinkSetSendable deserialization error: {0}")]
    SendableDeserialization(#[source] Box<dyn Error + Send + Sync>),

    #[error("LinkError: Link failed operation with error: {0}")]
    LinkError(#[source] Box<dyn Error + Send + Sync>),

    #[error("The receiver has been taken from this link set (or this LinkSet is a .clone())")]
    ReceiverTaken,

    #[error("A Link has closed")]
    Closed,

    #[error("The link set has terminated")]
    Terminated,
}
