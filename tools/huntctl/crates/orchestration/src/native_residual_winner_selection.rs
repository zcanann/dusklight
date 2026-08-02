//! Automatic selection pipeline for the best strict residual improvement.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::optimization_request::OptimizationRequest;
use crate::optimization_resume::load_optimization_resume_after_request_validation;
use crate::residual_campaign::{ResidualCampaignCandidate, ResidualCampaignCheckpoint};
use crate::residual_campaign_runner::{artifact_reference, campaign_root, load_checkpoint};
use crate::residual_winner_cold_replay::{
    RESIDUAL_WINNER_COLD_REPLAY_PROOF_FILE, ResidualWinnerColdReplayConfig,
    ResidualWinnerColdReplayProof, read_and_validate_residual_winner_cold_replay,
    run_residual_winner_cold_replay,
};
use crate::residual_winner_minimization::{
    ResidualWinnerMinimizationConfig, ResidualWinnerMinimizationSummary,
    run_residual_winner_minimization,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path};
use std::time::Duration;

pub const NATIVE_RESIDUAL_WINNER_SELECTION_SCHEMA_V1: &str =
    "dusklight-native-residual-winner-selection/v1";
pub const NATIVE_RESIDUAL_WINNER_SELECTION_MANIFEST: &str = "selection.json";
const MINIMIZATION_DIRECTORY: &str = "minimization";
const COLD_REPLAY_DIRECTORY: &str = "cold-replay";
const MAXIMUM_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

pub struct NativeResidualWinnerSelectionConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub source_request: ArtifactReference,
    pub source_execution: ArtifactReference,
    pub minimization_candidate_budget: u64,
    pub cold_replay_timeout: Duration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResidualWinnerSelection {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub source_request: ArtifactReference,
    pub source_execution: ArtifactReference,
    pub source_checkpoint: ArtifactReference,
    pub source_candidate: ArtifactReference,
    pub discovered_candidate_sha256: Digest,
    pub incumbent_first_hit_tick: u64,
    pub selected_first_hit_tick: u64,
    pub minimized_tape_sha256: Digest,
    pub full_route_tape_sha256: Digest,
    pub minimization_summary: ArtifactReference,
    pub cold_replay_proof: ArtifactReference,
    pub cold_replay_repetitions: u32,
}

