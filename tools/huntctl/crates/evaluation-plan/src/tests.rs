use super::*;

fn schedule() -> EvaluationWorkerSchedule {
    EvaluationWorkerSchedule::build(
        2,
        [
            ("candidate-a".into(), 1),
            ("candidate-a".into(), 2),
            ("candidate-b".into(), 1),
        ],
    )
    .unwrap()
}

#[test]
fn stable_lanes_cover_every_trial_once() {
    for trial_count in 1..=31 {
        for requested_workers in 1..=12 {
            let worker_lanes = requested_workers.min(trial_count);
            let schedule = EvaluationWorkerSchedule::build(
                worker_lanes,
                (0..trial_count).map(|index| (format!("candidate-{index}"), 1)),
            )
            .unwrap();
            let mut assigned = Vec::new();
            for worker_lane in 0..worker_lanes {
                let lane = schedule
                    .assignments_for_lane(worker_lane)
                    .unwrap()
                    .map(|assignment| assignment.trial_index)
                    .collect::<Vec<_>>();
                assert!(
                    lane.iter()
                        .all(|index| index % worker_lanes == worker_lane)
                );
                assigned.extend(lane);
            }
            assigned.sort_unstable();
            assert_eq!(assigned, (0..trial_count).collect::<Vec<_>>());
        }
    }
}

#[test]
fn definition_rejects_lane_and_identity_tampering() {
    let schedule = schedule();
    schedule.validate().unwrap();

    let mut wrong_lane = schedule.clone();
    wrong_lane.assignments[2].worker_id = "evaluation/worker-1".into();
    assert!(wrong_lane.validate().is_err());

    let mut duplicate_identity = schedule;
    duplicate_identity.assignments[2].candidate_id = "candidate-a".into();
    duplicate_identity.assignments[2].attempt = 2;
    assert!(duplicate_identity.validate().is_err());
}

#[test]
fn completed_claims_must_be_planned_exactly_once() {
    let schedule = schedule();
    schedule
        .validate_completed_claims([("candidate-b", 1, "evaluation/worker-0")])
        .unwrap();
    assert!(
        schedule
            .validate_completed_claims([("candidate-a", 1, "evaluation/worker-1")])
            .is_err()
    );
    assert!(
        schedule
            .validate_completed_claims([("candidate-c", 1, "evaluation/worker-0")])
            .is_err()
    );
    assert!(
        schedule
            .validate_completed_claims([
                ("candidate-a", 1, "evaluation/worker-0"),
                ("candidate-a", 1, "evaluation/worker-0"),
            ])
            .is_err()
    );
}

#[test]
fn digest_authenticates_the_exact_pretty_json_bytes() {
    let schedule = schedule();
    let bytes = serde_json::to_vec_pretty(&schedule).unwrap();
    assert_eq!(
        schedule.sha256().unwrap(),
        ArtifactDigest(Sha256::digest(bytes).into())
    );
}
