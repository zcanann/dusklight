use std::path::Path;
use std::process::Command;

#[test]
fn brave_edits_saves_and_reloads_a_demonstration() {
    let brave = Path::new("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser");
    if !brave.is_file() {
        eprintln!("skipping browser acceptance: Brave is not installed at the documented path");
        return;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/browser_acceptance.mjs");
    let status = Command::new("node")
        .arg(script)
        .env("ROUTE_PLANNER_BINARY", env!("CARGO_BIN_EXE_route-planner"))
        .env("ROUTE_PLANNER_BRAVE", brave)
        .status()
        .expect("Node 22 or newer is required for the browser acceptance test");
    assert!(status.success(), "browser acceptance script failed");
}
