//! Authenticated native campaign episodes projected into immutable replay generations.

use crate::native_residual_campaign::NativeResidualCampaignEvaluation;
use crate::native_suffix_worker::ValidatedNativeFrozenPolicyBatch;
use crate::optimization_request::OptimizationRequest;
use crate::residual_campaign::ResidualReplayCheckpoint;
use crate::residual_campaign_runner::{artifact_reference, read_artifact, write_exact_or_new};
use dusklight_evidence::native_episode_shard::{
    NativeEpisodeShard, authored_milestone_objective_identity,
};
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_learning::native_frozen_policy_suffix_batch::NativeFrozenPolicySuffixBatch;
use dusklight_learning::native_goal_frozen_policy::{
    NativeGoalFrozenPolicyAdmission, NativeGoalFrozenPolicyManifest,
};
use dusklight_learning::native_replay_corpus::{
    NativeReplayCorpus, ReplayEpisodeSource, ReplayExperienceRole,
};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::Path;

#[derive(Clone, Debug)]
struct ReplayAddition {
    shard_path: String,
    episode_id: String,
    checkpoint_identity: String,
    simulated_ticks: u64,
    first_hit_tick: Option<u64>,
}

pub(crate) struct PolicyReplayRollout<'a> {
    pub batch: &'a NativeFrozenPolicySuffixBatch,
    pub validated: &'a ValidatedNativeFrozenPolicyBatch,
    pub shard: &'a NativeEpisodeShard,
}

