use dusklight_automation_contracts::artifact::Digest;
use dusklight_control::option_execution::OptionExecution;
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::option_transition::OptionTransitionSample;
use dusklight_learning::option_values::OptionActionDescriptor;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const STATE_GRAPH_SCHEMA_V1: &str = "dusklight-state-graph/v1";
pub const FUTURE_EQUIVALENCE_PROOF_SCHEMA_V1: &str = "dusklight-future-equivalence-proof/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateGraphIdentity {
    pub execution_authority_sha256: Digest,
    /// Validator allowed to admit native future-equivalence proofs. Legacy
    /// graphs decode with zero and therefore keep transpositions disabled.
    #[serde(default)]
    pub future_equivalence_validator_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
}

impl StateGraphIdentity {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.execution_authority_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.root_checkpoint_sha256 == Digest::ZERO
        {
            return Err("state graph identity contains a zero digest");
        }
        Ok(())
    }
}

/// An exact state is route-specific until a future-equivalence proof says two
/// native states have interchangeable futures. Semantic similarity is never
/// sufficient to collapse this identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactStateId {
    pub route_checkpoint_sha256: Digest,
    pub state_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRecord {
    pub route_checkpoint_sha256: Digest,
    pub tape_sha256: Digest,
    pub tape_frames: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBoundaryLocator {
    pub episode_shard_sha256: Digest,
    pub option_offset_ticks: u32,
}

/// Portable route replay is always the fallback. A native episode locator is
/// optional acceleration evidence and never replaces the portable identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestorationLocator {
    pub route: RouteRecord,
    pub native_boundary: Option<NativeBoundaryLocator>,
    pub executable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateGraphNode {
    pub id: ExactStateId,
    pub state: FactSnapshot,
    pub terminal: bool,
    pub root_ticks: u64,
    pub restoration: RestorationLocator,
    pub incoming_segments: BTreeSet<Digest>,
    pub outgoing_segments: BTreeSet<Digest>,
    pub outgoing_expansions: BTreeSet<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedSegment {
    pub identity_sha256: Digest,
    pub parent_expansion_sha256: Digest,
    pub source: ExactStateId,
    pub target: ExactStateId,
    pub option_start_offset_ticks: u32,
    pub option_end_offset_ticks: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "status", deny_unknown_fields)]
pub enum ActionExpansionStatus {
    Untried,
    Leased {
        lease_sha256: Digest,
        expires_at_generation: u64,
    },
    Completed {
        authority: ExpansionEvidenceAuthority,
        route_checkpoint_sha256: Digest,
        evidence: BTreeMap<Digest, CompletedExpansionEvidence>,
    },
    FailedValidation {
        evidence_sha256: Digest,
    },
    Retryable {
        attempts: u32,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedExpansionEvidence {
    pub episode_group: u64,
    pub authority: ExpansionEvidenceAuthority,
    pub transition: Box<OptionTransitionSample>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpansionEvidenceAuthority {
    Executable,
    LearnerEvidenceOnly,
}

/// This is the only place a selected action appears in the graph. Interior
/// segments point back to this expansion; they are observations, not actions.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionExpansion {
    pub identity_sha256: Digest,
    pub source: ExactStateId,
    pub target: Option<ExactStateId>,
    pub action: OptionActionDescriptor,
    pub execution: Option<OptionExecution>,
    pub observed_segments: Vec<Digest>,
    pub status: ActionExpansionStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FutureEquivalenceProof {
    pub schema: String,
    pub proof_sha256: Digest,
    pub left: ExactStateId,
    pub right: ExactStateId,
    pub validator_sha256: Digest,
    pub native_evidence_sha256: Digest,
}

impl FutureEquivalenceProof {
    pub fn new(
        left: ExactStateId,
        right: ExactStateId,
        validator_sha256: Digest,
        native_evidence_sha256: Digest,
    ) -> Result<Self, &'static str> {
        let (left, right) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        let proof_sha256 =
            future_equivalence_proof_sha256(left, right, validator_sha256, native_evidence_sha256);
        let proof = Self {
            schema: FUTURE_EQUIVALENCE_PROOF_SCHEMA_V1.into(),
            proof_sha256,
            left,
            right,
            validator_sha256,
            native_evidence_sha256,
        };
        proof.validate()?;
        Ok(proof)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != FUTURE_EQUIVALENCE_PROOF_SCHEMA_V1
            || self.proof_sha256 == Digest::ZERO
            || self.validator_sha256 == Digest::ZERO
            || self.native_evidence_sha256 == Digest::ZERO
            || self.left >= self.right
            || self.proof_sha256
                != future_equivalence_proof_sha256(
                    self.left,
                    self.right,
                    self.validator_sha256,
                    self.native_evidence_sha256,
                )
        {
            return Err("future-equivalence proof is invalid or non-canonical");
        }
        Ok(())
    }
}

fn future_equivalence_proof_sha256(
    left: ExactStateId,
    right: ExactStateId,
    validator_sha256: Digest,
    native_evidence_sha256: Digest,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(FUTURE_EQUIVALENCE_PROOF_SCHEMA_V1.as_bytes());
    for node in [left, right] {
        hasher.update(node.route_checkpoint_sha256.0);
        hasher.update(node.state_sha256.0);
    }
    hasher.update(validator_sha256.0);
    hasher.update(native_evidence_sha256.0);
    Digest(hasher.finalize().into())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalPath {
    pub terminal: ExactStateId,
    pub route_checkpoint_sha256: Digest,
    pub root_to_terminal_ticks: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpansionAdmission {
    pub expansion_sha256: Digest,
    pub source: ExactStateId,
    pub target: ExactStateId,
    pub inserted_nodes: usize,
    pub inserted_segments: usize,
    pub duplicate: bool,
    pub authority_promoted: bool,
}
