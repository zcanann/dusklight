//! Learned-Q ordering for continuous residual trajectory proposals.
//!
//! Residual search remains responsible for producing lossless raw-PAD tapes.
//! This module only changes the order in which a finite batch is considered:
//! it evaluates the realized action at each affected frame against the exact
//! observed parent state. Native execution and authored terminal predicates
//! remain the only outcome and promotion authorities.

use crate::artifact::Digest;
use crate::fqi::FittedQ;
use crate::offline_rl::MovementActionSchema;
use crate::transition_corpus::TransitionCorpus;
use dusklight_search::residual_optimizer::{ResidualProposal, ResidualProposalBatch};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const RESIDUAL_CRITIC_RANKING_SCHEMA_V1: &str = "dusklight-residual-critic-ranking/v1";
const MAX_RANKING_PROPOSALS: usize = 16_384;

#[derive(Clone, Copy, Debug)]
pub struct ResidualCriticRankingConfig {
    /// Absolute incumbent tape frame represented by transition row zero.
    pub parent_corpus_start_frame: u64,
    /// Penalizes tree disagreement. This is a ranking heuristic, not a
    /// calibrated confidence bound.
    pub uncertainty_penalty: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCriticProposalScore {
    pub rank: usize,
    pub generation: u32,
    pub sample_index: u32,
    pub candidate_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub affected_frames: usize,
    pub scored_frames: usize,
    pub unsupported_action_frames: usize,
    pub conservative_mean_advantage: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCriticRankingReport {
    pub schema: &'static str,
    pub report_sha256: Digest,
    pub critic_sha256: Digest,
    pub parent_corpus_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub parent_corpus_start_frame: u64,
    pub uncertainty_penalty: f64,
    pub proposals: Vec<ResidualCriticProposalScore>,
    pub exact_simulation_authority: bool,
    pub route_authority: bool,
    pub promotion_authority: bool,
}

#[derive(Debug)]
pub struct ResidualCriticRankingError(String);

impl ResidualCriticRankingError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ResidualCriticRankingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualCriticRankingError {}

/// Ranks one already-generated residual batch and returns every proposal in
/// advisory order together with a content-sealed explanation of that order.
pub fn rank_residual_proposals(
    batch: ResidualProposalBatch,
    parent_corpus: &TransitionCorpus,
    critic: &FittedQ,
    config: ResidualCriticRankingConfig,
) -> Result<(ResidualProposalBatch, ResidualCriticRankingReport), ResidualCriticRankingError> {
    if batch.proposals.is_empty() || batch.proposals.len() > MAX_RANKING_PROPOSALS {
        return Err(ResidualCriticRankingError::new(
            "critic ranking requires a bounded nonempty residual batch",
        ));
    }
    if !config.uncertainty_penalty.is_finite() || config.uncertainty_penalty < 0.0 {
        return Err(ResidualCriticRankingError::new(
            "critic uncertainty penalty must be finite and nonnegative",
        ));
    }
    parent_corpus
        .validate()
        .map_err(|error| ResidualCriticRankingError::new(error.to_string()))?;
    let action_schema =
        MovementActionSchema::from_digest(parent_corpus.action_schema).ok_or_else(|| {
            ResidualCriticRankingError::new(
                "critic parent corpus does not use a supported movement-action schema",
            )
        })?;
    if critic.feature_width() != parent_corpus.feature_count as usize
        || critic.actions().is_empty()
        || critic
            .actions()
            .iter()
            .any(|action| *action >= action_schema.action_count())
    {
        return Err(ResidualCriticRankingError::new(
            "critic feature width or action support differs from its parent corpus",
        ));
    }
    let parent_end_frame = config
        .parent_corpus_start_frame
        .checked_add(parent_corpus.transitions.len() as u64)
        .ok_or_else(|| ResidualCriticRankingError::new("parent corpus frame range overflowed"))?;

    let critic_sha256 = critic_digest(critic)?;
    let parent_corpus_sha256 = parent_corpus
        .content_digest()
        .map_err(|error| ResidualCriticRankingError::new(error.to_string()))?;
    let mut scored = batch
        .proposals
        .into_iter()
        .map(|proposal| {
            score_proposal(
                proposal,
                parent_corpus,
                critic,
                action_schema,
                config,
                parent_end_frame,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    scored.sort_by(|(left, _), (right, _)| {
        (right.scored_frames > 0)
            .cmp(&(left.scored_frames > 0))
            .then_with(|| {
                right
                    .conservative_mean_advantage
                    .total_cmp(&left.conservative_mean_advantage)
            })
            .then_with(|| right.scored_frames.cmp(&left.scored_frames))
            .then_with(|| {
                left.unsupported_action_frames
                    .cmp(&right.unsupported_action_frames)
            })
            .then_with(|| left.candidate_sha256.cmp(&right.candidate_sha256))
    });

    let proposals = scored
        .iter()
        .enumerate()
        .map(|(rank, (score, _))| ResidualCriticProposalScore {
            rank,
            generation: score.generation,
            sample_index: score.sample_index,
            candidate_sha256: score.candidate_sha256,
            realized_tape_sha256: score.realized_tape_sha256,
            affected_frames: score.affected_frames,
            scored_frames: score.scored_frames,
            unsupported_action_frames: score.unsupported_action_frames,
            conservative_mean_advantage: score.conservative_mean_advantage,
        })
        .collect::<Vec<_>>();
    let mut report = ResidualCriticRankingReport {
        schema: RESIDUAL_CRITIC_RANKING_SCHEMA_V1,
        report_sha256: Digest::ZERO,
        critic_sha256,
        parent_corpus_sha256,
        feature_schema_sha256: parent_corpus.feature_schema,
        action_schema_sha256: parent_corpus.action_schema,
        parent_corpus_start_frame: config.parent_corpus_start_frame,
        uncertainty_penalty: config.uncertainty_penalty,
        proposals,
        exact_simulation_authority: true,
        route_authority: false,
        promotion_authority: false,
    };
    report.report_sha256 = report_identity(&report)?;
    let ranked_batch = ResidualProposalBatch {
        proposals: scored.into_iter().map(|(_, proposal)| proposal).collect(),
        rejected_invalid: batch.rejected_invalid,
        rejected_duplicate_tape: batch.rejected_duplicate_tape,
    };
    Ok((ranked_batch, report))
}

#[derive(Clone, Debug)]
struct UnrankedScore {
    generation: u32,
    sample_index: u32,
    candidate_sha256: Digest,
    realized_tape_sha256: Digest,
    affected_frames: usize,
    scored_frames: usize,
    unsupported_action_frames: usize,
    conservative_mean_advantage: f64,
}

fn score_proposal(
    proposal: ResidualProposal,
    parent_corpus: &TransitionCorpus,
    critic: &FittedQ,
    action_schema: MovementActionSchema,
    config: ResidualCriticRankingConfig,
    parent_end_frame: u64,
) -> Result<(UnrankedScore, ResidualProposal), ResidualCriticRankingError> {
    proposal
        .candidate
        .validate()
        .map_err(|error| ResidualCriticRankingError::new(error.to_string()))?;
    let span = proposal.compiled.report.intervention_span;
    if span.start_frame < config.parent_corpus_start_frame
        || span.end_frame_exclusive > parent_end_frame
        || span.end_frame_exclusive > proposal.compiled.tape.frames.len() as u64
        || span.start_frame >= span.end_frame_exclusive
    {
        return Err(ResidualCriticRankingError::new(
            "residual intervention is not covered by the exact parent-state corpus",
        ));
    }
    let mut advantage = 0.0;
    let mut scored_frames = 0_usize;
    let mut unsupported_action_frames = 0_usize;
    for frame in span.start_frame..span.end_frame_exclusive {
        let row = (frame - config.parent_corpus_start_frame) as usize;
        let transition = &parent_corpus.transitions[row];
        let pad = proposal.compiled.tape.frames[frame as usize].pads[0];
        let Some(action) = action_schema.action_id(pad) else {
            unsupported_action_frames += 1;
            continue;
        };
        if !critic.actions().contains(&action)
            || !critic.actions().contains(&transition.action.action_id)
        {
            unsupported_action_frames += 1;
            continue;
        }
        let proposed = critic
            .estimate(&transition.state, action)
            .map_err(|error| ResidualCriticRankingError::new(error.to_string()))?;
        let incumbent = critic
            .estimate(&transition.state, transition.action.action_id)
            .map_err(|error| ResidualCriticRankingError::new(error.to_string()))?;
        let conservative =
            |mean: f64, variance: f64| mean - config.uncertainty_penalty * variance.max(0.0).sqrt();
        advantage += conservative(proposed.mean, proposed.variance)
            - conservative(incumbent.mean, incumbent.variance);
        scored_frames += 1;
    }
    let affected_frames = usize::try_from(span.end_frame_exclusive - span.start_frame)
        .map_err(|_| ResidualCriticRankingError::new("affected-frame count overflowed"))?;
    let score = UnrankedScore {
        generation: proposal.generation,
        sample_index: proposal.sample_index,
        candidate_sha256: proposal.candidate.content_sha256,
        realized_tape_sha256: proposal.compiled.report.realized_tape_sha256,
        affected_frames,
        scored_frames,
        unsupported_action_frames,
        conservative_mean_advantage: if scored_frames == 0 {
            0.0
        } else {
            advantage / scored_frames as f64
        },
    };
    Ok((score, proposal))
}

fn critic_digest(critic: &FittedQ) -> Result<Digest, ResidualCriticRankingError> {
    let bytes = serde_json::to_vec(critic)
        .map_err(|error| ResidualCriticRankingError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.residual-ranking-critic/v1\0");
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn report_identity(
    report: &ResidualCriticRankingReport,
) -> Result<Digest, ResidualCriticRankingError> {
    let mut canonical = report.clone();
    canonical.report_sha256 = Digest::ZERO;
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| ResidualCriticRankingError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.residual-critic-ranking/v1\0");
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fqi::{FqiConfig, Transition as FqiTransition};
    use crate::observation_view::movement_state_v2_spec;
    use crate::offline_rl::movement_action_schema_digest_v2;
    use crate::tape::{InputFrame, InputTape, RawPadState};
    use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};
    use dusklight_search::residual_action::{
        AnalogChannel, AnalogResidual, ButtonResidual, ButtonResidualMode, ResidualCandidate,
        TemporalBasis, compile_residual_candidate,
    };

    fn reference(byte: u8) -> StateReference {
        StateReference {
            kind: StateReferenceKind::Boundary,
            digest: Digest([byte; 32]),
        }
    }

    fn parent() -> (InputTape, Vec<u8>) {
        let tape = InputTape {
            frames: (0..64)
                .map(|_| InputFrame {
                    owned_ports: 1,
                    pads: [
                        RawPadState {
                            connected: true,
                            ..RawPadState::default()
                        },
                        RawPadState::default(),
                        RawPadState::default(),
                        RawPadState::default(),
                    ],
                    ..InputFrame::default()
                })
                .collect(),
            ..InputTape::default()
        };
        let bytes = tape.encode().unwrap();
        (tape, bytes)
    }

    fn corpus() -> TransitionCorpus {
        let feature_schema = movement_state_v2_spec().digest().unwrap();
        let transitions = (0..64)
            .map(|index| Transition {
                source: reference(index as u8 + 1),
                state: vec![index as f32],
                action: MacroAction {
                    action_id: 0,
                    macro_kind: MovementActionSchema::V2.macro_kind(),
                    parameters: vec![0, 0, 0],
                },
                duration_ticks: 1,
                reward: -1.0,
                next: reference(index as u8 + 2),
                next_state: vec![index as f32 + 1.0],
                terminal: index == 63,
            })
            .collect();
        TransitionCorpus::new(
            feature_schema,
            movement_action_schema_digest_v2(),
            1,
            transitions,
        )
        .unwrap()
    }

    fn critic() -> FittedQ {
        let mut transitions = Vec::new();
        for state in 0..64 {
            transitions.push(FqiTransition {
                state: vec![state as f32],
                action: 0,
                duration: 1,
                reward: -1.0,
                next_state: vec![state as f32 + 1.0],
                terminal: true,
            });
            transitions.push(FqiTransition {
                state: vec![state as f32],
                action: 1,
                duration: 1,
                reward: 10.0,
                next_state: vec![state as f32 + 1.0],
                terminal: true,
            });
        }
        FittedQ::fit(
            1,
            &[0, 1],
            &transitions,
            &FqiConfig {
                iterations: 1,
                trees_per_action: 3,
                bootstrap: false,
                ..FqiConfig::default()
            },
        )
        .unwrap()
    }

    fn proposal(
        parent: &InputTape,
        parent_bytes: &[u8],
        sample_index: u32,
        delta: i16,
    ) -> ResidualProposal {
        let candidate = ResidualCandidate::seal(
            parent_bytes,
            vec![AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainY,
                basis: TemporalBasis::ExactFrame { frame: 8, delta },
            }],
            vec![],
        )
        .unwrap();
        let compiled = compile_residual_candidate(parent, parent_bytes, &candidate).unwrap();
        ResidualProposal {
            generation: 0,
            sample_index,
            genome: dusklight_search::residual_optimizer::ResidualGenome { genes: vec![] },
            candidate,
            compiled,
        }
    }

    fn unsupported_button_proposal(
        parent: &InputTape,
        parent_bytes: &[u8],
        sample_index: u32,
    ) -> ResidualProposal {
        let candidate = ResidualCandidate::seal(
            parent_bytes,
            vec![],
            vec![ButtonResidual {
                port: 0,
                buttons: 0x0400,
                start_frame: 8,
                duration_frames: 1,
                mode: ButtonResidualMode::Press,
            }],
        )
        .unwrap();
        let compiled = compile_residual_candidate(parent, parent_bytes, &candidate).unwrap();
        ResidualProposal {
            generation: 0,
            sample_index,
            genome: dusklight_search::residual_optimizer::ResidualGenome { genes: vec![] },
            candidate,
            compiled,
        }
    }

    #[test]
    fn learned_critic_orders_continuous_residuals_without_claiming_authority() {
        let (parent, bytes) = parent();
        // Positive Y maps to action 1; zero remains action 0. The smaller
        // negative perturbation stays within PADClamp's neutral deadzone.
        let weak = proposal(&parent, &bytes, 0, -1);
        let strong = proposal(&parent, &bytes, 1, 127);
        let batch = ResidualProposalBatch {
            proposals: vec![weak, strong],
            rejected_invalid: 3,
            rejected_duplicate_tape: 2,
        };
        let (ranked, report) = rank_residual_proposals(
            batch,
            &corpus(),
            &critic(),
            ResidualCriticRankingConfig {
                parent_corpus_start_frame: 0,
                uncertainty_penalty: 0.5,
            },
        )
        .unwrap();
        assert_eq!(ranked.proposals[0].sample_index, 1);
        assert_eq!(ranked.rejected_invalid, 3);
        assert_eq!(ranked.rejected_duplicate_tape, 2);
        assert!(report.proposals[0].conservative_mean_advantage > 0.0);
        assert_eq!(report.proposals[0].scored_frames, 1);
        assert!(report.exact_simulation_authority);
        assert!(!report.route_authority);
        assert!(!report.promotion_authority);
        assert_ne!(report.report_sha256, Digest::ZERO);
    }

    #[test]
    fn proposals_outside_parent_state_coverage_fail_closed() {
        let (parent, bytes) = parent();
        let batch = ResidualProposalBatch {
            proposals: vec![proposal(&parent, &bytes, 0, 127)],
            rejected_invalid: 0,
            rejected_duplicate_tape: 0,
        };
        let error = rank_residual_proposals(
            batch,
            &corpus(),
            &critic(),
            ResidualCriticRankingConfig {
                parent_corpus_start_frame: 9,
                uncertainty_penalty: 0.0,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("not covered"));
    }

    #[test]
    fn unsupported_continuous_surface_is_retained_behind_scored_proposals() {
        let (parent, bytes) = parent();
        let batch = ResidualProposalBatch {
            proposals: vec![
                unsupported_button_proposal(&parent, &bytes, 0),
                proposal(&parent, &bytes, 1, 127),
            ],
            rejected_invalid: 0,
            rejected_duplicate_tape: 0,
        };
        let (ranked, report) = rank_residual_proposals(
            batch,
            &corpus(),
            &critic(),
            ResidualCriticRankingConfig {
                parent_corpus_start_frame: 0,
                uncertainty_penalty: 0.0,
            },
        )
        .unwrap();
        assert_eq!(ranked.proposals[0].sample_index, 1);
        assert_eq!(ranked.proposals[1].sample_index, 0);
        assert_eq!(report.proposals[1].scored_frames, 0);
        assert_eq!(report.proposals[1].unsupported_action_frames, 1);
    }
}
