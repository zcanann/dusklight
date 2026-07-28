use super::*;

#[derive(Debug)]
pub(super) struct CachedGeneralizedTacticValueModel {
    pub(super) goal_distance_feature: usize,
    pub(super) model_revision: u64,
    pub(super) model: Arc<GeneralizedTacticValueModel>,
}

#[derive(Debug)]
pub(super) struct CachedContinuousTacticValueModel {
    pub(super) goal_distance_feature: usize,
    pub(super) model_revision: u64,
    pub(super) model: Arc<ContinuousTacticValueModel>,
}

impl TacticQCampaign {
    pub(super) fn generalized_model(
        &self,
        goal_distance_feature: usize,
    ) -> Result<Option<Arc<GeneralizedTacticValueModel>>, TacticQCampaignError> {
        if self.campaign_learner_authority_managed {
            return Ok(self
                .generalized_model
                .borrow()
                .as_ref()
                .filter(|cached| cached.goal_distance_feature == goal_distance_feature)
                .map(|cached| Arc::clone(&cached.model)));
        }
        if self.training_replay.len() < 2 {
            return Ok(None);
        }
        let stale = self
            .generalized_model
            .borrow()
            .as_ref()
            .is_none_or(|cached| {
                cached.goal_distance_feature != goal_distance_feature
                    || cached.model_revision != self.model_revision
            });
        if stale {
            let model = Arc::new(match self.value_treatment {
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1 => {
                    GeneralizedTacticValueModel::fit_fitted_q_transitions(
                        &self.training_replay,
                        goal_distance_feature,
                        self.model_config.fitted_q.iterations,
                        self.model_config.fitted_q.discount,
                    )?
                }
                TacticValueTreatment::GoalRelabeledFittedQKnnV2 => {
                    GeneralizedTacticValueModel::fit_achieved_goal_returns(
                        &self.training_replay,
                        goal_distance_feature,
                    )?
                }
                TacticValueTreatment::ContinuousFittedQForestV1 => {
                    return Ok(None);
                }
            });
            *self.generalized_model.borrow_mut() = Some(CachedGeneralizedTacticValueModel {
                goal_distance_feature,
                model_revision: self.model_revision,
                model,
            });
        }
        Ok(self
            .generalized_model
            .borrow()
            .as_ref()
            .map(|cached| Arc::clone(&cached.model)))
    }

    pub(super) fn continuous_model(
        &self,
        goal_distance_feature: usize,
    ) -> Result<Option<Arc<ContinuousTacticValueModel>>, TacticQCampaignError> {
        if self.campaign_learner_authority_managed {
            return Ok(self
                .continuous_model
                .borrow()
                .as_ref()
                .filter(|cached| cached.goal_distance_feature == goal_distance_feature)
                .map(|cached| Arc::clone(&cached.model)));
        }
        if self.value_treatment != TacticValueTreatment::ContinuousFittedQForestV1
            || self.training_replay.len() < 2
        {
            return Ok(None);
        }
        let stale = self
            .continuous_model
            .borrow()
            .as_ref()
            .is_none_or(|cached| {
                cached.goal_distance_feature != goal_distance_feature
                    || cached.model_revision != self.model_revision
            });
        if stale {
            let model = Arc::new(ContinuousTacticValueModel::fit(
                &self.training_replay,
                goal_distance_feature,
                self.model_config.fitted_q.iterations,
                self.model_config.fitted_q.discount,
            )?);
            *self.continuous_model.borrow_mut() = Some(CachedContinuousTacticValueModel {
                goal_distance_feature,
                model_revision: self.model_revision,
                model,
            });
        }
        Ok(self
            .continuous_model
            .borrow()
            .as_ref()
            .map(|cached| Arc::clone(&cached.model)))
    }
}
