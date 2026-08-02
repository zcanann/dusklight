//! Authenticated handoff from scratch terminal discovery to residual optimization.

use super::*;
use crate::native_residual_campaign::{
    NativeResidualExecutionBinding, materialize_native_residual_process_tape,
};
use crate::optimization_request::{
    CampaignClass, OptimizationIncumbent, OptimizationIncumbentAuthority, OptimizationRequest,
};
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_harness_contracts::run_contract::sha256_artifact_file;
use sha2::Sha256;
use std::path::Component;

pub const NATIVE_TACTIC_OPTIMIZATION_HANDOFF_SCHEMA_V1: &str =
    "dusklight-native-tactic-optimization-handoff/v1";
pub const NATIVE_TACTIC_OPTIMIZATION_HANDOFF_MANIFEST: &str = "handoff.json";
const INCUMBENT_TAPE_FILE: &str = "incumbent.tape";
const PROCESS_BOOT_TAPE_FILE: &str = "process-boot.tape";
const OPTIMIZATION_REQUEST_FILE: &str = "optimization.request.json";
const EXECUTION_BINDING_FILE: &str = "execution.json";

#[derive(Clone, Debug)]
pub struct NativeTacticOptimizationHandoffConfig<'a> {
    pub repository_root: &'a Path,
    pub source_optimization: &'a OptimizationRequest,
    pub source_execution: &'a NativeResidualExecutionBinding,
    pub cold_replay_bundle_root: &'a Path,
    pub output_root: &'a Path,
    pub request_id: Option<&'a str>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticOptimizationHandoff {
    pub schema: String,
    pub content_sha256: Digest,
    pub source_optimization_request_sha256: Digest,
    pub source_execution_binding_sha256: Digest,
    pub cold_replay_bundle: ArtifactReference,
    pub cold_replay_proof_sha256: Digest,
    pub seed: u64,
    pub state_graph_sha256: Digest,
    pub terminal_result_sha256: Digest,
    pub first_hit_tick: u64,
    pub incumbent_tape: ArtifactReference,
    pub optimization_request: ArtifactReference,
    pub execution_binding: ArtifactReference,
    pub process_boot_tape: ArtifactReference,
}

impl NativeTacticOptimizationHandoff {
    fn seal(
        config: &NativeTacticOptimizationHandoffConfig<'_>,
        bundle: ArtifactReference,
        authority: &NativeTacticColdReplayAuthority,
        incumbent_tape: ArtifactReference,
        optimization_request: ArtifactReference,
        execution_binding: ArtifactReference,
        process_boot_tape: ArtifactReference,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let mut handoff = Self {
            schema: NATIVE_TACTIC_OPTIMIZATION_HANDOFF_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            source_optimization_request_sha256: config.source_optimization.content_sha256,
            source_execution_binding_sha256: config.source_execution.content_sha256,
            cold_replay_bundle: bundle,
            cold_replay_proof_sha256: authority.proof.content_sha256,
            seed: authority.manifest.seed,
            state_graph_sha256: authority.manifest.state_graph_sha256,
            terminal_result_sha256: authority.manifest.terminal_result_sha256,
            first_hit_tick: authority.manifest.first_hit_tick,
            incumbent_tape,
            optimization_request,
            execution_binding,
            process_boot_tape,
        };
        handoff.content_sha256 = handoff.identity()?;
        handoff.validate_shape()?;
        Ok(handoff)
    }

    fn validate_shape(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_OPTIMIZATION_HANDOFF_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
            || self.source_optimization_request_sha256 == Digest::ZERO
            || self.source_execution_binding_sha256 == Digest::ZERO
            || self.cold_replay_proof_sha256 == Digest::ZERO
            || self.state_graph_sha256 == Digest::ZERO
            || self.terminal_result_sha256 == Digest::ZERO
            || self.first_hit_tick == 0
        {
            return Err(route_message(
                "native tactic optimization handoff manifest is invalid",
            ));
        }
        for artifact in [
            &self.cold_replay_bundle,
            &self.incumbent_tape,
            &self.optimization_request,
            &self.execution_binding,
            &self.process_boot_tape,
        ] {
            if artifact.sha256 == Digest::ZERO || !confined_relative_path(&artifact.path) {
                return Err(route_message(
                    "native tactic optimization handoff contains an invalid artifact reference",
                ));
            }
        }
        Ok(())
    }

