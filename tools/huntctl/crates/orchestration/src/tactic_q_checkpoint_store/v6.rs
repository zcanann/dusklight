use super::*;

const CHECKPOINT_MANIFEST_SCHEMA_V5: &str = "dusklight-tactic-q-checkpoint-manifest/v5";
const CHECKPOINT_INDEX_SCHEMA_V1: &str = "dusklight-tactic-q-checkpoint-index/v1";

pub(crate) struct TacticQCampaignPersistenceView<'a> {
    pub execution_authority_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub episode_group: u64,
    pub decision_index: u64,
    pub current: &'a LearnerState,
    pub route_tape: &'a InputTape,
    pub state_graph: &'a StateGraph,
    pub replay: &'a [OptionTransitionSample],
    pub replay_routes: &'a [InputTape],
    pub episode_groups: &'a [u64],
    pub prior_persistence: Option<&'a TacticQCheckpointPersistence>,
    pub model_revision: u64,
    pub model_config: &'a OptionValueConfig,
    pub exploration: TacticExplorationConfig,
}

pub(crate) struct TacticQCheckpointCommit {
    pub path: PathBuf,
    pub content_sha256: Digest,
    pub persistence: TacticQCheckpointPersistence,
    pub graph_head: StateGraphPersistenceHead,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCheckpointIndex {
    schema: String,
    parent: Option<StoredContentRef>,
    parent_rows: u64,
    transitions: Vec<StoredContentRef>,
    routes: Vec<StoredContentRef>,
    episode_groups: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredCheckpointManifestV5 {
    schema: String,
    content_sha256: Digest,
    execution_authority_sha256: Digest,
    feature_schema_sha256: Digest,
    objective_sha256: Digest,
    root_checkpoint_sha256: Digest,
    episode_group: u64,
    decision_index: u64,
    current: StoredLearnerState,
    route_tape: StoredContentRef,
    state_graph_head: StoredContentRef,
    state_graph_depth: u64,
    replay_index: StoredContentRef,
    replay_rows: u64,
    model_revision: u64,
    model_config: OptionValueConfig,
    exploration: TacticExplorationConfig,
}

pub(crate) fn write_checkpoint_v6(
    view: TacticQCampaignPersistenceView<'_>,
    directory: &Path,
    store: &TacticQContentStore,
) -> Result<TacticQCheckpointCommit, TacticQCampaignError> {
    validate_view(&view)?;
    if store
        .store
        .root()
        .file_name()
        .and_then(|name| name.to_str())
        != Some(CONTENT_DIRECTORY)
    {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint content root must use the discoverable objects directory",
        ));
    }
    fs::create_dir_all(directory).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    let current = StoredLearnerState {
        schema: view.current.schema.clone(),
        snapshot_sha256: view.current.snapshot_sha256,
        fact_registry_sha256: view.current.fact_registry_sha256,
        action_universe_sha256: view.current.action_universe_sha256,
        applicable_choice_schema_sha256: view.current.applicable_choice_schema_sha256,
        snapshot: store
            .store_fact(&view.current.snapshot)
            .map_err(checkpoint_store_error)?,
        action_mask: view.current.action_mask.clone(),
    };
    let route_tape = store
        .store_tape(view.route_tape)
        .map_err(checkpoint_store_error)?;
    let (state_graph_head, graph_head) = store
        .store_state_graph_journal(view.state_graph)
        .map_err(checkpoint_store_error)?;
    let replay_index = store_checkpoint_index(
        store,
        view.replay,
        view.replay_routes,
        view.episode_groups,
        view.prior_persistence.map(|persistence| {
            (
                StoredContentRef {
                    kind: ContentKind::TacticCheckpointIndex,
                    sha256: persistence.replay_index_sha256,
                },
                persistence.replay_rows,
            )
        }),
    )?;
    let persistence = TacticQCheckpointPersistence {
        schema: TACTIC_Q_CHECKPOINT_PERSISTENCE_SCHEMA_V1.into(),
        state_graph_head_sha256: graph_head.sha256,
        state_graph_depth: graph_head.depth,
        replay_index_sha256: replay_index.sha256,
        replay_rows: view.replay.len() as u64,
    };
    let content_sha256 = TacticQCheckpointIdentityV6 {
        schema: TACTIC_Q_CHECKPOINT_SCHEMA_V6,
        execution_authority_sha256: view.execution_authority_sha256,
        feature_schema_sha256: view.feature_schema_sha256,
        objective_sha256: view.objective_sha256,
        root_checkpoint_sha256: view.root_checkpoint_sha256,
        episode_group: view.episode_group,
        decision_index: view.decision_index,
        current: view.current,
        route_tape: view.route_tape,
        persistence: &persistence,
        model_revision: view.model_revision,
        model_config: view.model_config,
        exploration: view.exploration,
    }
    .content_sha256()?;
    let manifest = StoredCheckpointManifestV5 {
        schema: CHECKPOINT_MANIFEST_SCHEMA_V5.into(),
        content_sha256,
        execution_authority_sha256: view.execution_authority_sha256,
        feature_schema_sha256: view.feature_schema_sha256,
        objective_sha256: view.objective_sha256,
        root_checkpoint_sha256: view.root_checkpoint_sha256,
        episode_group: view.episode_group,
        decision_index: view.decision_index,
        current,
        route_tape,
        state_graph_head,
        state_graph_depth: graph_head.depth,
        replay_index,
        replay_rows: view.replay.len() as u64,
        model_revision: view.model_revision,
        model_config: view.model_config.clone(),
        exploration: view.exploration,
    };
    let raw = serde_cbor::to_vec(&manifest)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    let envelope = encode_checkpoint_envelope(&raw)?;
    let path = directory.join(format!(
        "tactic-q-{content_sha256}.{TACTIC_Q_CHECKPOINT_EXTENSION}"
    ));
    install_binary_artifact(&path, &envelope)?;
    Ok(TacticQCheckpointCommit {
        path,
        content_sha256,
        persistence,
        graph_head,
    })
}

