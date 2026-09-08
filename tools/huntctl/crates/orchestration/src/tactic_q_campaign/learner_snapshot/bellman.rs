//! Stateful Bellman snapshots bind the actual fitted values and their parent.
use super::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BellmanStateReference {
    pub state_sha256: Digest,
    pub previous_snapshot_sha256: Digest,
    pub completed_backups: u64,
    pub last_max_target_residual: f64,
}

#[derive(Clone, Copy)]
pub(super) enum BellmanSnapshotInput<'a> {
    Update(&'a TacticQImmutableLearnerSnapshot),
    Restore {
        model: &'a ContinuousTacticValueModel,
        reference: &'a BellmanStateReference,
    },
}

impl BellmanSnapshotInput<'_> {
    pub(super) fn reference(
        self,
        model: Option<&ContinuousTacticValueModel>,
    ) -> Result<BellmanStateReference, TacticQCampaignError> {
        let model = model.ok_or(TacticQCampaignError::InvalidState(
            "Bellman snapshot lacks model",
        ))?;
        let training = model
            .bellman_training()
            .ok_or(TacticQCampaignError::InvalidState(
                "Bellman snapshot lacks training state",
            ))?;
        let reference = BellmanStateReference {
            state_sha256: sha256(&model.training_state_bytes()?),
            previous_snapshot_sha256: match self {
                Self::Update(prior) => prior.sha256,
                Self::Restore { reference, .. } => reference.previous_snapshot_sha256,
            },
            completed_backups: training.completed_backups,
            last_max_target_residual: training.last_max_target_residual,
        };
        if let Self::Restore {
            reference: expected,
            ..
        } = self
        {
            if reference != *expected {
                return Err(TacticQCampaignError::InvalidState(
                    "restored Bellman state is detached from manifest",
                ));
            }
        }
        Ok(reference)
    }
}

pub(super) fn validate_reference(
    manifest: &TacticQLearnerSnapshot,
) -> Result<(), TacticQCampaignError> {
    if (manifest.schema == TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V8) != manifest.bellman_state.is_some()
    {
        return Err(TacticQCampaignError::InvalidState(
            "Bellman snapshot schema/state mismatch",
        ));
    }
    if let Some(reference) = &manifest.bellman_state {
        if !manifest.value_treatment.uses_bellman_forest()
            || manifest.kind != TacticQLearnerSnapshotKind::Learned
            || manifest.training_replay_rows < 2
            || reference.state_sha256 == Digest::ZERO
            || reference.previous_snapshot_sha256 == Digest::ZERO
            || reference.completed_backups == 0
            || !reference.last_max_target_residual.is_finite()
            || reference.last_max_target_residual < 0.0
        {
            return Err(TacticQCampaignError::InvalidState(
                "invalid Bellman snapshot reference",
            ));
        }
    }
    Ok(())
}

impl TacticQImmutableLearnerSnapshot {
    pub(crate) fn update_verified_bellman(
        corpus: TacticQTrainingCorpus,
        replay_revision: u64,
        training_hash: Digest,
        prior: &Self,
    ) -> Result<Self, TacticQCampaignError> {
        let old = &prior.manifest;
        old.validate()?;
        if old.content_sha256()? != prior.sha256 {
            return Err(TacticQCampaignError::InvalidState(
                "prior Bellman snapshot identity changed",
            ));
        }
        let old_rows = usize::try_from(old.training_replay_rows)
            .map_err(|_| TacticQCampaignError::InvalidState("prior replay too large"))?;
        if !old.value_treatment.uses_bellman_forest()
            || corpus.transitions.len() < 2
            || old_rows > corpus.transitions.len()
            || old_rows > corpus.episode_groups.len()
            || corpus.execution_authority_sha256 != old.execution_authority_sha256
            || corpus.feature_schema_sha256 != old.feature_schema_sha256
            || corpus.objective_sha256 != old.objective_sha256
            || corpus.root_checkpoint_sha256 != old.root_checkpoint_sha256
            || training_replay_sha256(
                &corpus.transitions[..old_rows],
                &corpus.episode_groups[..old_rows],
            )? != old.training_replay_sha256
        {
            return Err(TacticQCampaignError::InvalidState(
                "Bellman update does not extend its prior replay",
            ));
        }
        Self::fit_verified_replay_inner(
            corpus,
            replay_revision,
            old.model_revision
                .checked_add(1)
                .ok_or(TacticQCampaignError::InvalidState(
                    "model revision overflow",
                ))?,
            old.model_config.clone(),
            prior.goal_distance_feature,
            old.value_treatment,
            None,
            None,
            training_hash,
            Some(BellmanSnapshotInput::Update(prior)),
        )
    }

    pub(crate) fn restore_verified_bellman(
        corpus: TacticQTrainingCorpus,
        replay_revision: u64,
        training_hash: Digest,
        manifest: &TacticQLearnerSnapshot,
        goal_distance_feature: usize,
        model: &ContinuousTacticValueModel,
    ) -> Result<Self, TacticQCampaignError> {
        manifest.validate()?;
        let reference =
            manifest
                .bellman_state
                .as_ref()
                .ok_or(TacticQCampaignError::InvalidState(
                    "snapshot has no durable Bellman state",
                ))?;
        if model
            .bellman_replay_stats()
            .is_none_or(|stats| stats.native_rows != corpus.transitions.len())
            || model
                .bellman_training()
                .is_none_or(|training| training.discount != manifest.model_config.fitted_q.discount)
            || model.goal_query_kind().is_some()
                != (manifest.value_treatment == TacticValueTreatment::HindsightBellmanForestV3)
        {
            return Err(TacticQCampaignError::InvalidState(
                "Bellman state does not match replay/configuration",
            ));
        }
        let restored = Self::fit_verified_replay_inner(
            corpus,
            replay_revision,
            manifest.model_revision,
            manifest.model_config.clone(),
            goal_distance_feature,
            manifest.value_treatment,
            None,
            None,
            training_hash,
            Some(BellmanSnapshotInput::Restore { model, reference }),
        )?;
        if restored.manifest != *manifest {
            return Err(TacticQCampaignError::InvalidState(
                "restored Bellman snapshot changed identity",
            ));
        }
        Ok(restored)
    }

    pub(crate) fn bellman_state_bytes(&self) -> Result<Option<Vec<u8>>, TacticQCampaignError> {
        if self.manifest.bellman_state.is_none() {
            return Ok(None);
        }
        self.continuous_model
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "snapshot lacks Bellman model",
            ))?
            .training_state_bytes()
            .map(Some)
            .map_err(Into::into)
    }
}
