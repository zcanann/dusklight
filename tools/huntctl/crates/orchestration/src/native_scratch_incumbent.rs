//! Common authenticated incumbent contract for composable scratch refinements.

use crate::native_scratch_heading::inspect_native_scratch_heading_checkpoint;
use crate::native_scratch_learner::{
    NativeScratchRunConfig, ScratchRefinementSource, load_scratch_refinement_source,
};
use crate::native_scratch_option_refinement::inspect_native_scratch_option_checkpoint;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_learning::scratch_action_catalog::{
    SCRATCH_HEADING_COUNT, map_scratch_action_to_finer_catalog,
    scratch_action_catalog_with_heading_count,
};
use dusklight_learning::tactic_asset::TacticAssetCatalog;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub enum ScratchIncumbentSource<'a> {
    Scratch,
    Heading(&'a Path),
    Option(&'a Path),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedScratchIncumbent {
    pub checkpoint_sha256: Digest,
    pub seed: u64,
    pub heading_count: usize,
    pub incumbent_action_sequence: Vec<usize>,
    pub incumbent_ticks: u64,
}

pub fn load_authenticated_scratch_incumbent(
    config: &NativeScratchRunConfig<'_>,
    source: ScratchIncumbentSource<'_>,
    target_heading_count: usize,
) -> Result<AuthenticatedScratchIncumbent, ScratchIncumbentError> {
    match source {
        ScratchIncumbentSource::Scratch => {
            if target_heading_count != SCRATCH_HEADING_COUNT {
                return Err(incumbent_error(
                    "scratch learner source can only use its original heading catalog",
                ));
            }
            let source = load_scratch_refinement_source(config).map_err(incumbent_display)?;
            Ok(from_scratch_source(source))
        }
        ScratchIncumbentSource::Heading(path) => {
            let source =
                inspect_native_scratch_heading_checkpoint(path).map_err(incumbent_display)?;
            if source.schema != "dusklight-native-scratch-heading-inspection/v2"
                || source.checkpoint_schema != "dusklight-native-scratch-heading-checkpoint/v2"
                || source.stop_reason != "candidate_exhaustion"
                || source.candidates_remaining != 0
                || source.optimization_request_sha256 != config.optimization.content_sha256
                || source.execution_binding_sha256 != config.execution.content_sha256
                || source.seed != config.seed
            {
                return Err(incumbent_error(
                    "heading source is not an exhausted authenticated incumbent",
                ));
            }
            load_option_ids(
                source.checkpoint_sha256,
                source.seed,
                source.heading_count as usize,
                source.incumbent_ticks,
                source.incumbent_action_sequence_sha256,
                &source.incumbent_options,
                source.action_universe_sha256,
                target_heading_count,
            )
        }
        ScratchIncumbentSource::Option(path) => {
            let source =
                inspect_native_scratch_option_checkpoint(path).map_err(incumbent_display)?;
            if source.schema != "dusklight-native-scratch-option-refinement-inspection/v1"
                || source.checkpoint_schema
                    != "dusklight-native-scratch-option-refinement-checkpoint/v1"
                || source.stop_reason != "candidate_exhaustion"
                || source.candidates_remaining != 0
                || source.optimization_request_sha256 != config.optimization.content_sha256
                || source.execution_binding_sha256 != config.execution.content_sha256
                || source.seed != config.seed
            {
                return Err(incumbent_error(
                    "option source is not an exhausted authenticated incumbent",
                ));
            }
            load_option_ids(
                source.checkpoint_sha256,
                source.seed,
                SCRATCH_HEADING_COUNT,
                source.incumbent_ticks,
                source.incumbent_action_sequence_sha256,
                &source.incumbent_options,
                source.action_universe_sha256,
                target_heading_count,
            )
        }
    }
}

fn from_scratch_source(source: ScratchRefinementSource) -> AuthenticatedScratchIncumbent {
    AuthenticatedScratchIncumbent {
        checkpoint_sha256: source.checkpoint_sha256,
        seed: source.seed,
        heading_count: SCRATCH_HEADING_COUNT,
        incumbent_action_sequence: source.incumbent_action_sequence,
        incumbent_ticks: source.incumbent_ticks,
    }
}

#[allow(clippy::too_many_arguments)]
fn load_option_ids(
    checkpoint_sha256: Digest,
    seed: u64,
    source_heading_count: usize,
    ticks: u64,
    expected_sequence_sha256: Digest,
    options: &[String],
    expected_action_universe_sha256: Digest,
    target_heading_count: usize,
) -> Result<AuthenticatedScratchIncumbent, ScratchIncumbentError> {
    let source_catalog = scratch_action_catalog_with_heading_count(source_heading_count)
        .map_err(incumbent_display)?;
    if source_catalog.action_schema_sha256() != expected_action_universe_sha256 {
        return Err(incumbent_error("incumbent action universe is detached"));
    }
    let source_actions = option_ids_to_actions(options, &source_catalog)?;
    if action_sequence_sha256(&source_actions)? != expected_sequence_sha256 {
        return Err(incumbent_error("incumbent action sequence is detached"));
    }
    let incumbent_action_sequence = if target_heading_count == source_heading_count {
        source_actions
    } else if target_heading_count == source_heading_count.saturating_mul(2) {
        let target_catalog = scratch_action_catalog_with_heading_count(target_heading_count)
            .map_err(incumbent_display)?;
        source_actions
            .iter()
            .map(|action| {
                map_scratch_action_to_finer_catalog(&source_catalog, &target_catalog, *action)
                    .map_err(incumbent_display)
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        return Err(incumbent_error(
            "incumbent target headings must match or exactly double the source",
        ));
    };
    Ok(AuthenticatedScratchIncumbent {
        checkpoint_sha256,
        seed,
        heading_count: target_heading_count,
        incumbent_action_sequence,
        incumbent_ticks: ticks,
    })
}

fn option_ids_to_actions(
    options: &[String],
    catalog: &TacticAssetCatalog,
) -> Result<Vec<usize>, ScratchIncumbentError> {
    options
        .iter()
        .map(|option| {
            catalog
                .entries()
                .binary_search_by_key(&option.as_str(), |entry| entry.option_id())
                .map_err(|_| incumbent_error("incumbent option is absent from its catalog"))
        })
        .collect()
}

fn action_sequence_sha256(sequence: &[usize]) -> Result<Digest, ScratchIncumbentError> {
    let bytes = serde_cbor::to_vec(&sequence).map_err(incumbent_display)?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScratchIncumbentError(String);

impl fmt::Display for ScratchIncumbentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ScratchIncumbentError {}

fn incumbent_error(message: impl Into<String>) -> ScratchIncumbentError {
    ScratchIncumbentError(message.into())
}

fn incumbent_display(error: impl fmt::Display) -> ScratchIncumbentError {
    incumbent_error(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_ids_round_trip_and_promote_without_index_geometry() {
        let coarse = scratch_action_catalog_with_heading_count(16).unwrap();
        let options = vec![
            "scratch.camera_move.h03.t08.l1".to_owned(),
            "scratch.roll.h15.r03".to_owned(),
        ];
        let actions = option_ids_to_actions(&options, &coarse).unwrap();
        let loaded = load_option_ids(
            Digest([1; 32]),
            7,
            16,
            242,
            action_sequence_sha256(&actions).unwrap(),
            &options,
            coarse.action_schema_sha256(),
            32,
        )
        .unwrap();
        let fine = scratch_action_catalog_with_heading_count(32).unwrap();
        assert_eq!(
            fine.entries()[loaded.incumbent_action_sequence[0]].option_id(),
            "scratch.camera_move.h06.t08.l1"
        );
        assert_eq!(
            fine.entries()[loaded.incumbent_action_sequence[1]].option_id(),
            "scratch.roll.h30.r03"
        );
    }
}
