use huntctl::controller_program::{ControllerProgram, HEADER_SIZE, RECORD_SIZE};
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn compiles_and_inspects_reactive_controller() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("huntctl-controller-{unique}"));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("movement.duskctl");
    let output = directory.join("movement.dctl");
    fs::write(
        &source,
        r#"duskcontrol 1
frames 60
bezier replace from 0 for 60 p0 0 127 p1 0 127 p2 127 0 p3 127 0
seek actor add from 0 for 60 actor 123 offset 1 0 2 magnitude 30 stop 10
buttons from 5 for 1 B
"#,
    )
    .unwrap();

    let compile = Command::new(executable)
        .args([
            "controller",
            "compile",
            source.to_str().unwrap(),
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let bytes = fs::read(&output).unwrap();
    assert_eq!(bytes.len(), HEADER_SIZE + 3 * RECORD_SIZE);
    let decoded = ControllerProgram::decode(&bytes).unwrap();
    assert_eq!(decoded.duration_frames, 60);
    assert_eq!(decoded.layers.len(), 3);

    let inspect = Command::new(executable)
        .args(["controller", "inspect", output.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(summary["format"], "DUSKCTRL");
    assert_eq!(summary["version"]["major"], 1);
    assert_eq!(summary["duration_frames"], 60);
    assert_eq!(summary["layer_count"], 3);
    assert_eq!(summary["layers"][0]["kind"], "cubic_bezier");
    assert_eq!(summary["layers"][1]["kind"], "seek_actor");
    assert_eq!(summary["layers"][2]["mask"], 0x0200);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compile_reports_source_line_errors_without_output() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("huntctl-controller-bad-{unique}"));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("bad.duskctl");
    let output = directory.join("bad.dctl");
    fs::write(
        &source,
        "duskcontrol 1\nframes 10\nbuttons from 0 for 1 A A\n",
    )
    .unwrap();
    let compile = Command::new(executable)
        .args([
            "controller",
            "compile",
            source.to_str().unwrap(),
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!compile.status.success());
    assert!(String::from_utf8_lossy(&compile.stderr).contains("line 3"));
    assert!(!output.exists());
    fs::remove_dir_all(directory).unwrap();
}
