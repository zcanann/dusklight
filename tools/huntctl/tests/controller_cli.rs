use huntctl::controller_program::{ControllerProgram, HEADER_SIZE, RECORD_SIZE};
use huntctl::tape::InputTape;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn decode_hex(source: &str) -> Vec<u8> {
    let digits = source
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(digits.len() % 2, 0);
    digits
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).unwrap() as u8;
            let low = (pair[1] as char).to_digit(16).unwrap() as u8;
            (high << 4) | low
        })
        .collect()
}

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
        include_str!("../../../tests/fixtures/automation/move_targets.duskctl"),
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
    assert_eq!(bytes.len(), HEADER_SIZE + 13 * RECORD_SIZE);
    let golden = decode_hex(include_str!(
        "../../../tests/fixtures/automation/move_targets.dctl.hex"
    ));
    assert_eq!(bytes, golden);
    let decoded = ControllerProgram::decode(&bytes).unwrap();
    assert_eq!(decoded.duration_frames, 10);
    assert_eq!(decoded.layers.len(), 13);

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
    assert_eq!(summary["version"]["minor"], 4);
    assert_eq!(summary["duration_frames"], 10);
    assert_eq!(summary["layer_count"], 13);
    assert_eq!(summary["layers"][0]["kind"], "seek_coordinate");
    assert_eq!(summary["layers"][0]["frame"], "player");
    assert_eq!(summary["layers"][1]["kind"], "seek_plane");
    assert_eq!(summary["layers"][2]["kind"], "seek_resolved");
    assert_eq!(summary["layers"][2]["target"]["path_id"], 42);
    assert_eq!(summary["layers"][2]["target"]["point_index"], 7);
    assert_eq!(summary["layers"][3]["kind"], "seek_resolved");
    assert_eq!(summary["layers"][3]["target"]["opening_id"], 99);
    assert_eq!(summary["layers"][4]["kind"], "neutral");
    assert_eq!(summary["layers"][5]["kind"], "turn");
    assert_eq!(summary["layers"][6]["kind"], "brake");
    assert_eq!(summary["layers"][7]["kind"], "align");
    assert_eq!(summary["layers"][8]["kind"], "maintain_heading");
    assert_eq!(summary["layers"][9]["kind"], "maintain_distance");
    assert_eq!(summary["static_tape_compilable"], false);
    assert_eq!(
        summary["observation_provenance"]["schema"],
        "dusklight-controller-observation-provenance/v1"
    );
    assert_eq!(
        summary["observation_provenance"]["reactive_layers"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn flattens_static_controller_to_canonical_tape() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("huntctl-controller-flat-{unique}"));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("static.duskctl");
    let controller = directory.join("static.dctl");
    let tape = directory.join("static.tape");
    fs::write(
        &source,
        "duskcontrol 1\nframes 2\nturn replace from 0 for 2 direction right magnitude 70\ncamera replace from 0 for 2 x -20 y 30\nbuttons from 0 for 2 A\n",
    )
    .unwrap();
    assert!(
        Command::new(executable)
            .args([
                "controller",
                "compile",
                source.to_str().unwrap(),
                controller.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(executable)
            .args([
                "controller",
                "flatten",
                controller.to_str().unwrap(),
                tape.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success()
    );
    let decoded = InputTape::decode(&fs::read(&tape).unwrap()).unwrap().tape;
    assert_eq!(decoded.frames.len(), 2);
    assert_eq!(decoded.frames[0].pads[0].stick_x, 70);
    assert_eq!(decoded.frames[0].pads[0].substick_x, -20);
    assert_eq!(decoded.frames[0].pads[0].substick_y, 30);
    assert_eq!(decoded.frames[0].pads[0].buttons, 0x0100);
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
