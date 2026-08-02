use super::*;
use crate::optimization_request::OptimizationIncumbentAuthority;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestArtifacts(PathBuf);

impl TestArtifacts {
    fn new(repository: &Path) -> Self {
        let path = repository.join("build").join(format!(
            "native-residual-binding-test-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        if path.exists() {
            fs::remove_dir_all(&path).unwrap();
        }
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestArtifacts {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap()
}

fn fixture() -> (
    PathBuf,
    TestArtifacts,
    OptimizationRequest,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let repository = repository();
    let artifacts = TestArtifacts::new(&repository);
    let request_path = repository.join(
        "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
    );
    let mut optimization: OptimizationRequest =
        serde_json::from_slice(&fs::read(request_path).unwrap()).unwrap();
    if resolve_card_fixture_manifest(&repository, &optimization).is_err() {
        optimization.route.native_source_boundary_fingerprint =
            optimization.route.source_boundary_fingerprint.clone();
        optimization.refresh_content_sha256().unwrap();
        resolve_card_fixture_manifest(&repository, &optimization).unwrap();
    }
    let executable = artifacts.0.join("Dusklight");
    let game_data = artifacts.0.join("game.iso");
    let tape_path = artifacts.0.join("full.tape");
    let program_path = artifacts.0.join("goal.dmsp");
    let world_context_path = artifacts.0.join("world.context.json");
    fs::write(&executable, b"executable").unwrap();
    fs::write(artifacts.0.join("renderer.dll"), b"renderer").unwrap();
    fs::write(artifacts.0.join("support.DLL"), b"support").unwrap();
    fs::write(artifacts.0.join("not-a-library.txt"), b"ignored").unwrap();
    fs::write(&game_data, b"game-data").unwrap();
    fs::write(
        &world_context_path,
        serde_json::to_vec(&serde_json::json!({
            "schema": "dusklight-world-context/v1",
            "game_data_sha256": sha256_file(&game_data).unwrap(),
            "stages": []
        }))
        .unwrap(),
    )
    .unwrap();
    let tape = materialize_native_residual_process_tape(&repository, &optimization).unwrap();
    fs::write(&tape_path, tape.encode().unwrap()).unwrap();
    let source =
        fs::read_to_string(repository.join(&optimization.terminal_predicate.source.path)).unwrap();
    let program = dusklight_objectives::milestone_dsl::parse(&source).unwrap();
    let compiled = dusklight_objectives::milestone_dsl::compile(&program).unwrap();
    fs::write(&program_path, compiled.bytes).unwrap();
    (
        repository,
        artifacts,
        optimization,
        executable,
        game_data,
        tape_path,
        program_path,
        world_context_path,
    )
}

#[test]
fn execution_binding_seals_the_exact_native_checkpoint_authority() {
    let (root, _artifacts, optimization, executable, game_data, tape, program, world_context) =
        fixture();
    let card_fixture = resolve_card_fixture_manifest(&root, &optimization).unwrap();
    let binding = NativeResidualExecutionBinding::seal(
        &root,
        &optimization,
        &executable,
        &game_data,
        &tape,
        &program,
        &world_context,
        &card_fixture,
        8,
        false,
    )
    .unwrap();
    let report = binding.validate_files(&root, &optimization).unwrap();
    assert_eq!(report.source_frame, 506);
    assert_eq!(report.exploration_horizon_ticks, 160);
    assert_eq!(report.process_boot_tape_frames, 666);
    assert_eq!(report.materialized_route_frames, 632);
    assert_eq!(report.runtime_dependencies, 2);
    assert_eq!(binding.runtime_dependencies.len(), 2);
    assert_eq!(report.workers, 4);
    binding.validate_seal(&optimization).unwrap();

    fs::write(_artifacts.0.join("renderer.dll"), b"tampered").unwrap();
    binding
        .validate_control_files(&root, &optimization)
        .unwrap();
    assert!(binding.validate_files(&root, &optimization).is_err());
    fs::write(_artifacts.0.join("renderer.dll"), b"renderer").unwrap();
    fs::write(&program, b"tampered").unwrap();
    // Lineage validation consumes the sealed execution identity and a
    // checkpoint bound to it, not the old runtime files. A fresh launch
    // still authenticates those files in validate_files.
    binding.validate_seal(&optimization).unwrap();
    assert!(
        binding
            .validate_control_files(&root, &optimization)
            .is_err()
    );
    assert!(binding.validate_files(&root, &optimization).is_err());
}

#[test]
fn validated_execution_authority_is_scoped_to_exact_inputs() {
    let (root, _artifacts, optimization, executable, game_data, tape, program, world_context) =
        fixture();
    let card_fixture = resolve_card_fixture_manifest(&root, &optimization).unwrap();
    let binding = NativeResidualExecutionBinding::seal(
        &root,
        &optimization,
        &executable,
        &game_data,
        &tape,
        &program,
        &world_context,
        &card_fixture,
        8,
        false,
    )
    .unwrap();
    let authority =
        ValidatedNativeResidualExecution::authenticate(&root, &optimization, &binding).unwrap();
    authority
        .validate_scope(&root, &optimization, &binding)
        .unwrap();

    let mut detached = binding.clone();
    detached.content_sha256 = Digest([0x5a; 32]);
    assert!(
        authority
            .validate_scope(&root, &optimization, &detached)
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn execution_binding_authenticates_only_a_final_external_game_data_symlink() {
    let (root, _artifacts, optimization, executable, game_data, tape, program, world_context) =
        fixture();
    let external = std::env::temp_dir().join(format!(
        "dusklight-native-residual-game-data-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&external).unwrap();
    let external_game = external.join("game.iso");
    fs::write(&external_game, b"game-data").unwrap();
    fs::remove_file(&game_data).unwrap();
    std::os::unix::fs::symlink(&external_game, &game_data).unwrap();
    let card_fixture = resolve_card_fixture_manifest(&root, &optimization).unwrap();

    let binding = NativeResidualExecutionBinding::seal(
        &root,
        &optimization,
        &executable,
        &game_data,
        &tape,
        &program,
        &world_context,
        &card_fixture,
        8,
        false,
    )
    .unwrap();
    assert!(binding.game_data.path.starts_with("build/"));
    binding.validate_files(&root, &optimization).unwrap();

    fs::write(&external_game, b"changed").unwrap();
    assert!(binding.validate_files(&root, &optimization).is_err());
    fs::write(&external_game, b"game-data").unwrap();

    let external_directory_link = game_data.with_file_name("external-game-directory");
    std::os::unix::fs::symlink(&external, &external_directory_link).unwrap();
    assert!(
        NativeResidualExecutionBinding::seal(
            &root,
            &optimization,
            &executable,
            &external_directory_link.join("game.iso"),
            &tape,
            &program,
            &world_context,
            &card_fixture,
            8,
            false,
        )
        .is_err()
    );
    let nested_final_link = external.join("nested-game.iso");
    std::os::unix::fs::symlink(&external_game, &nested_final_link).unwrap();
    assert!(
        NativeResidualExecutionBinding::seal(
            &root,
            &optimization,
            &executable,
            &external_directory_link.join("nested-game.iso"),
            &tape,
            &program,
            &world_context,
            &root.join("routes/Glitch Exhibition/intro/benchmarks/process_boot.fixture.json"),
            8,
            false,
        )
        .is_err()
    );

    fs::remove_dir_all(external).unwrap();
}

#[test]
fn checked_ordon_boundary_is_the_materialized_parent_checkpoint() {
    let (root, _artifacts, mut optimization, ..) = fixture();
    optimization.route.source_boundary_index = 500;
    optimization.refresh_content_sha256().unwrap();
    assert!(optimization.validate_files(&root).is_err());
    assert!(materialize_native_residual_process_tape(&root, &optimization).is_err());
}

#[test]
fn cold_replay_incumbent_replaces_only_the_selected_timeline_segment() {
    let (root, artifacts, mut optimization, ..) = fixture();
    let authored = materialized_route_authority(&root, &optimization)
        .unwrap()
        .tape;
    let incumbent = optimization.incumbent.as_mut().unwrap();
    let original_path = root.join(&incumbent.tape.path);
    let mut replacement = InputTape::decode(&fs::read(original_path).unwrap())
        .unwrap()
        .tape;
    replacement.frames[0].pads[0].stick_x = replacement.frames[0].pads[0].stick_x.wrapping_add(1);
    let replacement_path = artifacts.0.join("discovered-incumbent.tape");
    fs::write(&replacement_path, replacement.encode().unwrap()).unwrap();
    incumbent.tape = artifact_reference(&root, &replacement_path, false).unwrap();
    incumbent.authority = OptimizationIncumbentAuthority::TacticColdReplay {
        bundle_manifest: ArtifactReference {
            path: "build/not-read-by-materialization/manifest.json".into(),
            sha256: Digest([9; 32]),
        },
    };

    let derived = materialized_route_authority(&root, &optimization)
        .unwrap()
        .tape;
    let boundary = optimization.route.source_boundary_index as usize;
    assert_eq!(derived.frames[..boundary], authored.frames[..boundary]);
    assert_eq!(derived.frames[boundary..], replacement.frames);
    assert_ne!(derived, authored);
}
