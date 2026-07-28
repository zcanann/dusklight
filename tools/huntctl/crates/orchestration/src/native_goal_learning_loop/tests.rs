use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

fn test_root() -> PathBuf {
    let nonce = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "dusklight-native-goal-loop-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("campaign/artifacts")).unwrap();
    root.canonicalize().unwrap()
}

fn artifact(root: &Path, name: &str, payload: &[u8]) -> ArtifactReference {
    let relative = format!("campaign/artifacts/{name}");
    fs::write(root.join(&relative), payload).unwrap();
    ArtifactReference {
        path: relative,
        sha256: sha256(payload),
    }
}

fn request(root: &Path) -> NativeGoalLearningLoopRequest {
    let initial_corpus = artifact(root, "initial-corpus.json", b"initial corpus");
    let initial_shard = artifact(root, "initial.dseps", b"initial shard");
    let mut request = NativeGoalLearningLoopRequest {
        schema: NATIVE_GOAL_LEARNING_LOOP_REQUEST_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        campaign_class: CampaignClass::DemonstrationAssistedDiscovery,
        demonstration_mode: DemonstrationMode::BehaviorCloningWarmStart,
        optimization_request_sha256: Digest([1; 32]),
        native_execution_sha256: Digest([2; 32]),
        initial_replay_corpus: initial_corpus,
        initial_episode_shards: vec![initial_shard],
        generation_limit: 3,
        rollouts_per_generation: 2,
        simulated_tick_budget: 1_000,
        trajectory: NativeGoalTrajectoryConfig::default(),
        reachability: NativeGoalReachabilityConfig::default(),
        policy: NativeGoalFrozenPolicyConfig::default(),
        resume: NativeGoalLearningLoopResume {
            journal_path: "campaign/resume/events.jsonl".into(),
            state_path: "campaign/resume/state.json".into(),
            artifact_root: "campaign/artifacts".into(),
        },
    };
    request.content_sha256 = request.identity().unwrap();
    request.validate().unwrap();
    request
}

#[test]
fn discovery_horizon_reserves_a_material_timeout_margin() {
    assert_eq!(minimum_discovery_horizon_ticks(1).unwrap(), 17);
    assert_eq!(minimum_discovery_horizon_ticks(125).unwrap(), 141);
    assert_eq!(minimum_discovery_horizon_ticks(131).unwrap(), 147);
    assert_eq!(minimum_discovery_horizon_ticks(1_000).unwrap(), 1_100);
    assert!(minimum_discovery_horizon_ticks(u64::MAX).is_err());
}

fn reachability_metrics() -> NativeGoalReachabilityMetrics {
    NativeGoalReachabilityMetrics {
        rows: 4,
        episodes: 2,
        successful_rows: 2,
        failed_rows: 2,
        reachability_brier: 0.1,
        baseline_reachability_brier: 0.2,
        reachability_relative_improvement: 0.5,
        successful_time_mae_ticks: 1.0,
        baseline_successful_time_mae_ticks: 2.0,
        successful_time_relative_improvement: 0.5,
        discounted_return_rmse: 0.1,
        baseline_discounted_return_rmse: 0.2,
        return_relative_improvement: 0.5,
        discounted_tick_cost_mae: 0.1,
        baseline_discounted_tick_cost_mae: 0.2,
        tick_cost_relative_improvement: 0.5,
        mean_reachability_stddev: 0.05,
        mean_return_stddev: 0.04,
    }
}

fn policy_metrics() -> NativeGoalFrozenPolicyMetrics {
    NativeGoalFrozenPolicyMetrics {
        rows: 4,
        episodes: 2,
        continuous_mae: 0.1,
        baseline_continuous_mae: 0.2,
        button_bit_error_rate: 0.1,
        baseline_button_bit_error_rate: 0.2,
        joint_error: 0.2,
        baseline_joint_error: 0.4,
        joint_relative_improvement: 0.5,
        decoded_pad_exact_rate: 0.5,
        baseline_decoded_pad_exact_rate: 0.25,
    }
}

