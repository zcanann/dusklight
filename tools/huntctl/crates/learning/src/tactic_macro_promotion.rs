//! Replay-derived tactic macro discovery and evidence-gated promotion.
//!
//! Candidates remain exact binary input tapes. This module owns only mining,
//! comparative evidence, provenance, and lifecycle state; native execution
//! continues through the ordinary tactic-asset adapter.

use crate::artifact::Digest;
use crate::option_values::OptionActionDescriptor;
use crate::tactic_asset::{
    EncodedTacticAssetSource, TacticAssetError, TacticAssetSource, TacticCatalogEntry,
};
use crate::tape::InputTape;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const TACTIC_MACRO_DISCOVERY_SCHEMA_V1: &str = "dusklight-tactic-macro-discovery/v1";
pub const TACTIC_MACRO_DISCOVERY_SCHEMA_V2: &str = "dusklight-tactic-macro-discovery/v2";
pub const TACTIC_MACRO_DISCOVERY_SCHEMA_V3: &str = "dusklight-tactic-macro-discovery/v3";
pub const TACTIC_MACRO_DISCOVERY_SCHEMA_V4: &str = "dusklight-tactic-macro-discovery/v4";
pub const TACTIC_MACRO_COMPONENT_SCHEMA_V1: &str = "dusklight-tactic-macro-component/v1";
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
    pub component: TacticMacroComponent,
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
    pub transition_sha256s: Vec<Digest>,
    pub entry: MacroEntryObservation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticMacroComponent {
    pub schema: String,
    pub action: OptionActionDescriptor,
    pub source: EncodedTacticAssetSource,
}

impl TacticMacroComponent {
    pub fn from_catalog_entry(entry: &TacticCatalogEntry) -> Result<Self, TacticAssetError> {
        let component = Self {
            schema: TACTIC_MACRO_COMPONENT_SCHEMA_V1.into(),
            action: entry.description().option.clone(),
            source: EncodedTacticAssetSource::capture(entry.source())?,
        };
        component.catalog_entry()?;
        Ok(component)
    }

