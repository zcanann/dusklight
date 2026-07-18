use huntctl::pool::{MixedBuildPolicy, StartupFailureKind, WorkerLaunch, WorkerPool};
use std::path::PathBuf;
use std::process::Command;

fn launch(label: &str, revision: &str) -> WorkerLaunch {
    WorkerLaunch {
        label: label.into(),
        program: PathBuf::from(env!("CARGO_BIN_EXE_huntctl")),
        args: vec![
            "mock-worker".into(),
            "--mock-revision".into(),
            revision.into(),
        ],
    }
}

#[test]
fn schedules_health_jobs_across_persistent_workers() {
    let start = WorkerPool::spawn(
        vec![
            launch("a", "same"),
            launch("b", "same"),
            launch("c", "same"),
        ],
        MixedBuildPolicy::RequireIdentical,
    );
    assert!(start.failures.is_empty());
    let mut pool = start.pool;
    assert_eq!(pool.worker_count(), 3);
    let health = pool.health_jobs(12);
    assert!(health.all_ok());
    assert_eq!(health.jobs.len(), 12);
    for (job_id, result) in health.jobs.iter().enumerate() {
        assert_eq!(result.worker_index, job_id % 3);
    }
    assert!(pool.shutdown().iter().all(|result| result.error.is_none()));
}

#[test]
fn mixed_build_policy_is_explicit() {
    let strict = WorkerPool::spawn(
        vec![launch("a", "one"), launch("b", "two")],
        MixedBuildPolicy::RequireIdentical,
    );
    assert_eq!(strict.pool.worker_count(), 1);
    assert_eq!(strict.failures.len(), 1);
    assert_eq!(
        strict.failures[0].kind,
        StartupFailureKind::IncompatibleBuild
    );
    assert!(strict.failures[0].message.contains("build.revision"));
    assert!(strict.failures[0].message.contains("build.describe"));

    let mut mixed = WorkerPool::spawn(
        vec![launch("a", "one"), launch("b", "two")],
        MixedBuildPolicy::AllowMixed,
    );
    assert!(mixed.failures.is_empty());
    assert_eq!(mixed.pool.worker_count(), 2);
    mixed.pool.shutdown();
}

#[test]
fn startup_failure_does_not_discard_healthy_workers() {
    let missing = WorkerLaunch {
        label: "missing".into(),
        program: PathBuf::from("definitely-not-a-real-hunt-worker-executable"),
        args: Vec::new(),
    };
    let mut start = WorkerPool::spawn(
        vec![missing, launch("healthy", "same")],
        MixedBuildPolicy::RequireIdentical,
    );
    assert_eq!(start.failures.len(), 1);
    assert_eq!(start.failures[0].kind, StartupFailureKind::Spawn);
    assert_eq!(start.pool.worker_count(), 1);
    assert!(start.pool.health_jobs(2).all_ok());
    start.pool.shutdown();
}

#[test]
fn pool_health_cli_reports_parallel_jobs() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let output = Command::new(executable)
        .args([
            "pool",
            "health",
            "--worker",
            executable,
            "--worker-arg",
            "mock-worker",
            "--workers",
            "3",
            "--checks",
            "9",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["active_workers"], 3);
    assert_eq!(value["health_jobs"].as_array().unwrap().len(), 9);
    assert!(
        value["health_jobs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|job| job["ok"] == true)
    );
}
