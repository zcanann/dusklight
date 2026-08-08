use super::*;

impl TacticQCampaign {
    pub(crate) fn state_graph(&self) -> Result<&StateGraph, TacticQCampaignError> {
        self.state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign requires bound graph state",
            ))
    }

    pub(crate) fn validated_state_graph(
        &self,
    ) -> Result<ValidatedStateGraph<'_>, TacticQCampaignError> {
        let graph = self.state_graph()?;
        let token =
            self.state_graph_validation
                .as_ref()
                .ok_or(TacticQCampaignError::InvalidState(
                    "campaign graph has no in-process validation authority",
                ))?;
        graph.validated_with_token(token).map_err(Into::into)
    }

    pub(crate) fn validated_graph_mutation<'a>(
        &self,
        graph: &'a StateGraph,
    ) -> Result<ValidatedStateGraph<'a>, TacticQCampaignError> {
        let token =
            self.state_graph_validation
                .as_ref()
                .ok_or(TacticQCampaignError::InvalidState(
                    "campaign graph has no in-process validation authority",
                ))?;
        graph.validated_with_token(token).map_err(Into::into)
    }

    pub fn visited_state_count(&self) -> usize {
        self.state_graph.as_ref().map_or(0, StateGraph::node_count)
    }

    pub fn completed_executable_graph_expansion_count(&self) -> usize {
        self.state_graph
            .as_ref()
            .map_or(0, StateGraph::completed_executable_expansion_count)
    }

    pub fn hindsight_replay(&self) -> &HindsightOptionReplay {
        &self.hindsight
    }

    pub fn frontier_archive(&self) -> Result<&BehaviorArchive, TacticQCampaignError> {
        Ok(&self.frontier_archive)
    }

    /// Count the bounded semantic frontier without cloning every retained
    /// transition and complete route tape.
    ///
    /// Native orchestration records this count after every decision. Building
    /// the executable archive there made a diagnostic integer perform the
    /// same allocation-heavy reconstruction used for an actual branch. The
    /// archive owns one elite per state descriptor, so the count can be
    /// derived directly and capped by the same archive bound.
    pub fn frontier_cell_count(&self) -> usize {
        self.state_graph.as_ref().map_or(0, |graph| {
            graph
                .nodes()
                .filter(|node| {
                    node.id != graph.root() && node.restoration.executable && !node.terminal
                })
                .count()
        })
    }

    pub fn demonstration_frontier_count(&self) -> usize {
        self.demonstration_frontier_state_sha256s().len()
    }

    pub(crate) fn demonstration_frontier_state_sha256s(&self) -> BTreeSet<Digest> {
        let demonstration_endpoints = self
            .training_replay
            .iter()
            .zip(&self.training_episode_groups)
            .filter(|(transition, episode_group)| {
                **episode_group == TACTIC_Q_DEMONSTRATION_EPISODE_GROUP
                    && !transition.value_sample.terminal
            })
            .map(|(transition, _)| {
                (
                    transition.next_checkpoint_sha256,
                    transition.after_state_sha256,
                )
            })
            .collect::<BTreeSet<_>>();
        self.frontier_archive
            .tactic_frontier_entries()
            .filter(|entry| {
                demonstration_endpoints
                    .contains(&(entry.route_checkpoint_sha256, entry.frontier_state_sha256))
            })
            .map(|entry| entry.frontier_state_sha256)
            .collect()
    }

    pub fn graph(&self) -> Result<TacticCampaignGraph, TacticQCampaignError> {
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign graph requires bound graph state",
            ))?;
        graph.validate()?;
        let root = graph.root();
        let root_checkpoint_sha256 = root.route_checkpoint_sha256;
        let root_state_sha256 = root.state_sha256;
        let mut nodes = BTreeMap::<(Digest, Digest), TacticCampaignGraphNode>::new();
        for node in graph.nodes() {
            insert_graph_node(
                &mut nodes,
                TacticCampaignGraphNode {
                    checkpoint_sha256: node.id.route_checkpoint_sha256,
                    state_sha256: node.id.state_sha256,
                    state: node.state.as_ref().clone(),
                    route_tape: graph
                        .route(node.id.route_checkpoint_sha256)
                        .ok_or(TacticQCampaignError::InvalidState(
                            "graph node route is absent",
                        ))?
                        .clone(),
                },
            )?;
        }
        let mut edges = Vec::with_capacity(graph.expansion_count());
        for (transition, route, episode_group) in graph.completed_evidence() {
            edges.push(TacticCampaignGraphEdge {
                episode_group,
                before_state_sha256: transition.before_state_sha256,
                after_state_sha256: transition.after_state_sha256,
                source_checkpoint_sha256: transition.source_checkpoint_sha256,
                next_checkpoint_sha256: transition.next_checkpoint_sha256,
                action: transition.value_sample.action.clone(),
                execution: transition.execution.clone(),
                reward: transition.value_sample.reward,
                terminal: transition.value_sample.terminal,
                route_tape: route.clone(),
            });
        }
        let mut reachable = BTreeSet::from([(root_checkpoint_sha256, root_state_sha256)]);
        loop {
            let before = reachable.len();
            for edge in &edges {
                if reachable.contains(&(edge.source_checkpoint_sha256, edge.before_state_sha256)) {
                    reachable.insert((edge.next_checkpoint_sha256, edge.after_state_sha256));
                }
            }
            if reachable.len() == before {
                break;
            }
        }
        Ok(TacticCampaignGraph {
            schema: "dusklight-tactic-campaign-graph/v1".into(),
            root_checkpoint_sha256,
            root_state_sha256,
            root_connected: reachable.len() == nodes.len(),
            nodes: nodes.into_values().collect(),
            edges,
        })
    }

    pub fn graph_projection(&self) -> Result<TacticCampaignGraphProjection, TacticQCampaignError> {
        let state_graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign graph projection requires bound graph state",
            ))?;
        let graph = self.graph()?;
        let root_checkpoint_sha256 = graph.root_checkpoint_sha256;
        let root_state_sha256 = graph.root_state_sha256;
        let current_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let retained = state_graph
            .nodes()
            .filter(|node| {
                node.id != state_graph.root() && node.restoration.executable && !node.terminal
            })
            .map(|node| node.id.route_checkpoint_sha256)
            .collect::<BTreeSet<_>>();
        let mut nodes = BTreeMap::<(Digest, Digest), TacticCampaignGraphProjectionNode>::new();
        for node in &graph.nodes {
            nodes.insert(
                (node.checkpoint_sha256, node.state_sha256),
                TacticCampaignGraphProjectionNode {
                    checkpoint_sha256: node.checkpoint_sha256,
                    state_sha256: node.state_sha256,
                    stage: node.state.world.stage.clone(),
                    room: node.state.world.room,
                    player_position: node.state.player.position_f32_bits.map(f32::from_bits),
                    terminal: node.state.terminal.reached == Some(true),
                    retained_frontier: retained.contains(&node.checkpoint_sha256),
                    current: node.checkpoint_sha256 == current_checkpoint_sha256
                        && node.state_sha256 == self.current.snapshot_sha256,
                },
            );
        }
        let mut edges = Vec::with_capacity(graph.edges.len());
        for (edge_index, edge) in graph.edges.iter().enumerate() {
            edges.push(TacticCampaignGraphProjectionEdge {
                edge_index: edge_index as u64,
                episode_group: edge.episode_group,
                before_state_sha256: edge.before_state_sha256,
                after_state_sha256: edge.after_state_sha256,
                source_checkpoint_sha256: edge.source_checkpoint_sha256,
                next_checkpoint_sha256: edge.next_checkpoint_sha256,
                option_id: edge.action.option_id.clone(),
                reward: edge.reward,
                duration_ticks: edge.execution.duration.realized_ticks,
                terminal: edge.terminal,
                start_frame: edge.execution.realized_tape_range.start_frame,
                end_frame_exclusive: edge.execution.realized_tape_range.end_frame_exclusive,
            });
        }
        Ok(TacticCampaignGraphProjection {
            schema: "dusklight-tactic-campaign-graph-projection/v1".into(),
            root_checkpoint_sha256,
            root_state_sha256,
            root_connected: graph.root_connected,
            frontier_cells: retained.len(),
            nodes: nodes.into_values().collect(),
            edges,
        })
    }

    pub fn diagnostics(&self) -> Result<TacticCampaignDiagnostics, TacticQCampaignError> {
        let graph = self.graph_projection()?;
        let state_graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign diagnostics require bound graph state",
            ))?;
        let frontiers = state_graph
            .nodes()
            .filter(|node| {
                node.id != state_graph.root() && node.restoration.executable && !node.terminal
            })
            .collect::<Vec<_>>();
        let directly_restorable_native_frontiers = frontiers
            .iter()
            .filter(|node| node.restoration.native_boundary.is_some())
            .count();
        let mut compositions = BTreeMap::<u64, Vec<Digest>>::new();
        let mut selected_actions = BTreeSet::new();
        for (transition, episode_group) in self.replay.iter().zip(&self.episode_groups) {
            let digest = action_digest(&transition.value_sample.action)?;
            selected_actions.insert(digest);
            compositions.entry(*episode_group).or_default().push(digest);
        }
        let mut composition_counts = BTreeMap::<Vec<Digest>, usize>::new();
        for composition in compositions.into_values().filter(|row| !row.is_empty()) {
            *composition_counts.entry(composition).or_default() += 1;
        }
        Ok(TacticCampaignDiagnostics {
            replay_rows: self.replay.len(),
            frontier_cells: frontiers.len(),
            logical_frontier_records: graph.nodes.len(),
            directly_restorable_native_frontiers,
            replay_only_frontiers: frontiers
                .len()
                .saturating_sub(directly_restorable_native_frontiers),
            unique_selected_actions: selected_actions.len(),
            zero_diversity_selection: self.replay.len() >= 2 && selected_actions.len() <= 1,
            repeated_identical_compositions: composition_counts.values().any(|count| *count > 1),
            no_progress_loop: has_no_progress_loop(&self.replay, &self.episode_groups)?,
            frontier_lost_root_connectivity: !graph.root_connected,
        })
    }

    /// Returns one root and one retained frontier branch on every call. The
    /// retained choices rotate from a seeded offset across every eligible
    /// archive cell; root connectivity is sampled explicitly instead of being
    /// left to archive luck.
    pub fn sample_root_and_frontier(
        &self,
        seed: u64,
        round: u64,
        _reference: &[TacticEndpointDescriptor],
        maximum_route_frames: usize,
    ) -> Result<[TacticCampaignBranch; 2], TacticQCampaignError> {
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "frontier sampling requires a bound state graph",
            ))?;
        let root = graph_root_branch(graph)?;
        let choices =
            graph_frontier_entries_validated(self.validated_state_graph()?, maximum_route_frames)?;
        if choices.is_empty() {
            return Err(TacticQCampaignError::InvalidState(
                "state graph has no eligible executable frontier",
            ));
        }
        let index = seeded_frontier_index(seed, round, choices.len());
        let selected = &choices[index];
        let frontier = TacticCampaignBranch {
            kind: TacticBranchKind::RetainedFrontier,
            logical_frontier: LogicalTacticFrontierRecord {
                identity_sha256: selected.route_checkpoint_sha256,
                state_sha256: selected.frontier_state_sha256,
                route_frames: selected.route_tape.frames.len() as u64,
                replayed_prefix_ticks: selected
                    .route_tape
                    .frames
                    .len()
                    .checked_sub(root.route_tape.frames.len())
                    .ok_or(TacticQCampaignError::InvalidState(
                        "frontier route precedes its native root",
                    ))? as u64,
            },
            restorable_native_checkpoint: None,
            acquisition: None,
            state: selected.frontier_state.clone(),
            route_tape: selected.route_tape.clone(),
            descriptor: Some(selected.descriptor.clone()),
        };
        Ok([root, frontier])
    }

    /// Return the authenticated root plus one learned frontier acquisition.
    ///
    /// Before any authenticated terminal supervision exists, least-expanded
    /// farthest-first spatial reachability frontiers drive acquisition; sparse
    /// action cost alone cannot say which branch approaches the objective.
    /// Semantic state and action diversity remain independently retained and
    /// proposed. Once terminal evidence exists, learned total first-hit cost
    /// and predicted future return take over, with coverage and uncertainty as
    /// tie-breakers. The last edge's immediate cost is evidence only, not a
    /// myopic ordering rule.
    pub fn sample_root_and_ranked_frontier<E, AE, F, A>(
        &self,
        seed: u64,
        round: u64,
        reference: &[TacticEndpointDescriptor],
        maximum_route_frames: usize,
        demonstration_curriculum: bool,
        goal_distance_feature: usize,
        encode: &F,
        applicable_actions: &A,
    ) -> Result<[TacticCampaignBranch; 2], TacticQCampaignError>
    where
        E: fmt::Display,
        AE: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&FactSnapshot) -> Result<Vec<OptionActionDescriptor>, AE>,
    {
        let [root, _] =
            self.sample_root_and_frontier(seed, round, reference, maximum_route_frames)?;
        let root_frames = root.route_tape.frames.len();
        let archive = self.frontier_archive()?;
        let exact_terminal_ticks = self.validated_state_graph()?.exact_terminal_returns()?;
        let terminal_value_supported = !exact_terminal_ticks.is_empty();
        let preferred = if !demonstration_curriculum && !terminal_value_supported {
            archive.select_tactic_reachability_frontier_within_route_frames(
                reference,
                archive.tactic_len(),
                maximum_route_frames,
            )
        } else {
            archive.select_tactic_frontier_within_route_frames(
                reference,
                archive.tactic_len(),
                maximum_route_frames,
            )
        };
        let mut graph_choices =
            graph_frontier_entries_validated(self.validated_state_graph()?, maximum_route_frames)?
                .into_iter()
                .map(|entry| {
                    (
                        (entry.route_checkpoint_sha256, entry.frontier_state_sha256),
                        entry,
                    )
                })
                .collect::<BTreeMap<_, _>>();
        let mut choices = preferred
            .into_iter()
            .filter_map(|entry| {
                graph_choices.remove(&(entry.route_checkpoint_sha256, entry.frontier_state_sha256))
            })
            .collect::<Vec<_>>();
        choices.extend(graph_choices.into_values());
        if terminal_value_supported && !demonstration_curriculum {
            choices.sort_by_key(|entry| {
                (
                    exact_terminal_ticks
                        .get(&crate::state_graph::ExactStateId {
                            route_checkpoint_sha256: entry.route_checkpoint_sha256,
                            state_sha256: entry.frontier_state_sha256,
                        })
                        .is_none(),
                    entry.first_seen_generation,
                    entry.route_checkpoint_sha256,
                    entry.frontier_state_sha256,
                )
            });
        }
        if demonstration_curriculum {
            let demonstration_endpoints = self
                .training_replay
                .iter()
                .zip(&self.training_episode_groups)
                .filter(|(_, group)| **group == TACTIC_Q_DEMONSTRATION_EPISODE_GROUP)
                .map(|(transition, _)| {
                    (
                        transition.next_checkpoint_sha256,
                        transition.after_state_sha256,
                    )
                })
                .collect::<BTreeSet<_>>();
            let demonstration_choices = choices
                .iter()
                .filter(|entry| {
                    demonstration_endpoints
                        .contains(&(entry.route_checkpoint_sha256, entry.frontier_state_sha256))
                })
                .cloned()
                .collect::<Vec<_>>();
            if !demonstration_choices.is_empty() {
                choices = demonstration_choices;
            }
        }
        if choices.is_empty() {
            return Err(TacticQCampaignError::InvalidState(
                "frontier archive has no eligible learned acquisition",
            ));
        }
        if !demonstration_curriculum && choices.len() > MAX_RANKED_FRONTIER_CANDIDATES {
            choices = limit_ranked_frontier_candidates(
                seed,
                round,
                root_frames,
                choices,
                &exact_terminal_ticks,
            );
        }
        let tie_offset = seeded_frontier_index(seed, round, choices.len());
        let choice_count = choices.len();
        let goal_reachability_deployment_ready = self
            .goal_reachability_calibration
            .as_ref()
            .is_some_and(|calibration| calibration.deployment_ready);
        let terminal_action_deployment_ready = self
            .terminal_action_calibration
            .as_ref()
            .is_some_and(|calibration| calibration.deployment_ready);
        let generalized_model = if demonstration_curriculum {
            None
        } else {
            match self.value_treatment {
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1 => {
                    self.generalized_model(goal_distance_feature)?
                }
                TacticValueTreatment::GoalRelabeledFittedQKnnV2
                | TacticValueTreatment::GoalRelabeledFrontierDoubleQV3
                | TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4 => self
                    .active_goal_relabel_model(
                        goal_distance_feature,
                        terminal_value_supported && terminal_action_deployment_ready,
                    )?,
                TacticValueTreatment::ContinuousFittedQForestV1 => None,
            }
        };
        let continuous_model = if !demonstration_curriculum
            && self.value_treatment == TacticValueTreatment::ContinuousFittedQForestV1
        {
            self.continuous_model(goal_distance_feature)?
        } else {
            None
        };
        let terminal_action_model = if !demonstration_curriculum
            && terminal_value_supported
            && terminal_action_deployment_ready
            && self.value_treatment.uses_terminal_frontier_action_value()
        {
            self.native_terminal_action_model(goal_distance_feature)?
        } else {
            None
        };
        let mut ranked = choices
            .into_iter()
            .enumerate()
            .map(|(novelty_rank, entry)| {
                let acquisition_estimates = if demonstration_curriculum {
                    (None, None, None, None, None)
                } else {
                    let features = encode(&entry.frontier_state)
                        .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
                    if features.is_empty() || features.iter().any(|value| !value.is_finite()) {
                        return Err(TacticQCampaignError::Features(
                            "frontier encoding is empty or non-finite".into(),
                        ));
                    }
                    let applicable = applicable_actions(&entry.frontier_state)
                        .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
                    if applicable.is_empty() {
                        return Err(TacticQCampaignError::InvalidState(
                            "frontier has no applicable executable actions".into(),
                        ));
                    }
                    if let Some(model) = terminal_action_model.as_ref() {
                        let context = GeneralizedTacticContext::from_facts(&entry.frontier_state)?;
                        let estimates = model.rank(&features, &context, &applicable)?;
                        (
                            estimates.first().map(|value| value.mean_q),
                            None,
                            None,
                            estimates
                                .iter()
                                .map(|value| value.ensemble_variance)
                                .max_by(f64::total_cmp),
                            None,
                        )
                    } else if let Some(model) = generalized_model.as_ref() {
                        let context = GeneralizedTacticContext::from_facts(&entry.frontier_state)?;
                        let goal_reachability_supported =
                            self.value_treatment.uses_goal_relabeling()
                                && !terminal_action_deployment_ready;
                        let estimates = if goal_reachability_supported {
                            model.rank_goal_reachability(&features, &context, &applicable)?
                        } else {
                            model.rank(&features, &context, &applicable)?
                        };
                        (
                            (!goal_reachability_supported)
                                .then(|| {
                                    estimates
                                        .first()
                                        .map(|value| f64::from(value.outcome.reward))
                                })
                                .flatten(),
                            goal_reachability_supported
                                .then(|| {
                                    estimates.first().map(|value| {
                                        f64::from(value.outcome.goal_progress_per_tick)
                                    })
                                })
                                .flatten(),
                            estimates.first().and_then(|value| {
                                (value.outcome.terminal > 0.0
                                    && value.outcome.duration_ticks.is_finite()
                                    && value.outcome.duration_ticks > 0.0)
                                    .then_some(f64::from(value.outcome.duration_ticks))
                            }),
                            None,
                            (!goal_reachability_supported || goal_reachability_deployment_ready)
                                .then(|| {
                                    estimates
                                        .iter()
                                        .map(|value| value.nearest_distance)
                                        .max_by(f32::total_cmp)
                                })
                                .flatten(),
                        )
                    } else if let Some(model) = continuous_model.as_ref() {
                        let context = GeneralizedTacticContext::from_facts(&entry.frontier_state)?;
                        let estimates = model.rank(&features, &context, &applicable)?;
                        (
                            estimates.first().map(|value| value.mean_q),
                            None,
                            None,
                            estimates
                                .iter()
                                .map(|value| value.ensemble_variance)
                                .max_by(f64::total_cmp),
                            None,
                        )
                    } else {
                        let estimates = self
                            .model
                            .as_ref()
                            .map(|model| model.rank_available_options(&features, &applicable))
                            .transpose()?;
                        (
                            estimates
                                .as_ref()
                                .and_then(|values| values.ranked.first())
                                .map(|value| value.mean_q),
                            None,
                            None,
                            estimates.as_ref().and_then(|values| {
                                values
                                    .ranked
                                    .iter()
                                    .map(|value| value.ensemble_variance)
                                    .max_by(f64::total_cmp)
                            }),
                            None,
                        )
                    }
                };
                let (
                    best_mean_q,
                    best_goal_progress_per_tick,
                    predicted_terminal_ticks_to_go,
                    maximum_ensemble_variance,
                    generalized_nearest_distance,
                ) = acquisition_estimates;
                let expansion_count = self
                    .replay
                    .iter()
                    .filter(|transition| {
                        transition.before_state_sha256 == entry.frontier_state_sha256
                            && transition.source_checkpoint_sha256 == entry.route_checkpoint_sha256
                    })
                    .count() as u64;
                let replayed_prefix_ticks = entry
                    .route_tape
                    .frames
                    .len()
                    .checked_sub(root_frames)
                    .ok_or(TacticQCampaignError::InvalidState(
                    "learned frontier route precedes its native root",
                ))? as u64;
                let exact_terminal_ticks_to_go = exact_terminal_ticks
                    .get(&crate::state_graph::ExactStateId {
                        route_checkpoint_sha256: entry.route_checkpoint_sha256,
                        state_sha256: entry.frontier_state_sha256,
                    })
                    .copied();
                let acquisition = TacticFrontierAcquisition {
                    expansion_count,
                    terminal: entry.transition.value_sample.terminal,
                    terminal_value_supported,
                    achieved_goal_value_supported: false,
                    goal_reachability_supported: !terminal_action_deployment_ready
                        && self.value_treatment.uses_goal_relabeling()
                        && generalized_model.is_some()
                        && goal_reachability_deployment_ready,
                    goal_reachability_evidence_available: !terminal_action_deployment_ready
                        && self.value_treatment.uses_goal_relabeling()
                        && generalized_model.is_some(),
                    reward: entry.transition.value_sample.reward,
                    best_mean_q,
                    best_goal_progress_per_tick,
                    predicted_terminal_ticks_to_go,
                    predicted_total_terminal_ticks: predicted_terminal_ticks_to_go
                        .map(|ticks| replayed_prefix_ticks as f64 + ticks),
                    exact_terminal_ticks_to_go,
                    exact_total_terminal_ticks: exact_terminal_ticks_to_go
                        .map(|ticks| replayed_prefix_ticks.saturating_add(ticks)),
                    maximum_ensemble_variance,
                    generalized_nearest_distance,
                    discovery_spatial_novelty: None,
                    novelty_rank: novelty_rank as u64,
                    replayed_prefix_ticks,
                };
                let tie_rank = (novelty_rank + choice_count - tie_offset) % choice_count;
                Ok((entry, acquisition, tie_rank))
            })
            .collect::<Result<Vec<_>, TacticQCampaignError>>()?;
        ranked.sort_by(|left, right| {
            if demonstration_curriculum {
                // This lane is coverage over human-connected states, not an
                // imitation-policy score. Every checkpoint receives a native
                // alternative-action trial before any one is repeated.
                left.1
                    .expansion_count
                    .cmp(&right.1.expansion_count)
                    .then_with(|| left.2.cmp(&right.2))
            } else {
                compare_frontier_acquisition(&left.1, &right.1).then_with(|| left.2.cmp(&right.2))
            }
            .then_with(|| left.1.novelty_rank.cmp(&right.1.novelty_rank))
            .then_with(|| left.0.descriptor.cmp(&right.0.descriptor))
        });
        let (selected, acquisition, _) = ranked
            .into_iter()
            .next()
            .expect("nonempty learned frontier ranking");
        let frontier = TacticCampaignBranch {
            kind: TacticBranchKind::RetainedFrontier,
            logical_frontier: LogicalTacticFrontierRecord {
                identity_sha256: selected.route_checkpoint_sha256,
                state_sha256: selected.frontier_state_sha256,
                route_frames: selected.route_tape.frames.len() as u64,
                replayed_prefix_ticks: acquisition.replayed_prefix_ticks,
            },
            restorable_native_checkpoint: None,
            acquisition: Some(acquisition),
            state: selected.frontier_state.clone(),
            route_tape: selected.route_tape,
            descriptor: Some(selected.descriptor),
        };
        Ok([root, frontier])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore_branch<A>(
        &mut self,
        branch: &TacticCampaignBranch,
        episode_group: u64,
        registry: &FactRegistry,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        entry_applicable: A,
    ) -> Result<(), TacticQCampaignError>
    where
        A: Fn(&TacticAssetDescription) -> bool,
    {
        branch
            .state
            .validate()
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        branch
            .route_tape
            .validate()
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "branch restore requires a bound state graph",
            ))?;
        let root = graph
            .node(graph.root())
            .ok_or(TacticQCampaignError::InvalidState(
                "state graph root is absent",
            ))?;
        let native_root_frames = usize::try_from(root.restoration.route.tape_frames)
            .map_err(|_| TacticQCampaignError::InvalidState("root route frames overflow"))?;
        let expected_replayed_prefix_ticks = branch
            .route_tape
            .frames
            .len()
            .checked_sub(native_root_frames)
            .ok_or(TacticQCampaignError::InvalidState(
                "frontier route precedes its native root",
            ))? as u64;
        let branch_id = crate::state_graph::ExactStateId {
            route_checkpoint_sha256: branch.logical_frontier.identity_sha256,
            state_sha256: branch.logical_frontier.state_sha256,
        };
        let graph_node = graph.node(branch_id);
        let admitted = graph_node.is_some_and(|node| {
            node.restoration.executable
                && graph.route(node.id.route_checkpoint_sha256) == Some(&branch.route_tape)
                && match branch.kind {
                    TacticBranchKind::Root => node.id == graph.root(),
                    TacticBranchKind::RetainedFrontier => node.id != graph.root() && !node.terminal,
                }
        });
        if !admitted
            || self.episode_groups.contains(&episode_group)
            || branch.restorable_native_checkpoint.is_some()
            || branch.logical_frontier.state_sha256
                != branch
                    .state
                    .content_sha256()
                    .map_err(|error| TacticQCampaignError::Features(error.to_string()))?
            || branch.state.tape_frame != branch.route_tape.frames.len() as u64
            || branch.logical_frontier.route_frames != branch.route_tape.frames.len() as u64
            || branch.logical_frontier.replayed_prefix_ticks != expected_replayed_prefix_ticks
            || branch.logical_frontier.identity_sha256
                != route_checkpoint(self.root_checkpoint_sha256, &branch.route_tape)?
        {
            return Err(TacticQCampaignError::InvalidState(
                "frontier branch is detached or reuses an episode group",
            ));
        }
        self.current = LearnerState::build(
            branch.state.clone(),
            registry,
            catalog,
            blueprints,
            entry_applicable,
        )?;
        self.route_tape = branch.route_tape.clone();
        self.episode_group = episode_group;
        Ok(())
    }
}

