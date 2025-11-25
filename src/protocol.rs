use std::{collections::VecDeque, fmt::Debug, io::Read};

use crate::{
    LinkError, LinkResult,
    epoch::{Epoch, opt_epoch_to_int},
};

#[derive(Clone, PartialEq, Eq)]
pub enum LinkProtocol {
    Reset {
        epoch: Option<Epoch>,
        request: bool,
    },
    Ack {
        /// The epoch to which this applies
        epoch: u64,
        /// The sequence number to be ack'd
        seq: u64,
        /// The index of the last byte to be ack'd
        last_index: u64,
    },
    MsgSlice {
        /// The epoch to which this applies
        epoch: u64,
        /// The sequence number of the slice
        seq: u64,
        /// The total length of this sequence
        seq_len: u64,

        /// The first index in the sent slice
        first_index: u64,

        /// The data in the slice
        data: Vec<u8>,
    },
    Ping,
    Pong,
}

impl LinkProtocol {
    /// The sequence number
    pub fn seq(&self) -> u64 {
        match self {
            LinkProtocol::Reset { .. } => 0,
            LinkProtocol::Ack { seq, .. } => *seq,
            LinkProtocol::MsgSlice { seq, .. } => *seq,
            LinkProtocol::Ping => 0,
            LinkProtocol::Pong => 0,
        }
    }

    /// The total length of data being sent by this sequence.
    pub fn seq_len(&self) -> u64 {
        match self {
            LinkProtocol::Reset { .. } => 0,
            LinkProtocol::Ack { .. } => 0,
            LinkProtocol::MsgSlice { seq_len, .. } => *seq_len,
            LinkProtocol::Ping => 0,
            LinkProtocol::Pong => 0,
        }
    }

    /// The index of the first byte in the slice
    pub fn first_index(&self) -> u64 {
        match self {
            LinkProtocol::Reset { .. } => 0,
            LinkProtocol::Ack { .. } => 0,
            LinkProtocol::MsgSlice { first_index, .. } => *first_index,
            LinkProtocol::Ping => 0,
            LinkProtocol::Pong => 0,
        }
    }

    /// The index of the last byte in the slice
    pub fn last_index(&self) -> u64 {
        match self {
            LinkProtocol::Reset { .. } => 0,
            LinkProtocol::Ack { last_index, .. } => *last_index,
            LinkProtocol::MsgSlice {
                first_index, data, ..
            } => *first_index + (data.len() as u64) - 1,
            LinkProtocol::Ping => 0,
            LinkProtocol::Pong => 0,
        }
    }

    /// The data in the slice
    pub fn data(&self) -> &[u8] {
        match self {
            LinkProtocol::Reset { .. } => &[],
            LinkProtocol::Ack { .. } => &[],
            LinkProtocol::MsgSlice { data, .. } => data.as_slice(),
            LinkProtocol::Ping => &[],
            LinkProtocol::Pong => &[],
        }
    }

    pub fn serialize(self) -> Vec<u8> {
        match self {
            LinkProtocol::Reset {
                epoch,
                request: reply,
            } => {
                let epoch = opt_epoch_to_int(epoch);
                let mut ret = Vec::with_capacity(10);
                ret.push(0); // Variant indicator for reset
                ret.extend_from_slice(&epoch.to_be_bytes());
                ret.push(reply as u8);
                ret
            }
            LinkProtocol::Ack {
                epoch,
                seq,
                last_index,
            } => {
                let mut ret = Vec::with_capacity(25);
                ret.push(1); // Variant indicator for Ack
                ret.extend_from_slice(&epoch.to_be_bytes());
                ret.extend_from_slice(&seq.to_be_bytes());
                ret.extend_from_slice(&last_index.to_be_bytes());
                ret
            }
            LinkProtocol::MsgSlice {
                epoch,
                seq,
                first_index,
                seq_len,
                mut data,
            } => {
                let mut ret = Vec::with_capacity(33 + data.len());
                let data_len = data.len() as u64;
                ret.push(2); // Variant indicator for MsgSlice
                ret.extend_from_slice(&epoch.to_be_bytes());
                ret.extend_from_slice(&seq.to_be_bytes());
                ret.extend_from_slice(&seq_len.to_be_bytes());
                ret.extend_from_slice(&first_index.to_be_bytes());
                ret.extend_from_slice(&data_len.to_be_bytes());
                ret.append(&mut data);
                ret
            }
            LinkProtocol::Ping => {
                let mut ret = Vec::with_capacity(1);
                ret.push(3);
                ret
            }
            LinkProtocol::Pong => {
                let mut ret = Vec::with_capacity(1);
                ret.push(4);
                ret
            }
        }
    }

