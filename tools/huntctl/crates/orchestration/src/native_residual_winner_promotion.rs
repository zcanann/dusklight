//! Immutable next-parent request derived from a selected residual winner.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_residual_winner_selection::NativeResidualWinnerSelection;
use crate::optimization_request::{
    OptimizationIncumbent, OptimizationIncumbentAuthority, OptimizationRequest,
};
use crate::residual_campaign::ResidualCampaignCandidate;
use crate::residual_winner_minimization::ResidualWinnerMinimizationSummary;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_search::residual_action::compile_residual_candidate_to_horizon;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path};

pub struct NativeResidualWinnerPromotionConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub selection: &'a NativeResidualWinnerSelection,
    pub promoted_id: &'a str,
    pub output_request: &'a Path,
}

pub fn promote_native_residual_winner(
    config: &NativeResidualWinnerPromotionConfig<'_>,
) -> Result<OptimizationRequest, NativeResidualWinnerPromotionError> {
    let root = config
        .repository_root
        .canonicalize()
        .map_err(promotion_error)?;
    config
        .selection
        .validate_files(&root, config.optimization, config.execution)
        .map_err(promotion_error)?;
    let output_relative = canonical_build_output(&root, config.output_request)?;
    if root.join(&output_relative).exists() {
        return Err(promotion_message(
            "promoted residual request output already exists",
        ));
    }
    let minimization: ResidualWinnerMinimizationSummary =
        read_reference(&root, &config.selection.minimization_summary)?;
    if minimization.minimized_first_hit_tick != config.selection.selected_first_hit_tick
        || minimization.minimized_tape_sha256 != config.selection.minimized_tape_sha256
    {
        return Err(promotion_message(
            "selected winner differs from its minimized incumbent",
        ));
    }
    let output_parent = output_relative
        .parent()
        .ok_or_else(|| promotion_message("promoted residual request has no output directory"))?;
    let suffix_bytes = final_suffix_bytes(&root, config.optimization, &minimization)?;
    if Digest(Sha256::digest(&suffix_bytes).into()) != minimization.minimized_tape_sha256 {
        return Err(promotion_message(
            "promoted residual suffix differs from the selected winner",
        ));
    }
    let output_directory = root.join(output_parent);
    fs::create_dir_all(&output_directory).map_err(promotion_error)?;
    let incumbent_path = output_directory.join("incumbent.tape");
    write_new(&incumbent_path, &suffix_bytes)?;
    let minimized_tape = ArtifactReference {
        path: slash_path(&output_parent.join("incumbent.tape"))?,
        sha256: minimization.minimized_tape_sha256,
    };
    let mut request = config.optimization.clone();
    request.id = config.promoted_id.to_owned();
    request.incumbent = Some(OptimizationIncumbent {
        tape: minimized_tape,
        first_hit_tick: config.selection.selected_first_hit_tick,
        authority: OptimizationIncumbentAuthority::ResidualWinnerColdReplay {
            proof: config.selection.cold_replay_proof.clone(),
            source_request: config.selection.source_request.clone(),
            source_execution: config.selection.source_execution.clone(),
        },
    });
    request.budgets.promotion_before_tick = config.selection.selected_first_hit_tick;
    request.proposal.critic_ranking = None;
    request.resume.state_path = slash_path(&output_parent.join("state.json"))?;
    request.resume.journal_path = slash_path(&output_parent.join("journal.jsonl"))?;
    request.resume.checkpoint_every_candidates = request
        .resume
        .checkpoint_every_candidates
        .min(request.budgets.candidate_budget)
        .max(1);
    if let Some(limit) = request.retention.failed_episode_limit.as_mut() {
        *limit = (*limit).min(request.budgets.candidate_budget).max(1);
    }
    request.horizon_tightening = None;
    request.reverse_curriculum = None;
    request.refresh_content_sha256().map_err(promotion_error)?;
    request.validate().map_err(promotion_error)?;
    Ok(request)
}

fn final_suffix_bytes(
    root: &Path,
    optimization: &OptimizationRequest,
    minimization: &ResidualWinnerMinimizationSummary,
) -> Result<Vec<u8>, NativeResidualWinnerPromotionError> {
    if let Some(reference) = &minimization.minimized_tape {
        return read_reference_bytes(root, reference);
    }
    let candidate: ResidualCampaignCandidate =
        read_reference(root, &minimization.source_candidate)?;
    let incumbent = optimization
        .incumbent
        .as_ref()
        .ok_or_else(|| promotion_message("promoted residual source has no incumbent"))?;
    let parent_bytes = read_reference_bytes(root, &incumbent.tape)?;
    let parent = InputTape::decode(&parent_bytes)
        .map_err(promotion_error)?
        .tape;
    let compiled = compile_residual_candidate_to_horizon(
        &parent,
        &parent_bytes,
        &candidate.candidate,
        optimization.budgets.exploration_horizon_ticks,
    )
    .map_err(promotion_error)?;
    Ok(compiled.bytes)
}

fn canonical_build_output(
    root: &Path,
    output: &Path,
) -> Result<std::path::PathBuf, NativeResidualWinnerPromotionError> {
    let relative = if output.is_absolute() {
        output
            .strip_prefix(root)
            .map_err(|_| promotion_message("promoted residual request escapes the repository"))?
            .to_path_buf()
    } else {
        output.to_path_buf()
    };
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !relative.starts_with("build")
        || relative.file_name().and_then(|name| name.to_str()) != Some("optimization.request.json")
    {
        return Err(promotion_message(
            "promoted residual request must be a canonical repository-relative build/**/optimization.request.json path",
        ));
    }
    Ok(relative)
}

fn slash_path(path: &Path) -> Result<String, NativeResidualWinnerPromotionError> {
    path.to_str()
        .map(|path| path.replace(std::path::MAIN_SEPARATOR, "/"))
        .ok_or_else(|| promotion_message("promoted residual path is not UTF-8"))
}

fn read_reference<T: for<'de> Deserialize<'de>>(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<T, NativeResidualWinnerPromotionError> {
    serde_json::from_slice(&read_reference_bytes(root, reference)?).map_err(promotion_error)
}

fn read_reference_bytes(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, NativeResidualWinnerPromotionError> {
    let relative = Path::new(&reference.path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(promotion_message(
            "promoted residual source has a noncanonical path",
        ));
    }
    let path = root.join(relative);
    let bytes = fs::read(&path).map_err(promotion_error)?;
    if Digest(Sha256::digest(&bytes).into()) != reference.sha256 {
        return Err(promotion_message("promoted residual source digest differs"));
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), NativeResidualWinnerPromotionError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(promotion_error)?;
    file.write_all(bytes).map_err(promotion_error)?;
    file.sync_all().map_err(promotion_error)
}

#[derive(Debug)]
pub struct NativeResidualWinnerPromotionError(String);

fn promotion_message(message: impl Into<String>) -> NativeResidualWinnerPromotionError {
    NativeResidualWinnerPromotionError(message.into())
}

fn promotion_error(error: impl fmt::Display) -> NativeResidualWinnerPromotionError {
    promotion_message(error.to_string())
}

impl fmt::Display for NativeResidualWinnerPromotionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeResidualWinnerPromotionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn promoted_request_output_is_confined_and_canonical() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-promotion-path-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("build")).unwrap();
        assert_eq!(
            canonical_build_output(&root, Path::new("build/next/optimization.request.json"))
                .unwrap(),
            Path::new("build/next/optimization.request.json")
        );
        for invalid in [
            "request.json",
            "build/optimization.json",
            "build/../optimization.request.json",
        ] {
            assert!(canonical_build_output(&root, Path::new(invalid)).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
