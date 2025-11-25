pub(crate) mod controller;
pub use controller::{LinkSet, LinkSetMessage, LinkSetSendable};
mod reader;
pub use reader::LinkSetReader;
#[cfg(feature = "serde")]
mod serde;