pub fn select_native_residual_winner(
    config: &NativeResidualWinnerSelectionConfig<'_>,
) -> Result<Option<ArtifactReference>, NativeResidualWinnerSelectionError> {
    let root = config
        .repository_root
        .canonicalize()
        .map_err(selection_error)?;
    config
        .execution
        .validate_files(&root, config.optimization)
        .map_err(selection_error)?;
    validate_source_artifact(
        &root,
        &config.source_request,
        config.optimization,
        "optimization request",
    )?;
    validate_source_artifact(
        &root,
        &config.source_execution,
        config.execution,
        "execution binding",
    )?;
    if config.minimization_candidate_budget == 0 || config.cold_replay_timeout.is_zero() {
        return Err(selection_message(
            "native residual winner selection budgets must be positive",
        ));
    }
    let incumbent = config.optimization.incumbent.as_ref().ok_or_else(|| {
        selection_message("native residual winner selection requires an incumbent")
    })?;
    let resume = load_optimization_resume_after_request_validation(config.optimization, &root)
        .map_err(selection_error)?;
    let checkpoint: ResidualCampaignCheckpoint = load_checkpoint(
        &root,
        config.optimization,
        config.execution.content_sha256,
        &resume,
    )
    .map_err(selection_error)?;
    let archive = checkpoint.restore_archive().map_err(selection_error)?;
    let Some(winner) = archive
        .successes()
        .iter()
        .find(|success| success.minimized_from.is_none())
    else {
        return Ok(None);
    };
    if winner.first_hit_tick >= incumbent.first_hit_tick {
        return Ok(None);
    }
    let matches = resume
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.compiled_tape_sha256 == winner.realized_tape_sha256
                && candidate.result.is_some()
        })
        .collect::<Vec<_>>();
    let candidate_state = match matches.as_slice() {
        [candidate] => *candidate,
        [] => {
            return Err(selection_message(
                "retained residual winner has no completed candidate artifact",
            ));
        }
        _ => {
            return Err(selection_message(
                "retained residual winner maps to multiple candidate artifacts",
            ));
        }
    };
    let candidate: ResidualCampaignCandidate =
        serde_json::from_slice(&read_reference(&root, &candidate_state.candidate)?)
            .map_err(selection_error)?;
    candidate.validate().map_err(selection_error)?;
    if candidate.candidate.content_sha256 != winner.candidate_sha256
        || candidate.compilation.realized_tape_sha256 != winner.realized_tape_sha256
    {
        return Err(selection_message(
            "retained residual winner differs from its candidate artifact",
        ));
    }
    let checkpoint_reference = resume
        .latest_optimizer_checkpoint
        .as_ref()
        .map(|latest| latest.artifact.clone())
        .ok_or_else(|| selection_message("residual winner has no durable checkpoint"))?;
    let campaign = campaign_root(&root, config.optimization).map_err(selection_error)?;
    let digest = winner.realized_tape_sha256.to_string();
    let selection_root = campaign.join("selected-winners").join(format!(
        "tick-{:06}-{}",
        winner.first_hit_tick,
        &digest[..12]
    ));
    fs::create_dir_all(&selection_root).map_err(selection_error)?;
    let manifest_path = selection_root.join(NATIVE_RESIDUAL_WINNER_SELECTION_MANIFEST);
    if manifest_path.is_file() {
        let manifest: NativeResidualWinnerSelection =
            serde_json::from_slice(&read_bounded(&manifest_path)?).map_err(selection_error)?;
        manifest.validate_files(&root, config.optimization, config.execution)?;
        return artifact_reference(&root, &manifest_path)
            .map(Some)
            .map_err(selection_error);
    }
    let minimization_root = selection_root.join(MINIMIZATION_DIRECTORY);
    let minimization = run_residual_winner_minimization(&ResidualWinnerMinimizationConfig {
        repository_root: &root,
        optimization: config.optimization,
        execution: config.execution,
        checkpoint: &checkpoint,
        source_request: config.source_request.clone(),
        source_execution: config.source_execution.clone(),
        source_checkpoint: checkpoint_reference.clone(),
        source_candidate: candidate_state.candidate.clone(),
        candidate: &candidate,
        output_root: &minimization_root,
        candidate_budget: config.minimization_candidate_budget,
        resume: true,
        cancellation: None,
    })
    .map_err(selection_error)?;
    if minimization.minimized_first_hit_tick >= incumbent.first_hit_tick {
        return Err(selection_message(
            "minimized residual winner is not a strict incumbent improvement",
        ));
    }
    let minimization_reference = artifact_reference(&root, &minimization_root.join("summary.json"))
        .map_err(selection_error)?;
    let cold_replay_root = selection_root.join(COLD_REPLAY_DIRECTORY);
    let proof = if cold_replay_root
        .join(RESIDUAL_WINNER_COLD_REPLAY_PROOF_FILE)
        .is_file()
    {
        read_and_validate_residual_winner_cold_replay(
            &root,
            config.optimization,
            config.execution,
            &cold_replay_root,
        )
        .map_err(selection_error)?
    } else {
        run_residual_winner_cold_replay(&ResidualWinnerColdReplayConfig {
            repository_root: &root,
            optimization: config.optimization,
            execution: config.execution,
            minimization_summary: &minimization,
            minimization_summary_artifact: minimization_reference.clone(),
            timeout: config.cold_replay_timeout,
            output_root: &cold_replay_root,
        })
        .map_err(selection_error)?
    };
    let proof_reference = artifact_reference(
        &root,
        &cold_replay_root.join(RESIDUAL_WINNER_COLD_REPLAY_PROOF_FILE),
    )
    .map_err(selection_error)?;
    let mut selection = NativeResidualWinnerSelection {
        schema: NATIVE_RESIDUAL_WINNER_SELECTION_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        source_request: config.source_request.clone(),
        source_execution: config.source_execution.clone(),
        source_checkpoint: checkpoint_reference,
        source_candidate: candidate_state.candidate.clone(),
        discovered_candidate_sha256: winner.candidate_sha256,
        incumbent_first_hit_tick: incumbent.first_hit_tick,
        selected_first_hit_tick: proof.first_hit_tick,
        minimized_tape_sha256: minimization.minimized_tape_sha256,
        full_route_tape_sha256: proof.controller_tape.sha256,
        minimization_summary: minimization_reference,
        cold_replay_proof: proof_reference,
        cold_replay_repetitions: u32::try_from(proof.attempts.len()).map_err(selection_error)?,
    };
    selection.content_sha256 = selection.identity()?;
    selection.validate_files(&root, config.optimization, config.execution)?;
    write_new(&manifest_path, &selection.to_pretty_json()?)?;
    artifact_reference(&root, &manifest_path)
        .map(Some)
        .map_err(selection_error)
}

