//! Revision-pinned benchmark metadata import and selection validation.

use crate::{option, required_path};
use huntctl::benchmark::skybook::SkybookManifest;
use huntctl::benchmark::skybook_selection::{SkybookSelection, SkybookSelectionDisposition};
use serde_json::json;
use std::error::Error;
use std::fs;
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
        _ => Err("benchmark command:\n  import-skybook --source CHECKOUT --output MANIFEST.json [--revision FULL_GIT_REVISION] [--repository URL]\n  validate-skybook-selection --manifest MANIFEST.json --selection SELECTION.json".into()),
    }
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
