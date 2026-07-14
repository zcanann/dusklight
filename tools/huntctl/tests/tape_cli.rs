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
    fs::write(
        &source,
        r#"dusktape 1
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
    assert_eq!(summary["source_version"]["major"], 2);
    assert_eq!(summary["source_version"]["minor"], 0);
    assert_eq!(summary["nominal_frame_count"], 4);
    assert_eq!(summary["wait_frame_count"], 1);
    assert_eq!(summary["minimum_tick_count"], 3);
    assert_eq!(summary["minimum_duration_seconds"], 0.1);
    assert_eq!(summary["wait_conditions"]["name_entry_active"], 1);

    let absolute_path = directory.join("absolute.tape");
    let mut absolute = decoded.tape.clone();
    for frame in &mut absolute.frames {
        frame.wait_condition = Default::default();
        frame.wait_timeout_ticks = 0;
    }
    fs::write(&absolute_path, absolute.encode().unwrap()).unwrap();
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