    pub fn deserialize(data: &mut VecDeque<u8>) -> LinkResult<Self> {
        // Deserialize variant
        let variant = data.pop_front().ok_or(LinkError::DeserializeEOF)?;

        match variant {
            0 => {
                // Deserialize epoch
                let mut buf = [0u8; 8];
                data.read_exact(&mut buf)
                    .map_err(|_| LinkError::DeserializeEOF)?;
                let epoch = u64::from_be_bytes(buf);

                // Deserialize reply
                let reply = data.pop_front().ok_or(LinkError::DeserializeEOF)? != 0;
                Ok(LinkProtocol::Reset {
                    epoch: Epoch::from_int(epoch),
                    request: reply,
                })
            }
            1 => {
                // Deserialize epoch
                let mut buf = [0u8; 8];
                data.read_exact(&mut buf)
                    .map_err(|_| LinkError::DeserializeEOF)?;
                let epoch = u64::from_be_bytes(buf);

                // Deserialize seq number
                if data.len() < 8 {
                    return Err(LinkError::DeserializeEOF);
                }
                let mut buf = [0u8; 8];
                data.read_exact(&mut buf)
                    .map_err(|_| LinkError::DeserializeEOF)?;
                let seq = u64::from_be_bytes(buf);

                // Deserialize last index
                if data.len() < 8 {
                    return Err(LinkError::DeserializeEOF);
                }
                let mut buf = [0u8; 8];
                data.read_exact(&mut buf)
                    .map_err(|_| LinkError::DeserializeEOF)?;
                let last_index = u64::from_be_bytes(buf);

                Ok(LinkProtocol::Ack {
                    epoch,
                    seq,
                    last_index,
                })
            }
            2 => {
                // Deserialize epoch
                let mut buf = [0u8; 8];
                data.read_exact(&mut buf)
                    .map_err(|_| LinkError::DeserializeEOF)?;
                let epoch = u64::from_be_bytes(buf);

                // Deserialize seq number
                if data.len() < 8 {
                    return Err(LinkError::DeserializeEOF);
                }
                let mut buf = [0u8; 8];
                data.read_exact(&mut buf)
                    .map_err(|_| LinkError::DeserializeEOF)?;
                let seq = u64::from_be_bytes(buf);

                // Deserialize total length of the slice
                if data.len() < 8 {
                    return Err(LinkError::DeserializeEOF);
                }
                let mut buf = [0u8; 8];
                data.read_exact(&mut buf)
                    .map_err(|_| LinkError::DeserializeEOF)?;
                let seq_len = u64::from_be_bytes(buf);

                // Deserialize the index of the first byte in the slice
                if data.len() < 8 {
                    return Err(LinkError::DeserializeEOF);
                }
                let mut buf = [0u8; 8];
                data.read_exact(&mut buf)
                    .map_err(|_| LinkError::DeserializeEOF)?;
                let first_index = u64::from_be_bytes(buf);

                // Deserialize offset
                if data.len() < 8 {
                    return Err(LinkError::DeserializeEOF);
                }
                let mut buf = [0u8; 8];
                data.read_exact(&mut buf)
                    .map_err(|_| LinkError::DeserializeEOF)?;
                let data_len = u64::from_be_bytes(buf);

                // Deserialize data
                if data_len == 0 {
                    return Err(LinkError::DeserializeInvalidLen);
                }
                if data.len() < data_len as usize {
                    return Err(LinkError::DeserializeEOF);
                }

                let mut read_data = Vec::with_capacity(data_len as usize);
                data.take(data_len)
                    .read_to_end(&mut read_data)
                    .map_err(|_| LinkError::DeserializeEOF)?;

                Ok(LinkProtocol::MsgSlice {
                    epoch,
                    seq,
                    first_index,
                    seq_len,
                    data: read_data,
                })
            }
            3 => Ok(LinkProtocol::Ping),
            4 => Ok(LinkProtocol::Pong),
            _ => Err(LinkError::DeserializeInvalid(variant)),
        }
    }
}

