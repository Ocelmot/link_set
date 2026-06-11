use std::{fmt::Display, str::FromStr};


/// An address that is tagged with the transport scheme it belongs with.
/// 
/// Of the form "scheme:addr".
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct Address{
    scheme: String,
    addr: String,
}

impl Address{
    pub fn new<A: Into<String>, B:Into<String>>(scheme: A, addr: B) -> Self {
        Self{
            scheme: scheme.into(),
            addr: addr.into(),
        }
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }
}

/// This means there was no ':' in the input string
#[derive(Debug, thiserror::Error)]
#[error("address must be of the form \"scheme:addr\"")]
pub struct AddressParseError;

impl FromStr for Address{
    type Err = AddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (scheme, addr) = s.split_once(':').ok_or(AddressParseError)?;
        Ok(Self::new(scheme, addr))
    }
}

impl Display for Address{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.scheme, self.addr)
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
