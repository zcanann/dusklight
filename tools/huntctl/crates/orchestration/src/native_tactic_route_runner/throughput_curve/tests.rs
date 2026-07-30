use super::*;

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "dusklight-throughput-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn sample(
    ordinal: u32,
    repetition: u32,
    workers: usize,
    wall_micros: u64,
) -> NativeTacticThroughputCurveSample {
    NativeTacticThroughputCurveSample {
        ordinal,
        repetition,
        workers,
        route_report_path: format!("sample-{ordinal}/report.json"),
        route_report_sha256: Digest([ordinal as u8; 32]),
        state_graph_sha256s: vec![Digest([9; 32])],
        useful_graph_expansion_set_sha256s: vec![Digest([8; 32])],
        completed_decisions: 16,
        unique_useful_graph_expansions: 256,
        wall_micros,
        process_launch_micros: 0,
        unique_useful_graph_expansions_per_second_millionths: per_second_millionths(
            256,
            wall_micros,
        ),
        peak_worker_checkpoint_resident_bytes: 100,
        checkpoint_pool_resident_bytes_upper_bound: 100 * workers as u64,
        maximum_model_replay_lag_revisions: 2,
        learner_updates: 4,
        model_snapshots_published: 4,
        training_replay_rows: 256,
        restore_samples: 272,
        non_root_restore_requests: 128,
        direct_restore_fallback_replays: 8,
        cache_evictions: 12,
        tactic_selection_micros: 100,
        checkpoint_branching_micros: 100,
        tactic_execution_micros: wall_micros / 2,
        native_simulation_micros: wall_micros.saturating_mul(workers as u64) / 2,
        ipc_and_result_transport_micros: 100,
        tactic_preparation_and_fact_extraction_micros: 100,
        model_update_micros: 100,
        evidence_projection_micros: 100,
        persistence_micros: 100,
        orchestration_micros: 100,
        result_validation_and_fact_extraction_micros: 100,
        graph_admission_micros: 100,
        native_worker_occupancy_per_million: 500_000,
    }
}

#[test]
fn throughput_output_mode_requires_explicit_resume() {
    assert!(curve_output_mode_is_valid(false, false));
    assert!(curve_output_mode_is_valid(true, true));
    assert!(!curve_output_mode_is_valid(true, false));
    assert!(!curve_output_mode_is_valid(false, true));
}

#[test]
fn completed_prefix_and_durable_partial_sample_are_resumable() {
    let test = TestDirectory::new("durable-partial");
    let output_root = test.path.join("curve");
    let progress_root = output_root.join(PROGRESS_DIRECTORY);
    fs::create_dir_all(&progress_root).unwrap();
    let schedule = throughput_sample_schedule(2).unwrap();
    fs::create_dir(sample_root(&output_root, 1, 1, 1)).unwrap();
    fs::create_dir(sample_root(&output_root, 2, 1, 2)).unwrap();

    reject_detached_sample_roots(&output_root, &schedule).unwrap();
    reject_non_prefix_progress(&output_root, &progress_root, &schedule, 1).unwrap();
    assert_eq!(
        curve_fleet_launch_start(1, schedule.len()).unwrap(),
        Some(1)
    );
}

#[test]
fn detached_and_non_prefix_sample_roots_are_rejected() {
    let detached = TestDirectory::new("detached");
    let detached_output = detached.path.join("curve");
    fs::create_dir(&detached_output).unwrap();
    let schedule = throughput_sample_schedule(2).unwrap();
    fs::create_dir(detached_output.join("sample-99-r1-w1")).unwrap();
    assert!(reject_detached_sample_roots(&detached_output, &schedule).is_err());

    let non_prefix = TestDirectory::new("non-prefix");
    let output_root = non_prefix.path.join("curve");
    let progress_root = output_root.join(PROGRESS_DIRECTORY);
    fs::create_dir_all(&progress_root).unwrap();
    fs::create_dir(sample_root(&output_root, 3, 1, 4)).unwrap();
    assert!(reject_non_prefix_progress(&output_root, &progress_root, &schedule, 1).is_err());
}

#[test]
fn torn_or_non_file_progress_records_are_rejected() {
    let test = TestDirectory::new("torn-progress");
    let empty = test.path.join("empty.dtcr");
    fs::write(&empty, b"").unwrap();
    assert!(read_progress_record::<NativeTacticThroughputRunCommit>(&empty).is_err());

    let directory = test.path.join("directory.dtcr");
    fs::create_dir(&directory).unwrap();
    assert!(read_progress_record::<NativeTacticThroughputRunCommit>(&directory).is_err());
}

#[test]
fn aggregate_only_resealing_requires_zero_fleet_launches() {
    let sample_count = throughput_sample_schedule(2).unwrap().len();
    assert_eq!(
        curve_fleet_launch_start(sample_count, sample_count).unwrap(),
        None
    );
    assert!(curve_fleet_launch_start(sample_count + 1, sample_count).is_err());
}

#[test]
fn throughput_schedule_balances_worker_order_between_repetitions() {
    assert_eq!(
        throughput_sample_schedule(2).unwrap(),
        vec![
            (1, 1, 1),
            (2, 1, 2),
            (3, 1, 4),
            (4, 1, 8),
            (5, 1, 16),
            (6, 2, 16),
            (7, 2, 8),
            (8, 2, 4),
            (9, 2, 2),
            (10, 2, 1),
        ]
    );
}