#[test]
fn checkpoint_report_seals_all_required_performance_axes() {
    let checkpoint = NativeGoalLearningCheckpointPerformance {
        generation: 1,
        input_corpus_sha256: Digest([1; 32]),
        output_corpus_sha256: Digest([2; 32]),
        output_entries: 10,
        output_transitions: 100,
        simulated_ticks: 320,
        rollouts: 4,
        terminal_successes: 1,
        terminal_success_millionths: 250_000,
        unique_success_ticks: 1,
        reachability_model_sha256: Digest([3; 32]),
        policy_manifest_sha256: Digest([4; 32]),
        critic_validation: reachability_metrics(),
        critic_test: reachability_metrics(),
        policy_validation: policy_metrics(),
        policy_test: policy_metrics(),
        unique_parent_states: 2,
        unique_consumed_actions: 8,
        unique_action_trajectories: 4,
        unique_state_identities: 80,
        contact_observations: 20,
        unique_contact_signatures: 3,
        collapse_detected: false,
    };
    let mut report = NativeGoalLearningCheckpointReport {
        schema: NATIVE_GOAL_LEARNING_CHECKPOINT_REPORT_SCHEMA_V1.into(),
        source_loop_state_sha256: Digest([5; 32]),
        checkpoints: vec![checkpoint],
        charged_simulated_ticks: 320,
        promotion_authority: false,
        report_sha256: Digest::ZERO,
    };
    report.report_sha256 = report.identity().unwrap();
    report.validate().unwrap();

    report.checkpoints[0].terminal_success_millionths = 1_000_000;
    report.report_sha256 = report.identity().unwrap();
    assert!(report.validate().is_err());
}

fn initialize_journal(root: &Path, request: &NativeGoalLearningLoopRequest) {
    let journal = root.join(&request.resume.journal_path);
    fs::create_dir_all(journal.parent().unwrap()).unwrap();
    fs::write(journal, []).unwrap();
}

fn append_test_event(
    root: &Path,
    request: &NativeGoalLearningLoopRequest,
    initial_corpus_sha256: Digest,
    event: NativeGoalLearningLoopEvent,
) -> NativeGoalLearningLoopState {
    append_test_event_with_schema(
        root,
        request,
        initial_corpus_sha256,
        NATIVE_GOAL_LEARNING_LOOP_RECORD_SCHEMA_V3,
        event,
    )
}

fn append_test_event_with_schema(
    root: &Path,
    request: &NativeGoalLearningLoopRequest,
    initial_corpus_sha256: Digest,
    schema: &str,
    event: NativeGoalLearningLoopEvent,
) -> NativeGoalLearningLoopState {
    let state = fold_journal(request, root, initial_corpus_sha256).unwrap();
    let mut record = NativeGoalLearningLoopRecord {
        schema: schema.into(),
        request_sha256: request.content_sha256,
        sequence: state.next_sequence,
        previous_record_sha256: state.last_record_sha256,
        event,
        record_sha256: Digest::ZERO,
    };
    record.record_sha256 = record_identity(&record).unwrap();
    let mut journal = OpenOptions::new()
        .append(true)
        .open(root.join(&request.resume.journal_path))
        .unwrap();
    journal.write_all(&record_bytes(&record).unwrap()).unwrap();
    journal.sync_all().unwrap();
    fold_journal(request, root, initial_corpus_sha256).unwrap()
}

fn generation_artifacts(
    root: &Path,
    generation: u16,
    phase: &str,
    count: usize,
) -> Vec<ArtifactReference> {
    (0..count)
        .map(|index| {
            let name = format!("generation-{generation}-{phase}-{index}.bin");
            artifact(root, &name, name.as_bytes())
        })
        .collect()
}

