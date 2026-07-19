use super::*;

#[derive(Clone)]
struct EditableSubgraph {
    id: String,
    name: String,
    parent: Option<String>,
    entry: String,
    exit: String,
    segments: BTreeSet<String>,
}

pub(super) fn create_subgraph(
    timeline_path: &Path,
    request: &BrowserSubgraphCreateRequest,
) -> Result<SubgraphEditResult, WorkbenchError> {
    let name = validate_subgraph_name(&request.name)?;
    edit_subgraphs(timeline_path, |timeline, groups| {
        if request.segments.is_empty() && request.subgraphs.is_empty() {
            return Err(WorkbenchError::new(
                "select at least one segment or subgraph",
            ));
        }
        if let Some(parent) = &request.parent
            && !groups.contains_key(parent)
        {
            return Err(WorkbenchError::new(format!(
                "unknown parent subgraph {parent:?}"
            )));
        }
        let mut direct_owners = BTreeMap::<String, String>::new();
        for group in groups.values() {
            for segment in &group.segments {
                direct_owners.insert(segment.clone(), group.id.clone());
            }
        }
        let expected_owner = request.parent.as_deref();
        let selected_segments = request.segments.iter().cloned().collect::<BTreeSet<_>>();
        let selected_groups = request.subgraphs.iter().cloned().collect::<BTreeSet<_>>();
        if selected_segments.len() != request.segments.len()
            || selected_groups.len() != request.subgraphs.len()
        {
            return Err(WorkbenchError::new("selection contains duplicate entries"));
        }
        for segment in &selected_segments {
            if !timeline.segments.contains_key(segment) {
                return Err(WorkbenchError::new(format!("unknown segment {segment:?}")));
            }
            if direct_owners.get(segment).map(String::as_str) != expected_owner {
                return Err(WorkbenchError::new(format!(
                    "segment {segment:?} is not directly inside the current graph"
                )));
            }
        }
        for id in &selected_groups {
            let group = groups
                .get(id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown subgraph {id:?}")))?;
            if group.parent.as_deref() != expected_owner {
                return Err(WorkbenchError::new(format!(
                    "subgraph {id:?} is not directly inside the current graph"
                )));
            }
        }
        let mut closure = selected_segments.clone();
        for id in &selected_groups {
            append_group_closure(groups, id, &mut closure);
        }
        let (entry, exit) = connected_region_boundaries(timeline, &closure)?;
        let id = unique_subgraph_id(&name, groups);
        for child in &selected_groups {
            groups.get_mut(child).expect("validated child").parent = Some(id.clone());
        }
        groups.insert(
            id.clone(),
            EditableSubgraph {
                id: id.clone(),
                name: name.clone(),
                parent: request.parent.clone(),
                entry,
                exit,
                segments: selected_segments,
            },
        );
        Ok(SubgraphEditResult {
            schema: SUBGRAPH_EDIT_RESULT_SCHEMA.into(),
            id,
            name,
            operation: "create".into(),
        })
    })
}

pub(super) fn rename_subgraph(
    timeline_path: &Path,
    request: &BrowserSubgraphRenameRequest,
) -> Result<SubgraphEditResult, WorkbenchError> {
    let name = validate_subgraph_name(&request.name)?;
    edit_subgraphs(timeline_path, |_timeline, groups| {
        let group = groups
            .get_mut(&request.id)
            .ok_or_else(|| WorkbenchError::new(format!("unknown subgraph {:?}", request.id)))?;
        if group.name != request.expected_name {
            return Err(WorkbenchError::new(
                "subgraph name changed; reload before renaming",
            ));
        }
        group.name = name.clone();
        Ok(SubgraphEditResult {
            schema: SUBGRAPH_EDIT_RESULT_SCHEMA.into(),
            id: request.id.clone(),
            name,
            operation: "rename".into(),
        })
    })
}

pub(super) fn ungroup_subgraph(
    timeline_path: &Path,
    request: &BrowserSubgraphUngroupRequest,
) -> Result<SubgraphEditResult, WorkbenchError> {
    edit_subgraphs(timeline_path, |_timeline, groups| {
        let removed = groups
            .remove(&request.id)
            .ok_or_else(|| WorkbenchError::new(format!("unknown subgraph {:?}", request.id)))?;
        if let Some(parent) = &removed.parent {
            groups
                .get_mut(parent)
                .expect("validated parent")
                .segments
                .extend(removed.segments);
        }
        for child in groups
            .values_mut()
            .filter(|group| group.parent.as_deref() == Some(&request.id))
        {
            child.parent = removed.parent.clone();
        }
        Ok(SubgraphEditResult {
            schema: SUBGRAPH_EDIT_RESULT_SCHEMA.into(),
            id: request.id.clone(),
            name: removed.name,
            operation: "ungroup".into(),
        })
    })
}

fn edit_subgraphs<T>(
    timeline_path: &Path,
    edit: impl FnOnce(&Timeline, &mut BTreeMap<String, EditableSubgraph>) -> Result<T, WorkbenchError>,
) -> Result<T, WorkbenchError> {
    let _edit = timeline_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("timeline edit lock is poisoned"))?;
    let path = validated_timeline_edit_path(timeline_path)?;
    let original = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!("cannot read timeline {}: {error}", path.display()))
    })?;
    let source = String::from_utf8(original.clone())
        .map_err(|_| WorkbenchError::new("timeline source is not UTF-8"))?;
    let timeline =
        Timeline::parse(&source).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let mut groups = timeline
        .subgraphs
        .values()
        .map(|group| {
            (
                group.id.clone(),
                EditableSubgraph {
                    id: group.id.clone(),
                    name: group.name.clone(),
                    parent: group.parent.clone(),
                    entry: group.entry_segment.clone(),
                    exit: group.exit_segment.clone(),
                    segments: group.segments.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let result = edit(&timeline, &mut groups)?;
    let replacement = render_subgraphs(&source, &groups)?;
    Timeline::parse(&replacement).map_err(|error| {
        WorkbenchError::new(format!(
            "subgraph edit produced an invalid timeline: {error}"
        ))
    })?;
    replace_timeline_atomically(&path, &original, replacement.as_bytes())?;
    Ok(result)
}

fn render_subgraphs(
    source: &str,
    groups: &BTreeMap<String, EditableSubgraph>,
) -> Result<String, WorkbenchError> {
    let mut lines = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let tokens =
            tokenize(line, index + 1).map_err(|error| WorkbenchError::new(error.to_string()))?;
        if !matches!(
            tokens.first().map(String::as_str),
            Some("subgraph" | "subgraph_label" | "subgraph_member")
        ) {
            lines.push(line);
        }
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let mut output = lines.join("\n");
    if !groups.is_empty() {
        output.push_str(
            "\n\n# Structural subgraphs. Playback remains the flattened segment chain.\n",
        );
        for group in groups.values() {
            match &group.parent {
                Some(parent) => output.push_str(&format!(
                    "subgraph {} inside {} entry {} exit {}\n",
                    group.id, parent, group.entry, group.exit
                )),
                None => output.push_str(&format!(
                    "subgraph {} root entry {} exit {}\n",
                    group.id, group.entry, group.exit
                )),
            }
            output.push_str(&format!("subgraph_label {} \"{}\"\n", group.id, group.name));
            for segment in &group.segments {
                output.push_str(&format!(
                    "subgraph_member {} segment {}\n",
                    group.id, segment
                ));
            }
        }
    }
    output.push('\n');
    Ok(output)
}

fn connected_region_boundaries(
    timeline: &Timeline,
    segments: &BTreeSet<String>,
) -> Result<(String, String), WorkbenchError> {
    let entries = segments
        .iter()
        .filter(|id| {
            timeline.segments[*id]
                .parent
                .as_ref()
                .is_none_or(|parent| !segments.contains(parent))
        })
        .cloned()
        .collect::<Vec<_>>();
    let exits = segments
        .iter()
        .filter(|id| {
            !timeline.segments.values().any(|segment| {
                segment.parent.as_deref() == Some(id) && segments.contains(&segment.id)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    if entries.len() != 1 || exits.len() != 1 {
        return Err(WorkbenchError::new(format!(
            "selection must be one connected, single-entry/single-exit region (found {} entries and {} exits)",
            entries.len(),
            exits.len()
        )));
    }
    for id in segments {
        if id != &exits[0]
            && timeline
                .segments
                .values()
                .any(|child| child.parent.as_deref() == Some(id) && !segments.contains(&child.id))
        {
            return Err(WorkbenchError::new(format!(
                "selection leaves the graph early from segment {id:?}"
            )));
        }
    }
    Ok((entries[0].clone(), exits[0].clone()))
}

fn append_group_closure(
    groups: &BTreeMap<String, EditableSubgraph>,
    id: &str,
    output: &mut BTreeSet<String>,
) {
    let Some(group) = groups.get(id) else { return };
    output.extend(group.segments.iter().cloned());
    for child in groups
        .values()
        .filter(|child| child.parent.as_deref() == Some(id))
    {
        append_group_closure(groups, &child.id, output);
    }
}

fn unique_subgraph_id(name: &str, groups: &BTreeMap<String, EditableSubgraph>) -> String {
    let base = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    let base = if base.is_empty() { "subgraph" } else { &base };
    let mut candidate = base.to_string();
    let mut suffix = 2;
    while groups.contains_key(&candidate) {
        candidate = format!("{base}_{suffix}");
        suffix += 1;
    }
    candidate
}

fn validate_subgraph_name(name: &str) -> Result<String, WorkbenchError> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 160
        || name
            .chars()
            .any(|character| character.is_control() || matches!(character, '"' | '\\'))
    {
        return Err(WorkbenchError::new(
            "subgraph name must be 1 to 160 characters without controls, quotes, or backslashes",
        ));
    }
    Ok(name.into())
}

fn replace_timeline_atomically(
    path: &Path,
    expected: &[u8],
    replacement: &[u8],
) -> Result<(), WorkbenchError> {
    if fs::read(path).ok().as_deref() != Some(expected) {
        return Err(WorkbenchError::new(
            "timeline changed while preparing edit; reload and retry",
        ));
    }
    let directory = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("timeline has no parent"))?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorkbenchError::new("timeline filename is not UTF-8"))?;
    let nonce = random_session_token()?;
    let temporary = directory.join(format!(".{filename}.{nonce}.tmp"));
    let backup = directory.join(format!(".{filename}.{nonce}.rollback"));
    let mut temporary_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| WorkbenchError::new(format!("cannot create timeline edit: {error}")))?;
    let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    temporary_file
        .write_all(replacement)
        .and_then(|()| temporary_file.sync_all())
        .map_err(|error| WorkbenchError::new(format!("cannot flush timeline edit: {error}")))?;
    drop(temporary_file);
    fs::rename(path, &backup)
        .map_err(|error| WorkbenchError::new(format!("cannot stage timeline backup: {error}")))?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::rename(&backup, path);
        let _ = fs::remove_file(&temporary);
        return Err(WorkbenchError::new(format!(
            "cannot install timeline edit: {error}"
        )));
    }
    temporary_cleanup.0 = None;
    let _ = fs::remove_file(backup);
    Ok(())
}