#[test]
fn sample_commit_binds_useful_expansion_evidence() {
    let plan = Digest([3; 32]);
    let route_path = Path::new("sample-01-r1-w1/report.json");
    let mut commit =
        NativeTacticThroughputSampleCommit::new(plan, sample(1, 1, 1, 1_000_000)).unwrap();
    commit.sample.route_report_path = path_text(route_path);
    commit.content_sha256 = commit.compute_content_sha256().unwrap();
    commit.validate(plan, 1, 1, 1, route_path).unwrap();

    commit.sample.useful_graph_expansion_set_sha256s[0] = Digest([7; 32]);
    assert!(commit.validate(plan, 1, 1, 1, route_path).is_err());
}

#[test]
fn fleet_launch_commit_rejects_detached_timing() {
    let plan = Digest([4; 32]);
    let mut commit = NativeTacticThroughputFleetLaunchCommit::new(plan, 16, 5_000).unwrap();
    commit.validate(plan, 16).unwrap();

    commit.launch_micros += 1;
    assert!(commit.validate(plan, 16).is_err());
}

#[test]
fn fixed_graph_work_builds_a_strict_scaling_curve() {
    let order = [1, 2, 4, 8, 16, 16, 8, 4, 2, 1];
    let samples = order
        .into_iter()
        .enumerate()
        .map(|(index, workers)| {
            let repetition = if index < 5 { 1 } else { 2 };
            sample(
                index as u32 + 1,
                repetition,
                workers,
                16_000_000 / workers as u64,
            )
        })
        .collect::<Vec<_>>();
    let derived = DerivedCurve::from_samples(&samples, 16, 256, 2_000, 2).unwrap();
    assert!(derived.fixed_work_satisfied);
    assert!(derived.identical_useful_expansion_evidence_satisfied);
    assert!(derived.memory_bound_satisfied);
    assert!(derived.learner_staleness_bound_satisfied);
    assert!(derived.long_work_exercised);
    assert!(derived.strictly_increasing_throughput);
    assert!(derived.passed());
    assert_eq!(
        derived.curve[4].speedup_over_one_worker_millionths,
        16_000_000
    );
    assert_eq!(derived.curve[4].parallel_efficiency_millionths, 1_000_000);
}

#[test]
fn raw_tick_speed_cannot_hide_non_scaling_useful_work() {
    let order = [1, 2, 4, 8, 16, 16, 8, 4, 2, 1];
    let mut samples = order
        .into_iter()
        .enumerate()
        .map(|(index, workers)| {
            let repetition = if index < 5 { 1 } else { 2 };
            sample(index as u32 + 1, repetition, workers, 1_000_000)
        })
        .collect::<Vec<_>>();
    samples[4].unique_useful_graph_expansions = 255;
    samples[4].unique_useful_graph_expansions_per_second_millionths =
        per_second_millionths(255, samples[4].wall_micros);
    let derived = DerivedCurve::from_samples(&samples, 16, 256, 2_000, 2).unwrap();
    assert!(!derived.fixed_work_satisfied);
    assert!(!derived.strictly_increasing_throughput);
    assert!(!derived.passed());
}

#[test]
fn warm_microbenchmark_cannot_claim_long_campaign_scaling() {
    let order = [1, 2, 4, 8, 16, 16, 8, 4, 2, 1];
    let mut samples = order
        .into_iter()
        .enumerate()
        .map(|(index, workers)| {
            sample(
                index as u32 + 1,
                if index < 5 { 1 } else { 2 },
                workers,
                16_000_000 / workers as u64,
            )
        })
        .collect::<Vec<_>>();
    samples[3].cache_evictions = 0;
    let derived = DerivedCurve::from_samples(&samples, 16, 256, 2_000, 2).unwrap();
    assert!(!derived.long_work_exercised);
    assert!(!derived.passed());
}

#[test]
fn persistent_curve_rejects_per_sample_process_relaunch() {
    let order = [1, 2, 4, 8, 16, 16, 8, 4, 2, 1];
    let samples = order
        .into_iter()
        .enumerate()
        .map(|(index, workers)| {
            sample(
                index as u32 + 1,
                if index < 5 { 1 } else { 2 },
                workers,
                16_000_000 / workers as u64,
            )
        })
        .collect::<Vec<_>>();
    let derived = DerivedCurve::from_samples(&samples, 16, 256, 2_000, 2).unwrap();
    let passed = derived.passed();
    let mut report = NativeTacticThroughputCurveReport {
        schema: NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V4.into(),
        content_sha256: Digest::ZERO,
        recorded_unix_millis: 1,
        operating_system: "test".into(),
        architecture: "test".into(),
        logical_cpu_count: 16,
        optimization_request_sha256: Digest([1; 32]),
        execution_binding_sha256: Digest([2; 32]),
        execution_plan_sha256: Digest([3; 32]),
        fleet_workers: 16,
        fleet_launch_micros: 10,
        repetitions: 2,
        worker_counts: NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.to_vec(),
        fixed_completed_decisions: 16,
        fixed_unique_useful_graph_expansions: 256,
        checkpoint_pool_memory_bound_bytes: 2_000,
        maximum_allowed_stale_revisions: 2,
        execution_order: samples,
        curve: derived.curve,
        fixed_work_satisfied: derived.fixed_work_satisfied,
        identical_useful_expansion_evidence_satisfied: derived
            .identical_useful_expansion_evidence_satisfied,
        memory_bound_satisfied: derived.memory_bound_satisfied,
        learner_staleness_bound_satisfied: derived.learner_staleness_bound_satisfied,
        long_work_exercised: derived.long_work_exercised,
        strictly_increasing_throughput: derived.strictly_increasing_throughput,
        passed,
    };
    report.refresh_content_sha256().unwrap();
    report.validate().unwrap();

    report.execution_order[0].process_launch_micros = 1;
    report.refresh_content_sha256().unwrap();
    assert!(report.validate().is_err());
}
