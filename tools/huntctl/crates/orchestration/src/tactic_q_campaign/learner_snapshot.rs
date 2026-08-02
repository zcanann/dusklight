use super::*;
use dusklight_learning::goal_reachability_calibration::calibrate_goal_reachability;

fn is_default_value_treatment(value: &TacticValueTreatment) -> bool {
    *value == TacticValueTreatment::LocalGeneralizedFittedQKnnV1
}

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
    #[serde(default, skip_serializing_if = "is_default_value_treatment")]
    pub value_treatment: TacticValueTreatment,
    pub execution_authority_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub training_replay_rows: u64,
    pub training_replay_sha256: Digest,
    pub model_revision: u64,
    pub model_config: OptionValueConfig,
    pub model_sha256: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_reachability_calibration: Option<GoalReachabilityCalibration>,
}

impl TacticQLearnerSnapshot {
    pub fn from_demonstration(
        corpus: &TacticQTrainingCorpus,
        model_config: OptionValueConfig,
        value_treatment: TacticValueTreatment,
    ) -> Result<Self, TacticQCampaignError> {
        validate_training_corpus(corpus)?;
        let snapshot = Self {
            schema: TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V4.into(),
            kind: TacticQLearnerSnapshotKind::Demonstration,
            value_treatment,
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
            goal_reachability_calibration: None,
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
        let legacy = self.schema == TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V1
            && self.value_treatment == TacticValueTreatment::LocalGeneralizedFittedQKnnV1;
        let current = self.schema == TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V2
            || self.schema == TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V3
            || self.schema == TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V4;
        if (!legacy && !current)
            || self.execution_authority_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.root_checkpoint_sha256 == Digest::ZERO
            || self.training_replay_sha256 == Digest::ZERO
            || (self.kind == TacticQLearnerSnapshotKind::Demonstration
                && (self.model_revision != 0 || self.model_sha256.is_some()))
            || self.model_sha256 == Some(Digest::ZERO)
            || (self.goal_reachability_calibration.is_some()
                && (!matches!(
                    self.schema.as_str(),
                    TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V3 | TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V4
                ) || !self.value_treatment.uses_goal_relabeling()))
            || self
                .goal_reachability_calibration
                .as_ref()
                .is_some_and(|calibration| {
                    u64::try_from(calibration.source_transitions)
                        .map_or(true, |rows| rows > self.training_replay_rows)
                })
        {
            return Err(TacticQCampaignError::InvalidState(
                "tactic learner snapshot is invalid",
            ));
        }
        serde_cbor::to_vec(&self.model_config)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        if let Some(calibration) = &self.goal_reachability_calibration {
            calibration.validate()?;
        }
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
    pub(super) native_terminal_model: Option<Arc<GeneralizedTacticValueModel>>,
    pub(super) native_terminal_action_model: Option<Arc<ContinuousTacticDoubleQModel>>,
    pub(super) continuous_model: Option<Arc<ContinuousTacticValueModel>>,
    pub(super) goal_distance_feature: usize,
}

impl TacticQImmutableLearnerSnapshot {
    pub fn fit(
        corpus: TacticQTrainingCorpus,
        replay_revision: u64,
        model_revision: u64,
        model_config: OptionValueConfig,
        goal_distance_feature: usize,
        value_treatment: TacticValueTreatment,
    ) -> Result<Self, TacticQCampaignError> {
        Self::fit_with_prior_goal_reachability_calibration(
            corpus,
            replay_revision,
            model_revision,
            model_config,
            goal_distance_feature,
            value_treatment,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fit_with_prior_goal_reachability_calibration(
        corpus: TacticQTrainingCorpus,
        replay_revision: u64,
        model_revision: u64,
        model_config: OptionValueConfig,
        goal_distance_feature: usize,
        value_treatment: TacticValueTreatment,
        prior_calibration: Option<&GoalReachabilityCalibration>,
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
            match value_treatment {
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1 => {
                    match GeneralizedTacticValueModel::fit_fitted_q_transitions(
                        &corpus.transitions,
                        goal_distance_feature,
                        model_config.fitted_q.iterations,
                        model_config.fitted_q.discount,
                    ) {
                        Ok(model) => Some(Arc::new(model)),
                        Err(GeneralizedTacticValueError::SampleCount) => None,
                        Err(error) => return Err(error.into()),
                    }
                }
                TacticValueTreatment::GoalRelabeledFittedQKnnV2
                | TacticValueTreatment::GoalRelabeledFrontierDoubleQV3
                | TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4 => Some(Arc::new(
                    GeneralizedTacticValueModel::fit_achieved_goal_returns(
                        &corpus.transitions,
                        goal_distance_feature,
                    )?,
                )),
                TacticValueTreatment::ContinuousFittedQForestV1 => None,
            }
        };
        let native_terminal_model = if corpus.transitions.len() >= 2
            && value_treatment.uses_goal_relabeling()
            && corpus
                .transitions
                .iter()
                .any(|transition| transition.value_sample.terminal)
        {
            match GeneralizedTacticValueModel::fit_fitted_q_transitions(
                &corpus.transitions,
                goal_distance_feature,
                model_config.fitted_q.iterations,
                model_config.fitted_q.discount,
            ) {
                Ok(model) => Some(Arc::new(model)),
                Err(GeneralizedTacticValueError::SampleCount) => None,
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        let native_terminal_action_model = if native_terminal_model.is_some() {
            let model = if value_treatment.uses_universal_terminal_action_head() {
                ContinuousTacticDoubleQModel::fit_universal_action_head(
                    &corpus.transitions,
                    goal_distance_feature,
                    model_config.fitted_q.iterations,
                    model_config.fitted_q.discount,
                )
            } else {
                ContinuousTacticDoubleQModel::fit(
                    &corpus.transitions,
                    goal_distance_feature,
                    model_config.fitted_q.iterations,
                    model_config.fitted_q.discount,
                )
            }?;
            Some(Arc::new(model))
        } else {
            None
        };
        let continuous_model = if corpus.transitions.len() >= 2
            && value_treatment == TacticValueTreatment::ContinuousFittedQForestV1
        {
            match ContinuousTacticValueModel::fit(
                &corpus.transitions,
                goal_distance_feature,
                model_config.fitted_q.iterations,
                model_config.fitted_q.discount,
            ) {
                Ok(model) => Some(Arc::new(model)),
                Err(GeneralizedTacticValueError::SampleCount) => None,
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        let goal_reachability_calibration =
            goal_reachability_calibration_prefix_rows(value_treatment, corpus.transitions.len())
                .map(|calibration_rows| {
                    if let Some(prior) = prior_calibration
                        .filter(|prior| prior.source_transitions == calibration_rows)
                    {
                        Ok(prior.clone())
                    } else {
                        calibrate_goal_reachability(
                            &corpus.transitions[..calibration_rows],
                            goal_distance_feature,
                        )
                        .map_err(TacticQCampaignError::from)
                    }
                })
                .transpose()?;
        let model_sha256 = model
            .as_ref()
            .map(|model| {
                serde_cbor::to_vec(model.as_ref())
                    .map(|raw| sha256(&raw))
                    .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))
            })
            .transpose()?;
        let manifest = TacticQLearnerSnapshot {
            schema: TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V4.into(),
            kind: TacticQLearnerSnapshotKind::Learned,
            value_treatment,
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
            goal_reachability_calibration: goal_reachability_calibration.clone(),
        };
        let sha256 = manifest.content_sha256()?;
        Ok(Self {
            manifest,
            sha256,
            replay_revision,
            training_corpus: Arc::new(corpus),
            model,
            generalized_model,
            native_terminal_model,
            native_terminal_action_model,
            continuous_model,
            goal_distance_feature,
        })
    }

    pub fn training_corpus(&self) -> &TacticQTrainingCorpus {
        &self.training_corpus
    }
}

fn calibration_prefix_rows(available_rows: usize) -> usize {
    if available_rows == 0 {
        0
    } else {
        1_usize << available_rows.ilog2()
    }
}

fn goal_reachability_calibration_prefix_rows(
    value_treatment: TacticValueTreatment,
    available_rows: usize,
) -> Option<usize> {
    value_treatment
        .uses_goal_relabeling()
        .then(|| calibration_prefix_rows(available_rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reachability_calibration_refreshes_only_at_doubling_prefixes() {
        assert_eq!(calibration_prefix_rows(0), 0);
        assert_eq!(calibration_prefix_rows(1), 1);
        assert_eq!(calibration_prefix_rows(2), 2);
        assert_eq!(calibration_prefix_rows(3), 2);
        assert_eq!(calibration_prefix_rows(15), 8);
        assert_eq!(calibration_prefix_rows(16), 16);
        assert_eq!(calibration_prefix_rows(31), 16);
        assert_eq!(calibration_prefix_rows(32), 32);
    }

    #[test]
    fn reachability_calibration_applies_to_all_goal_relabel_treatments() {
        assert_eq!(
            goal_reachability_calibration_prefix_rows(
                TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4,
                65,
            ),
            Some(64),
        );
        assert_eq!(
            goal_reachability_calibration_prefix_rows(
                TacticValueTreatment::GoalRelabeledFrontierDoubleQV3,
                65,
            ),
            Some(64),
        );
        assert_eq!(
            goal_reachability_calibration_prefix_rows(
                TacticValueTreatment::GoalRelabeledFittedQKnnV2,
                65,
            ),
            Some(64),
        );
        assert_eq!(
            goal_reachability_calibration_prefix_rows(
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
                65,
            ),
            None,
        );
    }

    #[test]
    fn legacy_snapshot_defaults_to_the_original_value_treatment() {
        let legacy = TacticQLearnerSnapshot {
            schema: TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V1.into(),
            kind: TacticQLearnerSnapshotKind::Learned,
            value_treatment: TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
            execution_authority_sha256: Digest([1; 32]),
            feature_schema_sha256: Digest([2; 32]),
            objective_sha256: Digest([3; 32]),
            root_checkpoint_sha256: Digest([4; 32]),
            training_replay_rows: 0,
            training_replay_sha256: Digest([5; 32]),
            model_revision: 0,
            model_config: OptionValueConfig::default(),
            model_sha256: None,
            goal_reachability_calibration: None,
        };
        let raw = serde_cbor::to_vec(&legacy).unwrap();
        let decoded: TacticQLearnerSnapshot = serde_cbor::from_slice(&raw).unwrap();
        assert_eq!(
            decoded.value_treatment,
            TacticValueTreatment::LocalGeneralizedFittedQKnnV1
        );
        decoded.validate().unwrap();
        assert_eq!(decoded.content_sha256().unwrap(), sha256(&raw));
    }
}
