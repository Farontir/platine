//! Files imported by the user and stored inside the project (images, HDRIs).
//!
//! A resource is named by the SHA-256 of its bytes: importing the same file twice stores
//! it once, and the name stays valid across Rust and platform versions.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// SHA-256 of a resource's bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceHash(pub [u8; 32]);

impl ResourceHash {
    /// Hash of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

impl fmt::Display for ResourceHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for ResourceHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResourceHash({self})")
    }
}

/// A string that is not 64 hexadecimal digits.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid resource hash `{0}`")]
pub struct ParseHashError(pub String);

impl FromStr for ResourceHash {
    type Err = ParseHashError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || ParseHashError(text.to_owned());
        if text.len() != 64 || !text.is_ascii() {
            return Err(error());
        }
        let mut bytes = [0; 32];
        for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
            let pair = std::str::from_utf8(pair).map_err(|_| error())?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| error())?;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for ResourceHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ResourceHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// What a resource is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceKind {
    /// Equirectangular environment (`.hdr` or `.exr`).
    Environment,
    /// Image used by a fill, a brush alpha or a parameter.
    Image,
}

/// An imported file. The bytes are stored next to the manifest, not inside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    /// File name it was imported from, for display and to keep its extension.
    pub name: String,
    /// Use of the resource.
    pub kind: ResourceKind,
    /// File contents; shared, as resources can be large.
    #[serde(skip)]
    pub bytes: Arc<[u8]>,
}

impl Resource {
    /// A resource holding `bytes`, and its hash.
    pub fn new(
        name: impl Into<String>,
        kind: ResourceKind,
        bytes: Arc<[u8]>,
    ) -> (ResourceHash, Self) {
        let hash = ResourceHash::of(&bytes);
        let resource = Self {
            name: name.into(),
            kind,
            bytes,
        };
        (hash, resource)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_round_trips_through_text() {
        let hash = ResourceHash::of(b"patine");
        let text = hash.to_string();
        assert_eq!(text.len(), 64);
        assert_eq!(text.parse::<ResourceHash>(), Ok(hash));
        assert!("xyz".parse::<ResourceHash>().is_err());
        assert!("g".repeat(64).parse::<ResourceHash>().is_err());
        assert!("é".repeat(32).parse::<ResourceHash>().is_err());
    }

    #[test]
    fn known_sha256() {
        // SHA-256 of the empty string.
        assert_eq!(
            ResourceHash::of(b"").to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
