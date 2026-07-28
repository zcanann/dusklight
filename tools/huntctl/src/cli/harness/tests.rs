use super::{
    learning_value_cell_draft_from_directory, pin_curriculum_source_checkpoint, repository_artifact,
};
use huntctl::search_evaluator::learning_value_comparison::LearningValueTreatmentKind;
use huntctl::search_evaluator::learning_value_evidence::LearningValuePhaseSource;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn curriculum_checkpoint_pin_survives_source_pruning_and_reuses_exact_bytes() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = repository_root.join("build").join(format!(
        "curriculum-pin-test-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let source_path = root.join("live-checkpoint.json");
    let output = root.join("child.request.json");
    let bytes = br#"{"checkpoint":"durable"}"#;
    fs::write(&source_path, bytes).unwrap();
    let source = repository_artifact(
        &repository_root,
        source_path.strip_prefix(&repository_root).unwrap(),
        "test checkpoint",
    )
    .unwrap();

    let pinned =
        pin_curriculum_source_checkpoint(&repository_root, &output, &source, bytes).unwrap();
    fs::remove_file(source_path).unwrap();
    assert_eq!(fs::read(repository_root.join(&pinned.path)).unwrap(), bytes);
    assert_eq!(pinned.sha256, source.sha256);
    assert_eq!(
        pin_curriculum_source_checkpoint(&repository_root, &output, &source, bytes,).unwrap(),
        pinned
    );

    fs::write(repository_root.join(&pinned.path), b"tampered").unwrap();
    assert!(pin_curriculum_source_checkpoint(&repository_root, &output, &source, bytes,).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cell_directory_draft_maps_learning_and_residual_artifacts() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let relative = PathBuf::from("build").join(format!(
        "learning-value-cell-draft-test-{}-{nonce}",
        std::process::id()
    ));
    let root = repository_root.join(&relative);
    for path in [
        "request.json",
        "execution/execution.json",
        "checkpoints/checkpoint-00000001.json",
        "checkpoints/checkpoint-00000002.json",
        "learning-loop/request.json",
        "learning-loop/state.json",
        "learning-loop/checkpoint-report.json",
        "refinement/request.json",
        "refinement/execution/execution.json",
        "refinement/checkpoints/checkpoint-00000001.json",
        "refinement/checkpoints/checkpoint-00000002.json",
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"artifact").unwrap();
    }

    let learned = learning_value_cell_draft_from_directory(
        &repository_root,
        &relative,
        "checkpoint".into(),
        17,
        LearningValueTreatmentKind::LearnedThenResidualRefinement,
    )
    .unwrap();
    assert_eq!(learned.phases.len(), 2);
    assert!(matches!(
        &learned.phases[0],
        LearningValuePhaseSource::StateReactive { loop_state, .. }
            if loop_state.path.ends_with("/learning-loop/state.json")
    ));
    assert!(matches!(
        &learned.phases[1],
        LearningValuePhaseSource::Residual { final_checkpoint, .. }
            if final_checkpoint.path.ends_with("/refinement/checkpoints/checkpoint-00000002.json")
    ));

    let residual = learning_value_cell_draft_from_directory(
        &repository_root,
        &relative,
        "checkpoint".into(),
        17,
        LearningValueTreatmentKind::CemResidual,
    )
    .unwrap();
    assert!(matches!(
        &residual.phases[0],
        LearningValuePhaseSource::Residual { final_checkpoint, .. }
            if final_checkpoint.path.ends_with("/checkpoints/checkpoint-00000002.json")
                && !final_checkpoint.path.contains("/refinement/")
    ));
    fs::remove_dir_all(root).unwrap();
}
