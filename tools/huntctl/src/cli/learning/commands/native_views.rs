//! Native encoder pretraining and typed observation-view commands.

use super::{
    ActorProfileCatalog, CompleteSetMultiTaskEncoder, ContentKind, ContentStore,
    DEFAULT_COLLISION_HISTORY_DEPTH, DEFAULT_EPISODE_HISTORY_DEPTH,
    DEFAULT_HISTORY_RECURRENT_WIDTH, GeometryObservationStatus, MAX_LEARN_INPUT_CORPORA,
    MultiTaskSetPooling, NativeAuxiliaryDataset, NativeCollisionHistoryView,
    NativeEncoderChannelFamily, NativeEncoderFeatureSpec, NativeEpisodeActorView,
    NativeEpisodeGeometryView, NativeEpisodeHistoryView, NativeEpisodeResourceLoadView,
    NativeEpisodeRoomLoadView, NativeEpisodeShard, NativeEpisodeSurfaceGraphView,
    NativeGeometryViewConfiguration, NativeMultiTaskActorCorpus,
    NativeSurfaceGraphViewConfiguration, ResourceArchiveKind, ResourceLoadOutcome,
    ResourceLoadSetStatus, RoomLoadSetStatus, RoomSceneSetStatus, SurfaceGraphObservationStatus,
    TrainableSetConfig, WorldInventory, fit_shuffled_auxiliary_control_with_pooling_and_temporal,
    option, repeated_option, required_path, u64_option, usage_error, usize_option,
};
use serde_json::json;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn command(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("pretrain-native-encoder") => {
            let learn_args = &args[1..];
            let dataset_path = required_path(learn_args, "--dataset")?;
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("native encoder output already exists: {}", output.display()).into(),
                );
            }
            let dataset: NativeAuxiliaryDataset =
                serde_json::from_slice(&fs::read(&dataset_path)?)?;
            let shard = NativeEpisodeShard::read(&input)?;
            let source_native_shard_sha256 = shard.content_sha256;
            let excluded = repeated_option(learn_args, "--exclude-family")
                .into_iter()
                .map(|name| {
                    NativeEncoderChannelFamily::parse(&name)
                        .ok_or_else(|| format!("unknown native encoder channel family: {name}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let history_depth = usize_option(learn_args, "--history-depth", 0)?;
            let history_encoding = option(learn_args, "--history-encoding");
            let history_width = option(learn_args, "--history-width")
                .map(|value| value.parse::<usize>())
                .transpose()?;
            let base_feature_spec = NativeEncoderFeatureSpec::excluding(excluded)?;
            let feature_spec = match history_encoding.as_deref() {
                None if history_width.is_none() => {
                    base_feature_spec.with_history_depth(history_depth)?
                }
                Some("stacked") if history_depth > 0 && history_width.is_none() => {
                    base_feature_spec.with_history_depth(history_depth)?
                }
                Some("recurrent-reservoir") if history_depth > 0 => base_feature_spec
                    .with_recurrent_history(
                        history_depth,
                        history_width.unwrap_or(DEFAULT_HISTORY_RECURRENT_WIDTH),
                    )?,
                Some("trainable-gru") if history_depth > 0 => base_feature_spec
                    .with_trainable_history(
                        history_depth,
                        history_width.unwrap_or(DEFAULT_HISTORY_RECURRENT_WIDTH),
                    )?,
                Some("stacked" | "recurrent-reservoir" | "trainable-gru") => {
                    return Err("native encoder history encoding requires --history-depth greater than zero".into());
                }
                Some(name) => {
                    return Err(format!("unknown native encoder history encoding: {name}").into());
                }
                None => {
                    return Err(
                        "--history-width requires --history-encoding recurrent-reservoir or trainable-gru".into(),
                    );
                }
            };
            let pooling = option(learn_args, "--pooling")
                .map(|name| {
                    MultiTaskSetPooling::parse(&name)
                        .ok_or_else(|| format!("unknown native encoder pooling mode: {name}"))
                })
                .transpose()?
                .unwrap_or(MultiTaskSetPooling::MeanMax);
            let corpus =
                NativeMultiTaskActorCorpus::build_with_spec(&dataset, &shard, feature_spec)?;
            let temporal = corpus.feature_spec.temporal_config();
            drop(shard);
            let defaults = TrainableSetConfig::default();
            let config = TrainableSetConfig {
                epochs: usize_option(learn_args, "--epochs", defaults.epochs)?,
                node_hidden_width: usize_option(
                    learn_args,
                    "--node-hidden-width",
                    defaults.node_hidden_width,
                )?,
                head_hidden_width: usize_option(
                    learn_args,
                    "--state-width",
                    defaults.head_hidden_width,
                )?,
                learning_rate: option(learn_args, "--learning-rate")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.learning_rate),
                l2_penalty: option(learn_args, "--l2-penalty")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.l2_penalty),
                gradient_clip: option(learn_args, "--gradient-clip")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.gradient_clip),
                minimum_relative_improvement: option(learn_args, "--minimum-relative-improvement")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.minimum_relative_improvement),
                seed: u64_option(learn_args, "--seed", defaults.seed)?,
                fixed_slot_count: defaults.fixed_slot_count,
            };
            let (report, model) = CompleteSetMultiTaskEncoder::fit_with_pooling_and_temporal(
                corpus.actor_feature_schema_sha256,
                corpus.training_dataset_sha256,
                corpus.validation_dataset_sha256,
                corpus.target_names.clone(),
                &corpus.training,
                &corpus.validation,
                config,
                pooling,
                temporal,
            )?;
            let test_evaluation = model.evaluate(&corpus.test)?;
            let shuffled_target_control = fit_shuffled_auxiliary_control_with_pooling_and_temporal(
                corpus.actor_feature_schema_sha256,
                corpus.target_names.clone(),
                corpus.training,
                corpus.validation_dataset_sha256,
                &corpus.validation,
                &corpus.test,
                config,
                pooling,
                temporal,
            )?;
            let artifact = json!({
                "schema": "dusklight-native-multitask-encoder-artifact/v13",
                "source_auxiliary_dataset_sha256": dataset.dataset_sha256,
                "source_native_shard_sha256": source_native_shard_sha256,
                "actor_feature_schema_sha256": corpus.actor_feature_schema_sha256,
                "feature_spec": corpus.feature_spec,
                "test_dataset_sha256": corpus.test_dataset_sha256,
                "report": report,
                "test_evaluation": test_evaluation,
                "shuffled_target_control": shuffled_target_control,
                "model": model,
            });
            let bytes = serde_json::to_vec_pretty(&artifact)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob =
                ContentStore::initialize(&artifact_store)?.put_bytes(&bytes, ContentKind::Model)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": artifact["schema"],
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": artifact["report"],
                    "test_evaluation": artifact["test_evaluation"],
                    "shuffled_target_control": artifact["shuffled_target_control"],
                }))?
            );
            Ok(())
        }
        Some("collision-history") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "collision history output already exists: {}",
                    output.display()
                )
                .into());
            }
            let history_depth = usize_option(
                learn_args,
                "--history-depth",
                DEFAULT_COLLISION_HISTORY_DEPTH,
            )?;
            let shard = NativeEpisodeShard::read(&input)?;
            let view = NativeCollisionHistoryView::build(&shard, history_depth)?;
            let bytes = view.canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeCollisionHistory)?;
            let solver_present = view
                .decisions
                .iter()
                .filter(|decision| {
                    view.snapshots[decision.current_snapshot_index as usize]
                        .solver
                        .is_some()
                })
                .count();
            let background_present = view
                .decisions
                .iter()
                .filter(|decision| {
                    view.snapshots[decision.current_snapshot_index as usize]
                        .background
                        .is_some()
                })
                .count();
            let solver_changes = view
                .auxiliary_targets
                .iter()
                .filter(|target| {
                    view.snapshots[target.before_snapshot_index as usize].solver
                        != view.snapshots[target.after_snapshot_index as usize].solver
                })
                .count();
            let background_changes = view
                .auxiliary_targets
                .iter()
                .filter(|target| {
                    view.snapshots[target.before_snapshot_index as usize].background
                        != view.snapshots[target.after_snapshot_index as usize].background
                })
                .count();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "history_depth": view.history_depth,
                    "snapshots": view.snapshots.len(),
                    "decisions": view.decisions.len(),
                    "auxiliary_targets": view.auxiliary_targets.len(),
                    "solver_present": solver_present,
                    "background_present": background_present,
                    "solver_changes": solver_changes,
                    "background_changes": background_changes,
                }))?
            );
            Ok(())
        }
        Some("episode-history") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "episode history output already exists: {}",
                    output.display()
                )
                .into());
            }
            let history_depth =
                usize_option(learn_args, "--history-depth", DEFAULT_EPISODE_HISTORY_DEPTH)?;
            let shard = NativeEpisodeShard::read(&input)?;
            let view = NativeEpisodeHistoryView::build(&shard, history_depth)?;
            let bytes = view.canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeEpisodeHistory)?;
            let populated_decisions = view
                .decisions
                .iter()
                .filter(|decision| !decision.completed_transition_indices.is_empty())
                .count();
            let maximum_realized_depth = view
                .decisions
                .iter()
                .map(|decision| decision.completed_transition_indices.len())
                .max()
                .unwrap_or(0);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "history_depth": view.history_depth,
                    "source_observations": view.source_observation_count,
                    "decisions": view.decisions.len(),
                    "transitions": view.transitions.len(),
                    "decisions_with_history": populated_decisions,
                    "maximum_realized_depth": maximum_realized_depth,
                }))?
            );
            Ok(())
        }
        Some("geometry-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let inventory_paths = repeated_option(learn_args, "--world-inventory");
            if inventory_paths.is_empty() || inventory_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn geometry-view requires 1..={MAX_LEARN_INPUT_CORPORA} --world-inventory INVENTORY.json"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("geometry view output already exists: {}", output.display()).into(),
                );
            }
            let defaults = NativeGeometryViewConfiguration::default();
            let configuration = NativeGeometryViewConfiguration {
                maximum_distance: option(learn_args, "--maximum-distance")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.maximum_distance),
                surface_limit: usize_option(learn_args, "--surface-limit", defaults.surface_limit)?,
            };
            let shard = NativeEpisodeShard::read(&input)?;
            let inventories = inventory_paths
                .iter()
                .map(|path| WorldInventory::read_canonical(Path::new(path)))
                .collect::<Result<Vec<_>, _>>()?;
            let view = NativeEpisodeGeometryView::build(&shard, &inventories, configuration)?;
            let bytes = view.canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeGeometryView)?;
            let present = view
                .observations
                .iter()
                .filter(|observation| observation.status == GeometryObservationStatus::Present)
                .count();
            let player_absent = view
                .observations
                .iter()
                .filter(|observation| observation.status == GeometryObservationStatus::PlayerAbsent)
                .count();
            let room_unavailable = view.observations.len() - present - player_absent;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "worlds": view.worlds.iter().map(|world| json!({
                        "stage": world.stage,
                        "inventory_sha256": world.inventory_sha256,
                        "spatial_index_sha256": world.spatial_index_sha256,
                        "placements": world.placements.len(),
                    })).collect::<Vec<_>>(),
                    "configuration": view.configuration,
                    "observations": view.observations.len(),
                    "present": present,
                    "player_absent": player_absent,
                    "room_unavailable": room_unavailable,
                    "probes": view.observations.iter()
                        .map(|observation| observation.probes.len()).sum::<usize>(),
                }))?
            );
            Ok(())
        }
        Some("surface-graph-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let inventory_paths = repeated_option(learn_args, "--world-inventory");
            if inventory_paths.is_empty() || inventory_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn surface-graph-view requires 1..={MAX_LEARN_INPUT_CORPORA} --world-inventory INVENTORY.json"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "surface graph view output already exists: {}",
                    output.display()
                )
                .into());
            }
            let defaults = NativeSurfaceGraphViewConfiguration::default();
            let configuration = NativeSurfaceGraphViewConfiguration {
                maximum_hops: option(learn_args, "--maximum-hops")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.maximum_hops),
                maximum_nodes: usize_option(learn_args, "--node-limit", defaults.maximum_nodes)?,
            };
            let geometry = NativeEpisodeGeometryView::decode_canonical(&fs::read(&input)?)?;
            let inventories = inventory_paths
                .iter()
                .map(|path| WorldInventory::read_canonical(Path::new(path)))
                .collect::<Result<Vec<_>, _>>()?;
            let view =
                NativeEpisodeSurfaceGraphView::build(&geometry, &inventories, configuration)?;
            let bytes = view.canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeSurfaceGraphView)?;
            let present = view
                .observations
                .iter()
                .filter(|observation| observation.status == SurfaceGraphObservationStatus::Present)
                .count();
            let no_surface_seed = view
                .observations
                .iter()
                .filter(|observation| {
                    observation.status == SurfaceGraphObservationStatus::NoSurfaceSeed
                })
                .count();
            let player_absent = view
                .observations
                .iter()
                .filter(|observation| {
                    observation.status == SurfaceGraphObservationStatus::PlayerAbsent
                })
                .count();
            let room_unavailable =
                view.observations.len() - present - no_surface_seed - player_absent;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_geometry_view_sha256": view.native_geometry_view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "worlds": view.worlds,
                    "configuration": view.configuration,
                    "observations": view.observations.len(),
                    "present": present,
                    "no_surface_seed": no_surface_seed,
                    "player_absent": player_absent,
                    "room_unavailable": room_unavailable,
                    "reachable_nodes": view.observations.iter()
                        .filter_map(|observation| observation.neighborhood.as_ref())
                        .map(|report| report.reachable_within_hops).sum::<usize>(),
                    "returned_nodes": view.observations.iter()
                        .filter_map(|observation| observation.neighborhood.as_ref())
                        .map(|report| report.returned_nodes).sum::<usize>(),
                    "truncated_neighborhoods": view.observations.iter()
                        .filter_map(|observation| observation.neighborhood.as_ref())
                        .filter(|report| report.truncated).count(),
                }))?
            );
            Ok(())
        }
        Some("room-load-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("room-load view output already exists: {}", output.display()).into(),
                );
            }
            let shard = NativeEpisodeShard::read(&input)?;
            let view = NativeEpisodeRoomLoadView::build(&shard)?;
            let bytes = view.canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeRoomLoadView)?;
            let present = view
                .observations
                .iter()
                .filter(|observation| observation.status == RoomLoadSetStatus::Present)
                .count();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "observations": view.observations.len(),
                    "present": present,
                    "room_rows": view.observations.iter()
                        .filter_map(|observation| observation.load.as_ref())
                        .map(|load| load.rooms.len()).sum::<usize>(),
                    "active_room_rows": view.observations.iter()
                        .filter_map(|observation| observation.load.as_ref())
                        .flat_map(|load| &load.rooms)
                        .filter(|room| room.status_flags != 0).count(),
                    "live_room_scenes": view.observations.iter()
                        .filter_map(|observation| observation.load.as_ref())
                        .flat_map(|load| &load.rooms)
                        .filter(|room| room.scene_status == RoomSceneSetStatus::Present).count(),
                }))?
            );
            Ok(())
        }
        Some("resource-load-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "resource-load view output already exists: {}",
                    output.display()
                )
                .into());
            }
            let shard = NativeEpisodeShard::read(&input)?;
            let view = NativeEpisodeResourceLoadView::build(&shard)?;
            let bytes = view.canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeResourceLoadView)?;
            let archives = view
                .observations
                .iter()
                .filter_map(|observation| observation.loads.as_ref())
                .flat_map(|loads| &loads.archives)
                .collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "observations": view.observations.len(),
                    "present": view.observations.iter()
                        .filter(|observation| observation.status == ResourceLoadSetStatus::Present)
                        .count(),
                    "archive_rows": archives.len(),
                    "object_rows": archives.iter()
                        .filter(|archive| archive.kind == ResourceArchiveKind::Object).count(),
                    "stage_rows": archives.iter()
                        .filter(|archive| archive.kind == ResourceArchiveKind::Stage).count(),
                    "mounting_rows": archives.iter()
                        .filter(|archive| archive.outcome == ResourceLoadOutcome::Mounting).count(),
                    "ready_rows": archives.iter()
                        .filter(|archive| archive.outcome == ResourceLoadOutcome::Ready).count(),
                    "failed_rows": archives.iter()
                        .filter(|archive| archive.outcome == ResourceLoadOutcome::Failed).count(),
                }))?
            );
            Ok(())
        }
        Some("actor-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let catalog_path = required_path(learn_args, "--actor-profile-catalog")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("actor view output already exists: {}", output.display()).into(),
                );
            }
            let shard = NativeEpisodeShard::read(&input)?;
            let catalog = ActorProfileCatalog::read_canonical(&catalog_path)?;
            let milestones = option(learn_args, "--milestones");
            let milestone_goal = option(learn_args, "--milestone-goal");
            let view = match (milestones, milestone_goal) {
                (None, None) => NativeEpisodeActorView::build(&shard, &catalog)?,
                (Some(program), Some(goal)) => NativeEpisodeActorView::build_for_goal(
                    &shard,
                    &catalog,
                    &fs::read(program)?,
                    &goal,
                )?,
                _ => {
                    return Err(
                        "learn actor-view requires both --milestones and --milestone-goal".into(),
                    );
                }
            };
            let bytes = view.canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeActorView)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "actor_profile_catalog_identity": view.actor_profile_catalog_identity,
                    "actor_profile_catalog_sha256": view.actor_profile_catalog_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "observations": view.observations.len(),
                    "actor_nodes": view.observations.iter()
                        .map(|observation| observation.actors.len()).sum::<usize>(),
                    "camera_frames": view.observations.iter()
                        .filter(|observation| observation.camera_frame_present).count(),
                    "player_frames": view.observations.iter()
                        .filter(|observation| observation.player_present).count(),
                    "parent_relations": view.observations.iter()
                        .flat_map(|observation| &observation.actors)
                        .filter(|actor| actor.parent_relative_position.is_some()).count(),
                    "goal": view.goal_graph.as_ref().map(|graph| &graph.definition_name),
                    "goal_anchors": view.goal_graph.as_ref()
                        .map_or(0, |graph| graph.spatial_anchors().len()),
                    "resolved_goal_anchor_observations": view.observations.iter()
                        .flat_map(|observation| &observation.goal_anchors)
                        .filter(|anchor| anchor.absolute_position.is_some()).count(),
                }))?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}