impl NativeResidualWinnerSelection {
    pub fn validate_files(
        &self,
        repository_root: &Path,
        optimization: &OptimizationRequest,
        execution: &NativeResidualExecutionBinding,
    ) -> Result<(), NativeResidualWinnerSelectionError> {
        self.validate_shape()?;
        let root = repository_root.canonicalize().map_err(selection_error)?;
        validate_source_artifact(
            &root,
            &self.source_request,
            optimization,
            "optimization request",
        )?;
        validate_source_artifact(
            &root,
            &self.source_execution,
            execution,
            "execution binding",
        )?;
        execution
            .validate_files(&root, optimization)
            .map_err(selection_error)?;
        let checkpoint: ResidualCampaignCheckpoint =
            serde_json::from_slice(&read_reference(&root, &self.source_checkpoint)?)
                .map_err(selection_error)?;
        checkpoint
            .validate(optimization, execution.content_sha256)
            .map_err(selection_error)?;
        let candidate: ResidualCampaignCandidate =
            serde_json::from_slice(&read_reference(&root, &self.source_candidate)?)
                .map_err(selection_error)?;
        candidate.validate().map_err(selection_error)?;
        let minimization: ResidualWinnerMinimizationSummary =
            serde_json::from_slice(&read_reference(&root, &self.minimization_summary)?)
                .map_err(selection_error)?;
        minimization
            .validate_files(&root)
            .map_err(selection_error)?;
        let proof_root = root
            .join(&self.cold_replay_proof.path)
            .parent()
            .ok_or_else(|| selection_message("cold replay proof has no parent"))?
            .to_path_buf();
        let proof: ResidualWinnerColdReplayProof =
            serde_json::from_slice(&read_reference(&root, &self.cold_replay_proof)?)
                .map_err(selection_error)?;
        let validated = read_and_validate_residual_winner_cold_replay(
            &root,
            optimization,
            execution,
            &proof_root,
        )
        .map_err(selection_error)?;
        if proof != validated
            || self.optimization_request_sha256 != optimization.content_sha256
            || self.execution_binding_sha256 != execution.content_sha256
            || self.discovered_candidate_sha256 != candidate.candidate.content_sha256
            || minimization.source_checkpoint != self.source_checkpoint
            || minimization.source_candidate != self.source_candidate
            || proof.minimization_summary != self.minimization_summary
            || self.selected_first_hit_tick != minimization.minimized_first_hit_tick
            || self.selected_first_hit_tick != proof.first_hit_tick
            || self.selected_first_hit_tick >= self.incumbent_first_hit_tick
            || self.minimized_tape_sha256 != minimization.minimized_tape_sha256
            || self.full_route_tape_sha256 != proof.controller_tape.sha256
            || self.cold_replay_repetitions != proof.attempts.len() as u32
        {
            return Err(selection_message(
                "native residual winner selection lineage is invalid or detached",
            ));
        }
        Ok(())
    }

    fn validate_shape(&self) -> Result<(), NativeResidualWinnerSelectionError> {
        if self.schema != NATIVE_RESIDUAL_WINNER_SELECTION_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || [
                &self.source_request,
                &self.source_execution,
                &self.source_checkpoint,
                &self.source_candidate,
                &self.minimization_summary,
                &self.cold_replay_proof,
            ]
            .into_iter()
            .any(|reference| !valid_reference(reference))
            || self.discovered_candidate_sha256 == Digest::ZERO
            || self.selected_first_hit_tick == 0
            || self.selected_first_hit_tick >= self.incumbent_first_hit_tick
            || self.minimized_tape_sha256 == Digest::ZERO
            || self.full_route_tape_sha256 == Digest::ZERO
            || self.cold_replay_repetitions != 2
        {
            return Err(selection_message(
                "native residual winner selection is invalid or detached",
            ));
        }
        Ok(())
    }

