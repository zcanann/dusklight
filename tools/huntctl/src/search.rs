//! Finite-sample search primitives for milestone-backed TAS optimization.

use crate::tape::{InputFrame, InputTape, RawPadState};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const CANDIDATE_SCHEMA: &str = "dusklight-search-candidate/v1";
pub const POPULATION_SCHEMA: &str = "dusklight-search-population/v1";
pub const RESULTS_SCHEMA: &str = "dusklight-search-results/v1";
const MAX_ACTIONS: usize = 4096;
const MAX_FRAMES: u64 = 1_000_000;
const BUTTON_A: u16 = 0x0100;
const BUTTON_B: u16 = 0x0200;
const BUTTON_START: u16 = 0x1000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentProfile {
    BootToFsp103,
    Fsp103ToFsp104,
}

impl SegmentProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BootToFsp103 => "boot_to_fsp103",
            Self::Fsp103ToFsp104 => "fsp103_to_fsp104",
        }
    }

    pub const fn target(self) -> MilestoneId {
        match self {
            Self::BootToFsp103 => MilestoneId::Fsp103RouteControl,
            Self::Fsp103ToFsp104 => MilestoneId::Fsp104Loaded,
        }
    }

    pub const fn target_depth(self) -> u16 {
        self.target().depth()
    }
}

