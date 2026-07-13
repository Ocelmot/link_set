use std::{fmt::Display, str::FromStr};

use base64::engine::general_purpose::URL_SAFE;

use crate::{LinkSetError, LinkSetResult};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AddressRepr {
    String(String),
    Bytes(Vec<u8>),
}

const STRING_SYMBOL: u8 = 0;
const BYTES_SYMBOL: u8 = 1;

impl AddressRepr {
    pub fn addr_string(&self) -> LinkSetResult<&str> {
        match self {
            AddressRepr::String(s) => Ok(s.as_str()),
            AddressRepr::Bytes(_) => Err(LinkSetError::IncorrectAddrType {
                expected: "string".into(),
                found: "bytes".into(),
            }),
        }
    }

    pub fn addr_bytes(&self) -> LinkSetResult<&[u8]> {
        match self {
            AddressRepr::String(_) => Err(LinkSetError::IncorrectAddrType {
                expected: "bytes".into(),
                found: "string".into(),
            }),
            AddressRepr::Bytes(b) => Ok(&b),
        }
    }
}

impl From<String> for AddressRepr {
    fn from(value: String) -> Self {
        AddressRepr::String(value)
    }
}

impl From<&str> for AddressRepr {
    fn from(value: &str) -> Self {
        value.to_string().into()
    }
}

impl From<Vec<u8>> for AddressRepr {
    fn from(value: Vec<u8>) -> Self {
        AddressRepr::Bytes(value)
    }
}

/// An address that is tagged with the transport scheme it belongs with.
///
/// Of the form "scheme:addr".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address {
    scheme: String,
    addr: AddressRepr,
}

impl Address {
    pub fn new<A: Into<String>, B: Into<AddressRepr>>(scheme: A, addr: B) -> Self {
        Self {
            scheme: scheme.into(),
            addr: addr.into(),
        }
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn addr(&self) -> &AddressRepr {
        &self.addr
    }

    pub fn addr_string(&self) -> LinkSetResult<&str> {
        self.addr.addr_string()
    }

    pub fn addr_bytes(&self) -> LinkSetResult<&[u8]> {
        self.addr.addr_bytes()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut ret = Vec::new();
        ret.extend_from_slice(self.scheme.as_bytes());
        ret.push(b':');
        match &self.addr {
            AddressRepr::String(string) => {
                ret.push(STRING_SYMBOL);
                ret.extend_from_slice(string.as_bytes());
            }
            AddressRepr::Bytes(bytes) => {
                ret.push(BYTES_SYMBOL);
                ret.extend_from_slice(&bytes);
            }
        }
        return ret;
    }

    pub fn from_bytes(bytes: &[u8]) -> LinkSetResult<Self> {
        let delim = bytes.iter().position(|b| *b == b':').ok_or(LinkSetError::DeserializeEOF)?;
        
        if bytes.len() < delim + 2 {
            Err(LinkSetError::DeserializeEOF)?;
        }

        let scheme = String::from_utf8(bytes[0..delim].to_vec())
            .map_err(|_| LinkSetError::DeserializeInvalid(bytes[delim]))?;
        let symbol = bytes[delim + 1];
        let addr_bytes = &bytes[delim + 2..];
        let repr = match symbol {
            STRING_SYMBOL => {
                let string = String::from_utf8(addr_bytes.to_vec())
                    .map_err(|_| LinkSetError::DeserializeInvalid(bytes[delim + 1]))?;
                AddressRepr::String(string)
            }
            BYTES_SYMBOL => AddressRepr::Bytes(addr_bytes.to_vec()),
            _ => Err(LinkSetError::DeserializeInvalid(symbol))?,
        };
        Ok(Self { scheme, addr: repr })
    }
}

/// This means there was no ':' in the input string
#[derive(Debug, thiserror::Error)]
#[error("address must be of the form \"scheme:repr:addr\", or b64 was invalid")]
pub struct AddressParseError;

impl FromStr for Address {
    type Err = AddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (scheme, remainder) = s.split_once(':').ok_or(AddressParseError)?;
        let repr = match remainder.split_once(':') {
            Some((repr, addr)) => {
                match repr {
                    "txt" => AddressRepr::String(addr.to_owned()),
                    "b64" => {
                        let decoded = base64::Engine::decode(&URL_SAFE, addr)
                            .map_err(|_| AddressParseError)?;
                        AddressRepr::Bytes(decoded)
                    }

                    // Unrecognized repr as String
                    _ => AddressRepr::String(remainder.to_owned()),
                }
            }
            None => {
                // No second colon, interpret as text
                AddressRepr::String(remainder.to_owned())
            }
        };
        Ok(Self {
            scheme: scheme.to_owned(),
            addr: repr,
        })
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.addr {
            AddressRepr::String(addr) => {
                write!(f, "{}:txt:{}", self.scheme, addr)
            }
            AddressRepr::Bytes(bytes) => {
                let addr = base64::Engine::encode(&URL_SAFE, bytes);
                write!(f, "{}:b64:{}", self.scheme, addr)
            }
        }
    }
}

