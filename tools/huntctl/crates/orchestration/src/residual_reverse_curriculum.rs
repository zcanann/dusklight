//! Authenticated reverse-curriculum request derivation for residual campaigns.
//!
//! A curriculum never invents a coordinate corridor or synthetic state. It
//! keeps the route-proved incumbent prefix exact, initially opens a bounded
//! terminal action window, and moves that window backward by one deterministic
//! step only after exact native successes support the current frontier.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::optimization_request::{OptimizationRequest, OptimizationReverseCurriculum};
use crate::residual_campaign::ResidualCampaignCheckpoint;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_search::residual_retention::ResidualOutcomeArchive;
pub use dusklight_search::residual_retention::ReverseCurriculumSupportPolicy;
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path};

const MAX_BOUND_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

pub fn seed_residual_reverse_curriculum_request(
    parent: &OptimizationRequest,
    source_request: ArtifactReference,
    new_id: String,
    policy: ReverseCurriculumSupportPolicy,
) -> Result<OptimizationRequest, ResidualReverseCurriculumError> {
    parent.validate().map_err(curriculum_error)?;
    validate_reference(&source_request)?;
    if new_id == parent.id || parent.reverse_curriculum.is_some() {
        return Err(curriculum_message(
            "reverse curriculum seed requires a fresh ID and a non-curriculum source",
        ));
    }
    if parent.incumbent.is_none() {
        return Err(curriculum_message(
            "reverse curriculum requires a route-proved incumbent trajectory",
        ));
    }
    validate_policy(policy)?;
    let root_start_frame = parent.proposal.search_space.start_frame;
    let terminal_end_frame_exclusive = parent.proposal.search_space.end_frame_exclusive;
    let width = terminal_end_frame_exclusive
        .checked_sub(root_start_frame)
        .ok_or_else(|| curriculum_message("reverse curriculum source window is invalid"))?;
    if policy.initial_tail_ticks >= width {
        return Err(curriculum_message(
            "reverse curriculum terminal window must be narrower than its source window",
        ));
    }
    let start_frame = terminal_end_frame_exclusive - policy.initial_tail_ticks;
    let mut child = parent.clone();
    child.id = new_id.clone();
    child.proposal.search_space.start_frame = start_frame;
    child.resume.state_path = format!("build/campaigns/{new_id}/state.json");
    child.resume.journal_path = format!("build/campaigns/{new_id}/journal.jsonl");
    // The source reference carries the complete prior lineage. Keep exactly one
    // active derivation edge on each request so independent delta validators do
    // not compete for authority over the same child.
    child.horizon_tightening = None;
    child.reverse_curriculum = Some(OptimizationReverseCurriculum {
        source_request,
        source_execution: None,
        source_checkpoint: None,
        generation: 0,
        root_start_frame,
        terminal_end_frame_exclusive,
        policy,
        evidence: None,
    });
    child.refresh_content_sha256().map_err(curriculum_error)?;
    child.validate().map_err(curriculum_error)?;
    Ok(child)
}

#[allow(clippy::too_many_arguments)]
pub fn expand_residual_reverse_curriculum_request(
    parent: &OptimizationRequest,
    source_request: ArtifactReference,
    source_execution: ArtifactReference,
    source_checkpoint: ArtifactReference,
    checkpoint: &ResidualCampaignCheckpoint,
    incumbent_tape: &InputTape,
    incumbent_tape_bytes: &[u8],
    new_id: String,
) -> Result<OptimizationRequest, ResidualReverseCurriculumError> {
    parent.validate().map_err(curriculum_error)?;
    for reference in [&source_request, &source_execution, &source_checkpoint] {
        validate_reference(reference)?;
    }
    let lineage = parent.reverse_curriculum.as_ref().ok_or_else(|| {
        curriculum_message("reverse curriculum expansion requires a seeded parent")
    })?;
    checkpoint
        .validate(parent, checkpoint.execution_binding_sha256)
        .map_err(curriculum_error)?;
    if new_id == parent.id
        || source_checkpoint.sha256 == Digest::ZERO
        || parent.proposal.search_space.start_frame <= lineage.root_start_frame
    {
        return Err(curriculum_message(
            "reverse curriculum cannot expand past its root or reuse its parent ID",
        ));
    }
    let proposed_start_frame = parent
        .proposal
        .search_space
        .start_frame
        .saturating_sub(lineage.policy.expansion_step_ticks)
        .max(lineage.root_start_frame);
    let archive = ResidualOutcomeArchive::restore_after_validation(checkpoint.retention.clone());
    let evidence = archive
        .reverse_curriculum_evidence(
            incumbent_tape,
            incumbent_tape_bytes,
            parent.proposal.search_space.start_frame,
            proposed_start_frame,
            lineage.policy,
        )
        .map_err(curriculum_error)?;
    let generation = lineage
        .generation
        .checked_add(1)
        .ok_or_else(|| curriculum_message("reverse curriculum generation overflowed"))?;
    let mut child = parent.clone();
    child.id = new_id.clone();
    child.proposal.search_space.start_frame = proposed_start_frame;
    child.resume.state_path = format!("build/campaigns/{new_id}/state.json");
    child.resume.journal_path = format!("build/campaigns/{new_id}/journal.jsonl");
    child.horizon_tightening = None;
    child.reverse_curriculum = Some(OptimizationReverseCurriculum {
        source_request,
        source_execution: Some(source_execution),
        source_checkpoint: Some(source_checkpoint),
        generation,
        root_start_frame: lineage.root_start_frame,
        terminal_end_frame_exclusive: lineage.terminal_end_frame_exclusive,
        policy: lineage.policy,
        evidence: Some(evidence),
    });
    child.refresh_content_sha256().map_err(curriculum_error)?;
    child.validate().map_err(curriculum_error)?;
    Ok(child)
}

