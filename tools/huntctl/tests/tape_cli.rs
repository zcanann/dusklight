use huntctl::tape::InputTape;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn compiles_and_inspects_program_with_marker_sidecar() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("huntctl-tape-{unique}"));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("boot.tas");
    let tape = directory.join("boot.tape");
    let fixture_json = directory.join("fixture.json");
    let fixture_binary = directory.join("fixture.bin");
    fs::write(
        &fixture_json,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "dusklight-scenario-fixture/v1",
            "name": "low-health wolf",
            "form": "wolf",
            "health": {"current": 4, "maximum": 20}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &source,
        r#"dusktape 1
boot stage F_SP103 0 0 -1
state neutral {}
state start { p0 buttons START }
marker start
repeat 2 start
wait name_entry_active 900
frame neutral
"#,
    )
    .unwrap();

    let compile = Command::new(executable)
        .args([
            "tape",
            "compile",
            source.to_str().unwrap(),
            tape.to_str().unwrap(),
            "--fixture",
            fixture_json.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let decoded = InputTape::decode(&fs::read(&tape).unwrap()).unwrap();
    assert_eq!(decoded.tape.frames.len(), 4);
    assert_eq!(decoded.tape.frames[0].pads[0].buttons, 0x1000);
    assert_eq!(decoded.tape.frames[2].wait_timeout_ticks, 900);
    assert!(matches!(
        &decoded.tape.boot,
        huntctl::tape::TapeBoot::Stage {
            fixture: Some(value),
            ..
        } if value.name == "low-health wolf"
    ));
    assert!(
        fs::read_to_string(format!("{}.markers.json", tape.display()))
            .unwrap()
            .contains("start")
    );

    let inspect = Command::new(executable)
        .args(["tape", "inspect", tape.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(inspect.status.success());
    let summary: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(summary["source_version"]["major"], 3);
    assert_eq!(summary["source_version"]["minor"], 2);
    assert_eq!(summary["boot"]["kind"], "stage");
    assert_eq!(summary["boot"]["stage"], "F_SP103");
    assert_eq!(summary["boot"]["fixture"]["form"], "wolf");
    assert_eq!(summary["nominal_frame_count"], 4);
    assert_eq!(summary["wait_frame_count"], 1);
    assert_eq!(summary["minimum_tick_count"], 3);
    assert_eq!(summary["minimum_duration_seconds"], 0.1);
    assert_eq!(summary["wait_conditions"]["name_entry_active"], 1);

    let fixture_compile = Command::new(executable)
        .args([
            "fixture",
            "compile",
            fixture_json.to_str().unwrap(),
            fixture_binary.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(fixture_compile.status.success());
    let fixture_inspect = Command::new(executable)
        .args(["fixture", "inspect", fixture_binary.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(fixture_inspect.status.success());
    let inspected_fixture: serde_json::Value =
        serde_json::from_slice(&fixture_inspect.stdout).unwrap();
    assert_eq!(inspected_fixture["name"], "low-health wolf");

    let absolute_path = directory.join("absolute.tape");
    let mut absolute = decoded.tape.clone();
    for frame in &mut absolute.frames {
        frame.wait_condition = Default::default();
        frame.wait_timeout_ticks = 0;
    }
    fs::write(&absolute_path, absolute.encode().unwrap()).unwrap();

    let dvd = directory.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let state_root = directory.join("run-state");
    let run = Command::new(executable)
        .args([
            "tape",
            "run",
            absolute_path.to_str().unwrap(),
            "--game",
            executable,
            "--game-arg",
            "mock-search-worker",
            "--dvd",
            dvd.to_str().unwrap(),
            "--state-root",
            state_root.to_str().unwrap(),
            "--milestone-goal",
            "gameplay-ready-f-sp103",
            "--timeout-seconds",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let run_summary: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(run_summary["schema"], "huntctl-tape-run/v1");
    assert_eq!(run_summary["boot"]["kind"], "stage");
    assert_eq!(run_summary["exit_code"], 0);
    assert!(state_root.join("milestones.json").is_file());

    let minimized_path = directory.join("minimized.tape");
    let minimize = Command::new(executable)
        .args([
            "tape",
            "minimize",
            absolute_path.to_str().unwrap(),
            minimized_path.to_str().unwrap(),
            "--game",
            executable,
            "--game-arg",
            "mock-search-worker",
            "--dvd",
            dvd.to_str().unwrap(),
            "--state-root",
            directory.join("minimize-state").to_str().unwrap(),
            "--milestone-goal",
            "arbitrary-map-goal",
            "--repetitions",
            "2",
            "--timeout-seconds",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        minimize.status.success(),
        "{}",
        String::from_utf8_lossy(&minimize.stderr)
    );
    let minimize_summary: serde_json::Value = serde_json::from_slice(&minimize.stdout).unwrap();
    assert_eq!(minimize_summary["schema"], "huntctl-tape-minimization/v1");
    assert_eq!(minimize_summary["boot"]["kind"], "stage");
    assert_eq!(minimize_summary["minimized_frames"], 1);
    assert_eq!(minimize_summary["minimized_active_frames"], 0);
    assert_eq!(
        minimize_summary["proof"]["boundary_fingerprint"]["schema"],
        "dusklight.milestone-boundary/v4"
    );
    let minimized = InputTape::decode(&fs::read(&minimized_path).unwrap())
        .unwrap()
        .tape;
    assert_eq!(minimized.boot, absolute.boot);
    assert_eq!(minimized.frames.len(), 1);

    let recorded_path = directory.join("recorded.tape");
    let record = Command::new(executable)
        .args([
            "tape",
            "record",
            absolute_path.to_str().unwrap(),
            recorded_path.to_str().unwrap(),
            "--game",
            executable,
            "--game-arg",
            "mock-record-worker",
            "--dvd",
            dvd.to_str().unwrap(),
            "--state-root",
            directory.join("record-state").to_str().unwrap(),
            "--timeout-seconds",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        record.status.success(),
        "{}",
        String::from_utf8_lossy(&record.stderr)
    );
    let record_summary: serde_json::Value = serde_json::from_slice(&record.stdout).unwrap();
    assert_eq!(record_summary["boot"]["kind"], "stage");
    assert_eq!(record_summary["seed_frames"], 4);
    assert_eq!(record_summary["recorded_frames"], 1);
    let recorded = InputTape::decode(&fs::read(&recorded_path).unwrap())
        .unwrap()
        .tape;
    assert_eq!(recorded.boot, absolute.boot);
    assert_eq!(recorded.frames.len(), 5);

    let concatenated = directory.join("nested").join("chain.tape");
    let concat = Command::new(executable)
        .args([
            "tape",
            "concat",
            concatenated.to_str().unwrap(),
            absolute_path.to_str().unwrap(),
            absolute_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        concat.status.success(),
        "{}",
        String::from_utf8_lossy(&concat.stderr)
    );
    let chain = InputTape::decode(&fs::read(&concatenated).unwrap())
        .unwrap()
        .tape;
    assert_eq!(chain.frames.len(), 8);
    assert_eq!(chain.frames[0], chain.frames[4]);

    let slice_path = directory.join("nested").join("slice.tape");
    let slice = Command::new(executable)
        .args([
            "tape",
            "slice",
            concatenated.to_str().unwrap(),
            slice_path.to_str().unwrap(),
            "--start",
            "2",
            "--frames",
            "3",
        ])
        .output()
        .unwrap();
    assert!(
        slice.status.success(),
        "{}",
        String::from_utf8_lossy(&slice.stderr)
    );
    let sliced = InputTape::decode(&fs::read(&slice_path).unwrap())
        .unwrap()
        .tape;
    assert_eq!(sliced.frames, chain.frames[2..5]);

    let overlay_path = directory.join("overlay.tape");
    let mut overlay = absolute.clone();
    overlay.boot = Default::default();
    overlay.frames.truncate(2);
    for (index, frame) in overlay.frames.iter_mut().enumerate() {
        frame.owned_ports = 0b0010;
        frame.pads[1].stick_x = 40 + index as i8;
    }
    fs::write(&overlay_path, overlay.encode().unwrap()).unwrap();
    let layered_path = directory.join("nested").join("layered.tape");
    let layer = Command::new(executable)
        .args([
            "tape",
            "layer",
            absolute_path.to_str().unwrap(),
            overlay_path.to_str().unwrap(),
            layered_path.to_str().unwrap(),
            "--start",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        layer.status.success(),
        "{}",
        String::from_utf8_lossy(&layer.stderr)
    );
    let layered = InputTape::decode(&fs::read(&layered_path).unwrap())
        .unwrap()
        .tape;
    assert_eq!(layered.boot, absolute.boot);
    assert_eq!(layered.frames[0], absolute.frames[0]);
    assert_eq!(layered.frames[1].pads[0], absolute.frames[1].pads[0]);
    assert_eq!(layered.frames[1].pads[1].stick_x, 40);
    assert_eq!(layered.frames[2].pads[1].stick_x, 41);

    overlay.tick_rate_numerator = 60;
    let authoring_path = directory.join("authoring-60hz.tape");
    fs::write(&authoring_path, overlay.encode().unwrap()).unwrap();
    let resampled_path = directory.join("nested").join("resampled.tape");
    let resample = Command::new(executable)
        .args([
            "tape",
            "resample",
            authoring_path.to_str().unwrap(),
            resampled_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        resample.status.success(),
        "{}",
        String::from_utf8_lossy(&resample.stderr)
    );
    let resampled = InputTape::decode(&fs::read(&resampled_path).unwrap())
        .unwrap()
        .tape;
    assert_eq!(
        (
            resampled.tick_rate_numerator,
            resampled.tick_rate_denominator
        ),
        (30, 1)
    );
    assert_eq!(resampled.frames, overlay.frames[..1]);

    let diff = Command::new(executable)
        .args([
            "tape",
            "diff",
            absolute_path.to_str().unwrap(),
            layered_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(diff.status.success());
    let diff: serde_json::Value = serde_json::from_slice(&diff.stdout).unwrap();
    assert_eq!(diff["schema"], "huntctl-tape-diff/v1");
    assert_eq!(diff["identical"], false);
    assert_eq!(diff["differing_frame_count"], 2);
    assert!(
        diff["frames"][0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["field"] == "p1.stick_x")
    );

    let invalid_slice = Command::new(executable)
        .args([
            "tape",
            "slice",
            concatenated.to_str().unwrap(),
            slice_path.to_str().unwrap(),
            "--start",
            "7",
            "--frames",
            "2",
        ])
        .output()
        .unwrap();
    assert!(!invalid_slice.status.success());

    let missing_input = Command::new(executable)
        .args([
            "tape",
            "concat",
            concatenated.to_str().unwrap(),
            absolute_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!missing_input.status.success());
    fs::remove_dir_all(directory).unwrap();
}
