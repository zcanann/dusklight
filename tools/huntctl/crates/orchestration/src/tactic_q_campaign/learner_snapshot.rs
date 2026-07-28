use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticQLearnerSnapshotKind {
    Learned,
    Demonstration,
}

/// Immutable identity of one fitted policy and the exact replay root that
/// produced it.
///
/// The transition journal remains the source of full training rows. This
/// compact manifest binds their ordered authenticated identities, episode
/// groups, learner configuration, and serialized fitted model without
/// duplicating a growing corpus into every decision record.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQLearnerSnapshot {
    pub schema: String,
    pub kind: TacticQLearnerSnapshotKind,
    pub execution_authority_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub training_replay_rows: u64,
    pub training_replay_sha256: Digest,
    pub model_revision: u64,
    pub model_config: OptionValueConfig,
    pub model_sha256: Option<Digest>,
}

impl TacticQLearnerSnapshot {
    pub fn from_demonstration(
        corpus: &TacticQTrainingCorpus,
        model_config: OptionValueConfig,
    ) -> Result<Self, TacticQCampaignError> {
        validate_training_corpus(corpus)?;
        let snapshot = Self {
            schema: TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V1.into(),
            kind: TacticQLearnerSnapshotKind::Demonstration,
            execution_authority_sha256: corpus.execution_authority_sha256,
            feature_schema_sha256: corpus.feature_schema_sha256,
            objective_sha256: corpus.objective_sha256,
            root_checkpoint_sha256: corpus.root_checkpoint_sha256,
            training_replay_rows: corpus.transitions.len() as u64,
            training_replay_sha256: training_replay_sha256(
                &corpus.transitions,
                &corpus.episode_groups,
            )?,
            model_revision: 0,
            model_config,
            model_sha256: None,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn content_sha256(&self) -> Result<Digest, TacticQCampaignError> {
        self.validate()?;
        let raw = serde_cbor::to_vec(self)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        Ok(sha256(&raw))
    }

    pub fn validate(&self) -> Result<(), TacticQCampaignError> {
        if self.schema != TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V1
            || self.execution_authority_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.root_checkpoint_sha256 == Digest::ZERO
            || self.training_replay_sha256 == Digest::ZERO
            || (self.kind == TacticQLearnerSnapshotKind::Demonstration
                && (self.model_revision != 0 || self.model_sha256.is_some()))
            || self.model_sha256 == Some(Digest::ZERO)
        {
            return Err(TacticQCampaignError::InvalidState(
                "tactic learner snapshot is invalid",
            ));
        }
        serde_cbor::to_vec(&self.model_config)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        Ok(())
    }
}

/// One immutable fitted policy published by the campaign learner authority.
///
/// Lanes may retain their own executable trajectory and exploration history,
/// but they never mutate these models. The manifest binds the models to the
/// exact ordered replay prefix used by the single fitter.
#[derive(Clone, Debug)]
pub struct TacticQImmutableLearnerSnapshot {
    pub manifest: TacticQLearnerSnapshot,
    pub sha256: Digest,
    pub replay_revision: u64,
    pub(super) training_corpus: Arc<TacticQTrainingCorpus>,
    pub(super) model: Option<Arc<OptionValueModel>>,
    pub(super) generalized_model: Option<Arc<GeneralizedTacticValueModel>>,
    pub(super) goal_distance_feature: usize,
}

impl TacticQImmutableLearnerSnapshot {
    pub fn fit(
        corpus: TacticQTrainingCorpus,
        replay_revision: u64,
        model_revision: u64,
        model_config: OptionValueConfig,
        goal_distance_feature: usize,
    ) -> Result<Self, TacticQCampaignError> {
        validate_training_corpus(&corpus)?;
        if replay_revision != corpus.transitions.len() as u64 {
            return Err(TacticQCampaignError::InvalidState(
                "learner snapshot replay revision does not match its corpus",
            ));
        }
        let model = replay_model(
            corpus.feature_schema_sha256,
            corpus.objective_sha256,
            &corpus.transitions,
            &corpus.episode_groups,
            &model_config,
        )?
        .map(Arc::new);
        let generalized_model = if corpus.transitions.len() < 2 {
            None
        } else {
            Some(Arc::new(
                GeneralizedTacticValueModel::fit_fitted_q_transitions(
                    &corpus.transitions,
                    goal_distance_feature,
                    model_config.fitted_q.iterations,
                    model_config.fitted_q.discount,
                )?,
            ))
        };
        let model_sha256 = model
            .as_ref()
            .map(|model| {
                serde_cbor::to_vec(model.as_ref())
                    .map(|raw| sha256(&raw))
                    .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))
            })
            .transpose()?;
        let manifest = TacticQLearnerSnapshot {
            schema: TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V1.into(),
            kind: TacticQLearnerSnapshotKind::Learned,
            execution_authority_sha256: corpus.execution_authority_sha256,
            feature_schema_sha256: corpus.feature_schema_sha256,
            objective_sha256: corpus.objective_sha256,
            root_checkpoint_sha256: corpus.root_checkpoint_sha256,
            training_replay_rows: corpus.transitions.len() as u64,
            training_replay_sha256: training_replay_sha256(
                &corpus.transitions,
                &corpus.episode_groups,
            )?,
            model_revision,
            model_config,
            model_sha256,
        };
        let sha256 = manifest.content_sha256()?;
        Ok(Self {
            manifest,
            sha256,
            replay_revision,
            training_corpus: Arc::new(corpus),
            model,
            generalized_model,
            goal_distance_feature,
        })
    }

    pub fn training_corpus(&self) -> &TacticQTrainingCorpus {
        &self.training_corpus
    }
}