pub(crate) fn validate_reverse_curriculum_files(
    root: &Path,
    child: &OptimizationRequest,
    depth: usize,
) -> Result<(), String> {
    let binding = child
        .reverse_curriculum
        .as_ref()
        .ok_or_else(|| "reverse-curriculum request has no lineage binding".to_string())?;
    let parent: OptimizationRequest = read_bound_json(root, &binding.source_request)?;
    parent
        .validate_files_with_depth(root, depth + 1)
        .map_err(|error| format!("invalid curriculum source request: {error}"))?;
    let expected = if binding.generation == 0 {
        seed_residual_reverse_curriculum_request(
            &parent,
            binding.source_request.clone(),
            child.id.clone(),
            binding.policy,
        )
        .map_err(|error| error.to_string())?
    } else {
        let execution_reference = binding.source_execution.as_ref().ok_or_else(|| {
            "reverse-curriculum expansion lacks its execution binding".to_string()
        })?;
        let checkpoint_reference = binding.source_checkpoint.as_ref().ok_or_else(|| {
            "reverse-curriculum expansion lacks its checkpoint binding".to_string()
        })?;
        let execution: NativeResidualExecutionBinding = read_bound_json(root, execution_reference)?;
        execution
            .validate_seal(&parent)
            .map_err(|error| format!("invalid curriculum source execution: {error}"))?;
        let checkpoint: ResidualCampaignCheckpoint = read_bound_json(root, checkpoint_reference)?;
        let incumbent = parent
            .incumbent
            .as_ref()
            .ok_or_else(|| "reverse-curriculum source has no incumbent trajectory".to_string())?;
        let incumbent_bytes = read_bound_bytes(root, &incumbent.tape)?;
        let incumbent_tape = InputTape::decode(&incumbent_bytes)
            .map_err(|error| format!("invalid curriculum incumbent tape: {error}"))?
            .tape;
        expand_residual_reverse_curriculum_request(
            &parent,
            binding.source_request.clone(),
            execution_reference.clone(),
            checkpoint_reference.clone(),
            &checkpoint,
            &incumbent_tape,
            &incumbent_bytes,
            child.id.clone(),
        )
        .map_err(|error| error.to_string())?
    };
    if &expected != child {
        return Err(
            "reverse-curriculum request changes fields outside the authenticated delta".into(),
        );
    }
    Ok(())
}

fn validate_policy(
    policy: ReverseCurriculumSupportPolicy,
) -> Result<(), ResidualReverseCurriculumError> {
    if policy.initial_tail_ticks < 32
        || policy.expansion_step_ticks == 0
        || policy.minimum_successes < 2
        || policy.minimum_behavior_classes < 2
        || policy.minimum_behavior_classes > policy.minimum_successes
        || policy.minimum_success_millionths == 0
        || policy.minimum_success_millionths > 1_000_000
    {
        return Err(curriculum_message(
            "reverse curriculum policy requires a bounded terminal window and several exact continuations",
        ));
    }
    Ok(())
}

