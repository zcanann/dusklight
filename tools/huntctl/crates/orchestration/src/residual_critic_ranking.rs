//! Request-bound fitted-Q ranking for continuous residual campaign batches.

use crate::optimization_request::{OptimizationCriticRanking, OptimizationRequest};
use dusklight_evidence::transition_corpus::TransitionCorpus;
use dusklight_evidence::transition_evidence::{ExactActionEvidence, TransitionEvidenceBundle};
use dusklight_learning::fqi::{
    FittedQ, FqiConfig, MAX_FQI_ACTIONS, MAX_FQI_TRANSITIONS, Transition,
};
use dusklight_learning::observation_view::movement_state_v2_spec;
use dusklight_learning::offline_rl::MovementActionSchema;
use dusklight_proposals::residual_critic_ranking::{
    ResidualCriticRankingConfig, ResidualCriticRankingReport, rank_residual_proposals,
};
use dusklight_search::residual_optimizer::ResidualProposalBatch;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

pub struct PreparedResidualCriticRanker {
    critic: FittedQ,
    parent_corpus: TransitionCorpus,
    config: ResidualCriticRankingConfig,
}

impl PreparedResidualCriticRanker {
    pub fn load(
        root: &Path,
        optimization: &OptimizationRequest,
    ) -> Result<Option<Self>, ResidualCriticBindingError> {
        let Some(binding) = &optimization.proposal.critic_ranking else {
            return Ok(None);
        };
        let training = binding
            .training_corpora
            .iter()
            .map(|source| read_bound_corpus(root, &source.corpus.path, &source.evidence.path))
            .collect::<Result<Vec<_>, _>>()?;
        let parent = read_bound_corpus(
            root,
            &binding.parent_corpus.corpus.path,
            &binding.parent_corpus.evidence.path,
        )?;
        validate_parent_evidence(optimization, binding, &parent)?;
        let training = training
            .into_iter()
            .map(|source| source.corpus)
            .collect::<Vec<_>>();
        Ok(Some(Self::from_corpora(&training, parent.corpus, binding)?))
    }

    pub(crate) fn from_corpora(
        training: &[TransitionCorpus],
        parent_corpus: TransitionCorpus,
        binding: &OptimizationCriticRanking,
    ) -> Result<Self, ResidualCriticBindingError> {
        validate_corpus_set(training, &parent_corpus)?;
        let critic = fit_critic(training, binding)?;
        Ok(Self {
            critic,
            parent_corpus,
            config: ResidualCriticRankingConfig {
                parent_corpus_start_frame: binding.parent_corpus_start_frame,
                uncertainty_penalty: f64::from(binding.uncertainty_penalty_millionths)
                    / 1_000_000.0,
            },
        })
    }

    pub fn rank(
        &self,
        batch: ResidualProposalBatch,
    ) -> Result<(ResidualProposalBatch, ResidualCriticRankingReport), ResidualCriticBindingError>
    {
        rank_residual_proposals(batch, &self.parent_corpus, &self.critic, self.config)
            .map_err(|error| ResidualCriticBindingError::new(error.to_string()))
    }
}

pub fn validate_critic_ranking_files(
    root: &Path,
    optimization: &OptimizationRequest,
    binding: &OptimizationCriticRanking,
) -> Result<(), ResidualCriticBindingError> {
    let training = binding
        .training_corpora
        .iter()
        .map(|source| read_bound_corpus(root, &source.corpus.path, &source.evidence.path))
        .collect::<Result<Vec<_>, _>>()?;
    let parent = read_bound_corpus(
        root,
        &binding.parent_corpus.corpus.path,
        &binding.parent_corpus.evidence.path,
    )?;
    validate_parent_evidence(optimization, binding, &parent)?;
    let training = training
        .into_iter()
        .map(|source| source.corpus)
        .collect::<Vec<_>>();
    validate_corpus_set(&training, &parent.corpus)?;
    let end = binding
        .parent_corpus_start_frame
        .checked_add(parent.corpus.transitions.len() as u64)
        .ok_or_else(|| ResidualCriticBindingError::new("critic parent-state range overflowed"))?;
    if optimization.proposal.search_space.end_frame_exclusive > end {
        return Err(ResidualCriticBindingError::new(
            "critic parent-state corpus does not cover the residual search surface",
        ));
    }
    Ok(())
}

fn validate_corpus_set(
    training: &[TransitionCorpus],
    parent: &TransitionCorpus,
) -> Result<(), ResidualCriticBindingError> {
    parent
        .validate()
        .map_err(|error| ResidualCriticBindingError::new(error.to_string()))?;
    let spec = movement_state_v2_spec();
    let feature_schema = spec
        .digest()
        .map_err(|error| ResidualCriticBindingError::new(error.to_string()))?;
    let Some(action_schema) = MovementActionSchema::from_digest(parent.action_schema) else {
        return Err(ResidualCriticBindingError::new(
            "critic parent-state corpus uses an unsupported action schema",
        ));
    };
    if parent.feature_schema != feature_schema
        || parent.feature_count != spec.features.len() as u32
        || training.is_empty()
    {
        return Err(ResidualCriticBindingError::new(
            "critic parent-state corpus differs from movement-state/v2",
        ));
    }
    for corpus in training {
        corpus
            .validate()
            .map_err(|error| ResidualCriticBindingError::new(error.to_string()))?;
        if corpus.feature_schema != parent.feature_schema
            || corpus.action_schema != parent.action_schema
            || corpus.feature_count != parent.feature_count
            || corpus.transitions.iter().any(|transition| {
                transition.action.macro_kind != action_schema.macro_kind()
                    || transition.action.action_id >= action_schema.action_count()
            })
        {
            return Err(ResidualCriticBindingError::new(
                "critic training corpus differs from its exact parent-state schemas",
            ));
        }
    }
    Ok(())
}