    fn identity(&self) -> Result<Digest, NativeResidualWinnerSelectionError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(
            b"dusklight.native-residual-winner-selection/v1\0",
            &canonical,
        )
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeResidualWinnerSelectionError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(selection_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

fn validate_source_artifact<T>(
    root: &Path,
    reference: &ArtifactReference,
    expected: &T,
    label: &str,
) -> Result<(), NativeResidualWinnerSelectionError>
where
    T: for<'de> Deserialize<'de> + PartialEq,
{
    let actual: T =
        serde_json::from_slice(&read_reference(root, reference)?).map_err(selection_error)?;
    if &actual != expected {
        return Err(selection_message(format!(
            "native residual winner {label} differs from its artifact"
        )));
    }
    Ok(())
}

fn read_reference(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, NativeResidualWinnerSelectionError> {
    if !valid_reference(reference) {
        return Err(selection_message("selection artifact reference is invalid"));
    }
    let path = root
        .join(&reference.path)
        .canonicalize()
        .map_err(selection_error)?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(selection_message(
            "selection artifact escapes the repository",
        ));
    }
    let bytes = read_bounded(&path)?;
    if Digest(Sha256::digest(&bytes).into()) != reference.sha256 {
        return Err(selection_message("selection artifact digest differs"));
    }
    Ok(bytes)
}

fn valid_reference(reference: &ArtifactReference) -> bool {
    let path = Path::new(&reference.path);
    reference.sha256 != Digest::ZERO
        && !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, NativeResidualWinnerSelectionError> {
    let metadata = fs::symlink_metadata(path).map_err(selection_error)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAXIMUM_ARTIFACT_BYTES
    {
        return Err(selection_message(
            "selection artifact is invalid or oversized",
        ));
    }
    fs::read(path).map_err(selection_error)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), NativeResidualWinnerSelectionError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(selection_error)?;
    file.write_all(bytes).map_err(selection_error)?;
    file.sync_all().map_err(selection_error)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, NativeResidualWinnerSelectionError> {
    let bytes = serde_json::to_vec(value).map_err(selection_error)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Debug)]
pub struct NativeResidualWinnerSelectionError(String);

fn selection_message(message: impl Into<String>) -> NativeResidualWinnerSelectionError {
    NativeResidualWinnerSelectionError(message.into())
}

fn selection_error(error: impl fmt::Display) -> NativeResidualWinnerSelectionError {
    selection_message(error.to_string())
}

impl fmt::Display for NativeResidualWinnerSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeResidualWinnerSelectionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(value: u8) -> Digest {
        Digest([value; 32])
    }

    fn reference(path: &str, value: u8) -> ArtifactReference {
        ArtifactReference {
            path: path.into(),
            sha256: digest(value),
        }
    }

    fn selection() -> NativeResidualWinnerSelection {
        let mut selection = NativeResidualWinnerSelection {
            schema: NATIVE_RESIDUAL_WINNER_SELECTION_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: digest(1),
            execution_binding_sha256: digest(2),
            source_request: reference("build/campaign/request.json", 3),
            source_execution: reference("build/campaign/execution.json", 4),
            source_checkpoint: reference("build/campaign/checkpoint.json", 5),
            source_candidate: reference("build/campaign/candidate.json", 6),
            discovered_candidate_sha256: digest(7),
            incumbent_first_hit_tick: 239,
            selected_first_hit_tick: 231,
            minimized_tape_sha256: digest(8),
            full_route_tape_sha256: digest(9),
            minimization_summary: reference("build/selection/minimization/summary.json", 10),
            cold_replay_proof: reference("build/selection/cold-replay/proof.json", 11),
            cold_replay_repetitions: 2,
        };
        selection.content_sha256 = selection.identity().unwrap();
        selection
    }

    #[test]
    fn selection_shape_requires_a_strict_twice_replayed_improvement() {
        selection().validate_shape().unwrap();

        let mut tied = selection();
        tied.selected_first_hit_tick = tied.incumbent_first_hit_tick;
        tied.content_sha256 = tied.identity().unwrap();
        assert!(tied.validate_shape().is_err());

        let mut single = selection();
        single.cold_replay_repetitions = 1;
        single.content_sha256 = single.identity().unwrap();
        assert!(single.validate_shape().is_err());
    }
}