pub(super) fn manifest_schema() -> &'static str {
    CHECKPOINT_MANIFEST_SCHEMA_V5
}

pub(super) fn decode_manifest(
    raw: &[u8],
) -> Result<StoredCheckpointManifestV5, TacticQCampaignError> {
    decode_cbor(raw).map_err(checkpoint_store_error)
}

pub(super) fn load_checkpoint(
    manifest: &StoredCheckpointManifestV5,
    store: &TacticQContentStore,
) -> Result<TacticQCampaignCheckpoint, TacticQCampaignError> {
    if manifest.schema != CHECKPOINT_MANIFEST_SCHEMA_V5
        || manifest.content_sha256 == Digest::ZERO
        || manifest.execution_authority_sha256 == Digest::ZERO
        || manifest.feature_schema_sha256 == Digest::ZERO
        || manifest.objective_sha256 == Digest::ZERO
        || manifest.root_checkpoint_sha256 == Digest::ZERO
        || manifest.state_graph_head.kind != ContentKind::StateGraphJournal
        || manifest.state_graph_head.sha256 == Digest::ZERO
        || manifest.replay_index.kind != ContentKind::TacticCheckpointIndex
    {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint v6 manifest identity is invalid",
        ));
    }
    let current = LearnerState {
        schema: manifest.current.schema.clone(),
        snapshot_sha256: manifest.current.snapshot_sha256,
        fact_registry_sha256: manifest.current.fact_registry_sha256,
        action_universe_sha256: manifest.current.action_universe_sha256,
        applicable_choice_schema_sha256: manifest.current.applicable_choice_schema_sha256,
        snapshot: store
            .load_fact(manifest.current.snapshot)
            .map_err(checkpoint_store_error)?,
        action_mask: manifest.current.action_mask.clone(),
    };
    let route_tape = store
        .load_tape(manifest.route_tape)
        .map_err(checkpoint_store_error)?;
    let graph_head = StateGraphPersistenceHead {
        sha256: manifest.state_graph_head.sha256,
        depth: manifest.state_graph_depth,
    };
    let state_graph = store
        .load_state_graph_journal(graph_head)
        .map_err(checkpoint_store_error)?;
    let (replay, replay_routes, episode_groups) =
        load_checkpoint_index(store, manifest.replay_index, manifest.replay_rows)?;
    let training = crate::tactic_q_campaign::graph_training_projection(&state_graph)?;
    let persistence = TacticQCheckpointPersistence {
        schema: TACTIC_Q_CHECKPOINT_PERSISTENCE_SCHEMA_V1.into(),
        state_graph_head_sha256: graph_head.sha256,
        state_graph_depth: graph_head.depth,
        replay_index_sha256: manifest.replay_index.sha256,
        replay_rows: manifest.replay_rows,
    };
    let checkpoint = TacticQCampaignCheckpoint {
        schema: TACTIC_Q_CHECKPOINT_SCHEMA_V6.into(),
        content_sha256: manifest.content_sha256,
        execution_authority_sha256: manifest.execution_authority_sha256,
        feature_schema_sha256: manifest.feature_schema_sha256,
        objective_sha256: manifest.objective_sha256,
        root_checkpoint_sha256: manifest.root_checkpoint_sha256,
        episode_group: manifest.episode_group,
        decision_index: manifest.decision_index,
        current,
        route_tape,
        state_graph,
        replay,
        replay_routes,
        episode_groups,
        training_replay: training.transitions,
        training_replay_routes: training.routes,
        training_episode_groups: training.episode_groups,
        model_revision: manifest.model_revision,
        model_config: manifest.model_config.clone(),
        exploration: manifest.exploration,
        persistence: Some(persistence),
        persistence_validated: true,
    };
    validate_checkpoint(&checkpoint)?;
    Ok(checkpoint)
}

