use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GoalRelabeledCriticAuthority {
    AchievedGoal,
    NativeTerminal,
}

pub(super) fn goal_relabel_critic_authority(
    has_native_terminal_support: bool,
    terminal_action_deployment_ready: bool,
) -> GoalRelabeledCriticAuthority {
    if has_native_terminal_support && terminal_action_deployment_ready {
        GoalRelabeledCriticAuthority::NativeTerminal
    } else {
        GoalRelabeledCriticAuthority::AchievedGoal
    }
}

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

#[derive(Debug)]
pub(super) struct CachedContinuousTacticDoubleQModel {
    pub(super) goal_distance_feature: usize,
    pub(super) model_revision: u64,
    pub(super) model: Arc<ContinuousTacticDoubleQModel>,
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
                    match GeneralizedTacticValueModel::fit_fitted_q_transitions(
                        &self.training_replay,
                        goal_distance_feature,
                        self.model_config.fitted_q.iterations,
                        self.model_config.fitted_q.discount,
                    ) {
                        Ok(model) => model,
                        Err(GeneralizedTacticValueError::SampleCount) => return Ok(None),
                        Err(error) => return Err(error.into()),
                    }
                }
                TacticValueTreatment::GoalRelabeledFittedQKnnV2
                | TacticValueTreatment::GoalRelabeledFrontierDoubleQV3
                | TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4 => {
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

    /// Native-terminal critic used only after authenticated terminal support
    /// exists under the goal-relabeled treatment.
    ///
    /// The achieved-goal critic deliberately strips terminal authority. It is
    /// useful for cold-start reachability but cannot estimate terminal
    /// ticks-to-go or shorten a successful route.
    pub(super) fn native_terminal_model(
        &self,
        goal_distance_feature: usize,
    ) -> Result<Option<Arc<GeneralizedTacticValueModel>>, TacticQCampaignError> {
        if !self.value_treatment.uses_goal_relabeling()
            || !self
                .training_replay
                .iter()
                .any(|transition| transition.value_sample.terminal)
        {
            return Ok(None);
        }
        if self.campaign_learner_authority_managed {
            return Ok(self
                .native_terminal_model
                .borrow()
                .as_ref()
                .filter(|cached| cached.goal_distance_feature == goal_distance_feature)
                .map(|cached| Arc::clone(&cached.model)));
        }
        if self.training_replay.len() < 2 {
            return Ok(None);
        }
        let stale = self
            .native_terminal_model
            .borrow()
            .as_ref()
            .is_none_or(|cached| {
                cached.goal_distance_feature != goal_distance_feature
                    || cached.model_revision != self.model_revision
            });
        if stale {
            let model = match GeneralizedTacticValueModel::fit_fitted_q_transitions(
                &self.training_replay,
                goal_distance_feature,
                self.model_config.fitted_q.iterations,
                self.model_config.fitted_q.discount,
            ) {
                Ok(model) => Arc::new(model),
                Err(GeneralizedTacticValueError::SampleCount) => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            *self.native_terminal_model.borrow_mut() = Some(CachedGeneralizedTacticValueModel {
                goal_distance_feature,
                model_revision: self.model_revision,
                model,
            });
        }
        Ok(self
            .native_terminal_model
            .borrow()
            .as_ref()
            .map(|cached| Arc::clone(&cached.model)))
    }

    pub(super) fn active_goal_relabel_model(
        &self,
        goal_distance_feature: usize,
        terminal_action_deployment_ready: bool,
    ) -> Result<Option<Arc<GeneralizedTacticValueModel>>, TacticQCampaignError> {
        match goal_relabel_critic_authority(
            self.training_replay
                .iter()
                .any(|transition| transition.value_sample.terminal),
            terminal_action_deployment_ready,
        ) {
            GoalRelabeledCriticAuthority::AchievedGoal => {
                self.generalized_model(goal_distance_feature)
            }
            GoalRelabeledCriticAuthority::NativeTerminal => {
                self.native_terminal_model(goal_distance_feature)
            }
        }
    }

    pub(super) fn native_terminal_action_model(
        &self,
        goal_distance_feature: usize,
    ) -> Result<Option<Arc<ContinuousTacticDoubleQModel>>, TacticQCampaignError> {
        if !self.value_treatment.uses_goal_relabeling()
            || !self
                .training_replay
                .iter()
                .any(|transition| transition.value_sample.terminal)
        {
            return Ok(None);
        }
        if self.campaign_learner_authority_managed {
            return Ok(self
                .native_terminal_action_model
                .borrow()
                .as_ref()
                .filter(|cached| cached.goal_distance_feature == goal_distance_feature)
                .map(|cached| Arc::clone(&cached.model)));
        }
        if self.training_replay.len() < 2 {
            return Ok(None);
        }
        let stale = self
            .native_terminal_action_model
            .borrow()
            .as_ref()
            .is_none_or(|cached| {
                cached.goal_distance_feature != goal_distance_feature
                    || cached.model_revision != self.model_revision
            });
        if stale {
            let fitted = if self.value_treatment.uses_universal_terminal_action_head() {
                ContinuousTacticDoubleQModel::fit_universal_action_head(
                    &self.training_replay,
                    goal_distance_feature,
                    self.model_config.fitted_q.iterations,
                    self.model_config.fitted_q.discount,
                )
            } else {
                ContinuousTacticDoubleQModel::fit(
                    &self.training_replay,
                    goal_distance_feature,
                    self.model_config.fitted_q.iterations,
                    self.model_config.fitted_q.discount,
                )
            };
            let model = match fitted {
                Ok(model) => Arc::new(model),
                Err(GeneralizedTacticValueError::SampleCount) => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            *self.native_terminal_action_model.borrow_mut() =
                Some(CachedContinuousTacticDoubleQModel {
                    goal_distance_feature,
                    model_revision: self.model_revision,
                    model,
                });
        }
        Ok(self
            .native_terminal_action_model
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
            let model = match ContinuousTacticValueModel::fit(
                &self.training_replay,
                goal_distance_feature,
                self.model_config.fitted_q.iterations,
                self.model_config.fitted_q.discount,
            ) {
                Ok(model) => Arc::new(model),
                Err(GeneralizedTacticValueError::SampleCount) => return Ok(None),
                Err(error) => return Err(error.into()),
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_terminal_support_requires_calibrated_ranking_before_handoff() {
        assert_eq!(
            goal_relabel_critic_authority(false, false),
            GoalRelabeledCriticAuthority::AchievedGoal
        );
        assert_eq!(
            goal_relabel_critic_authority(true, true),
            GoalRelabeledCriticAuthority::NativeTerminal
        );
        assert_eq!(
            goal_relabel_critic_authority(true, false),
            GoalRelabeledCriticAuthority::AchievedGoal,
            "terminal samples alone must not disable the all-experience critic",
        );
    }
}
