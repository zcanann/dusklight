use crate::{option, required_path};
use huntctl::harness::inspection::inspect_objective;
use huntctl::harness::objective_suite::ArtifactReference;
use huntctl::harness::run_contract::{HarnessRunRequest, HarnessRunResult};
use sha2::{Digest as _, Sha256};
use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub(super) fn inspect(args: &[String]) -> Result<(), Box<dyn Error>> {
    let command_args = &args[1..];
    let request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
    let result_path = option(command_args, "--result").map(PathBuf::from);
    let result: Option<HarnessRunResult> = result_path
        .as_ref()
        .map(|path| -> Result<_, Box<dyn Error>> { Ok(serde_json::from_slice(&fs::read(path)?)?) })
        .transpose()?;
    let artifact_root = option(command_args, "--artifact-root").map(PathBuf::from);
    if result.is_some() != artifact_root.is_some() {
        return Err(
            "harness inspect-objective requires --result and --artifact-root together".into(),
        );
    }
    let inspection = inspect_objective(
        &request,
        &repository_root(command_args)?,
        result.as_ref().zip(artifact_root.as_deref()),
    )?;
    print!("{inspection}");
    Ok(())
}

pub(super) fn validate_or_seal_result(args: &[String], seal: bool) -> Result<(), Box<dyn Error>> {
    let result_path = required_path(args, if seal { "--input" } else { "--result" })?;
    let output = seal.then(|| required_path(args, "--output")).transpose()?;
    if let Some(output) = &output {
        refuse_existing_output(output, "run-result")?;
    }
    let request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(required_path(args, "--request")?)?)?;
    request.validate_files(&repository_root(args)?)?;
    let artifact_root = required_path(args, "--artifact-root")?;
    let mut result: HarnessRunResult = serde_json::from_slice(&fs::read(&result_path)?)?;
    if seal {
        result.refresh_content_sha256()?;
    }
    let report = result.validate_files(&request, &artifact_root)?;
    if let Some(output) = output {
        write_new_file(&output, result.to_pretty_json()?)?;
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub(super) fn repository_root(args: &[String]) -> Result<PathBuf, Box<dyn Error>> {
    Ok(option(args, "--repository-root")
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?))
}

pub(super) fn repository_file(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let unresolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository_root.join(path)
    };
    let resolved = unresolved.canonicalize()?;
    if !resolved.starts_with(repository_root) || !resolved.is_file() {
        return Err(format!("{label} must be a file inside the repository").into());
    }
    Ok(resolved)
}

pub(super) fn repository_game_data(
    repository_root: &Path,
    path: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    let unresolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository_root.join(path)
    };
    let relative = unresolved
        .strip_prefix(repository_root)
        .map_err(|_| "game data must use a repository-relative path")?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("game data must use a canonical repository-relative path".into());
    }
    let entry = fs::symlink_metadata(&unresolved)?;
    let resolved = unresolved.canonicalize()?;
    if !resolved.is_file()
        || (!resolved.starts_with(repository_root)
            && (!entry.file_type().is_symlink()
                || repository_path_has_symlinked_parent(repository_root, relative)?))
    {
        return Err(
            "game data must be a repository file or a final repository-relative symlink".into(),
        );
    }
    Ok(unresolved)
}

