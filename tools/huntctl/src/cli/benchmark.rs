//! Revision-pinned benchmark metadata import and selection validation.

use crate::{option, required_path, u32_option};
use huntctl::benchmark::skybook::SkybookManifest;
use huntctl::benchmark::skybook_pilot::SkybookPilot;
use huntctl::benchmark::skybook_selection::{SkybookSelection, SkybookSelectionDisposition};
use huntctl::harness::objective_suite::{
    ArtifactReference, ExpectedTerminalClass, ObjectiveBoot, ObjectiveCaseRole,
    ObjectiveProgramReference, ObjectiveSeed, ObjectiveSuiteCase, ObservationViewReference,
    SchemaIdentity,
};
use huntctl::harness::observation_contract::{
    OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1, ObjectiveObservationRequirements,
    ObservationFamilyRequirement, family_for_fact,
};
use huntctl::harness::request_materialization::{
    NativeRequestConfig, inspect_native_inputs, materialize_native_request, protocol_for_cases,
};
use huntctl::harness::run_contract::{
    HarnessFidelityMode, HarnessRunRequest, sha256_artifact_file,
};
use huntctl::learning::offline_rl::{MovementActionSchema, movement_action_schema_digest_v2};
use huntctl::observation_view::ObservationSpec;
use huntctl::throughput_benchmark::{
    ColdProcessBenchmarkConfig, ColdProcessBenchmarkReport, run_cold_process_benchmark,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
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
        Some("validate-skybook-pilot") => {
            let pilot_args = &args[1..];
            let manifest_path = required_path(pilot_args, "--manifest")?;
            let pilot_path = required_path(pilot_args, "--pilot")?;
            let repository_root = option(pilot_args, "--repository-root")
                .map(Into::into)
                .unwrap_or(env::current_dir()?);
            let manifest: SkybookManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
            let pilot: SkybookPilot = serde_json::from_slice(&fs::read(&pilot_path)?)?;
            pilot.validate_against(&manifest, &repository_root)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": pilot.schema,
                    "content_sha256": pilot.content_sha256,
                    "source_revision": pilot.source_git_revision,
                    "page_slug": pilot.page.slug,
                    "positive_tape_sha256": pilot.positive_case.tape_sha256,
                    "negative_control": pilot.negative_control.id,
                    "implementation_artifact_count": pilot.implementation_artifacts.len(),
                    "pilot": pilot_path,
                }))?
            );
            Ok(())
        }
        Some("route-cold-process") => route_cold_process(&args[1..]),
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
                prefix_ticks: option(benchmark_args, "--prefix-ticks")
                    .map(|value| value.parse::<u64>())
                    .transpose()?,
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
        _ => Err("benchmark command:\n  import-skybook --source CHECKOUT --output MANIFEST.json [--revision FULL_GIT_REVISION] [--repository URL]\n  validate-skybook-selection --manifest MANIFEST.json --selection SELECTION.json\n  validate-skybook-pilot --manifest MANIFEST.json --pilot PILOT.json [--repository-root ROOT]\n  route-cold-process --timeline FILE --segment ID --goal GOAL --game PATH --dvd PATH --artifact-root RELATIVE_ROOT [--output REPORT.json] [--repository-root ROOT] [--repetitions N] [--timeout-seconds N]\n  cold-process --request REQUEST.json --artifact-root RELATIVE_ROOT --output REPORT.json [--repository-root ROOT] [--repetitions N] [--prefix-ticks N]\n  validate-cold-process --report REPORT.json".into()),
    }
}

