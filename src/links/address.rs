use std::{fmt::Display, str::FromStr};


/// An address that is tagged with the transport scheme it belongs with.
/// 
/// Of the form "scheme:addr".
#[derive(Debug, PartialEq, Eq)]
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
#[derive(Debug)]
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