fn limit_ranked_frontier_candidates(
    seed: u64,
    round: u64,
    root_frames: usize,
    choices: Vec<TacticFrontierEntry>,
    exact_terminal_ticks: &BTreeMap<crate::state_graph::ExactStateId, u64>,
) -> Vec<TacticFrontierEntry> {
    let (mut exact, mut exploratory): (Vec<_>, Vec<_>) = choices.into_iter().partition(|entry| {
        exact_terminal_ticks.contains_key(&crate::state_graph::ExactStateId {
            route_checkpoint_sha256: entry.route_checkpoint_sha256,
            state_sha256: entry.frontier_state_sha256,
        })
    });
    exact.sort_by(|left, right| {
        let total = |entry: &TacticFrontierEntry| {
            (entry.route_tape.frames.len().saturating_sub(root_frames) as u64).saturating_add(
                exact_terminal_ticks[&crate::state_graph::ExactStateId {
                    route_checkpoint_sha256: entry.route_checkpoint_sha256,
                    state_sha256: entry.frontier_state_sha256,
                }],
            )
        };
        total(left)
            .cmp(&total(right))
            .then_with(|| left.first_seen_generation.cmp(&right.first_seen_generation))
            .then_with(|| left.descriptor.cmp(&right.descriptor))
    });
    if exact.len() >= MAX_RANKED_FRONTIER_CANDIDATES {
        exact.truncate(MAX_RANKED_FRONTIER_CANDIDATES);
        return exact;
    }
    let remaining = MAX_RANKED_FRONTIER_CANDIDATES - exact.len();
    let offset = seeded_frontier_index(seed, round, exploratory.len());
    exploratory.rotate_left(offset);
    exploratory.truncate(remaining);
    exact.extend(exploratory);
    exact
}
