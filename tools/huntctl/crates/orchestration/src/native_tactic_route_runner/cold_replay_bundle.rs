//! Portable evidence bundle for an exact tactic-route cold replay.

use super::cold_replay::{read_proof_artifact, validate_native_tactic_cold_replay_artifacts};
use super::scratch_discovery::route_report_sha256;
use super::scratch_evidence_bundle::{blob_path, read_blob, read_json_blob};
use super::*;
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::tactic_q_campaign::TacticQFinalResult;
use sha2::Sha256;
use std::collections::{BTreeSet, VecDeque};
use std::path::Component;

pub const NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_BUNDLE_SCHEMA_V1: &str =
    "dusklight-native-tactic-cold-replay-evidence-bundle/v1";
pub const NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_MANIFEST: &str = "manifest.json";
const CAMPAIGN_DIRECTORY: &str = "campaign";
const COLD_REPLAY_DIRECTORY: &str = "cold-replay";
const MAXIMUM_BUNDLE_TREE_ENTRIES: usize = 100_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticColdReplayEvidenceBundle {
    pub schema: String,
    pub content_sha256: Digest,
    pub scratch_bundle_sha256: Digest,
    pub cold_replay_proof_sha256: Digest,
    pub seed: u64,
    pub state_graph_sha256: Digest,
    pub terminal_result_sha256: Digest,
    pub controller_tape_sha256: Digest,
    pub first_hit_tick: u64,
    pub repetitions: u32,
    pub passed: bool,
}

/// Fully validated route authority retained by a portable cold-replay bundle.
///
/// The manifest remains compact, while this view exposes the exact proof and
/// controller tape needed to derive another authenticated campaign.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeTacticColdReplayAuthority {
    pub manifest: NativeTacticColdReplayEvidenceBundle,
    pub proof: NativeTacticColdReplayProof,
    pub controller_tape: InputTape,
}

impl NativeTacticColdReplayAuthority {
    /// Extract the exact source-boundary-to-terminal tape used as a residual
    /// optimization incumbent. Cold replay retains the complete named-root
    /// tape, while residual execution restores the authenticated source
    /// boundary before applying this local input program.
    pub fn incumbent_tape(&self) -> Result<InputTape, NativeTacticRouteRunError> {
        let start = usize::try_from(self.proof.source_boundary_index).map_err(route_error)?;
        let local_frames = usize::try_from(self.proof.first_hit_tick)
            .map_err(route_error)?
            .checked_add(1)
            .ok_or_else(|| route_message("optimization authority route length overflowed"))?;
        let end = start
            .checked_add(local_frames)
            .ok_or_else(|| route_message("optimization authority route length overflowed"))?;
        let frames = self
            .controller_tape
            .frames
            .get(start..end)
            .ok_or_else(|| route_message("optimization authority tape is shorter than its proof"))?
            .to_vec();
        if end != self.controller_tape.frames.len() {
            return Err(route_message(
                "optimization authority tape extends beyond its first terminal boundary",
            ));
        }
        Ok(InputTape {
            boot: self.controller_tape.boot.clone(),
            tick_rate_numerator: self.controller_tape.tick_rate_numerator,
            tick_rate_denominator: self.controller_tape.tick_rate_denominator,
            frames,
        })
    }
}

impl NativeTacticColdReplayEvidenceBundle {
    pub fn build(
        bundle_root: &Path,
        scratch_bundle_root: &Path,
        cold_replay_root: &Path,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if bundle_root.exists() {
            return Err(route_message(
                "native tactic cold replay evidence bundle output already exists",
            ));
        }
        let scratch_bundle_root = scratch_bundle_root.canonicalize().map_err(route_error)?;
        let cold_replay_root = cold_replay_root.canonicalize().map_err(route_error)?;
        let output = resolved_new_directory(bundle_root)?;
        if output.starts_with(&scratch_bundle_root) || output.starts_with(&cold_replay_root) {
            return Err(route_message(
                "cold replay evidence output cannot be nested inside an input bundle",
            ));
        }
        let (scratch, proof) = validate_pair(&scratch_bundle_root, &cold_replay_root)?;
        fs::create_dir(&output).map_err(route_error)?;
        let mut guard = ColdReplayBundleBuildGuard::new(output.clone());
        copy_physical_tree(&scratch_bundle_root, &output.join(CAMPAIGN_DIRECTORY))?;
        copy_cold_replay_evidence(
            &cold_replay_root,
            &output.join(COLD_REPLAY_DIRECTORY),
            &proof,
        )?;
        let mut bundle = Self {
            schema: NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_BUNDLE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            scratch_bundle_sha256: scratch.content_sha256,
            cold_replay_proof_sha256: proof.content_sha256,
            seed: proof.seed,
            state_graph_sha256: proof.state_graph_sha256,
            terminal_result_sha256: proof.terminal_result_sha256,
            controller_tape_sha256: proof.controller_tape.sha256,
            first_hit_tick: proof.first_hit_tick,
            repetitions: u32::try_from(proof.attempts.len()).map_err(route_error)?,
            passed: true,
        };
        bundle.content_sha256 = bundle.identity()?;
        write_new(
            &output.join(NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_MANIFEST),
            &bundle.to_pretty_json()?,
        )?;
        let retained = Self::read_and_validate(&output)?;
        if retained != bundle {
            return Err(route_message(
                "retained cold replay evidence differs after publication",
            ));
        }
        guard.commit();
        Ok(bundle)
    }