fn validate_reference(reference: &ArtifactReference) -> Result<(), ResidualReverseCurriculumError> {
    let path = Path::new(&reference.path);
    if reference.sha256 == Digest::ZERO
        || path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(curriculum_message(
            "reverse curriculum source artifact reference is invalid",
        ));
    }
    Ok(())
}

fn read_bound_json<T: DeserializeOwned>(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<T, String> {
    let bytes = read_bound_bytes(root, reference)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("curriculum source artifact is invalid JSON: {error}"))
}

fn read_bound_bytes(root: &Path, reference: &ArtifactReference) -> Result<Vec<u8>, String> {
    let relative = Path::new(&reference.path);
    if reference.sha256 == Digest::ZERO
        || relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("curriculum source artifact reference is invalid".into());
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot inspect curriculum source artifact: {error}"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_BOUND_ARTIFACT_BYTES
    {
        return Err("curriculum source artifact is not a bounded physical file".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("cannot resolve curriculum source artifact: {error}"))?;
    if !canonical.starts_with(root) {
        return Err("curriculum source artifact escapes the repository".into());
    }
    let bytes = fs::read(canonical)
        .map_err(|error| format!("cannot read curriculum source artifact: {error}"))?;
    if Digest(Sha256::digest(&bytes).into()) != reference.sha256 {
        return Err("curriculum source artifact digest differs".into());
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidualReverseCurriculumError(String);

fn curriculum_message(message: impl Into<String>) -> ResidualReverseCurriculumError {
    ResidualReverseCurriculumError(message.into())
}

fn curriculum_error(error: impl fmt::Display) -> ResidualReverseCurriculumError {
    curriculum_message(error.to_string())
}

impl fmt::Display for ResidualReverseCurriculumError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualReverseCurriculumError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_residual_campaign::{
        NativeResidualExecutionBinding, materialize_native_residual_process_tape,
    };
    use crate::residual_campaign::{ResidualCampaignCheckpoint, ResidualCampaignOptimizer};
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
                "reverse-curriculum-test-{}-{}",
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

    fn parent() -> OptimizationRequest {
        let root = repository();
        serde_json::from_slice(
            &fs::read(root.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap()
    }

    fn reference(root: &Path, path: &Path) -> ArtifactReference {
        ArtifactReference {
            path: path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
            sha256: Digest(Sha256::digest(fs::read(path).unwrap()).into()),
        }
    }

    fn policy() -> ReverseCurriculumSupportPolicy {
        ReverseCurriculumSupportPolicy {
            initial_tail_ticks: 64,
            expansion_step_ticks: 16,
            minimum_successes: 3,
            minimum_behavior_classes: 3,
            minimum_success_millionths: 500_000,
        }
    }

    fn checkpoint_with_first_edit(
        seed: &OptimizationRequest,
        first_edit_frame: u64,
    ) -> ResidualCampaignCheckpoint {
        let root = repository();
        let incumbent = seed.incumbent.as_ref().unwrap();
        let parent_bytes = fs::read(root.join(&incumbent.tape.path)).unwrap();
        let parent_tape = InputTape::decode(&parent_bytes).unwrap().tape;
        let mut archive =
            ResidualOutcomeArchive::new(seed.residual_retention_config().unwrap()).unwrap();
        for (index, verdict) in [
            ExactTerminalVerdict::Reached {
                first_hit_tick: 124,
            },
            ExactTerminalVerdict::Reached {
                first_hit_tick: 125,
            },
            ExactTerminalVerdict::Reached {
                first_hit_tick: 126,
            },
            ExactTerminalVerdict::Miss,
        ]
        .into_iter()
        .enumerate()
        {
            let frame = first_edit_frame + index as u64;
            let current = parent_tape.frames[frame as usize].pads[0].stick_x;
            let magnitude = 4 + index as i16;
            let delta = if current >= 0 { -magnitude } else { magnitude };
            let candidate = ResidualCandidate::seal(
                &parent_bytes,
                vec![AnalogResidual {
                    port: 0,
                    channel: AnalogChannel::MainX,
                    basis: TemporalBasis::ExactFrame { frame, delta },
                }],
                vec![],
            )
            .unwrap();
            let compiled = compile_residual_candidate_to_horizon(
                &parent_tape,
                &parent_bytes,
                &candidate,
                seed.budgets.exploration_horizon_ticks,
            )
            .unwrap();
            archive
                .record(
                    &compiled,
                    ResidualEvaluationEvidence {
                        candidate_sha256: candidate.content_sha256,
                        realized_tape_sha256: compiled.report.realized_tape_sha256,
                        terminal_program_sha256: seed.terminal_predicate.program_sha256,
                        terminal_definition_sha256: seed.terminal_predicate.definition_sha256,
                        evaluation_sha256: Digest([10 + index as u8; 32]),
                        episode_sha256: Digest([20 + index as u8; 32]),
                        behavior_sha256: Digest([30 + index as u8; 32]),
                        verdict,
                        shaped_progress_millionths: None,
                        native_risk_events: None,
                    },
                )
                .unwrap();
        }
        let optimizer: ResidualCampaignOptimizer = new_optimizer(seed, &parent_bytes).unwrap();
        ResidualCampaignCheckpoint::seal(
            seed,
            Digest([41; 32]),
            1,
            4,
            optimizer.snapshot().unwrap(),
            &archive,
            None,
        )
        .unwrap()
    }

    fn supported_checkpoint(seed: &OptimizationRequest) -> ResidualCampaignCheckpoint {
        checkpoint_with_first_edit(seed, seed.proposal.search_space.start_frame)
    }

    #[test]
    fn seed_uses_only_the_derived_terminal_window_and_validates_files() {
        let root = repository();
        let source = parent();
        let request_path = root.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        );
        let seed = seed_residual_reverse_curriculum_request(
            &source,
            reference(&root, &request_path),
            "ordon-q125-reverse-curriculum-g0".into(),
            policy(),
        )
        .unwrap();
        assert_eq!(seed.proposal.search_space.start_frame, 62);
        assert_eq!(seed.proposal.search_space.end_frame_exclusive, 126);
        assert_eq!(seed.reverse_curriculum.as_ref().unwrap().generation, 0);
        seed.validate_files(&root).unwrap();

        let mut detached = seed;
        detached.proposal.search_space.start_frame += 1;
        detached.refresh_content_sha256().unwrap();
        assert!(detached.validate().is_err());
    }

    #[test]
    fn expansion_requires_exact_basin_support_and_moves_one_step_backward() {
        let root = repository();
        let source = parent();
        let request_path = root.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        );
        let source_reference = reference(&root, &request_path);
        let seed = seed_residual_reverse_curriculum_request(
            &source,
            source_reference,
            "ordon-q125-reverse-curriculum-g0".into(),
            policy(),
        )
        .unwrap();
        let checkpoint = supported_checkpoint(&seed);
        let incumbent = seed.incumbent.as_ref().unwrap();
        let incumbent_bytes = fs::read(root.join(&incumbent.tape.path)).unwrap();
        let incumbent_tape = InputTape::decode(&incumbent_bytes).unwrap().tape;
        let child = expand_residual_reverse_curriculum_request(
            &seed,
            ArtifactReference {
                path: "build/curriculum/g0.request.json".into(),
                sha256: Digest([1; 32]),
            },
            ArtifactReference {
                path: "build/curriculum/g0.execution.json".into(),
                sha256: Digest([2; 32]),
            },
            ArtifactReference {
                path: "build/curriculum/g0.checkpoint.json".into(),
                sha256: Digest([3; 32]),
            },
            &checkpoint,
            &incumbent_tape,
            &incumbent_bytes,
            "ordon-q125-reverse-curriculum-g1".into(),
        )
        .unwrap();
        assert_eq!(child.proposal.search_space.start_frame, 46);
        let lineage = child.reverse_curriculum.as_ref().unwrap();
        assert_eq!(lineage.generation, 1);
        let evidence = lineage.evidence.as_ref().unwrap();
        assert_eq!(evidence.evaluated_tapes, 4);
        assert_eq!(evidence.successful_tapes, 3);
        assert_eq!(evidence.successful_behavior_classes, 3);
        assert_eq!(evidence.success_millionths, 750_000);

        let prefix_detached = checkpoint_with_first_edit(&seed, 0);
        assert!(
            expand_residual_reverse_curriculum_request(
                &seed,
                ArtifactReference {
                    path: "build/curriculum/g0.request.json".into(),
                    sha256: Digest([1; 32]),
                },
                ArtifactReference {
                    path: "build/curriculum/g0.execution.json".into(),
                    sha256: Digest([2; 32]),
                },
                ArtifactReference {
                    path: "build/curriculum/g0.checkpoint.json".into(),
                    sha256: Digest([3; 32]),
                },
                &prefix_detached,
                &incumbent_tape,
                &incumbent_bytes,
                "ordon-q125-reverse-curriculum-prefix-detached".into(),
            )
            .is_err()
        );

        let mut unsupported = seed;
        unsupported
            .reverse_curriculum
            .as_mut()
            .unwrap()
            .policy
            .minimum_successes = 4;
        unsupported.refresh_content_sha256().unwrap();
        let checkpoint = supported_checkpoint(&unsupported);
        assert!(
            expand_residual_reverse_curriculum_request(
                &unsupported,
                ArtifactReference {
                    path: "build/curriculum/g0.request.json".into(),
                    sha256: Digest([1; 32]),
                },
                ArtifactReference {
                    path: "build/curriculum/g0.execution.json".into(),
                    sha256: Digest([2; 32]),
                },
                ArtifactReference {
                    path: "build/curriculum/g0.checkpoint.json".into(),
                    sha256: Digest([3; 32]),
                },
                &checkpoint,
                &incumbent_tape,
                &incumbent_bytes,
                "ordon-q125-reverse-curriculum-unsupported".into(),
            )
            .is_err()
        );
    }

    #[test]
    fn file_validation_rebuilds_expansion_and_rejects_resealed_support_drift() {
        let root = repository();
        let artifacts = TestArtifacts::new(&root);
        let source = parent();
        let checked_request_path = root.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        );
        let seed = seed_residual_reverse_curriculum_request(
            &source,
            reference(&root, &checked_request_path),
            "ordon-q125-reverse-curriculum-file-g0".into(),
            policy(),
        )
        .unwrap();
        let seed_path = artifacts.0.join("seed.request.json");
        fs::write(&seed_path, seed.to_pretty_json().unwrap()).unwrap();

        let executable = artifacts.0.join("Dusklight");
        let game_data = artifacts.0.join("game.iso");
        let process_tape = artifacts.0.join("process-route.tape");
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
        let tape = materialize_native_residual_process_tape(&root, &seed).unwrap();
        fs::write(&process_tape, tape.encode().unwrap()).unwrap();
        let source_text =
            fs::read_to_string(root.join(&seed.terminal_predicate.source.path)).unwrap();
        let program = dusklight_objectives::milestone_dsl::parse(&source_text).unwrap();
        let compiled = dusklight_objectives::milestone_dsl::compile(&program).unwrap();
        fs::write(&program_path, compiled.bytes).unwrap();
        let execution = NativeResidualExecutionBinding::seal(
            &root,
            &seed,
            &executable,
            &game_data,
            &process_tape,
            &program_path,
            &world_path,
            &root.join("routes/Glitch Exhibition/intro/benchmarks/process_boot.fixture.json"),
            8,
            false,
        )
        .unwrap();
        let execution_path = artifacts.0.join("execution.json");
        fs::write(&execution_path, execution.to_pretty_json().unwrap()).unwrap();

        let source_checkpoint = supported_checkpoint(&seed);
        let archive = source_checkpoint.restore_archive().unwrap();
        let checkpoint = ResidualCampaignCheckpoint::seal(
            &seed,
            execution.content_sha256,
            source_checkpoint.generation,
            source_checkpoint.completed_candidates,
            source_checkpoint.optimizer,
            &archive,
            None,
        )
        .unwrap();
        let checkpoint_path = artifacts.0.join("checkpoint.json");
        fs::write(&checkpoint_path, checkpoint.to_pretty_json().unwrap()).unwrap();
        let incumbent = seed.incumbent.as_ref().unwrap();
        let incumbent_bytes = fs::read(root.join(&incumbent.tape.path)).unwrap();
        let incumbent_tape = InputTape::decode(&incumbent_bytes).unwrap().tape;

        let child = expand_residual_reverse_curriculum_request(
            &seed,
            reference(&root, &seed_path),
            reference(&root, &execution_path),
            reference(&root, &checkpoint_path),
            &checkpoint,
            &incumbent_tape,
            &incumbent_bytes,
            "ordon-q125-reverse-curriculum-file-g1".into(),
        )
        .unwrap();
        child.validate_files(&root).unwrap();

        let mut detached = child;
        let evidence = detached
            .reverse_curriculum
            .as_mut()
            .unwrap()
            .evidence
            .as_mut()
            .unwrap();
        evidence.evaluated_tapes = 5;
        evidence.successful_tapes = 4;
        evidence.success_millionths = 800_000;
        detached.refresh_content_sha256().unwrap();
        detached.validate().unwrap();
        assert!(detached.validate_files(&root).is_err());
    }
}
