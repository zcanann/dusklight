use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::error::Error;
use std::fmt;
use std::str::FromStr;

pub const ARTIFACT_SCHEMA_VERSION: u16 = 1;

/// A stable content digest. The protocol does not prescribe the hashing
/// implementation, but production artifacts should use SHA-256.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    pub const ZERO: Self = Self([0; 32]);

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseDigestError;

impl fmt::Display for ParseDigestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("digest must contain exactly 64 hexadecimal characters")
    }
}

impl Error for ParseDigestError {}

impl FromStr for Digest {
    type Err = ParseDigestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64 {
            return Err(ParseDigestError);
        }
        let mut bytes = [0_u8; 32];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let pair = std::str::from_utf8(pair).map_err(|_| ParseDigestError)?;
            bytes[index] = u8::from_str_radix(pair, 16).map_err(|_| ParseDigestError)?;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// Everything that can invalidate a deterministic replay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildIdentity {
    pub dusklight_commit: String,
    pub aurora_commit: String,
    pub compiler: String,
    pub target: String,
    pub profile: String,
    pub feature_digest: Digest,
    pub game_digest: Digest,
    pub dirty_digest: Option<Digest>,
    pub fidelity_profile: String,
}

/// Portable identity for a corpus artifact. It contains no host paths or
/// native pointers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    pub schema_version: u16,
    pub content_digest: Digest,
    pub build: BuildIdentity,
    pub scenario_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_hex_round_trip() {
        let digest = Digest([0x5a; 32]);
        assert_eq!(digest.to_string().parse::<Digest>().unwrap(), digest);
    }

    #[test]
    fn digest_rejects_bad_input() {
        assert!("abcd".parse::<Digest>().is_err());
        assert!(["g"; 64].concat().parse::<Digest>().is_err());
    }
}