/// Serializes as the same "scheme:addr" string produced by [Display]
#[cfg(feature = "serde")]
impl serde::Serialize for Address {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Deserializes from a "scheme:addr" string via [FromStr]
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Address {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test_log::test]
    fn string_round_trip() {
        let addr = Address::new("test_scheme", "test_addr");

        let displayed_addr = format!("{}", &addr);

        let parsed_addr: Address = displayed_addr.parse().expect("address should round-trip");

        assert_eq!(addr, parsed_addr, "addresses should match");
    }

    #[test_log::test]
    fn parse_string_default() {
        let displayed_addr = String::from("test_scheme:192.168.0.1:8080");

        let parsed_addr: Address = displayed_addr.parse().expect("address should round-trip");

        assert_eq!(parsed_addr.scheme(), "test_scheme");

        let parsed_addr_string = parsed_addr.addr_string().unwrap();
        assert_eq!(parsed_addr_string, "192.168.0.1:8080");
    }

    #[test_log::test]
    fn parse_string_default_txt() {
        let displayed_addr = String::from("test_scheme:txt:192.168.0.1:8080");

        let parsed_addr: Address = displayed_addr.parse().expect("address should round-trip");

        assert_eq!(parsed_addr.scheme(), "test_scheme");

        let parsed_addr_string = parsed_addr.addr_string().unwrap();
        assert_eq!(parsed_addr_string, "192.168.0.1:8080");
    }

    #[test_log::test]
    fn parse_string_empty_txt() {
        let displayed_addr = String::from("test_scheme:txt:");

        let parsed_addr: Address = displayed_addr.parse().expect("address should round-trip");

        assert_eq!(parsed_addr.scheme(), "test_scheme");

        let parsed_addr_string = parsed_addr.addr_string().unwrap();
        assert_eq!(parsed_addr_string, "");
    }

    #[test_log::test]
    fn parse_string_default_b64() {
        let displayed_addr = String::from("test_scheme:b64:MTkyLjE2OC4wLjE6ODA4MA==");

        let parsed_addr: Address = displayed_addr.parse().expect("address should round-trip");

        assert_eq!(parsed_addr.scheme(), "test_scheme");

        let parsed_addr_bytes = parsed_addr.addr_bytes().unwrap();
        assert_eq!(parsed_addr_bytes, b"192.168.0.1:8080");
    }

    #[test_log::test]
    fn parse_string_empty_b64() {
        let displayed_addr = String::from("test_scheme:b64:");

        let parsed_addr: Address = displayed_addr.parse().expect("address should round-trip");

        assert_eq!(parsed_addr.scheme(), "test_scheme");

        let parsed_addr_bytes = parsed_addr.addr_bytes().unwrap();
        assert_eq!(parsed_addr_bytes, b"");
    }

    #[test_log::test]
    fn display_round_trip() {
        let addr = Address::new("test_scheme", "test_addr");

        let displayed_addr = format!("{}", &addr);

        let parsed_addr: Address = displayed_addr.parse().expect("address should round-trip");

        assert_eq!(addr, parsed_addr, "addresses should match");
    }

    #[test_log::test]
    fn compact_bytes_round_trip() {
        let addr = Address::new("test_scheme", "test_addr");

        let bytes = addr.to_bytes();

        let parsed_addr = Address::from_bytes(&bytes).expect("address should round-trip");

        assert_eq!(addr, parsed_addr, "addresses should match");
    }



    #[test_log::test]
    fn empty_compact_bytes_round_trip() {
        let addr = Address::new("test_scheme", "");

        let bytes = addr.to_bytes();

        let parsed_addr = Address::from_bytes(&bytes).expect("address should round-trip");

        assert_eq!(addr, parsed_addr, "addresses should match");
    }

    #[test_log::test]
    fn empty_compact_bytes_input() {

        let bytes = b"test_scheme:";

        let parsed_addr = Address::from_bytes(bytes);
        println!("{:?}", parsed_addr);
        // assert_eq!(parsed_addr, Err(()));

        // assert_eq!(parsed_addr.scheme.as_str(), "test_scheme", "schemes should match");
        // assert_eq!(parsed_addr.addr.addr_bytes(), "test_scheme", "schemes should match");
    }
}