fn route_cold_process(args: &[String]) -> Result<(), Box<dyn Error>> {
    let repository_root = option(args, "--repository-root")
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?)
        .canonicalize()?;
    let timeline_path = resolve_repository_file(
        &repository_root,
        &required_path(args, "--timeline")?,
        "timeline",
    )?;
    let timeline = huntctl::timeline::Timeline::parse(&fs::read_to_string(&timeline_path)?)?;
    let timeline_artifact_root = timeline_path
        .parent()
        .ok_or("route timeline has no artifact root")?;
    timeline.validate_artifacts(Some(timeline_artifact_root))?;

    let segment_id = option(args, "--segment").ok_or("missing required --segment ID")?;
    let segment = timeline
        .segments
        .get(&segment_id)
        .ok_or_else(|| format!("unknown timeline segment {segment_id:?}"))?;
    let parent_id = segment.parent.as_deref().ok_or_else(|| {
        format!("route benchmark segment {segment_id:?} has no parent-prefix boundary")
    })?;
    let goal_id = option(args, "--goal").ok_or("missing required --goal GOAL")?;
    let goal = timeline
        .goals
        .get(&goal_id)
        .ok_or_else(|| format!("unknown timeline goal {goal_id:?}"))?;
    let segment_owns_goal = goal.segment == segment_id
        || timeline
            .proofs
            .iter()
            .any(|proof| proof.segment == segment_id && proof.goal == goal_id);
    if !segment_owns_goal {
        return Err(
            format!("segment {segment_id:?} neither defines nor proves goal {goal_id:?}").into(),
        );
    }

    let prefix = huntctl::route_workbench::materialize_segment_chain(
        &timeline,
        timeline_artifact_root,
        parent_id,
    )?;
    let full = huntctl::route_workbench::materialize_segment_chain(
        &timeline,
        timeline_artifact_root,
        &segment_id,
    )?;
    if full.steps.len() != prefix.steps.len() + 1
        || full.steps.last().map(|step| step.segment.as_str()) != Some(segment_id.as_str())
        || full.steps[..prefix.steps.len()]
            .iter()
            .map(|step| step.segment.as_str())
            .ne(prefix.steps.iter().map(|step| step.segment.as_str()))
    {
        return Err(format!(
            "segment {segment_id:?} does not extend its materialized parent chain exactly once"
        )
        .into());
    }

    let artifact_destination_root = option(args, "--artifact-root")
        .ok_or("route-cold-process requires --artifact-root RELATIVE_ROOT")?;
    let benchmark_relative =
        canonical_relative_path(&artifact_destination_root, "benchmark artifact root")?;
    let benchmark_root = repository_root.join(&benchmark_relative);
    if benchmark_root.exists() {
        return Err(format!(
            "route benchmark artifact root already exists: {}",
            benchmark_root.display()
        )
        .into());
    }
    let input_relative = canonical_relative_path(
        &format!("{artifact_destination_root}-inputs"),
        "benchmark input root",
    )?;
    let input_root = repository_root.join(&input_relative);
    if input_root.exists() {
        return Err(format!(
            "route benchmark input root already exists: {}",
            input_root.display()
        )
        .into());
    }
    fs::create_dir_all(&input_root)?;
    let tape_path = input_root.join("full.tape");
    write_new_file(&tape_path, &full.tape.encode()?)?;

    let goal_source_relative = timeline
        .goal_predicate_source(&goal_id)
        .ok_or_else(|| format!("route goal {goal_id:?} has no predicate source"))?;
    let goal_source_path = resolve_repository_file(
        &repository_root,
        &timeline_artifact_root.join(goal_source_relative),
        "goal predicate source",
    )?;
    let goal_source = fs::read_to_string(&goal_source_path)?;
    let goal_program = huntctl::milestone_dsl::parse(&goal_source)?;
    let compiled_goal = huntctl::milestone_dsl::compile(&goal_program)?;
    if !goal_program
        .definitions
        .iter()
        .any(|definition| definition.name == goal.predicate)
    {
        return Err(format!(
            "goal source {} does not define predicate {:?}",
            goal_source_path.display(),
            goal.predicate
        )
        .into());
    }
    let required_facts =
        huntctl::milestone_dsl::required_query_facts(&goal_program, &goal.predicate)?;
    let mut families = BTreeSet::new();
    for fact in &required_facts {
        families.insert(
            family_for_fact(fact)
                .ok_or_else(|| format!("goal fact {fact:?} has no observation family"))?
                .to_owned(),
        );
    }

    let support_root = timeline_path.with_extension("").join("benchmarks");
    let scenario_path = option(args, "--scenario")
        .map(PathBuf::from)
        .unwrap_or_else(|| support_root.join("process_boot.fixture.json"));
    let observation_path = option(args, "--observation")
        .map(PathBuf::from)
        .unwrap_or_else(|| support_root.join(format!("{goal_id}.observation.json")));
    let scenario_path = resolve_repository_file(&repository_root, &scenario_path, "scenario")?;
    let observation_path =
        resolve_repository_file(&repository_root, &observation_path, "observation view")?;
    let observation: ObservationSpec = serde_json::from_slice(&fs::read(&observation_path)?)?;
    observation.validate()?;
    if observation.objective.id != goal.predicate {
        return Err(format!(
            "observation view objective {:?} does not match predicate {:?}",
            observation.objective.id, goal.predicate
        )
        .into());
    }

    let repetitions = u32_option(args, "--repetitions", 5)?;
    let case = ObjectiveSuiteCase {
        id: format!("route-cold-{segment_id}"),
        description: format!(
            "Cold-process benchmark of segment {segment_id} reaching {}.",
            goal.predicate
        ),
        role: ObjectiveCaseRole::Positive,
        control_for: None,
        boot: ObjectiveBoot::Process,
        scenario: artifact_reference(&repository_root, &scenario_path)?,
        objective: ObjectiveProgramReference {
            source: artifact_reference(&repository_root, &goal_source_path)?,
            program_sha256: huntctl::Digest(compiled_goal.program_sha256),
            goal: goal.predicate.clone(),
        },
        observation_view: ObservationViewReference {
            source: artifact_reference(&repository_root, &observation_path)?,
            schema_sha256: observation.digest()?,
        },
        action_schema: SchemaIdentity {
            id: MovementActionSchema::V2.id().into(),
            sha256: movement_action_schema_digest_v2(),
        },
        observation_requirements: ObjectiveObservationRequirements {
            schema: OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1.into(),
            families: families
                .into_iter()
                .map(|id| ObservationFamilyRequirement {
                    id,
                    minimum_version: 1,
                })
                .collect(),
            facts: required_facts,
        },
        seed: ObjectiveSeed::Tape {
            artifact: artifact_reference(&repository_root, &tape_path)?,
        },
        logical_tick_budget: u64::try_from(full.tape.frames.len())?,
        host_timeout_seconds: u32_option(args, "--timeout-seconds", 120)?,
        repetitions: u16::try_from(repetitions)?,
        expected_terminal: ExpectedTerminalClass::Reached,
    };
    let inputs = inspect_native_inputs(
        &repository_root,
        &required_path(args, "--game")?,
        &required_path(args, "--dvd")?,
    )?;
    let protocol = protocol_for_cases(std::slice::from_ref(&case))?;
    let template_destination = benchmark_relative.join("template");
    let request = materialize_native_request(&NativeRequestConfig {
        case: &case,
        inputs: &inputs,
        protocol: &protocol,
        request_id: &format!("route-cold-{segment_id}"),
        artifact_destination: &template_destination,
        fidelity: HarnessFidelityMode::Headless,
        native_evidence: None,
        rng_seed: 0x4455_534b_524f_5554,
    })?;
    let report = run_cold_process_benchmark(&ColdProcessBenchmarkConfig {
        request_template: &request,
        repository_root: &repository_root,
        artifact_destination_root: &path_string(&benchmark_relative, "benchmark root")?,
        repetitions,
        prefix_ticks: Some(u64::try_from(prefix.tape.frames.len())?),
    })?;
    let output = option(args, "--output")
        .map(PathBuf::from)
        .unwrap_or_else(|| benchmark_root.join("report.json"));
    write_new_file(&output, &report.to_pretty_json()?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(issue) = &report.comparison_issue {
        return Err(format!(
            "route cold-process attempts are not comparable: {issue}; report: {}",
            output.display()
        )
        .into());
    }
    Ok(())
}

