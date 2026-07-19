use super::*;

pub(super) fn configured_artifact_root(
    config: &WorkbenchConfig,
) -> Result<PathBuf, WorkbenchError> {
    let repository = fs::canonicalize(&config.repository_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve repository root {}: {error}",
            config.repository_root.display()
        ))
    })?;
    let timeline = fs::canonicalize(&config.timeline_path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve timeline {}: {error}",
            config.timeline_path.display()
        ))
    })?;
    if !timeline.starts_with(&repository) {
        return Err(WorkbenchError::new(format!(
            "timeline {} is outside repository {}",
            timeline.display(),
            repository.display()
        )));
    }
    timeline
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| WorkbenchError::new("timeline has no parent directory"))
}

/// Parse the checked-in timeline on every request so edits in the Git working
/// tree are visible without synchronizing a secondary store.
pub fn load_authoritative_timeline(path: &Path) -> Result<Timeline, WorkbenchError> {
    let source = fs::read_to_string(path)
        .map_err(|error| WorkbenchError::new(format!("cannot read {}: {error}", path.display())))?;
    Timeline::parse(&source).map_err(|error| WorkbenchError::new(error.to_string()))
}

pub(super) fn source_revision(source: &[u8]) -> String {
    format!("{:x}", Sha256::digest(source))
}

pub(super) fn validate_milestone_program_source(
    timeline: &Timeline,
    source: &str,
) -> Result<(MilestoneProgram, milestone_dsl::CompiledMilestones), WorkbenchError> {
    let program = milestone_dsl::parse(source)
        .map_err(|error| WorkbenchError::new(format!("invalid milestone program: {error}")))?;
    let authored = program
        .definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut referenced = timeline
        .goals
        .values()
        .map(|goal| goal.predicate.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(origin) = &timeline.origin {
        referenced.insert(origin.predicate.as_str());
    }
    if let Some(missing) = referenced.difference(&authored).next() {
        return Err(WorkbenchError::new(format!(
            "timeline references predicate {missing:?}, but the predicate program does not define it"
        )));
    }
    let compiled = milestone_dsl::compile(&program).map_err(|error| {
        WorkbenchError::new(format!("cannot compile milestone program: {error}"))
    })?;
    Ok((program, compiled))
}

pub(super) fn validated_milestone_program_path(
    timeline: &Timeline,
    root: &Path,
) -> Result<Option<PathBuf>, WorkbenchError> {
    let Some(relative) = &timeline.predicate_program else {
        return Ok(None);
    };
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(WorkbenchError::new(
            "configured milestone program is not a contained relative path",
        ));
    }
    let root = fs::canonicalize(root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve milestone program root {}: {error}",
            root.display()
        ))
    })?;
    let mut candidate = root.clone();
    for component in relative.components() {
        candidate.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot inspect configured milestone program path {}: {error}",
                candidate.display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(WorkbenchError::new(format!(
                "configured milestone program path {} contains a symbolic link",
                candidate.display()
            )));
        }
    }
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot inspect configured milestone program {}: {error}",
            candidate.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(WorkbenchError::new(format!(
            "configured milestone program {} is not a regular file",
            candidate.display()
        )));
    }
    let resolved = fs::canonicalize(&candidate).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve configured milestone program {}: {error}",
            candidate.display()
        ))
    })?;
    if !resolved.starts_with(&root) {
        return Err(WorkbenchError::new(format!(
            "configured milestone program {} escapes root {}",
            resolved.display(),
            root.display()
        )));
    }
    Ok(Some(resolved))
}