impl std::str::FromStr for SegmentProfile {
    type Err = SearchError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "boot_to_fsp103" => Ok(Self::BootToFsp103),
            "fsp103_to_fsp104" => Ok(Self::Fsp103ToFsp104),
            _ => Err(SearchError::InvalidSegment(value.into())),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneId {
    Fsp103Playable,
    Fsp103RouteControl,
    Fsp104LoadingTrigger,
    Fsp104Loaded,
}

impl MilestoneId {
    pub const fn depth(self) -> u16 {
        match self {
            Self::Fsp103Playable => 1,
            Self::Fsp103RouteControl => 2,
            Self::Fsp104LoadingTrigger => 3,
            Self::Fsp104Loaded => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerButton {
    A,
    B,
    Start,
}

impl ControllerButton {
    const fn mask(self) -> u16 {
        match self {
            Self::A => BUTTON_A,
            Self::B => BUTTON_B,
            Self::Start => BUTTON_START,
        }
    }
}

/// Search-friendly controller macros. Angles use degrees: 0 is forward and
/// +90 is right. A roll presses B on its first frame and holds its stick for
/// the requested recovery frames.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MacroAction {
    Move {
        angle_degrees: i16,
        magnitude: u8,
        frames: u32,
    },
    Roll {
        angle_degrees: i16,
        magnitude: u8,
        recovery_frames: u32,
    },
    Neutral {
        frames: u32,
    },
    Press {
        buttons: Vec<ControllerButton>,
        hold_frames: u32,
        neutral_frames: u32,
    },
}

impl MacroAction {
    fn frame_count(&self) -> u64 {
        match self {
            Self::Move { frames, .. } | Self::Neutral { frames } => u64::from(*frames),
            Self::Roll {
                recovery_frames, ..
            } => 1 + u64::from(*recovery_frames),
            Self::Press {
                hold_frames,
                neutral_frames,
                ..
            } => u64::from(*hold_frames) + u64::from(*neutral_frames),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub schema: String,
    pub segment: SegmentProfile,
    pub actions: Vec<MacroAction>,
    pub ancestry: Ancestry,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ancestry {
    pub generation: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation: Option<String>,
}

impl Candidate {
    pub fn baseline(segment: SegmentProfile) -> Self {
        let actions = match segment {
            SegmentProfile::BootToFsp103 => boot_baseline(),
            SegmentProfile::Fsp103ToFsp104 => route_baseline(),
        };
        Self {
            schema: CANDIDATE_SCHEMA.into(),
            segment,
            actions,
            ancestry: Ancestry::default(),
        }
    }

    pub fn validate(&self) -> Result<(), SearchError> {
        if self.schema != CANDIDATE_SCHEMA {
            return Err(SearchError::InvalidSchema(self.schema.clone()));
        }
        if self.actions.is_empty() || self.actions.len() > MAX_ACTIONS {
            return Err(SearchError::InvalidActionCount(self.actions.len()));
        }
        let mut frames = 0_u64;
        for action in &self.actions {
            match action {
                MacroAction::Move {
                    magnitude, frames, ..
                } => {
                    validate_magnitude(*magnitude)?;
                    validate_duration(*frames)?;
                }
                MacroAction::Roll {
                    magnitude,
                    recovery_frames,
                    ..
                } => {
                    validate_magnitude(*magnitude)?;
                    if *recovery_frames > 300 {
                        return Err(SearchError::InvalidDuration(*recovery_frames));
                    }
                }
                MacroAction::Neutral { frames } => validate_duration(*frames)?,
                MacroAction::Press {
                    buttons,
                    hold_frames,
                    neutral_frames,
                } => {
                    if buttons.is_empty() || *hold_frames == 0 || *hold_frames > 30 {
                        return Err(SearchError::InvalidButtonPress);
                    }
                    if *neutral_frames > 10_000 {
                        return Err(SearchError::InvalidDuration(*neutral_frames));
                    }
                }
            }
            frames = frames
                .checked_add(action.frame_count())
                .ok_or(SearchError::TooManyFrames)?;
        }
        if frames > MAX_FRAMES {
            return Err(SearchError::TooManyFrames);
        }
        Ok(())
    }

    /// Stable content ID. Ancestry is deliberately excluded so identical input
    /// programs deduplicate even when rediscovered through different parents.
    pub fn id(&self) -> Result<String, SearchError> {
        self.validate()?;
        let identity = serde_json::to_vec(&(self.segment, &self.actions))?;
        Ok(format!("{:x}", Sha256::digest(identity)))
    }

    pub fn frame_count(&self) -> u64 {
        self.actions.iter().map(MacroAction::frame_count).sum()
    }

    pub fn compile(&self) -> Result<InputTape, SearchError> {
        self.validate()?;
        let capacity =
            usize::try_from(self.frame_count()).map_err(|_| SearchError::TooManyFrames)?;
        let mut frames = Vec::with_capacity(capacity);
        for action in &self.actions {
            match action {
                MacroAction::Move {
                    angle_degrees,
                    magnitude,
                    frames: count,
                } => push_frames(
                    &mut frames,
                    move_frame(*angle_degrees, *magnitude, 0),
                    *count,
                ),
                MacroAction::Roll {
                    angle_degrees,
                    magnitude,
                    recovery_frames,
                } => {
                    frames.push(move_frame(*angle_degrees, *magnitude, BUTTON_B));
                    push_frames(
                        &mut frames,
                        move_frame(*angle_degrees, *magnitude, 0),
                        *recovery_frames,
                    );
                }
                MacroAction::Neutral { frames: count } => {
                    push_frames(&mut frames, owned_frame(RawPadState::default()), *count)
                }
                MacroAction::Press {
                    buttons,
                    hold_frames,
                    neutral_frames,
                } => {
                    let buttons = buttons.iter().fold(0, |mask, button| mask | button.mask());
                    let pad = RawPadState {
                        buttons,
                        ..RawPadState::default()
                    };
                    push_frames(&mut frames, owned_frame(pad), *hold_frames);
                    push_frames(
                        &mut frames,
                        owned_frame(RawPadState::default()),
                        *neutral_frames,
                    );
                }
            }
        }
        Ok(InputTape {
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            frames,
        })
    }

    /// Losslessly infers typed boot macros from an absolute raw tape. Only
    /// neutral frames and zero-stick A/B/Start pulses are accepted; analog,
    /// reactive, multi-port, or noncanonical pad state is rejected instead of
    /// being guessed.
    pub fn from_absolute_tape(
        segment: SegmentProfile,
        tape: &InputTape,
    ) -> Result<Self, SearchError> {
        tape.validate()?;
        if tape.tick_rate_numerator != 30 || tape.tick_rate_denominator != 1 {
            return Err(SearchError::NonCanonicalTape(
                "typed candidates currently require a 30/1 tick rate".into(),
            ));
        }
        if tape.frames.is_empty() {
            return Err(SearchError::NonCanonicalTape("tape is empty".into()));
        }
        #[derive(Clone, Copy, Eq, PartialEq)]
        enum State {
            Neutral,
            Press(u16),
        }
        let classify = |frame: &InputFrame| -> Result<State, SearchError> {
            if frame.owned_ports != 0x0f
                || frame.wait_condition != crate::tape::WaitCondition::None
                || frame.wait_timeout_ticks != 0
                || frame.pads[1..] != [RawPadState::default(); 3]
            {
                return Err(SearchError::NonCanonicalTape(
                    "frame ownership, reactive waits, or secondary ports are not safely expressible"
                        .into(),
                ));
            }
            let pad = frame.pads[0];
            let canonical = RawPadState {
                buttons: pad.buttons,
                ..RawPadState::default()
            };
            if pad != canonical {
                return Err(SearchError::NonCanonicalTape(
                    "analog, trigger, disconnected, or error pad state is ambiguous".into(),
                ));
            }
            if pad.buttons & !(BUTTON_A | BUTTON_B | BUTTON_START) != 0 {
                return Err(SearchError::NonCanonicalTape(format!(
                    "button mask 0x{:04x} is not a typed A/B/Start pulse",
                    pad.buttons
                )));
            }
            Ok(if pad.buttons == 0 {
                State::Neutral
            } else {
                State::Press(pad.buttons)
            })
        };
        let mut runs: Vec<(State, u32)> = Vec::new();
        for frame in &tape.frames {
            let state = classify(frame)?;
            if let Some((last, count)) = runs.last_mut()
                && *last == state
            {
                *count = count.checked_add(1).ok_or(SearchError::TooManyFrames)?;
            } else {
                runs.push((state, 1));
            }
        }
        let mut actions = Vec::new();
        let mut index = 0;
        while index < runs.len() {
            match runs[index] {
                (State::Neutral, frames) => {
                    actions.push(MacroAction::Neutral { frames });
                    index += 1;
                }
                (State::Press(mask), hold_frames) => {
                    if hold_frames > 30 {
                        return Err(SearchError::NonCanonicalTape(
                            "button hold exceeds the typed press limit".into(),
                        ));
                    }
                    let neutral_frames = runs
                        .get(index + 1)
                        .and_then(|(state, frames)| (*state == State::Neutral).then_some(*frames))
                        .unwrap_or(0);
                    if neutral_frames > 10_000 {
                        return Err(SearchError::NonCanonicalTape(
                            "post-press neutral run exceeds the typed press limit".into(),
                        ));
                    }
                    let mut buttons = Vec::new();
                    for (button, button_mask) in [
                        (ControllerButton::A, BUTTON_A),
                        (ControllerButton::B, BUTTON_B),
                        (ControllerButton::Start, BUTTON_START),
                    ] {
                        if mask & button_mask != 0 {
                            buttons.push(button);
                        }
                    }
                    actions.push(MacroAction::Press {
                        buttons,
                        hold_frames,
                        neutral_frames,
                    });
                    index += if neutral_frames == 0 { 1 } else { 2 };
                }
            }
        }
        let candidate = Self {
            schema: CANDIDATE_SCHEMA.into(),
            segment,
            actions,
            ancestry: Ancestry {
                generation: 0,
                parent_id: None,
                mutation: Some("lossless absolute-tape import".into()),
            },
        };
        candidate.validate()?;
        if candidate.compile()? != *tape {
            return Err(SearchError::NonCanonicalTape(
                "typed inference did not reproduce the source tape exactly".into(),
            ));
        }
        Ok(candidate)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationManifest {
    pub schema: String,
    pub segment: SegmentProfile,
    pub generation: u32,
    pub rng_seed: u64,
    pub members: Vec<PopulationMember>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationMember {
    pub candidate_id: String,
    pub candidate_file: PathBuf,
    pub tape_file: PathBuf,
    pub frame_count: u64,
    pub ancestry: Ancestry,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResults {
    pub schema: String,
    pub segment: SegmentProfile,
    /// Results are keyed by the content ID from the population manifest.
    pub candidates: BTreeMap<String, CandidateResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateResult {
    /// Deepest memory-backed milestone reached by this candidate.
    pub milestone_depth: u16,
    /// Repeated restores/evaluations attempted.
    pub attempts: u32,
    /// Trials which reached `milestone_depth`.
    pub successes: u32,
    /// First-hit ticks for successful trials. Empty when no milestone was hit.
    #[serde(default)]
    pub first_hit_ticks: Vec<u64>,
}

/// One JSON artifact emitted by the native/PowerShell evaluator.
#[derive(Clone, Debug, Deserialize)]
pub struct EvaluationArtifact {
    pub schema_version: u32,
    pub candidate_id: String,
    pub search_result: CandidateResult,
}

impl CandidateResult {
    pub fn validate(&self) -> Result<(), SearchError> {
        if self.attempts == 0
            || self.successes > self.attempts
            || self.first_hit_ticks.len() != self.successes as usize
            || !(self.successes == 0 || self.successes == self.attempts)
            || (self.milestone_depth == 0) != (self.successes == 0)
            || self
                .first_hit_ticks
                .windows(2)
                .any(|ticks| ticks[0] != ticks[1])
        {
            return Err(SearchError::InvalidResult);
        }
        Ok(())
    }

    pub fn score(&self) -> Result<LexicographicScore, SearchError> {
        self.validate()?;
        let mut ticks = self.first_hit_ticks.clone();
        ticks.sort_unstable();
        let median_first_hit_tick = if ticks.is_empty() {
            u64::MAX
        } else {
            ticks[(ticks.len() - 1) / 2]
        };
        Ok(LexicographicScore {
            milestone_depth: self.milestone_depth,
            successes: self.successes,
            attempts: self.attempts,
            median_first_hit_tick,
            best_first_hit_tick: ticks.first().copied().unwrap_or(u64::MAX),
        })
    }
}

/// Higher is better under `Ord`: depth, then earlier first hit. Repeat
/// stability is an evaluator invariant and never a ranking dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LexicographicScore {
    pub milestone_depth: u16,
    pub successes: u32,
    pub attempts: u32,
    pub median_first_hit_tick: u64,
    pub best_first_hit_tick: u64,
}

impl Ord for LexicographicScore {
    fn cmp(&self, other: &Self) -> Ordering {
        self.milestone_depth
            .cmp(&other.milestone_depth)
            .then_with(|| other.median_first_hit_tick.cmp(&self.median_first_hit_tick))
            .then_with(|| other.best_first_hit_tick.cmp(&self.best_first_hit_tick))
    }
}

impl PartialOrd for LexicographicScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LeaderboardEntry {
    pub rank: usize,
    pub candidate_id: String,
    pub frame_count: u64,
    pub score: LexicographicScore,
}

#[derive(Clone, Copy, Debug)]
pub struct EvolutionConfig {
    pub population_size: usize,
    pub elite_count: usize,
    pub rng_seed: u64,
}

pub fn rank_population(
    manifest: &PopulationManifest,
    results: &SearchResults,
) -> Result<Vec<LeaderboardEntry>, SearchError> {
    validate_pair(manifest, results)?;
    let mut rows = manifest
        .members
        .iter()
        .filter_map(|member| {
            results
                .candidates
                .get(&member.candidate_id)
                .map(|result| (member, result))
        })
        .map(|(member, result)| {
            Ok(LeaderboardEntry {
                rank: 0,
                candidate_id: member.candidate_id.clone(),
                frame_count: member.frame_count,
                score: result.score()?,
            })
        })
        .collect::<Result<Vec<_>, SearchError>>()?;
    rows.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.frame_count.cmp(&right.frame_count))
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    for (index, row) in rows.iter_mut().enumerate() {
        row.rank = index + 1;
    }
    Ok(rows)
}

/// Collects one or more evaluator artifacts into the generation result schema.
/// If trials reached different depths, success rate and ticks are measured at
/// the deepest observed milestone while all trials remain in the denominator.
pub fn collect_results(
    manifest: &PopulationManifest,
    artifacts: impl IntoIterator<Item = EvaluationArtifact>,
) -> Result<SearchResults, SearchError> {
    if manifest.schema != POPULATION_SCHEMA {
        return Err(SearchError::InvalidSchema(manifest.schema.clone()));
    }
    let member_ids: HashSet<_> = manifest
        .members
        .iter()
        .map(|member| member.candidate_id.as_str())
        .collect();
    let mut candidates: BTreeMap<String, CandidateResult> = BTreeMap::new();
    for artifact in artifacts {
        if artifact.schema_version != 1 {
            return Err(SearchError::InvalidEvaluationSchema(
                artifact.schema_version,
            ));
        }
        if !member_ids.contains(artifact.candidate_id.as_str()) {
            return Err(SearchError::UnknownCandidate(artifact.candidate_id));
        }
        artifact.search_result.validate()?;
        match candidates.entry(artifact.candidate_id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(artifact.search_result);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let current = entry.get_mut();
                if current.milestone_depth != artifact.search_result.milestone_depth
                    || (current.successes == 0) != (artifact.search_result.successes == 0)
                    || current.first_hit_ticks.first()
                        != artifact.search_result.first_hit_ticks.first()
                {
                    return Err(SearchError::InvalidResult);
                }
                let total_attempts = current
                    .attempts
                    .checked_add(artifact.search_result.attempts)
                    .ok_or(SearchError::InvalidResult)?;
                current.successes = current
                    .successes
                    .checked_add(artifact.search_result.successes)
                    .ok_or(SearchError::InvalidResult)?;
                current
                    .first_hit_ticks
                    .extend(artifact.search_result.first_hit_ticks);
                current.attempts = total_attempts;
                current.validate()?;
            }
        }
    }
    Ok(SearchResults {
        schema: RESULTS_SCHEMA.into(),
        segment: manifest.segment,
        candidates,
    })
}

pub fn write_seed_population(
    output: &Path,
    base: Candidate,
    population_size: usize,
    rng_seed: u64,
) -> Result<PopulationManifest, SearchError> {
    if population_size == 0 {
        return Err(SearchError::InvalidPopulation);
    }
    base.validate()?;
    let mut rng = SplitMix64::new(rng_seed);
    let mut candidates = vec![base.clone()];
    let mut ids = HashSet::from([base.id()?]);
    let mut attempts = 0;
    while candidates.len() < population_size {
        attempts += 1;
        if attempts > population_size * 100 {
            return Err(SearchError::PopulationStalled);
        }
        let child = mutate(&base, 0, &mut rng)?;
        if ids.insert(child.id()?) {
            candidates.push(child);
        }
    }
    write_population(output, base.segment, 0, rng_seed, candidates)
}

/// Writes an exact, caller-supplied population without applying mutation.
/// This is used by deterministic reducers which construct every candidate.
pub fn write_explicit_population(
    output: &Path,
    segment: SegmentProfile,
    generation: u32,
    candidates: Vec<Candidate>,
) -> Result<PopulationManifest, SearchError> {
    if candidates.is_empty() {
        return Err(SearchError::InvalidPopulation);
    }
    let mut ids = HashSet::new();
    for candidate in &candidates {
        candidate.validate()?;
        if candidate.segment != segment || !ids.insert(candidate.id()?) {
            return Err(SearchError::InvalidPopulation);
        }
    }
    write_population(output, segment, generation, 0, candidates)
}

pub fn evolve_population(
    source_manifest_path: &Path,
    results: &SearchResults,
    output: &Path,
    config: EvolutionConfig,
) -> Result<PopulationManifest, SearchError> {
    if config.population_size == 0
        || config.elite_count == 0
        || config.elite_count > config.population_size
    {
        return Err(SearchError::InvalidPopulation);
    }
    let source: PopulationManifest = serde_json::from_slice(&fs::read(source_manifest_path)?)?;
    validate_pair(&source, results)?;
    let leaderboard = rank_population(&source, results)?;
    if leaderboard.is_empty() {
        return Err(SearchError::NoEvaluatedCandidates);
    }
    let root = source_manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let member_by_id: BTreeMap<_, _> = source
        .members
        .iter()
        .map(|member| (member.candidate_id.as_str(), member))
        .collect();
    let elite_count = config.elite_count.min(leaderboard.len());
    let mut elites = Vec::with_capacity(elite_count);
    for row in leaderboard.iter().take(elite_count) {
        let member = member_by_id[row.candidate_id.as_str()];
        let candidate: Candidate =
            serde_json::from_slice(&fs::read(root.join(&member.candidate_file))?)?;
        candidate.validate()?;
        elites.push(candidate);
    }
    let generation = source.generation + 1;
    let mut next = elites.clone();
    let mut ids = next
        .iter()
        .map(Candidate::id)
        .collect::<Result<HashSet<_>, _>>()?;
    let mut rng = SplitMix64::new(config.rng_seed);
    let mut attempts = 0;
    while next.len() < config.population_size {
        attempts += 1;
        if attempts > config.population_size * 200 {
            return Err(SearchError::PopulationStalled);
        }
        let parent_index = rng.usize(elites.len());
        let child = mutate(&elites[parent_index], generation, &mut rng)?;
        if ids.insert(child.id()?) {
            next.push(child);
        }
    }
    write_population(output, source.segment, generation, config.rng_seed, next)
}

fn validate_pair(
    manifest: &PopulationManifest,
    results: &SearchResults,
) -> Result<(), SearchError> {
    if manifest.schema != POPULATION_SCHEMA || results.schema != RESULTS_SCHEMA {
        return Err(SearchError::InvalidSchema(format!(
            "{} / {}",
            manifest.schema, results.schema
        )));
    }
    if manifest.segment != results.segment {
        return Err(SearchError::SegmentMismatch);
    }
    for result in results.candidates.values() {
        result.validate()?;
    }
    Ok(())
}

fn write_population(
    output: &Path,
    segment: SegmentProfile,
    generation: u32,
    rng_seed: u64,
    candidates: Vec<Candidate>,
) -> Result<PopulationManifest, SearchError> {
    fs::create_dir_all(output)?;
    let mut members = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if candidate.segment != segment {
            return Err(SearchError::SegmentMismatch);
        }
        let id = candidate.id()?;
        let candidate_file = PathBuf::from(format!("{id}.candidate.json"));
        let tape_file = PathBuf::from(format!("{id}.tape"));
        fs::write(
            output.join(&candidate_file),
            serde_json::to_vec_pretty(&candidate)?,
        )?;
        fs::write(output.join(&tape_file), candidate.compile()?.encode()?)?;
        members.push(PopulationMember {
            candidate_id: id,
            candidate_file,
            tape_file,
            frame_count: candidate.frame_count(),
            ancestry: candidate.ancestry,
        });
    }
    let manifest = PopulationManifest {
        schema: POPULATION_SCHEMA.into(),
        segment,
        generation,
        rng_seed,
        members,
    };
    fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

fn mutate(
    parent: &Candidate,
    generation: u32,
    rng: &mut SplitMix64,
) -> Result<Candidate, SearchError> {
    let mut child = parent.clone();
    let parent_id = parent.id()?;
    let route = child.segment == SegmentProfile::Fsp103ToFsp104;
    let mutation_kind = if route { rng.usize(7) } else { rng.usize(3) };
    let description;
    match mutation_kind {
        0 => {
            let index = rng.usize(child.actions.len());
            let delta_limit = (24_i32 - i32::try_from(generation.min(18)).unwrap()).max(3);
            let delta = rng.signed(delta_limit);
            change_duration(&mut child.actions[index], delta);
            description = format!("duration[{index}]{delta:+}");
        }
        1 if !route => {
            let presses: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| matches!(action, MacroAction::Press { .. }))
                .map(|(index, _)| index)
                .collect();
            if presses.is_empty() {
                return Err(SearchError::PopulationStalled);
            }
            let index = presses[rng.usize(presses.len())];
            let delta_limit = (32_i32 - i32::try_from(generation.min(24)).unwrap()).max(8);
            let delta = rng.signed(delta_limit);
            change_duration(&mut child.actions[index], delta);
            description = format!("boot_gap[{index}]{delta:+}");
        }
        1 if route => {
            let movable: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| {
                    matches!(action, MacroAction::Move { .. } | MacroAction::Roll { .. })
                })
                .map(|(index, _)| index)
                .collect();
            if movable.is_empty() {
                return Err(SearchError::PopulationStalled);
            }
            let index = movable[rng.usize(movable.len())];
            let delta_limit = (12_i32 - i32::try_from(generation.min(9)).unwrap()).max(2);
            let delta = rng.signed(delta_limit);
            change_angle(&mut child.actions[index], delta);
            description = format!("angle[{index}]{delta:+}");
        }
        2 if !route => {
            let shrinkable: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter_map(|(index, action)| match action {
                    MacroAction::Press { neutral_frames, .. } if *neutral_frames > 0 => Some(index),
                    _ => None,
                })
                .collect();
            if shrinkable.is_empty() {
                return Err(SearchError::PopulationStalled);
            }
            let index = shrinkable[rng.usize(shrinkable.len())];
            let delta = -i32::try_from(1 + rng.usize(48)).unwrap();
            change_duration(&mut child.actions[index], delta);
            description = format!("boot_shrink[{index}]{delta:+}");
        }
        2 if route => {
            let movable: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| {
                    matches!(action, MacroAction::Move { .. } | MacroAction::Roll { .. })
                })
                .map(|(index, _)| index)
                .collect();
            let index = movable[rng.usize(movable.len())];
            let delta = rng.signed(12);
            change_magnitude(&mut child.actions[index], delta);
            description = format!("magnitude[{index}]{delta:+}");
        }
        3 if route => {
            let moves: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter_map(|(index, action)| match action {
                    MacroAction::Move {
                        angle_degrees,
                        magnitude,
                        ..
                    } => Some((index, *angle_degrees, *magnitude)),
                    _ => None,
                })
                .collect();
            let (index, angle_degrees, magnitude) = moves[rng.usize(moves.len())];
            child.actions.insert(
                index,
                MacroAction::Roll {
                    angle_degrees,
                    magnitude,
                    recovery_frames: 8 + rng.usize(10) as u32,
                },
            );
            description = format!("insert_roll[{index}]");
        }
        4 if route && child.actions.len() > 1 => {
            let index = rng.usize(child.actions.len());
            child.actions.remove(index);
            description = format!("delete[{index}]");
        }
        5 if route => {
            let moves: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter_map(|(index, action)| match action {
                    MacroAction::Move {
                        angle_degrees,
                        magnitude,
                        frames,
                    } if *frames >= 4 => Some((index, *angle_degrees, *magnitude, *frames)),
                    _ => None,
                })
                .collect();
            if moves.is_empty() {
                return mutate(parent, generation, rng);
            }
            let (index, angle, magnitude, frames) = moves[rng.usize(moves.len())];
            let first = frames / 2;
            child.actions[index] = MacroAction::Move {
                angle_degrees: angle,
                magnitude,
                frames: first,
            };
            child.actions.insert(
                index + 1,
                MacroAction::Move {
                    angle_degrees: normalize_angle(i32::from(angle) + rng.signed(8)),
                    magnitude,
                    frames: frames - first,
                },
            );
            description = format!("split_move[{index}]");
        }
        _ => {
            let neutral: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| matches!(action, MacroAction::Neutral { .. }))
                .map(|(index, _)| index)
                .collect();
            let index = if neutral.is_empty() {
                rng.usize(child.actions.len())
            } else {
                neutral[rng.usize(neutral.len())]
            };
            let delta = rng.signed(48);
            change_duration(&mut child.actions[index], delta);
            description = format!("timing[{index}]{delta:+}");
        }
    }
    child.ancestry = Ancestry {
        generation,
        parent_id: Some(parent_id),
        mutation: Some(description),
    };
    child.validate()?;
    Ok(child)
}

fn change_duration(action: &mut MacroAction, delta: i32) {
    fn adjusted(value: u32, delta: i32, minimum: u32) -> u32 {
        (i64::from(value) + i64::from(delta)).clamp(i64::from(minimum), 10_000) as u32
    }
    match action {
        MacroAction::Move { frames, .. } | MacroAction::Neutral { frames } => {
            *frames = adjusted(*frames, delta, 1)
        }
        MacroAction::Roll {
            recovery_frames, ..
        } => *recovery_frames = adjusted(*recovery_frames, delta, 0).min(300),
        MacroAction::Press { neutral_frames, .. } => {
            *neutral_frames = adjusted(*neutral_frames, delta, 0)
        }
    }
}

fn change_angle(action: &mut MacroAction, delta: i32) {
    match action {
        MacroAction::Move { angle_degrees, .. } | MacroAction::Roll { angle_degrees, .. } => {
            *angle_degrees = normalize_angle(i32::from(*angle_degrees) + delta)
        }
        _ => {}
    }
}

fn change_magnitude(action: &mut MacroAction, delta: i32) {
    match action {
        MacroAction::Move { magnitude, .. } | MacroAction::Roll { magnitude, .. } => {
            *magnitude = (i32::from(*magnitude) + delta).clamp(1, 127) as u8
        }
        _ => {}
    }
}

fn normalize_angle(angle: i32) -> i16 {
    (angle + 180).rem_euclid(360) as i16 - 180
}

fn boot_baseline() -> Vec<MacroAction> {
    let neutral = |frames| MacroAction::Neutral { frames };
    let press = |button, neutral_frames| MacroAction::Press {
        buttons: vec![button],
        hold_frames: 1,
        neutral_frames,
    };
    vec![
        neutral(115),
        press(ControllerButton::Start, 31),
        press(ControllerButton::Start, 252),
        press(ControllerButton::A, 7),
        press(ControllerButton::Start, 29),
        press(ControllerButton::Start, 1),
        press(ControllerButton::Start, 67),
        press(ControllerButton::Start, 1),
        press(ControllerButton::Start, 65),
        press(ControllerButton::Start, 1),
        press(ControllerButton::Start, 115),
    ]
}

fn route_baseline() -> Vec<MacroAction> {
    vec![
        // Direct-stage processes still need time to construct the room and
        // player. This is deliberately part of the candidate so evolution can
        // safely drive it downward and the resulting tape remains standalone.
        MacroAction::Neutral { frames: 180 },
        MacroAction::Move {
            angle_degrees: 0,
            magnitude: 127,
            frames: 45,
        },
        MacroAction::Move {
            angle_degrees: 90,
            magnitude: 127,
            frames: 14,
        },
        MacroAction::Move {
            angle_degrees: 0,
            magnitude: 127,
            frames: 20,
        },
        MacroAction::Move {
            angle_degrees: 90,
            magnitude: 127,
            frames: 10,
        },
        MacroAction::Move {
            angle_degrees: 0,
            magnitude: 127,
            frames: 316,
        },
        MacroAction::Neutral { frames: 80 },
    ]
}

fn validate_magnitude(magnitude: u8) -> Result<(), SearchError> {
    if magnitude <= 127 {
        Ok(())
    } else {
        Err(SearchError::InvalidMagnitude(magnitude))
    }
}

fn validate_duration(frames: u32) -> Result<(), SearchError> {
    if (1..=10_000).contains(&frames) {
        Ok(())
    } else {
        Err(SearchError::InvalidDuration(frames))
    }
}

fn owned_frame(pad: RawPadState) -> InputFrame {
    let mut frame = InputFrame {
        owned_ports: 0x0f,
        ..InputFrame::default()
    };
    frame.pads[0] = pad;
    frame
}

fn move_frame(angle_degrees: i16, magnitude: u8, buttons: u16) -> InputFrame {
    let radians = f64::from(angle_degrees).to_radians();
    let magnitude = f64::from(magnitude);
    let pad = RawPadState {
        buttons,
        stick_x: (radians.sin() * magnitude).round().clamp(-127.0, 127.0) as i8,
        stick_y: (radians.cos() * magnitude).round().clamp(-127.0, 127.0) as i8,
        ..RawPadState::default()
    };
    owned_frame(pad)
}

fn push_frames(output: &mut Vec<InputFrame>, frame: InputFrame, count: u32) {
    output.extend(std::iter::repeat_n(frame, count as usize));
}

#[derive(Clone, Copy, Debug)]
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }

    fn usize(&mut self, exclusive_max: usize) -> usize {
        debug_assert!(exclusive_max > 0);
        (self.next() % exclusive_max as u64) as usize
    }

    fn signed(&mut self, inclusive_magnitude: i32) -> i32 {
        let width = u64::try_from(inclusive_magnitude * 2 + 1).unwrap();
        (self.next() % width) as i32 - inclusive_magnitude
    }
}

#[derive(Debug)]
pub enum SearchError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Tape(crate::tape::TapeError),
    InvalidSchema(String),
    InvalidSegment(String),
    SegmentMismatch,
    InvalidActionCount(usize),
    InvalidMagnitude(u8),
    InvalidDuration(u32),
    InvalidButtonPress,
    TooManyFrames,
    InvalidPopulation,
    PopulationStalled,
    InvalidResult,
    InvalidEvaluationSchema(u32),
    UnknownCandidate(String),
    NoEvaluatedCandidates,
    NonCanonicalTape(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "search I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "invalid search JSON: {error}"),
            Self::Tape(error) => write!(formatter, "could not encode candidate tape: {error}"),
            Self::InvalidSchema(schema) => {
                write!(formatter, "unsupported search schema {schema:?}")
            }
            Self::InvalidSegment(segment) => {
                write!(formatter, "unknown search segment {segment:?}")
            }
            Self::SegmentMismatch => formatter.write_str("population and results segment mismatch"),
            Self::InvalidActionCount(count) => {
                write!(formatter, "invalid candidate action count {count}")
            }
            Self::InvalidMagnitude(value) => {
                write!(formatter, "stick magnitude {value} exceeds 127")
            }
            Self::InvalidDuration(value) => write!(formatter, "invalid macro duration {value}"),
            Self::InvalidButtonPress => formatter.write_str("invalid button press macro"),
            Self::TooManyFrames => formatter.write_str("candidate expands beyond the frame limit"),
            Self::InvalidPopulation => formatter.write_str("invalid population/elite count"),
            Self::PopulationStalled => {
                formatter.write_str("could not generate a unique population")
            }
            Self::InvalidResult => formatter.write_str("candidate result counts are inconsistent"),
            Self::InvalidEvaluationSchema(version) => {
                write!(formatter, "unsupported evaluator schema version {version}")
            }
            Self::UnknownCandidate(id) => {
                write!(formatter, "evaluation references unknown candidate {id}")
            }
            Self::NoEvaluatedCandidates => {
                formatter.write_str("results contain no population candidates")
            }
            Self::NonCanonicalTape(message) => {
                write!(formatter, "raw tape cannot be inferred safely: {message}")
            }
        }
    }
}

