//! Exact phase/provenance sidecars for compact learner transition corpora.

use crate::artifact::Digest;
use crate::episode::{
    EpisodeLineage, EpisodeManifest, EpisodeObjectiveIdentity, EpisodeOutcomeClass,
};
use crate::option_execution::OptionExecution;
use crate::run_terminal::HarnessTerminalReason;
use crate::tape::{InputFrame, InputTape};
use crate::trace::{
    DecodedTrace, TraceChannel, TraceChannelStatus, TracePhase, TraceRecord, TraceSelectedActor,
};
use crate::transition_corpus::{StateReferenceKind, TransitionCorpus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const TRANSITION_EVIDENCE_SCHEMA_V1: &str = "dusklight-transition-evidence/v1";
pub const IMMUTABLE_EPISODE_SCHEMA_V1: &str = "dusklight-immutable-episode/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionEvidenceBundle {
    pub schema: String,
    pub corpus_sha256: Digest,
    pub trace_sha256: Digest,
    pub tape_sha256: Digest,
    pub event_side_table: Vec<EventFactsEvidence>,
    pub entity_side_table: Vec<EntityFactsEvidence>,
    pub transitions: Vec<AlignedTransitionEvidence>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AlignedTransitionEvidence {
    pub corpus_transition_index: u64,
    pub pre_action: ObservationBoundaryEvidence,
    pub action: ExactActionEvidence,
    pub duration_ticks: u32,
    pub post_action: ObservationBoundaryEvidence,
    pub event: SideTableTransitionEvidence,
    pub entities: SideTableTransitionEvidence,
    pub reward: RewardEvidence,
    pub predicate: PredicateTransitionEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason: Option<TerminalReasonEvidence>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationBoundaryEvidence {
    pub reference_kind: EvidenceReferenceKind,
    pub reference_sha256: Digest,
    pub boundary_index: u64,
    pub simulation_tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tape_frame: Option<u64>,
    pub phase: EvidencePhase,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceReferenceKind {
    Boundary,
    Snapshot,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidencePhase {
    PreInput,
    PostSimulation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExactActionEvidence {
    PadFrame { tape_frame: u64, frame: InputFrame },
    Option { execution: OptionExecution },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceAvailability {
    Present,
    Absent,
    Unavailable,
    Truncated,
    Unrequested,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventFactsEvidence {
    pub availability: EvidenceAvailability,
    pub running: bool,
    pub event_id: i16,
    pub mode: u8,
    pub status: u8,
    pub map_tool_id: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_hash: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SideTableTransitionEvidence {
    pub pre_action: u32,
    pub post_action: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EntityFactsEvidence {
    pub availability: EvidenceAvailability,
    pub observed_count: u32,
    pub truncated: bool,
    pub actors: Vec<EntityActorEvidence>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EntityActorEvidence {
    pub session_process_id: u32,
    pub actor_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    pub health: i16,
    pub status: u32,
    pub position: [f32; 3],
    pub current_angle: [i16; 3],
    pub shape_angle: [i16; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RewardEvidence {
    pub training_reward: f32,
    pub components: Vec<RewardComponentEvidence>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RewardComponentEvidence {
    pub name: String,
    pub value: f32,
    pub source_fact: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateFactsEvidence {
    pub availability: EvidenceAvailability,
    pub configured: bool,
    pub reached: bool,
    pub authored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_name_hash: Option<u32>,
    pub requested_count: u16,
    pub hit_count: u16,
    pub stable_ticks: u16,
    pub consecutive_ticks: u16,
    pub sequence_steps: u8,
    pub sequence_next_step: u8,
    pub sequence_within_ticks: u16,
    pub sequence_elapsed_ticks: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hit_tick: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateTransitionEvidence {
    pub pre_action: PredicateFactsEvidence,
    pub post_action: PredicateFactsEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalReasonEvidence {
    ObjectiveReached,
    OptionEnded { reason: String },
    Crash { reason: String },
    Timeout { stalled_millis: u64 },
    Truncated { source: String },
    DeclaredExtractionBoundary,
}

pub struct TransitionEvidenceBuild<'a> {
    pub corpus: &'a TransitionCorpus,
    pub trace: &'a DecodedTrace,
    pub tape: &'a InputTape,
    pub trace_sha256: Digest,
    pub tape_sha256: Digest,
    pub start_tape_frame: u64,
    pub end_tape_frame: u64,
    pub terminal_reason: Option<TerminalReasonEvidence>,
}

/// One content-sealed episode view. It resolves the manifest, compact corpus,
/// and sparse transition-evidence joins once so consumers cannot shift an
/// action or reward onto the wrong observation boundary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutableEpisodeArtifact {
    pub schema: String,
    pub content_sha256: Digest,
    pub episode_sha256: Digest,
    pub objective: EpisodeObjectiveIdentity,
    pub lineage: EpisodeLineage,
    pub terminal: HarnessTerminalReason,
    pub terminal_detail: String,
    pub realized_tape_sha256: Digest,
    pub gameplay_trace_sha256: Digest,
    pub transition_corpus_sha256: Digest,
    pub transition_evidence_sha256: Digest,
    pub steps: Vec<ImmutableEpisodeStep>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutableEpisodeStep {
    pub index: u64,
    pub pre_input_state: ImmutableEpisodeState,
    pub consumed_action: ExactActionEvidence,
    pub duration_ticks: u32,
    pub post_simulation_state: ImmutableEpisodeState,
    pub objective: PredicateTransitionEvidence,
    pub reward: RewardEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason: Option<TerminalReasonEvidence>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutableEpisodeState {
    /// The canonical post-simulation boundary that becomes the immutable
    /// state observed immediately before the next input is consumed.
    pub boundary: ObservationBoundaryEvidence,
    pub features: Vec<f32>,
    pub event: EventFactsEvidence,
    pub entities: EntityFactsEvidence,
}

pub struct ImmutableEpisodeBuild<'a> {
    pub manifest: &'a EpisodeManifest,
    pub corpus: &'a TransitionCorpus,
    pub evidence: &'a TransitionEvidenceBundle,
    pub transition_evidence_sha256: Digest,
    pub terminal: HarnessTerminalReason,
    pub terminal_detail: &'a str,
}

impl ImmutableEpisodeArtifact {
    pub fn build(input: ImmutableEpisodeBuild<'_>) -> Result<Self, TransitionEvidenceError> {
        input
            .manifest
            .validate(input.corpus)
            .map_err(|error| TransitionEvidenceError::new(error.to_string()))?;
        input.evidence.validate(input.corpus)?;
        if input.transition_evidence_sha256 == Digest::ZERO
            || input.manifest.artifacts.transition_evidence_sha256
                != input.transition_evidence_sha256
            || input.manifest.artifacts.absolute_tape_sha256 != input.evidence.tape_sha256
            || input.manifest.artifacts.gameplay_trace_sha256 != input.evidence.trace_sha256
            || input.manifest.artifacts.transition_corpus_sha256 != input.evidence.corpus_sha256
            || input.manifest.outcome.class != outcome_class(input.terminal)
        {
            return Err(TransitionEvidenceError::new(
                "immutable episode source identities do not agree",
            ));
        }
        let mut steps = Vec::with_capacity(input.corpus.transitions.len());
        for (transition, evidence) in input
            .corpus
            .transitions
            .iter()
            .zip(&input.evidence.transitions)
        {
            steps.push(ImmutableEpisodeStep {
                index: evidence.corpus_transition_index,
                pre_input_state: ImmutableEpisodeState {
                    boundary: evidence.pre_action.clone(),
                    features: transition.state.clone(),
                    event: input.evidence.event_side_table[evidence.event.pre_action as usize]
                        .clone(),
                    entities: input.evidence.entity_side_table
                        [evidence.entities.pre_action as usize]
                        .clone(),
                },
                consumed_action: evidence.action.clone(),
                duration_ticks: evidence.duration_ticks,
                post_simulation_state: ImmutableEpisodeState {
                    boundary: evidence.post_action.clone(),
                    features: transition.next_state.clone(),
                    event: input.evidence.event_side_table[evidence.event.post_action as usize]
                        .clone(),
                    entities: input.evidence.entity_side_table
                        [evidence.entities.post_action as usize]
                        .clone(),
                },
                objective: evidence.predicate.clone(),
                reward: evidence.reward.clone(),
                terminal_reason: evidence.terminal_reason.clone(),
            });
        }
        let mut artifact = Self {
            schema: IMMUTABLE_EPISODE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            episode_sha256: input.manifest.episode_sha256,
            objective: input.manifest.objective.clone(),
            lineage: input.manifest.lineage.clone(),
            terminal: input.terminal,
            terminal_detail: input.terminal_detail.into(),
            realized_tape_sha256: input.evidence.tape_sha256,
            gameplay_trace_sha256: input.evidence.trace_sha256,
            transition_corpus_sha256: input.evidence.corpus_sha256,
            transition_evidence_sha256: input.transition_evidence_sha256,
            steps,
        };
        artifact.content_sha256 = artifact.compute_content_sha256()?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn validate(&self) -> Result<(), TransitionEvidenceError> {
        if self.schema != IMMUTABLE_EPISODE_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.episode_sha256 == Digest::ZERO
            || self.objective.digest == Digest::ZERO
            || self.realized_tape_sha256 == Digest::ZERO
            || self.gameplay_trace_sha256 == Digest::ZERO
            || self.transition_corpus_sha256 == Digest::ZERO
            || self.transition_evidence_sha256 == Digest::ZERO
            || self.terminal_detail.is_empty()
            || self.terminal_detail.len() > 8_192
            || self
                .terminal_detail
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
            || self.steps.is_empty()
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(TransitionEvidenceError::new(
                "immutable episode identity or required field is invalid",
            ));
        }
        for (index, step) in self.steps.iter().enumerate() {
            if step.index != index as u64
                || step.duration_ticks == 0
                || step.pre_input_state.boundary.phase != EvidencePhase::PostSimulation
                || step.post_simulation_state.boundary.phase != EvidencePhase::PostSimulation
                || step.post_simulation_state.boundary.simulation_tick
                    != step.pre_input_state.boundary.simulation_tick
                        + u64::from(step.duration_ticks)
                || step.pre_input_state.features.is_empty()
                || step.pre_input_state.features.len() != step.post_simulation_state.features.len()
                || !canonical_features(&step.pre_input_state.features)
                || !canonical_features(&step.post_simulation_state.features)
                || step.reward.components.is_empty()
                || !step.reward.training_reward.is_finite()
                || step
                    .reward
                    .components
                    .iter()
                    .any(|component| !component.value.is_finite())
            {
                return Err(TransitionEvidenceError::new(
                    "immutable episode step alignment is invalid",
                ));
            }
            validate_exact_action(
                &step.consumed_action,
                step.duration_ticks,
                self.realized_tape_sha256,
            )?;
            if let ExactActionEvidence::PadFrame { tape_frame, .. } = &step.consumed_action
                && (step.post_simulation_state.boundary.tape_frame != Some(*tape_frame)
                    || step
                        .pre_input_state
                        .boundary
                        .tape_frame
                        .and_then(|frame| frame.checked_add(1))
                        != Some(*tape_frame))
            {
                return Err(TransitionEvidenceError::new(
                    "immutable episode action is shifted from its state boundaries",
                ));
            }
            if let Some(next) = self.steps.get(index + 1)
                && (step.post_simulation_state.boundary.reference_sha256
                    != next.pre_input_state.boundary.reference_sha256
                    || step.post_simulation_state.features != next.pre_input_state.features)
            {
                return Err(TransitionEvidenceError::new(
                    "immutable episode states do not form one ordered chain",
                ));
            }
            if index + 1 != self.steps.len() && step.terminal_reason.is_some() {
                return Err(TransitionEvidenceError::new(
                    "immutable episode terminal reason appears before the final step",
                ));
            }
        }
        let last = self.steps.last().expect("nonempty was checked");
        if self.terminal == HarnessTerminalReason::Reached
            && (!last.objective.post_action.reached
                || last.terminal_reason != Some(TerminalReasonEvidence::ObjectiveReached))
        {
            return Err(TransitionEvidenceError::new(
                "reached episode lacks aligned objective terminal evidence",
            ));
        }
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, TransitionEvidenceError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&unsigned)
            .map_err(|error| TransitionEvidenceError::new(error.to_string()))?;
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest as _;
        hasher.update(b"dusklight.immutable-episode/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn canonical_features(features: &[f32]) -> bool {
    features
        .iter()
        .all(|value| value.is_finite() && value.to_bits() != (-0.0_f32).to_bits())
}

fn outcome_class(terminal: HarnessTerminalReason) -> EpisodeOutcomeClass {
    match terminal {
        HarnessTerminalReason::Reached => EpisodeOutcomeClass::Successful,
        HarnessTerminalReason::Exhausted
        | HarnessTerminalReason::Impossible
        | HarnessTerminalReason::TargetLost
        | HarnessTerminalReason::Rejected
        | HarnessTerminalReason::Cancelled => EpisodeOutcomeClass::Failed,
        HarnessTerminalReason::Unsupported | HarnessTerminalReason::CapabilityMismatch => {
            EpisodeOutcomeClass::Unsupported
        }
        HarnessTerminalReason::HostTimeout | HarnessTerminalReason::Hung => {
            EpisodeOutcomeClass::TimedOut
        }
        HarnessTerminalReason::WorkerCrashed | HarnessTerminalReason::GameCrashed => {
            EpisodeOutcomeClass::Crashed
        }
        HarnessTerminalReason::IdentityMismatch
        | HarnessTerminalReason::ProtocolFailure
        | HarnessTerminalReason::Nondeterministic => EpisodeOutcomeClass::Desynced,
    }
}

impl TransitionEvidenceBundle {
    pub fn build(input: TransitionEvidenceBuild<'_>) -> Result<Self, TransitionEvidenceError> {
        let expected_count = input
            .end_tape_frame
            .checked_sub(input.start_tape_frame)
            .and_then(|span| span.checked_add(1))
            .ok_or_else(|| TransitionEvidenceError::new("invalid evidence frame range"))?;
        if expected_count as usize != input.corpus.transitions.len() {
            return Err(TransitionEvidenceError::new(
                "evidence frame range does not match corpus transition count",
            ));
        }
        let mut by_frame = BTreeMap::new();
        for record in &input.trace.records {
            let Some(frame) = record.tape_frame else {
                continue;
            };
            if by_frame.insert(frame, record).is_some() {
                return Err(TransitionEvidenceError::new(
                    "trace contains duplicate tape-frame observations",
                ));
            }
        }
        let mut transitions = Vec::with_capacity(input.corpus.transitions.len());
        let mut event_side_table = Vec::new();
        let mut event_indices = BTreeMap::new();
        let mut entity_side_table = Vec::new();
        let mut entity_indices = BTreeMap::new();
        for (index, transition) in input.corpus.transitions.iter().enumerate() {
            let action_frame = input.start_tape_frame + index as u64;
            let pre_frame = action_frame
                .checked_sub(1)
                .ok_or_else(|| TransitionEvidenceError::new("action has no pre-state frame"))?;
            let pre = by_frame
                .get(&pre_frame)
                .copied()
                .ok_or_else(|| TransitionEvidenceError::new("missing pre-action trace record"))?;
            let post = by_frame
                .get(&action_frame)
                .copied()
                .ok_or_else(|| TransitionEvidenceError::new("missing post-action trace record"))?;
            validate_boundary(pre)?;
            validate_boundary(post)?;
            if post.simulation_tick != pre.simulation_tick + u64::from(transition.duration_ticks) {
                return Err(TransitionEvidenceError::new(
                    "trace tick distance disagrees with transition duration",
                ));
            }
            let tape_index = usize::try_from(action_frame)
                .map_err(|_| TransitionEvidenceError::new("tape frame index overflow"))?;
            let frame = input
                .tape
                .frames
                .get(tape_index)
                .cloned()
                .ok_or_else(|| TransitionEvidenceError::new("action frame exceeds tape"))?;
            let terminal_reason = if transition.terminal {
                if index + 1 != input.corpus.transitions.len() {
                    return Err(TransitionEvidenceError::new(
                        "terminal transition must end the extracted episode",
                    ));
                }
                Some(input.terminal_reason.clone().ok_or_else(|| {
                    TransitionEvidenceError::new("terminal transition lacks an exact reason")
                })?)
            } else {
                if index + 1 == input.corpus.transitions.len() && input.terminal_reason.is_some() {
                    return Err(TransitionEvidenceError::new(
                        "nonterminal extraction supplied a terminal reason",
                    ));
                }
                None
            };
            let event = SideTableTransitionEvidence {
                pre_action: intern_side_table(
                    &mut event_side_table,
                    &mut event_indices,
                    event_facts(pre),
                )?,
                post_action: intern_side_table(
                    &mut event_side_table,
                    &mut event_indices,
                    event_facts(post),
                )?,
            };
            let entities = SideTableTransitionEvidence {
                pre_action: intern_side_table(
                    &mut entity_side_table,
                    &mut entity_indices,
                    entity_facts(pre),
                )?,
                post_action: intern_side_table(
                    &mut entity_side_table,
                    &mut entity_indices,
                    entity_facts(post),
                )?,
            };
            transitions.push(AlignedTransitionEvidence {
                corpus_transition_index: index as u64,
                pre_action: boundary(pre, transition.source),
                action: ExactActionEvidence::PadFrame {
                    tape_frame: action_frame,
                    frame,
                },
                duration_ticks: transition.duration_ticks,
                post_action: boundary(post, transition.next),
                event,
                entities,
                reward: RewardEvidence {
                    training_reward: transition.reward,
                    components: vec![RewardComponentEvidence {
                        name: "base_reward".into(),
                        value: transition.reward,
                        source_fact: "transition_corpus.reward".into(),
                    }],
                },
                predicate: PredicateTransitionEvidence {
                    pre_action: predicate_facts(pre),
                    post_action: predicate_facts(post),
                },
                terminal_reason,
            });
        }
        let bundle = Self {
            schema: TRANSITION_EVIDENCE_SCHEMA_V1.into(),
            corpus_sha256: input
                .corpus
                .content_digest()
                .map_err(|error| TransitionEvidenceError::new(error.to_string()))?,
            trace_sha256: input.trace_sha256,
            tape_sha256: input.tape_sha256,
            event_side_table,
            entity_side_table,
            transitions,
        };
        bundle.validate(input.corpus)?;
        Ok(bundle)
    }

    pub fn validate(&self, corpus: &TransitionCorpus) -> Result<(), TransitionEvidenceError> {
        if self.schema != TRANSITION_EVIDENCE_SCHEMA_V1
            || self.trace_sha256 == Digest::ZERO
            || self.tape_sha256 == Digest::ZERO
            || self.corpus_sha256
                != corpus
                    .content_digest()
                    .map_err(|error| TransitionEvidenceError::new(error.to_string()))?
            || self.transitions.len() != corpus.transitions.len()
        {
            return Err(TransitionEvidenceError::new(
                "transition evidence identity or count mismatch",
            ));
        }
        validate_side_table(&self.event_side_table, "event")?;
        validate_side_table(&self.entity_side_table, "entity")?;
        for facts in &self.entity_side_table {
            validate_entity_facts(facts)?;
        }
        for (index, (evidence, transition)) in
            self.transitions.iter().zip(&corpus.transitions).enumerate()
        {
            if evidence.corpus_transition_index != index as u64
                || evidence.duration_ticks != transition.duration_ticks
                || evidence.pre_action.reference_kind != reference_kind(transition.source.kind)
                || evidence.post_action.reference_kind != reference_kind(transition.next.kind)
                || evidence.pre_action.reference_sha256 != transition.source.digest
                || evidence.post_action.reference_sha256 != transition.next.digest
                || evidence.pre_action.phase != EvidencePhase::PostSimulation
                || evidence.post_action.phase != EvidencePhase::PostSimulation
                || evidence.pre_action.boundary_index != evidence.pre_action.simulation_tick + 1
                || evidence.post_action.boundary_index != evidence.post_action.simulation_tick + 1
                || evidence.post_action.simulation_tick
                    != evidence.pre_action.simulation_tick + u64::from(evidence.duration_ticks)
                || evidence.reward.training_reward.to_bits() != transition.reward.to_bits()
                || evidence.reward.components.is_empty()
                || transition.terminal != evidence.terminal_reason.is_some()
                || !evidence.reward.training_reward.is_finite()
                || evidence
                    .reward
                    .components
                    .iter()
                    .any(|component| !component.value.is_finite())
            {
                return Err(TransitionEvidenceError::new(
                    "transition evidence does not match compact corpus record",
                ));
            }
            validate_side_table_reference(evidence.event, self.event_side_table.len(), "event")?;
            validate_side_table_reference(
                evidence.entities,
                self.entity_side_table.len(),
                "entity",
            )?;
            validate_exact_action(&evidence.action, evidence.duration_ticks, self.tape_sha256)?;
            if let ExactActionEvidence::PadFrame { tape_frame, .. } = &evidence.action {
                if evidence.post_action.tape_frame != Some(*tape_frame)
                    || evidence
                        .pre_action
                        .tape_frame
                        .and_then(|frame| frame.checked_add(1))
                        != Some(*tape_frame)
                {
                    return Err(TransitionEvidenceError::new(
                        "pad-frame action is not aligned to its observation boundaries",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn intern_side_table<T: Serialize>(
    table: &mut Vec<T>,
    indices: &mut BTreeMap<Vec<u8>, u32>,
    value: T,
) -> Result<u32, TransitionEvidenceError> {
    let key = serde_json::to_vec(&value)
        .map_err(|error| TransitionEvidenceError::new(error.to_string()))?;
    if let Some(index) = indices.get(&key) {
        return Ok(*index);
    }
    let index = u32::try_from(table.len())
        .map_err(|_| TransitionEvidenceError::new("sparse side table exceeds u32 indices"))?;
    table.push(value);
    indices.insert(key, index);
    Ok(index)
}

fn validate_side_table<T: Serialize>(
    table: &[T],
    name: &str,
) -> Result<(), TransitionEvidenceError> {
    let mut unique = BTreeSet::new();
    for value in table {
        let key = serde_json::to_vec(value)
            .map_err(|error| TransitionEvidenceError::new(error.to_string()))?;
        if !unique.insert(key) {
            return Err(TransitionEvidenceError::new(format!(
                "{name} side table contains duplicate facts"
            )));
        }
    }
    Ok(())
}

fn validate_side_table_reference(
    reference: SideTableTransitionEvidence,
    table_len: usize,
    name: &str,
) -> Result<(), TransitionEvidenceError> {
    if usize::try_from(reference.pre_action).map_or(true, |index| index >= table_len)
        || usize::try_from(reference.post_action).map_or(true, |index| index >= table_len)
    {
        return Err(TransitionEvidenceError::new(format!(
            "transition references a missing {name} side-table entry"
        )));
    }
    Ok(())
}

fn validate_entity_facts(facts: &EntityFactsEvidence) -> Result<(), TransitionEvidenceError> {
    if facts.actors.len() > 16
        || facts.observed_count < facts.actors.len() as u32
        || facts.truncated != (facts.observed_count > facts.actors.len() as u32)
        || facts
            .actors
            .windows(2)
            .any(|actors| actors[0].session_process_id >= actors[1].session_process_id)
        || facts
            .actors
            .iter()
            .flat_map(|actor| actor.position)
            .any(|component| !component.is_finite())
        || (facts.availability != EvidenceAvailability::Present
            && (facts.observed_count != 0 || facts.truncated || !facts.actors.is_empty()))
    {
        return Err(TransitionEvidenceError::new(
            "entity side-table entry is inconsistent or noncanonical",
        ));
    }
    Ok(())
}

fn reference_kind(kind: StateReferenceKind) -> EvidenceReferenceKind {
    match kind {
        StateReferenceKind::Boundary => EvidenceReferenceKind::Boundary,
        StateReferenceKind::Snapshot => EvidenceReferenceKind::Snapshot,
    }
}

fn boundary(
    record: &TraceRecord,
    reference: crate::transition_corpus::StateReference,
) -> ObservationBoundaryEvidence {
    ObservationBoundaryEvidence {
        reference_kind: reference_kind(reference.kind),
        reference_sha256: reference.digest,
        boundary_index: record.boundary_index,
        simulation_tick: record.simulation_tick,
        tape_frame: record.tape_frame,
        phase: match record.observation_phase {
            TracePhase::PreInput => EvidencePhase::PreInput,
            TracePhase::PostSimulation => EvidencePhase::PostSimulation,
        },
    }
}

fn validate_boundary(record: &TraceRecord) -> Result<(), TransitionEvidenceError> {
    if record.observation_phase != TracePhase::PostSimulation
        || record.boundary_index != record.simulation_tick + 1
    {
        Err(TransitionEvidenceError::new(
            "transition evidence requires canonical post-simulation boundaries",
        ))
    } else {
        Ok(())
    }
}

fn availability(record: &TraceRecord, channel: TraceChannel) -> EvidenceAvailability {
    match record.channel_status.get(&channel) {
        Some(TraceChannelStatus::Present) => EvidenceAvailability::Present,
        Some(TraceChannelStatus::Absent) => EvidenceAvailability::Absent,
        Some(TraceChannelStatus::Unavailable) => EvidenceAvailability::Unavailable,
        Some(TraceChannelStatus::Truncated) => EvidenceAvailability::Truncated,
        Some(TraceChannelStatus::NotSampled) | None => EvidenceAvailability::Unrequested,
    }
}

fn event_facts(record: &TraceRecord) -> EventFactsEvidence {
    EventFactsEvidence {
        availability: availability(record, TraceChannel::Event),
        running: record.event_running(),
        event_id: record.event_id,
        mode: record.event_mode,
        status: record.event_status,
        map_tool_id: record.event_map_tool_id,
        name_hash: record
            .event_name_hash_present
            .then_some(record.event_name_hash),
    }
}

fn entity_facts(record: &TraceRecord) -> EntityFactsEvidence {
    let availability = availability(record, TraceChannel::SelectedActors);
    let Some(selected) = &record.selected_actors else {
        return EntityFactsEvidence {
            availability,
            observed_count: 0,
            truncated: false,
            actors: Vec::new(),
        };
    };
    EntityFactsEvidence {
        availability,
        observed_count: selected.observed_count,
        truncated: selected.truncated,
        actors: selected.actors.iter().map(entity_actor).collect(),
    }
}

fn entity_actor(actor: &TraceSelectedActor) -> EntityActorEvidence {
    EntityActorEvidence {
        session_process_id: actor.session_process_id,
        actor_name: actor.actor_name,
        set_id: actor.set_id,
        home_room: actor.home_room,
        current_room: actor.current_room,
        health: actor.health,
        status: actor.status,
        position: actor.position,
        current_angle: actor.current_angle,
        shape_angle: actor.shape_angle,
    }
}

fn predicate_facts(record: &TraceRecord) -> PredicateFactsEvidence {
    let availability = availability(record, TraceChannel::GoalProgress);
    if let Some(goal) = &record.goal_progress {
        PredicateFactsEvidence {
            availability,
            configured: goal.configured,
            reached: goal.reached,
            authored: goal.authored,
            goal_name_hash: goal.goal_name_hash,
            requested_count: goal.requested_count,
            hit_count: goal.hit_count,
            stable_ticks: goal.stable_ticks,
            consecutive_ticks: goal.consecutive_ticks,
            sequence_steps: goal.sequence_steps,
            sequence_next_step: goal.sequence_next_step,
            sequence_within_ticks: goal.sequence_within_ticks,
            sequence_elapsed_ticks: goal.sequence_elapsed_ticks,
            first_hit_tick: goal.first_hit_tick,
        }
    } else {
        PredicateFactsEvidence {
            availability,
            configured: false,
            reached: false,
            authored: false,
            goal_name_hash: None,
            requested_count: 0,
            hit_count: 0,
            stable_ticks: 0,
            consecutive_ticks: 0,
            sequence_steps: 0,
            sequence_next_step: 0,
            sequence_within_ticks: 0,
            sequence_elapsed_ticks: 0,
            first_hit_tick: None,
        }
    }
}

fn validate_exact_action(
    action: &ExactActionEvidence,
    duration_ticks: u32,
    tape_sha256: Digest,
) -> Result<(), TransitionEvidenceError> {
    match action {
        ExactActionEvidence::PadFrame { frame, .. } => {
            if duration_ticks != 1 || frame.owned_ports & !0x0f != 0 {
                return Err(TransitionEvidenceError::new(
                    "pad-frame evidence has invalid duration or ownership",
                ));
            }
        }
        ExactActionEvidence::Option { execution } => {
            execution
                .validate()
                .map_err(|error| TransitionEvidenceError::new(error.to_string()))?;
            if execution.duration.realized_ticks != duration_ticks
                || execution.tape_sha256 != tape_sha256
            {
                return Err(TransitionEvidenceError::new(
                    "option evidence duration or tape identity mismatch",
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct TransitionEvidenceError(String);

impl TransitionEvidenceError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for TransitionEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TransitionEvidenceError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::{
        EPISODE_CONTEXT_SCHEMA_V1, EpisodeContext, EpisodeManifestBuild, EpisodeOutcome,
        EpisodeOutcomeClass, EpisodeProducerIdentity, EpisodeProducerKind, EpisodeSeed,
        RunBuildIdentity,
    };
    use crate::tape::RawPadState;
    use crate::trace::{TraceGoalProgress, TraceSelectedActor, TraceSelectedActors};
    use crate::transition_corpus::{MacroAction, StateReference, Transition};
    use sha2::Digest as ShaDigest;

    fn reference(byte: u8) -> StateReference {
        StateReference {
            kind: StateReferenceKind::Boundary,
            digest: Digest([byte; 32]),
        }
    }

    fn fixture() -> (TransitionCorpus, DecodedTrace, InputTape) {
        let corpus = TransitionCorpus::new(
            Digest([0x11; 32]),
            Digest([0x22; 32]),
            1,
            vec![Transition {
                source: reference(0x33),
                state: vec![1.0],
                action: MacroAction {
                    action_id: 7,
                    macro_kind: 1,
                    parameters: vec![0x100, 42, -17],
                },
                duration_ticks: 1,
                reward: 2.5,
                next: reference(0x44),
                next_state: vec![2.0],
                terminal: true,
            }],
        )
        .unwrap();

        let mut pre = TraceRecord {
            boundary_index: 1,
            simulation_tick: 0,
            tape_frame: Some(0),
            event_id: 9,
            event_mode: 2,
            event_status: 3,
            event_map_tool_id: 4,
            event_name_hash: 0xdead_beef,
            event_name_hash_present: true,
            ..TraceRecord::default()
        };
        pre.channel_status
            .insert(TraceChannel::Event, TraceChannelStatus::Present);
        pre.channel_status
            .insert(TraceChannel::GoalProgress, TraceChannelStatus::Present);
        pre.channel_status
            .insert(TraceChannel::SelectedActors, TraceChannelStatus::Present);
        pre.goal_progress = Some(TraceGoalProgress {
            configured: true,
            reached: false,
            authored: true,
            goal_name_hash: Some(0x1234_5678),
            requested_count: 2,
            hit_count: 1,
            stable_ticks: 0,
            consecutive_ticks: 0,
            sequence_steps: 2,
            sequence_next_step: 1,
            sequence_within_ticks: 30,
            sequence_elapsed_ticks: 4,
            first_hit_tick: Some(0),
        });
        pre.selected_actors = Some(TraceSelectedActors {
            observed_count: 1,
            truncated: false,
            actors: vec![TraceSelectedActor {
                session_process_id: 17,
                actor_name: 0x123,
                set_id: 4,
                home_room: 1,
                current_room: 1,
                health: 3,
                status: 0x80,
                position: [1.0, 2.0, 3.0],
                current_angle: [0, 0x1000, 0],
                shape_angle: [0, 0x1000, 0],
            }],
        });

        let mut post = pre.clone();
        post.boundary_index = 2;
        post.simulation_tick = 1;
        post.tape_frame = Some(1);
        post.flags = 1 << 2;
        post.goal_progress.as_mut().unwrap().reached = true;
        post.goal_progress.as_mut().unwrap().hit_count = 2;
        post.goal_progress.as_mut().unwrap().sequence_next_step = 2;

        let trace = DecodedTrace {
            version: 2,
            boot: Default::default(),
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: TraceChannel::Event.bit()
                | TraceChannel::GoalProgress.bit()
                | TraceChannel::SelectedActors.bit(),
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records: vec![pre, post],
        };

        let mut action = InputFrame {
            owned_ports: 0b0011,
            ..InputFrame::default()
        };
        action.pads[0] = RawPadState {
            buttons: 0x100,
            stick_x: 42,
            stick_y: -17,
            ..RawPadState::default()
        };
        action.pads[1].trigger_right = 127;
        let tape = InputTape {
            frames: vec![InputFrame::default(), action],
            ..InputTape::default()
        };
        (corpus, trace, tape)
    }

    fn build<'a>(
        corpus: &'a TransitionCorpus,
        trace: &'a DecodedTrace,
        tape: &'a InputTape,
        terminal_reason: Option<TerminalReasonEvidence>,
    ) -> Result<TransitionEvidenceBundle, TransitionEvidenceError> {
        TransitionEvidenceBundle::build(TransitionEvidenceBuild {
            corpus,
            trace,
            tape,
            trace_sha256: Digest([0x55; 32]),
            tape_sha256: Digest([0x66; 32]),
            start_tape_frame: 1,
            end_tape_frame: 1,
            terminal_reason,
        })
    }

    fn manifest(corpus: &TransitionCorpus, evidence_sha256: Digest) -> EpisodeManifest {
        let context = EpisodeContext {
            schema: EPISODE_CONTEXT_SCHEMA_V1.into(),
            run_identity: None,
            run_build: RunBuildIdentity {
                executable_sha256: Digest([0x77; 32]),
                dusklight_commit: Some("abc123".into()),
                aurora_commit: Some("def456".into()),
                target: Some("aarch64-apple-darwin".into()),
                profile: Some("debug".into()),
                feature_digest: None,
            },
            objective: EpisodeObjectiveIdentity {
                id: "test-objective".into(),
                digest: Digest([0x88; 32]),
            },
            producer: EpisodeProducerIdentity {
                kind: EpisodeProducerKind::Evolution,
                name: "huntctl".into(),
                version: "1".into(),
            },
            seed: EpisodeSeed::Deterministic { value: 42 },
            worker_id: "worker-0".into(),
            lineage: EpisodeLineage {
                candidate_id: Some("candidate-a".into()),
                parent_candidate_id: Some("candidate-parent".into()),
                generation: 2,
                intervention: None,
            },
            outcome: EpisodeOutcome {
                class: EpisodeOutcomeClass::Successful,
                reason: "objective reached".into(),
            },
        };
        EpisodeManifest::build(EpisodeManifestBuild {
            context: &context,
            boot: &Default::default(),
            corpus,
            query_view_id: "movement-state/v2",
            tape_sha256: Digest([0x66; 32]),
            trace_sha256: Digest([0x55; 32]),
            transition_evidence_sha256: evidence_sha256,
        })
        .unwrap()
    }

    #[test]
    fn records_exact_phase_aligned_transition_evidence() {
        let (corpus, trace, tape) = fixture();
        let bundle = build(
            &corpus,
            &trace,
            &tape,
            Some(TerminalReasonEvidence::ObjectiveReached),
        )
        .unwrap();
        let evidence = &bundle.transitions[0];
        assert_eq!(evidence.pre_action.simulation_tick, 0);
        assert_eq!(evidence.post_action.simulation_tick, 1);
        assert!(!bundle.event_side_table[evidence.event.pre_action as usize].running);
        assert!(bundle.event_side_table[evidence.event.post_action as usize].running);
        assert_eq!(bundle.event_side_table.len(), 2);
        assert_eq!(bundle.entity_side_table.len(), 1);
        assert_eq!(evidence.entities.pre_action, evidence.entities.post_action);
        assert_eq!(bundle.entity_side_table[0].actors[0].session_process_id, 17);
        assert!(!evidence.predicate.pre_action.reached);
        assert!(evidence.predicate.post_action.reached);
        assert_eq!(evidence.reward.components[0].value, 2.5);
        assert_eq!(
            evidence.terminal_reason,
            Some(TerminalReasonEvidence::ObjectiveReached)
        );
        let ExactActionEvidence::PadFrame { tape_frame, frame } = &evidence.action else {
            panic!("expected exact pad-frame action")
        };
        assert_eq!(*tape_frame, 1);
        assert_eq!(frame.owned_ports, 0b0011);
        assert_eq!(frame.pads[0].stick_x, 42);
        assert_eq!(frame.pads[1].trigger_right, 127);

        let encoded = serde_json::to_vec(&bundle).unwrap();
        let decoded: TransitionEvidenceBundle = serde_json::from_slice(&encoded).unwrap();
        decoded.validate(&corpus).unwrap();
        assert_eq!(decoded, bundle);
    }

    #[test]
    fn seals_one_self_contained_aligned_episode() {
        let (corpus, trace, tape) = fixture();
        let evidence = build(
            &corpus,
            &trace,
            &tape,
            Some(TerminalReasonEvidence::ObjectiveReached),
        )
        .unwrap();
        let evidence_sha256 =
            Digest(sha2::Sha256::digest(serde_json::to_vec_pretty(&evidence).unwrap()).into());
        let manifest = manifest(&corpus, evidence_sha256);
        let artifact = ImmutableEpisodeArtifact::build(ImmutableEpisodeBuild {
            manifest: &manifest,
            corpus: &corpus,
            evidence: &evidence,
            transition_evidence_sha256: evidence_sha256,
            terminal: HarnessTerminalReason::Reached,
            terminal_detail: "objective reached with complete replay proof",
        })
        .unwrap();
        assert_eq!(artifact.steps.len(), 1);
        assert_eq!(artifact.steps[0].pre_input_state.features, [1.0]);
        assert_eq!(artifact.steps[0].post_simulation_state.features, [2.0]);
        assert_eq!(
            artifact.lineage.candidate_id.as_deref(),
            Some("candidate-a")
        );
        assert_eq!(artifact.realized_tape_sha256, Digest([0x66; 32]));

        let encoded = serde_json::to_vec(&artifact).unwrap();
        let decoded: ImmutableEpisodeArtifact = serde_json::from_slice(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, artifact);
    }

    #[test]
    fn rejects_tampering_and_one_tick_action_shifts() {
        let (corpus, trace, tape) = fixture();
        let evidence = build(
            &corpus,
            &trace,
            &tape,
            Some(TerminalReasonEvidence::ObjectiveReached),
        )
        .unwrap();
        let evidence_sha256 =
            Digest(sha2::Sha256::digest(serde_json::to_vec_pretty(&evidence).unwrap()).into());
        let manifest = manifest(&corpus, evidence_sha256);
        let artifact = ImmutableEpisodeArtifact::build(ImmutableEpisodeBuild {
            manifest: &manifest,
            corpus: &corpus,
            evidence: &evidence,
            transition_evidence_sha256: evidence_sha256,
            terminal: HarnessTerminalReason::Reached,
            terminal_detail: "objective reached with complete replay proof",
        })
        .unwrap();

        let mut tampered = artifact.clone();
        tampered.steps[0].reward.training_reward = 99.0;
        assert!(tampered.validate().is_err());

        let mut shifted = artifact;
        shifted.steps[0].post_simulation_state.boundary.tape_frame = Some(2);
        shifted.content_sha256 = shifted.compute_content_sha256().unwrap();
        assert!(
            shifted
                .validate()
                .unwrap_err()
                .to_string()
                .contains("shifted")
        );
    }

    #[test]
    fn rejects_terminal_transition_without_reason() {
        let (corpus, trace, tape) = fixture();
        assert!(
            build(&corpus, &trace, &tape, None)
                .unwrap_err()
                .to_string()
                .contains("lacks an exact reason")
        );
    }

    #[test]
    fn rejects_duplicate_tape_frame_observations() {
        let (corpus, mut trace, tape) = fixture();
        trace.records.push(trace.records[0].clone());
        assert!(
            build(
                &corpus,
                &trace,
                &tape,
                Some(TerminalReasonEvidence::ObjectiveReached)
            )
            .unwrap_err()
            .to_string()
            .contains("duplicate tape-frame")
        );
    }

    #[test]
    fn rejects_duplicate_sparse_side_table_entries() {
        let (corpus, trace, tape) = fixture();
        let mut bundle = build(
            &corpus,
            &trace,
            &tape,
            Some(TerminalReasonEvidence::ObjectiveReached),
        )
        .unwrap();
        bundle
            .entity_side_table
            .push(bundle.entity_side_table[0].clone());
        assert!(
            bundle
                .validate(&corpus)
                .unwrap_err()
                .to_string()
                .contains("duplicate facts")
        );
    }
}
