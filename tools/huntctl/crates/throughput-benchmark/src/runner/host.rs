use crate::report::ColdProcessBenchmarkHost;
use crate::{ColdProcessBenchmarkError, benchmark_error};
use std::process::Command;

pub(super) fn capture_host() -> Result<ColdProcessBenchmarkHost, ColdProcessBenchmarkError> {
    let logical_cpu_count = std::thread::available_parallelism()
        .map_err(|error| benchmark_error(format!("cannot query logical CPU count: {error}")))?
        .get();
    let operating_system_version = if cfg!(target_os = "macos") {
        command_value("sw_vers", &["-productVersion"])
    } else {
        command_value("uname", &["-sr"])
    };
    Ok(ColdProcessBenchmarkHost {
        operating_system: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        operating_system_version,
        hardware_model: cfg!(target_os = "macos")
            .then(|| command_value("sysctl", &["-n", "hw.model"]))
            .flatten(),
        cpu_model: cfg!(target_os = "macos")
            .then(|| command_value("sysctl", &["-n", "machdep.cpu.brand_string"]))
            .flatten(),
        logical_cpu_count,
        memory_bytes: cfg!(target_os = "macos")
            .then(|| command_value("sysctl", &["-n", "hw.memsize"]))
            .flatten()
            .and_then(|value| value.parse().ok()),
    })
}

fn command_value(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.into())
}
