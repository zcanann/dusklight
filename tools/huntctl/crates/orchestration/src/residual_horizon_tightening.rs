//! Authenticated post-discovery exploration-horizon tightening.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::optimization_request::{OptimizationHorizonTightening, OptimizationRequest};
use crate::residual_campaign::ResidualCampaignCheckpoint;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
pub use dusklight_search::residual_retention::HorizonSupportPolicy;
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path};

const MAX_BOUND_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

#[allow(clippy::too_many_arguments)]
pub fn tighten_residual_horizon_request(
    parent: &OptimizationRequest,
    source_request: ArtifactReference,
    source_execution: ArtifactReference,
    source_checkpoint: ArtifactReference,
    checkpoint: &ResidualCampaignCheckpoint,
    new_id: String,
    proposed_horizon_ticks: u64,
    policy: HorizonSupportPolicy,
) -> Result<OptimizationRequest, ResidualHorizonTighteningError> {
    parent.validate().map_err(tightening_error)?;
    checkpoint
        .validate(parent, checkpoint.execution_binding_sha256)
        .map_err(tightening_error)?;
    if new_id == parent.id {
        return Err(tightening_message(
            "horizon tightening cannot reuse the parent request ID",
        ));
    }
    let archive = checkpoint.restore_archive().map_err(tightening_error)?;
    let evidence = archive
        .horizon_tightening_evidence(proposed_horizon_ticks, policy)
        .map_err(tightening_error)?;
    let mut child = parent.clone();
    child.id = new_id.clone();
    child.budgets.exploration_horizon_ticks = proposed_horizon_ticks;
    child.budgets.simulated_tick_budget = tightened_tick_budget(parent, proposed_horizon_ticks)?;
    child.resume.state_path = format!("build/campaigns/{new_id}/state.json");
    child.resume.journal_path = format!("build/campaigns/{new_id}/journal.jsonl");
    child.horizon_tightening = Some(OptimizationHorizonTightening {
        source_request,
        source_execution,
        source_checkpoint,
        policy,
        evidence,
    });
    child.refresh_content_sha256().map_err(tightening_error)?;
    child.validate().map_err(tightening_error)?;
    Ok(child)
}

pub(crate) fn validate_horizon_tightening_files(
    root: &Path,
    child: &OptimizationRequest,
    depth: usize,
) -> Result<(), String> {
    let binding = child
        .horizon_tightening
        .as_ref()
        .ok_or_else(|| "horizon-tightened request has no lineage binding".to_string())?;
    let parent: OptimizationRequest = read_bound_json(root, &binding.source_request)?;
    parent
        .validate_files_with_depth(root, depth + 1)
        .map_err(|error| format!("invalid horizon source request: {error}"))?;
    let execution: NativeResidualExecutionBinding =
        read_bound_json(root, &binding.source_execution)?;
    execution
        .validate_files(root, &parent)
        .map_err(|error| format!("invalid horizon source execution: {error}"))?;
    let checkpoint: ResidualCampaignCheckpoint = read_bound_json(root, &binding.source_checkpoint)?;
    checkpoint
        .validate(&parent, execution.content_sha256)
        .map_err(|error| format!("invalid horizon source checkpoint: {error}"))?;
    let expected = tighten_residual_horizon_request(
        &parent,
        binding.source_request.clone(),
        binding.source_execution.clone(),
        binding.source_checkpoint.clone(),
        &checkpoint,
        child.id.clone(),
        child.budgets.exploration_horizon_ticks,
        binding.policy,
    )
    .map_err(|error| error.to_string())?;
    if &expected != child {
        return Err(
            "horizon-tightened request changes fields outside the authenticated delta".into(),
        );
    }
    Ok(())
}

