use super::{SearchExecutionConfig, bind_route_origin_card_fixture};
use huntctl::tape::TapeBoot;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn temporary_root(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "huntctl-route-card-fixture-{name}-{}-{nonce}",
        std::process::id()
    ))
}

fn execution(root: &Path) -> SearchExecutionConfig {
    SearchExecutionConfig {
        game: root.join("dusklight"),
        dvd: root.join("game.iso"),
        working_directory: root.to_path_buf(),
        game_args_prefix: Vec::new(),
        timeout: Duration::from_secs(1),
        harness: None,
    }
}

#[test]
fn process_boot_route_binds_its_declared_card_fixture() {
    let root = temporary_root("process");
    let fixture = root.join("orig/process-boot");
    fs::create_dir_all(&fixture).unwrap();
    let timeline = huntctl::timeline::Timeline::parse(
            "timeline test\norigin boot predicate process_boot source process_boot.milestones card_fixture orig/process-boot\n",
        )
        .unwrap();
    let mut execution = execution(&root);

    bind_route_origin_card_fixture(&timeline, &TapeBoot::Process, &mut execution).unwrap();

    assert_eq!(
        execution.game_args_prefix,
        [
            "--automation-card-fixture".to_owned(),
            fs::canonicalize(&fixture)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned(),
        ]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stage_boot_route_does_not_apply_the_process_card_fixture() {
    let root = temporary_root("stage");
    fs::create_dir_all(root.join("orig/process-boot")).unwrap();
    let timeline = huntctl::timeline::Timeline::parse(
            "timeline test\norigin boot predicate process_boot source process_boot.milestones card_fixture orig/process-boot\n",
        )
        .unwrap();
    let mut execution = execution(&root);
    let boot = TapeBoot::Stage {
        stage: "F_SP103".into(),
        room: 0,
        point: 0,
        layer: -1,
        save_slot: None,
        fixture: None,
    };

    bind_route_origin_card_fixture(&timeline, &boot, &mut execution).unwrap();

    assert!(execution.game_args_prefix.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn route_binding_rejects_uncontrolled_game_arguments() {
    let root = temporary_root("arguments");
    fs::create_dir_all(&root).unwrap();
    let timeline = huntctl::timeline::Timeline::parse(
        "timeline test\norigin boot predicate process_boot source process_boot.milestones\n",
    )
    .unwrap();
    let mut execution = execution(&root);
    execution.game_args_prefix = vec!["--stage".into(), "F_SP103,0,0,-1".into()];

    let error =
        bind_route_origin_card_fixture(&timeline, &TapeBoot::Process, &mut execution).unwrap_err();

    assert!(error.to_string().contains("does not accept --game-arg"));
    fs::remove_dir_all(root).unwrap();
}
