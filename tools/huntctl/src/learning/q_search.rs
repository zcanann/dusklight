//! Finite-batch Q-guided proposals for deterministic movement tapes.
//!
//! This module never scores or promotes gameplay. It consumes already proved
//! episode corpora, proposes small state-aware tape interventions, and hands
//! ordinary candidates back to the native evaluator.

use crate::action_guidance::{
    ACTION_GUIDANCE_SCHEMA_V1, AdvisoryActionMask, movement_action_mask_v1,
};
use crate::episode::EpisodeOutcomeClass;
use crate::fqi::{FittedQ, FqiConfig, QEstimate, Transition as FqiTransition};
use crate::offline_rl::{
    MOVEMENT_ACTION_COUNT_V2, MOVEMENT_CATEGORICAL_FEATURES_V1, canonical_movement_pad_v2,
    movement_action_id_v2, movement_action_schema_digest_v2, movement_feature_schema_digest_v1,
};
use crate::search::{Ancestry, Candidate, InterventionRange};
use crate::transition_corpus::TransitionCorpus;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::error::Error;
use std::fmt;

const MAX_PROPOSAL_STATES: usize = 4096;
const EXPLORATION_WEIGHT: f64 = 1.5;
const UNMASKED_Q_PROBE_INTERVAL: usize = 4;

#[derive(Clone, Debug)]
pub struct QEpisode {
    pub candidate: Candidate,
    pub corpus: TransitionCorpus,
    pub outcome: EpisodeOutcomeClass,
}