pub(super) fn milestone_program_projection(
    timeline: &Timeline,
    root: &Path,
) -> Result<Option<GraphPredicateProgram>, WorkbenchError> {
    let Some(path) = validated_milestone_program_path(timeline, root)? else {
        return Ok(None);
    };
    let bytes = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read configured milestone program {}: {error}",
            path.display()
        ))
    })?;
    let source = String::from_utf8(bytes.clone()).map_err(|_| {
        WorkbenchError::new(format!(
            "configured milestone program {} is not UTF-8",
            path.display()
        ))
    })?;
    let (program, compiled) = validate_milestone_program_source(timeline, &source)?;
    let definition_digests = compiled
        .definitions
        .into_iter()
        .map(|definition| {
            let digest = definition
                .sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            (definition.name, digest)
        })
        .collect::<BTreeMap<_, _>>();
    let program_sha256 = compiled
        .program_sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(Some(GraphPredicateProgram {
        schema: MILESTONE_PROGRAM_SCHEMA.into(),
        source,
        revision_sha256: source_revision(&bytes),
        program_sha256,
        definitions: program
            .definitions
            .into_iter()
            .map(|definition| GraphPredicate {
                definition_sha256: definition_digests[&definition.name].clone(),
                name: definition.name,
                phase: definition.phase,
                stable_ticks: definition.stable_ticks,
                expression: definition.when,
                then: definition.then,
                within_ticks: definition.within_ticks,
            })
            .collect(),
    }))
}

pub(super) fn is_exact_boot_boundary_predicate(definition: &GraphPredicate) -> bool {
    use milestone_dsl::{Comparison, EvaluationPhase, Expression, Field, Value};

    fn collect<'a>(
        expression: &'a Expression,
        leaves: &mut Vec<(Field, Comparison, &'a Value)>,
    ) -> bool {
        match expression {
            Expression::And(left, right) => collect(left, leaves) && collect(right, leaves),
            Expression::Compare {
                field,
                operator,
                value,
            } => {
                leaves.push((*field, *operator, value));
                true
            }
            Expression::Not(_) | Expression::Or(_, _) | Expression::Query { .. } => false,
        }
    }

    if definition.phase != EvaluationPhase::PreInput
        || definition.stable_ticks != 1
        || !definition.then.is_empty()
        || definition.within_ticks.is_some()
    {
        return false;
    }
    let mut leaves = Vec::new();
    if !collect(&definition.expression, &mut leaves) || leaves.len() != 2 {
        return false;
    }
    let boot_kind = leaves.iter().any(|(field, operator, value)| {
        *field == Field::BoundaryKind
            && *operator == Comparison::Equal
            && matches!(value, Value::Symbol(symbol) if symbol == "boot")
    });
    let boundary_zero = leaves.iter().any(|(field, operator, value)| {
        *field == Field::BoundaryIndex
            && *operator == Comparison::Equal
            && matches!(value, Value::U64(0))
    });
    boot_kind && boundary_zero
}

#[derive(Debug)]
pub(super) enum MilestoneProgramUpdateError {
    Stale { expected: String, actual: String },
    Invalid(WorkbenchError),
}

impl fmt::Display for MilestoneProgramUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale { expected, actual } => write!(
                formatter,
                "milestone program edit is stale: expected revision {expected}, current revision is {actual}"
            ),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl From<WorkbenchError> for MilestoneProgramUpdateError {
    fn from(error: WorkbenchError) -> Self {
        Self::Invalid(error)
    }
}

pub(super) struct RemoveFileOnDrop(pub(super) Option<PathBuf>);

impl Drop for RemoveFileOnDrop {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

pub(super) fn rollback_milestone_program(
    backup: &Path,
    target: &Path,
) -> Result<(), WorkbenchError> {
    if target.exists() {
        return Err(WorkbenchError::new(format!(
            "cannot restore milestone program backup {} because {} now exists",
            backup.display(),
            target.display()
        )));
    }
    fs::rename(backup, target).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot restore milestone program backup {} to {}: {error}",
            backup.display(),
            target.display()
        ))
    })
}

