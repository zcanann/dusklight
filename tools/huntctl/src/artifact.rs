use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::error::Error;
use std::fmt;
use std::str::FromStr;

pub const ARTIFACT_SCHEMA_VERSION: u16 = 2;

/// A stable content digest. The protocol does not prescribe the hashing
/// implementation, but production artifacts should use SHA-256.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
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
    pub protocol_name: String,
    pub protocol_version: u16,
    pub protocol_capabilities_digest: Digest,
    pub scenario_id: String,
    pub region_digest: Digest,
    pub language_assets_digest: Digest,
    pub scenario_digest: Digest,
    pub predicate_program_digest: Digest,
    pub action_schema_digest: Digest,
    pub observation_schema_digest: Digest,
    pub settings_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactIdentityError {
    pub field: &'static str,
    pub detail: &'static str,
}

impl fmt::Display for ArtifactIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.field, self.detail)
    }
}

impl Error for ArtifactIdentityError {}

impl BuildIdentity {
    pub fn validate(&self) -> Result<(), ArtifactIdentityError> {
        for (field, value) in [
            ("build.dusklight_commit", self.dusklight_commit.as_str()),
            ("build.aurora_commit", self.aurora_commit.as_str()),
            ("build.compiler", self.compiler.as_str()),
            ("build.target", self.target.as_str()),
            ("build.profile", self.profile.as_str()),
            ("build.fidelity_profile", self.fidelity_profile.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ArtifactIdentityError {
                    field,
                    detail: "must not be empty",
                });
            }
        }
        for (field, value) in [
            ("build.feature_digest", self.feature_digest),
            ("build.game_digest", self.game_digest),
        ] {
            require_digest(field, value)?;
        }
        if self.dirty_digest == Some(Digest::ZERO) {
            return Err(ArtifactIdentityError {
                field: "build.dirty_digest",
                detail: "must be absent for a clean tree or a nonzero digest",
            });
        }
        Ok(())
    }
}

impl ArtifactIdentity {
    pub fn validate(&self) -> Result<(), ArtifactIdentityError> {
        if self.schema_version != ARTIFACT_SCHEMA_VERSION {
            return Err(ArtifactIdentityError {
                field: "schema_version",
                detail: "is unsupported",
            });
        }
        self.build.validate()?;
        if self.protocol_name.trim().is_empty() {
            return Err(ArtifactIdentityError {
                field: "protocol_name",
                detail: "must not be empty",
            });
        }
        if self.protocol_version == 0 {
            return Err(ArtifactIdentityError {
                field: "protocol_version",
                detail: "must be nonzero",
            });
        }
        if self.scenario_id.trim().is_empty() {
            return Err(ArtifactIdentityError {
                field: "scenario_id",
                detail: "must not be empty",
            });
        }
        for (field, value) in [
            ("content_digest", self.content_digest),
            (
                "protocol_capabilities_digest",
                self.protocol_capabilities_digest,
            ),
            ("region_digest", self.region_digest),
            ("language_assets_digest", self.language_assets_digest),
            ("scenario_digest", self.scenario_digest),
            ("predicate_program_digest", self.predicate_program_digest),
            ("action_schema_digest", self.action_schema_digest),
            ("observation_schema_digest", self.observation_schema_digest),
            ("settings_digest", self.settings_digest),
        ] {
            require_digest(field, value)?;
        }
        Ok(())
    }
}

fn require_digest(field: &'static str, value: Digest) -> Result<(), ArtifactIdentityError> {
    if value == Digest::ZERO {
        return Err(ArtifactIdentityError {
            field,
            detail: "must be a nonzero SHA-256 digest",
        });
    }
    Ok(())
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

    #[test]
    fn complete_identity_rejects_ambiguous_zero_digest() {
        let identity = ArtifactIdentity {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            content_digest: Digest::ZERO,
            build: BuildIdentity {
                dusklight_commit: "dusk".into(),
                aurora_commit: "aurora".into(),
                compiler: "clang".into(),
                target: "arm64-apple-darwin".into(),
                profile: "debug".into(),
                feature_digest: Digest([1; 32]),
                game_digest: Digest([2; 32]),
                dirty_digest: None,
                fidelity_profile: "native".into(),
            },
            protocol_name: "dusklight-automation".into(),
            protocol_version: 2,
            protocol_capabilities_digest: Digest([3; 32]),
            scenario_id: "fixture".into(),
            region_digest: Digest([4; 32]),
            language_assets_digest: Digest([5; 32]),
            scenario_digest: Digest([6; 32]),
            predicate_program_digest: Digest([7; 32]),
            action_schema_digest: Digest([8; 32]),
            observation_schema_digest: Digest([9; 32]),
            settings_digest: Digest([10; 32]),
        };
        assert_eq!(identity.validate().unwrap_err().field, "content_digest");
    }
}