#[derive(Clone, Copy, Debug)]
pub struct QProposalConfig {
    pub generation: u32,
    pub max_proposals: usize,
    pub iterations: usize,
    pub trees_per_action: usize,
    pub seed: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct QProposalSummary {
    pub schema: &'static str,
    pub training_transitions: usize,
    pub training_actions: usize,
    pub proposal_states: usize,
    pub action_guidance_schema: &'static str,
    pub state_masked_proposal_states: usize,
    pub guided_action_evaluations: usize,
    pub unmasked_action_evaluations: usize,
    pub unmasked_q_probe_states: usize,
    pub guided_exploit_interventions: usize,
    pub unmasked_exploratory_interventions: usize,
    pub proposals: usize,
    pub coverage: CollectionCoverage,
    pub proposer_attribution: Vec<ProposerAttribution>,
    pub policy_collapse_audit: PolicyCollapseAudit,
}

#[derive(Clone, Debug, Serialize)]
pub struct CollectionCoverage {
    pub episodes: usize,
    pub effective_decisions: usize,
    pub stage_rooms: BTreeMap<String, u64>,
    pub spatial_cells: BTreeMap<String, u64>,
    pub player_procedures: BTreeMap<String, u64>,
    pub options: BTreeMap<String, u64>,
    pub parameter_bins: BTreeMap<String, u64>,
    pub durations: BTreeMap<String, u64>,
    pub goal_phases: BTreeMap<String, u64>,
    pub outcomes: BTreeMap<String, u64>,
    pub action_support: BTreeMap<u32, u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProposerAttribution {
    pub proposer: &'static str,
    pub requested_budget: usize,
    pub available_interventions: usize,
    pub generated_candidates: usize,
    pub uncertainty_is_heuristic: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PolicyCollapseAudit {
    pub unique_parent_episodes: usize,
    pub available_parent_episodes: usize,
    pub unique_proposed_actions: usize,
    pub action_catalog_size: u32,
    pub single_parent_collapse: bool,
    pub single_action_collapse: bool,
}

#[derive(Clone, Debug)]
pub struct QProposalBatch {
    pub candidates: Vec<Candidate>,
    pub summary: QProposalSummary,
}

#[derive(Debug)]
pub struct QSearchError(String);

impl QSearchError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for QSearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for QSearchError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProposalKind {
    GuidedExploit,
    EnsembleDisagreement,
    SystematicAlternate,
    RandomProbe,
    LatinHypercube,
}

impl ProposalKind {
    fn name(self) -> &'static str {
        match self {
            Self::GuidedExploit => "guided_exploit",
            Self::EnsembleDisagreement => "ensemble_disagreement",
            Self::SystematicAlternate => "systematic_alternate",
            Self::RandomProbe => "random_probe",
            Self::LatinHypercube => "latin_hypercube",
        }
    }

    fn mutation_prefix(self) -> &'static str {
        match self {
            Self::GuidedExploit => "q_guided",
            Self::EnsembleDisagreement => "q_disagreement_heuristic",
            Self::SystematicAlternate => "systematic_probe",
            Self::RandomProbe => "random_probe",
            Self::LatinHypercube => "latin_hypercube",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Intervention {
    episode: usize,
    frame: usize,
    action: u32,
    score: f64,
    kind: ProposalKind,
    width: usize,
}

pub fn propose_q_candidates(
    training_corpora: &[TransitionCorpus],
    episodes: &[QEpisode],
    config: QProposalConfig,
) -> Result<QProposalBatch, QSearchError> {
    if training_corpora.is_empty() || episodes.is_empty() || config.max_proposals == 0 {
        return Err(QSearchError::new(
            "Q proposals require training corpora, aligned parent episodes, and a nonzero budget",
        ));
    }
    let feature_schema = movement_feature_schema_digest_v1();
    let action_schema = movement_action_schema_digest_v2();
    let mut transitions = Vec::new();
    let mut episode_groups = Vec::new();
    let mut actions = BTreeSet::new();
    let mut next_episode_group = 0_u64;
    for corpus in training_corpora {
        corpus
            .validate()
            .map_err(|error| QSearchError::new(error.to_string()))?;
        if corpus.feature_schema != feature_schema
            || corpus.action_schema != action_schema
            || corpus.feature_count == 0
        {
            return Err(QSearchError::new(
                "Q proposal corpus does not use the authenticated movement schemas",
            ));
        }
        let mut ended_terminal = false;
        for transition in &corpus.transitions {
            actions.insert(transition.action.action_id);
            transitions.push(FqiTransition {
                state: transition.state.clone(),
                action: transition.action.action_id,
                duration: transition.duration_ticks,
                reward: transition.reward,
                next_state: transition.next_state.clone(),
                terminal: transition.terminal,
            });
            episode_groups.push(next_episode_group);
            ended_terminal = transition.terminal;
            if transition.terminal {
                next_episode_group = next_episode_group.checked_add(1).ok_or_else(|| {
                    QSearchError::new("Q proposal episode-group count overflowed")
                })?;
            }
        }
        if !ended_terminal {
            next_episode_group = next_episode_group
                .checked_add(1)
                .ok_or_else(|| QSearchError::new("Q proposal episode-group count overflowed"))?;
        }
    }
    let actions: Vec<_> = actions.into_iter().collect();
    let feature_width = training_corpora[0].feature_count as usize;
    let model = (actions.len() >= 2)
        .then(|| {
            FittedQ::fit_with_episode_groups(
                feature_width,
                &actions,
                &transitions,
                &episode_groups,
                &FqiConfig {
                    iterations: config.iterations,
                    trees_per_action: config.trees_per_action,
                    max_tree_depth: 8,
                    seed: config.seed,
                    categorical_features: MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec(),
                    ..FqiConfig::default()
                },
            )
            .map_err(|error| QSearchError::new(error.to_string()))
        })
        .transpose()?;

    let aligned = episodes
        .iter()
        .map(validate_episode_alignment)
        .collect::<Result<Vec<_>, _>>()?;
    let total_states: usize = aligned
        .iter()
        .map(|episode| episode.corpus.transitions.len())
        .sum();
    let stride = total_states.div_ceil(MAX_PROPOSAL_STATES).max(1);
    let mut exploit = Vec::new();
    let mut explore = Vec::new();
    let mut systematic = Vec::new();
    let mut random = Vec::new();
    let mut latin_hypercube = Vec::new();
    let mut considered = 0_usize;
    let mut state_masked = 0;
    let mut guided_action_evaluations = 0_usize;
    let mut unmasked_action_evaluations = 0_usize;
    let mut unmasked_q_probe_states = 0_usize;
    let mut ordinal = 0;
    let action_support = collection_action_support(training_corpora);
    let mut balanced_actions: Vec<u32> = (0..MOVEMENT_ACTION_COUNT_V2).collect();
    balanced_actions
        .sort_by_key(|action| (action_support.get(action).copied().unwrap_or(0), *action));
    for (episode_index, episode) in aligned.iter().enumerate() {
        for (frame, transition) in episode.corpus.transitions.iter().enumerate() {
            let take = ordinal % stride == 0;
            ordinal += 1;
            if !take {
                continue;
            }
            considered += 1;
            let guidance = movement_action_mask_v1(&transition.state)
                .map_err(|error| QSearchError::new(error.to_string()))?;
            state_masked += usize::from(
                (0..MOVEMENT_ACTION_COUNT_V2).any(|action| !guidance.recommends(action)),
            );
            if let Some(model) = &model {
                let current = model
                    .estimate(&transition.state, transition.action.action_id)
                    .map_err(|error| QSearchError::new(error.to_string()))?;
                let guided_actions = model
                    .actions()
                    .iter()
                    .copied()
                    .filter(|action| {
                        *action != transition.action.action_id && guidance.recommends(*action)
                    })
                    .collect::<Vec<_>>();
                guided_action_evaluations += guided_actions.len();
                let guided_alternatives = model
                    .rank_action_subset(&transition.state, &guided_actions)
                    .map_err(|error| QSearchError::new(error.to_string()))?
                    .into_iter()
                    .collect::<Vec<_>>();
                if let Some(best) = guided_exploit(&guided_alternatives, &guidance) {
                    exploit.push(Intervention {
                        episode: episode_index,
                        frame,
                        action: best.action,
                        score: best.mean - current.mean,
                        kind: ProposalKind::GuidedExploit,
                        width: [1, 2, 4][ordinal % 3],
                    });
                }
                if (considered - 1).is_multiple_of(UNMASKED_Q_PROBE_INTERVAL) {
                    unmasked_q_probe_states += 1;
                    let alternatives = model
                        .rank_actions(&transition.state)
                        .map_err(|error| QSearchError::new(error.to_string()))?
                        .into_iter()
                        .filter(|estimate| estimate.action != transition.action.action_id)
                        .collect::<Vec<_>>();
                    unmasked_action_evaluations += alternatives.len();
                    if let Some(best) = unmasked_explore(&alternatives, current.mean) {
                        explore.push(Intervention {
                            episode: episode_index,
                            frame,
                            action: best.action,
                            score: exploration_score(best, current.mean),
                            kind: ProposalKind::EnsembleDisagreement,
                            width: [1, 2, 4][(ordinal + 1) % 3],
                        });
                    }
                }
            }
            let systematic_action = balanced_actions
                .iter()
                .cycle()
                .skip(ordinal % balanced_actions.len())
                .find(|action| **action != transition.action.action_id)
                .copied()
                .expect("movement catalog has alternate actions");
            systematic.push(Intervention {
                episode: episode_index,
                frame,
                action: systematic_action,
                score: -(action_support.get(&systematic_action).copied().unwrap_or(0) as f64),
                kind: ProposalKind::SystematicAlternate,
                width: [1, 2, 4][ordinal % 3],
            });
            let random_word = mix_probe(config.seed, ordinal as u64);
            let mut random_action = (random_word % u64::from(MOVEMENT_ACTION_COUNT_V2)) as u32;
            if random_action == transition.action.action_id {
                random_action = (random_action + 1) % MOVEMENT_ACTION_COUNT_V2;
            }
            random.push(Intervention {
                episode: episode_index,
                frame,
                action: random_action,
                score: 0.0,
                kind: ProposalKind::RandomProbe,
                width: [1, 2, 4][(random_word as usize >> 8) % 3],
            });
            let stratum = considered - 1;
            let mut latin_action = ((stratum * MOVEMENT_ACTION_COUNT_V2 as usize
                / total_states.min(MAX_PROPOSAL_STATES).max(1))
                + config.seed as usize)
                % MOVEMENT_ACTION_COUNT_V2 as usize;
            if latin_action as u32 == transition.action.action_id {
                latin_action = (latin_action + 1) % MOVEMENT_ACTION_COUNT_V2 as usize;
            }
            latin_hypercube.push(Intervention {
                episode: episode_index,
                frame,
                action: latin_action as u32,
                score: 0.0,
                kind: ProposalKind::LatinHypercube,
                width: [1, 2, 4][stratum % 3],
            });
        }
    }
    sort_interventions(&mut exploit);
    sort_interventions(&mut explore);
    sort_interventions(&mut systematic);

    let mut candidates = Vec::new();
    let mut ids = episodes
        .iter()
        .map(|episode| {
            episode
                .candidate
                .id()
                .map_err(|error| QSearchError::new(error.to_string()))
        })
        .collect::<Result<HashSet<_>, _>>()?;
    let budgets = split_proposer_budget(config.max_proposals);
    let pools = [
        (ProposalKind::SystematicAlternate, &systematic, budgets[0]),
        (ProposalKind::EnsembleDisagreement, &explore, budgets[1]),
        (ProposalKind::RandomProbe, &random, budgets[2]),
        (ProposalKind::LatinHypercube, &latin_hypercube, budgets[3]),
        (ProposalKind::GuidedExploit, &exploit, budgets[4]),
    ];
    let mut proposer_attribution = Vec::new();
    for (kind, pool, budget) in pools {
        let before = candidates.len();
        append_interventions(
            pool,
            budget,
            &aligned,
            episodes,
            config.generation,
            &mut ids,
            &mut candidates,
        )?;
        proposer_attribution.push(ProposerAttribution {
            proposer: kind.name(),
            requested_budget: budget,
            available_interventions: pool.len(),
            generated_candidates: candidates.len() - before,
            uncertainty_is_heuristic: kind == ProposalKind::EnsembleDisagreement,
        });
    }

    let policy_collapse_audit = policy_collapse_audit(&candidates, episodes.len())?;
    Ok(QProposalBatch {
        summary: QProposalSummary {
            schema: "dusklight-q-proposals/v4",
            training_transitions: transitions.len(),
            training_actions: actions.len(),
            proposal_states: considered,
            action_guidance_schema: ACTION_GUIDANCE_SCHEMA_V1,
            state_masked_proposal_states: state_masked,
            guided_action_evaluations,
            unmasked_action_evaluations,
            unmasked_q_probe_states,
            guided_exploit_interventions: exploit.len(),
            unmasked_exploratory_interventions: explore.len(),
            proposals: candidates.len(),
            coverage: collection_coverage(episodes),
            proposer_attribution,
            policy_collapse_audit,
        },
        candidates,
    })
}

struct AlignedEpisode<'a> {
    corpus: &'a TransitionCorpus,
    tape: crate::tape::InputTape,
}

fn validate_episode_alignment(episode: &QEpisode) -> Result<AlignedEpisode<'_>, QSearchError> {
    episode
        .corpus
        .validate()
        .map_err(|error| QSearchError::new(error.to_string()))?;
    let tape = episode
        .candidate
        .compile()
        .map_err(|error| QSearchError::new(error.to_string()))?;
    if episode.corpus.transitions.is_empty() || episode.corpus.transitions.len() > tape.frames.len()
    {
        return Err(QSearchError::new(
            "Q parent episode is empty or longer than its candidate suffix",
        ));
    }
    for (index, transition) in episode.corpus.transitions.iter().enumerate() {
        let pad = tape.frames[index].pads[0];
        let expected = [
            i16::from(pad.stick_x),
            i16::from(pad.stick_y),
            pad.buttons as i16,
        ];
        if transition.duration_ticks != 1
            || transition.action.parameters != expected
            || movement_action_id_v2(pad) != Some(transition.action.action_id)
        {
            return Err(QSearchError::new(format!(
                "Q parent episode action {index} is not aligned with its candidate tape"
            )));
        }
    }
    Ok(AlignedEpisode {
        corpus: &episode.corpus,
        tape,
    })
}

fn split_proposer_budget(total: usize) -> [usize; 5] {
    let mut budgets = [total / 5; 5];
    for budget in budgets.iter_mut().take(total % 5) {
        *budget += 1;
    }
    budgets
}

#[allow(clippy::too_many_arguments)]
fn append_interventions(
    interventions: &[Intervention],
    budget: usize,
    aligned: &[AlignedEpisode<'_>],
    episodes: &[QEpisode],
    generation: u32,
    ids: &mut HashSet<String>,
    candidates: &mut Vec<Candidate>,
) -> Result<(), QSearchError> {
    let target = candidates.len().saturating_add(budget);
    for intervention in interventions {
        if candidates.len() >= target {
            break;
        }
        let mut tape = aligned[intervention.episode].tape.clone();
        let pad = canonical_movement_pad_v2(intervention.action)
            .ok_or_else(|| QSearchError::new("probe selected an unknown movement action"))?;
        let end = intervention
            .frame
            .saturating_add(intervention.width)
            .min(tape.frames.len());
        for frame in &mut tape.frames[intervention.frame..end] {
            frame.pads[0] = pad;
        }
        let parent = &episodes[intervention.episode].candidate;
        let mut candidate = Candidate::from_absolute_tape(parent.segment, &tape)
            .map_err(|error| QSearchError::new(error.to_string()))?;
        candidate.ancestry = Ancestry {
            generation,
            parent_id: Some(
                parent
                    .id()
                    .map_err(|error| QSearchError::new(error.to_string()))?,
            ),
            mutation: Some(format!(
                "{}[{}..{}]=action{} score={:.6}",
                intervention.kind.mutation_prefix(),
                intervention.frame,
                end,
                intervention.action,
                intervention.score
            )),
            intervention: Some(InterventionRange {
                start_frame: intervention.frame as u64,
                end_frame_exclusive: end as u64,
                parent_end_frame_exclusive: end as u64,
            }),
        };
        let id = candidate
            .id()
            .map_err(|error| QSearchError::new(error.to_string()))?;
        if ids.insert(id) {
            candidates.push(candidate);
        }
    }
    Ok(())
}

fn collection_action_support(corpora: &[TransitionCorpus]) -> BTreeMap<u32, u64> {
    let mut support = BTreeMap::new();
    for transition in corpora.iter().flat_map(|corpus| &corpus.transitions) {
        *support.entry(transition.action.action_id).or_default() += 1;
    }
    support
}

fn collection_coverage(episodes: &[QEpisode]) -> CollectionCoverage {
    let mut coverage = CollectionCoverage {
        episodes: episodes.len(),
        effective_decisions: 0,
        stage_rooms: BTreeMap::new(),
        spatial_cells: BTreeMap::new(),
        player_procedures: BTreeMap::new(),
        options: BTreeMap::new(),
        parameter_bins: BTreeMap::new(),
        durations: BTreeMap::new(),
        goal_phases: BTreeMap::new(),
        outcomes: BTreeMap::new(),
        action_support: BTreeMap::new(),
    };
    for episode in episodes {
        increment(&mut coverage.outcomes, outcome_name(episode.outcome).into());
        for transition in &episode.corpus.transitions {
            coverage.effective_decisions += 1;
            let state = &transition.state;
            let stage: String = state[..8]
                .iter()
                .map(|value| (value * 255.0).round().clamp(0.0, 255.0) as u8)
                .take_while(|byte| *byte != 0)
                .map(char::from)
                .collect();
            increment(
                &mut coverage.stage_rooms,
                format!("{stage}:{}", state[8].round() as i32),
            );
            increment(
                &mut coverage.spatial_cells,
                format!(
                    "{stage}:{}:{},{}",
                    state[8].round() as i32,
                    (state[17] * 32.0).floor() as i32,
                    (state[19] * 32.0).floor() as i32
                ),
            );
            increment(
                &mut coverage.player_procedures,
                (state[16].round() as i32).to_string(),
            );
            increment(&mut coverage.options, "pad_frame".into());
            increment(
                &mut coverage.parameter_bins,
                format!("action-{}", transition.action.action_id),
            );
            increment(
                &mut coverage.durations,
                transition.duration_ticks.to_string(),
            );
            let phase = if transition.terminal || state[48] <= 1.0 / 1024.0 {
                "terminal"
            } else if state[47] <= 1.0 / 1024.0 {
                "opening"
            } else {
                "middle"
            };
            increment(&mut coverage.goal_phases, phase.into());
            *coverage
                .action_support
                .entry(transition.action.action_id)
                .or_default() += 1;
        }
    }
    coverage
}

fn increment(counts: &mut BTreeMap<String, u64>, key: String) {
    *counts.entry(key).or_default() += 1;
}

fn outcome_name(outcome: EpisodeOutcomeClass) -> &'static str {
    match outcome {
        EpisodeOutcomeClass::Successful => "successful",
        EpisodeOutcomeClass::Failed => "failed",
        EpisodeOutcomeClass::Crashed => "crashed",
        EpisodeOutcomeClass::TimedOut => "timed_out",
        EpisodeOutcomeClass::Desynced => "desynced",
        EpisodeOutcomeClass::Unsupported => "unsupported",
        EpisodeOutcomeClass::Truncated => "truncated",
    }
}

fn mix_probe(seed: u64, ordinal: u64) -> u64 {
    let mut value = seed ^ ordinal.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn policy_collapse_audit(
    candidates: &[Candidate],
    available_parent_episodes: usize,
) -> Result<PolicyCollapseAudit, QSearchError> {
    let parents: BTreeSet<_> = candidates
        .iter()
        .filter_map(|candidate| candidate.ancestry.parent_id.as_deref())
        .collect();
    let mut actions = BTreeSet::new();
    for candidate in candidates {
        let Some(intervention) = &candidate.ancestry.intervention else {
            continue;
        };
        let tape = candidate
            .compile()
            .map_err(|error| QSearchError::new(error.to_string()))?;
        let frame = usize::try_from(intervention.start_frame)
            .map_err(|_| QSearchError::new("proposal intervention frame does not fit usize"))?;
        if let Some(action) = tape
            .frames
            .get(frame)
            .and_then(|frame| movement_action_id_v2(frame.pads[0]))
        {
            actions.insert(action);
        }
    }
    Ok(PolicyCollapseAudit {
        unique_parent_episodes: parents.len(),
        available_parent_episodes,
        unique_proposed_actions: actions.len(),
        action_catalog_size: MOVEMENT_ACTION_COUNT_V2,
        single_parent_collapse: candidates.len() > 1
            && available_parent_episodes > 1
            && parents.len() <= 1,
        single_action_collapse: candidates.len() > 1 && actions.len() <= 1,
    })
}

fn exploration_score(estimate: crate::fqi::QEstimate, current_mean: f64) -> f64 {
    estimate.mean - current_mean + EXPLORATION_WEIGHT * estimate.variance.max(0.0).sqrt()
}

fn guided_exploit(alternatives: &[QEstimate], guidance: &AdvisoryActionMask) -> Option<QEstimate> {
    alternatives
        .iter()
        .copied()
        .find(|estimate| guidance.recommends(estimate.action))
}

fn unmasked_explore(alternatives: &[QEstimate], current_mean: f64) -> Option<QEstimate> {
    alternatives.iter().copied().max_by(|left, right| {
        exploration_score(*left, current_mean).total_cmp(&exploration_score(*right, current_mean))
    })
}

fn sort_interventions(interventions: &mut [Intervention]) {
    interventions.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.episode.cmp(&right.episode))
            .then_with(|| left.frame.cmp(&right.frame))
            .then_with(|| left.action.cmp(&right.action))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_guidance::{ACTION_GUIDANCE_SCHEMA_V1, movement_action_mask_v1};
    use crate::artifact::Digest;
    use crate::search::SegmentProfile;
    use crate::tape::{InputFrame, InputTape, RawPadState};
    use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};

