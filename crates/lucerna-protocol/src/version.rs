//! Versão do protocolo (`MAJOR.MINOR`).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Versão do protocolo falada por este crate.
pub const PROTOCOL_VERSION: Version = Version { major: 1, minor: 0 };

/// Versão `MAJOR.MINOR` do protocolo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Version {
    /// Muda quando há quebra de compatibilidade.
    pub major: u32,
    /// Muda quando algo novo e opcional é acrescentado.
    pub minor: u32,
}

impl Version {
    /// Duas versões são compatíveis quando têm o mesmo `MAJOR`.
    pub fn is_compatible_with(&self, other: &Version) -> bool {
        self.major == other.major
    }
}

/// Texto que não é uma versão `MAJOR.MINOR`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("versão inválida: {0:?}")]
pub struct VersionError(String);

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for Version {
    type Err = VersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || VersionError(s.to_string());
        let (major, minor) = s.split_once('.').ok_or_else(bad)?;
        let part = |p: &str| {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            p.parse::<u32>().map_err(|_| bad())
        };
        Ok(Version {
            major: part(major)?,
            minor: part(minor)?,
        })
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
