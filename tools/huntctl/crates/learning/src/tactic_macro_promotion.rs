//! Replay-derived tactic macro discovery and evidence-gated promotion.
//!
//! Candidates remain exact binary input tapes. This module owns only mining,
//! comparative evidence, provenance, and lifecycle state; native execution
//! continues through the ordinary tactic-asset adapter.

use crate::artifact::Digest;
use crate::tactic_asset::{TacticAssetError, TacticAssetSource, TacticCatalogEntry};
use crate::tape::InputTape;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const TACTIC_MACRO_DISCOVERY_SCHEMA_V1: &str = "dusklight-tactic-macro-discovery/v1";
pub const TACTIC_MACRO_DISCOVERY_SCHEMA_V2: &str = "dusklight-tactic-macro-discovery/v2";
pub const MAX_DISCOVERY_OBSERVATIONS: usize = 4_096;
pub const MAX_DISCOVERED_MACROS: usize = 32;
pub const MAX_DISCOVERED_MACRO_TICKS: usize = 64;
pub const MIN_DISCOVERY_OCCURRENCES: usize = 2;
pub const MIN_PROMOTION_COMPARISONS: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct MacroDiscoveryObservation {
    pub seed: u64,
    pub frontier_state_sha256: Digest,
    pub transition_sha256: Digest,
    pub option_id: String,
    pub entry: MacroEntryObservation,
    pub tape: InputTape,
    pub reward: f32,
    pub goal_progress: f32,
    pub terminal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroEntryObservation {
    pub stage: String,
    pub room: i8,
    pub player_procedure: Option<u16>,
    pub player_contacts: Option<u8>,
    pub goal_distance_f32_bits: u32,
}

impl MacroEntryObservation {
    fn validate(&self) -> Result<(), &'static str> {
        let goal_distance = f32::from_bits(self.goal_distance_f32_bits);
        if self.stage.is_empty() || !goal_distance.is_finite() || goal_distance < 0.0 {
            return Err("macro entry observation is invalid");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroSourceProvenance {
    pub seed: u64,
    pub frontier_state_sha256: Digest,
    pub transition_sha256: Digest,
    pub option_id: String,
    pub entry: MacroEntryObservation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticMacroEntryCondition {
    pub stages_and_rooms: BTreeSet<(String, i8)>,
    pub player_procedures: BTreeSet<Option<u16>>,
    pub player_contacts: BTreeSet<Option<u8>>,
    pub minimum_goal_distance: f32,
    pub maximum_goal_distance: f32,
}

impl TacticMacroEntryCondition {
    pub fn matches(
        &self,
        stage: &str,
        room: i8,
        player_procedure: Option<u16>,
        player_contacts: Option<u8>,
        goal_distance: f32,
        goal_distance_padding: f32,
    ) -> bool {
        goal_distance.is_finite()
            && goal_distance_padding.is_finite()
            && goal_distance_padding >= 0.0
            && self.stages_and_rooms.contains(&(stage.to_owned(), room))
            && self.player_procedures.contains(&player_procedure)
            && self.player_contacts.contains(&player_contacts)
            && goal_distance >= (self.minimum_goal_distance - goal_distance_padding).max(0.0)
            && goal_distance <= self.maximum_goal_distance + goal_distance_padding
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredMacroCandidate {
    pub candidate_sha256: Digest,
    pub option_id: String,
    pub tape: InputTape,
    pub sources: Vec<MacroSourceProvenance>,
}

impl DiscoveredMacroCandidate {
    pub fn catalog_entry(&self) -> Result<TacticCatalogEntry, TacticAssetError> {
        TacticCatalogEntry::new(
            self.option_id.clone(),
            TacticAssetSource::RecordedTape(self.tape.clone()),
        )
    }

    pub fn entry_condition(&self) -> Result<TacticMacroEntryCondition, &'static str> {
        validate_candidate(self)?;
        let mut stages_and_rooms = BTreeSet::new();
        let mut player_procedures = BTreeSet::new();
        let mut player_contacts = BTreeSet::new();
        let mut minimum_goal_distance = f32::INFINITY;
        let mut maximum_goal_distance = f32::NEG_INFINITY;
        for source in &self.sources {
            stages_and_rooms.insert((source.entry.stage.clone(), source.entry.room));
            player_procedures.insert(source.entry.player_procedure);
            player_contacts.insert(source.entry.player_contacts);
            let distance = f32::from_bits(source.entry.goal_distance_f32_bits);
            minimum_goal_distance = minimum_goal_distance.min(distance);
            maximum_goal_distance = maximum_goal_distance.max(distance);
        }
        Ok(TacticMacroEntryCondition {
            stages_and_rooms,
            player_procedures,
            player_contacts,
            minimum_goal_distance,
            maximum_goal_distance,
        })
    }
}

pub fn replay_macro_candidate(
    tape: InputTape,
    mut sources: Vec<MacroSourceProvenance>,
) -> Result<DiscoveredMacroCandidate, &'static str> {
    sources.sort_by(|left, right| {
        left.transition_sha256
            .cmp(&right.transition_sha256)
            .then_with(|| left.frontier_state_sha256.cmp(&right.frontier_state_sha256))
            .then_with(|| left.seed.cmp(&right.seed))
            .then_with(|| left.option_id.cmp(&right.option_id))
    });
    sources.dedup_by_key(|source| source.transition_sha256);
    let candidate_sha256 = macro_tape_sha256(&tape)?;
    let candidate = DiscoveredMacroCandidate {
        candidate_sha256,
        option_id: format!("promoted/{}", short_digest(candidate_sha256)),
        tape,
        sources,
    };
    validate_candidate(&candidate)?;
    Ok(candidate)
}

pub fn discover_replay_macros(
    observations: &[MacroDiscoveryObservation],
) -> Result<Vec<DiscoveredMacroCandidate>, &'static str> {
    if observations.is_empty() || observations.len() > MAX_DISCOVERY_OBSERVATIONS {
        return Err("macro discovery observation count is invalid");
    }
    let mut buckets = BTreeMap::<Digest, MacroBucket>::new();
    for observation in observations {
        validate_observation(observation)?;
        for width in [4_usize, 8, 16, 32, 64] {
            if width > observation.tape.frames.len() || width > MAX_DISCOVERED_MACRO_TICKS {
                continue;
            }
            for start in 0..=observation.tape.frames.len() - width {
                let tape = InputTape {
                    boot: observation.tape.boot.clone(),
                    tick_rate_numerator: observation.tape.tick_rate_numerator,
                    tick_rate_denominator: observation.tape.tick_rate_denominator,
                    frames: observation.tape.frames[start..start + width].to_vec(),
                };
                let candidate_sha256 = macro_tape_sha256(&tape)?;
                let bucket = buckets
                    .entry(candidate_sha256)
                    .or_insert_with(|| MacroBucket {
                        tape,
                        sources: BTreeMap::new(),
                        terminal_sources: 0,
                    });
                if bucket
                    .sources
                    .insert(
                        observation.transition_sha256,
                        MacroSourceProvenance {
                            seed: observation.seed,
                            frontier_state_sha256: observation.frontier_state_sha256,
                            transition_sha256: observation.transition_sha256,
                            option_id: observation.option_id.clone(),
                            entry: observation.entry.clone(),
                        },
                    )
                    .is_none()
                {
                    bucket.terminal_sources += usize::from(observation.terminal);
                }
            }
        }
    }
    let mut candidates = buckets
        .into_iter()
        .filter(|(_, bucket)| bucket.sources.len() >= MIN_DISCOVERY_OCCURRENCES)
        .map(|(candidate_sha256, bucket)| {
            (
                bucket.terminal_sources,
                bucket.sources.len(),
                bucket.tape.frames.len(),
                DiscoveredMacroCandidate {
                    candidate_sha256,
                    option_id: format!("promoted/{}", short_digest(candidate_sha256)),
                    tape: bucket.tape,
                    sources: bucket.sources.into_values().collect(),
                },
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.3.candidate_sha256.cmp(&right.3.candidate_sha256))
    });
    candidates.truncate(MAX_DISCOVERED_MACROS);
    Ok(candidates
        .into_iter()
        .map(|(_, _, _, candidate)| candidate)
        .collect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacroPromotionStatus {
    Proposed,
    Promoted,
    Demoted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MacroComparisonEvidence {
    pub comparison_sha256: Digest,
    pub candidate_sha256: Digest,
    pub seed: u64,
    pub frontier_state_sha256: Digest,
    pub candidate_terminal: bool,
    pub candidate_progress: f32,
    pub candidate_ticks: u32,
    pub primitive_terminal: bool,
    pub primitive_progress: f32,
    pub primitive_ticks: u32,
}

impl MacroComparisonEvidence {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        candidate_sha256: Digest,
        seed: u64,
        frontier_state_sha256: Digest,
        candidate_terminal: bool,
        candidate_progress: f32,
        candidate_ticks: u32,
        primitive_terminal: bool,
        primitive_progress: f32,
        primitive_ticks: u32,
    ) -> Result<Self, &'static str> {
        let mut evidence = Self {
            comparison_sha256: Digest::ZERO,
            candidate_sha256,
            seed,
            frontier_state_sha256,
            candidate_terminal,
            candidate_progress,
            candidate_ticks,
            primitive_terminal,
            primitive_progress,
            primitive_ticks,
        };
        evidence.validate()?;
        evidence.comparison_sha256 = comparison_sha256(&evidence);
        Ok(evidence)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.candidate_sha256 == Digest::ZERO
            || self.frontier_state_sha256 == Digest::ZERO
            || self.candidate_ticks == 0
            || self.primitive_ticks == 0
            || !self.candidate_progress.is_finite()
            || !self.primitive_progress.is_finite()
        {
            return Err("macro comparison is invalid");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroPromotionEvent {
    pub status: MacroPromotionStatus,
    pub supporting_comparisons: Vec<Digest>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MacroPromotionRecord {
    pub candidate: DiscoveredMacroCandidate,
    pub status: MacroPromotionStatus,
    pub comparisons: Vec<MacroComparisonEvidence>,
    pub history: Vec<MacroPromotionEvent>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TacticMacroPromotionRegistry {
    records: BTreeMap<Digest, MacroPromotionRecord>,
}

impl TacticMacroPromotionRegistry {
    pub fn propose(&mut self, candidate: DiscoveredMacroCandidate) -> Result<(), &'static str> {
        validate_candidate(&candidate)?;
        match self.records.get(&candidate.candidate_sha256) {
            Some(existing) if existing.candidate != candidate => {
                Err("macro candidate identity collision")
            }
            Some(_) => Ok(()),
            None => {
                self.records.insert(
                    candidate.candidate_sha256,
                    MacroPromotionRecord {
                        candidate,
                        status: MacroPromotionStatus::Proposed,
                        comparisons: Vec::new(),
                        history: vec![MacroPromotionEvent {
                            status: MacroPromotionStatus::Proposed,
                            supporting_comparisons: Vec::new(),
                        }],
                    },
                );
                Ok(())
            }
        }
    }

    pub fn observe(
        &mut self,
        evidence: MacroComparisonEvidence,
    ) -> Result<MacroPromotionStatus, &'static str> {
        evidence.validate()?;
        if evidence.comparison_sha256 != comparison_sha256(&evidence) {
            return Err("macro comparison identity is detached");
        }
        let record = self
            .records
            .get_mut(&evidence.candidate_sha256)
            .ok_or("macro comparison candidate is unknown")?;
        if record
            .comparisons
            .iter()
            .any(|existing| existing.comparison_sha256 == evidence.comparison_sha256)
        {
            return Ok(record.status);
        }
        record.comparisons.push(evidence);
        let next = lifecycle_status(record);
        if next != record.status {
            record.status = next;
            record.history.push(MacroPromotionEvent {
                status: next,
                supporting_comparisons: record
                    .comparisons
                    .iter()
                    .map(|comparison| comparison.comparison_sha256)
                    .collect(),
            });
        }
        Ok(record.status)
    }

    pub fn records(&self) -> impl ExactSizeIterator<Item = &MacroPromotionRecord> {
        self.records.values()
    }

    pub fn promoted(&self) -> impl Iterator<Item = &MacroPromotionRecord> {
        self.records
            .values()
            .filter(|record| record.status == MacroPromotionStatus::Promoted)
    }
}

fn lifecycle_status(record: &MacroPromotionRecord) -> MacroPromotionStatus {
    let comparisons = &record.comparisons;
    let distinct_states = comparisons
        .iter()
        .map(|evidence| evidence.frontier_state_sha256)
        .collect::<BTreeSet<_>>();
    let distinct_seeds = comparisons
        .iter()
        .map(|evidence| evidence.seed)
        .collect::<BTreeSet<_>>();
    if comparisons.len() < MIN_PROMOTION_COMPARISONS
        || distinct_states.len() < MIN_PROMOTION_COMPARISONS
        || distinct_seeds.len() < MIN_PROMOTION_COMPARISONS
    {
        return record.status;
    }
    let recent = &comparisons[comparisons.len() - MIN_PROMOTION_COMPARISONS..];
    if record.status == MacroPromotionStatus::Promoted && !aggregate_improves(recent) {
        return MacroPromotionStatus::Demoted;
    }
    if record.status != MacroPromotionStatus::Promoted && aggregate_improves(comparisons) {
        return MacroPromotionStatus::Promoted;
    }
    record.status
}

fn aggregate_improves(evidence: &[MacroComparisonEvidence]) -> bool {
    let candidate_terminals = evidence
        .iter()
        .filter(|comparison| comparison.candidate_terminal)
        .count();
    let primitive_terminals = evidence
        .iter()
        .filter(|comparison| comparison.primitive_terminal)
        .count();
    if candidate_terminals != primitive_terminals {
        return candidate_terminals > primitive_terminals;
    }
    if candidate_terminals == 0 {
        // Auxiliary progress is useful training signal but cannot promote a
        // policy action in the absence of authenticated terminal evidence.
        return false;
    }
    let candidate_ticks = evidence
        .iter()
        .map(|comparison| u64::from(comparison.candidate_ticks))
        .sum::<u64>();
    let primitive_ticks = evidence
        .iter()
        .map(|comparison| u64::from(comparison.primitive_ticks))
        .sum::<u64>();
    candidate_ticks < primitive_ticks
}

fn validate_observation(observation: &MacroDiscoveryObservation) -> Result<(), &'static str> {
    observation
        .tape
        .validate()
        .map_err(|_| "macro discovery tape is invalid")?;
    if observation.frontier_state_sha256 == Digest::ZERO
        || observation.transition_sha256 == Digest::ZERO
        || observation.option_id.is_empty()
        || observation.tape.frames.is_empty()
        || observation.tape.frames.len() > MAX_DISCOVERED_MACRO_TICKS
        || !observation.reward.is_finite()
        || !observation.goal_progress.is_finite()
    {
        return Err("macro discovery observation is invalid");
    }
    observation.entry.validate()?;
    Ok(())
}

fn validate_candidate(candidate: &DiscoveredMacroCandidate) -> Result<(), &'static str> {
    let distinct_sources = candidate
        .sources
        .iter()
        .map(|source| source.transition_sha256)
        .collect::<BTreeSet<_>>();
    if candidate.candidate_sha256 == Digest::ZERO
        || candidate.option_id != format!("promoted/{}", short_digest(candidate.candidate_sha256))
        || candidate.sources.len() < MIN_DISCOVERY_OCCURRENCES
        || candidate.sources.len() != distinct_sources.len()
        || candidate.sources.iter().any(|source| {
            source.frontier_state_sha256 == Digest::ZERO
                || source.transition_sha256 == Digest::ZERO
                || source.option_id.is_empty()
                || source.entry.validate().is_err()
        })
        || macro_tape_sha256(&candidate.tape)? != candidate.candidate_sha256
    {
        return Err("discovered macro candidate is invalid");
    }
    Ok(())
}

fn macro_tape_sha256(tape: &InputTape) -> Result<Digest, &'static str> {
    tape.validate().map_err(|_| "macro tape is invalid")?;
    let encoded = tape.encode().map_err(|_| "macro tape encoding failed")?;
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_MACRO_DISCOVERY_SCHEMA_V2.as_bytes());
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    Ok(Digest(hasher.finalize().into()))
}

fn comparison_sha256(evidence: &MacroComparisonEvidence) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_MACRO_DISCOVERY_SCHEMA_V2.as_bytes());
    hasher.update(evidence.candidate_sha256.0);
    hasher.update(evidence.seed.to_le_bytes());
    hasher.update(evidence.frontier_state_sha256.0);
    hasher.update([
        u8::from(evidence.candidate_terminal),
        u8::from(evidence.primitive_terminal),
    ]);
    hasher.update(evidence.candidate_progress.to_bits().to_le_bytes());
    hasher.update(evidence.candidate_ticks.to_le_bytes());
    hasher.update(evidence.primitive_progress.to_bits().to_le_bytes());
    hasher.update(evidence.primitive_ticks.to_le_bytes());
    Digest(hasher.finalize().into())
}

fn short_digest(digest: Digest) -> String {
    digest.0[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct MacroBucket {
    tape: InputTape,
    sources: BTreeMap<Digest, MacroSourceProvenance>,
    terminal_sources: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::InputFrame;

    fn tape(value: i8, ticks: usize) -> InputTape {
        let mut frame = InputFrame::default();
        frame.owned_ports = 1;
        frame.pads[0].stick_x = value;
        InputTape {
            frames: vec![frame; ticks],
            ..InputTape::default()
        }
    }

    fn observation(seed: u64, state: u8, transition: u8) -> MacroDiscoveryObservation {
        MacroDiscoveryObservation {
            seed,
            frontier_state_sha256: Digest([state; 32]),
            transition_sha256: Digest([transition; 32]),
            option_id: format!("family/move/{transition}"),
            entry: MacroEntryObservation {
                stage: "F_SP103".into(),
                room: 1,
                player_procedure: Some(3),
                player_contacts: Some(1),
                goal_distance_f32_bits: (100.0 + f32::from(state)).to_bits(),
            },
            tape: tape(80, 8),
            reward: -0.08,
            goal_progress: -16.0,
            terminal: false,
        }
    }

    #[test]
    fn recurring_binary_fragments_become_value_neutral_exact_candidates() {
        let candidates =
            discover_replay_macros(&[observation(11, 1, 3), observation(13, 2, 4)]).unwrap();
        assert!(!candidates.is_empty());
        let longest = &candidates[0];
        assert_eq!(longest.tape.frames.len(), 8);
        assert_eq!(longest.sources.len(), 2);
        assert!(longest.option_id.starts_with("promoted/"));
        let entry = longest.catalog_entry().unwrap();
        let exact = entry.exact_static_realization().unwrap().unwrap();
        assert_eq!(exact.tape, longest.tape);
    }

    #[test]
    fn connected_component_provenance_can_propose_one_exact_composed_macro() {
        let candidate = replay_macro_candidate(
            tape(80, 16),
            vec![
                MacroSourceProvenance {
                    seed: 11,
                    frontier_state_sha256: Digest([1; 32]),
                    transition_sha256: Digest([3; 32]),
                    option_id: "family/seek/a".into(),
                    entry: observation(11, 1, 3).entry,
                },
                MacroSourceProvenance {
                    seed: 11,
                    frontier_state_sha256: Digest([2; 32]),
                    transition_sha256: Digest([4; 32]),
                    option_id: "family/curve/b".into(),
                    entry: observation(11, 2, 4).entry,
                },
            ],
        )
        .unwrap();
        assert_eq!(candidate.tape.frames.len(), 16);
        assert_eq!(candidate.sources.len(), 2);
        let condition = candidate.entry_condition().unwrap();
        assert_eq!(
            condition.stages_and_rooms,
            BTreeSet::from([("F_SP103".into(), 1)])
        );
        assert_eq!(condition.minimum_goal_distance, 101.0);
        assert_eq!(condition.maximum_goal_distance, 102.0);
        assert_eq!(
            candidate
                .catalog_entry()
                .unwrap()
                .exact_static_realization()
                .unwrap()
                .unwrap()
                .tape,
            candidate.tape
        );
    }

    #[test]
    fn promotion_requires_strict_comparative_gain_across_states_and_seeds() {
        let candidate = discover_replay_macros(&[observation(11, 1, 3), observation(13, 2, 4)])
            .unwrap()[0]
            .clone();
        let mut registry = TacticMacroPromotionRegistry::default();
        registry.propose(candidate.clone()).unwrap();
        let evidence = |seed, state| {
            MacroComparisonEvidence::new(
                candidate.candidate_sha256,
                seed,
                Digest([state; 32]),
                true,
                -100.0,
                7,
                true,
                8.0,
                8,
            )
            .unwrap()
        };
        assert_eq!(
            registry.observe(evidence(11, 1)).unwrap(),
            MacroPromotionStatus::Proposed
        );
        assert_eq!(
            registry.observe(evidence(13, 2)).unwrap(),
            MacroPromotionStatus::Promoted
        );
        assert_eq!(registry.promoted().count(), 1);
    }

    #[test]
    fn auxiliary_progress_alone_cannot_promote_a_macro() {
        let candidate = discover_replay_macros(&[observation(11, 1, 3), observation(13, 2, 4)])
            .unwrap()[0]
            .clone();
        let mut registry = TacticMacroPromotionRegistry::default();
        registry.propose(candidate.clone()).unwrap();
        for (seed, state) in [(11, 1), (13, 2)] {
            assert_eq!(
                registry
                    .observe(
                        MacroComparisonEvidence::new(
                            candidate.candidate_sha256,
                            seed,
                            Digest([state; 32]),
                            false,
                            1_000.0,
                            4,
                            false,
                            -1_000.0,
                            40,
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                MacroPromotionStatus::Proposed
            );
        }
    }

    #[test]
    fn later_regression_demotes_without_erasing_provenance() {
        let candidate = discover_replay_macros(&[observation(11, 1, 3), observation(13, 2, 4)])
            .unwrap()[0]
            .clone();
        let mut registry = TacticMacroPromotionRegistry::default();
        registry.propose(candidate.clone()).unwrap();
        for (seed, state) in [(11, 1), (13, 2)] {
            registry
                .observe(
                    MacroComparisonEvidence::new(
                        candidate.candidate_sha256,
                        seed,
                        Digest([state; 32]),
                        true,
                        20.0,
                        8,
                        false,
                        8.0,
                        8,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        for (seed, state) in [(17, 3), (19, 4)] {
            registry
                .observe(
                    MacroComparisonEvidence::new(
                        candidate.candidate_sha256,
                        seed,
                        Digest([state; 32]),
                        false,
                        1.0,
                        8,
                        true,
                        8.0,
                        8,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let record = registry.records().next().unwrap();
        assert_eq!(record.status, MacroPromotionStatus::Demoted);
        assert_eq!(record.comparisons.len(), 4);
        assert_eq!(
            record
                .history
                .iter()
                .map(|event| event.status)
                .collect::<Vec<_>>(),
            vec![
                MacroPromotionStatus::Proposed,
                MacroPromotionStatus::Promoted,
                MacroPromotionStatus::Demoted
            ]
        );
        assert_eq!(record.candidate.sources.len(), 2);
    }
}
