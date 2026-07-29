use super::*;

impl TacticQCampaign {
    pub fn visited_state_count(&self) -> usize {
        self.visited_states.len()
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
        self.training_replay
            .iter()
            .zip(&self.training_episode_groups)
            .filter(|(transition, episode_group)| {
                **episode_group != TACTIC_Q_MODEL_ONLY_EPISODE_GROUP
                    && !transition.value_sample.terminal
            })
            .map(|(transition, _)| tactic_state_descriptor(&transition.after, false))
            .collect::<BTreeSet<_>>()
            .len()
            .min(MAX_BEHAVIOR_ARCHIVE_ENTRIES)
    }

    pub fn demonstration_frontier_count(&self) -> usize {
        self.training_replay
            .iter()
            .zip(&self.training_episode_groups)
            .filter(|(transition, episode_group)| {
                **episode_group == TACTIC_Q_DEMONSTRATION_EPISODE_GROUP
                    && !transition.value_sample.terminal
            })
            .map(|(transition, _)| tactic_state_descriptor(&transition.after, false))
            .collect::<BTreeSet<_>>()
            .len()
            .min(MAX_BEHAVIOR_ARCHIVE_ENTRIES)
    }

    pub fn graph(&self) -> Result<TacticCampaignGraph, TacticQCampaignError> {
        let root = self
            .replay
            .first()
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign graph requires replay",
            ))?;
        let root_checkpoint_sha256 = root.source_checkpoint_sha256;
        // One realized PAD checkpoint can legitimately have multiple
        // learner-facing snapshots when distinct tactic labels compile to the
        // same input. `recent_option` records that provenance, so graph nodes
        // are identified by both the restorable checkpoint and fact snapshot.
        let mut nodes = BTreeMap::<(Digest, Digest), TacticCampaignGraphNode>::new();
        let mut edges = Vec::with_capacity(self.replay.len());
        for ((transition, route), episode_group) in self
            .replay
            .iter()
            .zip(&self.replay_routes)
            .zip(&self.episode_groups)
        {
            let start = usize::try_from(transition.execution.realized_tape_range.start_frame)
                .map_err(|_| TacticQCampaignError::InvalidState("graph tape range overflows"))?;
            let before_node = TacticCampaignGraphNode {
                checkpoint_sha256: transition.source_checkpoint_sha256,
                state_sha256: transition.before_state_sha256,
                state: transition.before.clone(),
                route_tape: tape_prefix(route, start),
            };
            let after_node = TacticCampaignGraphNode {
                checkpoint_sha256: transition.next_checkpoint_sha256,
                state_sha256: transition.after_state_sha256,
                state: transition.after.clone(),
                route_tape: route.clone(),
            };
            insert_graph_node(&mut nodes, before_node)?;
            insert_graph_node(&mut nodes, after_node)?;
            edges.push(TacticCampaignGraphEdge {
                episode_group: *episode_group,
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
        let root_state_sha256 = root.before_state_sha256;
        let mut reachable = BTreeSet::from([(root_checkpoint_sha256, root.before_state_sha256)]);
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
        let root = self
            .replay
            .first()
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign graph requires replay",
            ))?;
        let root_checkpoint_sha256 = root.source_checkpoint_sha256;
        let root_state_sha256 = root.before_state_sha256;
        let current_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let retained = self
            .frontier_archive()?
            .tactic_route_checkpoints()
            .collect::<BTreeSet<_>>();
        let mut nodes = BTreeMap::<(Digest, Digest), TacticCampaignGraphProjectionNode>::new();
        let mut edges = Vec::with_capacity(self.replay.len());
        for (edge_index, (transition, episode_group)) in
            self.replay.iter().zip(&self.episode_groups).enumerate()
        {
            for (checkpoint_sha256, state_sha256, state) in [
                (
                    transition.source_checkpoint_sha256,
                    transition.before_state_sha256,
                    &transition.before,
                ),
                (
                    transition.next_checkpoint_sha256,
                    transition.after_state_sha256,
                    &transition.after,
                ),
            ] {
                let node = TacticCampaignGraphProjectionNode {
                    checkpoint_sha256,
                    state_sha256,
                    stage: state.world.stage.clone(),
                    room: state.world.room,
                    player_position: state.player.position_f32_bits.map(f32::from_bits),
                    terminal: state.terminal.reached == Some(true),
                    retained_frontier: retained.contains(&checkpoint_sha256),
                    current: checkpoint_sha256 == current_checkpoint_sha256
                        && state_sha256 == self.current.snapshot_sha256,
                };
                let identity = (checkpoint_sha256, state_sha256);
                if nodes
                    .get(&identity)
                    .is_some_and(|existing| existing != &node)
                {
                    return Err(TacticQCampaignError::InvalidState(
                        "one checkpoint-state identity has conflicting projected graph nodes",
                    ));
                }
                nodes.entry(identity).or_insert(node);
            }
            edges.push(TacticCampaignGraphProjectionEdge {
                edge_index: edge_index as u64,
                episode_group: *episode_group,
                before_state_sha256: transition.before_state_sha256,
                after_state_sha256: transition.after_state_sha256,
                source_checkpoint_sha256: transition.source_checkpoint_sha256,
                next_checkpoint_sha256: transition.next_checkpoint_sha256,
                option_id: transition.value_sample.action.option_id.clone(),
                reward: transition.value_sample.reward,
                duration_ticks: transition.execution.duration.realized_ticks,
                terminal: transition.value_sample.terminal,
                start_frame: transition.execution.realized_tape_range.start_frame,
                end_frame_exclusive: transition.execution.realized_tape_range.end_frame_exclusive,
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
        Ok(TacticCampaignGraphProjection {
            schema: "dusklight-tactic-campaign-graph-projection/v1".into(),
            root_checkpoint_sha256,
            root_state_sha256,
            root_connected: reachable.len() == nodes.len(),
            frontier_cells: retained.len(),
            nodes: nodes.into_values().collect(),
            edges,
        })
    }

    pub fn diagnostics(&self) -> Result<TacticCampaignDiagnostics, TacticQCampaignError> {
        let archive = self.frontier_archive()?;
        let graph = self.graph_projection()?;
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
            frontier_cells: archive.tactic_len(),
            logical_frontier_records: graph.nodes.len(),
            directly_restorable_native_frontiers: 0,
            replay_only_frontiers: archive.tactic_len(),
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
        let first = self
            .replay
            .first()
            .ok_or(TacticQCampaignError::InvalidState(
                "frontier sampling requires replay",
            ))?;
        let first_route = &self.replay_routes[0];
        let root_frames = usize::try_from(first.execution.realized_tape_range.start_frame)
            .map_err(|_| TacticQCampaignError::InvalidState("root tape range overflows"))?;
        let root_route = tape_prefix(first_route, root_frames);
        let root_identity = route_checkpoint(self.root_checkpoint_sha256, &root_route)?;
        let root = TacticCampaignBranch {
            kind: TacticBranchKind::Root,
            logical_frontier: LogicalTacticFrontierRecord {
                identity_sha256: root_identity,
                state_sha256: first.before_state_sha256,
                route_frames: root_route.frames.len() as u64,
                replayed_prefix_ticks: 0,
            },
            restorable_native_checkpoint: None,
            acquisition: None,
            state: first.before.clone(),
            route_tape: root_route,
            descriptor: None,
        };
        let archive = self.frontier_archive()?;
        let choices = archive
            .tactic_frontiers()
            .into_iter()
            .filter(|entry| entry.route_tape.frames.len() <= maximum_route_frames)
            .collect::<Vec<_>>();
        if choices.is_empty() {
            return Err(TacticQCampaignError::InvalidState(
                "frontier archive has no eligible restorable endpoint",
            ));
        }
        let index = seeded_frontier_index(seed, round, choices.len());
        let selected = &choices[index];
        let replayed_prefix_ticks = selected
            .route_tape
            .frames
            .len()
            .checked_sub(root_frames)
            .ok_or(TacticQCampaignError::InvalidState(
                "frontier route precedes its native root",
            ))? as u64;
        let frontier = TacticCampaignBranch {
            kind: TacticBranchKind::RetainedFrontier,
            logical_frontier: LogicalTacticFrontierRecord {
                identity_sha256: selected.route_checkpoint_sha256,
                state_sha256: selected.frontier_state_sha256,
                route_frames: selected.route_tape.frames.len() as u64,
                replayed_prefix_ticks,
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
        let terminal_value_supported = self
            .training_replay
            .iter()
            .any(|transition| transition.value_sample.terminal);
        let exact_terminal_ticks = exact_terminal_ticks_to_go_by_state(&self.training_replay);
        let mut choices = if !demonstration_curriculum && !terminal_value_supported {
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
        if terminal_value_supported && !demonstration_curriculum {
            append_exact_terminal_frontiers(
                &mut choices,
                self.root_checkpoint_sha256,
                &self.training_replay,
                &self.training_replay_routes,
                &self.training_episode_groups,
                &exact_terminal_ticks,
                maximum_route_frames,
            )?;
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
        let generalized_model = if demonstration_curriculum {
            None
        } else {
            match self.value_treatment {
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1 => {
                    self.generalized_model(goal_distance_feature)?
                }
                TacticValueTreatment::GoalRelabeledFittedQKnnV2 => {
                    self.active_goal_relabel_model(goal_distance_feature)?
                }
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
        let mut ranked = choices
            .into_iter()
            .enumerate()
            .map(|(novelty_rank, entry)| {
                let acquisition_estimates = if demonstration_curriculum {
                    (None, None, None, None)
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
                    if let Some(model) = generalized_model.as_ref() {
                        let context = GeneralizedTacticContext::from_facts(&entry.frontier_state)?;
                        let estimates = model.rank(&features, &context, &applicable)?;
                        (
                            estimates
                                .first()
                                .map(|value| f64::from(value.outcome.reward)),
                            estimates.first().and_then(|value| {
                                (value.outcome.terminal > 0.0
                                    && value.outcome.duration_ticks.is_finite()
                                    && value.outcome.duration_ticks > 0.0)
                                    .then_some(f64::from(value.outcome.duration_ticks))
                            }),
                            None,
                            estimates
                                .iter()
                                .map(|value| value.nearest_distance)
                                .max_by(f32::total_cmp),
                        )
                    } else if let Some(model) = continuous_model.as_ref() {
                        let context = GeneralizedTacticContext::from_facts(&entry.frontier_state)?;
                        let estimates = model.rank(&features, &context, &applicable)?;
                        (
                            estimates.first().map(|value| value.mean_q),
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
                    .get(&entry.frontier_state_sha256)
                    .copied();
                let acquisition = TacticFrontierAcquisition {
                    expansion_count,
                    terminal: entry.transition.value_sample.terminal,
                    terminal_value_supported,
                    achieved_goal_value_supported: !terminal_value_supported
                        && self.value_treatment == TacticValueTreatment::GoalRelabeledFittedQKnnV2
                        && generalized_model.is_some(),
                    reward: entry.transition.value_sample.reward,
                    best_mean_q,
                    predicted_terminal_ticks_to_go,
                    predicted_total_terminal_ticks: predicted_terminal_ticks_to_go
                        .map(|ticks| replayed_prefix_ticks as f64 + ticks),
                    exact_terminal_ticks_to_go,
                    exact_total_terminal_ticks: exact_terminal_ticks_to_go
                        .map(|ticks| replayed_prefix_ticks.saturating_add(ticks)),
                    maximum_ensemble_variance,
                    generalized_nearest_distance,
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
        let frontier = self.frontier_archive()?;
        let native_root_frames = self
            .replay
            .first()
            .and_then(|first| usize::try_from(first.execution.realized_tape_range.start_frame).ok())
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign has no native root boundary",
            ))?;
        let expected_replayed_prefix_ticks = branch
            .route_tape
            .frames
            .len()
            .checked_sub(native_root_frames)
            .ok_or(TacticQCampaignError::InvalidState(
                "frontier route precedes its native root",
            ))? as u64;
        let admitted = match branch.kind {
            TacticBranchKind::Root => self.replay.first().is_some_and(|first| {
                first.before_state_sha256 == branch.logical_frontier.state_sha256
                    && first.source_checkpoint_sha256 == branch.logical_frontier.identity_sha256
            }),
            TacticBranchKind::RetainedFrontier => {
                frontier.contains_tactic_frontier(
                    branch.logical_frontier.identity_sha256,
                    branch.logical_frontier.state_sha256,
                ) || self
                    .training_replay
                    .iter()
                    .zip(&self.training_replay_routes)
                    .zip(&self.training_episode_groups)
                    .any(|((transition, route), episode_group)| {
                        if *episode_group == TACTIC_Q_MODEL_ONLY_EPISODE_GROUP {
                            return false;
                        }
                        let exact_endpoint = transition.after_state_sha256
                            == branch.logical_frontier.state_sha256
                            && transition.next_checkpoint_sha256
                                == branch.logical_frontier.identity_sha256
                            && route == &branch.route_tape;
                        exact_endpoint
                            || transition.intermediate_boundaries.iter().any(|boundary| {
                                let Ok(offset) = usize::try_from(boundary.offset_ticks) else {
                                    return false;
                                };
                                let Ok(start) = usize::try_from(
                                    transition.execution.realized_tape_range.start_frame,
                                ) else {
                                    return false;
                                };
                                let Some(end) = start.checked_add(offset) else {
                                    return false;
                                };
                                boundary.state_sha256 == branch.logical_frontier.state_sha256
                                    && route.frames.get(..end)
                                        == Some(branch.route_tape.frames.as_slice())
                                    && branch.logical_frontier.identity_sha256
                                        == route_checkpoint(
                                            self.root_checkpoint_sha256,
                                            &branch.route_tape,
                                        )
                                        .ok()
                                        .unwrap_or(Digest::ZERO)
                            })
                    })
            }
        };
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
    exact_terminal_ticks: &BTreeMap<Digest, u64>,
) -> Vec<TacticFrontierEntry> {
    let (mut exact, mut exploratory): (Vec<_>, Vec<_>) = choices
        .into_iter()
        .partition(|entry| exact_terminal_ticks.contains_key(&entry.frontier_state_sha256));
    exact.sort_by(|left, right| {
        let total = |entry: &TacticFrontierEntry| {
            (entry.route_tape.frames.len().saturating_sub(root_frames) as u64)
                .saturating_add(exact_terminal_ticks[&entry.frontier_state_sha256])
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

#[allow(clippy::too_many_arguments)]
pub(super) fn append_exact_terminal_frontiers(
    choices: &mut Vec<TacticFrontierEntry>,
    root_checkpoint_sha256: Digest,
    transitions: &[OptionTransitionSample],
    routes: &[InputTape],
    episode_groups: &[u64],
    exact_terminal_ticks: &BTreeMap<Digest, u64>,
    maximum_route_frames: usize,
) -> Result<(), TacticQCampaignError> {
    if transitions.len() != routes.len() || transitions.len() != episode_groups.len() {
        return Err(TacticQCampaignError::InvalidState(
            "terminal frontier replay shape is invalid",
        ));
    }
    let mut identities = choices
        .iter()
        .map(|entry| (entry.route_checkpoint_sha256, entry.frontier_state_sha256))
        .collect::<BTreeSet<_>>();
    for (generation, ((transition, route), episode_group)) in transitions
        .iter()
        .zip(routes)
        .zip(episode_groups)
        .enumerate()
    {
        if *episode_group == TACTIC_Q_MODEL_ONLY_EPISODE_GROUP {
            continue;
        }
        if !transition.value_sample.terminal
            && route.frames.len() <= maximum_route_frames
            && exact_terminal_ticks.contains_key(&transition.after_state_sha256)
        {
            let identity = (
                transition.next_checkpoint_sha256,
                transition.after_state_sha256,
            );
            if identities.insert(identity) {
                choices.push(TacticFrontierEntry {
                    descriptor: tactic_endpoint_descriptor(transition)
                        .map_err(|error| TacticQCampaignError::Frontier(error.to_string()))?,
                    root_checkpoint_sha256,
                    route_checkpoint_sha256: transition.next_checkpoint_sha256,
                    frontier_state_sha256: transition.after_state_sha256,
                    frontier_state: transition.after.clone(),
                    transition: transition.clone(),
                    route_tape: route.clone(),
                    first_seen_generation: generation as u64,
                });
            }
        }
        let start = usize::try_from(transition.execution.realized_tape_range.start_frame)
            .map_err(|_| TacticQCampaignError::InvalidState("dense frontier tape overflows"))?;
        for boundary in &transition.intermediate_boundaries {
            if !exact_terminal_ticks.contains_key(&boundary.state_sha256) {
                continue;
            }
            let end = start
                .checked_add(usize::try_from(boundary.offset_ticks).map_err(|_| {
                    TacticQCampaignError::InvalidState("dense frontier offset overflows")
                })?)
                .ok_or(TacticQCampaignError::InvalidState(
                    "dense frontier route overflows",
                ))?;
            if end > route.frames.len() || end > maximum_route_frames {
                continue;
            }
            let route_tape = tape_prefix(route, end);
            let route_checkpoint_sha256 = route_checkpoint(root_checkpoint_sha256, &route_tape)?;
            if !identities.insert((route_checkpoint_sha256, boundary.state_sha256)) {
                continue;
            }
            choices.push(TacticFrontierEntry {
                descriptor: tactic_endpoint_descriptor_for_state(
                    &boundary.state,
                    false,
                    &transition.value_sample.action,
                )
                .map_err(|error| TacticQCampaignError::Frontier(error.to_string()))?,
                root_checkpoint_sha256,
                route_checkpoint_sha256,
                frontier_state_sha256: boundary.state_sha256,
                frontier_state: boundary.state.clone(),
                transition: transition.clone(),
                route_tape,
                first_seen_generation: generation as u64,
            });
        }
    }
    Ok(())
}

fn exact_terminal_ticks_to_go_by_state(
    transitions: &[OptionTransitionSample],
) -> BTreeMap<Digest, u64> {
    let mut ticks = transitions
        .iter()
        .filter(|transition| transition.value_sample.terminal)
        .map(|transition| (transition.after_state_sha256, 0_u64))
        .collect::<BTreeMap<_, _>>();
    loop {
        let mut changed = false;
        for transition in transitions {
            let Some(after_ticks) = ticks.get(&transition.after_state_sha256).copied() else {
                continue;
            };
            let duration = u64::from(transition.value_sample.duration_ticks);
            for boundary in &transition.intermediate_boundaries {
                let candidate = after_ticks
                    .saturating_add(duration.saturating_sub(u64::from(boundary.offset_ticks)));
                if ticks
                    .get(&boundary.state_sha256)
                    .is_none_or(|current| candidate < *current)
                {
                    ticks.insert(boundary.state_sha256, candidate);
                    changed = true;
                }
            }
            let candidate = after_ticks.saturating_add(duration);
            if ticks
                .get(&transition.before_state_sha256)
                .is_none_or(|before_ticks| candidate < *before_ticks)
            {
                ticks.insert(transition.before_state_sha256, candidate);
                changed = true;
            }
        }
        if !changed {
            return ticks;
        }
    }
}
