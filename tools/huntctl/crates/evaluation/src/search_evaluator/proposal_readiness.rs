//! Admission checks for learned proposals entering anchored evaluation.

use super::*;

pub(super) fn required_native_facts_supported(attempts: &[AttemptEvidence]) -> bool {
    if attempts
        .iter()
        .any(|attempt| attempt.harness_terminal.is_some())
    {
        return native_terminals_support_required_facts(
            attempts.iter().map(|attempt| attempt.harness_terminal),
        );
    }

    // Anchored route evaluation predates HarnessRunResult, but its first
    // repetition extracts the same authenticated observation view into a
    // sealed transition corpus. That successful extraction is direct evidence
    // that the required native facts and channel ABI were available. Later
    // repetitions intentionally omit traces and must not negate it.
    !attempts.is_empty()
        && attempts
            .iter()
            .all(|attempt| attempt.infrastructure_error.is_none())
        && attempts
            .iter()
            .any(|attempt| attempt.transition_corpus.is_some())
}

pub(super) fn native_terminals_support_required_facts(
    terminals: impl IntoIterator<Item = Option<HarnessTerminalReason>>,
) -> bool {
    let mut observed = false;
    for terminal in terminals {
        observed = true;
        if matches!(
            terminal,
            None | Some(
                HarnessTerminalReason::Unsupported | HarnessTerminalReason::CapabilityMismatch
            )
        ) {
            return false;
        }
    }
    observed
}

pub(super) fn learned_holdout_scores_adequate(
    rows: impl IntoIterator<Item = (bool, LexicographicScore)>,
) -> bool {
    let mut best_learned: Option<LexicographicScore> = None;
    let mut best_baseline: Option<LexicographicScore> = None;
    for (learned, score) in rows {
        let best = if learned {
            &mut best_learned
        } else {
            &mut best_baseline
        };
        if best.as_ref().is_none_or(|incumbent| score > *incumbent) {
            *best = Some(score);
        }
    }
    matches!((best_learned, best_baseline), (Some(learned), Some(baseline)) if learned >= baseline)
}

pub(super) fn learned_proposal_held_out_performance(
    manifest: &PopulationManifest,
    leaderboard: &[LeaderboardEntry],
) -> bool {
    let member_by_id = manifest
        .members
        .iter()
        .map(|member| (member.candidate_id.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    learned_holdout_scores_adequate(leaderboard.iter().filter_map(|row| {
        let member = member_by_id.get(row.candidate_id.as_str())?;
        let learned = member.ancestry.mutation.as_deref().is_some_and(|mutation| {
            mutation.starts_with("q_guided")
                || mutation.starts_with("q_temporal_consensus")
                || mutation.starts_with("q_disagreement_heuristic")
        });
        Some((learned, row.score))
    }))
}
