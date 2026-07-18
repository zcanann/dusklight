//! Portable proposal identity shared by scripted, random, search, and learned agents.

use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const CANDIDATE_ENVELOPE_SCHEMA_V1: &str = "dusklight-candidate-envelope/v1";
const MAX_ID_BYTES: usize = 96;
const MAX_VERSION_BYTES: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateEnvelope {
    pub schema: String,
    pub content_sha256: Digest,
    pub candidate_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_candidate_sha256: Option<Digest>,
    pub generation: u32,
    pub objective: NamedDigest,
    pub action_schema: NamedDigest,
    pub seed: u64,
    pub proposer: ProposerIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NamedDigest {
    pub id: String,
    pub sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposerIdentity {
    pub kind: ProposerKind,
    pub id: String,
    pub version: String,
    pub configuration_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposerKind {
    Scripted,
    Random,
    StructuredSearch,
    Learned,
}

impl CandidateEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        candidate_sha256: Digest,
        parent_candidate_sha256: Option<Digest>,
        generation: u32,
        objective: NamedDigest,
        action_schema: NamedDigest,
        seed: u64,
        proposer: ProposerIdentity,
    ) -> Result<Self, CandidateEnvelopeError> {
        let mut envelope = Self {
            schema: CANDIDATE_ENVELOPE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            candidate_sha256,
            parent_candidate_sha256,
            generation,
            objective,
            action_schema,
            seed,
            proposer,
        };
        envelope.content_sha256 = envelope.compute_content_sha256()?;
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn validate(&self) -> Result<(), CandidateEnvelopeError> {
        if self.schema != CANDIDATE_ENVELOPE_SCHEMA_V1 {
            return Err(envelope_error("unsupported candidate-envelope schema"));
        }
        if self.candidate_sha256 == Digest::ZERO
            || self.parent_candidate_sha256 == Some(Digest::ZERO)
            || self.parent_candidate_sha256 == Some(self.candidate_sha256)
        {
            return Err(envelope_error("candidate lineage identity is invalid"));
        }
        if (self.generation == 0) != self.parent_candidate_sha256.is_none() {
            return Err(envelope_error(
                "generation zero must be parentless and later generations require one exact parent",
            ));
        }
        self.objective.validate("objective")?;
        self.action_schema.validate("action schema")?;
        self.proposer.validate()?;
        if self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(envelope_error(
                "candidate-envelope content identity is invalid",
            ));
        }
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, CandidateEnvelopeError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&unsigned).map_err(|error| {
            envelope_error(format!("cannot encode candidate envelope: {error}"))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.candidate-envelope/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

impl NamedDigest {
    pub fn new(id: impl Into<String>, sha256: Digest) -> Self {
        Self {
            id: id.into(),
            sha256,
        }
    }

    fn validate(&self, label: &str) -> Result<(), CandidateEnvelopeError> {
        validate_id(label, &self.id)?;
        if self.sha256 == Digest::ZERO {
            return Err(envelope_error(format!("{label} digest is zero")));
        }
        Ok(())
    }
}

impl ProposerIdentity {
    fn validate(&self) -> Result<(), CandidateEnvelopeError> {
        validate_id("proposer", &self.id)?;
        if self.version.is_empty()
            || self.version.len() > MAX_VERSION_BYTES
            || self.version.chars().any(char::is_whitespace)
            || self.configuration_sha256 == Digest::ZERO
        {
            return Err(envelope_error("proposer identity is invalid"));
        }
        Ok(())
    }
}

fn validate_id(label: &str, value: &str) -> Result<(), CandidateEnvelopeError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b'/')
        })
    {
        return Err(envelope_error(format!("{label} identifier is invalid")));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateEnvelopeError(String);

impl fmt::Display for CandidateEnvelopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CandidateEnvelopeError {}

fn envelope_error(message: impl Into<String>) -> CandidateEnvelopeError {
    CandidateEnvelopeError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposer(kind: ProposerKind) -> ProposerIdentity {
        ProposerIdentity {
            kind,
            id: match kind {
                ProposerKind::Scripted => "scripted.fixture",
                ProposerKind::Random => "random.uniform",
                ProposerKind::StructuredSearch => "search.mutation",
                ProposerKind::Learned => "learned.tree-fqi",
            }
            .into(),
            version: "1".into(),
            configuration_sha256: Digest([0x44; 32]),
        }
    }

    fn envelope(kind: ProposerKind) -> CandidateEnvelope {
        CandidateEnvelope::build(
            Digest([0x11; 32]),
            Some(Digest([0x22; 32])),
            3,
            NamedDigest::new("talk-to-npc", Digest([0x33; 32])),
            NamedDigest::new("movement/v2", Digest([0x55; 32])),
            42,
            proposer(kind),
        )
        .unwrap()
    }

    #[test]
    fn every_proposer_kind_uses_one_round_tripping_contract() {
        for kind in [
            ProposerKind::Scripted,
            ProposerKind::Random,
            ProposerKind::StructuredSearch,
            ProposerKind::Learned,
        ] {
            let envelope = envelope(kind);
            let encoded = serde_json::to_vec(&envelope).unwrap();
            let decoded: CandidateEnvelope = serde_json::from_slice(&encoded).unwrap();
            decoded.validate().unwrap();
            assert_eq!(decoded, envelope);
        }
    }

    #[test]
    fn exact_parent_objective_action_seed_and_proposer_are_authenticated() {
        let original = envelope(ProposerKind::Learned);
        let mut variants = Vec::new();
        let mut parent = original.clone();
        parent.parent_candidate_sha256 = Some(Digest([0x23; 32]));
        variants.push(parent);
        let mut objective = original.clone();
        objective.objective.sha256 = Digest([0x34; 32]);
        variants.push(objective);
        let mut action = original.clone();
        action.action_schema.sha256 = Digest([0x56; 32]);
        variants.push(action);
        let mut seed = original.clone();
        seed.seed += 1;
        variants.push(seed);
        let mut proposer = original.clone();
        proposer.proposer.kind = ProposerKind::StructuredSearch;
        variants.push(proposer);
        assert!(variants.iter().all(|variant| variant.validate().is_err()));
    }

    #[test]
    fn lineage_and_required_identities_fail_closed() {
        let mut no_parent = envelope(ProposerKind::Random);
        no_parent.parent_candidate_sha256 = None;
        assert!(no_parent.validate().is_err());

        let root = CandidateEnvelope::build(
            Digest([0x11; 32]),
            None,
            0,
            NamedDigest::new("pick-up-rock", Digest([0x33; 32])),
            NamedDigest::new("movement/v2", Digest([0x55; 32])),
            0,
            proposer(ProposerKind::Scripted),
        )
        .unwrap();
        root.validate().unwrap();

        let mut zero_action = root;
        zero_action.action_schema.sha256 = Digest::ZERO;
        assert!(zero_action.validate().is_err());
    }
}