    pub fn read_and_validate(bundle_root: &Path) -> Result<Self, NativeTacticRouteRunError> {
        Ok(Self::read_authority(bundle_root)?.manifest)
    }

    pub fn read_authority(
        bundle_root: &Path,
    ) -> Result<NativeTacticColdReplayAuthority, NativeTacticRouteRunError> {
        let bundle: Self =
            read_bounded_json(&bundle_root.join(NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_MANIFEST))?;
        bundle.validate_shape()?;
        let (scratch, proof) = validate_pair(
            &bundle_root.join(CAMPAIGN_DIRECTORY),
            &bundle_root.join(COLD_REPLAY_DIRECTORY),
        )?;
        if bundle.scratch_bundle_sha256 != scratch.content_sha256
            || bundle.cold_replay_proof_sha256 != proof.content_sha256
            || bundle.seed != proof.seed
            || bundle.state_graph_sha256 != proof.state_graph_sha256
            || bundle.terminal_result_sha256 != proof.terminal_result_sha256
            || bundle.controller_tape_sha256 != proof.controller_tape.sha256
            || bundle.first_hit_tick != proof.first_hit_tick
            || usize::try_from(bundle.repetitions).map_err(route_error)? != proof.attempts.len()
        {
            return Err(route_message(
                "cold replay evidence bundle is detached from its retained campaign or proof",
            ));
        }
        let controller_tape_bytes = read_proof_artifact(
            &bundle_root.join(COLD_REPLAY_DIRECTORY),
            &proof.controller_tape,
        )?;
        let controller_tape = InputTape::decode(&controller_tape_bytes)
            .map_err(route_error)?
            .tape;
        Ok(NativeTacticColdReplayAuthority {
            manifest: bundle,
            proof,
            controller_tape,
        })
    }

    fn validate_shape(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_BUNDLE_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
            || self.scratch_bundle_sha256 == Digest::ZERO
            || self.cold_replay_proof_sha256 == Digest::ZERO
            || self.state_graph_sha256 == Digest::ZERO
            || self.terminal_result_sha256 == Digest::ZERO
            || self.controller_tape_sha256 == Digest::ZERO
            || self.repetitions < 2
            || !self.passed
        {
            return Err(route_message(
                "native tactic cold replay evidence bundle manifest is invalid",
            ));
        }
        Ok(())
    }

    fn identity(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical).map_err(route_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-tactic-cold-replay-evidence-bundle/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }

    fn to_pretty_json(&self) -> Result<Vec<u8>, NativeTacticRouteRunError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(route_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

fn validate_pair(
    scratch_bundle_root: &Path,
    cold_replay_root: &Path,
) -> Result<
    (
        NativeTacticScratchEvidenceBundle,
        NativeTacticColdReplayProof,
    ),
    NativeTacticRouteRunError,
> {
    let scratch = NativeTacticScratchEvidenceBundle::read_and_validate(scratch_bundle_root)?;
    let proof: NativeTacticColdReplayProof =
        read_bounded_json(&cold_replay_root.join(NATIVE_TACTIC_COLD_REPLAY_PROOF_FILE))?;
    proof.validate_shape()?;
    let request: OptimizationRequest =
        read_json_blob(scratch_bundle_root, &scratch.optimization_request)?;
    let execution: NativeResidualExecutionBinding =
        read_json_blob(scratch_bundle_root, &scratch.execution_binding)?;
    let plan = NativeTacticExecutionPlan::read(&blob_path(
        scratch_bundle_root,
        &scratch.execution_plan.blob,
    ))?;
    let route: NativeTacticRouteReport =
        read_json_blob(scratch_bundle_root, &scratch.route_report)?;
    let bundled_seed = scratch
        .seeds
        .iter()
        .find(|seed| seed.seed == proof.seed)
        .ok_or_else(|| route_message("cold replay seed is absent from the scratch bundle"))?;
    let reported_seed = route
        .seeds
        .iter()
        .find(|seed| seed.seed == proof.seed)
        .ok_or_else(|| route_message("cold replay seed is absent from the bundled route report"))?;
    let tape_artifact = bundled_seed
        .best_terminal_tape
        .as_ref()
        .ok_or_else(|| route_message("cold replay seed has no bundled terminal tape"))?;
    let result_artifact = bundled_seed
        .best_terminal_result
        .as_ref()
        .ok_or_else(|| route_message("cold replay seed has no bundled terminal result"))?;
    let tape_bytes = read_blob(scratch_bundle_root, tape_artifact)?;
    let tape = InputTape::decode(&tape_bytes).map_err(route_error)?.tape;
    let result = TacticQFinalResult::read(&blob_path(scratch_bundle_root, &result_artifact.blob))
        .map_err(route_error)?;
    validate_native_tactic_cold_replay_artifacts(
        cold_replay_root,
        &proof,
        &request,
        &tape,
        &tape_bytes,
        proof.first_hit_tick,
    )?;
    if proof.optimization_request_sha256 != request.content_sha256
        || proof.execution_binding_sha256 != execution.content_sha256
        || proof.execution_plan_sha256 != plan.identity()?
        || proof.route_report_sha256 != route_report_sha256(&route)?
        || proof.route_report_sha256 != scratch.route_report.logical_identity_sha256
        || proof.seed != bundled_seed.seed
        || proof.state_graph_sha256 != bundled_seed.state_graph_sha256
        || proof.terminal_result_sha256 != result.content_sha256
        || proof.terminal_result_sha256 != result_artifact.logical_identity_sha256
        || proof.terminal_state_sha256 != result.terminal_state_sha256
        || proof.terminal_state_sha256
            != bundled_seed
                .best_terminal_state_sha256
                .unwrap_or(Digest::ZERO)
        || proof.objective_sha256 != result.objective_sha256
        || proof.source_boundary_index != request.route.source_boundary_index
        || proof.source_boundary_fingerprint != request.route.source_boundary_fingerprint
        || proof.native_source_boundary_fingerprint
            != request.route.native_source_boundary_fingerprint
        || proof.goal != request.terminal_predicate.goal
        || proof.terminal_program_sha256 != request.terminal_predicate.program_sha256
        || proof.terminal_definition_sha256 != request.terminal_predicate.definition_sha256
        || proof.first_hit_tick != bundled_seed.best_authenticated_tick.unwrap_or(u64::MAX)
        || proof.first_hit_tick >= request.budgets.exploration_horizon_ticks
        || proof.first_hit_tick != route.best_authenticated_tick.unwrap_or(u64::MAX)
        || proof.first_hit_tick != reported_seed.best_authenticated_tick.unwrap_or(u64::MAX)
        || proof.controller_tape.sha256 != result.route_tape_sha256
        || proof.controller_tape.sha256 != tape_artifact.blob.sha256
        || proof.executable != execution.executable
        || proof.runtime_dependencies != execution.runtime_dependencies
        || proof.game_data != execution.game_data
        || proof.milestone_program != execution.milestone_program
        || proof.world_context != execution.world_context
        || proof.card_fixture_manifest != execution.card_fixture_manifest
    {
        return Err(route_message(
            "cold replay proof differs from its self-contained scratch campaign",
        ));
    }
    Ok((scratch, proof))
}

fn resolved_new_directory(path: &Path) -> Result<PathBuf, NativeTacticRouteRunError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .map_err(route_error)?;
    let name = path
        .file_name()
        .ok_or_else(|| route_message("cold replay bundle output has no directory name"))?;
    Ok(parent.join(name))
}

fn copy_physical_tree(
    source_root: &Path,
    destination_root: &Path,
) -> Result<(), NativeTacticRouteRunError> {
    let metadata = fs::symlink_metadata(source_root).map_err(route_error)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "cold replay bundle source is not a physical directory",
        ));
    }
    fs::create_dir(destination_root).map_err(route_error)?;
    let mut pending = VecDeque::from([(source_root.to_path_buf(), destination_root.to_path_buf())]);
    let mut entries = 0_usize;
    while let Some((source, destination)) = pending.pop_front() {
        let mut children = fs::read_dir(&source)
            .map_err(route_error)?
            .map(|entry| entry.map_err(route_error))
            .collect::<Result<Vec<_>, _>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            entries = entries
                .checked_add(1)
                .ok_or_else(|| route_message("cold replay bundle tree entry count overflowed"))?;
            if entries > MAXIMUM_BUNDLE_TREE_ENTRIES {
                return Err(route_message("cold replay bundle tree is oversized"));
            }
            let metadata = child.file_type().map_err(route_error)?;
            let target = destination.join(child.file_name());
            if metadata.is_symlink() {
                return Err(route_message(
                    "cold replay bundle source contains a symbolic link",
                ));
            }
            if metadata.is_dir() {
                fs::create_dir(&target).map_err(route_error)?;
                pending.push_back((child.path(), target));
            } else if metadata.is_file() {
                fs::copy(child.path(), target).map_err(route_error)?;
            } else {
                return Err(route_message(
                    "cold replay bundle source contains a special file",
                ));
            }
        }
    }
    Ok(())
}