fn tightened_tick_budget(
    parent: &OptimizationRequest,
    proposed_horizon_ticks: u64,
) -> Result<u64, ResidualHorizonTighteningError> {
    let demonstration_ticks = parent
        .incumbent
        .as_ref()
        .ok_or_else(|| tightening_message("horizon tightening requires an incumbent campaign"))?
        .first_hit_tick;
    let terminal_runs = 1_u64
        .checked_add(parent.execution.alternate_terminal_goals.len() as u64)
        .ok_or_else(|| tightening_message("horizon terminal-run count overflowed"))?;
    let derived = parent
        .budgets
        .candidate_budget
        .checked_mul(u64::from(parent.execution.repetitions))
        .and_then(|count| count.checked_mul(terminal_runs))
        .and_then(|count| count.checked_mul(proposed_horizon_ticks))
        .and_then(|ticks| ticks.checked_add(demonstration_ticks))
        .ok_or_else(|| tightening_message("tightened simulated-tick budget overflowed"))?;
    Ok(derived.min(parent.budgets.simulated_tick_budget))
}

fn read_bound_json<T: DeserializeOwned>(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<T, String> {
    let relative = Path::new(&reference.path);
    if reference.sha256 == Digest::ZERO
        || relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("horizon source artifact reference is invalid".into());
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot inspect horizon source artifact: {error}"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_BOUND_ARTIFACT_BYTES
    {
        return Err("horizon source artifact is not a bounded physical file".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("cannot resolve horizon source artifact: {error}"))?;
    if !canonical.starts_with(root) {
        return Err("horizon source artifact escapes the repository".into());
    }
    let bytes = fs::read(canonical)
        .map_err(|error| format!("cannot read horizon source artifact: {error}"))?;
    if Digest(Sha256::digest(&bytes).into()) != reference.sha256 {
        return Err("horizon source artifact digest differs".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("horizon source artifact is invalid JSON: {error}"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidualHorizonTighteningError(String);

fn tightening_message(message: impl Into<String>) -> ResidualHorizonTighteningError {
    ResidualHorizonTighteningError(message.into())
}

fn tightening_error(error: impl fmt::Display) -> ResidualHorizonTighteningError {
    tightening_message(error.to_string())
}

impl fmt::Display for ResidualHorizonTighteningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualHorizonTighteningError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_residual_campaign::materialize_native_residual_process_tape;
    use crate::residual_campaign::ResidualCampaignOptimizer;
    use crate::residual_campaign_runner::new_optimizer;
    use dusklight_automation_contracts::tape::InputTape;
    use dusklight_search::residual_action::{
        AnalogChannel, AnalogResidual, ResidualCandidate, TemporalBasis,
        compile_residual_candidate_to_horizon,
    };
    use dusklight_search::residual_retention::{
        ExactTerminalVerdict, ResidualEvaluationEvidence, ResidualOutcomeArchive,
    };

    fn repository() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap()
    }

    struct TestArtifacts(std::path::PathBuf);

    impl TestArtifacts {
        fn new(root: &Path) -> Self {
            let path = root.join("build").join(format!(
                "horizon-tightening-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestArtifacts {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn reference(root: &Path, path: &Path) -> ArtifactReference {
        let bytes = fs::read(path).unwrap();
        ArtifactReference {
            path: path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace(std::path::MAIN_SEPARATOR, "/"),
            sha256: Digest(Sha256::digest(bytes).into()),
        }
    }

    fn source() -> (
        OptimizationRequest,
        ResidualCampaignCheckpoint,
        ArtifactReference,
        ArtifactReference,
        ArtifactReference,
    ) {
        let root = repository();
        let parent: OptimizationRequest = serde_json::from_slice(
            &fs::read(root.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let incumbent = parent.incumbent.as_ref().unwrap();
        let parent_bytes = fs::read(root.join(&incumbent.tape.path)).unwrap();
        let parent_tape = InputTape::decode(&parent_bytes).unwrap().tape;
        let mut archive =
            ResidualOutcomeArchive::new(parent.residual_retention_config().unwrap()).unwrap();
        for (index, first_hit_tick) in [130_u64, 135, 150].into_iter().enumerate() {
            let candidate = ResidualCandidate::seal(
                &parent_bytes,
                vec![AnalogResidual {
                    port: 0,
                    channel: AnalogChannel::MainX,
                    basis: TemporalBasis::ExactFrame {
                        frame: index as u64 + 1,
                        delta: 4 + index as i16,
                    },
                }],
                vec![],
            )
            .unwrap();
            let compiled = compile_residual_candidate_to_horizon(
                &parent_tape,
                &parent_bytes,
                &candidate,
                parent.budgets.exploration_horizon_ticks,
            )
            .unwrap();
            archive
                .record(
                    &compiled,
                    ResidualEvaluationEvidence {
                        candidate_sha256: candidate.content_sha256,
                        realized_tape_sha256: compiled.report.realized_tape_sha256,
                        terminal_program_sha256: parent.terminal_predicate.program_sha256,
                        terminal_definition_sha256: parent.terminal_predicate.definition_sha256,
                        evaluation_sha256: Digest([10 + index as u8; 32]),
                        episode_sha256: Digest([20 + index as u8; 32]),
                        behavior_sha256: Digest([30 + index as u8; 32]),
                        verdict: ExactTerminalVerdict::Reached { first_hit_tick },
                        shaped_progress_millionths: None,
                        native_risk_events: None,
                    },
                )
                .unwrap();
        }
        let optimizer: ResidualCampaignOptimizer = new_optimizer(&parent, &parent_bytes).unwrap();
        let execution_sha256 = Digest([41; 32]);
        let checkpoint = ResidualCampaignCheckpoint::seal(
            &parent,
            execution_sha256,
            1,
            3,
            optimizer.snapshot().unwrap(),
            &archive,
            None,
        )
        .unwrap();
        let request_ref = ArtifactReference {
            path: "routes/source.request.json".into(),
            sha256: parent.content_sha256,
        };
        let execution_ref = ArtifactReference {
            path: "build/campaigns/source/execution.json".into(),
            sha256: execution_sha256,
        };
        let checkpoint_ref = ArtifactReference {
            path: "build/campaigns/source/checkpoint.json".into(),
            sha256: checkpoint.content_sha256,
        };
        (
            parent,
            checkpoint,
            request_ref,
            execution_ref,
            checkpoint_ref,
        )
    }

    #[test]
    fn supported_basin_produces_only_the_exact_tightened_request_delta() {
        let (parent, checkpoint, request_ref, execution_ref, checkpoint_ref) = source();
        let child = tighten_residual_horizon_request(
            &parent,
            request_ref,
            execution_ref,
            checkpoint_ref,
            &checkpoint,
            "ordon-q125-residual-cem-h140".into(),
            140,
            HorizonSupportPolicy {
                minimum_successes: 2,
                minimum_behavior_classes: 2,
                minimum_support_millionths: 600_000,
            },
        )
        .unwrap();
        child.validate().unwrap();
        assert_eq!(child.budgets.exploration_horizon_ticks, 140);
        assert_eq!(child.budgets.simulated_tick_budget, 1_147_005);
        assert_eq!(
            child.resume.state_path,
            "build/campaigns/ordon-q125-residual-cem-h140/state.json"
        );
        let evidence = &child.horizon_tightening.as_ref().unwrap().evidence;
        assert_eq!(evidence.retained_successes, 3);
        assert_eq!(evidence.supporting_successes, 2);
        assert_eq!(evidence.supporting_behavior_classes, 2);
        assert_eq!(evidence.support_millionths, 666_666);

        let mut normalized = child.clone();
        normalized.id = parent.id.clone();
        normalized.budgets = parent.budgets.clone();
        normalized.resume = parent.resume.clone();
        normalized.horizon_tightening = parent.horizon_tightening.clone();
        normalized.content_sha256 = parent.content_sha256;
        assert_eq!(normalized, parent);
    }

    #[test]
    fn unsupported_or_detached_tightening_fails_closed() {
        let (parent, checkpoint, mut request_ref, execution_ref, checkpoint_ref) = source();
        request_ref.sha256 = Digest::ZERO;
        assert!(
            tighten_residual_horizon_request(
                &parent,
                request_ref,
                execution_ref.clone(),
                checkpoint_ref.clone(),
                &checkpoint,
                "detached-horizon".into(),
                140,
                HorizonSupportPolicy {
                    minimum_successes: 2,
                    minimum_behavior_classes: 2,
                    minimum_support_millionths: 600_000,
                },
            )
            .is_err()
        );
        assert!(
            tighten_residual_horizon_request(
                &parent,
                ArtifactReference {
                    path: "routes/source.request.json".into(),
                    sha256: parent.content_sha256,
                },
                execution_ref,
                checkpoint_ref,
                &checkpoint,
                "unsupported-horizon".into(),
                140,
                HorizonSupportPolicy {
                    minimum_successes: 3,
                    minimum_behavior_classes: 3,
                    minimum_support_millionths: 1_000_000,
                },
            )
            .is_err()
        );
    }

    #[test]
    fn file_validation_recomputes_the_parent_checkpoint_and_exact_delta() {
        let root = repository();
        let artifacts = TestArtifacts::new(&root);
        let (parent, source_checkpoint, _, _, _) = source();
        let executable = artifacts.0.join("Dusklight");
        let game_data = artifacts.0.join("game.iso");
        let tape_path = artifacts.0.join("process-route.tape");
        let program_path = artifacts.0.join("terminal.dmsp");
        let world_path = artifacts.0.join("world.json");
        fs::write(&executable, b"executable").unwrap();
        fs::write(&game_data, b"game-data").unwrap();
        fs::write(
            &world_path,
            serde_json::to_vec(&serde_json::json!({
                "schema": "dusklight-world-context/v1",
                "game_data_sha256": Digest(Sha256::digest(b"game-data").into()),
                "stages": []
            }))
            .unwrap(),
        )
        .unwrap();
        let tape = materialize_native_residual_process_tape(&root, &parent).unwrap();
        fs::write(&tape_path, tape.encode().unwrap()).unwrap();
        let source_text =
            fs::read_to_string(root.join(&parent.terminal_predicate.source.path)).unwrap();
        let program = dusklight_objectives::milestone_dsl::parse(&source_text).unwrap();
        let compiled = dusklight_objectives::milestone_dsl::compile(&program).unwrap();
        fs::write(&program_path, compiled.bytes).unwrap();
        let execution = NativeResidualExecutionBinding::seal(
            &root,
            &parent,
            &executable,
            &game_data,
            &tape_path,
            &program_path,
            &world_path,
            &root.join("routes/Glitch Exhibition/intro/benchmarks/process_boot.fixture.json"),
            8,
            false,
        )
        .unwrap();
        let execution_path = artifacts.0.join("execution.json");
        fs::write(&execution_path, execution.to_pretty_json().unwrap()).unwrap();
        let archive = source_checkpoint.restore_archive().unwrap();
        let checkpoint = ResidualCampaignCheckpoint::seal(
            &parent,
            execution.content_sha256,
            source_checkpoint.generation,
            source_checkpoint.completed_candidates,
            source_checkpoint.optimizer.clone(),
            &archive,
            None,
        )
        .unwrap();
        let checkpoint_path = artifacts.0.join("checkpoint.json");
        fs::write(&checkpoint_path, checkpoint.to_pretty_json().unwrap()).unwrap();
        let request_path = root.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        );
        let child = tighten_residual_horizon_request(
            &parent,
            reference(&root, &request_path),
            reference(&root, &execution_path),
            reference(&root, &checkpoint_path),
            &checkpoint,
            "ordon-q125-residual-cem-h140-validated".into(),
            140,
            HorizonSupportPolicy {
                minimum_successes: 2,
                minimum_behavior_classes: 2,
                minimum_support_millionths: 600_000,
            },
        )
        .unwrap();
        child.validate_files(&root).unwrap();

        let mut tampered = child;
        tampered
            .horizon_tightening
            .as_mut()
            .unwrap()
            .evidence
            .supporting_successes = 3;
        tampered.refresh_content_sha256().unwrap();
        tampered.validate().unwrap();
        assert!(tampered.validate_files(&root).is_err());
    }
}
