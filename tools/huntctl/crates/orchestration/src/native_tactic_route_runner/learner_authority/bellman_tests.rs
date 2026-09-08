use super::*;
use crate::tactic_q_campaign::parameterized_learning_tests::{
    base_facts, collect_sibling_feedback,
};
use dusklight_learning::tactic_features::GoalConditionedTacticFeatureEncoder;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn bellman_authority_resumes_exact_values_history_and_subsequent_update() {
    let encoder = GoalConditionedTacticFeatureEncoder::new([1.0, 0.0, 0.0]).unwrap();
    let corpus = collect_sibling_feedback(&base_facts(), &encoder);
    let root = std::env::temp_dir().join(format!(
        "dusklight-bellman-resume-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let journal = root.join("replay.dtrp");
    let objects = root.join("objects");
    let identity = TacticReplayControlPlaneIdentity::new(
        corpus.execution_authority_sha256,
        corpus.feature_schema_sha256,
        corpus.objective_sha256,
        corpus.root_checkpoint_sha256,
    )
    .unwrap();
    let mut config = OptionValueConfig::default();
    config.fitted_q.iterations = 2;
    let treatment = TacticValueTreatment::HindsightBellmanForestV3;
    let replay = TacticReplayControlPlane::create(&journal, &objects, identity.clone()).unwrap();
    let mut authority = CampaignTacticLearnerAuthority::new(
        replay,
        config.clone(),
        encoder.goal_distance_feature(),
        treatment,
        1,
        None,
    )
    .unwrap();
    let publish_pair = |authority: &mut CampaignTacticLearnerAuthority, offset: usize| {
        let snapshot = authority.snapshot();
        for index in offset..offset + 2 {
            authority
                .publish(
                    0,
                    offset as u64 / 2,
                    snapshot.sha256,
                    &corpus.transitions[index],
                    &corpus.routes[index],
                    corpus.episode_groups[index],
                )
                .unwrap();
        }
    };
    publish_pair(&mut authority, 0);
    let first = authority.fit_current().unwrap();
    publish_pair(&mut authority, 2);
    let second = authority.fit_current().unwrap();
    assert_eq!(
        first
            .manifest
            .bellman_state
            .as_ref()
            .unwrap()
            .completed_backups,
        2
    );
    assert_eq!(
        second
            .manifest
            .bellman_state
            .as_ref()
            .unwrap()
            .completed_backups,
        4
    );
    assert_eq!(
        second
            .manifest
            .bellman_state
            .as_ref()
            .unwrap()
            .previous_snapshot_sha256,
        first.sha256
    );
    let second_bytes = second.bellman_state_bytes().unwrap();
    drop(authority);

    let replay = TacticReplayControlPlane::open(&journal, &objects, &identity).unwrap();
    let mut resumed = CampaignTacticLearnerAuthority::new(
        replay,
        config.clone(),
        encoder.goal_distance_feature(),
        treatment,
        1,
        None,
    )
    .unwrap();
    assert_eq!(resumed.snapshot().sha256, second.sha256);
    assert_eq!(
        resumed.snapshot().bellman_state_bytes().unwrap(),
        second_bytes
    );
    assert_eq!(
        resumed.invocation_metrics().updates,
        0,
        "restart must not perform another Bellman update"
    );
    let historical = resumed
        .snapshot_by_identity(first.sha256, first.replay_revision)
        .unwrap();
    assert_eq!(
        historical.bellman_state_bytes().unwrap(),
        first.bellman_state_bytes().unwrap()
    );
    assert_eq!(
        resumed.snapshot().sha256,
        second.sha256,
        "historical lookup must not move the head"
    );
    publish_pair(&mut resumed, 4);
    let replay = resumed.replay.snapshot().unwrap();
    let training_hash = replay.training_replay_sha256();
    let expected = TacticQImmutableLearnerSnapshot::update_verified_bellman(
        replay.corpus,
        replay.version.revision,
        training_hash,
        &second,
    )
    .unwrap();
    let third = resumed.fit_current().unwrap();
    assert_eq!(third.sha256, expected.sha256);
    assert_eq!(
        third.bellman_state_bytes().unwrap(),
        expected.bellman_state_bytes().unwrap()
    );
    assert_eq!(
        third
            .manifest
            .bellman_state
            .as_ref()
            .unwrap()
            .completed_backups,
        6
    );
    assert!(resumed.replay.bellman_state(Digest([0xff; 32])).is_err());
    drop(resumed);
    let store = dusklight_evidence::content_store::ContentStore::open(&objects).unwrap();
    let blob = store.blob_path(third.manifest.bellman_state.as_ref().unwrap().state_sha256);
    let held = blob.with_extension("held");
    fs::rename(&blob, &held).unwrap();
    let reopen = || {
        let replay = TacticReplayControlPlane::open(&journal, &objects, &identity).unwrap();
        CampaignTacticLearnerAuthority::new(
            replay,
            config.clone(),
            encoder.goal_distance_feature(),
            treatment,
            1,
            None,
        )
    };
    assert!(
        reopen().is_err(),
        "missing state must not silently cold-refit"
    );
    fs::write(&blob, b"corrupted training state").unwrap();
    assert!(
        reopen().is_err(),
        "corrupt state must not silently cold-refit"
    );
    fs::remove_file(&blob).unwrap();
    fs::rename(&held, &blob).unwrap();
    assert_eq!(reopen().unwrap().snapshot().sha256, third.sha256);
    fs::remove_dir_all(root).unwrap();
}
