use super::*;

pub(super) fn applicable_untried_descriptors(
    choices: &[LearnerActionMaskEntry],
    tried_here: &BTreeSet<&str>,
) -> Vec<OptionActionDescriptor> {
    choices
        .iter()
        .filter(|choice| {
            choice.applicable && !tried_here.contains(choice.descriptor.option_id.as_str())
        })
        .map(|choice| choice.descriptor.clone())
        .collect()
}

pub(super) fn seeded_frontier_index(seed: u64, round: u64, choice_count: usize) -> usize {
    debug_assert!(choice_count > 0);
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-tactic-frontier-sample/v2");
    hasher.update(seed.to_le_bytes());
    let digest = hasher.finalize();
    let count = choice_count as u64;
    let offset = u64::from_le_bytes(digest[..8].try_into().unwrap()) % count;
    ((offset + round % count) % count) as usize
}

pub(super) fn option_f64(value: Option<f64>) -> f64 {
    value.unwrap_or(f64::NEG_INFINITY)
}

fn has_action_conditioned_frontier_value(acquisition: &TacticFrontierAcquisition) -> bool {
    acquisition.best_mean_q.is_some() && acquisition.maximum_ensemble_variance.is_some()
}

pub(super) fn compare_frontier_acquisition(
    left: &TacticFrontierAcquisition,
    right: &TacticFrontierAcquisition,
) -> std::cmp::Ordering {
    let terminal = right.terminal.cmp(&left.terminal);
    if terminal != std::cmp::Ordering::Equal {
        return terminal;
    }
    if !left.terminal_value_supported && !right.terminal_value_supported {
        if left.goal_reachability_supported != right.goal_reachability_supported {
            return right
                .goal_reachability_supported
                .cmp(&left.goal_reachability_supported);
        }
        if left.goal_reachability_supported && right.goal_reachability_supported {
            // This comparator is the learned-exploitation partition. Broad
            // graph coverage has its own sealed acquisition ranks. Putting
            // expansion count first here makes exploitation impossible when
            // every branch produces more fresh frontier states than it can
            // consume: an expanded promising state can never outrank the
            // perpetually growing zero-expansion set.
            return option_f64(right.best_goal_progress_per_tick)
                .total_cmp(&option_f64(left.best_goal_progress_per_tick))
                .then_with(|| left.expansion_count.cmp(&right.expansion_count))
                .then_with(|| left.novelty_rank.cmp(&right.novelty_rank))
                .then_with(|| {
                    option_f64(right.generalized_nearest_distance.map(f64::from)).total_cmp(
                        &option_f64(left.generalized_nearest_distance.map(f64::from)),
                    )
                });
        }
        if left.achieved_goal_value_supported != right.achieved_goal_value_supported {
            return right
                .achieved_goal_value_supported
                .cmp(&left.achieved_goal_value_supported);
        }
        if left.achieved_goal_value_supported && right.achieved_goal_value_supported {
            // Preserve one expansion of every fresh state cell, then let the
            // learned achieved-goal return decide which equally fresh
            // continuation is most promising. This is learned first-hit cost,
            // not sparse action cost or native terminal evidence.
            return left
                .expansion_count
                .cmp(&right.expansion_count)
                .then_with(|| {
                    option_f64(right.best_mean_q).total_cmp(&option_f64(left.best_mean_q))
                })
                .then_with(|| left.novelty_rank.cmp(&right.novelty_rank))
                .then_with(|| {
                    option_f64(right.generalized_nearest_distance.map(f64::from)).total_cmp(
                        &option_f64(left.generalized_nearest_distance.map(f64::from)),
                    )
                });
        }
        // With no authenticated terminal sample, sparse return is only the
        // negative duration already spent. Treating it as goal evidence traps
        // acquisition near cheap, shallow actions. Cover the farthest-first
        // semantic archive until real terminal supervision exists.
        return left
            .expansion_count
            .cmp(&right.expansion_count)
            .then_with(|| left.novelty_rank.cmp(&right.novelty_rank))
            .then_with(|| {
                option_f64(right.generalized_nearest_distance.map(f64::from)).total_cmp(
                    &option_f64(left.generalized_nearest_distance.map(f64::from)),
                )
            })
            .then_with(|| {
                option_f64(right.maximum_ensemble_variance)
                    .total_cmp(&option_f64(left.maximum_ensemble_variance))
            });
    }
    if left.terminal_value_supported != right.terminal_value_supported {
        return right
            .terminal_value_supported
            .cmp(&left.terminal_value_supported);
    }
    match (
        left.exact_total_terminal_ticks,
        right.exact_total_terminal_ticks,
    ) {
        (Some(left_ticks), Some(right_ticks)) => {
            let left_action_value = has_action_conditioned_frontier_value(left);
            let right_action_value = has_action_conditioned_frontier_value(right);
            return left_ticks
                .cmp(&right_ticks)
                .then_with(|| right_action_value.cmp(&left_action_value))
                .then_with(|| {
                    if left_action_value && right_action_value {
                        option_f64(right.best_mean_q).total_cmp(&option_f64(left.best_mean_q))
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| {
                    if left_action_value && right_action_value {
                        left.maximum_ensemble_variance
                            .unwrap_or(f64::INFINITY)
                            .total_cmp(&right.maximum_ensemble_variance.unwrap_or(f64::INFINITY))
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| left.expansion_count.cmp(&right.expansion_count));
        }
        (Some(_), None) => return std::cmp::Ordering::Less,
        (None, Some(_)) => return std::cmp::Ordering::Greater,
        (None, None) => {}
    }
    match (
        left.predicted_total_terminal_ticks,
        right.predicted_total_terminal_ticks,
    ) {
        (Some(left_ticks), Some(right_ticks)) => {
            // Q-to-go alone systematically favors the latest checkpoint on
            // one successful route. Compare the learned first-hit cost from
            // the authenticated root, then spread trials across equal-cost
            // curriculum frontiers.
            left_ticks
                .total_cmp(&right_ticks)
                .then_with(|| left.expansion_count.cmp(&right.expansion_count))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => option_f64(right.best_mean_q)
            .total_cmp(&option_f64(left.best_mean_q))
            .then_with(|| left.expansion_count.cmp(&right.expansion_count))
            .then_with(|| {
                option_f64(right.generalized_nearest_distance.map(f64::from)).total_cmp(
                    &option_f64(left.generalized_nearest_distance.map(f64::from)),
                )
            })
            .then_with(|| {
                option_f64(right.maximum_ensemble_variance)
                    .total_cmp(&option_f64(left.maximum_ensemble_variance))
            })
            .then_with(|| left.replayed_prefix_ticks.cmp(&right.replayed_prefix_ticks)),
    }
}

pub(super) fn ensure_blueprint_proposal(
    ranking: &LiveTacticRanking,
    maximum_proposals: usize,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticQCampaignError> {
    if maximum_proposals <= 1
        || proposals
            .iter()
            .any(|proposal| proposal.descriptor.option_id.starts_with("blueprint/"))
    {
        return Ok(());
    }
    let Some(composition) = ranking
        .choices
        .iter()
        .find(|choice| choice.applicable && choice.kind == ConcreteTacticChoiceKind::Blueprint)
        .map(|choice| choice.descriptor.clone())
    else {
        return Ok(());
    };
    let mut selected = proposals
        .last()
        .cloned()
        .ok_or(TacticQCampaignError::InvalidState(
            "tactic proposal batch is empty",
        ))?;
    selected.descriptor = composition;
    selected.reason = TacticSelectionReason::BatchDiversity;
    if proposals.len() < maximum_proposals {
        proposals.push(selected);
    } else if let Some(last) = proposals.last_mut() {
        *last = selected;
    }
    Ok(())
}

pub(super) fn action_digest(
    action: &OptionActionDescriptor,
) -> Result<Digest, TacticQCampaignError> {
    let bytes = serde_json::to_vec(action)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

pub(super) fn insert_graph_node(
    nodes: &mut BTreeMap<(Digest, Digest), TacticCampaignGraphNode>,
    node: TacticCampaignGraphNode,
) -> Result<(), TacticQCampaignError> {
    let identity = (node.checkpoint_sha256, node.state_sha256);
    if let Some(existing) = nodes.get(&identity) {
        if existing != &node {
            return Err(TacticQCampaignError::InvalidState(
                "one checkpoint-state identity has conflicting campaign graph nodes",
            ));
        }
    } else {
        nodes.insert(identity, node);
    }
    Ok(())
}

pub(crate) fn has_no_progress_loop(
    replay: &[OptionTransitionSample],
    episode_groups: &[u64],
) -> Result<bool, TacticQCampaignError> {
    let mut visited = BTreeMap::<u64, BTreeSet<Digest>>::new();
    for (transition, episode_group) in replay.iter().zip(episode_groups) {
        let states = visited.entry(*episode_group).or_default();
        states.insert(semantic_state_digest(&transition.before)?);
        if !transition.value_sample.terminal
            && !states.insert(semantic_state_digest(&transition.after)?)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn semantic_state_digest(
    snapshot: &FactSnapshot,
) -> Result<Digest, TacticQCampaignError> {
    // Clocks, replay history, and the previously emitted pad identify when and
    // how a state was observed, not whether gameplay made semantic progress.
    // Everything else remains visible so actor, flag, event, kinematic, and
    // derived-condition progress all break a cycle.
    let mut normalized = snapshot.clone();
    normalized.schema = FACT_SNAPSHOT_SCHEMA_V2.into();
    normalized.boundary_index = 0;
    normalized.simulation_tick = 0;
    normalized.tape_frame = 0;
    normalized.state_identity = [0; 16];
    normalized.recent_history.clear();
    normalized.recent_option = None;
    normalized.player.previous_pad = None;
    let bytes = serde_json::to_vec(&normalized)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}
