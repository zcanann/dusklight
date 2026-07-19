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
    validated_predicate_source_path(relative, root).map(Some)
}

pub(super) fn validated_predicate_source_path(
    relative: &Path,
    root: &Path,
) -> Result<PathBuf, WorkbenchError> {
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
    Ok(resolved)
}

fn owned_predicate_program_projection(
    root: &Path,
    relative: &Path,
    expected: &str,
    local: bool,
) -> Result<GraphPredicateProgram, WorkbenchError> {
    let path = validated_predicate_source_path(relative, root)?;
    let bytes = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read configured predicate source {}: {error}",
            path.display()
        ))
    })?;
    let source = String::from_utf8(bytes.clone()).map_err(|_| {
        WorkbenchError::new(format!(
            "configured predicate source {} is not UTF-8",
            path.display()
        ))
    })?;
    let program = milestone_dsl::parse(&source)
        .map_err(|error| WorkbenchError::new(format!("invalid milestone program: {error}")))?;
    if local && (program.definitions.len() != 1 || program.definitions[0].name != expected) {
        return Err(WorkbenchError::new(format!(
            "predicate source {} must define exactly its owned predicate {expected:?}",
            path.display()
        )));
    }
    if !program
        .definitions
        .iter()
        .any(|definition| definition.name == expected)
    {
        return Err(WorkbenchError::new(format!(
            "predicate source {} does not define {expected:?}",
            path.display()
        )));
    }
    let compiled = milestone_dsl::compile(&program).map_err(|error| {
        WorkbenchError::new(format!("cannot compile milestone program: {error}"))
    })?;
    graph_predicate_program(source, bytes, program, compiled)
}

pub(super) fn origin_predicate_program_projection(
    timeline: &Timeline,
    root: &Path,
) -> Result<Option<GraphPredicateProgram>, WorkbenchError> {
    let Some(origin) = &timeline.origin else {
        return Ok(None);
    };
    let relative = timeline
        .origin_predicate_source()
        .ok_or_else(|| WorkbenchError::new("origin has no predicate source"))?;
    owned_predicate_program_projection(
        root,
        relative,
        &origin.predicate,
        origin.predicate_source.is_some(),
    )
    .map(Some)
}

pub(super) fn goal_predicate_program_projection(
    timeline: &Timeline,
    root: &Path,
    goal_id: &str,
) -> Result<GraphPredicateProgram, WorkbenchError> {
    let goal = timeline
        .goals
        .get(goal_id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown goal {goal_id:?}")))?;
    let relative = timeline
        .goal_predicate_source(goal_id)
        .ok_or_else(|| WorkbenchError::new(format!("goal {goal_id:?} has no predicate source")))?;
    owned_predicate_program_projection(
        root,
        relative,
        &goal.predicate,
        goal.predicate_source.is_some(),
    )
}

fn graph_predicate_program(
    source: String,
    bytes: Vec<u8>,
    program: MilestoneProgram,
    compiled: milestone_dsl::CompiledMilestones,
) -> Result<GraphPredicateProgram, WorkbenchError> {
    let definition_digests = compiled
        .definitions
        .iter()
        .map(|definition| {
            let digest = definition
                .sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            (definition.name.clone(), digest)
        })
        .collect::<BTreeMap<_, _>>();
    let program_sha256 = compiled
        .program_sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(GraphPredicateProgram {
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
    })
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
    let (relative, expected, local) = if request.owner.is_empty() {
        (
            timeline
                .predicate_program
                .as_deref()
                .ok_or_else(|| WorkbenchError::new("timeline has no legacy predicate program"))?,
            None,
            false,
        )
    } else if request.owner == "origin:boot" {
        let origin = timeline
            .origin
            .as_ref()
            .ok_or_else(|| WorkbenchError::new("timeline has no Boot origin"))?;
        (
            timeline
                .origin_predicate_source()
                .ok_or_else(|| WorkbenchError::new("Boot origin has no predicate source"))?,
            Some(origin.predicate.as_str()),
            origin.predicate_source.is_some(),
        )
    } else {
        let goal = timeline.goals.get(&request.owner).ok_or_else(|| {
            WorkbenchError::new(format!("unknown predicate owner {:?}", request.owner))
        })?;
        (
            timeline
                .goal_predicate_source(&goal.id)
                .ok_or_else(|| WorkbenchError::new("goal has no predicate source"))?,
            Some(goal.predicate.as_str()),
            goal.predicate_source.is_some(),
        )
    };
    let path = validated_predicate_source_path(relative, root)?;
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
    if let Some(expected) = expected {
        let program = milestone_dsl::parse(&request.source)
            .map_err(|error| WorkbenchError::new(format!("invalid milestone program: {error}")))?;
        if local && (program.definitions.len() != 1 || program.definitions[0].name != expected) {
            return Err(WorkbenchError::new(format!(
                "owned predicate source must define exactly {expected:?}"
            ))
            .into());
        }
        if !program
            .definitions
            .iter()
            .any(|definition| definition.name == expected)
        {
            return Err(WorkbenchError::new(format!(
                "predicate source does not define {expected:?}"
            ))
            .into());
        }
        milestone_dsl::compile(&program).map_err(|error| {
            WorkbenchError::new(format!("cannot compile milestone program: {error}"))
        })?;
    } else {
        validate_milestone_program_source(timeline, &request.source)?;
    }

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

    let revalidated = validated_predicate_source_path(relative, root)?;
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

    if request.owner.is_empty() {
        milestone_program_projection(timeline, root)?
            .ok_or_else(|| WorkbenchError::new("timeline lost its legacy predicate program"))
            .map_err(Into::into)
    } else if request.owner == "origin:boot" {
        origin_predicate_program_projection(timeline, root)?
            .ok_or_else(|| WorkbenchError::new("timeline lost its Boot predicate source"))
            .map_err(Into::into)
    } else {
        goal_predicate_program_projection(timeline, root, &request.owner).map_err(Into::into)
    }
}

fn validate_new_predicate_identifier(value: &str, kind: &str) -> Result<(), WorkbenchError> {
    if value.is_empty()
        || value.len() > 96
        || !value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
        })
        || !value.as_bytes()[0].is_ascii_lowercase()
    {
        return Err(WorkbenchError::new(format!(
            "{kind} must start with a lowercase letter and contain only lowercase letters, digits, and underscores (96 characters maximum)"
        )));
    }
    Ok(())
}

