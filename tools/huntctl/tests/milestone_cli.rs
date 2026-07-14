use huntctl::milestone_dsl;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn compiles_formats_and_inspects_authored_milestones() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("huntctl-milestone-{unique}"));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("route.milestones");
    let binary = directory.join("route.dmsp");
    fs::write(
        &source,
        r#"milestones 1.0
milestone process_boot {
  phase pre_input
  when boundary.kind == "boot" && boundary.index == 0
}

milestone link_control {
  phase post_sim
  stable 2
  when stage.name == "F_SP103" && player.exists && player.is_link && event.id == -1
}
"#,
    )
    .unwrap();

    let compile = Command::new(executable)
        .args([
            "milestone",
            "compile",
            source.to_str().unwrap(),
            binary.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let decoded = milestone_dsl::decode(&fs::read(&binary).unwrap()).unwrap();
    assert_eq!(decoded.definitions.len(), 2);

    let inspect = Command::new(executable)
        .args(["milestone", "inspect", binary.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(summary["format"], "DMSP");
    assert_eq!(summary["definitions"][0]["id"], "process_boot");
    assert_eq!(summary["definitions"][1]["id"], "link_control");
    assert!(summary["source"].as_str().unwrap().contains("stable 2"));

    let formatted = Command::new(executable)
        .args(["milestone", "format", source.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(formatted.status.success());
    assert_eq!(
        milestone_dsl::parse(std::str::from_utf8(&formatted.stdout).unwrap()).unwrap(),
        decoded.program
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compile_rejects_unknown_fields_without_writing_output() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("huntctl-milestone-bad-{unique}"));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("bad.milestones");
    let binary = directory.join("bad.dmsp");
    fs::write(
        &source,
        "milestones 1.0\nmilestone bad { phase post_sim when player.magic == true }\n",
    )
    .unwrap();
    let compile = Command::new(executable)
        .args([
            "milestone",
            "compile",
            source.to_str().unwrap(),
            binary.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!compile.status.success());
    assert!(String::from_utf8_lossy(&compile.stderr).contains("unknown milestone field"));
    assert!(!binary.exists());
    fs::remove_dir_all(directory).unwrap();
}