fn fit_critic(
    training: &[TransitionCorpus],
    binding: &OptimizationCriticRanking,
) -> Result<FittedQ, ResidualCriticBindingError> {
    let parent = training
        .first()
        .ok_or_else(|| ResidualCriticBindingError::new("critic training set is empty"))?;
    validate_corpus_set(training, parent)?;
    let mut actions = BTreeSet::new();
    let mut transitions = Vec::new();
    let mut groups = Vec::new();
    for (group, corpus) in training.iter().enumerate() {
        for row in &corpus.transitions {
            actions.insert(row.action.action_id);
            transitions.push(Transition {
                state: row.state.clone(),
                action: row.action.action_id,
                duration: row.duration_ticks,
                reward: row.reward,
                next_state: row.next_state.clone(),
                terminal: row.terminal,
            });
            groups.push(group as u64);
        }
    }
    if actions.is_empty()
        || actions.len() > MAX_FQI_ACTIONS
        || transitions.len() > MAX_FQI_TRANSITIONS
    {
        return Err(ResidualCriticBindingError::new(
            "critic training action or transition support exceeds fitted-Q bounds",
        ));
    }
    let config = FqiConfig {
        iterations: usize::from(binding.iterations),
        trees_per_action: usize::from(binding.trees_per_action),
        seed: binding.seed,
        categorical_features: movement_state_v2_spec().categorical_features(),
        ..FqiConfig::default()
    };
    FittedQ::fit_with_episode_groups(
        parent.feature_count as usize,
        &actions.into_iter().collect::<Vec<_>>(),
        &transitions,
        &groups,
        &config,
    )
    .map_err(|error| ResidualCriticBindingError::new(error.to_string()))
}

fn read_corpus(path: &Path) -> Result<TransitionCorpus, ResidualCriticBindingError> {
    TransitionCorpus::read_zstd_file(path)
        .or_else(|_| {
            fs::read(path)
                .map_err(dusklight_evidence::transition_corpus::TransitionCorpusError::Io)
                .and_then(|bytes| TransitionCorpus::decode(&bytes))
        })
        .map_err(|error| ResidualCriticBindingError::new(error.to_string()))
}

struct BoundCriticCorpus {
    corpus: TransitionCorpus,
    evidence: TransitionEvidenceBundle,
}

fn read_bound_corpus(
    root: &Path,
    corpus_path: &str,
    evidence_path: &str,
) -> Result<BoundCriticCorpus, ResidualCriticBindingError> {
    let corpus = read_corpus(&root.join(corpus_path))?;
    let evidence: TransitionEvidenceBundle = serde_json::from_slice(
        &fs::read(root.join(evidence_path))
            .map_err(|error| ResidualCriticBindingError::new(error.to_string()))?,
    )
    .map_err(|error| ResidualCriticBindingError::new(error.to_string()))?;
    evidence
        .validate(&corpus)
        .map_err(|error| ResidualCriticBindingError::new(error.to_string()))?;
    Ok(BoundCriticCorpus { corpus, evidence })
}

fn validate_parent_evidence(
    optimization: &OptimizationRequest,
    binding: &OptimizationCriticRanking,
    parent: &BoundCriticCorpus,
) -> Result<(), ResidualCriticBindingError> {
    let incumbent = optimization.incumbent.as_ref().ok_or_else(|| {
        ResidualCriticBindingError::new("critic-ranked residual search requires an incumbent")
    })?;
    let first_frame = parent
        .evidence
        .transitions
        .first()
        .and_then(|transition| match &transition.action {
            ExactActionEvidence::PadFrame { tape_frame, .. } => Some(*tape_frame),
            ExactActionEvidence::Option { .. } => None,
        })
        .ok_or_else(|| {
            ResidualCriticBindingError::new(
                "critic parent-state evidence does not begin with an exact PAD frame",
            )
        })?;
    if parent.evidence.tape_sha256 != incumbent.tape.sha256
        || first_frame != binding.parent_corpus_start_frame
        || parent
            .evidence
            .transitions
            .iter()
            .enumerate()
            .any(|(index, transition)| {
                !matches!(
                    &transition.action,
                    ExactActionEvidence::PadFrame { tape_frame, .. }
                        if *tape_frame == first_frame + index as u64
                )
            })
    {
        return Err(ResidualCriticBindingError::new(
            "critic parent-state evidence is detached from the exact incumbent tape or frame range",
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub struct ResidualCriticBindingError(String);

impl ResidualCriticBindingError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ResidualCriticBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualCriticBindingError {}