/// Appends every exact native attempt in one completed residual generation to
/// a cumulative immutable replay corpus. Residual proposals are randomized
/// coverage experience: terminal success remains an outcome, not a role
/// reclassification. Policy, demonstration, and alternate-terminal producers
/// use the same corpus contract through their own collection authorities.
pub(crate) fn append_residual_replay_generation(
    root: &Path,
    campaign: &Path,
    optimization: &OptimizationRequest,
    previous: Option<&ResidualReplayCheckpoint>,
    evaluations: &[NativeResidualCampaignEvaluation],
) -> Result<ResidualReplayCheckpoint, CampaignReplayError> {
    if evaluations.is_empty() {
        return Err(replay_message(
            "residual replay generation has no completed evaluations",
        ));
    }
    let previous = previous
        .map(|replay| {
            let corpus = load_corpus(root, &replay.artifact)?;
            replay.validate_corpus(&corpus).map_err(replay_error)?;
            Ok(corpus)
        })
        .transpose()?;
    if let Some(previous) = &previous {
        validate_residual_corpus_scope(optimization, previous)?;
    }
    let mut additions = Vec::new();
    let mut shard_references = BTreeMap::new();
    let mut episode_keys = BTreeSet::new();
    for evaluation in evaluations {
        if evaluation.optimization_request_sha256 != optimization.content_sha256 {
            return Err(replay_message(
                "residual replay evaluation differs from its optimization request",
            ));
        }
        for attempt in &evaluation.attempts {
            let prior = shard_references.insert(
                attempt.episode_shard.path.clone(),
                attempt.episode_shard.clone(),
            );
            if prior
                .as_ref()
                .is_some_and(|prior| prior != &attempt.episode_shard)
            {
                return Err(replay_message(
                    "residual replay shard path has conflicting artifact identities",
                ));
            }
            if !episode_keys.insert((
                attempt.episode_shard.sha256,
                attempt.wire_candidate_id.clone(),
            )) {
                return Err(replay_message(
                    "residual replay generation repeats an authenticated episode",
                ));
            }
            additions.push(ReplayAddition {
                shard_path: attempt.episode_shard.path.clone(),
                episode_id: attempt.wire_candidate_id.clone(),
                checkpoint_identity: attempt.restore_identity.clone(),
                simulated_ticks: attempt.simulated_ticks,
                first_hit_tick: attempt.first_hit_tick,
            });
        }
    }

    let shards = shard_references
        .into_iter()
        .map(|(path, reference)| {
            let bytes = read_artifact(root, &reference).map_err(replay_error)?;
            let shard = NativeEpisodeShard::decode(&bytes).map_err(replay_error)?;
            if shard.content_sha256 != reference.sha256
                || shard.source_frame != optimization.route.source_boundary_index
                || u64::from(shard.maximum_ticks) != optimization.budgets.exploration_horizon_ticks
                || shard.metadata.source_boundary_fingerprint
                    != optimization.route.native_source_boundary_fingerprint
                || shard.metadata.objective != optimization.terminal_predicate.goal
                || shard.metadata.policy_model.is_some()
            {
                return Err(replay_message(
                    "residual replay shard is detached from its campaign source",
                ));
            }
            shard
                .verify_authored_objective(
                    &optimization.terminal_predicate.program_sha256.to_string(),
                    &optimization
                        .terminal_predicate
                        .definition_sha256
                        .to_string(),
                )
                .map_err(replay_error)?;
            Ok((path, shard))
        })
        .collect::<Result<BTreeMap<_, _>, CampaignReplayError>>()?;
    let sources = additions
        .iter()
        .map(|addition| {
            let shard = shards
                .get(&addition.shard_path)
                .ok_or_else(|| replay_message("residual replay shard disappeared"))?;
            let episode_index = shard
                .episodes
                .iter()
                .position(|episode| episode.id == addition.episode_id)
                .ok_or_else(|| {
                    replay_message("residual replay episode is absent from its native shard")
                })?;
            let episode = &shard.episodes[episode_index];
            let exact_first_hit = episode
                .first_hit_tick
                .map(|tick| u64::from(tick).saturating_add(1));
            if shard.metadata.checkpoint_identity != addition.checkpoint_identity
                || u64::from(episode.ticks_executed) != addition.simulated_ticks
                || exact_first_hit != addition.first_hit_tick
            {
                return Err(replay_message(
                    "residual replay episode differs from its exact native evaluation",
                ));
            }
            Ok(ReplayEpisodeSource {
                shard,
                episode_index,
                role: ReplayExperienceRole::RandomizedCoverage,
                policy_lineage_sha256: None,
                parent_entry_sha256: None,
            })
        })
        .collect::<Result<Vec<_>, CampaignReplayError>>()?;
    let corpus = NativeReplayCorpus::build(previous.as_ref(), &sources).map_err(replay_error)?;
    let bytes = pretty_json(&corpus)?;
    let path = campaign.join("replay").join(format!(
        "generation-{:08}-{}.json",
        corpus.generation, corpus.corpus_sha256
    ));
    write_exact_or_new(&path, &bytes).map_err(replay_error)?;
    let artifact = artifact_reference(root, &path).map_err(replay_error)?;
    ResidualReplayCheckpoint::seal(artifact, &corpus).map_err(replay_error)
}

