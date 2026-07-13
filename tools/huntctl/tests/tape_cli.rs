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
    let source = directory.join("boot.json");
    let tape = directory.join("boot.tape");
    fs::write(
        &source,
        r#"{
      "schema":"dusktape-program/v1",
      "steps":[
        {"op":"marker","name":"start"},
        {"op":"repeat","count":2,"frame":{"pads":{"0":{"buttons":["START"]}}}},
        {"op":"frame","frame":{}}
      ]
    }"#,
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
    assert_eq!(decoded.tape.frames.len(), 3);
    assert_eq!(decoded.tape.frames[0].pads[0].buttons, 0x1000);
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
    assert!(
        String::from_utf8(inspect.stdout)
            .unwrap()
            .contains("\"frame_count\": 3")
    );
    fs::remove_dir_all(directory).unwrap();
}