    fn corpus_for(candidate: &Candidate) -> TransitionCorpus {
        let tape = candidate.compile().unwrap();
        let transitions = tape
            .frames
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let action_id = movement_action_id_v2(frame.pads[0]).unwrap();
                let mut state = vec![0.0; 49];
                state[17] = index as f32;
                let mut next_state = state.clone();
                next_state[17] += 1.0;
                Transition {
                    source: StateReference {
                        kind: StateReferenceKind::Boundary,
                        digest: Digest([index as u8 + 1; 32]),
                    },
                    state,
                    action: MacroAction {
                        action_id,
                        macro_kind: 2,
                        parameters: vec![
                            i16::from(frame.pads[0].stick_x),
                            i16::from(frame.pads[0].stick_y),
                            frame.pads[0].buttons as i16,
                        ],
                    },
                    duration_ticks: 1,
                    reward: -1.0,
                    next: StateReference {
                        kind: StateReferenceKind::Boundary,
                        digest: Digest([index as u8 + 2; 32]),
                    },
                    next_state,
                    terminal: index + 1 == tape.frames.len(),
                }
            })
            .collect();
        TransitionCorpus::new(
            movement_feature_schema_digest_v1(),
            movement_action_schema_digest_v2(),
            49,
            transitions,
        )
        .unwrap()
    }

    #[test]
    fn fitted_q_proposals_are_deterministic_aligned_ordinary_candidates() {
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: (0..8)
                .map(|index| InputFrame {
                    owned_ports: 1,
                    pads: [
                        canonical_movement_pad_v2(if index % 2 == 0 { 0 } else { 18 }).unwrap(),
                        disconnected,
                        disconnected,
                        disconnected,
                    ],
                    ..InputFrame::default()
                })
                .collect(),
            ..InputTape::default()
        };
        let candidate =
            Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
        let corpus = corpus_for(&candidate);
        let episodes = [QEpisode {
            candidate: candidate.clone(),
            corpus: corpus.clone(),
            outcome: EpisodeOutcomeClass::Successful,
        }];
        let config = QProposalConfig {
            generation: 1,
            max_proposals: 4,
            iterations: 4,
            trees_per_action: 3,
            seed: 7,
        };
        let first = propose_q_candidates(std::slice::from_ref(&corpus), &episodes, config).unwrap();
        let second = propose_q_candidates(&[corpus], &episodes, config).unwrap();
        assert!(!first.candidates.is_empty());
        assert_eq!(first.summary.proposals, first.candidates.len());
        assert_eq!(
            first.summary.action_guidance_schema,
            ACTION_GUIDANCE_SCHEMA_V1
        );
        assert!(first.summary.state_masked_proposal_states > 0);
        assert_eq!(first.summary.proposal_states, 8);
        assert_eq!(first.summary.guided_action_evaluations, 4);
        assert_eq!(first.summary.unmasked_q_probe_states, 2);
        assert_eq!(first.summary.unmasked_action_evaluations, 2);
        assert!(first.summary.guided_exploit_interventions > 0);
        assert!(first.summary.unmasked_exploratory_interventions > 0);
        assert_eq!(
            first
                .candidates
                .iter()
                .map(Candidate::id)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            second
                .candidates
                .iter()
                .map(Candidate::id)
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        );
        assert!(first.candidates.iter().all(|proposal| {
            proposal.validate().is_ok()
                && proposal.ancestry.parent_id.as_deref() == Some(candidate.id().unwrap().as_str())
                && proposal.ancestry.intervention.is_some()
        }));
        assert_eq!(first.summary.coverage.episodes, 1);
        assert_eq!(
            first.summary.coverage.effective_decisions,
            episodes[0].corpus.transitions.len()
        );
        assert_eq!(first.summary.proposer_attribution.len(), 5);
        assert!(
            first
                .summary
                .proposer_attribution
                .iter()
                .any(|item| item.proposer == "ensemble_disagreement"
                    && item.uncertainty_is_heuristic)
        );
        assert!(!first.summary.policy_collapse_audit.single_action_collapse);
        assert_eq!(
            first
                .summary
                .proposer_attribution
                .iter()
                .map(|item| item.requested_budget)
                .sum::<usize>(),
            config.max_proposals
        );
    }

    #[test]
    fn guidance_lane_prefers_mask_while_exploration_lane_remains_unmasked() {
        let mut state = vec![0.0; 49];
        state[11] = 1.0;
        state[12] = 1.0;
        state[13] = 1.0;
        let guidance = movement_action_mask_v1(&state).unwrap();
        let masked_high_value = QEstimate {
            action: 67,
            mean: 10.0,
            variance: 4.0,
        };
        let recommended_lower_value = QEstimate {
            action: 0,
            mean: 1.0,
            variance: 0.0,
        };
        let ranked = [masked_high_value, recommended_lower_value];
        assert_eq!(
            guided_exploit(&ranked, &guidance).unwrap().action,
            recommended_lower_value.action
        );
        assert_eq!(
            unmasked_explore(&ranked, 0.0).unwrap().action,
            masked_high_value.action
        );
        assert!(guided_exploit(&[masked_high_value], &guidance).is_none());
        assert_eq!(
            unmasked_explore(&[masked_high_value], 0.0).unwrap().action,
            masked_high_value.action
        );
    }
}