    fn identity(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical).map_err(route_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-tactic-optimization-handoff/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeTacticRouteRunError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(route_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

pub fn build_native_tactic_optimization_handoff(
    config: &NativeTacticOptimizationHandoffConfig<'_>,
) -> Result<NativeTacticOptimizationHandoff, NativeTacticRouteRunError> {
    let root = config.repository_root.canonicalize().map_err(route_error)?;
    config
        .source_execution
        .validate_files(&root, config.source_optimization)
        .map_err(route_error)?;
    if config.source_optimization.campaign_class != CampaignClass::FromScratchDiscovery
        || config.source_optimization.incumbent.is_some()
    {
        return Err(route_message(
            "terminal optimization handoff requires an incumbent-free from-scratch campaign",
        ));
    }
    let bundle_root = config
        .cold_replay_bundle_root
        .canonicalize()
        .map_err(route_error)?;
    if !bundle_root.starts_with(&root) || !bundle_root.is_dir() {
        return Err(route_message(
            "terminal optimization cold-replay bundle must be retained inside the repository",
        ));
    }
    let authority = NativeTacticColdReplayEvidenceBundle::read_authority(&bundle_root)?;
    if authority.proof.optimization_request_sha256 != config.source_optimization.content_sha256
        || authority.proof.execution_binding_sha256 != config.source_execution.content_sha256
    {
        return Err(route_message(
            "terminal optimization cold replay belongs to another request or execution",
        ));
    }
    let output = resolve_new_build_directory(&root, config.output_root)?;
    fs::create_dir(&output).map_err(route_error)?;
    let mut guard = HandoffBuildGuard::new(output.clone());

    let incumbent = authority.incumbent_tape()?;
    let incumbent_path = output.join(INCUMBENT_TAPE_FILE);
    write_new(&incumbent_path, &incumbent.encode().map_err(route_error)?)?;
    let incumbent_reference = artifact_reference(&root, &incumbent_path)?;
    let bundle_reference = artifact_reference(
        &root,
        &bundle_root.join(NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_MANIFEST),
    )?;

    let mut optimization = config.source_optimization.clone();
    optimization.id = config
        .request_id
        .map(str::to_owned)
        .unwrap_or_else(|| derived_request_id(&optimization.id, authority.manifest.seed));
    optimization.campaign_class = CampaignClass::LocalTasRefinement;
    optimization.incumbent = Some(OptimizationIncumbent {
        tape: incumbent_reference.clone(),
        first_hit_tick: authority.manifest.first_hit_tick,
        authority: OptimizationIncumbentAuthority::TacticColdReplay {
            bundle_manifest: bundle_reference.clone(),
        },
    });
    optimization.budgets.promotion_before_tick = authority.manifest.first_hit_tick;
    optimization.proposal.search_space.start_frame = 0;
    optimization.proposal.search_space.end_frame_exclusive =
        u64::try_from(incumbent.frames.len()).map_err(route_error)?;
    optimization.proposal.critic_ranking = None;
    let output_relative = repository_relative(&root, &output)?;
    optimization.resume.state_path = format!("{output_relative}/state.json");
    optimization.resume.journal_path = format!("{output_relative}/journal.jsonl");
    optimization.horizon_tightening = None;
    optimization.reverse_curriculum = None;
    optimization.refresh_content_sha256().map_err(route_error)?;
    optimization.validate_files(&root).map_err(route_error)?;
    let request_path = output.join(OPTIMIZATION_REQUEST_FILE);
    write_new(
        &request_path,
        &optimization.to_pretty_json().map_err(route_error)?,
    )?;

    let process_tape =
        materialize_native_residual_process_tape(&root, &optimization).map_err(route_error)?;
    let process_tape_path = output.join(PROCESS_BOOT_TAPE_FILE);
    write_new(
        &process_tape_path,
        &process_tape.encode().map_err(route_error)?,
    )?;
    let execution = NativeResidualExecutionBinding::seal(
        &root,
        &optimization,
        &root.join(&config.source_execution.executable.path),
        &root.join(&config.source_execution.game_data.path),
        &process_tape_path,
        &root.join(&config.source_execution.milestone_program.path),
        &root.join(&config.source_execution.world_context.path),
        &root.join(&config.source_execution.card_fixture_manifest.path),
        config.source_execution.checkpoint_validation_ticks,
        config.source_execution.verify_state_hashes,
    )
    .map_err(route_error)?;
    let execution_path = output.join(EXECUTION_BINDING_FILE);
    write_new(
        &execution_path,
        &execution.to_pretty_json().map_err(route_error)?,
    )?;

    let handoff = NativeTacticOptimizationHandoff::seal(
        config,
        bundle_reference,
        &authority,
        incumbent_reference,
        artifact_reference(&root, &request_path)?,
        artifact_reference(&root, &execution_path)?,
        artifact_reference(&root, &process_tape_path)?,
    )?;
    write_new(
        &output.join(NATIVE_TACTIC_OPTIMIZATION_HANDOFF_MANIFEST),
        &handoff.to_pretty_json()?,
    )?;
    execution
        .validate_files(&root, &optimization)
        .map_err(route_error)?;
    guard.commit();
    Ok(handoff)
}

fn derived_request_id(source: &str, seed: u64) -> String {
    let suffix = format!("-terminal-{seed}-refinement");
    let prefix_bytes = 128_usize.saturating_sub(suffix.len());
    let mut prefix = source;
    while prefix.len() > prefix_bytes {
        prefix = &prefix[..prefix.len() - 1];
    }
    format!("{prefix}{suffix}")
}

fn resolve_new_build_directory(
    root: &Path,
    requested: &Path,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    let unresolved = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let relative = unresolved
        .strip_prefix(root)
        .map_err(|_| route_message("terminal optimization output is outside the repository"))?;
    if relative
        .components()
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        != Some("build")
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || unresolved.exists()
    {
        return Err(route_message(
            "terminal optimization output must be a new confined build directory",
        ));
    }
    let parent = unresolved
        .parent()
        .ok_or_else(|| route_message("terminal optimization output has no parent"))?;
    fs::create_dir_all(parent).map_err(route_error)?;
    let parent = parent.canonicalize().map_err(route_error)?;
    if !parent.starts_with(root) {
        return Err(route_message(
            "terminal optimization output parent escapes the repository",
        ));
    }
    Ok(parent.join(
        unresolved
            .file_name()
            .ok_or_else(|| route_message("terminal optimization output has no name"))?,
    ))
}

fn artifact_reference(
    root: &Path,
    path: &Path,
) -> Result<ArtifactReference, NativeTacticRouteRunError> {
    let path = path.canonicalize().map_err(route_error)?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(route_message(
            "terminal optimization artifact is outside the repository",
        ));
    }
    Ok(ArtifactReference {
        path: repository_relative(root, &path)?,
        sha256: sha256_artifact_file(&path).map_err(route_error)?,
    })
}

fn repository_relative(root: &Path, path: &Path) -> Result<String, NativeTacticRouteRunError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| route_message("terminal optimization path is outside the repository"))?;
    let value = relative
        .to_str()
        .ok_or_else(|| route_message("terminal optimization path is not UTF-8"))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    if !confined_relative_path(&value) {
        return Err(route_message(
            "terminal optimization path is not a confined relative path",
        ));
    }
    Ok(value)
}

fn confined_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

struct HandoffBuildGuard {
    root: PathBuf,
    committed: bool,
}

impl HandoffBuildGuard {
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

impl Drop for HandoffBuildGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_id_is_stable_and_bounded() {
        assert_eq!(
            derived_request_id("scratch", 155_921),
            "scratch-terminal-155921-refinement"
        );
        assert_eq!(derived_request_id(&"a".repeat(128), 7).len(), 128);
    }

    #[test]
    fn only_confined_relative_artifacts_are_accepted() {
        assert!(confined_relative_path("build/handoff/incumbent.tape"));
        assert!(!confined_relative_path("../incumbent.tape"));
        assert!(!confined_relative_path("/tmp/incumbent.tape"));
    }
}