pub(super) fn repository_path_has_symlinked_parent(
    repository_root: &Path,
    relative: &Path,
) -> Result<bool, Box<dyn Error>> {
    let mut current = repository_root.to_path_buf();
    let Some(parent) = relative.parent() else {
        return Ok(false);
    };
    for component in parent.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn repository_artifact(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<ArtifactReference, Box<dyn Error>> {
    let resolved = repository_file(repository_root, path, label)?;
    let bytes = fs::read(&resolved)?;
    let relative = resolved
        .strip_prefix(repository_root)?
        .to_str()
        .ok_or_else(|| format!("{label} path is not UTF-8"))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    Ok(ArtifactReference {
        path: relative,
        sha256: huntctl::Digest(Sha256::digest(bytes).into()),
    })
}

pub(super) fn learning_value_cell_draft_from_directory(
    repository_root: &Path,
    cell_root: &Path,
    checkpoint_id: String,
    deterministic_seed: u64,
    treatment: huntctl::search_evaluator::learning_value_comparison::LearningValueTreatmentKind,
) -> Result<
    huntctl::search_evaluator::learning_value_evidence::LearningValueCellDraft,
    Box<dyn Error>,
> {
    use huntctl::search_evaluator::learning_value_comparison::LearningValueTreatmentKind;
    use huntctl::search_evaluator::learning_value_evidence::{
        LEARNING_VALUE_CELL_DRAFT_SCHEMA_V1, LearningValueCellDraft, LearningValuePhaseSource,
    };

    if cell_root.is_absolute()
        || cell_root
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !cell_root.starts_with("build")
    {
        return Err(
            "learning-value cell root must be a repository-relative build/ directory".into(),
        );
    }
    let cell_root = repository_root.join(cell_root).canonicalize()?;
    if !cell_root.starts_with(repository_root) || !cell_root.is_dir() {
        return Err("learning-value cell root must resolve to a repository directory".into());
    }
    let artifact = |relative: &str, label: &str| {
        repository_artifact(repository_root, &cell_root.join(relative), label)
    };
    let learning = || -> Result<LearningValuePhaseSource, Box<dyn Error>> {
        Ok(LearningValuePhaseSource::StateReactive {
            loop_request: artifact("learning-loop/request.json", "learning-value loop request")?,
            optimization_request: artifact("request.json", "learning-value optimization request")?,
            execution_binding: artifact(
                "execution/execution.json",
                "learning-value execution binding",
            )?,
            loop_state: artifact("learning-loop/state.json", "learning-value loop state")?,
            checkpoint_report: artifact(
                "learning-loop/checkpoint-report.json",
                "learning-value checkpoint report",
            )?,
        })
    };
    let residual = |prefix: &str| -> Result<LearningValuePhaseSource, Box<dyn Error>> {
        let path = |suffix: &str| {
            if prefix.is_empty() {
                suffix.to_owned()
            } else {
                format!("{prefix}/{suffix}")
            }
        };
        Ok(LearningValuePhaseSource::Residual {
            optimization_request: artifact(
                &path("request.json"),
                "learning-value residual request",
            )?,
            execution_binding: artifact(
                &path("execution/execution.json"),
                "learning-value residual execution binding",
            )?,
            final_checkpoint: latest_residual_checkpoint(
                repository_root,
                &cell_root.join(path("checkpoints")),
            )?,
        })
    };
    let phases = match treatment {
        LearningValueTreatmentKind::IndependentRandomResidual
        | LearningValueTreatmentKind::CemResidual => vec![residual("")?],
        LearningValueTreatmentKind::DemonstrationAssistedStateReactive
        | LearningValueTreatmentKind::FromScratchStateReactive => vec![learning()?],
        LearningValueTreatmentKind::LearnedThenResidualRefinement => {
            vec![learning()?, residual("refinement")?]
        }
    };
    Ok(LearningValueCellDraft {
        schema: LEARNING_VALUE_CELL_DRAFT_SCHEMA_V1.into(),
        checkpoint_id,
        deterministic_seed,
        treatment,
        phases,
    })
}

pub(super) fn latest_residual_checkpoint(
    repository_root: &Path,
    checkpoint_root: &Path,
) -> Result<ArtifactReference, Box<dyn Error>> {
    let mut paths = fs::read_dir(checkpoint_root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("checkpoint-") && name.ends_with(".json"))
    });
    paths.sort();
    let path = paths
        .last()
        .ok_or("learning-value residual phase has no numbered campaign checkpoint")?;
    repository_artifact(
        repository_root,
        path,
        "learning-value residual final checkpoint",
    )
}

pub(super) fn repository_build_output(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !path.starts_with("build")
    {
        return Err(format!("{label} output must be a repository-relative build/ path").into());
    }
    Ok(repository_root.join(path))
}

pub(super) fn is_runtime_dependency_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    name.ends_with(".dll")
        || name.ends_with(".dylib")
        || name.ends_with(".so")
        || name.contains(".so.")
}

pub(super) fn refuse_existing_output(path: &Path, label: &str) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!("harness {label} output already exists: {}", path.display()).into());
    }
    Ok(())
}

pub(super) fn write_new_file(path: &Path, bytes: Vec<u8>) -> Result<(), Box<dyn Error>> {
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
    file.write_all(&bytes)?;
    file.flush()?;
    Ok(())
}

pub(super) fn pin_curriculum_source_checkpoint(
    repository_root: &Path,
    output: &Path,
    source: &ArtifactReference,
    bytes: &[u8],
) -> Result<ArtifactReference, Box<dyn Error>> {
    let output_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("expanded reverse curriculum output name is not UTF-8")?;
    let pinned = output.with_file_name(format!(
        "{output_name}.source-checkpoint-{}.json",
        source.sha256
    ));
    if pinned.exists() {
        let metadata = fs::symlink_metadata(&pinned)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || fs::read(&pinned)? != bytes {
            return Err(format!(
                "pinned reverse curriculum checkpoint differs: {}",
                pinned.display()
            )
            .into());
        }
    } else {
        write_new_file(&pinned, bytes.to_vec())?;
    }
    repository_artifact(
        repository_root,
        pinned.strip_prefix(repository_root)?,
        "pinned reverse curriculum source checkpoint",
    )
}