fn generation_policy_evidence(
    root: &Path,
    generation: u16,
) -> (Vec<ArtifactReference>, ArtifactReference, u16) {
    let mut shards = Vec::new();
    let mut references = Vec::new();
    for version in [26, 27] {
        let bytes = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../../../tests/fixtures/automation/native_episode_v{version}.dseps"
        )))
        .unwrap();
        let reference = artifact(
            root,
            &format!("generation-{generation}-shard-v{version}.dseps"),
            &bytes,
        );
        let shard = NativeEpisodeShard::decode(&bytes).unwrap();
        assert_eq!(shard.content_sha256, reference.sha256);
        references.push(reference);
        shards.push(shard);
    }
    let successes = shards
        .iter()
        .flat_map(|shard| &shard.episodes)
        .filter(|episode| episode.success)
        .count() as u16;
    assert!(successes <= 2);
    let report = NativePolicyCollapseReport::build(generation, &shards).unwrap();
    let diagnostics = artifact(
        root,
        &format!("generation-{generation}-collapse.json"),
        &pretty_json(&report).unwrap(),
    );
    (references, diagnostics, successes)
}

#[test]
fn three_generations_fold_with_exact_phase_and_parent_lineage() {
    let root = test_root();
    let request = request(&root);
    initialize_journal(&root, &request);
    let initial = Digest([3; 32]);
    let mut active = initial;
    let mut state = fold_journal(&request, &root, initial).unwrap();
    assert_eq!(state.next_sequence, 1);

    for generation in 1..=3 {
        let prepared = generation_artifacts(&root, generation, "prepared", 5);
        let batches = generation_artifacts(&root, generation, "batch", 2);
        state = append_test_event(
            &root,
            &request,
            initial,
            NativeGoalLearningLoopEvent::GenerationPrepared {
                generation,
                input_corpus_sha256: active,
                dataset_sha256: Digest([10 + generation as u8; 32]),
                reachability_model_sha256: Digest([20 + generation as u8; 32]),
                policy_manifest_sha256: Digest([30 + generation as u8; 32]),
                frozen_model_xxh3_128: format!("{generation:032x}"),
                dataset: prepared[0].clone(),
                reachability_model: prepared[1].clone(),
                policy_manifest: prepared[2].clone(),
                frozen_model: prepared[3].clone(),
                native_batches: batches,
            },
        );
        let prepared_record_sha256 = state.generations.last().unwrap().prepared_record_sha256;
        let results = generation_artifacts(&root, generation, "result", 2);
        let (shards, collapse_diagnostics, successes) =
            generation_policy_evidence(&root, generation);
        let reinference = generation_artifacts(&root, generation, "reinference", 2);
        let realized = generation_artifacts(&root, generation, "realized", 2);
        state = append_test_event(
            &root,
            &request,
            initial,
            NativeGoalLearningLoopEvent::GenerationExecuted {
                generation,
                prepared_record_sha256,
                native_results: results,
                episode_shards: shards,
                reinference_reports: reinference,
                realized_tapes: realized,
                simulated_ticks: 20,
                successes,
            },
        );
        let executed_record_sha256 = state
            .generations
            .last()
            .unwrap()
            .executed_record_sha256
            .unwrap();
        active = Digest([40 + generation as u8; 32]);
        let corpus = artifact(
            &root,
            &format!("generation-{generation}-corpus.json"),
            &[generation as u8],
        );
        state = append_test_event(
            &root,
            &request,
            initial,
            NativeGoalLearningLoopEvent::GenerationCommitted {
                generation,
                executed_record_sha256,
                output_corpus_sha256: active,
                output_corpus: corpus,
                collapse_diagnostics: Some(collapse_diagnostics),
                entries: u64::from(generation) + 4,
                transitions: u64::from(generation) * 20,
            },
        );
        assert_eq!(state.active_corpus_sha256, active);
    }

    state = append_test_event(
        &root,
        &request,
        initial,
        NativeGoalLearningLoopEvent::LoopStopped {
            next_generation: 4,
            reason: NativeGoalLearningStopReason::GenerationLimitReached,
            active_corpus_sha256: active,
            evidence: None,
            proposal_source: NativeGoalLearningProposalSource::FrozenGoalPolicy,
        },
    );
    assert_eq!(state.committed_generations, 3);
    assert_eq!(state.record_count, 10);
    assert_eq!(state.charged_simulated_ticks, 60);
    assert_eq!(state.active_corpus_sha256, active);
    assert_eq!(
        state.stopped.as_ref().unwrap().reason,
        NativeGoalLearningStopReason::GenerationLimitReached
    );
    assert!(
        state
            .generations
            .windows(2)
            .all(|pair| pair[0].output_corpus_sha256 == Some(pair[1].input_corpus_sha256))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resume_ignores_a_torn_tail_but_rejects_artifact_tampering() {
    let root = test_root();
    let request = request(&root);
    initialize_journal(&root, &request);
    let initial = Digest([3; 32]);
    let prepared = generation_artifacts(&root, 1, "prepared", 5);
    let batches = generation_artifacts(&root, 1, "batch", 2);
    let state = append_test_event(
        &root,
        &request,
        initial,
        NativeGoalLearningLoopEvent::GenerationPrepared {
            generation: 1,
            input_corpus_sha256: initial,
            dataset_sha256: Digest([11; 32]),
            reachability_model_sha256: Digest([21; 32]),
            policy_manifest_sha256: Digest([31; 32]),
            frozen_model_xxh3_128: "1".repeat(32),
            dataset: prepared[0].clone(),
            reachability_model: prepared[1].clone(),
            policy_manifest: prepared[2].clone(),
            frozen_model: prepared[3].clone(),
            native_batches: batches,
        },
    );
    let mut journal = OpenOptions::new()
        .append(true)
        .open(root.join(&request.resume.journal_path))
        .unwrap();
    journal.write_all(b"{\"torn\"").unwrap();
    journal.sync_all().unwrap();
    let resumed = fold_journal(&request, &root, initial).unwrap();
    assert_eq!(resumed.record_count, state.record_count);
    assert_eq!(resumed.last_record_sha256, state.last_record_sha256);

    fs::write(root.join(&prepared[0].path), b"tampered").unwrap();
    assert!(
        fold_journal(&request, &root, initial)
            .unwrap_err()
            .to_string()
            .contains("artifact digest differs")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn collapse_diagnostics_are_recomputed_from_journaled_native_shards() {
    let root = test_root();
    let (references, diagnostics, successes) = generation_policy_evidence(&root, 1);
    validate_collapse_diagnostics(
        &root,
        1,
        DemonstrationMode::BehaviorCloningWarmStart,
        &diagnostics,
        &references,
        successes,
    )
    .unwrap();

    let shards = references
        .iter()
        .map(|reference| NativeEpisodeShard::read(root.join(&reference.path)).unwrap())
        .collect::<Vec<_>>();
    let detached = NativePolicyCollapseReport::build(2, &shards).unwrap();
    let detached_reference = artifact(
        &root,
        "detached-collapse.json",
        &pretty_json(&detached).unwrap(),
    );
    assert!(
        validate_collapse_diagnostics(
            &root,
            1,
            DemonstrationMode::BehaviorCloningWarmStart,
            &detached_reference,
            &references,
            successes,
        )
        .unwrap_err()
        .to_string()
        .contains("differs from its demonstration treatment or realized native shards")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn v2_journal_resumes_with_a_diagnostic_v3_commit() {
    let root = test_root();
    let request = request(&root);
    initialize_journal(&root, &request);
    let initial = Digest([3; 32]);
    let prepared = generation_artifacts(&root, 1, "prepared", 5);
    let batches = generation_artifacts(&root, 1, "batch", 2);
    let mut state = append_test_event_with_schema(
        &root,
        &request,
        initial,
        NATIVE_GOAL_LEARNING_LOOP_RECORD_SCHEMA_V2,
        NativeGoalLearningLoopEvent::GenerationPrepared {
            generation: 1,
            input_corpus_sha256: initial,
            dataset_sha256: Digest([11; 32]),
            reachability_model_sha256: Digest([21; 32]),
            policy_manifest_sha256: Digest([31; 32]),
            frozen_model_xxh3_128: "1".repeat(32),
            dataset: prepared[0].clone(),
            reachability_model: prepared[1].clone(),
            policy_manifest: prepared[2].clone(),
            frozen_model: prepared[3].clone(),
            native_batches: batches,
        },
    );
    let (shards, collapse_diagnostics, successes) = generation_policy_evidence(&root, 1);
    state = append_test_event(
        &root,
        &request,
        initial,
        NativeGoalLearningLoopEvent::GenerationExecuted {
            generation: 1,
            prepared_record_sha256: state.generations[0].prepared_record_sha256,
            native_results: generation_artifacts(&root, 1, "result", 2),
            episode_shards: shards,
            reinference_reports: generation_artifacts(&root, 1, "reinference", 2),
            realized_tapes: generation_artifacts(&root, 1, "realized", 2),
            simulated_ticks: 20,
            successes,
        },
    );
    state = append_test_event(
        &root,
        &request,
        initial,
        NativeGoalLearningLoopEvent::GenerationCommitted {
            generation: 1,
            executed_record_sha256: state.generations[0].executed_record_sha256.unwrap(),
            output_corpus_sha256: Digest([41; 32]),
            output_corpus: artifact(&root, "generation-1-corpus.json", b"corpus"),
            collapse_diagnostics: Some(collapse_diagnostics),
            entries: 5,
            transitions: 20,
        },
    );
    assert_eq!(state.committed_generations, 1);
    assert!(state.generations[0].collapse_diagnostics.is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reducer_rejects_skips_partial_execution_and_premature_stop() {
    let root = test_root();
    let request = request(&root);
    initialize_journal(&root, &request);
    let initial = Digest([3; 32]);
    let mut state = fold_journal(&request, &root, initial).unwrap();
    let prepared = generation_artifacts(&root, 2, "prepared", 5);
    let batches = generation_artifacts(&root, 2, "batch", 2);
    let skipped = NativeGoalLearningLoopEvent::GenerationPrepared {
        generation: 2,
        input_corpus_sha256: initial,
        dataset_sha256: Digest([12; 32]),
        reachability_model_sha256: Digest([22; 32]),
        policy_manifest_sha256: Digest([32; 32]),
        frozen_model_xxh3_128: "2".repeat(32),
        dataset: prepared[0].clone(),
        reachability_model: prepared[1].clone(),
        policy_manifest: prepared[2].clone(),
        frozen_model: prepared[3].clone(),
        native_batches: batches,
    };
    assert!(
        apply_event(
            &request,
            &root,
            &mut state,
            NATIVE_GOAL_LEARNING_LOOP_RECORD_SCHEMA_V3,
            &skipped,
            Digest([9; 32])
        )
        .is_err()
    );
    let premature = NativeGoalLearningLoopEvent::LoopStopped {
        next_generation: 1,
        reason: NativeGoalLearningStopReason::GenerationLimitReached,
        active_corpus_sha256: initial,
        evidence: None,
        proposal_source: NativeGoalLearningProposalSource::FrozenGoalPolicy,
    };
    assert!(
        apply_event(
            &request,
            &root,
            &mut state,
            NATIVE_GOAL_LEARNING_LOOP_RECORD_SCHEMA_V3,
            &premature,
            Digest([9; 32])
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn request_class_and_trajectory_are_bound_to_each_demonstration_mode() {
    let root = test_root();
    for mode in [
        DemonstrationMode::Absent,
        DemonstrationMode::ReplayOnly,
        DemonstrationMode::BehaviorCloningWarmStart,
        DemonstrationMode::ReverseCurriculumCheckpoints,
    ] {
        let mut candidate = request(&root);
        candidate.demonstration_mode = mode;
        candidate.trajectory.demonstration_mode = mode;
        candidate.campaign_class = if mode == DemonstrationMode::Absent {
            CampaignClass::FromScratchDiscovery
        } else {
            CampaignClass::DemonstrationAssistedDiscovery
        };
        candidate.content_sha256 = candidate.identity().unwrap();
        candidate.validate().unwrap();

        candidate.trajectory.demonstration_mode = DemonstrationMode::ReplayOnly;
        if mode != DemonstrationMode::ReplayOnly {
            candidate.content_sha256 = candidate.identity().unwrap();
            assert!(candidate.validate().is_err());
        }
    }
    fs::remove_dir_all(root).unwrap();
}