fn resolve_repository_file(
    repository_root: &Path,
    authored: &Path,
    label: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let path = if authored.is_absolute() {
        authored.to_path_buf()
    } else {
        repository_root.join(authored)
    };
    let path = path
        .canonicalize()
        .map_err(|error| format!("cannot resolve {label} {}: {error}", path.display()))?;
    if !path.is_file() || !path.starts_with(repository_root) {
        return Err(format!("{label} must be a file beneath the repository root").into());
    }
    Ok(path)
}

fn canonical_relative_path(value: &str, label: &str) -> Result<PathBuf, Box<dyn Error>> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("{label} must be a canonical relative path").into());
    }
    Ok(path)
}

fn artifact_reference(
    repository_root: &Path,
    path: &Path,
) -> Result<ArtifactReference, Box<dyn Error>> {
    let relative = path
        .strip_prefix(repository_root)
        .map_err(|_| format!("artifact {} escaped repository", path.display()))?;
    Ok(ArtifactReference {
        path: path_string(relative, "artifact")?,
        sha256: sha256_artifact_file(path)?,
    })
}

fn path_string(path: &Path, label: &str) -> Result<String, Box<dyn Error>> {
    path.to_str()
        .map(|value| value.replace(std::path::MAIN_SEPARATOR, "/"))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{label} is not valid UTF-8").into())
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
