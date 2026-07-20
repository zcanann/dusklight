//! Finite-batch Q-guided proposals for deterministic movement tapes.
//!
//! This module never scores or promotes gameplay. It consumes already proved
//! episode corpora, proposes small state-aware tape interventions, and hands
//! ordinary candidates back to the native evaluator.

use super::online_lineage::{OnlineDatasetGeneration, OnlineModelLineage};
use super::training_guard::{
    CoverageGuardConfig, CriticSnapshot, LearnedProposalGate, OnlineCoverageGate,
    OnlineTrainingHealth, TrainingGuardConfig,
};
use crate::action_guidance::{
    ACTION_GUIDANCE_SCHEMA_V2, AdvisoryActionMask, movement_action_mask_v2,
};
use crate::artifact::Digest;
use crate::candidate_envelope::{CandidateEnvelope, NamedDigest, ProposerIdentity, ProposerKind};
use crate::episode::EpisodeOutcomeClass;
use crate::fqi::{
    FITTED_Q_MODEL_SCHEMA_V2, FittedQ, FqiConfig, QEstimate, Transition as FqiTransition,
};
use crate::observation_view::movement_state_v2_spec;
use crate::offline_rl::{
    MOVEMENT_ACTION_COUNT_V2, MOVEMENT_REWARD_SCHEMA_V2, canonical_movement_pad_v2,
    movement_action_id_v2, movement_action_schema_digest_v2,
};
use crate::search::{Ancestry, Candidate, InterventionRange};
use crate::transition_corpus::TransitionCorpus;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::error::Error;
use std::fmt;

const MAX_PROPOSAL_STATES: usize = 4096;
const MAX_TRAINING_HEALTH_STATES: usize = 4096;
const EXPLORATION_WEIGHT: f64 = 1.5;
const UNMASKED_Q_PROBE_INTERVAL: usize = 4;
const Q_TERMINAL_REWARD_SCHEMA_V1: &str = "dusklight-route-q-terminal-reward/v1";
const Q_GOAL_TERMINAL_ADJUSTMENT: f32 = 512.0;
const Q_FAILURE_TERMINAL_ADJUSTMENT: f32 = -512.0;
const LEARNED_PARENT_POLICY_V2: &str = "authenticated-progress-repair/v2";
const INITIAL_TRIAL_BUDGET_POLICY_V2: &str = "two-learned-then-safe-fallback/v2";

#[derive(Clone, Debug)]
pub struct QEpisode {
    pub candidate: Candidate,
    pub corpus: TransitionCorpus,
    pub outcome: EpisodeOutcomeClass,
    pub objective: NamedDigest,
}

#[derive(Clone, Copy, Debug)]
pub struct QProposalConfig {
    pub generation: u32,
    pub max_proposals: usize,
    pub iterations: usize,
    pub trees_per_action: usize,
    pub seed: u64,
    pub readiness: QProposalReadinessEvidence,
}

#[derive(Clone, Copy, Debug)]
pub struct QProposalReadinessEvidence {
    pub required_facts_supported: bool,
    pub determinism_proved: bool,
    pub held_out_performance_adequate: bool,
    pub initial_bounded_trial: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct QProposalSummary {
    pub schema: &'static str,
    pub dataset_generation_sha256: Option<Digest>,
    pub model_lineage: Option<OnlineModelLineage>,
    pub training_transitions: usize,
    pub training_actions: usize,
    pub coverage_gate: OnlineCoverageGate,
    pub proposal_gate: LearnedProposalGate,
    pub training_health: Option<OnlineTrainingHealth>,
    pub proposal_states: usize,
    pub terminal_reward_schema: &'static str,
    pub step_reward_schema: &'static str,
    pub goal_terminal_adjustment: f32,
    pub failure_terminal_adjustment: f32,
    pub learned_parent_policy: &'static str,
    pub initial_trial_budget_policy: &'static str,
    pub learned_parent_episodes: usize,
    pub learned_parent_states: usize,
    pub action_guidance_schema: &'static str,
    pub state_masked_proposal_states: usize,
    pub guided_action_evaluations: usize,
    pub unmasked_action_evaluations: usize,
    pub unmasked_q_probe_states: usize,
    pub guided_exploit_interventions: usize,
    pub temporal_consensus_interventions: usize,
    pub unmasked_exploratory_interventions: usize,
    pub structured_counterfactual_interventions: usize,
    pub archive_novelty_interventions: usize,
    pub blind_coverage_interventions: usize,
    pub collection_cycle_offset: usize,
    pub collection_schedule: Vec<&'static str>,
    pub schedule_policy: &'static str,
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
    pub envelopes: Vec<CandidateEnvelope>,
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
    TemporalConsensus,
    EnsembleDisagreement,
    StructuredCounterfactual,
    ArchiveNovelty,
    RandomProbe,
    LatinHypercube,
}

impl ProposalKind {
    fn name(self) -> &'static str {
        match self {
            Self::GuidedExploit => "guided_exploit",
            Self::TemporalConsensus => "temporal_consensus",
            Self::EnsembleDisagreement => "ensemble_disagreement",
            Self::StructuredCounterfactual => "structured_counterfactual",
            Self::ArchiveNovelty => "archive_novelty",
            Self::RandomProbe | Self::LatinHypercube => "blind_coverage",
        }
    }