impl Debug for LinkProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reset { epoch, request } => f
                .debug_struct("Reset")
                .field("epoch", epoch)
                .field("request", request)
                .finish(),
            Self::Ack {
                epoch,
                seq,
                last_index,
            } => f
                .debug_struct("Ack")
                .field("epoch", epoch)
                .field("seq", seq)
                .field("last_index", last_index)
                .finish(),
            Self::MsgSlice {
                epoch,
                seq,
                seq_len,
                first_index,
                data,
            } => f
                .debug_struct("MsgSlice")
                .field("epoch", epoch)
                .field("seq", seq)
                .field("seq_len", seq_len)
                .field("first_index", first_index)
                .field("data", &format!("<{} bytes>", data.len()))
                .finish(),
            Self::Ping => f.debug_struct("Ping").finish(),
            Self::Pong => f.debug_struct("Pong").finish(),
        }
    }
}

#[cfg(test)]
mod tests {

    use rand::random;
    use tracing::debug;

    use super::*;

    #[test_log::test]
    fn reset_round_trip() {
        let epoch = Epoch::random();
        let request = random();
        debug!("Testing epoch {:?}, reply {}", epoch, request);
        let rst = LinkProtocol::Reset {
            epoch: Some(epoch),
            request,
        };

        let serialized = rst.clone().serialize();
        let mut serialized = VecDeque::from(serialized);

        let deserialized = LinkProtocol::deserialize(&mut serialized).expect("should deserialize");

        assert_eq!(rst, deserialized);
    }

    #[test_log::test]
    fn slice_round_trip() {
        let epoch = random();
        let seq = random();
        let seq_len = random();
        let first_index = random();
        let data = random::<[u8; 20]>().into();
        debug!(
            "Testing epoch {}, seq {}, seq_len {}, first_index {}, data {:?}",
            epoch, seq, seq_len, first_index, data
        );
        let msg_slice = LinkProtocol::MsgSlice {
            epoch,
            seq,
            seq_len,
            first_index,
            data,
        };

        let serialized = msg_slice.clone().serialize();
        let mut serialized = VecDeque::from(serialized);

        let deserialized = LinkProtocol::deserialize(&mut serialized).expect("should deserialize");

        assert_eq!(msg_slice, deserialized);
    }

    #[test_log::test]
    fn ack_round_trip() {
        let epoch = random();
        let seq = random();
        let last_index = random();
        debug!(
            "Testing epoch {}, seq {}, last_index {}",
            epoch, seq, last_index
        );
        let msg_slice = LinkProtocol::Ack {
            epoch,
            seq,
            last_index,
        };

        let serialized = msg_slice.clone().serialize();
        let mut serialized = VecDeque::from(serialized);

        let deserialized = LinkProtocol::deserialize(&mut serialized).expect("should deserialize");

        assert_eq!(msg_slice, deserialized);
    }

    #[test_log::test]
    fn ping_round_trip() {
        let msg_ping = LinkProtocol::Ping;

        let serialized = msg_ping.clone().serialize();
        let mut serialized = VecDeque::from(serialized);

        let deserialized = LinkProtocol::deserialize(&mut serialized).expect("should deserialize");

        assert_eq!(msg_ping, deserialized);
    }

    #[test_log::test]
    fn pong_round_trip() {
        let msg_ping = LinkProtocol::Pong;

        let serialized = msg_ping.clone().serialize();
        let mut serialized = VecDeque::from(serialized);

        let deserialized = LinkProtocol::deserialize(&mut serialized).expect("should deserialize");

        assert_eq!(msg_ping, deserialized);
    }
}