fn new_goal_source_relative_path(
    timeline_path: &Path,
    predicate: &str,
) -> Result<PathBuf, WorkbenchError> {
    let stem = timeline_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| WorkbenchError::new("timeline filename is not UTF-8"))?;
    if stem.is_empty()
        || !stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(WorkbenchError::new(
            "timeline filename must use letters, digits, underscores, or hyphens to create owned predicates",
        ));
    }
    Ok(PathBuf::from(stem)
        .join("predicates")
        .join(format!("{predicate}.milestones")))
}

fn prepare_owned_predicate_parent(root: &Path, relative: &Path) -> Result<PathBuf, WorkbenchError> {
    let root = fs::canonicalize(root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve predicate source root {}: {error}",
            root.display()
        ))
    })?;
    let parent = relative
        .parent()
        .ok_or_else(|| WorkbenchError::new("owned predicate source has no parent"))?;
    let mut candidate = root.clone();
    for component in parent.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(WorkbenchError::new(
                "owned predicate source is not a contained relative path",
            ));
        };
        candidate.push(component);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(WorkbenchError::new(format!(
                    "owned predicate directory {} is a symbolic link",
                    candidate.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(WorkbenchError::new(format!(
                    "owned predicate directory {} is not a directory",
                    candidate.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&candidate).map_err(|error| {
                    WorkbenchError::new(format!(
                        "cannot create owned predicate directory {}: {error}",
                        candidate.display()
                    ))
                })?;
            }
            Err(error) => {
                return Err(WorkbenchError::new(format!(
                    "cannot inspect owned predicate directory {}: {error}",
                    candidate.display()
                )));
            }
        }
    }
    let resolved = fs::canonicalize(&candidate).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve owned predicate directory {}: {error}",
            candidate.display()
        ))
    })?;
    if !resolved.starts_with(root) {
        return Err(WorkbenchError::new(
            "owned predicate directory escapes the timeline root",
        ));
    }
    Ok(resolved)
}

fn append_goal_declaration(
    source: &str,
    goal: &str,
    segment: &str,
    predicate: &str,
    relative: &Path,
) -> Result<String, WorkbenchError> {
    let relative = relative
        .iter()
        .map(|component| component.to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| WorkbenchError::new("owned predicate path is not UTF-8"))?
        .join("/");
    let mut replacement = source.to_owned();
    if !replacement.ends_with('\n') {
        replacement.push('\n');
    }
    replacement.push_str(&format!(
        "goal {goal} on {segment} predicate {predicate} source {relative}\n"
    ));
    Ok(replacement)
}

