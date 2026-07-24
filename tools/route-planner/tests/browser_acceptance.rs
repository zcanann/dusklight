use std::path::{Path, PathBuf};
use std::process::Command;

fn browser_binary() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("ROUTE_PLANNER_BROWSER") {
        let configured = PathBuf::from(configured);
        assert!(
            configured.is_file(),
            "ROUTE_PLANNER_BROWSER does not name a browser executable: {}",
            configured.display()
        );
        return Some(configured);
    }
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ]
    } else {
        &[
            "/usr/bin/google-chrome",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
        ]
    };
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
}

#[test]
fn chromium_edits_saves_and_reloads_a_demonstration() {
    let Some(browser) = browser_binary() else {
        assert!(
            !std::env::var("ROUTE_PLANNER_REQUIRE_BROWSER").is_ok_and(|value| value == "1"),
            "browser acceptance is required, but no supported Chromium executable was found"
        );
        eprintln!("skipping browser acceptance: no supported Chromium executable was found");
        return;
    };
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/browser_acceptance.mjs");
    let status = Command::new("node")
        .arg(script)
        .env("ROUTE_PLANNER_BINARY", env!("CARGO_BIN_EXE_route-planner"))
        .env("ROUTE_PLANNER_BROWSER", browser)
        .status()
        .expect("Node 22 or newer is required for the browser acceptance test");
    assert!(status.success(), "browser acceptance script failed");
}