    fn mutation_prefix(self) -> &'static str {
        match self {
            Self::GuidedExploit => "q_guided",
            Self::TemporalConsensus => "q_temporal_consensus",
            Self::EnsembleDisagreement => "q_disagreement_heuristic",
            Self::StructuredCounterfactual => "structured_counterfactual",
            Self::ArchiveNovelty => "archive_novelty",
            Self::RandomProbe => "blind_random_probe",
            Self::LatinHypercube => "blind_latin_hypercube",
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

#[derive(Clone, Copy, Debug)]
struct MovementFeatureIndices {
    position: [usize; 3],
    procedure: usize,
    progress_fraction: usize,
    elapsed: usize,
    remaining: usize,
}

impl MovementFeatureIndices {
    fn from_spec(spec: &crate::observation_view::ObservationSpec) -> Result<Self, QSearchError> {
        let index = |name: &str| {
            spec.feature_index(name).ok_or_else(|| {
                QSearchError::new(format!(
                    "movement observation is missing required feature {name:?}"
                ))
            })
        };
        Ok(Self {
            position: [
                index("player.position_x")?,
                index("player.position_y")?,
                index("player.position_z")?,
            ],
            procedure: index("player.procedure")?,
            progress_fraction: index("objective.progress_fraction")?,
            elapsed: index("window.elapsed")?,
            remaining: index("window.remaining")?,
        })
    }
}

#[derive(Serialize)]
struct RouteQTrainingConfig<'a> {
    fqi: &'a FqiConfig,
    step_reward_schema: &'static str,
    terminal_reward_schema: &'static str,
    goal_terminal_adjustment: f32,
    failure_terminal_adjustment: f32,
}

#[derive(Serialize)]
struct ProposalConfigurationIdentity {
    schema: &'static str,
    step_reward_schema: &'static str,
    terminal_reward_schema: &'static str,
    goal_terminal_adjustment_bits: u32,
    failure_terminal_adjustment_bits: u32,
    learned_parent_policy: &'static str,
    initial_trial_budget_policy: &'static str,
    action_guidance_schema: &'static str,
    generation: u32,
    max_proposals: usize,
    iterations: usize,
    trees_per_action: usize,
    seed: u64,
    required_facts_supported: bool,
    determinism_proved: bool,
    held_out_performance_adequate: bool,
    initial_bounded_trial: bool,
    dataset_generation_sha256: Option<Digest>,
    previous_model_lineage_sha256: Option<Digest>,
}

pub fn propose_q_candidates(
    training_corpora: &[TransitionCorpus],
    episodes: &[QEpisode],
    config: QProposalConfig,
) -> Result<QProposalBatch, QSearchError> {
    propose_q_candidates_internal(training_corpora, episodes, config, None)
}

pub fn propose_q_candidates_with_lineage(
    training_corpora: &[TransitionCorpus],
    episodes: &[QEpisode],
    config: QProposalConfig,
    dataset_generation: &OnlineDatasetGeneration,
    previous_model_lineage: Option<&OnlineModelLineage>,
) -> Result<QProposalBatch, QSearchError> {
    if dataset_generation.generation != config.generation {
        return Err(QSearchError::new(
            "Q proposal generation does not match its immutable dataset generation",
        ));
    }
    dataset_generation
        .validate_corpora(training_corpora)
        .map_err(|error| QSearchError::new(error.to_string()))?;
    propose_q_candidates_internal(
        training_corpora,
        episodes,
        config,
        Some((dataset_generation, previous_model_lineage)),
    )
}

fn propose_q_candidates_internal(
    training_corpora: &[TransitionCorpus],
    episodes: &[QEpisode],
    config: QProposalConfig,
    lineage: Option<(&OnlineDatasetGeneration, Option<&OnlineModelLineage>)>,
) -> Result<QProposalBatch, QSearchError> {
    if training_corpora.is_empty() || episodes.is_empty() || config.max_proposals == 0 {
        return Err(QSearchError::new(
            "Q proposals require training corpora, aligned parent episodes, and a nonzero budget",
        ));
    }
    let observation_spec = movement_state_v2_spec();
    let feature_indices = MovementFeatureIndices::from_spec(&observation_spec)?;
    let feature_schema = observation_spec
        .digest()
        .map_err(|error| QSearchError::new(error.to_string()))?;
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
        for (index, transition) in corpus.transitions.iter().enumerate() {
            let final_transition = index + 1 == corpus.transitions.len();
            actions.insert(transition.action.action_id);
            transitions.push(route_q_training_transition(transition, final_transition));
            episode_groups.push(next_episode_group);
        }
        next_episode_group = next_episode_group
            .checked_add(1)
            .ok_or_else(|| QSearchError::new("Q proposal episode-group count overflowed"))?;
    }
    let actions: Vec<_> = actions.into_iter().collect();
    let action_support = collection_action_support(training_corpora);
    let state_bins = training_corpora
        .iter()
        .flat_map(|corpus| &corpus.transitions)
        .map(|transition| archive_context_key(&transition.state, feature_indices))
        .collect::<BTreeSet<_>>()
        .len();
    let coverage_gate = OnlineCoverageGate::evaluate(
        transitions.len(),
        &action_support,
        state_bins,
        CoverageGuardConfig::default(),
    )
    .map_err(|error| QSearchError::new(error.to_string()))?;
    let proposal_gate = LearnedProposalGate::evaluate(
        &coverage_gate,
        config.readiness.required_facts_supported,
        config.readiness.determinism_proved,
        config.readiness.held_out_performance_adequate,
        config.readiness.initial_bounded_trial,
    );
    let feature_width = training_corpora[0].feature_count as usize;
    let training_limits = TrainingGuardConfig::default();
    if config.iterations as f64 > training_limits.maximum_update_to_data_ratio {
        return Err(QSearchError::new(format!(
            "online Q update-to-data ratio {} exceeds cap {}",
            config.iterations, training_limits.maximum_update_to_data_ratio
        )));
    }
    let fqi_config = FqiConfig {
        iterations: config.iterations,
        trees_per_action: config.trees_per_action,
        max_tree_depth: 8,
        seed: config.seed,
        categorical_features: observation_spec.categorical_features(),
        ..FqiConfig::default()
    };
    let model = proposal_gate
        .learned_policy_enabled
        .then(|| {
            FittedQ::fit_with_episode_groups(
                feature_width,
                &actions,
                &transitions,
                &episode_groups,
                &fqi_config,
            )
            .map_err(|error| QSearchError::new(error.to_string()))
        })
        .transpose()?;
    let lineage_training_config = RouteQTrainingConfig {
        fqi: &fqi_config,
        step_reward_schema: MOVEMENT_REWARD_SCHEMA_V2,
        terminal_reward_schema: Q_TERMINAL_REWARD_SCHEMA_V1,
        goal_terminal_adjustment: Q_GOAL_TERMINAL_ADJUSTMENT,
        failure_terminal_adjustment: Q_FAILURE_TERMINAL_ADJUSTMENT,
    };
    let model_lineage = match (lineage, model.as_ref()) {
        (Some((dataset, previous)), Some(model)) => Some(
            OnlineModelLineage::build(
                dataset,
                previous,
                FITTED_Q_MODEL_SCHEMA_V2,
                &lineage_training_config,
                model,
            )
            .map_err(|error| QSearchError::new(error.to_string()))?,
        ),
        _ => None,
    };
    let training_health = model
        .as_ref()
        .map(|model| {
            let health_stride = transitions
                .len()
                .div_ceil(MAX_TRAINING_HEALTH_STATES)
                .max(1);
            let snapshots = transitions
                .iter()
                .step_by(health_stride)
                .map(|transition| {
                    model
                        .rank_actions(&transition.state)
                        .map_err(|error| QSearchError::new(error.to_string()))
                        .map(|estimates| {
                            estimates.into_iter().map(|estimate| CriticSnapshot {
                                primary: estimate.mean,
                                secondary: estimate.mean + estimate.variance.max(0.0).sqrt(),
                            })
                        })
                })
                .collect::<Result<Vec<_>, QSearchError>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let updates = u64::try_from(transitions.len())
                .ok()
                .and_then(|rows| rows.checked_mul(config.iterations as u64))
                .ok_or_else(|| QSearchError::new("online Q update count overflowed"))?;
            OnlineTrainingHealth::evaluate(transitions.len(), updates, &snapshots, training_limits)
                .and_then(OnlineTrainingHealth::require_healthy)
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
    let mut temporal = Vec::new();
    let mut explore = Vec::new();
    let mut systematic = Vec::new();
    let mut archive_novelty = Vec::new();
    let mut random = Vec::new();
    let mut latin_hypercube = Vec::new();
    let mut considered = 0_usize;
    let mut state_masked = 0;
    let mut guided_action_evaluations = 0_usize;
    let mut unmasked_action_evaluations = 0_usize;
    let mut unmasked_q_probe_states = 0_usize;
    let mut ordinal = 0;
    let learned_parent_episodes = aligned
        .iter()
        .enumerate()
        .filter(|(index, episode)| {
            learned_parent_eligible(episodes[*index].outcome, episode.corpus, feature_indices)
        })
        .count();
    let learned_parent_states = aligned
        .iter()
        .enumerate()
        .filter(|(index, episode)| {
            learned_parent_eligible(episodes[*index].outcome, episode.corpus, feature_indices)
        })
        .map(|(_, episode)| episode.corpus.transitions.len())
        .sum();
    let archive_context_support = aligned
        .iter()
        .flat_map(|episode| &episode.corpus.transitions)
        .fold(BTreeMap::<String, u64>::new(), |mut counts, transition| {
            *counts
                .entry(archive_context_key(&transition.state, feature_indices))
                .or_default() += 1;
            counts
        });
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
            let guidance = movement_action_mask_v2(&transition.state)
                .map_err(|error| QSearchError::new(error.to_string()))?;
            state_masked += usize::from(
                (0..MOVEMENT_ACTION_COUNT_V2).any(|action| !guidance.recommends(action)),
            );
            if let Some(model) = &model
                && learned_parent_eligible(
                    episodes[episode_index].outcome,
                    episode.corpus,
                    feature_indices,
                )
            {
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
                kind: ProposalKind::StructuredCounterfactual,
                width: [1, 2, 4][ordinal % 3],
            });
            let novelty_action = balanced_actions
                .iter()
                .cycle()
                .skip((ordinal + balanced_actions.len() / 2) % balanced_actions.len())
                .find(|action| **action != transition.action.action_id)
                .copied()
                .expect("movement catalog has alternate actions");
            let context_support = archive_context_support
                .get(&archive_context_key(&transition.state, feature_indices))
                .copied()
                .unwrap_or(0);
            archive_novelty.push(Intervention {
                episode: episode_index,
                frame,
                action: novelty_action,
                score: -(context_support as f64)
                    - action_support.get(&novelty_action).copied().unwrap_or(0) as f64 / 1024.0,
                kind: ProposalKind::ArchiveNovelty,
                width: [1, 2, 4][(ordinal + 2) % 3],
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
                / total_states.clamp(1, MAX_PROPOSAL_STATES))
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
    if let Some(model) = &model {
        for (episode_index, episode) in aligned.iter().enumerate() {
            if learned_parent_eligible(
                episodes[episode_index].outcome,
                episode.corpus,
                feature_indices,
            ) {
                temporal.extend(temporal_consensus_interventions(
                    model,
                    episode_index,
                    episode.corpus,
                )?);
            }
        }
    }
    sort_interventions(&mut exploit);
    sort_interventions(&mut temporal);
    sort_interventions(&mut explore);
    sort_interventions(&mut systematic);
    sort_interventions(&mut archive_novelty);

    let blind_coverage = random
        .iter()
        .copied()
        .zip(latin_hypercube.iter().copied())
        .flat_map(|(random, latin)| [random, latin])
        .collect::<Vec<_>>();

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
    let (collection_cycle_offset, budgets, schedule_policy) =
        if proposal_gate.learned_policy_enabled && proposal_gate.bounded_exploration_enabled {
            (
                0,
                split_initial_learned_budget(config.max_proposals),
                "bounded_exploration_floor_three_learned_then_safe_fallback",
            )
        } else if proposal_gate.learned_policy_enabled {
            let offset = config.generation as usize % 6;
            (
                offset,
                split_proposer_budget(config.max_proposals, offset),
                "generation_rotated_budget_then_round_robin_available_lanes",
            )
        } else {
            let offset = 3 + config.generation as usize % 3;
            (
                offset,
                split_fallback_budget(config.max_proposals, config.generation as usize % 3),
                "readiness_fallback_structured_archive_blind_round_robin",
            )
        };
    let pools = [
        (ProposalKind::GuidedExploit, &exploit, budgets[0]),
        (ProposalKind::TemporalConsensus, &temporal, budgets[1]),
        (ProposalKind::EnsembleDisagreement, &explore, budgets[2]),
        (
            ProposalKind::StructuredCounterfactual,
            &systematic,
            budgets[3],
        ),
        (ProposalKind::ArchiveNovelty, &archive_novelty, budgets[4]),
        (ProposalKind::RandomProbe, &blind_coverage, budgets[5]),
    ];
    let mut cursors = [0_usize; 6];
    let mut generated = [0_usize; 6];
    let mut collection_schedule = Vec::new();
    while candidates.len() < config.max_proposals {
        let mut progressed = false;
        for offset in 0..pools.len() {
            let lane = (collection_cycle_offset + offset) % pools.len();
            let (kind, pool, budget) = &pools[lane];
            if generated[lane] >= *budget {
                continue;
            }
            while cursors[lane] < pool.len() {
                let cursor = cursors[lane];
                cursors[lane] += 1;
                let before = candidates.len();
                append_interventions(
                    &pool[cursor..cursor + 1],
                    1,
                    &aligned,
                    episodes,
                    config.generation,
                    &mut ids,
                    &mut candidates,
                )?;
                if candidates.len() > before {
                    generated[lane] += 1;
                    collection_schedule.push(kind.name());
                    progressed = true;
                    break;
                }
            }
        }
        if !progressed {
            break;
        }
    }
    let proposer_attribution = pools
        .iter()
        .enumerate()
        .map(|(lane, (kind, pool, budget))| ProposerAttribution {
            proposer: kind.name(),
            requested_budget: *budget,
            available_interventions: pool.len(),
            generated_candidates: generated[lane],
            uncertainty_is_heuristic: *kind == ProposalKind::EnsembleDisagreement,
        })
        .collect();

    let policy_collapse_audit = policy_collapse_audit(&candidates, episodes.len())?;
    let objective = &episodes[0].objective;
    if episodes
        .iter()
        .any(|episode| episode.objective != *objective)
    {
        return Err(QSearchError::new(
            "Q proposal parents do not share one exact objective identity",
        ));
    }
    let configuration_sha256 = proposal_configuration_sha256(&config, lineage)?;
    let envelopes = candidates
        .iter()
        .map(|candidate| {
            candidate_envelope(
                candidate,
                objective.clone(),
                action_schema,
                config.seed,
                configuration_sha256,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(QProposalBatch {
        summary: QProposalSummary {
            schema: "dusklight-q-proposals/v12",
            dataset_generation_sha256: lineage.map(|(dataset, _)| dataset.generation_sha256),
            model_lineage,
            training_transitions: transitions.len(),
            training_actions: actions.len(),
            coverage_gate,
            proposal_gate,
            training_health,
            proposal_states: considered,
            step_reward_schema: MOVEMENT_REWARD_SCHEMA_V2,
            terminal_reward_schema: Q_TERMINAL_REWARD_SCHEMA_V1,
            goal_terminal_adjustment: Q_GOAL_TERMINAL_ADJUSTMENT,
            failure_terminal_adjustment: Q_FAILURE_TERMINAL_ADJUSTMENT,
            learned_parent_policy: LEARNED_PARENT_POLICY_V2,
            initial_trial_budget_policy: INITIAL_TRIAL_BUDGET_POLICY_V2,
            learned_parent_episodes,
            learned_parent_states,
            action_guidance_schema: ACTION_GUIDANCE_SCHEMA_V2,
            state_masked_proposal_states: state_masked,
            guided_action_evaluations,
            unmasked_action_evaluations,
            unmasked_q_probe_states,
            guided_exploit_interventions: exploit.len(),
            temporal_consensus_interventions: temporal.len(),
            unmasked_exploratory_interventions: explore.len(),
            structured_counterfactual_interventions: systematic.len(),
            archive_novelty_interventions: archive_novelty.len(),
            blind_coverage_interventions: blind_coverage.len(),
            collection_cycle_offset,
            collection_schedule,
            schedule_policy,
            proposals: candidates.len(),
            coverage: collection_coverage(episodes, feature_indices),
            proposer_attribution,
            policy_collapse_audit,
        },
        candidates,
        envelopes,
    })
}

fn route_q_training_transition(
    transition: &crate::transition_corpus::Transition,
    final_transition: bool,
) -> FqiTransition {
    let terminal_adjustment = if final_transition {
        if transition.terminal {
            Q_GOAL_TERMINAL_ADJUSTMENT
        } else {
            Q_FAILURE_TERMINAL_ADJUSTMENT
        }
    } else {
        0.0
    };
    FqiTransition {
        state: transition.state.clone(),
        action: transition.action.action_id,
        duration: transition.duration_ticks,
        reward: transition.reward + terminal_adjustment,
        next_state: transition.next_state.clone(),
        // A candidate tape ending is a terminal decision for this bounded
        // proposal problem even when it missed the objective. The immutable
        // corpus retains the native terminal bit.
        terminal: final_transition,
    }
}

fn learned_parent_eligible(
    outcome: EpisodeOutcomeClass,
    corpus: &TransitionCorpus,
    indices: MovementFeatureIndices,
) -> bool {
    if outcome == EpisodeOutcomeClass::Successful {
        return true;
    }
    if outcome != EpisodeOutcomeClass::Failed {
        return false;
    }
    let Some(first) = corpus.transitions.first() else {
        return false;
    };
    let initial = first.state[indices.progress_fraction];
    initial.is_finite()
        && corpus.transitions.iter().any(|transition| {
            let progress = transition.next_state[indices.progress_fraction];
            progress.is_finite() && progress > initial + f32::EPSILON
        })
}

fn proposal_configuration_sha256(
    config: &QProposalConfig,
    lineage: Option<(&OnlineDatasetGeneration, Option<&OnlineModelLineage>)>,
) -> Result<Digest, QSearchError> {
    let identity = ProposalConfigurationIdentity {
        schema: "dusklight-q-proposals/v12",
        step_reward_schema: MOVEMENT_REWARD_SCHEMA_V2,
        terminal_reward_schema: Q_TERMINAL_REWARD_SCHEMA_V1,
        goal_terminal_adjustment_bits: Q_GOAL_TERMINAL_ADJUSTMENT.to_bits(),
        failure_terminal_adjustment_bits: Q_FAILURE_TERMINAL_ADJUSTMENT.to_bits(),
        learned_parent_policy: LEARNED_PARENT_POLICY_V2,
        initial_trial_budget_policy: INITIAL_TRIAL_BUDGET_POLICY_V2,
        action_guidance_schema: ACTION_GUIDANCE_SCHEMA_V2,
        generation: config.generation,
        max_proposals: config.max_proposals,
        iterations: config.iterations,
        trees_per_action: config.trees_per_action,
        seed: config.seed,
        required_facts_supported: config.readiness.required_facts_supported,
        determinism_proved: config.readiness.determinism_proved,
        held_out_performance_adequate: config.readiness.held_out_performance_adequate,
        initial_bounded_trial: config.readiness.initial_bounded_trial,
        dataset_generation_sha256: lineage.map(|(dataset, _)| dataset.generation_sha256),
        previous_model_lineage_sha256: lineage
            .and_then(|(_, model)| model.map(|model| model.lineage_sha256)),
    };
    let bytes =
        serde_json::to_vec(&identity).map_err(|error| QSearchError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.q-proposer-configuration/v2\0");
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn candidate_envelope(
    candidate: &Candidate,
    objective: NamedDigest,
    action_schema_sha256: Digest,
    seed: u64,
    configuration_sha256: Digest,
) -> Result<CandidateEnvelope, QSearchError> {
    let mutation = candidate
        .ancestry
        .mutation
        .as_deref()
        .ok_or_else(|| QSearchError::new("Q proposal has no typed proposer attribution"))?;
    let (kind, id) = if mutation.starts_with("q_guided") {
        (ProposerKind::Learned, "learned.tree-fqi-guided")
    } else if mutation.starts_with("q_temporal_consensus") {
        (ProposerKind::Learned, "learned.tree-fqi-temporal-consensus")
    } else if mutation.starts_with("q_disagreement_heuristic") {
        (ProposerKind::Learned, "learned.tree-fqi-disagreement")
    } else if mutation.starts_with("structured_counterfactual") {
        (ProposerKind::StructuredSearch, "search.counterfactual")
    } else if mutation.starts_with("archive_novelty") {
        (ProposerKind::StructuredSearch, "search.archive-novelty")
    } else if mutation.starts_with("blind_") {
        (ProposerKind::Random, "random.coverage")
    } else {
        return Err(QSearchError::new(
            "Q proposal mutation has no supported proposer identity",
        ));
    };
    let candidate_sha256 = candidate
        .id()
        .map_err(|error| QSearchError::new(error.to_string()))?
        .parse()
        .map_err(|error| QSearchError::new(format!("invalid candidate digest: {error}")))?;
    let parent_candidate_sha256 = candidate
        .ancestry
        .parent_id
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|error| QSearchError::new(format!("invalid parent candidate digest: {error}")))?;
    CandidateEnvelope::build(
        candidate_sha256,
        parent_candidate_sha256,
        candidate.ancestry.generation,
        objective,
        NamedDigest::new("movement-action/v2", action_schema_sha256),
        seed,
        ProposerIdentity {
            kind,
            id: id.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            configuration_sha256,
        },
    )
    .map_err(|error| QSearchError::new(error.to_string()))
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

fn split_proposer_budget(total: usize, cycle_offset: usize) -> [usize; 6] {
    let mut budgets = [total / 6; 6];
    for offset in 0..total % 5 {
        budgets[(cycle_offset + offset) % budgets.len()] += 1;
    }
    budgets
}

fn split_initial_learned_budget(total: usize) -> [usize; 6] {
    let mut budgets = [
        usize::from(total > 0),
        usize::from(total > 1),
        usize::from(total > 2),
        0,
        0,
        0,
    ];
    for ordinal in 3..total {
        budgets[3 + (ordinal - 3) % 3] += 1;
    }
    budgets
}

fn split_fallback_budget(total: usize, cycle_offset: usize) -> [usize; 6] {
    let mut budgets = [0; 6];
    for ordinal in 0..total {
        budgets[3 + (cycle_offset + ordinal) % 3] += 1;
    }
    budgets
}

/// Build temporally extended learned actions by asking which single action has
/// the highest mean fitted-Q advantage over an observed window. This is an
/// open-loop candidate proposal, not a simulated counterfactual: the native
/// evaluator remains the only authority on whether holding that action
/// actually produces a straighter or faster trajectory.
fn temporal_consensus_interventions(
    model: &FittedQ,
    episode: usize,
    corpus: &TransitionCorpus,
) -> Result<Vec<Intervention>, QSearchError> {
    const HORIZONS: [usize; 4] = [8, 16, 32, 64];
    let actions = model.actions();
    let mut advantages = Vec::with_capacity(corpus.transitions.len());
    for transition in &corpus.transitions {
        let guidance = movement_action_mask_v2(&transition.state)
            .map_err(|error| QSearchError::new(error.to_string()))?;
        let ranked = model
            .rank_actions(&transition.state)
            .map_err(|error| QSearchError::new(error.to_string()))?;
        let current = ranked
            .iter()
            .find(|estimate| estimate.action == transition.action.action_id)
            .ok_or_else(|| QSearchError::new("fitted-Q model omitted an observed action"))?
            .mean;
        let row = actions
            .iter()
            .map(|action| {
                ranked
                    .iter()
                    .find(|estimate| estimate.action == *action)
                    .filter(|_| guidance.recommends(*action))
                    .map_or(f64::NEG_INFINITY, |estimate| estimate.mean - current)
            })
            .collect::<Vec<_>>();
        advantages.push(row);
    }

    let mut interventions = Vec::new();
    for width in HORIZONS {
        if width > advantages.len() {
            continue;
        }
        for frame in 0..=advantages.len() - width {
            let best = actions
                .iter()
                .enumerate()
                .filter_map(|(action_index, action)| {
                    let sum = advantages[frame..frame + width]
                        .iter()
                        .map(|row| row[action_index])
                        .try_fold(0.0, |sum, value| value.is_finite().then_some(sum + value))?;
                    Some((*action, sum / width as f64))
                })
                .max_by(|left, right| {
                    left.1
                        .total_cmp(&right.1)
                        .then_with(|| right.0.cmp(&left.0))
                });
            if let Some((action, score)) = best {
                interventions.push(Intervention {
                    episode,
                    frame,
                    action,
                    score,
                    kind: ProposalKind::TemporalConsensus,
                    width,
                });
            }
        }
    }
    Ok(interventions)
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

fn archive_context_key(state: &[f32], indices: MovementFeatureIndices) -> String {
    let stage: String = state[..8]
        .iter()
        .map(|value| (value * 255.0).round().clamp(0.0, 255.0) as u8)
        .take_while(|byte| *byte != 0)
        .map(char::from)
        .collect();
    format!(
        "{stage}:{}:{},{}:proc{}:phase{}",
        state[8].round() as i32,
        (state[indices.position[0]] * 32.0).floor() as i32,
        (state[indices.position[2]] * 32.0).floor() as i32,
        state[indices.procedure].round() as i32,
        if state[indices.remaining] <= 1.0 / 1024.0 {
            "terminal"
        } else if state[indices.elapsed] <= 1.0 / 1024.0 {
            "opening"
        } else {
            "middle"
        }
    )
}

fn collection_coverage(
    episodes: &[QEpisode],
    indices: MovementFeatureIndices,
) -> CollectionCoverage {
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
                    (state[indices.position[0]] * 32.0).floor() as i32,
                    (state[indices.position[2]] * 32.0).floor() as i32
                ),
            );
            increment(
                &mut coverage.player_procedures,
                (state[indices.procedure].round() as i32).to_string(),
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
            let phase = if transition.terminal || state[indices.remaining] <= 1.0 / 1024.0 {
                "terminal"
            } else if state[indices.elapsed] <= 1.0 / 1024.0 {
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
    use crate::action_guidance::{ACTION_GUIDANCE_SCHEMA_V2, movement_action_mask_v2};
    use crate::artifact::Digest;
    use crate::evaluation_isolation::{EvaluationAttemptInput, EvaluationGenerationSeal};
    use crate::search::SegmentProfile;
    use crate::tape::{InputFrame, InputTape, RawPadState};
    use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};

    fn objective() -> NamedDigest {
        NamedDigest::new("q-search-test", Digest([0xa5; 32]))
    }

    fn admitted_readiness() -> QProposalReadinessEvidence {
        QProposalReadinessEvidence {
            required_facts_supported: true,
            determinism_proved: true,
            held_out_performance_adequate: true,
            initial_bounded_trial: false,
        }
    }

    fn corpus_for(candidate: &Candidate) -> TransitionCorpus {
        let observation_spec = movement_state_v2_spec();
        let feature_count = observation_spec.feature_count();
        let player_present = observation_spec.feature_index("player.present").unwrap();
        let player_is_link = observation_spec.feature_index("player.is_link").unwrap();
        let procedure_present = observation_spec
            .feature_index("player.procedure_present")
            .unwrap();
        let procedure = observation_spec.feature_index("player.procedure").unwrap();
        let position_x = observation_spec.feature_index("player.position_x").unwrap();
        let event_running = observation_spec.feature_index("event.running").unwrap();
        let progress_configured = observation_spec
            .feature_index("objective.progress_configured")
            .unwrap();
        let progress_fraction = observation_spec
            .feature_index("objective.progress_fraction")
            .unwrap();
        let elapsed = observation_spec.feature_index("window.elapsed").unwrap();
        let remaining = observation_spec.feature_index("window.remaining").unwrap();
        let tape = candidate.compile().unwrap();
        let frame_count = tape.frames.len();
        let transitions = tape
            .frames
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let action_id = movement_action_id_v2(frame.pads[0]).unwrap();
                let mut state = vec![0.0; feature_count as usize];
                state[player_present] = 1.0;
                state[player_is_link] = 1.0;
                state[procedure_present] = 1.0;
                state[procedure] = 4.0;
                state[position_x] = index as f32 / 32.0;
                state[event_running] = f32::from(index == 0);
                state[progress_configured] = 1.0;
                state[progress_fraction] = 1.0 / 3.0;
                state[elapsed] = index as f32 / 1024.0;
                state[remaining] = frame_count.saturating_sub(index) as f32 / 1024.0;
                let mut next_state = state.clone();
                next_state[position_x] += 1.0 / 32.0;
                next_state[elapsed] += 1.0 / 1024.0;
                next_state[remaining] = frame_count.saturating_sub(index + 1) as f32 / 1024.0;
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
            observation_spec.digest().unwrap(),
            movement_action_schema_digest_v2(),
            feature_count,
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
            objective: objective(),
        }];
        let config = QProposalConfig {
            generation: 1,
            max_proposals: 4,
            iterations: 4,
            trees_per_action: 3,
            seed: 7,
            readiness: admitted_readiness(),
        };
        let first = propose_q_candidates(std::slice::from_ref(&corpus), &episodes, config).unwrap();
        let second = propose_q_candidates(&[corpus], &episodes, config).unwrap();
        assert!(!first.candidates.is_empty());
        assert_eq!(first.summary.proposals, first.candidates.len());
        assert_eq!(first.envelopes.len(), first.candidates.len());
        assert!(first.envelopes.iter().all(|envelope| {
            envelope.validate().is_ok()
                && envelope.objective == objective()
                && envelope.action_schema.sha256 == movement_action_schema_digest_v2()
                && envelope.seed == config.seed
        }));
        assert_eq!(
            first.summary.action_guidance_schema,
            ACTION_GUIDANCE_SCHEMA_V2
        );
        assert!(first.summary.state_masked_proposal_states > 0);
        assert_eq!(first.summary.proposal_states, 8);
        let health = first.summary.training_health.as_ref().unwrap();
        assert_eq!(health.update_to_data_ratio, 4.0);
        assert_eq!(
            health.disposition,
            super::super::training_guard::TrainingHealthDisposition::Healthy
        );
        assert_eq!(first.summary.schema, "dusklight-q-proposals/v12");
        assert_eq!(first.summary.step_reward_schema, MOVEMENT_REWARD_SCHEMA_V2);
        assert_eq!(
            first.summary.terminal_reward_schema,
            Q_TERMINAL_REWARD_SCHEMA_V1
        );
        assert_eq!(
            first.summary.learned_parent_policy,
            LEARNED_PARENT_POLICY_V2
        );
        assert_eq!(
            first.summary.initial_trial_budget_policy,
            INITIAL_TRIAL_BUDGET_POLICY_V2
        );
        assert_eq!(first.summary.learned_parent_episodes, 1);
        assert_eq!(first.summary.learned_parent_states, 8);
        assert!(first.summary.dataset_generation_sha256.is_none());
        assert!(first.summary.model_lineage.is_none());
        assert!(first.summary.coverage_gate.learned_policy_enabled);
        assert!(first.summary.proposal_gate.learned_policy_enabled);
        assert_eq!(first.summary.collection_cycle_offset, 1);
        assert!(first.summary.guided_action_evaluations > 0);
        assert_eq!(first.summary.unmasked_q_probe_states, 2);
        assert_eq!(first.summary.unmasked_action_evaluations, 2);
        assert!(first.summary.guided_exploit_interventions > 0);
        assert!(first.summary.temporal_consensus_interventions > 0);
        assert!(first.summary.unmasked_exploratory_interventions > 0);
        assert!(first.summary.structured_counterfactual_interventions > 0);
        assert!(first.summary.archive_novelty_interventions > 0);
        assert!(first.summary.blind_coverage_interventions > 0);
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
        assert_eq!(first.summary.proposer_attribution.len(), 6);
        assert_eq!(
            first.summary.collection_schedule.len(),
            first.summary.proposals
        );
        assert!(
            first
                .summary
                .collection_schedule
                .windows(2)
                .all(|pair| pair[0] != pair[1])
        );
        assert_eq!(
            first.summary.schedule_policy,
            "generation_rotated_budget_then_round_robin_available_lanes"
        );
        for proposer in [
            "structured_counterfactual",
            "ensemble_disagreement",
            "archive_novelty",
            "blind_coverage",
            "guided_exploit",
            "temporal_consensus",
        ] {
            assert!(
                first
                    .summary
                    .proposer_attribution
                    .iter()
                    .any(|item| item.proposer == proposer)
            );
        }
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
    fn route_q_projection_values_success_and_failed_tape_endings() {
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: vec![InputFrame {
                owned_ports: 1,
                pads: [
                    canonical_movement_pad_v2(0).unwrap(),
                    disconnected,
                    disconnected,
                    disconnected,
                ],
                ..InputFrame::default()
            }],
            ..InputTape::default()
        };
        let candidate =
            Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
        let corpus = corpus_for(&candidate);
        let successful = route_q_training_transition(&corpus.transitions[0], true);
        assert!(successful.terminal);
        assert_eq!(successful.reward, -1.0 + Q_GOAL_TERMINAL_ADJUSTMENT);

        let mut missed = corpus.transitions[0].clone();
        missed.terminal = false;
        let failed = route_q_training_transition(&missed, true);
        assert!(failed.terminal);
        assert_eq!(failed.reward, -1.0 + Q_FAILURE_TERMINAL_ADJUSTMENT);

        let interior = route_q_training_transition(&missed, false);
        assert!(!interior.terminal);
        assert_eq!(interior.reward, -1.0);
    }

    #[test]
    fn authenticated_progress_failure_is_a_learned_repair_parent() {
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
        let mut corpus = corpus_for(&candidate);
        let progress = movement_state_v2_spec()
            .feature_index("objective.progress_fraction")
            .unwrap();
        for (index, transition) in corpus.transitions.iter_mut().enumerate() {
            if index >= 3 {
                transition.state[progress] = 2.0 / 3.0;
            }
            if index >= 2 {
                transition.next_state[progress] = 2.0 / 3.0;
            }
        }
        corpus.transitions.last_mut().unwrap().terminal = false;
        corpus.validate().unwrap();
        let batch = propose_q_candidates(
            std::slice::from_ref(&corpus),
            &[QEpisode {
                candidate: candidate.clone(),
                corpus: corpus.clone(),
                outcome: EpisodeOutcomeClass::Failed,
                objective: objective(),
            }],
            QProposalConfig {
                generation: 1,
                max_proposals: 4,
                iterations: 4,
                trees_per_action: 3,
                seed: 77,
                readiness: admitted_readiness(),
            },
        )
        .unwrap();
        assert_eq!(batch.summary.learned_parent_episodes, 1);
        assert_eq!(batch.summary.learned_parent_states, 8);
        assert!(batch.summary.guided_exploit_interventions > 0);
        assert!(batch.summary.temporal_consensus_interventions > 0);
        assert!(batch.candidates.iter().any(|proposal| {
            proposal.ancestry.parent_id.as_deref() == Some(candidate.id().unwrap().as_str())
                && proposal
                    .ancestry
                    .mutation
                    .as_deref()
                    .is_some_and(|mutation| mutation.starts_with("q_"))
        }));
    }

    #[test]
    fn online_q_binds_deterministic_model_to_immutable_dataset_generation() {
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: (0..4)
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
        let corpus_digest = corpus.content_digest().unwrap();
        let seal = EvaluationGenerationSeal::build(
            0,
            2,
            2,
            2,
            0,
            &[
                EvaluationAttemptInput {
                    candidate_id: "candidate-a".into(),
                    attempt: 1,
                    worker_id: "evaluation/worker-0".into(),
                    transition_corpus_sha256: Some(corpus_digest),
                },
                EvaluationAttemptInput {
                    candidate_id: "candidate-a".into(),
                    attempt: 2,
                    worker_id: "evaluation/worker-1".into(),
                    transition_corpus_sha256: None,
                },
            ],
        )
        .unwrap();
        let dataset =
            OnlineDatasetGeneration::build(None, &seal, std::slice::from_ref(&corpus)).unwrap();
        let episodes = [QEpisode {
            candidate,
            corpus: corpus.clone(),
            outcome: EpisodeOutcomeClass::Successful,
            objective: objective(),
        }];
        let config = QProposalConfig {
            generation: 1,
            max_proposals: 2,
            iterations: 2,
            trees_per_action: 3,
            seed: 9,
            readiness: admitted_readiness(),
        };
        let first = propose_q_candidates_with_lineage(
            std::slice::from_ref(&corpus),
            &episodes,
            config,
            &dataset,
            None,
        )
        .unwrap();
        let second =
            propose_q_candidates_with_lineage(&[corpus], &episodes, config, &dataset, None)
                .unwrap();
        assert_eq!(
            first.summary.dataset_generation_sha256,
            Some(dataset.generation_sha256)
        );
        let lineage = first.summary.model_lineage.as_ref().unwrap();
        lineage.validate().unwrap();
        assert_eq!(
            second.summary.model_lineage.as_ref().unwrap(),
            lineage,
            "same immutable dataset and training config must reproduce exact model lineage"
        );
    }

    #[test]
    fn inadequate_action_support_reassigns_learned_budget_to_safe_fallbacks() {
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: (0..4)
                .map(|_| InputFrame {
                    owned_ports: 1,
                    pads: [
                        canonical_movement_pad_v2(0).unwrap(),
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
        let batch = propose_q_candidates(
            std::slice::from_ref(&corpus),
            &[QEpisode {
                candidate,
                corpus: corpus.clone(),
                outcome: EpisodeOutcomeClass::Successful,
                objective: objective(),
            }],
            QProposalConfig {
                generation: 1,
                max_proposals: 3,
                iterations: 2,
                trees_per_action: 3,
                seed: 9,
                readiness: admitted_readiness(),
            },
        )
        .unwrap();
        assert_eq!(
            batch.summary.coverage_gate.disposition,
            super::super::training_guard::CoverageDisposition::FallbackInsufficientActionSupport
        );
        assert!(!batch.summary.coverage_gate.learned_policy_enabled);
        assert_eq!(
            batch.summary.coverage_gate.fallback_policy,
            Some("structured_archive_blind_only")
        );
        assert!(batch.summary.model_lineage.is_none());
        assert!(batch.summary.training_health.is_none());
        assert_eq!(
            batch.summary.schedule_policy,
            "readiness_fallback_structured_archive_blind_round_robin"
        );
        assert!(
            batch
                .summary
                .collection_schedule
                .iter()
                .all(|lane| matches!(
                    *lane,
                    "structured_counterfactual" | "archive_novelty" | "blind_coverage"
                ))
        );
        assert_eq!(
            batch
                .summary
                .proposer_attribution
                .iter()
                .take(3)
                .map(|lane| lane.requested_budget)
                .sum::<usize>(),
            0
        );
    }

    #[test]
    fn unsupported_facts_unproved_determinism_and_bad_holdout_disable_learning() {
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
        let batch = propose_q_candidates(
            std::slice::from_ref(&corpus),
            &[QEpisode {
                candidate,
                corpus: corpus.clone(),
                outcome: EpisodeOutcomeClass::Successful,
                objective: objective(),
            }],
            QProposalConfig {
                generation: 1,
                max_proposals: 3,
                iterations: 2,
                trees_per_action: 3,
                seed: 9,
                readiness: QProposalReadinessEvidence {
                    required_facts_supported: false,
                    determinism_proved: false,
                    held_out_performance_adequate: false,
                    initial_bounded_trial: false,
                },
            },
        )
        .unwrap();
        assert!(batch.summary.coverage_gate.learned_policy_enabled);
        assert!(!batch.summary.proposal_gate.learned_policy_enabled);
        assert_eq!(
            batch.summary.proposal_gate.blockers,
            [
                super::super::training_guard::LearnedProposalBlocker::RequiredFactsUnsupported,
                super::super::training_guard::LearnedProposalBlocker::DeterminismUnproved,
            ]
        );
        assert!(batch.summary.model_lineage.is_none());
        assert!(batch.summary.training_health.is_none());
        assert!(
            batch
                .summary
                .collection_schedule
                .iter()
                .all(|lane| matches!(
                    *lane,
                    "structured_counterfactual" | "archive_novelty" | "blind_coverage"
                ))
        );
    }

    #[test]
    fn remainder_budget_rotates_across_all_collection_lanes() {
        assert_eq!(split_proposer_budget(3, 0), [1, 1, 1, 0, 0, 0]);
        assert_eq!(split_proposer_budget(3, 1), [0, 1, 1, 1, 0, 0]);
        assert_eq!(split_proposer_budget(3, 4), [1, 0, 0, 0, 1, 1]);
        let mut totals = [0; 6];
        for generation in 0..6 {
            for (total, budget) in totals.iter_mut().zip(split_proposer_budget(3, generation)) {
                *total += budget;
            }
        }
        assert_eq!(totals, [3; 6]);
    }

    #[test]
    fn initial_trial_is_capped_and_cannot_bypass_fact_or_determinism_gates() {
        let ready_coverage = OnlineCoverageGate::evaluate(
            8,
            &BTreeMap::from([(0, 4), (18, 4)]),
            8,
            CoverageGuardConfig::default(),
        )
        .unwrap();
        let admitted = LearnedProposalGate::evaluate(&ready_coverage, true, true, false, true);
        assert!(admitted.learned_policy_enabled);
        assert_eq!(split_initial_learned_budget(64), [1, 1, 1, 21, 20, 20]);

        let unsupported = LearnedProposalGate::evaluate(&ready_coverage, false, true, false, true);
        assert!(!unsupported.learned_policy_enabled);
        assert!(unsupported.blockers.contains(
            &super::super::training_guard::LearnedProposalBlocker::RequiredFactsUnsupported
        ));
        let nondeterministic =
            LearnedProposalGate::evaluate(&ready_coverage, true, false, false, true);
        assert!(!nondeterministic.learned_policy_enabled);
        assert!(
            nondeterministic.blockers.contains(
                &super::super::training_guard::LearnedProposalBlocker::DeterminismUnproved
            )
        );
    }

    #[test]
    fn guidance_lane_prefers_mask_while_exploration_lane_remains_unmasked() {
        let mut state = vec![0.0; 98];
        state[15] = 1.0;
        state[16] = 1.0;
        state[37] = 1.0;
        let guidance = movement_action_mask_v2(&state).unwrap();
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

    #[test]
    fn online_q_rejects_excessive_update_to_data_ratio_before_proposing() {
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: vec![InputFrame {
                owned_ports: 1,
                pads: [
                    canonical_movement_pad_v2(0).unwrap(),
                    disconnected,
                    disconnected,
                    disconnected,
                ],
                ..InputFrame::default()
            }],
            ..InputTape::default()
        };
        let candidate =
            Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
        let corpus = corpus_for(&candidate);
        let error = propose_q_candidates(
            std::slice::from_ref(&corpus),
            &[QEpisode {
                candidate,
                corpus: corpus.clone(),
                outcome: EpisodeOutcomeClass::Successful,
                objective: objective(),
            }],
            QProposalConfig {
                generation: 0,
                max_proposals: 1,
                iterations: 33,
                trees_per_action: 1,
                seed: 1,
                readiness: admitted_readiness(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("update-to-data ratio"));
    }
}