/// Appends one fully reinferred native policy generation. The trained manifest
/// is the replay role's policy lineage; the `.dsfrozen` model, online request,
/// exact result, and v3 shard must all agree before any episode is admitted.
pub(crate) fn append_policy_replay_generation(
    root: &Path,
    campaign: &Path,
    optimization: &OptimizationRequest,
    previous: &ResidualReplayCheckpoint,
    manifest: &NativeGoalFrozenPolicyManifest,
    model_bytes: &[u8],
    rollouts: &[PolicyReplayRollout<'_>],
) -> Result<ResidualReplayCheckpoint, CampaignReplayError> {
    let previous_corpus = load_corpus(root, &previous.artifact)?;
    previous
        .validate_corpus(&previous_corpus)
        .map_err(replay_error)?;
    let corpus = build_policy_replay_generation(
        optimization,
        &previous_corpus,
        manifest,
        model_bytes,
        rollouts,
    )?;
    let bytes = pretty_json(&corpus)?;
    let path = campaign.join("replay").join(format!(
        "generation-{:08}-{}.json",
        corpus.generation, corpus.corpus_sha256
    ));
    write_exact_or_new(&path, &bytes).map_err(replay_error)?;
    let artifact = artifact_reference(root, &path).map_err(replay_error)?;
    ResidualReplayCheckpoint::seal(artifact, &corpus).map_err(replay_error)
}

pub(crate) fn build_policy_replay_generation(
    optimization: &OptimizationRequest,
    previous: &NativeReplayCorpus,
    manifest: &NativeGoalFrozenPolicyManifest,
    model_bytes: &[u8],
    rollouts: &[PolicyReplayRollout<'_>],
) -> Result<NativeReplayCorpus, CampaignReplayError> {
    previous.validate().map_err(replay_error)?;
    manifest.validate(model_bytes).map_err(replay_error)?;
    if rollouts.is_empty()
        || manifest.admission != NativeGoalFrozenPolicyAdmission::FrozenPolicyCandidate
        || manifest.source_replay_corpus_sha256 != previous.corpus_sha256
        || manifest.objective_sha256 != optimization.terminal_predicate.definition_sha256
        || manifest.goal_program_sha256 != optimization.terminal_predicate.program_sha256
        || manifest.observation_schema != previous.observation_schema
        || manifest.action_schema != previous.action_schema
    {
        return Err(replay_message(
            "policy replay manifest is unadmitted or detached from its source corpus and objective",
        ));
    }
    let objective_identity = authored_milestone_objective_identity(
        &optimization.terminal_predicate.program_sha256.to_string(),
        &optimization
            .terminal_predicate
            .definition_sha256
            .to_string(),
    )
    .map_err(replay_error)?;
    if manifest.goal_objective_identity != objective_identity {
        return Err(replay_message(
            "policy replay manifest differs from the authored objective identity",
        ));
    }

    let mut candidate_ids = BTreeSet::new();
    let mut checkpoint_identities = BTreeSet::new();
    let mut sources = Vec::with_capacity(rollouts.len());
    for rollout in rollouts {
        rollout.batch.validate(model_bytes).map_err(replay_error)?;
        rollout
            .validated
            .reinference
            .validate()
            .map_err(replay_error)?;
        let candidate = rollout
            .validated
            .execution
            .candidates
            .first()
            .filter(|_| rollout.validated.execution.candidates.len() == 1)
            .ok_or_else(|| replay_message("policy replay rollout has no exact candidate"))?;
        let episode = rollout
            .shard
            .episodes
            .first()
            .filter(|_| rollout.shard.episodes.len() == 1)
            .ok_or_else(|| replay_message("policy replay shard has no exact episode"))?;
        if rollout.batch.candidates.len() != 1
            || rollout.batch.candidates[0].id != candidate.id
            || candidate.id != episode.id
            || !candidate_ids.insert(candidate.id.as_str())
            || rollout.batch.source_frame as u64 != optimization.route.source_boundary_index
            || rollout.batch.source_boundary_fingerprint
                != optimization.route.native_source_boundary_fingerprint
            || rollout.batch.maximum_ticks as u64 != optimization.budgets.exploration_horizon_ticks
            || rollout.shard.source_frame != optimization.route.source_boundary_index
            || u64::from(rollout.shard.maximum_ticks)
                != optimization.budgets.exploration_horizon_ticks
            || rollout.shard.metadata.source_boundary_fingerprint
                != optimization.route.native_source_boundary_fingerprint
            || rollout.shard.metadata.objective != optimization.terminal_predicate.goal
            || rollout.shard.metadata.objective_identity != objective_identity
            || rollout.shard.metadata.observation_schema != manifest.observation_schema
            || rollout.shard.metadata.action_schema != manifest.action_schema
            || rollout.validated.execution.episode_shard_path.is_empty()
            || rollout.validated.execution.restore_identity
                != rollout.shard.metadata.checkpoint_identity
            || rollout.validated.execution.simulated_ticks != u64::from(episode.ticks_executed)
            || candidate.simulated_ticks != u64::from(episode.ticks_executed)
            || candidate.first_hit_tick
                != episode
                    .first_hit_tick
                    .map(|tick| u64::from(tick).saturating_add(1))
            || rollout.validated.reinference.shard_content_sha256 != rollout.shard.content_sha256
            || rollout.validated.reinference.model_xxh3_128 != manifest.frozen_model_xxh3_128
            || rollout.validated.reinference.feature_schema_sha256 != manifest.feature_schema_sha256
            || rollout.validated.reinference.action_schema_sha256
                != manifest.factorized_action_schema_sha256
            || rollout.validated.reinference.objective_sha256 != manifest.objective_sha256
            || rollout.validated.reinference.checkpoint_identity
                != rollout.shard.metadata.checkpoint_identity
            || rollout.validated.reinference.source_boundary_fingerprint
                != rollout.shard.metadata.source_boundary_fingerprint
            || rollout.validated.reinference.episode_count != 1
            || rollout.validated.reinference.transition_count
                != usize::try_from(episode.ticks_executed).map_err(replay_error)?
        {
            return Err(replay_message(
                "policy replay rollout differs from its model, request, result, shard, or objective",
            ));
        }
        checkpoint_identities.insert(rollout.shard.metadata.checkpoint_identity.as_str());
        rollout
            .shard
            .verify_authored_objective(
                &optimization.terminal_predicate.program_sha256.to_string(),
                &optimization
                    .terminal_predicate
                    .definition_sha256
                    .to_string(),
            )
            .map_err(replay_error)?;
        sources.push(ReplayEpisodeSource {
            shard: rollout.shard,
            episode_index: 0,
            role: ReplayExperienceRole::PolicyRollout,
            policy_lineage_sha256: Some(manifest.manifest_sha256),
            parent_entry_sha256: None,
        });
    }
    if checkpoint_identities.len() != 1 {
        return Err(replay_message(
            "policy replay generation mixes native checkpoint identities",
        ));
    }
    NativeReplayCorpus::build(Some(previous), &sources).map_err(replay_error)
}

pub(crate) fn load_corpus(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<NativeReplayCorpus, CampaignReplayError> {
    let corpus: NativeReplayCorpus =
        serde_json::from_slice(&read_artifact(root, reference).map_err(replay_error)?)
            .map_err(replay_error)?;
    corpus.validate().map_err(replay_error)?;
    Ok(corpus)
}

pub(crate) fn validate_residual_corpus_scope(
    optimization: &OptimizationRequest,
    corpus: &NativeReplayCorpus,
) -> Result<(), CampaignReplayError> {
    corpus.validate().map_err(replay_error)?;
    let objective_identity = authored_milestone_objective_identity(
        &optimization.terminal_predicate.program_sha256.to_string(),
        &optimization
            .terminal_predicate
            .definition_sha256
            .to_string(),
    )
    .map_err(replay_error)?;
    if corpus.entries.iter().any(|entry| {
        entry.role != ReplayExperienceRole::RandomizedCoverage
            || entry.policy_lineage_sha256.is_some()
            || entry.parent_entry_sha256.is_some()
            || entry.source_frame != optimization.route.source_boundary_index
            || entry.source_boundary_fingerprint
                != optimization.route.native_source_boundary_fingerprint
            || entry.objective != optimization.terminal_predicate.goal
            || entry.objective_identity != objective_identity
    }) {
        return Err(replay_message(
            "residual replay corpus contains experience outside its campaign scope",
        ));
    }
    Ok(())
}

fn pretty_json(value: &impl serde::Serialize) -> Result<Vec<u8>, CampaignReplayError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(replay_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CampaignReplayError(String);

fn replay_message(message: impl Into<String>) -> CampaignReplayError {
    CampaignReplayError(message.into())
}

fn replay_error(error: impl fmt::Display) -> CampaignReplayError {
    replay_message(error.to_string())
}

impl fmt::Display for CampaignReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CampaignReplayError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_residual_campaign::NativeResidualAttempt;
    use dusklight_automation_contracts::artifact::Digest;
    use dusklight_search::residual_retention::{ExactTerminalVerdict, ResidualEvaluationEvidence};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    const PROGRAM_SHA256: &str = "b8cbfafaa025b883cecd2db4e4bef30696c801a591ce736d1281defd8af0c169";
    const DEFINITION_SHA256: &str =
        "631b025f41e16251e47f340fb0030fab07be15433204d2fdef8eb08915b11e57";
    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    fn repository() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap()
    }

    fn test_root() -> PathBuf {
        let nonce = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "dusklight-campaign-replay-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("build/campaigns/replay-test")).unwrap();
        root.canonicalize().unwrap()
    }

    fn optimization(shard: &NativeEpisodeShard) -> OptimizationRequest {
        let repository = repository();
        let mut optimization: OptimizationRequest = serde_json::from_slice(
            &fs::read(repository.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        optimization.route.source_boundary_index = shard.source_frame;
        optimization.route.native_source_boundary_fingerprint =
            shard.metadata.source_boundary_fingerprint.clone();
        optimization.budgets.exploration_horizon_ticks = u64::from(shard.maximum_ticks);
        optimization.terminal_predicate.goal = shard.metadata.objective.clone();
        optimization.terminal_predicate.program_sha256 = PROGRAM_SHA256.parse().unwrap();
        optimization.terminal_predicate.definition_sha256 = DEFINITION_SHA256.parse().unwrap();
        optimization
    }

    fn install_authored_fixture(path: &Path) {
        let fixture = repository().join("tests/fixtures/automation/native_episode_v14.dseps");
        let mut bytes = fs::read(fixture).unwrap();
        let placeholder = b"cccccccccccccccccccccccccccccccc";
        let objective = b"d0d98dc29bd4190312933ff7d10d9c11";
        let offsets = bytes
            .windows(placeholder.len())
            .enumerate()
            .filter_map(|(offset, window)| (window == placeholder).then_some(offset))
            .collect::<Vec<_>>();
        assert_eq!(offsets.len(), 1);
        bytes[offsets[0]..offsets[0] + objective.len()].copy_from_slice(objective);
        fs::write(path, bytes).unwrap();
    }

    fn evaluation(
        optimization: &OptimizationRequest,
        reference: &ArtifactReference,
        shard: &NativeEpisodeShard,
        episode_index: usize,
    ) -> NativeResidualCampaignEvaluation {
        let episode = &shard.episodes[episode_index];
        let first_hit_tick = episode
            .first_hit_tick
            .map(|tick| u64::from(tick).saturating_add(1));
        NativeResidualCampaignEvaluation {
            schema: "test-native-evaluation".into(),
            content_sha256: Digest([1; 32]),
            optimization_request_sha256: optimization.content_sha256,
            execution_binding_sha256: Digest([2; 32]),
            candidate_id: format!("candidate-{episode_index}"),
            candidate_sha256: Digest([3; 32]),
            realized_tape_sha256: Digest([4; 32]),
            attempts: vec![NativeResidualAttempt {
                repetition: 1,
                worker_seed: 7,
                wire_candidate_id: episode.id.clone(),
                batch_request: ArtifactReference {
                    path: "build/campaigns/replay-test/request.json".into(),
                    sha256: Digest([5; 32]),
                },
                batch_result: ArtifactReference {
                    path: "build/campaigns/replay-test/result.json".into(),
                    sha256: Digest([6; 32]),
                },
                episode_shard: reference.clone(),
                restore_identity: shard.metadata.checkpoint_identity.clone(),
                checkpoint_bytes: 1,
                simulated_ticks: u64::from(episode.ticks_executed),
                first_hit_tick,
                terminal_boundary_fingerprint: "7".repeat(32),
                behavior_sha256: Digest([8; 32]),
            }],
            simulated_ticks: u64::from(episode.ticks_executed),
            terminal_boundary_fingerprint: "7".repeat(32),
            evidence: ResidualEvaluationEvidence {
                candidate_sha256: Digest([3; 32]),
                realized_tape_sha256: Digest([4; 32]),
                terminal_program_sha256: optimization.terminal_predicate.program_sha256,
                terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
                evaluation_sha256: Digest([9; 32]),
                episode_sha256: Digest([10; 32]),
                behavior_sha256: Digest([8; 32]),
                verdict: first_hit_tick.map_or(ExactTerminalVerdict::Miss, |first_hit_tick| {
                    ExactTerminalVerdict::Reached { first_hit_tick }
                }),
                shaped_progress_millionths: None,
                native_risk_events: None,
            },
        }
    }

    #[test]
    fn generations_are_content_addressed_cumulative_and_exactly_bound() {
        let root = test_root();
        let campaign = root.join("build/campaigns/replay-test");
        let shard_path = campaign.join("episodes.dseps");
        install_authored_fixture(&shard_path);
        let shard = NativeEpisodeShard::read(&shard_path).unwrap();
        assert!(shard.episodes.len() >= 2);
        let optimization = optimization(&shard);
        let reference = artifact_reference(&root, &shard_path).unwrap();
        let first_evaluation = evaluation(&optimization, &reference, &shard, 0);

        let first = append_residual_replay_generation(
            &root,
            &campaign,
            &optimization,
            None,
            std::slice::from_ref(&first_evaluation),
        )
        .unwrap();
        let repeated = append_residual_replay_generation(
            &root,
            &campaign,
            &optimization,
            None,
            &[first_evaluation],
        )
        .unwrap();
        assert_eq!(first, repeated);
        let first_corpus = load_corpus(&root, &first.artifact).unwrap();
        assert_eq!(first_corpus.generation, 1);
        assert_eq!(first_corpus.report.entries, 1);
        assert_eq!(
            first_corpus.entries[0].role,
            ReplayExperienceRole::RandomizedCoverage
        );

        let second = append_residual_replay_generation(
            &root,
            &campaign,
            &optimization,
            Some(&first),
            &[evaluation(&optimization, &reference, &shard, 1)],
        )
        .unwrap();
        let second_corpus = load_corpus(&root, &second.artifact).unwrap();
        assert_eq!(second_corpus.generation, 2);
        assert_eq!(
            second_corpus.parent_corpus_sha256,
            Some(first_corpus.corpus_sha256)
        );
        assert_eq!(second_corpus.report.entries, 2);

        let mut detached = evaluation(&optimization, &reference, &shard, 1);
        detached.attempts[0].restore_identity = "0".repeat(32);
        let error = append_residual_replay_generation(
            &root,
            &campaign,
            &optimization,
            Some(&first),
            &[detached],
        )
        .unwrap_err();
        assert!(error.to_string().contains("exact native evaluation"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_or_objective_detached_episodes_fail_closed() {
        let root = test_root();
        let campaign = root.join("build/campaigns/replay-test");
        let shard_path = campaign.join("episodes.dseps");
        install_authored_fixture(&shard_path);
        let shard = NativeEpisodeShard::read(&shard_path).unwrap();
        let mut optimization = optimization(&shard);
        let reference = artifact_reference(&root, &shard_path).unwrap();
        let row = evaluation(&optimization, &reference, &shard, 0);
        assert!(
            append_residual_replay_generation(
                &root,
                &campaign,
                &optimization,
                None,
                &[row.clone(), row]
            )
            .unwrap_err()
            .to_string()
            .contains("repeats an authenticated episode")
        );

        let success = shard
            .episodes
            .iter()
            .position(|episode| episode.success)
            .unwrap();
        let wrong_role = NativeReplayCorpus::build(
            None,
            &[ReplayEpisodeSource {
                shard: &shard,
                episode_index: success,
                role: ReplayExperienceRole::Demonstration,
                policy_lineage_sha256: None,
                parent_entry_sha256: None,
            }],
        )
        .unwrap();
        assert!(
            validate_residual_corpus_scope(&optimization, &wrong_role)
                .unwrap_err()
                .to_string()
                .contains("outside its campaign scope")
        );

        optimization.terminal_predicate.definition_sha256 = Digest([11; 32]);
        let error = append_residual_replay_generation(
            &root,
            &campaign,
            &optimization,
            None,
            &[evaluation(&optimization, &reference, &shard, 0)],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match authored milestone")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