fn validate_view(view: &TacticQCampaignPersistenceView<'_>) -> Result<(), TacticQCampaignError> {
    view.current.validate()?;
    view.route_tape
        .validate()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    let graph_identity = &view.state_graph.identity;
    let current_route = route_checkpoint_sha256(view.root_checkpoint_sha256, view.route_tape)?;
    let current_id = ExactStateId {
        route_checkpoint_sha256: current_route,
        state_sha256: view.current.snapshot_sha256,
    };
    if view.execution_authority_sha256 == Digest::ZERO
        || view.feature_schema_sha256 == Digest::ZERO
        || view.objective_sha256 == Digest::ZERO
        || view.root_checkpoint_sha256 == Digest::ZERO
        || view.replay.len() != view.replay_routes.len()
        || view.replay.len() != view.episode_groups.len()
        || view.decision_index != view.replay.len() as u64
        || graph_identity.execution_authority_sha256 != view.execution_authority_sha256
        || graph_identity.feature_schema_sha256 != view.feature_schema_sha256
        || graph_identity.objective_sha256 != view.objective_sha256
        || graph_identity.root_checkpoint_sha256 != view.root_checkpoint_sha256
        || view.state_graph.node(current_id).is_none()
        || view.prior_persistence.is_some_and(|persistence| {
            persistence.schema != TACTIC_Q_CHECKPOINT_PERSISTENCE_SCHEMA_V1
                || persistence.replay_rows > view.replay.len() as u64
        })
        || view.current.snapshot.tape_frame != view.route_tape.frames.len() as u64
    {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint v6 campaign view is invalid",
        ));
    }
    Ok(())
}

fn store_checkpoint_index(
    store: &TacticQContentStore,
    transitions: &[OptionTransitionSample],
    routes: &[InputTape],
    episode_groups: &[u64],
    previous: Option<(StoredContentRef, u64)>,
) -> Result<StoredContentRef, TacticQCampaignError> {
    if transitions.len() != routes.len() || transitions.len() != episode_groups.len() {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint index shape is invalid",
        ));
    }
    let previous_rows = previous.map_or(0, |(_, rows)| rows);
    let start = usize::try_from(previous_rows)
        .map_err(|_| TacticQCampaignError::InvalidState("checkpoint index rows overflow"))?;
    if start > transitions.len() {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint replay is not append-only",
        ));
    }
    if start == transitions.len()
        && let Some((reference, _)) = previous
    {
        return Ok(reference);
    }
    let mut stored_transitions = Vec::with_capacity(transitions.len() - start);
    let mut stored_routes = Vec::with_capacity(routes.len() - start);
    for (transition, route) in transitions[start..].iter().zip(&routes[start..]) {
        stored_transitions.push(
            store
                .store_option_transition(transition, route)
                .map_err(checkpoint_store_error)?,
        );
        stored_routes.push(store.store_tape(route).map_err(checkpoint_store_error)?);
    }
    let raw = serde_cbor::to_vec(&StoredCheckpointIndex {
        schema: CHECKPOINT_INDEX_SCHEMA_V1.into(),
        parent: previous.map(|(reference, _)| reference),
        parent_rows: previous_rows,
        transitions: stored_transitions,
        routes: stored_routes,
        episode_groups: episode_groups[start..].to_vec(),
    })
    .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(StoredContentRef::from(
        &store
            .store
            .put_bytes(&raw, ContentKind::TacticCheckpointIndex)
            .map_err(TacticQContentStoreError::Store)
            .map_err(checkpoint_store_error)?,
    ))
}

type LoadedCheckpointIndex = (Vec<OptionTransitionSample>, Vec<InputTape>, Vec<u64>);

fn load_checkpoint_index(
    store: &TacticQContentStore,
    reference: StoredContentRef,
    expected_rows: u64,
) -> Result<LoadedCheckpointIndex, TacticQCampaignError> {
    let mut records = Vec::new();
    let mut next = Some((reference, expected_rows));
    while let Some((reference, rows)) = next {
        require_kind(reference, ContentKind::TacticCheckpointIndex)
            .map_err(checkpoint_store_error)?;
        let index: StoredCheckpointIndex =
            store.read_cbor(reference).map_err(checkpoint_store_error)?;
        let appended_rows = index.transitions.len() as u64;
        if index.schema != CHECKPOINT_INDEX_SCHEMA_V1
            || index.transitions.len() != index.routes.len()
            || index.transitions.len() != index.episode_groups.len()
            || index.parent_rows.checked_add(appended_rows) != Some(rows)
            || (index.parent.is_none() && index.parent_rows != 0)
            || (index.parent.is_some() && appended_rows == 0)
        {
            return Err(TacticQCampaignError::InvalidState(
                "checkpoint index is invalid",
            ));
        }
        next = index.parent.map(|parent| (parent, index.parent_rows));
        records.push(index);
    }
    let capacity = usize::try_from(expected_rows)
        .map_err(|_| TacticQCampaignError::InvalidState("checkpoint index rows overflow"))?;
    let mut transitions = Vec::with_capacity(capacity);
    let mut routes = Vec::with_capacity(capacity);
    let mut episode_groups = Vec::with_capacity(capacity);
    for index in records.into_iter().rev() {
        for reference in index.transitions {
            transitions.push(
                store
                    .load_option_transition(reference)
                    .map_err(checkpoint_store_error)?,
            );
        }
        for reference in index.routes {
            routes.push(store.load_tape(reference).map_err(checkpoint_store_error)?);
        }
        episode_groups.extend(index.episode_groups);
    }
    Ok((transitions, routes, episode_groups))
}