/// Create the first goal for a segment as two Git-visible artifacts: one goal
/// declaration in the authoritative timeline and one source file owned only by
/// that goal. No proof is synthesized; a proof must still come from observation.
pub(super) fn create_goal_predicate(
    timeline_path: &Path,
    root: &Path,
    request: &BrowserGoalPredicateCreateRequest,
) -> Result<GraphPredicateProgram, MilestoneProgramUpdateError> {
    let segment_id = request.create_on_segment.as_str();
    let predicate = request.predicate.as_str();
    if !request.expected_revision_sha256.is_empty() {
        return Err(WorkbenchError::new(
            "new goal edits must begin from the explicit empty revision",
        )
        .into());
    }
    validate_new_predicate_identifier(&request.owner, "goal ID")?;
    validate_new_predicate_identifier(predicate, "predicate name")?;

    let program = milestone_dsl::parse(&request.source)
        .map_err(|error| WorkbenchError::new(format!("invalid milestone program: {error}")))?;
    if program.definitions.len() != 1 || program.definitions[0].name != predicate {
        return Err(WorkbenchError::new(format!(
            "owned predicate source must define exactly {predicate:?}"
        ))
        .into());
    }
    milestone_dsl::compile(&program).map_err(|error| {
        WorkbenchError::new(format!("cannot compile milestone program: {error}"))
    })?;

    let _predicate_edit = milestone_program_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("milestone program edit lock is poisoned"))?;
    let _timeline_edit = timeline_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("timeline edit lock is poisoned"))?;
    let timeline_path = fs::canonicalize(timeline_path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve timeline {}: {error}",
            timeline_path.display()
        ))
    })?;
    let root = fs::canonicalize(root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve timeline root {}: {error}",
            root.display()
        ))
    })?;
    if timeline_path.parent() != Some(root.as_path()) {
        return Err(WorkbenchError::new("timeline is outside its configured artifact root").into());
    }
    let original = fs::read(&timeline_path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read timeline {}: {error}",
            timeline_path.display()
        ))
    })?;
    let timeline_source = String::from_utf8(original.clone())
        .map_err(|_| WorkbenchError::new("timeline source is not UTF-8"))?;
    let timeline = Timeline::parse(&timeline_source)
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    if !timeline.segments.contains_key(segment_id) {
        return Err(WorkbenchError::new(format!("unknown segment {segment_id:?}")).into());
    }
    if timeline
        .goals
        .values()
        .any(|goal| goal.segment == segment_id)
    {
        return Err(WorkbenchError::new(
            "segment already has an authored goal; reload and edit that predicate",
        )
        .into());
    }
    if timeline.goals.contains_key(&request.owner) {
        return Err(WorkbenchError::new(format!("goal {:?} already exists", request.owner)).into());
    }
    if timeline
        .goals
        .values()
        .any(|goal| goal.predicate == predicate)
        || timeline
            .origin
            .as_ref()
            .is_some_and(|origin| origin.predicate == predicate)
    {
        return Err(WorkbenchError::new(format!(
            "predicate {predicate:?} is already owned by another route node"
        ))
        .into());
    }

    let relative = new_goal_source_relative_path(&timeline_path, predicate)?;
    let replacement = append_goal_declaration(
        &timeline_source,
        &request.owner,
        segment_id,
        predicate,
        &relative,
    )?;
    let replacement_timeline =
        Timeline::parse(&replacement).map_err(|error| WorkbenchError::new(error.to_string()))?;
    replacement_timeline
        .inspect()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;

    let parent = prepare_owned_predicate_parent(&root, &relative)?;
    let filename = relative
        .file_name()
        .ok_or_else(|| WorkbenchError::new("owned predicate source has no filename"))?;
    let target = parent.join(filename);
    if target.exists() {
        return Err(WorkbenchError::new(format!(
            "owned predicate source {} already exists",
            target.display()
        ))
        .into());
    }
    let nonce = random_session_token()?;
    let temporary = parent.join(format!(".{predicate}.{nonce}.tmp"));
    let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create owned predicate temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    file.write_all(request.source.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot flush owned predicate source {}: {error}",
                temporary.display()
            ))
        })?;
    drop(file);
    fs::rename(&temporary, &target).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot install owned predicate source {}: {error}",
            target.display()
        ))
    })?;
    temporary_cleanup.0 = None;

    let timeline_result = (|| {
        if fs::read(&timeline_path).ok().as_deref() != Some(original.as_slice()) {
            return Err(WorkbenchError::new(
                "timeline changed while preparing predicate creation; reload and retry",
            ));
        }
        let filename = timeline_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| WorkbenchError::new("timeline filename is not UTF-8"))?;
        let temporary = root.join(format!(".{filename}.{nonce}.tmp"));
        let backup = root.join(format!(".{filename}.{nonce}.rollback"));
        let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| {
                WorkbenchError::new(format!("cannot create timeline edit: {error}"))
            })?;
        file.write_all(replacement.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| WorkbenchError::new(format!("cannot flush timeline edit: {error}")))?;
        drop(file);
        fs::rename(&timeline_path, &backup).map_err(|error| {
            WorkbenchError::new(format!("cannot stage timeline backup: {error}"))
        })?;
        if let Err(error) = fs::rename(&temporary, &timeline_path) {
            let _ = fs::rename(&backup, &timeline_path);
            return Err(WorkbenchError::new(format!(
                "cannot install timeline edit: {error}"
            )));
        }
        temporary_cleanup.0 = None;
        let _ = fs::remove_file(backup);
        Ok(())
    })();
    if let Err(error) = timeline_result {
        let _ = fs::remove_file(&target);
        return Err(error.into());
    }

    goal_predicate_program_projection(&replacement_timeline, &root, &request.owner)
        .map_err(Into::into)
}
