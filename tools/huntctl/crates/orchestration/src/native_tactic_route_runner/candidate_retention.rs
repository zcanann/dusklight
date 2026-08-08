use super::*;

pub(super) fn retain_successful_result(
    root: &Path,
    decision_index: u64,
    candidate: &TacticQFinalResult,
    best: &mut Option<TacticQFinalResult>,
) -> Result<(), NativeTacticRouteRunError> {
    write_retained_result(root, decision_index, candidate)?;
    if best
        .as_ref()
        .is_some_and(|incumbent| !successful_result_is_better(candidate, incumbent))
    {
        return Ok(());
    }
    *best = Some(candidate.clone());
    Ok(())
}

pub(super) fn synchronize_graph_terminal_result(
    root: &Path,
    decision_index: u64,
    campaign: &TacticQCampaign,
    best: &mut Option<TacticQFinalResult>,
) -> Result<(), NativeTacticRouteRunError> {
    if let Some(result) = best.as_ref() {
        if campaign
            .final_result_matches_graph_terminal(result)
            .map_err(route_error)?
        {
            return Ok(());
        }
    }
    let graph_result = campaign
        .final_result_from_graph_best_terminal()
        .map_err(route_error)?;
    let Some(graph_result) = graph_result else {
        if best.is_some() {
            return Err(route_message(
                "retained terminal artifact exists without graph terminal authority",
            ));
        }
        return Ok(());
    };
    write_retained_result(root, decision_index, &graph_result)?;
    *best = Some(graph_result);
    Ok(())
}

fn write_retained_result(
    root: &Path,
    decision_index: u64,
    candidate: &TacticQFinalResult,
) -> Result<(), NativeTacticRouteRunError> {
    let path = root.join(format!(
        "success-{:06}-{decision_index:06}-{}.dtqz",
        candidate.route_tape.frames.len(),
        candidate.content_sha256
    ));
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            let retained = TacticQFinalResult::read(&path).map_err(route_error)?;
            if retained != *candidate {
                return Err(route_message(
                    "retained terminal path collides with different evidence",
                ));
            }
            return Ok(());
        }
        Ok(_) => {
            return Err(route_message(
                "retained terminal path is not a physical file",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(route_error(error)),
    }
    candidate.write(&path).map_err(route_error)?;
    Ok(())
}

pub(super) fn load_best_retained_success(
    root: &Path,
    execution_authority_sha256: Digest,
    objective_sha256: Digest,
    root_checkpoint_sha256: Digest,
) -> Result<Option<TacticQFinalResult>, NativeTacticRouteRunError> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(route_error(error)),
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "retained-success root is not a physical directory",
        ));
    }
    let mut paths = fs::read_dir(root)
        .map_err(route_error)?
        .map(|entry| entry.map(|entry| entry.path()).map_err(route_error))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let maximum_retained_successes = (MAX_ROUTE_DECISIONS as usize)
        .checked_mul(MAX_TACTIC_PROPOSALS_PER_DECISION)
        .ok_or_else(|| route_message("retained-success limit overflowed"))?;
    if paths.len() > maximum_retained_successes {
        return Err(route_message("retained-success directory is oversized"));
    }
    let mut best = None;
    for path in paths {
        if path.extension().and_then(|extension| extension.to_str()) != Some("dtqz") {
            return Err(route_message(
                "retained-success directory contains an unexpected artifact",
            ));
        }
        let result = TacticQFinalResult::read(&path).map_err(route_error)?;
        if result.execution_authority_sha256 != execution_authority_sha256
            || result.objective_sha256 != objective_sha256
            || result.root_checkpoint_sha256 != root_checkpoint_sha256
        {
            return Err(route_message(
                "retained-success artifact is detached from the campaign",
            ));
        }
        if best
            .as_ref()
            .is_none_or(|incumbent| successful_result_is_better(&result, incumbent))
        {
            best = Some(result);
        }
    }
    Ok(best)
}

pub(super) fn successful_result_is_better(
    candidate: &TacticQFinalResult,
    incumbent: &TacticQFinalResult,
) -> bool {
    successful_route_rank_is_better(
        candidate.route_tape.frames.len(),
        candidate.content_sha256,
        incumbent.route_tape.frames.len(),
        incumbent.content_sha256,
    )
}

pub(super) fn route_frames_first_hit_tick(route_frames: u64, source_frame: u64) -> Option<u64> {
    route_frames
        .checked_sub(source_frame)
        .and_then(|route_ticks| route_ticks.checked_sub(1))
}

#[cfg(test)]
pub(super) fn route_frames_promote(
    route_frames: u64,
    source_frame: u64,
    promotion_before_tick: u64,
) -> bool {
    route_frames_first_hit_tick(route_frames, source_frame)
        .is_some_and(|first_hit_tick| first_hit_tick < promotion_before_tick)
}

pub(super) fn successful_route_rank_is_better(
    candidate_frames: usize,
    candidate_sha256: Digest,
    incumbent_frames: usize,
    incumbent_sha256: Digest,
) -> bool {
    candidate_frames < incumbent_frames
        || (candidate_frames == incumbent_frames && candidate_sha256.0 < incumbent_sha256.0)
}
