use super::*;

pub(crate) fn native_tactic_applicable_action_surface_identity(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    shard: &NativeEpisodeShard,
) -> Result<(Digest, u64, u64), NativeTacticRouteRunError> {
    let initial_observation = shard
        .episodes
        .first()
        .and_then(|episode| episode.steps.first())
        .map(|step| &step.pre_input)
        .ok_or_else(|| route_message("native tactic action-surface audit has no source state"))?;
    let initial_facts =
        FactSnapshot::from_native_learning(initial_observation, &[], None, Vec::new())
            .map_err(route_error)?;
    let GoalConditionedTacticContext { encoder, .. } =
        atomic_goal_conditioned_tactic_context(root, optimization, execution, &initial_facts)?;
    let action_schema_sha256 = parameterized_policy_action_schema_sha256(None);
    let maximum_ticks = goal_tactic_maximum_ticks(optimization.budgets.exploration_horizon_ticks)?;
    let registry = FactRegistry::canonical();
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-native-tactic-applicable-action-surface/v1");
    hasher.update(action_schema_sha256.0);
    hasher.update(encoder.schema_sha256.0);
    hasher.update(maximum_ticks.to_le_bytes());
    hasher.update((shard.episodes.len() as u64).to_le_bytes());
    let mut boundary_count = 0_u64;
    let mut descriptor_count = 0_u64;
    for episode in &shard.episodes {
        hasher.update((episode.id.len() as u64).to_le_bytes());
        hasher.update(episode.id.as_bytes());
        hasher.update((episode.steps.len() as u64).to_le_bytes());
        for (step_index, step) in episode.steps.iter().enumerate() {
            let prior = episode.steps[..step_index]
                .iter()
                .rev()
                .take(dusklight_learning::fact_snapshot::MAX_FACT_HISTORY)
                .map(|prior| prior.pre_input.clone())
                .collect::<Vec<_>>();
            let prior = prior.into_iter().rev().collect::<Vec<_>>();
            let state =
                FactSnapshot::from_native_learning(&step.pre_input, &prior, None, Vec::new())
                    .map_err(route_error)?;
            // The parity probe has no campaign history or promoted actions. Its
            // fixed seed and boundary-index decision identity make the complete
            // primitive action query reproducible in every subsystem condition.
            let proposals = parameterized_catalog_for_state(
                0,
                step.pre_input.boundary_index,
                &state,
                &encoder,
                maximum_ticks,
                None,
                action_schema_sha256,
            )?;
            let learner = LearnerState::build(
                state,
                &registry,
                &proposals.catalog,
                &proposals.blueprints,
                |_| true,
            )
            .map_err(route_error)?;
            let mut descriptors = learner
                .action_mask
                .into_iter()
                .filter(|choice| choice.applicable)
                .map(|choice| choice.descriptor.content_sha256().map_err(route_error))
                .collect::<Result<Vec<_>, _>>()?;
            if descriptors.is_empty() {
                return Err(route_message(
                    "native tactic action-surface audit has no applicable action",
                ));
            }
            descriptors.sort_by(|left, right| left.0.cmp(&right.0));
            boundary_count = boundary_count
                .checked_add(1)
                .ok_or_else(|| route_message("native tactic action-surface count overflows"))?;
            descriptor_count = descriptor_count
                .checked_add(u64::try_from(descriptors.len()).map_err(route_error)?)
                .ok_or_else(|| route_message("native tactic descriptor count overflows"))?;
            hasher.update(step.pre_input.boundary_index.to_le_bytes());
            hasher.update((descriptors.len() as u64).to_le_bytes());
            for descriptor in descriptors {
                hasher.update(descriptor.0);
            }
        }
    }
    if boundary_count == 0 || descriptor_count < boundary_count {
        return Err(route_message(
            "native tactic action-surface audit coverage is empty",
        ));
    }
    Ok((
        Digest(hasher.finalize().into()),
        boundary_count,
        descriptor_count,
    ))
}
