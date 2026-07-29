use super::*;

impl TacticQCampaign {
    pub fn decide<E, F>(
        &self,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        encode: &F,
    ) -> Result<TacticQDecision, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        let mut batch = self.decide_batch(catalog, blueprints, encode, 1)?;
        let selected = batch
            .proposals
            .pop()
            .ok_or(TacticQCampaignError::InvalidState(
                "tactic proposal batch is empty",
            ))?;
        Ok(TacticQDecision {
            ranking: batch.ranking,
            selected,
        })
    }

    pub fn decide_batch<E, F>(
        &self,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        encode: &F,
        maximum_proposals: usize,
    ) -> Result<TacticQProposalBatch, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        let live = LiveTacticCatalog::build(&self.current, catalog, blueprints)?;
        let features = encode(&self.current.snapshot)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        if features.is_empty() || features.iter().any(|value| !value.is_finite()) {
            return Err(TacticQCampaignError::Features(
                "state encoding is empty or non-finite".into(),
            ));
        }
        let ranking = if let Some(model) = &self.model {
            live.rank(model, &features)?
        } else {
            LiveTacticRanking {
                learner_snapshot_sha256: live.learner_snapshot_sha256,
                action_universe_sha256: live.action_universe_sha256,
                choices: live.choices.clone(),
                values: AvailableOptionRanking {
                    ranked: Vec::new(),
                    unsupported: live.descriptors().cloned().collect(),
                },
            }
        };
        let current_cell = tactic_state_descriptor(&self.current.snapshot, false);
        let tried_here = self
            .training_replay
            .iter()
            .filter(|transition| tactic_state_descriptor(&transition.before, false) == current_cell)
            .map(|transition| transition.value_sample.action.option_id.as_str())
            .collect::<BTreeSet<_>>();
        let state_untried = live
            .descriptors()
            .filter(|descriptor| !tried_here.contains(descriptor.option_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let mut proposals = choose_tactic_batch_with_state_untried(
            &ranking,
            self.decision_index,
            self.exploration,
            &state_untried,
            maximum_proposals,
        )?;
        ensure_blueprint_proposal(&ranking, maximum_proposals, &mut proposals)?;
        Ok(TacticQProposalBatch { ranking, proposals })
    }

    /// Rank an ephemeral set of bounded instances under a stable tactic-family
    /// schema. The executable instances may be new at this decision; the
    /// option-value model scores exact instances it has seen and leaves new
    /// parameter combinations explicit for exploration.
    pub fn decide_parameterized_batch<E, F>(
        &self,
        proposal_catalog: &TacticAssetCatalog,
        proposal_blueprints: &[TacticBlueprint],
        family_schema_sha256: Digest,
        encode: &F,
        maximum_proposals: usize,
    ) -> Result<TacticQProposalBatch, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        self.decide_parameterized_batch_with_policy(
            proposal_catalog,
            proposal_blueprints,
            family_schema_sha256,
            encode,
            maximum_proposals,
            0,
            TacticProposalPolicy::Learned,
            None,
            false,
        )
    }

    pub fn decide_parameterized_batch_with_policy<E, F>(
        &self,
        proposal_catalog: &TacticAssetCatalog,
        proposal_blueprints: &[TacticBlueprint],
        family_schema_sha256: Digest,
        encode: &F,
        maximum_proposals: usize,
        acquisition_partition: u64,
        policy: TacticProposalPolicy,
        goal_distance_feature: Option<usize>,
        force_exploration: bool,
    ) -> Result<TacticQProposalBatch, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        self.current.validate()?;
        if family_schema_sha256 == Digest::ZERO || maximum_proposals == 0 {
            return Err(TacticQCampaignError::InvalidState(
                "parameterized tactic proposal schema or capacity is invalid",
            ));
        }
        for blueprint in proposal_blueprints {
            blueprint
                .compile_static(proposal_catalog)
                .map_err(LiveTacticCatalogError::Blueprint)?;
        }
        let applicable = ApplicableTacticChoices::enumerate(
            proposal_catalog,
            proposal_blueprints,
            |description| tactic_intrinsically_applicable(description, &self.current.snapshot),
            |_| Some(false),
        )
        .map_err(LiveTacticCatalogError::Blueprint)?;
        let mut choices = Vec::with_capacity(applicable.candidates.len());
        for entry in proposal_catalog.entries() {
            proposal_catalog
                .prepare_execution(entry.option_id())
                .map_err(LiveTacticCatalogError::Asset)?;
        }
        for (candidate, applicable) in applicable
            .candidates
            .into_iter()
            .zip(applicable.applicable_mask)
        {
            choices.push(LearnerActionMaskEntry {
                choice_id: candidate.choice_id,
                kind: candidate.kind,
                descriptor: candidate.descriptor,
                duration: candidate.duration,
                applicable,
            });
        }
        let descriptors = choices
            .iter()
            .map(|choice| choice.descriptor.clone())
            .collect::<Vec<_>>();
        let features = encode(&self.current.snapshot)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        if features.is_empty() || features.iter().any(|value| !value.is_finite()) {
            return Err(TacticQCampaignError::Features(
                "state encoding is empty or non-finite".into(),
            ));
        }
        let values = if let Some(model) = &self.model {
            model.rank_available_options(&features, &descriptors)?
        } else {
            AvailableOptionRanking {
                ranked: Vec::new(),
                unsupported: descriptors.clone(),
            }
        };
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: self.current.snapshot_sha256,
            action_universe_sha256: family_schema_sha256,
            choices,
            values,
        };
        let current_cell = tactic_state_descriptor(&self.current.snapshot, false);
        let tried_here = self
            .training_replay
            .iter()
            .filter(|transition| tactic_state_descriptor(&transition.before, false) == current_cell)
            .map(|transition| transition.value_sample.action.option_id.as_str())
            .collect::<BTreeSet<_>>();
        let state_untried = applicable_untried_descriptors(&ranking.choices, &tried_here);
        let exploration = if force_exploration {
            TacticExplorationConfig {
                seed: self.exploration.seed,
                epsilon_per_million: 1_000_000,
            }
        } else {
            self.exploration
        };
        let mut proposals = choose_tactic_batch_for_policy(
            &ranking,
            self.decision_index,
            exploration,
            &state_untried,
            maximum_proposals,
            policy,
        )?;
        let forced_primary = if force_exploration {
            let proposal = proposals
                .first()
                .cloned()
                .ok_or(TacticQCampaignError::InvalidState(
                    "forced exploration did not produce a primary proposal",
                ))?;
            if !matches!(
                proposal.reason,
                TacticSelectionReason::Epsilon | TacticSelectionReason::UnsupportedBootstrap
            ) {
                return Err(TacticQCampaignError::InvalidState(
                    "forced exploration primary is value-selected",
                ));
            }
            Some(proposal)
        } else {
            None
        };
        if policy != TacticProposalPolicy::RandomValid {
            ensure_blueprint_proposal(&ranking, maximum_proposals, &mut proposals)?;
        }
        if policy == TacticProposalPolicy::Learned {
            let context = GeneralizedTacticContext::from_facts(&self.current.snapshot)?;
            let applicable_descriptors = ranking
                .choices
                .iter()
                .filter(|choice| choice.applicable)
                .map(|choice| choice.descriptor.clone())
                .collect::<Vec<_>>();
            // Partition zero is the dedicated terminal-support policy lane.
            // A one-seed plan cycles acquisition ranks to retain parallel-lane
            // coverage, so an authenticated demonstration-frontier
            // intervention must also select terminal support explicitly rather
            // than silently becoming an ordinary rank-N acquisition. Remaining
            // boundaries stay Q-ranked, preserving independent improvement.
            let terminal_support_acquisition = acquisition_partition == 0 || force_exploration;
            let native_terminal_supported = self
                .training_replay
                .iter()
                .any(|transition| transition.value_sample.terminal);
            if let Some(goal_distance_feature) = goal_distance_feature {
                let ranked_applicable = match self.value_treatment {
                    TacticValueTreatment::LocalGeneralizedFittedQKnnV1 => self
                        .generalized_model(goal_distance_feature)?
                        .map(|model| {
                            if terminal_support_acquisition {
                                model.rank_terminal_support(
                                    &features,
                                    &context,
                                    &applicable_descriptors,
                                )
                            } else {
                                model.rank(&features, &context, &applicable_descriptors)
                            }
                        })
                        .transpose()?
                        .map(|estimates| {
                            estimates
                                .into_iter()
                                .map(|estimate| estimate.descriptor)
                                .collect::<Vec<_>>()
                        }),
                    TacticValueTreatment::GoalRelabeledFittedQKnnV2 => {
                        if native_terminal_supported {
                            self.native_terminal_action_model(goal_distance_feature)?
                                .map(|model| {
                                    model.rank(&features, &context, &applicable_descriptors)
                                })
                                .transpose()?
                                .map(|estimates| {
                                    estimates
                                        .into_iter()
                                        .map(|estimate| estimate.descriptor)
                                        .collect::<Vec<_>>()
                                })
                        } else {
                            self.active_goal_relabel_model(goal_distance_feature)?
                                .map(|model| {
                                    model.rank(&features, &context, &applicable_descriptors)
                                })
                                .transpose()?
                                .map(|estimates| {
                                    estimates
                                        .into_iter()
                                        .map(|estimate| estimate.descriptor)
                                        .collect::<Vec<_>>()
                                })
                        }
                    }
                    TacticValueTreatment::ContinuousFittedQForestV1 => self
                        .continuous_model(goal_distance_feature)?
                        .map(|model| model.rank(&features, &context, &applicable_descriptors))
                        .transpose()?
                        .map(|estimates| {
                            estimates
                                .into_iter()
                                .map(|estimate| estimate.descriptor)
                                .collect::<Vec<_>>()
                        }),
                };
                if let Some(ranked_applicable) = ranked_applicable {
                    ensure_generalized_value_acquisition(
                        &ranked_applicable,
                        acquisition_partition,
                        maximum_proposals,
                        &mut proposals,
                    )?;
                    if terminal_support_acquisition
                        && (self.value_treatment
                            == TacticValueTreatment::LocalGeneralizedFittedQKnnV1
                            || (self.value_treatment
                                == TacticValueTreatment::GoalRelabeledFittedQKnnV2
                                && native_terminal_supported))
                    {
                        ensure_terminal_support_factor_acquisitions(
                            &ranked_applicable,
                            maximum_proposals,
                            &mut proposals,
                        )?;
                    }
                }
            }
            ensure_action_factor_coverage(
                &context,
                &applicable_descriptors,
                &state_untried,
                acquisition_partition,
                maximum_proposals,
                &mut proposals,
            )?;
            retain_generalized_value_acquisition(&mut proposals)?;
        }
        if let Some(primary) = forced_primary {
            proposals.retain(|proposal| proposal.descriptor != primary.descriptor);
            proposals.insert(0, primary);
            proposals.truncate(maximum_proposals);
        }
        if proposals.iter().any(|proposal| {
            !ranking.choices.iter().any(|choice| {
                choice.applicable
                    && choice.choice_id == proposal.descriptor.option_id
                    && choice.descriptor == proposal.descriptor
            })
        }) {
            return Err(TacticQCampaignError::InvalidState(
                "parameterized proposal batch contains an inapplicable tactic".into(),
            ));
        }
        Ok(TacticQProposalBatch { ranking, proposals })
    }

    /// Score and capture a native proposal without mutating the retained
    /// campaign path. Callers can evaluate several outcomes from this exact
    /// boundary, choose one deterministically, and admit only that winner.
    pub fn evaluate_rewarded_outcome<E, F>(
        &self,
        outcome: NativeTacticWorkerOutcome,
        encode: &F,
        reward_spec: &TacticRewardSpec,
    ) -> Result<EvaluatedRewardedTacticOutcome, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        if outcome.selected.decision_index != self.decision_index
            || outcome.selected.learner_snapshot_sha256 != self.current.snapshot_sha256
            || outcome.source_checkpoint_sha256 != self.root_checkpoint_sha256
            || !extends(&self.route_tape, &outcome.route_tape)
        {
            return Err(TacticQCampaignError::InvalidState(
                "native proposal outcome is detached from the campaign boundary",
            ));
        }
        let state = encode(&self.current.snapshot)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let next_state = encode(&outcome.next_facts)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let endpoint = tactic_state_descriptor(&outcome.next_facts, outcome.terminal);
        let reward = reward_spec.evaluate_with_motion(
            self.feature_schema_sha256,
            &state,
            &next_state,
            outcome.execution.duration.realized_ticks,
            outcome.terminal,
            !self.visited_states.contains(&endpoint),
            outcome
                .next_facts
                .recent_option
                .as_ref()
                .and_then(|option| option.trajectory),
        )?;
        let source_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let next_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &outcome.route_tape)?;
        let mut transition = OptionTransitionSample::capture(
            self.feature_schema_sha256,
            source_checkpoint_sha256,
            next_checkpoint_sha256,
            self.current.snapshot.clone(),
            outcome.next_facts.clone(),
            outcome.execution.clone(),
            &outcome.route_tape,
            reward.training_reward,
            outcome.terminal,
            encode,
        )?;
        transition.execution_authority_sha256 = self.execution_authority_sha256;
        transition.validate()?;
        Ok(EvaluatedRewardedTacticOutcome {
            outcome,
            transition,
            reward,
        })
    }

    /// Admit every native-evaluated alternative into the deduplicated training
    /// replay without changing the executable retained path. The subsequent
    /// winner admission performs the scheduled critic refit over this complete
    /// batch.
    pub fn admit_evaluated_replay(
        &mut self,
        evaluated: &[EvaluatedRewardedTacticOutcome],
        episode_groups: &[u64],
    ) -> Result<usize, TacticQCampaignError> {
        if evaluated.is_empty()
            || evaluated.len() != episode_groups.len()
            || episode_groups[0] != self.episode_group
            || episode_groups[1..].contains(&self.episode_group)
            || episode_groups
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != episode_groups.len()
        {
            return Err(TacticQCampaignError::InvalidState(
                "evaluated tactic replay batch has invalid episode lineages",
            ));
        }
        let source_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let mut training_replay = self.training_replay.clone();
        let mut training_replay_routes = self.training_replay_routes.clone();
        let mut training_episode_groups = self.training_episode_groups.clone();
        let mut identities = self.training_identities.clone();
        let mut frontier_archive = self.frontier_archive.clone();
        let mut admitted = 0;
        for (evaluated, episode_group) in evaluated.iter().zip(episode_groups) {
            evaluated.transition.validate()?;
            if evaluated.outcome.selected.decision_index != self.decision_index
                || evaluated.outcome.selected.learner_snapshot_sha256
                    != self.current.snapshot_sha256
                || evaluated.outcome.source_checkpoint_sha256 != self.root_checkpoint_sha256
                || evaluated.transition.before_state_sha256 != self.current.snapshot_sha256
                || evaluated.transition.source_checkpoint_sha256 != source_checkpoint_sha256
                || evaluated.transition.after_state_sha256
                    != evaluated
                        .outcome
                        .next_facts
                        .content_sha256()
                        .map_err(|error| TacticQCampaignError::Features(error.to_string()))?
                || evaluated.transition.next_checkpoint_sha256
                    != route_checkpoint(self.root_checkpoint_sha256, &evaluated.outcome.route_tape)?
                || evaluated.transition.value_sample.action != evaluated.outcome.selected.descriptor
                || evaluated.transition.value_sample.reward.to_bits()
                    != evaluated.reward.training_reward.to_bits()
                || !extends(&self.route_tape, &evaluated.outcome.route_tape)
            {
                return Err(TacticQCampaignError::InvalidState(
                    "evaluated tactic replay is detached from its shared frontier",
                ));
            }
            let identity = evaluated.transition.replay_identity_sha256()?;
            if identities.insert(identity) {
                consider_frontier_transition(
                    &mut frontier_archive,
                    self.root_checkpoint_sha256,
                    &evaluated.transition,
                    &evaluated.outcome.route_tape,
                    *episode_group,
                    training_replay.len(),
                )?;
                training_replay.push(evaluated.transition.clone());
                training_replay_routes.push(evaluated.outcome.route_tape.clone());
                training_episode_groups.push(*episode_group);
                admitted += 1;
            }
        }
        self.training_replay = training_replay;
        self.training_replay_routes = training_replay_routes;
        self.training_episode_groups = training_episode_groups;
        self.training_identities = identities;
        self.frontier_archive = frontier_archive;
        Ok(admitted)
    }

    /// Execute and retain one native tactic boundary, then rebuild the Q model
    /// from every replay row accumulated so far.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_and_refit<W, E, F, A, R>(
        &mut self,
        worker: &mut W,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        paths: &NativeTacticWorkerPaths,
        encode: &F,
        entry_applicable: A,
        reward: R,
    ) -> Result<TacticQCampaignStep, TacticQCampaignError>
    where
        W: PersistentTacticBatchWorker,
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
        R: Fn(&FactSnapshot, &FactSnapshot, &OptionExecution) -> f32,
    {
        let decision = self.decide(catalog, blueprints, encode)?;
        let outcome = execute_selected_tactic(
            worker,
            &decision.selected,
            catalog,
            blueprints,
            &self.current.snapshot,
            &self.route_tape,
            None,
            paths,
        )?;
        self.retain_and_refit(
            decision,
            outcome,
            catalog,
            blueprints,
            registry,
            encode,
            entry_applicable,
            reward,
            true,
        )
    }

    /// Reward-policy variant of [`Self::execute_and_refit`]. It composes
    /// terminal bonus, exact tick cost, first-visit novelty, and optional
    /// potential shaping without granting any of them terminal authority.
    /// Replay is retained on every call; callers may batch the fitted-Q rebuild
    /// after the first model exists. A terminal outcome always forces a refit.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_and_refit_rewarded<W, E, F, A>(
        &mut self,
        worker: &mut W,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        paths: &NativeTacticWorkerPaths,
        encode: &F,
        entry_applicable: A,
        reward_spec: &TacticRewardSpec,
        refit_model: bool,
    ) -> Result<RewardedTacticQCampaignStep, TacticQCampaignError>
    where
        W: PersistentTacticBatchWorker,
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
    {
        let decision = self.decide(catalog, blueprints, encode)?;
        let outcome = execute_selected_tactic(
            worker,
            &decision.selected,
            catalog,
            blueprints,
            &self.current.snapshot,
            &self.route_tape,
            None,
            paths,
        )?;
        self.retain_and_refit_rewarded(
            decision,
            outcome,
            catalog,
            blueprints,
            registry,
            encode,
            entry_applicable,
            reward_spec,
            refit_model,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn retain_and_refit_rewarded<E, F, A>(
        &mut self,
        decision: TacticQDecision,
        outcome: NativeTacticWorkerOutcome,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        encode: &F,
        entry_applicable: A,
        reward_spec: &TacticRewardSpec,
        refit_model: bool,
    ) -> Result<RewardedTacticQCampaignStep, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
    {
        let state = encode(&self.current.snapshot)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let next_state = encode(&outcome.next_facts)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let endpoint = tactic_state_descriptor(&outcome.next_facts, outcome.terminal);
        let reward = reward_spec.evaluate_with_motion(
            self.feature_schema_sha256,
            &state,
            &next_state,
            outcome.execution.duration.realized_ticks,
            outcome.terminal,
            !self.visited_states.contains(&endpoint),
            outcome
                .next_facts
                .recent_option
                .as_ref()
                .and_then(|option| option.trajectory),
        )?;
        let training_reward = reward.training_reward;
        let refit_model = refit_model || outcome.terminal;
        let step = self.retain_and_refit(
            decision,
            outcome,
            catalog,
            blueprints,
            registry,
            encode,
            entry_applicable,
            move |_, _, _| training_reward,
            refit_model,
        )?;
        Ok(RewardedTacticQCampaignStep { step, reward })
    }

    /// Admit an already executed native outcome. This is public so alternate
    /// executors (including observation-loop workers) can share exactly the
    /// same replay and refit path.
    #[allow(clippy::too_many_arguments)]
    pub fn retain_and_refit<E, F, A, R>(
        &mut self,
        decision: TacticQDecision,
        outcome: NativeTacticWorkerOutcome,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        encode: &F,
        entry_applicable: A,
        reward: R,
        refit_model: bool,
    ) -> Result<TacticQCampaignStep, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
        R: Fn(&FactSnapshot, &FactSnapshot, &OptionExecution) -> f32,
    {
        if decision.selected != outcome.selected
            || decision.selected.decision_index != self.decision_index
            || decision.selected.learner_snapshot_sha256 != self.current.snapshot_sha256
            || outcome.source_checkpoint_sha256 != self.root_checkpoint_sha256
            || !extends(&self.route_tape, &outcome.route_tape)
        {
            return Err(TacticQCampaignError::InvalidState(
                "native outcome is detached from the selected campaign boundary",
            ));
        }
        let next = LearnerState::build(
            outcome.next_facts.clone(),
            registry,
            catalog,
            blueprints,
            entry_applicable,
        )?;
        let reward_value = reward(&self.current.snapshot, &next.snapshot, &outcome.execution);
        if !reward_value.is_finite() {
            return Err(TacticQCampaignError::InvalidState(
                "campaign reward is non-finite",
            ));
        }
        let source_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let next_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &outcome.route_tape)?;
        let mut transition = OptionTransitionSample::capture(
            self.feature_schema_sha256,
            source_checkpoint_sha256,
            next_checkpoint_sha256,
            self.current.snapshot.clone(),
            next.snapshot.clone(),
            outcome.execution,
            &outcome.route_tape,
            reward_value,
            outcome.terminal,
            encode,
        )?;
        transition.execution_authority_sha256 = self.execution_authority_sha256;
        transition.validate()?;

        let mut replay = self.replay.clone();
        replay.push(transition.clone());
        let mut replay_routes = self.replay_routes.clone();
        replay_routes.push(outcome.route_tape.clone());
        let mut episode_groups = self.episode_groups.clone();
        episode_groups.push(self.episode_group);
        let mut training_replay = self.training_replay.clone();
        let mut training_replay_routes = self.training_replay_routes.clone();
        let mut training_episode_groups = self.training_episode_groups.clone();
        let mut training_identities = self.training_identities.clone();
        let mut frontier_archive = self.frontier_archive.clone();
        if training_identities.insert(transition.replay_identity_sha256()?) {
            consider_frontier_transition(
                &mut frontier_archive,
                self.root_checkpoint_sha256,
                &transition,
                &outcome.route_tape,
                self.episode_group,
                training_replay.len(),
            )?;
            training_replay.push(transition.clone());
            training_replay_routes.push(outcome.route_tape.clone());
            training_episode_groups.push(self.episode_group);
        }
        let model_update = if refit_model {
            Some(replay_model(
                self.feature_schema_sha256,
                self.objective_sha256,
                &training_replay,
                &training_episode_groups,
                &self.model_config,
            )?)
        } else {
            None
        };

        self.visited_states.insert(tactic_state_descriptor(
            &next.snapshot,
            transition.value_sample.terminal,
        ));
        self.current = next;
        self.route_tape = outcome.route_tape;
        self.replay = replay;
        self.replay_routes = replay_routes;
        self.episode_groups = episode_groups;
        self.training_replay = training_replay;
        self.training_replay_routes = training_replay_routes;
        self.training_episode_groups = training_episode_groups;
        self.training_identities = training_identities;
        self.frontier_archive = frontier_archive;
        if let Some(model) = model_update {
            // Exact-descriptor FQI is a small-data control, not the scalable
            // action representation. Clear it once a dynamic controller
            // universe exceeds its categorical capacity; the shared
            // state-action outcome model continues to consume every row.
            self.model = model.map(Arc::new);
            self.model_revision = self.model_revision.saturating_add(1);
            self.campaign_learner_authority_managed = false;
        }
        self.decision_index =
            self.decision_index
                .checked_add(1)
                .ok_or(TacticQCampaignError::InvalidState(
                    "campaign decision index overflowed",
                ))?;
        Ok(TacticQCampaignStep {
            decision,
            reward: reward_value,
            replay_rows: self.replay.len(),
            transition,
        })
    }
}
