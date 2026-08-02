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
        Ok(TacticQProposalBatch {
            ranking,
            proposals,
            goal_reachability_estimates: Vec::new(),
            goal_reachability_calibration: None,
        })
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
        let native_terminal_supported = self.native_terminal_supported();
        // Partition zero is the terminal-support acquisition only after the
        // graph has terminal evidence. Before that, every lane belongs to the
        // discovery regime and must retain its configured epsilon.
        let terminal_support_acquisition = acquisition_partition == 0 || force_exploration;
        let exploration = proposal_exploration_config(
            self.exploration,
            force_exploration,
            native_terminal_supported,
            terminal_support_acquisition,
        );
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
        let mut goal_reachability_estimates = Vec::new();
        let mut goal_reachability_calibration = None;
        if policy.uses_learned_selector() {
            let context = GeneralizedTacticContext::from_facts(&self.current.snapshot)?;
            let applicable_descriptors = ranking
                .choices
                .iter()
                .filter(|choice| choice.applicable)
                .map(|choice| choice.descriptor.clone())
                .collect::<Vec<_>>();
            // Partition zero is the dedicated terminal-support policy lane
            // once a native terminal exists. Other partitions must continue
            // consulting the achieved-goal critic: open exploratory branches
            // are otherwise fitted and immediately ignored merely because a
            // demonstration or earlier route supplied terminal evidence.
            let goal_reachability_acquisition = self.value_treatment.uses_goal_relabeling()
                && (!native_terminal_supported || !terminal_support_acquisition);
            let goal_reachability_deployment_ready = goal_reachability_acquisition
                && self
                    .goal_reachability_calibration
                    .as_ref()
                    .is_some_and(|calibration| calibration.deployment_ready);
            if goal_reachability_acquisition {
                goal_reachability_calibration = self.goal_reachability_calibration.clone();
            }
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
                    TacticValueTreatment::GoalRelabeledFittedQKnnV2
                    | TacticValueTreatment::GoalRelabeledFrontierDoubleQV3
                    | TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4 => {
                        if native_terminal_supported && terminal_support_acquisition {
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
                            self.active_goal_relabel_model(
                                goal_distance_feature,
                                terminal_support_acquisition,
                            )?
                            .map(|model| {
                                model.rank_goal_reachability(
                                    &features,
                                    &context,
                                    &applicable_descriptors,
                                )
                            })
                            .transpose()?
                            .map(|estimates| {
                                goal_reachability_estimates = estimates
                                    .iter()
                                    .map(|estimate| TacticQGoalReachabilityEstimate {
                                        descriptor: estimate.descriptor.clone(),
                                        predicted_goal_progress_per_tick: estimate
                                            .outcome
                                            .goal_progress_per_tick,
                                        nearest_distance: estimate.nearest_distance,
                                    })
                                    .collect();
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
                    if goal_reachability_acquisition {
                        if goal_reachability_deployment_ready {
                            ensure_goal_reachability_acquisition(
                                &ranked_applicable,
                                acquisition_partition,
                                maximum_proposals,
                                &mut proposals,
                            )?;
                        }
                    } else {
                        ensure_generalized_value_acquisition(
                            &ranked_applicable,
                            acquisition_partition,
                            maximum_proposals,
                            &mut proposals,
                        )?;
                    }
                    if terminal_support_acquisition
                        && (self.value_treatment
                            == TacticValueTreatment::LocalGeneralizedFittedQKnnV1
                            || (self.value_treatment.uses_goal_relabeling()
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
            if goal_reachability_deployment_ready {
                retain_goal_reachability_acquisition(&mut proposals)?;
            } else {
                retain_generalized_value_acquisition(&mut proposals)?;
            }
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
        Ok(TacticQProposalBatch {
            ranking,
            proposals,
            goal_reachability_estimates,
            goal_reachability_calibration,
        })
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
        transition.intermediate_boundaries = outcome.intermediate_boundaries.clone();
        transition.execution_authority_sha256 = self.execution_authority_sha256;
        let transition = AuthenticatedOptionTransition::new(transition)?;
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
        self.admit_evaluated_replay_with_leases(evaluated, episode_groups, None)
    }

    /// Complete graph-leased native work atomically. Every outcome must name
    /// the descriptor and lease selected before worker execution.
    pub fn admit_leased_evaluated_replay(
        &mut self,
        evaluated: &[EvaluatedRewardedTacticOutcome],
        episode_groups: &[u64],
        leases: &[TacticExpansionLease],
    ) -> Result<usize, TacticQCampaignError> {
        self.admit_evaluated_replay_with_leases(evaluated, episode_groups, Some(leases))
    }

    fn admit_evaluated_replay_with_leases(
        &mut self,
        evaluated: &[EvaluatedRewardedTacticOutcome],
        episode_groups: &[u64],
        leases: Option<&[TacticExpansionLease]>,
    ) -> Result<usize, TacticQCampaignError> {
        if evaluated.is_empty()
            || evaluated.len() != episode_groups.len()
            || leases.is_some_and(|leases| leases.len() != evaluated.len())
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
        let mut state_graph =
            self.state_graph
                .clone()
                .ok_or(TacticQCampaignError::InvalidState(
                    "evaluated replay requires a bound state graph",
                ))?;
        let mut frontier_archive = self.frontier_archive.clone();
        let mut admitted = 0;
        let mut projection_keys = Vec::with_capacity(evaluated.len());
        for (index, (evaluated, episode_group)) in evaluated.iter().zip(episode_groups).enumerate()
        {
            if evaluated.outcome.selected.decision_index != self.decision_index
                || evaluated.outcome.selected.learner_snapshot_sha256
                    != self.current.snapshot_sha256
                || evaluated.outcome.source_checkpoint_sha256 != self.root_checkpoint_sha256
                || evaluated.transition.before_state_sha256 != self.current.snapshot_sha256
                || evaluated.transition.before != self.current.snapshot
                || evaluated.transition.after != evaluated.outcome.next_facts
                || evaluated.transition.value_sample.action != evaluated.outcome.selected.descriptor
                || evaluated.transition.value_sample.reward.to_bits()
                    != evaluated.reward.training_reward.to_bits()
                || !extends(&self.route_tape, &evaluated.outcome.route_tape)
            {
                return Err(TacticQCampaignError::InvalidState(
                    "evaluated tactic replay is detached from its shared frontier",
                ));
            }
            let authority = if *episode_group == TACTIC_Q_MODEL_ONLY_EPISODE_GROUP {
                crate::state_graph::ExpansionEvidenceAuthority::LearnerEvidenceOnly
            } else {
                crate::state_graph::ExpansionEvidenceAuthority::Executable
            };
            let admission = if let Some(leases) = leases {
                let lease = &leases[index];
                if lease.descriptor != evaluated.transition.value_sample.action {
                    return Err(TacticQCampaignError::InvalidState(
                        "evaluated tactic replay is detached from its graph lease",
                    ));
                }
                let admission = state_graph.admit_leased_authenticated_completed_expansion(
                    evaluated.transition.clone(),
                    evaluated.outcome.route_tape.clone(),
                    *episode_group,
                    authority,
                    lease.lease_sha256,
                )?;
                if admission.expansion_sha256 != lease.expansion_sha256 {
                    return Err(TacticQCampaignError::InvalidState(
                        "completed graph lease names a different expansion",
                    ));
                }
                admission
            } else {
                state_graph.admit_authenticated_completed_expansion(
                    evaluated.transition.clone(),
                    evaluated.outcome.route_tape.clone(),
                    *episode_group,
                    authority,
                )?
            };
            if !admission.duplicate || admission.authority_promoted {
                consider_frontier_transition(
                    &mut frontier_archive,
                    self.root_checkpoint_sha256,
                    &evaluated.transition,
                    &evaluated.outcome.route_tape,
                    *episode_group,
                    state_graph.expansion_count().saturating_sub(1),
                )?;
            }
            if !admission.duplicate {
                admitted += 1;
            }
            projection_keys.push((admission.expansion_sha256, admission.evidence_sha256));
        }
        let projection_rows = graph_training_projection_rows(&state_graph, projection_keys)?;
        validate_graph_training_projection_merge(
            &self.training_projection_keys,
            &self.training_replay,
            &self.training_replay_routes,
            &self.training_episode_groups,
            &projection_rows,
        )?;
        self.state_graph = Some(state_graph);
        merge_graph_training_projection(
            &mut self.training_projection_keys,
            &mut self.training_replay,
            &mut self.training_replay_routes,
            &mut self.training_episode_groups,
            projection_rows,
        );
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
        self.retain_rewarded_with_update_mode(
            decision,
            outcome,
            catalog,
            blueprints,
            registry,
            encode,
            entry_applicable,
            reward_spec,
            refit_model,
            true,
        )
    }

    /// Retain authenticated experience and graph evidence without changing the
    /// policy model, including after a terminal outcome. This is the explicit
    /// frozen-policy control path; ordinary learning must use
    /// [`Self::retain_and_refit_rewarded`].
    #[allow(clippy::too_many_arguments)]
    pub fn retain_rewarded_without_policy_update<E, F, A>(
        &mut self,
        decision: TacticQDecision,
        outcome: NativeTacticWorkerOutcome,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        encode: &F,
        entry_applicable: A,
        reward_spec: &TacticRewardSpec,
    ) -> Result<RewardedTacticQCampaignStep, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
    {
        self.retain_rewarded_with_update_mode(
            decision,
            outcome,
            catalog,
            blueprints,
            registry,
            encode,
            entry_applicable,
            reward_spec,
            false,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn retain_rewarded_with_update_mode<E, F, A>(
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
        refit_terminal: bool,
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
        let refit_model = refit_model || (refit_terminal && outcome.terminal);
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
        transition.intermediate_boundaries = outcome.intermediate_boundaries;
        transition.execution_authority_sha256 = self.execution_authority_sha256;
        transition.validate()?;

        let mut replay = self.replay.clone();
        replay.push(transition.clone());
        let mut replay_routes = self.replay_routes.clone();
        replay_routes.push(outcome.route_tape.clone());
        let mut episode_groups = self.episode_groups.clone();
        episode_groups.push(self.episode_group);
        let evidence_sha256 = transition.replay_identity_sha256()?;
        let expansion_sha256 = crate::state_graph::action_expansion_identity(
            crate::state_graph::ExactStateId {
                route_checkpoint_sha256: transition.source_checkpoint_sha256,
                state_sha256: transition.before_state_sha256,
            },
            &transition.value_sample.action,
        )?;
        let expected_authority = if self.episode_group == TACTIC_Q_MODEL_ONLY_EPISODE_GROUP {
            crate::state_graph::ExpansionEvidenceAuthority::LearnerEvidenceOnly
        } else {
            crate::state_graph::ExpansionEvidenceAuthority::Executable
        };
        let transition_already_admitted = self
            .state_graph
            .as_ref()
            .and_then(|graph| graph.expansion(expansion_sha256))
            .and_then(|expansion| match &expansion.status {
                crate::state_graph::ActionExpansionStatus::Completed { evidence, .. } => {
                    evidence.get(&evidence_sha256)
                }
                _ => None,
            })
            .is_some_and(|evidence| {
                evidence.episode_group == self.episode_group
                    && evidence.authority == expected_authority
                    && evidence.transition.as_ref() == &transition
            });
        let projection_already_contains_transition = self
            .training_replay
            .iter()
            .zip(&self.training_replay_routes)
            .zip(&self.training_episode_groups)
            .any(|((row, route), episode_group)| {
                row == &transition
                    && route == &outcome.route_tape
                    && *episode_group == self.episode_group
            });
        if transition_already_admitted && !projection_already_contains_transition {
            return Err(TacticQCampaignError::InvalidState(
                "pre-admitted native transition is absent from the learner projection",
            ));
        }
        let (state_graph_update, frontier_archive_update, projection_update) =
            if transition_already_admitted {
                (None, None, None)
            } else {
                let mut state_graph =
                    self.state_graph
                        .clone()
                        .ok_or(TacticQCampaignError::InvalidState(
                            "native execution requires a bound state graph",
                        ))?;
                let mut frontier_archive = self.frontier_archive.clone();
                let graph_admission = state_graph.admit_completed_expansion(
                    transition.clone(),
                    outcome.route_tape.clone(),
                    self.episode_group,
                    expected_authority,
                )?;
                if !graph_admission.duplicate || graph_admission.authority_promoted {
                    consider_frontier_transition(
                        &mut frontier_archive,
                        self.root_checkpoint_sha256,
                        &transition,
                        &outcome.route_tape,
                        self.episode_group,
                        state_graph.expansion_count().saturating_sub(1),
                    )?;
                }
                let projection = graph_training_projection(&state_graph)?;
                (Some(state_graph), Some(frontier_archive), Some(projection))
            };
        let model_update = if refit_model {
            let (transitions, episode_groups) = projection_update
                .as_ref()
                .map(|projection| {
                    (
                        projection.transitions.as_slice(),
                        projection.episode_groups.as_slice(),
                    )
                })
                .unwrap_or((
                    self.training_replay.as_slice(),
                    self.training_episode_groups.as_slice(),
                ));
            Some(replay_model(
                self.feature_schema_sha256,
                self.objective_sha256,
                transitions,
                episode_groups,
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
        if let Some(state_graph) = state_graph_update {
            self.state_graph = Some(state_graph);
        }
        if let Some(projection) = projection_update {
            self.training_projection_keys = projection.keys;
            self.training_replay = projection.transitions;
            self.training_replay_routes = projection.routes;
            self.training_episode_groups = projection.episode_groups;
        }
        if let Some(frontier_archive) = frontier_archive_update {
            self.frontier_archive = frontier_archive;
        }
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

fn proposal_exploration_config(
    configured: TacticExplorationConfig,
    force_exploration: bool,
    native_terminal_supported: bool,
    terminal_support_acquisition: bool,
) -> TacticExplorationConfig {
    let epsilon_per_million = if force_exploration {
        1_000_000
    } else if native_terminal_supported && terminal_support_acquisition {
        0
    } else {
        configured.epsilon_per_million
    };
    TacticExplorationConfig {
        seed: configured.seed,
        epsilon_per_million,
    }
}

#[cfg(test)]
mod proposal_exploration_tests {
    use super::*;

    #[test]
    fn discovery_epsilon_applies_to_support_and_ranked_acquisitions() {
        let configured = TacticExplorationConfig {
            seed: 17,
            epsilon_per_million: 350_000,
        };

        assert_eq!(
            proposal_exploration_config(configured, false, false, true),
            configured
        );
        assert_eq!(
            proposal_exploration_config(configured, false, false, false),
            configured
        );
    }

    #[test]
    fn authenticated_terminal_suppresses_only_the_support_acquisition() {
        let configured = TacticExplorationConfig {
            seed: 17,
            epsilon_per_million: 350_000,
        };

        assert_eq!(
            proposal_exploration_config(configured, false, true, true).epsilon_per_million,
            0
        );
        assert_eq!(
            proposal_exploration_config(configured, false, true, false),
            configured
        );
        assert_eq!(
            proposal_exploration_config(configured, true, true, true).epsilon_per_million,
            1_000_000
        );
    }
}
