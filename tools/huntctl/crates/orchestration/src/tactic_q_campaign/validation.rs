use super::*;

pub(crate) struct TacticQCheckpointRuntimeValidation {
    pub(crate) state_graph: StateGraphValidationToken,
    pub(crate) training_projection_keys: Vec<(Digest, Digest)>,
}

pub(crate) fn validate_checkpoint(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<(), TacticQCampaignError> {
    validate_checkpoint_for_resume(checkpoint).map(drop)
}

pub(crate) fn validate_checkpoint_for_resume(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<TacticQCheckpointRuntimeValidation, TacticQCampaignError> {
    let validation = validate_checkpoint_snapshot_for_runtime(checkpoint)?;
    if checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V6 && !checkpoint.persistence_validated {
        return Err(TacticQCampaignError::InvalidState(
            "campaign checkpoint persistence was not authenticated",
        ));
    }
    Ok(validation)
}

/// Validates the complete inline checkpoint payload and its content identity.
///
/// This is deliberately weaker than [`validate_checkpoint`]: an evidence
/// bundle may carry a self-contained inspection snapshot without the external
/// content store that authenticated its persistence references. Such a
/// snapshot is safe to inspect but must not be passed to `resume`.
pub(crate) fn validate_checkpoint_snapshot(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<(), TacticQCampaignError> {
    validate_checkpoint_snapshot_for_runtime(checkpoint).map(drop)
}

fn validate_checkpoint_snapshot_for_runtime(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<TacticQCheckpointRuntimeValidation, TacticQCampaignError> {
    if checkpoint.content_sha256 == Digest::ZERO {
        return Err(TacticQCampaignError::InvalidState(
            "campaign checkpoint content identity is invalid",
        ));
    }
    let reconstructed = checkpoint_digest(checkpoint)?;
    if checkpoint.content_sha256 != reconstructed {
        return Err(TacticQCampaignError::CheckpointIdentityMismatch {
            stored: checkpoint.content_sha256,
            reconstructed,
        });
    }
    validate_checkpoint_payload_for_runtime(checkpoint)
}

pub(crate) fn validate_checkpoint_payload(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<(), TacticQCampaignError> {
    validate_checkpoint_payload_for_runtime(checkpoint).map(drop)
}

fn validate_checkpoint_payload_for_runtime(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<TacticQCheckpointRuntimeValidation, TacticQCampaignError> {
    checkpoint.current.validate()?;
    checkpoint
        .route_tape
        .validate()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    let legacy = checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V5;
    let current = checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V6;
    let validated_graph = checkpoint.state_graph.validated()?;
    let graph_identity = &checkpoint.state_graph.identity;
    let persistence_valid = match (&checkpoint.persistence, legacy, current) {
        (None, true, false) => true,
        (Some(persistence), false, true) => {
            persistence.schema == TACTIC_Q_CHECKPOINT_PERSISTENCE_SCHEMA_V1
                && persistence.state_graph_head_sha256 != Digest::ZERO
                && persistence.replay_index_sha256 != Digest::ZERO
                && persistence.replay_rows == checkpoint.replay.len() as u64
        }
        _ => false,
    };
    if (!legacy && !current)
        || !persistence_valid
        || checkpoint.feature_schema_sha256 == Digest::ZERO
        || checkpoint.objective_sha256 == Digest::ZERO
        || checkpoint.root_checkpoint_sha256 == Digest::ZERO
        || graph_identity.execution_authority_sha256 != checkpoint.execution_authority_sha256
        || graph_identity.feature_schema_sha256 != checkpoint.feature_schema_sha256
        || graph_identity.objective_sha256 != checkpoint.objective_sha256
        || graph_identity.root_checkpoint_sha256 != checkpoint.root_checkpoint_sha256
        || checkpoint.exploration.epsilon_per_million > 1_000_000
        || checkpoint.replay.len() != checkpoint.episode_groups.len()
        || checkpoint.replay.len() != checkpoint.replay_routes.len()
        || checkpoint.decision_index != checkpoint.replay.len() as u64
        || checkpoint.training_replay.len() != checkpoint.training_replay_routes.len()
        || checkpoint.training_replay.len() != checkpoint.training_episode_groups.len()
        || checkpoint.training_replay.len() < checkpoint.replay.len()
        || checkpoint.current.snapshot.tape_frame != checkpoint.route_tape.frames.len() as u64
    {
        return Err(TacticQCampaignError::InvalidState(
            "campaign checkpoint identity or shape is invalid",
        ));
    }
    let mut endpoints = BTreeMap::<u64, (Digest, Digest)>::new();
    for ((transition, route), episode_group) in checkpoint
        .replay
        .iter()
        .zip(&checkpoint.replay_routes)
        .zip(&checkpoint.episode_groups)
    {
        transition.validate()?;
        if transition.execution_authority_sha256 != checkpoint.execution_authority_sha256
            || transition.feature_schema_sha256 != checkpoint.feature_schema_sha256
            || endpoints.get(episode_group).is_some_and(|(state, route)| {
                *state != transition.before_state_sha256
                    || *route != transition.source_checkpoint_sha256
            })
        {
            return Err(TacticQCampaignError::InvalidState(
                "campaign checkpoint replay chain is detached",
            ));
        }
        transition
            .execution
            .validate_against_tape(route)
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
        let start = usize::try_from(transition.execution.realized_tape_range.start_frame)
            .map_err(|_| TacticQCampaignError::InvalidState("replay tape range overflows"))?;
        let end = usize::try_from(transition.execution.realized_tape_range.end_frame_exclusive)
            .map_err(|_| TacticQCampaignError::InvalidState("replay tape range overflows"))?;
        if end > route.frames.len()
            || transition.source_checkpoint_sha256
                != route_checkpoint(
                    checkpoint.root_checkpoint_sha256,
                    &tape_prefix(route, start),
                )?
            || transition.next_checkpoint_sha256
                != route_checkpoint(checkpoint.root_checkpoint_sha256, &tape_prefix(route, end))?
        {
            return Err(TacticQCampaignError::InvalidState(
                "campaign checkpoint replay route is detached",
            ));
        }
        endpoints.insert(
            *episode_group,
            (
                transition.after_state_sha256,
                transition.next_checkpoint_sha256,
            ),
        );
    }
    if let Some((after, route)) = endpoints.get(&checkpoint.episode_group)
        && (*after != checkpoint.current.snapshot_sha256
            || *route
                != route_checkpoint(checkpoint.root_checkpoint_sha256, &checkpoint.route_tape)?)
    {
        return Err(TacticQCampaignError::InvalidState(
            "campaign checkpoint current state is not the replay endpoint",
        ));
    }
    let training_replay = &checkpoint.training_replay;
    let training_routes = &checkpoint.training_replay_routes;
    let training_groups = &checkpoint.training_episode_groups;
    let mut training_identities = BTreeSet::new();
    for ((transition, route), _) in training_replay
        .iter()
        .zip(training_routes)
        .zip(training_groups)
    {
        validate_training_transition(
            checkpoint.execution_authority_sha256,
            checkpoint.feature_schema_sha256,
            checkpoint.root_checkpoint_sha256,
            transition,
            route,
        )?;
        if !training_identities.insert(transition.replay_identity_sha256()?) {
            return Err(TacticQCampaignError::InvalidState(
                "campaign training replay is detached or duplicated",
            ));
        }
    }
    if checkpoint.replay.iter().any(|transition| {
        transition
            .replay_identity_sha256()
            .map_or(true, |identity| !training_identities.contains(&identity))
    }) {
        return Err(TacticQCampaignError::InvalidState(
            "retained replay is absent from training replay",
        ));
    }
    let training_projection_keys = validate_training_projection_and_keys(
        validated_graph,
        &checkpoint.training_replay,
        &checkpoint.training_replay_routes,
        &checkpoint.training_episode_groups,
    )?;
    let current_route =
        route_checkpoint(checkpoint.root_checkpoint_sha256, &checkpoint.route_tape)?;
    if checkpoint
        .state_graph
        .node(crate::state_graph::ExactStateId {
            route_checkpoint_sha256: current_route,
            state_sha256: checkpoint.current.snapshot_sha256,
        })
        .is_none()
    {
        return Err(TacticQCampaignError::InvalidState(
            "campaign current boundary is absent from the state graph",
        ));
    }
    Ok(TacticQCheckpointRuntimeValidation {
        state_graph: validated_graph.validation_token(),
        training_projection_keys,
    })
}

pub(crate) fn validate_training_corpus(
    corpus: &TacticQTrainingCorpus,
) -> Result<(), TacticQCampaignError> {
    if corpus.feature_schema_sha256 == Digest::ZERO
        || corpus.objective_sha256 == Digest::ZERO
        || corpus.root_checkpoint_sha256 == Digest::ZERO
        || corpus.transitions.len() != corpus.routes.len()
        || corpus.transitions.len() != corpus.episode_groups.len()
    {
        return Err(TacticQCampaignError::InvalidState(
            "shared tactic training corpus identity or shape is invalid",
        ));
    }
    let mut identities = BTreeSet::new();
    for (transition, route) in corpus.transitions.iter().zip(&corpus.routes) {
        validate_training_transition(
            corpus.execution_authority_sha256,
            corpus.feature_schema_sha256,
            corpus.root_checkpoint_sha256,
            transition,
            route,
        )?;
        if !identities.insert(transition.replay_identity_sha256()?) {
            return Err(TacticQCampaignError::InvalidState(
                "shared tactic training corpus contains duplicate transitions",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_training_transition(
    execution_authority_sha256: Digest,
    feature_schema_sha256: Digest,
    root_checkpoint_sha256: Digest,
    transition: &OptionTransitionSample,
    route: &InputTape,
) -> Result<(), TacticQCampaignError> {
    transition.validate()?;
    transition
        .execution
        .validate_against_tape(route)
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    let start = usize::try_from(transition.execution.realized_tape_range.start_frame)
        .map_err(|_| TacticQCampaignError::InvalidState("training tape range overflows"))?;
    let end = usize::try_from(transition.execution.realized_tape_range.end_frame_exclusive)
        .map_err(|_| TacticQCampaignError::InvalidState("training tape range overflows"))?;
    if transition.execution_authority_sha256 != execution_authority_sha256
        || transition.feature_schema_sha256 != feature_schema_sha256
        || end > route.frames.len()
        || transition.source_checkpoint_sha256
            != route_checkpoint(root_checkpoint_sha256, &tape_prefix(route, start))?
        || transition.next_checkpoint_sha256
            != route_checkpoint(root_checkpoint_sha256, &tape_prefix(route, end))?
    {
        return Err(TacticQCampaignError::InvalidState(
            "campaign training replay is detached",
        ));
    }
    Ok(())
}

pub(crate) fn replay_model(
    feature_schema_sha256: Digest,
    objective_sha256: Digest,
    replay: &[OptionTransitionSample],
    episode_groups: &[u64],
    config: &OptionValueConfig,
) -> Result<Option<OptionValueModel>, TacticQCampaignError> {
    if replay.len() != episode_groups.len() {
        return Err(TacticQCampaignError::InvalidState(
            "replay model rows and episode groups differ",
        ));
    }
    let returns = authenticated_terminal_conditional_returns(replay)?;
    let supported = replay
        .iter()
        .zip(episode_groups)
        .zip(returns)
        .filter_map(|((transition, episode_group), objective_return)| {
            objective_return.map(|objective_return| {
                let mut sample = transition.value_sample.clone();
                sample.reward = objective_return;
                // This regression row already contains the complete closed
                // return. Prevent the generic critic from bootstrapping it a
                // second time.
                sample.terminal = true;
                (sample, *episode_group)
            })
        })
        .collect::<Vec<_>>();
    let Some((first, _)) = supported.first() else {
        return Ok(None);
    };
    let exact_actions = supported
        .iter()
        .map(|(sample, _)| sample.action.content_sha256())
        .collect::<Result<BTreeSet<_>, OptionValueError>>()?;
    if exact_actions.len() > MAX_OPTION_ACTIONS {
        return Ok(None);
    }
    let batch = OptionValueBatch::new(
        feature_schema_sha256,
        objective_sha256,
        first.state.len(),
        supported.iter().map(|(sample, _)| sample.clone()).collect(),
        supported
            .iter()
            .map(|(_, episode_group)| *episode_group)
            .collect(),
    )?;
    Ok(Some(OptionValueModel::fit_batch(&batch, config)?))
}

pub(crate) fn training_replay_sha256(
    transitions: &[OptionTransitionSample],
    episode_groups: &[u64],
) -> Result<Digest, TacticQCampaignError> {
    if transitions.len() != episode_groups.len() {
        return Err(TacticQCampaignError::InvalidState(
            "learner snapshot replay shape is invalid",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.tactic-q-learner-replay/v1\0");
    hasher.update((transitions.len() as u64).to_le_bytes());
    for (transition, episode_group) in transitions.iter().zip(episode_groups) {
        hasher.update(transition.replay_identity_sha256()?.0);
        hasher.update(episode_group.to_le_bytes());
    }
    Ok(Digest(hasher.finalize().into()))
}

pub(crate) fn checkpoint_digest(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<Digest, TacticQCampaignError> {
    if checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V6 {
        let persistence =
            checkpoint
                .persistence
                .as_ref()
                .ok_or(TacticQCampaignError::InvalidState(
                    "current checkpoint persistence identity is missing",
                ))?;
        let identity = TacticQCheckpointIdentityV6 {
            schema: &checkpoint.schema,
            execution_authority_sha256: checkpoint.execution_authority_sha256,
            feature_schema_sha256: checkpoint.feature_schema_sha256,
            objective_sha256: checkpoint.objective_sha256,
            root_checkpoint_sha256: checkpoint.root_checkpoint_sha256,
            episode_group: checkpoint.episode_group,
            decision_index: checkpoint.decision_index,
            current: &checkpoint.current,
            route_tape: &checkpoint.route_tape,
            persistence,
            model_revision: checkpoint.model_revision,
            model_config: &checkpoint.model_config,
            exploration: checkpoint.exploration,
        };
        return identity.content_sha256();
    }
    let mut canonical = checkpoint.clone();
    canonical.content_sha256 = Digest::ZERO;
    let bytes = serde_cbor::to_vec(&canonical)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

#[derive(Serialize)]
pub(crate) struct TacticQCheckpointIdentityV6<'a> {
    pub schema: &'a str,
    pub execution_authority_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub episode_group: u64,
    pub decision_index: u64,
    pub current: &'a LearnerState,
    pub route_tape: &'a InputTape,
    pub persistence: &'a TacticQCheckpointPersistence,
    pub model_revision: u64,
    pub model_config: &'a OptionValueConfig,
    pub exploration: TacticExplorationConfig,
}

impl TacticQCheckpointIdentityV6<'_> {
    pub(crate) fn content_sha256(&self) -> Result<Digest, TacticQCampaignError> {
        let bytes = serde_cbor::to_vec(self)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        Ok(sha256(&bytes))
    }
}

pub(crate) fn validate_final_result(
    result: &TacticQFinalResult,
) -> Result<(), TacticQCampaignError> {
    result
        .route_tape
        .validate()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    result
        .terminal
        .validate()
        .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
    let route_bytes = result
        .route_tape
        .encode()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    let replay_bytes = serde_cbor::to_vec(&(&result.replay, &result.replay_routes))
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    if result.schema != TACTIC_Q_FINAL_RESULT_SCHEMA_V2
        || result.content_sha256 == Digest::ZERO
        || result.content_sha256 != final_result_digest(result)?
        || result.objective_sha256 == Digest::ZERO
        || result.root_checkpoint_sha256 == Digest::ZERO
        || result.route_tape_sha256 != sha256(&route_bytes)
        || result.replay_sha256 != sha256(&replay_bytes)
        || result.replay.len() != result.replay_routes.len()
        || result.replay.iter().any(|transition| {
            transition.execution_authority_sha256 != result.execution_authority_sha256
        })
        || result.terminal_state_sha256
            != result
                .terminal
                .content_sha256()
                .map_err(|error| TacticQCampaignError::Features(error.to_string()))?
        || result.terminal.terminal.configured != Some(true)
        || result.terminal.terminal.reached != Some(true)
        || result.terminal.tape_frame != result.route_tape.frames.len() as u64
        || result
            .replay
            .last()
            .map(|transition| transition.after_state_sha256)
            != Some(result.terminal_state_sha256)
    {
        return Err(TacticQCampaignError::InvalidState(
            "final tactic-Q result is not an authenticated terminal route",
        ));
    }
    for (transition, route) in result.replay.iter().zip(&result.replay_routes) {
        transition.validate()?;
        transition
            .execution
            .validate_against_tape(route)
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    }
    Ok(())
}

pub(crate) fn final_result_digest(
    result: &TacticQFinalResult,
) -> Result<Digest, TacticQCampaignError> {
    // The component digests already bind the exact route, replay, and terminal
    // payloads. Seal the small identity tuple instead of serializing those
    // multi-megabyte values a second time merely to derive the outer identity.
    let bytes = if result.execution_authority_sha256 == Digest::ZERO {
        serde_cbor::to_vec(&(
            &result.schema,
            result.objective_sha256,
            result.root_checkpoint_sha256,
            result.route_tape_sha256,
            result.replay_sha256,
            result.terminal_state_sha256,
        ))
    } else {
        serde_cbor::to_vec(&(
            "dusklight-tactic-q-final-result-identity/v3",
            &result.schema,
            result.execution_authority_sha256,
            result.objective_sha256,
            result.root_checkpoint_sha256,
            result.route_tape_sha256,
            result.replay_sha256,
            result.terminal_state_sha256,
        ))
    }
    .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

pub(crate) fn tape_prefix(tape: &InputTape, frame_count: usize) -> InputTape {
    InputTape {
        boot: tape.boot.clone(),
        tick_rate_numerator: tape.tick_rate_numerator,
        tick_rate_denominator: tape.tick_rate_denominator,
        frames: tape.frames[..frame_count].to_vec(),
    }
}

pub(crate) fn extends(prefix: &InputTape, route: &InputTape) -> bool {
    prefix.boot == route.boot
        && prefix.tick_rate_numerator == route.tick_rate_numerator
        && prefix.tick_rate_denominator == route.tick_rate_denominator
        && route.frames.starts_with(&prefix.frames)
        && route.frames.len() > prefix.frames.len()
}

pub(crate) fn route_checkpoint(
    root_checkpoint_sha256: Digest,
    route: &InputTape,
) -> Result<Digest, TacticQCampaignError> {
    crate::state_graph::route_checkpoint_sha256(root_checkpoint_sha256, route)
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))
}

pub(crate) fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}