impl Error for SearchError {}

impl From<std::io::Error> for SearchError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for SearchError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<crate::tape::TapeError> for SearchError {
    fn from(value: crate::tape::TapeError) -> Self {
        Self::Tape(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_ir_compiles_analog_roll_and_press() {
        let candidate = Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            segment: SegmentProfile::Fsp103ToFsp104,
            actions: vec![
                MacroAction::Move {
                    angle_degrees: 90,
                    magnitude: 127,
                    frames: 2,
                },
                MacroAction::Roll {
                    angle_degrees: 0,
                    magnitude: 100,
                    recovery_frames: 2,
                },
                MacroAction::Press {
                    buttons: vec![ControllerButton::Start],
                    hold_frames: 1,
                    neutral_frames: 1,
                },
            ],
            ancestry: Ancestry::default(),
        };
        let tape = candidate.compile().unwrap();
        assert_eq!(tape.frames.len(), 7);
        assert_eq!(tape.frames[0].pads[0].stick_x, 127);
        assert_eq!(tape.frames[0].pads[0].stick_y, 0);
        assert_eq!(tape.frames[2].pads[0].buttons, BUTTON_B);
        assert_eq!(tape.frames[2].pads[0].stick_y, 100);
        assert_eq!(tape.frames[5].pads[0].buttons, BUTTON_START);
        assert_eq!(tape.frames[6].pads[0].buttons, 0);
    }

    #[test]
    fn absolute_boot_tape_inference_is_lossless_and_rejects_analog() {
        let source = Candidate::baseline(SegmentProfile::BootToFsp103)
            .compile()
            .unwrap();
        let imported =
            Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &source).unwrap();
        assert_eq!(imported.compile().unwrap(), source);
        assert!(
            imported
                .actions
                .iter()
                .any(|action| matches!(action, MacroAction::Press { .. }))
        );

        let analog = Candidate::baseline(SegmentProfile::Fsp103ToFsp104)
            .compile()
            .unwrap();
        assert!(matches!(
            Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &analog),
            Err(SearchError::NonCanonicalTape(_))
        ));
    }

    #[test]
    fn boot_mutation_directly_targets_press_gaps() {
        let parent = Candidate::baseline(SegmentProfile::BootToFsp103);
        let mut rng = SplitMix64::new(0x5eed);
        let mut gap_mutations = 0;
        let mut shrink_mutations = 0;
        for _ in 0..256 {
            let child = mutate(&parent, 0, &mut rng).unwrap();
            let mutation = child.ancestry.mutation.as_deref().unwrap();
            if mutation.starts_with("boot_gap[") || mutation.starts_with("boot_shrink[") {
                let changed: Vec<_> = parent
                    .actions
                    .iter()
                    .zip(&child.actions)
                    .enumerate()
                    .filter(|(_, (before, after))| before != after)
                    .collect();
                assert!(changed.len() <= 1);
                if let Some((_, (_, action))) = changed.first() {
                    assert!(matches!(action, MacroAction::Press { .. }));
                }
            }
            gap_mutations += usize::from(mutation.starts_with("boot_gap["));
            shrink_mutations += usize::from(mutation.starts_with("boot_shrink["));
        }
        assert!(gap_mutations > 50);
        assert!(shrink_mutations > 50);
    }

    #[test]
    fn score_is_depth_then_tick_and_fractional_repeats_are_invalid() {
        let score = |depth, successes, attempts, ticks| {
            CandidateResult {
                milestone_depth: depth,
                attempts,
                successes,
                first_hit_ticks: ticks,
            }
            .score()
            .unwrap()
        };
        assert!(score(4, 10, 10, vec![500; 10]) > score(3, 10, 10, vec![1; 10]));
        assert!(score(4, 10, 10, vec![99; 10]) > score(4, 10, 10, vec![100; 10]));
        assert!(matches!(
            CandidateResult {
                milestone_depth: 4,
                attempts: 10,
                successes: 9,
                first_hit_ticks: vec![500; 9],
            }
            .score(),
            Err(SearchError::InvalidResult)
        ));
    }

    #[test]
    fn evaluator_trials_reject_disagreement() {
        let candidate = Candidate::baseline(SegmentProfile::Fsp103ToFsp104);
        let candidate_id = candidate.id().unwrap();
        let manifest = PopulationManifest {
            schema: POPULATION_SCHEMA.into(),
            segment: candidate.segment,
            generation: 0,
            rng_seed: 1,
            members: vec![PopulationMember {
                candidate_id: candidate_id.clone(),
                candidate_file: PathBuf::from("candidate.json"),
                tape_file: PathBuf::from("candidate.tape"),
                frame_count: candidate.frame_count(),
                ancestry: Ancestry::default(),
            }],
        };
        let artifact = |depth, tick| EvaluationArtifact {
            schema_version: 1,
            candidate_id: candidate_id.clone(),
            search_result: CandidateResult {
                milestone_depth: depth,
                attempts: 1,
                successes: 1,
                first_hit_ticks: vec![tick],
            },
        };
        assert!(matches!(
            collect_results(&manifest, [artifact(3, 570), artifact(4, 603)]),
            Err(SearchError::InvalidResult)
        ));
    }

    #[test]
    fn seeded_evolution_is_reproducible_and_keeps_champion() {
        let root = std::env::temp_dir().join(format!("huntctl-search-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = write_seed_population(
            &root.join("g0"),
            Candidate::baseline(SegmentProfile::Fsp103ToFsp104),
            8,
            42,
        )
        .unwrap();
        let champion = first.members[3].candidate_id.clone();
        let candidates = first
            .members
            .iter()
            .enumerate()
            .map(|(index, member)| {
                (
                    member.candidate_id.clone(),
                    CandidateResult {
                        milestone_depth: if index == 3 { 4 } else { 3 },
                        attempts: 2,
                        successes: 2,
                        first_hit_ticks: vec![100 + index as u64; 2],
                    },
                )
            })
            .collect();
        let results = SearchResults {
            schema: RESULTS_SCHEMA.into(),
            segment: first.segment,
            candidates,
        };
        let config = EvolutionConfig {
            population_size: 8,
            elite_count: 2,
            rng_seed: 99,
        };
        let next = evolve_population(
            &root.join("g0/manifest.json"),
            &results,
            &root.join("g1"),
            config,
        )
        .unwrap();
        let again = evolve_population(
            &root.join("g0/manifest.json"),
            &results,
            &root.join("g1-again"),
            config,
        )
        .unwrap();
        assert_eq!(next.members[0].candidate_id, champion);
        assert_eq!(
            next.members
                .iter()
                .map(|member| &member.candidate_id)
                .collect::<Vec<_>>(),
            again
                .members
                .iter()
                .map(|member| &member.candidate_id)
                .collect::<Vec<_>>()
        );
        assert!(next.members.iter().skip(2).all(|member| {
            member.ancestry.generation == 1 && member.ancestry.parent_id.is_some()
        }));
        fs::remove_dir_all(root).unwrap();
    }
}