pub(super) fn update_milestone_program(
    timeline: &Timeline,
    root: &Path,
    request: &BrowserMilestoneProgramUpdateRequest,
) -> Result<GraphPredicateProgram, MilestoneProgramUpdateError> {
    let _edit = milestone_program_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("milestone program edit lock is poisoned"))?;
    let path = validated_milestone_program_path(timeline, root)?
        .ok_or_else(|| WorkbenchError::new("timeline has no configured milestone program"))?;
    let current = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read configured milestone program {}: {error}",
            path.display()
        ))
    })?;
    let current_revision = source_revision(&current);
    if request.expected_revision_sha256 != current_revision {
        return Err(MilestoneProgramUpdateError::Stale {
            expected: request.expected_revision_sha256.clone(),
            actual: current_revision,
        });
    }
    validate_milestone_program_source(timeline, &request.source)?;

    let parent = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("milestone program has no parent directory"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorkbenchError::new("milestone program filename is not UTF-8"))?;
    let nonce = random_session_token()?;
    let temporary = parent.join(format!(".{name}.{nonce}.tmp"));
    let backup = parent.join(format!(".{name}.{nonce}.rollback"));
    let mut temporary_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create adjacent milestone program temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    if let Err(error) = temporary_file
        .write_all(request.source.as_bytes())
        .and_then(|()| temporary_file.sync_all())
    {
        return Err(WorkbenchError::new(format!(
            "cannot flush milestone program temporary file {}: {error}",
            temporary.display()
        ))
        .into());
    }
    drop(temporary_file);

    let revalidated = validated_milestone_program_path(timeline, root)?
        .ok_or_else(|| WorkbenchError::new("timeline lost its configured milestone program"))?;
    if revalidated != path {
        return Err(WorkbenchError::new(
            "configured milestone program path changed while preparing the edit",
        )
        .into());
    }
    let before_replace = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot re-read configured milestone program {}: {error}",
            path.display()
        ))
    })?;
    let before_replace_revision = source_revision(&before_replace);
    if request.expected_revision_sha256 != before_replace_revision {
        return Err(MilestoneProgramUpdateError::Stale {
            expected: request.expected_revision_sha256.clone(),
            actual: before_replace_revision,
        });
    }

    if let Err(error) = fs::rename(&path, &backup) {
        return Err(WorkbenchError::new(format!(
            "cannot stage milestone program rollback backup {}: {error}",
            backup.display()
        ))
        .into());
    }
    let moved_revision = fs::symlink_metadata(&backup)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot inspect milestone program rollback backup {}: {error}",
                backup.display()
            ))
        })
        .and_then(|metadata| {
            if metadata.is_file() && !metadata.file_type().is_symlink() {
                Ok(())
            } else {
                Err(WorkbenchError::new(format!(
                    "milestone program changed to a non-regular file during replacement: {}",
                    backup.display()
                )))
            }
        })
        .and_then(|()| fs::read(&backup).map_err(|error| WorkbenchError::new(error.to_string())))
        .map(|bytes| source_revision(&bytes))
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot verify milestone program rollback backup {}: {error}",
                backup.display()
            ))
        });
    match moved_revision {
        Ok(actual) if actual == request.expected_revision_sha256 => {}
        result => {
            let rollback = rollback_milestone_program(&backup, &path);
            rollback?;
            return match result {
                Ok(actual) => Err(MilestoneProgramUpdateError::Stale {
                    expected: request.expected_revision_sha256.clone(),
                    actual,
                }),
                Err(error) => Err(error.into()),
            };
        }
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        let rollback = rollback_milestone_program(&backup, &path);
        rollback?;
        return Err(WorkbenchError::new(format!(
            "cannot replace milestone program {}: {error}",
            path.display()
        ))
        .into());
    }
    temporary_cleanup.0 = None;
    let _ = fs::remove_file(&backup);

    milestone_program_projection(timeline, root)?
        .ok_or_else(|| WorkbenchError::new("timeline lost its configured milestone program"))
        .map_err(Into::into)
}