fn copy_cold_replay_evidence(
    source_root: &Path,
    destination_root: &Path,
    proof: &NativeTacticColdReplayProof,
) -> Result<(), NativeTacticRouteRunError> {
    fs::create_dir(destination_root).map_err(route_error)?;
    let mut relative_paths = BTreeSet::from([
        PathBuf::from(NATIVE_TACTIC_COLD_REPLAY_PROOF_FILE),
        PathBuf::from(&proof.controller_tape.path),
    ]);
    for attempt in &proof.attempts {
        for path in [
            &attempt.controller_tape.path,
            &attempt.milestone_result.path,
            &attempt.stdout.path,
            &attempt.stderr.path,
        ] {
            relative_paths.insert(PathBuf::from(path));
        }
    }
    for relative in relative_paths {
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(route_message(
                "cold replay evidence contains an unconfined artifact path",
            ));
        }
        let source = source_root.join(&relative);
        let metadata = fs::symlink_metadata(&source).map_err(route_error)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(route_message(
                "cold replay evidence artifact is not a physical file",
            ));
        }
        let destination = destination_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(route_error)?;
        }
        fs::copy(source, destination).map_err(route_error)?;
    }
    Ok(())
}

struct ColdReplayBundleBuildGuard {
    root: PathBuf,
    committed: bool,
}

impl ColdReplayBundleBuildGuard {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for ColdReplayBundleBuildGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> NativeTacticColdReplayEvidenceBundle {
        let mut manifest = NativeTacticColdReplayEvidenceBundle {
            schema: NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_BUNDLE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            scratch_bundle_sha256: Digest([1; 32]),
            cold_replay_proof_sha256: Digest([2; 32]),
            seed: 155_921,
            state_graph_sha256: Digest([3; 32]),
            terminal_result_sha256: Digest([4; 32]),
            controller_tape_sha256: Digest([5; 32]),
            first_hit_tick: 123,
            repetitions: 2,
            passed: true,
        };
        manifest.content_sha256 = manifest.identity().unwrap();
        manifest
    }

    #[test]
    fn sealed_manifest_requires_two_exact_repetitions_without_imposing_a_score_threshold() {
        manifest().validate_shape().unwrap();
        let mut invalid = manifest();
        invalid.repetitions = 1;
        invalid.content_sha256 = invalid.identity().unwrap();
        assert!(invalid.validate_shape().is_err());

        let mut discovery = manifest();
        discovery.first_hit_tick = 314;
        discovery.content_sha256 = discovery.identity().unwrap();
        discovery.validate_shape().unwrap();
    }

    #[test]
    fn manifest_identity_binds_campaign_and_replay_proof() {
        let original = manifest();
        let mut detached = original.clone();
        detached.cold_replay_proof_sha256 = Digest([9; 32]);
        assert_ne!(original.content_sha256, detached.identity().unwrap());
    }

    #[test]
    fn optimization_incumbent_is_the_exact_source_to_first_terminal_suffix() {
        let proof = super::cold_replay::tests::proof();
        let mut frames = vec![InputFrame::default(); proof.controller_tape_frames as usize];
        for (index, frame) in frames.iter_mut().enumerate() {
            frame.pads[0].stick_x = index as i8;
        }
        let mut authority = NativeTacticColdReplayAuthority {
            manifest: manifest(),
            proof,
            controller_tape: InputTape {
                frames,
                ..InputTape::default()
            },
        };

        let incumbent = authority.incumbent_tape().unwrap();
        assert_eq!(incumbent.frames.len(), 3);
        assert_eq!(incumbent.frames[0].pads[0].stick_x, 10);
        assert_eq!(incumbent.frames[2].pads[0].stick_x, 12);

        authority.controller_tape.frames.push(InputFrame::default());
        assert!(authority.incumbent_tape().is_err());
    }
}
