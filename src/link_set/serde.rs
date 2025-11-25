use std::{error::Error, fmt::Display};

use bincode::error::{DecodeError, EncodeError};

use crate::LinkSetSendable;

#[derive(Debug)]
pub enum SerdeError {
    Encode(EncodeError),
    Decode(DecodeError),
}

impl Error for SerdeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            SerdeError::Encode(encode_error) => Some(encode_error),
            SerdeError::Decode(decode_error) => Some(decode_error),
        }
    }

    fn cause(&self) -> Option<&dyn Error> {
        self.source()
    }
}

impl Display for SerdeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SerdeError::Encode(encode_error) => write!(f, "EncodeError: {}", encode_error),
            SerdeError::Decode(decode_error) => write!(f, "DecodeError: {}", decode_error),
        }
    }
}

impl From<EncodeError> for SerdeError {
    fn from(value: EncodeError) -> Self {
        Self::Encode(value)
    }
}

impl From<DecodeError> for SerdeError {
    fn from(value: DecodeError) -> Self {
        Self::Decode(value)
    }
}

impl<S> LinkSetSendable for S
where
    S: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
{
    type E = SerdeError;
    fn to_bytes(self) -> Result<Vec<u8>, SerdeError> {
        Ok(bincode::serde::encode_to_vec(
            self,
            bincode::config::standard(),
        )?)
    }

    fn from_bytes(bytes: Vec<u8>) -> Result<Self, SerdeError>
    where
        Self: Sized,
    {
        let (msg, _len) = bincode::serde::decode_from_slice(&bytes, bincode::config::standard())?;
        if _len < bytes.len() {
            Err(SerdeError::Decode(DecodeError::OtherString(
                "Deserialization finished, but there are more bytes".to_owned(),
            )))
        } else {
            Ok(msg)
        }
    }
}