    pub fn catalog_entry(&self) -> Result<TacticCatalogEntry, TacticAssetError> {
        if self.schema != TACTIC_MACRO_COMPONENT_SCHEMA_V1 {
            return Err(TacticAssetError::InvalidAsset(
                "tactic macro component schema is invalid".into(),
            ));
        }
        self.action
            .validate()
            .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))?;
        let entry = TacticCatalogEntry::new(self.action.option_id.clone(), self.source.decode()?)?;
        if entry.description().option != self.action {
            return Err(TacticAssetError::InvalidAsset(
                "tactic macro component action differs from its executable source".into(),
            ));
        }
        Ok(entry)
    }

    pub fn content_sha256(&self) -> Result<Digest, TacticAssetError> {
        self.catalog_entry()?;
        let mut hasher = Sha256::new();
        hasher.update(TACTIC_MACRO_COMPONENT_SCHEMA_V1.as_bytes());
        let action_sha256 = self
            .action
            .content_sha256()
            .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))?;
        hasher.update(action_sha256.0);
        hasher.update(self.source.content_sha256()?.0);
        Ok(Digest(hasher.finalize().into()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticMacroEntryCondition {
    pub cells: Vec<TacticMacroEntryConditionCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticMacroEntryConditionCell {
    pub stage: String,
    pub room: i8,
    pub player_procedure: Option<u16>,
    pub player_contacts: Option<u8>,
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
        self.distance_to_support(
            stage,
            room,
            player_procedure,
            player_contacts,
            goal_distance,
            goal_distance_padding,
        )
        .is_some()
    }

    pub fn distance_to_support(
        &self,
        stage: &str,
        room: i8,
        player_procedure: Option<u16>,
        player_contacts: Option<u8>,
        goal_distance: f32,
        goal_distance_padding: f32,
    ) -> Option<f32> {
        if !goal_distance.is_finite()
            || !goal_distance_padding.is_finite()
            || goal_distance_padding < 0.0
        {
            return None;
        }
        self.cells
            .iter()
            .filter(|cell| {
                cell.stage == stage
                    && cell.room == room
                    && cell.player_procedure == player_procedure
                    && cell.player_contacts == player_contacts
                    && goal_distance
                        >= (cell.minimum_goal_distance - goal_distance_padding).max(0.0)
                    && goal_distance <= cell.maximum_goal_distance + goal_distance_padding
            })
            .map(|cell| {
                if goal_distance < cell.minimum_goal_distance {
                    cell.minimum_goal_distance - goal_distance
                } else if goal_distance > cell.maximum_goal_distance {
                    goal_distance - cell.maximum_goal_distance
                } else {
                    0.0
                }
            })
            .min_by(f32::total_cmp)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredMacroCandidate {
    pub candidate_sha256: Digest,
    pub option_id: String,
    pub tape: InputTape,
    pub components: Vec<TacticMacroComponent>,
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
        let mut cells = BTreeMap::<
            (String, i8, Option<u16>, Option<u8>),
            (f32, f32),
        >::new();
        for source in &self.sources {
            let distance = f32::from_bits(source.entry.goal_distance_f32_bits);
            let range = cells
                .entry((
                    source.entry.stage.clone(),
                    source.entry.room,
                    source.entry.player_procedure,
                    source.entry.player_contacts,
                ))
                .or_insert((distance, distance));
            range.0 = range.0.min(distance);
            range.1 = range.1.max(distance);
        }
        Ok(TacticMacroEntryCondition {
            cells: cells
                .into_iter()
                .map(
                    |(
                        (stage, room, player_procedure, player_contacts),
                        (minimum_goal_distance, maximum_goal_distance),
                    )| TacticMacroEntryConditionCell {
                        stage,
                        room,
                        player_procedure,
                        player_contacts,
                        minimum_goal_distance,
                        maximum_goal_distance,
                    },
                )
                .collect(),
        })
    }
}

pub fn replay_macro_candidate(
    tape: InputTape,
    components: Vec<TacticMacroComponent>,
    mut sources: Vec<MacroSourceProvenance>,
) -> Result<DiscoveredMacroCandidate, &'static str> {
    sources.sort_by(|left, right| {
        left.transition_sha256s
            .cmp(&right.transition_sha256s)
            .then_with(|| left.frontier_state_sha256.cmp(&right.frontier_state_sha256))
            .then_with(|| left.seed.cmp(&right.seed))
    });
    sources.dedup_by(|left, right| left.transition_sha256s == right.transition_sha256s);
    let candidate_sha256 = macro_candidate_sha256(&tape, &components)?;
    let candidate = DiscoveredMacroCandidate {
        candidate_sha256,
        option_id: format!("promoted/{}", short_digest(candidate_sha256)),
        tape,
        components,
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
            // Only prefixes have an authenticated entry observation without
            // another native replay. Interior fragments would inherit the
            // enclosing option's pre-input state and fabricate their entry
            // condition. Connected decision sequences are mined separately
            // from exact journal boundaries.
            let tape = InputTape {
                boot: observation.tape.boot.clone(),
                tick_rate_numerator: observation.tape.tick_rate_numerator,
                tick_rate_denominator: observation.tape.tick_rate_denominator,
                frames: observation.tape.frames[..width].to_vec(),
            };
            let components = vec![observation.component.clone()];
            let candidate_sha256 = macro_candidate_sha256(&tape, &components)?;
            let bucket = buckets
                .entry(candidate_sha256)
                .or_insert_with(|| MacroBucket {
                    tape,
                    components,
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
                        transition_sha256s: vec![observation.transition_sha256],
                        entry: observation.entry.clone(),
                    },
                )
                .is_none()
            {
                bucket.terminal_sources += usize::from(observation.terminal);
            }
        }
    }
    let mut candidates = buckets
        .into_iter()
        .filter(|(_, bucket)| {
            bucket.sources.len() >= MIN_DISCOVERY_OCCURRENCES
                && bucket
                    .sources
                    .values()
                    .map(|source| source.frontier_state_sha256)
                    .collect::<BTreeSet<_>>()
                    .len()
                    >= MIN_DISCOVERY_OCCURRENCES
        })
        .map(|(candidate_sha256, bucket)| {
            (
                bucket.terminal_sources,
                bucket.sources.len(),
                bucket.tape.frames.len(),
                DiscoveredMacroCandidate {
                    candidate_sha256,
                    option_id: format!("promoted/{}", short_digest(candidate_sha256)),
                    tape: bucket.tape,
                    components: bucket.components,
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
    evidence.len() >= MIN_PROMOTION_COMPARISONS
        && evidence.iter().all(|comparison| {
            if comparison.candidate_terminal != comparison.primitive_terminal {
                comparison.candidate_terminal
            } else {
                comparison.candidate_terminal
                    && comparison.candidate_ticks < comparison.primitive_ticks
            }
        })
}

fn validate_observation(observation: &MacroDiscoveryObservation) -> Result<(), &'static str> {
    observation
        .tape
        .validate()
        .map_err(|_| "macro discovery tape is invalid")?;
    if observation.frontier_state_sha256 == Digest::ZERO
        || observation.transition_sha256 == Digest::ZERO
        || observation.tape.frames.is_empty()
        || observation.tape.frames.len() > MAX_DISCOVERED_MACRO_TICKS
        || !observation.reward.is_finite()
        || !observation.goal_progress.is_finite()
    {
        return Err("macro discovery observation is invalid");
    }
    observation
        .component
        .catalog_entry()
        .map_err(|_| "macro discovery component is invalid")?;
    observation.entry.validate()?;
    Ok(())
}

fn validate_candidate(candidate: &DiscoveredMacroCandidate) -> Result<(), &'static str> {
    let distinct_sources = candidate
        .sources
        .iter()
        .map(|source| source.transition_sha256s.clone())
        .collect::<BTreeSet<_>>();
    let distinct_source_states = candidate
        .sources
        .iter()
        .map(|source| source.frontier_state_sha256)
        .collect::<BTreeSet<_>>();
    if candidate.candidate_sha256 == Digest::ZERO
        || candidate.option_id != format!("promoted/{}", short_digest(candidate.candidate_sha256))
        || candidate.sources.len() < MIN_DISCOVERY_OCCURRENCES
        || candidate.sources.len() != distinct_sources.len()
        || distinct_source_states.len() < MIN_DISCOVERY_OCCURRENCES
        || candidate.components.is_empty()
        || candidate.components.len() > MAX_DISCOVERED_MACRO_TICKS
        || candidate
            .components
            .iter()
            .any(|component| component.catalog_entry().is_err())
        || candidate.sources.iter().any(|source| {
            source.frontier_state_sha256 == Digest::ZERO
                || source.transition_sha256s.is_empty()
                || source.transition_sha256s.contains(&Digest::ZERO)
                || source.transition_sha256s.len() != candidate.components.len()
                || source.entry.validate().is_err()
        })
        || macro_candidate_sha256(&candidate.tape, &candidate.components)?
            != candidate.candidate_sha256
    {
        return Err("discovered macro candidate is invalid");
    }
    Ok(())
}

fn macro_candidate_sha256(
    tape: &InputTape,
    components: &[TacticMacroComponent],
) -> Result<Digest, &'static str> {
    tape.validate().map_err(|_| "macro tape is invalid")?;
    if components.is_empty()
        || components
            .iter()
            .any(|component| component.catalog_entry().is_err())
    {
        return Err("macro component sequence is invalid");
    }
    let encoded = tape.encode().map_err(|_| "macro tape encoding failed")?;
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_MACRO_DISCOVERY_SCHEMA_V4.as_bytes());
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    hasher.update((components.len() as u64).to_le_bytes());
    for component in components {
        hasher.update(
            component
                .content_sha256()
                .map_err(|_| "macro component identity failed")?
                .0,
        );
    }
    Ok(Digest(hasher.finalize().into()))
}

fn comparison_sha256(evidence: &MacroComparisonEvidence) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_MACRO_DISCOVERY_SCHEMA_V4.as_bytes());
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
    components: Vec<TacticMacroComponent>,
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
        let tape = tape(80, 8);
        let entry = TacticCatalogEntry::new(
            "family/move",
            TacticAssetSource::RecordedTape(tape.clone()),
        )
        .unwrap();
        MacroDiscoveryObservation {
            seed,
            frontier_state_sha256: Digest([state; 32]),
            transition_sha256: Digest([transition; 32]),
            component: TacticMacroComponent::from_catalog_entry(&entry).unwrap(),
            entry: MacroEntryObservation {
                stage: "F_SP103".into(),
                room: 1,
                player_procedure: Some(3),
                player_contacts: Some(1),
                goal_distance_f32_bits: (100.0 + f32::from(state)).to_bits(),
            },
            tape,
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
        assert_eq!(
            longest.components,
            vec![observation(17, 3, 5).component]
        );
        assert!(
            longest
                .sources
                .iter()
                .all(|source| source.transition_sha256s.len() == 1)
        );
        assert!(longest.option_id.starts_with("promoted/"));
        let entry = longest.catalog_entry().unwrap();
        let exact = entry.exact_static_realization().unwrap().unwrap();
        assert_eq!(exact.tape, longest.tape);
    }

    #[test]
    fn interior_fragments_without_boundary_observations_are_not_candidates() {
        let mut left = observation(11, 1, 3);
        let mut right = observation(13, 2, 4);
        left.tape.frames[..4]
            .iter_mut()
            .for_each(|frame| frame.pads[0].stick_x = -80);
        right.tape.frames[..4]
            .iter_mut()
            .for_each(|frame| frame.pads[0].stick_x = 80);
        left.tape.frames[4..]
            .iter_mut()
            .for_each(|frame| frame.pads[0].stick_x = 40);
        right.tape.frames[4..]
            .iter_mut()
            .for_each(|frame| frame.pads[0].stick_x = 40);

        assert!(discover_replay_macros(&[left, right]).unwrap().is_empty());
    }

    #[test]
    fn repeated_transitions_from_one_state_do_not_teach_an_entry_condition() {
        let left = observation(11, 1, 3);
        let right = observation(13, 1, 4);
        assert!(discover_replay_macros(&[left, right]).unwrap().is_empty());
    }

    #[test]
    fn identical_tapes_from_different_typed_actions_do_not_claim_one_composition() {
        let left = observation(11, 1, 3);
        let mut right = observation(13, 2, 4);
        let entry = TacticCatalogEntry::new(
            "family/roll",
            TacticAssetSource::RecordedTape(right.tape.clone()),
        )
        .unwrap();
        right.component = TacticMacroComponent::from_catalog_entry(&entry).unwrap();

        assert!(discover_replay_macros(&[left, right]).unwrap().is_empty());
    }

    #[test]
    fn independent_entry_states_can_propose_one_exact_composed_macro() {
        let candidate = replay_macro_candidate(
            tape(80, 16),
            vec![
                observation(11, 1, 3).component,
                observation(11, 2, 4).component,
            ],
            vec![
                MacroSourceProvenance {
                    seed: 11,
                    frontier_state_sha256: Digest([1; 32]),
                    transition_sha256s: vec![Digest([3; 32]), Digest([5; 32])],
                    entry: observation(11, 1, 3).entry,
                },
                MacroSourceProvenance {
                    seed: 11,
                    frontier_state_sha256: Digest([2; 32]),
                    transition_sha256s: vec![Digest([4; 32]), Digest([6; 32])],
                    entry: observation(11, 2, 4).entry,
                },
            ],
        )
        .unwrap();
        assert_eq!(candidate.tape.frames.len(), 16);
        assert_eq!(candidate.components.len(), 2);
        assert_eq!(candidate.sources.len(), 2);
        let condition = candidate.entry_condition().unwrap();
        assert_eq!(condition.cells.len(), 1);
        assert_eq!(condition.cells[0].stage, "F_SP103");
        assert_eq!(condition.cells[0].room, 1);
        assert_eq!(condition.cells[0].minimum_goal_distance, 101.0);
        assert_eq!(condition.cells[0].maximum_goal_distance, 102.0);
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
    fn entry_conditions_preserve_joint_categorical_evidence() {
        let component = observation(11, 1, 3).component;
        let mut left = observation(11, 1, 3);
        left.entry.stage = "ROOM_A".into();
        left.entry.room = 1;
        left.entry.player_procedure = Some(10);
        left.entry.player_contacts = Some(1);
        left.entry.goal_distance_f32_bits = 100.0_f32.to_bits();
        let mut right = observation(13, 2, 4);
        right.entry.stage = "ROOM_B".into();
        right.entry.room = 2;
        right.entry.player_procedure = Some(20);
        right.entry.player_contacts = Some(2);
        right.entry.goal_distance_f32_bits = 200.0_f32.to_bits();
        let candidate = replay_macro_candidate(
            tape(80, 8),
            vec![component],
            vec![
                MacroSourceProvenance {
                    seed: left.seed,
                    frontier_state_sha256: left.frontier_state_sha256,
                    transition_sha256s: vec![left.transition_sha256],
                    entry: left.entry,
                },
                MacroSourceProvenance {
                    seed: right.seed,
                    frontier_state_sha256: right.frontier_state_sha256,
                    transition_sha256s: vec![right.transition_sha256],
                    entry: right.entry,
                },
            ],
        )
        .unwrap();
        let condition = candidate.entry_condition().unwrap();

        assert!(condition.matches("ROOM_A", 1, Some(10), Some(1), 100.0, 0.0));
        assert!(condition.matches("ROOM_B", 2, Some(20), Some(2), 200.0, 0.0));
        assert!(!condition.matches("ROOM_A", 1, Some(20), Some(2), 100.0, 0.0));
        assert!(!condition.matches("ROOM_B", 2, Some(10), Some(1), 200.0, 0.0));
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
    fn cheap_failures_cannot_offset_a_slower_terminal_route() {
        let candidate_sha256 = Digest([7; 32]);
        let comparison = |seed,
                          state,
                          candidate_terminal,
                          candidate_ticks,
                          primitive_terminal,
                          primitive_ticks| {
            MacroComparisonEvidence::new(
                candidate_sha256,
                seed,
                Digest([state; 32]),
                candidate_terminal,
                0.0,
                candidate_ticks,
                primitive_terminal,
                0.0,
                primitive_ticks,
            )
            .unwrap()
        };
        let misleading_aggregate = vec![
            comparison(11, 1, true, 100, true, 90),
            comparison(13, 2, false, 1, false, 64),
        ];
        let one_win_one_tie = vec![
            comparison(11, 1, true, 8, false, 8),
            comparison(13, 2, false, 8, false, 8),
        ];

        assert!(!aggregate_improves(&misleading_aggregate));
        assert!(!aggregate_improves(&one_win_one_tie));
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
