//! Revision-pinned benchmark metadata import and selection validation.

use crate::{option, required_path, u32_option};
use huntctl::benchmark::skybook::SkybookManifest;
use huntctl::benchmark::skybook_selection::{SkybookSelection, SkybookSelectionDisposition};
use huntctl::harness::run_contract::HarnessRunRequest;
use huntctl::throughput_benchmark::{
    ColdProcessBenchmarkConfig, ColdProcessBenchmarkReport, run_cold_process_benchmark,
};
use serde_json::json;
use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

pub(crate) fn command_benchmark(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("import-skybook") => {
            let import_args = &args[1..];
            let source = required_path(import_args, "--source")?;
            let output = required_path(import_args, "--output")?;
            if output.exists() {
                return Err(format!("Skybook manifest already exists: {}", output.display()).into());
            }
            let revision = clean_git_revision(&source, "_posts")?;
            if let Some(expected) = option(import_args, "--revision")
                && expected != revision
            {
                return Err(format!(
                    "Skybook checkout revision {revision} does not match requested {expected}"
                )
                .into());
            }
            let repository = option(import_args, "--repository")
                .unwrap_or_else(|| "https://github.com/qwertyquerty/skybook".into());
            let manifest = SkybookManifest::import_directory(&source, &repository, &revision)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, manifest.to_pretty_json()?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": manifest.schema,
                    "source_revision": manifest.source.git_revision,
                    "page_count": manifest.source.post_count,
                    "categorized_glitch_count": manifest.source.categorized_glitch_count,
                    "content_digest": manifest.content_sha256,
                    "output": output,
                }))?
            );
            Ok(())
        }
        Some("validate-skybook-selection") => {
            let selection_args = &args[1..];
            let manifest_path = required_path(selection_args, "--manifest")?;
            let selection_path = required_path(selection_args, "--selection")?;
            let manifest: SkybookManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
            let selection: SkybookSelection =
                serde_json::from_slice(&fs::read(&selection_path)?)?;
            selection.validate_against(&manifest)?;
            let selected = selection
                .entries
                .iter()
                .filter(|entry| entry.disposition == SkybookSelectionDisposition::Selected)
                .count();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": selection.schema,
                    "content_digest": selection.content_sha256,
                    "source_revision": selection.source_git_revision,
                    "approved_by": selection.approved_by,
                    "selected_page_count": selected,
                    "entry_count": selection.entries.len(),
                    "selection": selection_path,
                }))?
            );
            Ok(())
        }
        Some("cold-process") => {
            let benchmark_args = &args[1..];
            let request_path = required_path(benchmark_args, "--request")?;
            let output = required_path(benchmark_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "cold-process report already exists: {}",
                    output.display()
                )
                .into());
            }
            let repository_root = option(benchmark_args, "--repository-root")
                .map(Into::into)
                .unwrap_or(env::current_dir()?);
            let artifact_destination_root = option(benchmark_args, "--artifact-root")
                .ok_or("benchmark cold-process requires --artifact-root RELATIVE_ROOT")?;
            let request: HarnessRunRequest =
                serde_json::from_slice(&fs::read(&request_path)?)?;
            let report = run_cold_process_benchmark(&ColdProcessBenchmarkConfig {
                request_template: &request,
                repository_root: &repository_root,
                artifact_destination_root: &artifact_destination_root,
                repetitions: u32_option(benchmark_args, "--repetitions", 5)?,
            })?;
            write_new_file(&output, &report.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if let Some(issue) = &report.comparison_issue {
                return Err(format!(
                    "cold-process attempts are not comparable: {issue}; report: {}",
                    output.display()
                )
                .into());
            }
            Ok(())
        }
        Some("validate-cold-process") => {
            let report: ColdProcessBenchmarkReport = serde_json::from_slice(&fs::read(
                required_path(&args[1..], "--report")?,
            )?)?;
            report.validate()?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        _ => Err("benchmark command:\n  import-skybook --source CHECKOUT --output MANIFEST.json [--revision FULL_GIT_REVISION] [--repository URL]\n  validate-skybook-selection --manifest MANIFEST.json --selection SELECTION.json\n  cold-process --request REQUEST.json --artifact-root RELATIVE_ROOT --output REPORT.json [--repository-root ROOT] [--repetitions N]\n  validate-cold-process --report REPORT.json".into()),
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}

fn clean_git_revision(checkout: &Path, imported_path: &str) -> Result<String, Box<dyn Error>> {
    let revision_output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["rev-parse", "HEAD"])
        .output()?;
    if !revision_output.status.success() {
        return Err(format!(
            "cannot resolve Git revision for {}: {}",
            checkout.display(),
            String::from_utf8_lossy(&revision_output.stderr).trim()
        )
        .into());
    }
    let revision = String::from_utf8(revision_output.stdout)?.trim().to_owned();
    let status_output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args([
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            imported_path,
        ])
        .output()?;
    if !status_output.status.success() {
        return Err(format!(
            "cannot inspect Git state for {}: {}",
            checkout.display(),
            String::from_utf8_lossy(&status_output.stderr).trim()
        )
        .into());
    }
    let dirty = String::from_utf8(status_output.stdout)?;
    if !dirty.trim().is_empty() {
        return Err(format!(
            "refusing to import dirty Skybook {imported_path} content at {revision}:\n{}",
            dirty.trim_end()
        )
        .into());
    }
    Ok(revision)
}
