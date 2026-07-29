use super::*;

pub(super) struct Parser<'a> {
    source: &'a str,
    timeline_name: Option<String>,
    predicate_program: Option<PathBuf>,
    origin: Option<Origin>,
    segments: BTreeMap<String, Segment>,
    segment_labels: BTreeMap<String, (String, usize)>,
    subgraphs: BTreeMap<String, Subgraph>,
    subgraph_labels: BTreeMap<String, (String, usize)>,
    subgraph_members: Vec<(String, String, usize)>,
    goals: BTreeMap<String, Goal>,
    proofs: Vec<GoalProof>,
    continuations: BTreeMap<String, Continuation>,
    branches: BTreeMap<String, Branch>,
}

impl<'a> Parser<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        Self {
            source,
            timeline_name: None,
            predicate_program: None,
            origin: None,
            segments: BTreeMap::new(),
            segment_labels: BTreeMap::new(),
            subgraphs: BTreeMap::new(),
            subgraph_labels: BTreeMap::new(),
            subgraph_members: Vec::new(),
            goals: BTreeMap::new(),
            proofs: Vec::new(),
            continuations: BTreeMap::new(),
            branches: BTreeMap::new(),
        }
    }

    pub(super) fn parse(mut self) -> Result<Timeline, TimelineError> {
        for (index, raw_line) in self.source.lines().enumerate() {
            let line_number = index + 1;
            let tokens = tokenize(raw_line, line_number)?;
            if tokens.is_empty() {
                continue;
            }
            match tokens[0].as_str() {
                "timeline" => self.parse_timeline(&tokens, line_number)?,
                "predicate_program" => self.parse_predicate_program(&tokens, line_number)?,
                "origin" => self.parse_origin(&tokens, line_number)?,
                "segment" => self.parse_segment(&tokens, line_number)?,
                "label" => self.parse_segment_label(&tokens, line_number)?,
                "subgraph" => self.parse_subgraph(&tokens, line_number)?,
                "subgraph_label" => self.parse_subgraph_label(&tokens, line_number)?,
                "subgraph_member" => self.parse_subgraph_member(&tokens, line_number)?,
                "goal" => self.parse_goal(&tokens, line_number)?,
                "proof" => self.parse_proof(&tokens, line_number)?,
                "continuation" => self.parse_continuation(&tokens, line_number)?,
                "branch" => self.parse_branch(&tokens, line_number)?,
                "continue" => self.parse_continue(&tokens, line_number)?,
                other => {
                    return Err(TimelineError::at(
                        line_number,
                        1,
                        format!("unknown timeline statement {other:?}"),
                    ));
                }
            }
        }
        for (id, (label, line)) in self.segment_labels {
            let segment = self.segments.get_mut(&id).ok_or_else(|| {
                TimelineError::at(line, 1, format!("label references unknown segment {id}"))
            })?;
            segment.name = Some(label);
        }
        for (id, (label, line)) in self.subgraph_labels {
            let subgraph = self.subgraphs.get_mut(&id).ok_or_else(|| {
                TimelineError::at(
                    line,
                    1,
                    format!("subgraph_label references unknown subgraph {id}"),
                )
            })?;
            subgraph.name = label;
        }
        for (id, segment, line) in self.subgraph_members {
            let subgraph = self.subgraphs.get_mut(&id).ok_or_else(|| {
                TimelineError::at(
                    line,
                    1,
                    format!("subgraph_member references unknown subgraph {id}"),
                )
            })?;
            if !subgraph.segments.insert(segment.clone()) {
                return Err(TimelineError::at(
                    line,
                    1,
                    format!("duplicate membership of segment {segment} in subgraph {id}"),
                ));
            }
        }
        let timeline = Timeline {
            name: self
                .timeline_name
                .ok_or_else(|| TimelineError::new("missing timeline declaration"))?,
            predicate_program: self.predicate_program,
            origin: self.origin,
            segments: self.segments,
            subgraphs: self.subgraphs,
            goals: self.goals,
            proofs: self.proofs,
            continuations: self.continuations,
            branches: self.branches,
        };
        timeline.validate_structure()?;
        Ok(timeline)
    }

    fn parse_timeline(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        exact_len(tokens, 2, line, "timeline NAME")?;
        if self.timeline_name.replace(tokens[1].clone()).is_some() {
            return Err(TimelineError::at(line, 1, "duplicate timeline declaration"));
        }
        Ok(())
    }

    fn parse_predicate_program(
        &mut self,
        tokens: &[String],
        line: usize,
    ) -> Result<(), TimelineError> {
        exact_len(tokens, 2, line, "predicate_program PATH")?;
        let path = parse_contained_relative_path(&tokens[1], line, "predicate program")?;
        if self.predicate_program.replace(path).is_some() {
            return Err(TimelineError::at(
                line,
                1,
                "duplicate predicate_program declaration",
            ));
        }
        Ok(())
    }

    fn parse_origin(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        if tokens.len() != 4 && tokens.len() != 6 && tokens.len() != 8 {
            return Err(TimelineError::at(
                line,
                1,
                "expected origin boot predicate PREDICATE [source PATH [card_fixture PATH]]",
            ));
        }
        if tokens[1] != "boot" {
            return Err(TimelineError::at(
                line,
                1,
                "the only supported origin is boot",
            ));
        }
        expect(tokens, 2, "predicate", line)?;
        let predicate_source = if tokens.len() >= 6 {
            expect(tokens, 4, "source", line)?;
            Some(parse_contained_relative_path(
                &tokens[5],
                line,
                "predicate source",
            )?)
        } else {
            None
        };
        let card_fixture = if tokens.len() == 8 {
            expect(tokens, 6, "card_fixture", line)?;
            Some(parse_contained_relative_path(
                &tokens[7],
                line,
                "card fixture",
            )?)
        } else {
            None
        };
        let origin = Origin {
            id: tokens[1].clone(),
            predicate: tokens[3].clone(),
            predicate_source,
            card_fixture,
            line,
        };
        if self.origin.replace(origin).is_some() {
            return Err(TimelineError::at(line, 1, "duplicate origin declaration"));
        }
        Ok(())
    }

    fn parse_segment(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        let (parent, cursor) = match tokens.get(2).map(String::as_str) {
            Some("root") => (None, 3),
            Some("after") => (Some(required_token(tokens, 3, line, "parent segment")?), 4),
            _ => {
                return Err(TimelineError::at(
                    line,
                    1,
                    "expected segment ID root profile PROFILE uses KIND VALUE starts FINGERPRINT produces FINGERPRINT or segment ID after PARENT_SEGMENT profile PROFILE uses KIND VALUE starts FINGERPRINT produces FINGERPRINT",
                ));
            }
        };
        exact_len(
            tokens,
            cursor + 9,
            line,
            "segment ID (root | after PARENT_SEGMENT) profile PROFILE uses KIND VALUE starts FINGERPRINT produces FINGERPRINT",
        )?;
        expect(tokens, cursor, "profile", line)?;
        expect(tokens, cursor + 2, "uses", line)?;
        expect(tokens, cursor + 5, "starts", line)?;
        expect(tokens, cursor + 7, "produces", line)?;
        let id = tokens[1].clone();
        let profile = tokens[cursor + 1]
            .parse()
            .map_err(|error: crate::search::SearchError| {
                TimelineError::at(line, 1, error.to_string())
            })?;
        let artifact = match tokens[cursor + 3].as_str() {
            "baseline" => ArtifactSource::Baseline(tokens[cursor + 4].parse().map_err(
                |error: crate::search::SearchError| TimelineError::at(line, 1, error.to_string()),
            )?),
            "candidate" => ArtifactSource::Candidate(PathBuf::from(&tokens[cursor + 4])),
            "tas" => ArtifactSource::Tas(PathBuf::from(&tokens[cursor + 4])),
            "tape" => ArtifactSource::Tape(PathBuf::from(&tokens[cursor + 4])),
            kind => {
                return Err(TimelineError::at(
                    line,
                    1,
                    format!("unknown segment artifact kind {kind:?}"),
                ));
            }
        };
        let segment = Segment {
            id: id.clone(),
            name: None,
            parent,
            profile,
            artifact,
            start_fingerprint: tokens[cursor + 6].clone(),
            end_fingerprint: tokens[cursor + 8].clone(),
            line,
        };
        if self.segments.insert(id.clone(), segment).is_some() {
            return Err(TimelineError::at(
                line,
                1,
                format!("duplicate segment {id}"),
            ));
        }
        Ok(())
    }

    fn parse_segment_label(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        exact_len(tokens, 3, line, "label SEGMENT DISPLAY_NAME")?;
        let id = tokens[1].clone();
        let label = tokens[2].trim().to_owned();
        if label.is_empty() || label.len() > 160 || label.chars().any(char::is_control) {
            return Err(TimelineError::at(
                line,
                1,
                "segment label must be 1 to 160 UTF-8 bytes without controls",
            ));
        }
        if self
            .segment_labels
            .insert(id.clone(), (label, line))
            .is_some()
        {
            return Err(TimelineError::at(
                line,
                1,
                format!("duplicate label for segment {id}"),
            ));
        }
        Ok(())
    }

    fn parse_subgraph(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        if tokens.len() != 7 && tokens.len() != 8 {
            return Err(TimelineError::at(
                line,
                1,
                "expected subgraph ID (root | inside PARENT) entry SEGMENT exit SEGMENT",
            ));
        }
        let (parent, cursor) = match tokens[2].as_str() {
            "root" if tokens.len() == 7 => (None, 3),
            "inside" if tokens.len() == 8 => (Some(tokens[3].clone()), 4),
            _ => {
                return Err(TimelineError::at(
                    line,
                    1,
                    "expected subgraph ID (root | inside PARENT) entry SEGMENT exit SEGMENT",
                ));
            }
        };
        expect(tokens, cursor, "entry", line)?;
        expect(tokens, cursor + 2, "exit", line)?;
        let id = tokens[1].clone();
        let subgraph = Subgraph {
            id: id.clone(),
            name: id.clone(),
            parent,
            entry_segment: tokens[cursor + 1].clone(),
            exit_segment: tokens[cursor + 3].clone(),
            segments: BTreeSet::new(),
            line,
        };
        if self.subgraphs.insert(id.clone(), subgraph).is_some() {
            return Err(TimelineError::at(
                line,
                1,
                format!("duplicate subgraph {id}"),
            ));
        }
        Ok(())
    }

    fn parse_subgraph_label(
        &mut self,
        tokens: &[String],
        line: usize,
    ) -> Result<(), TimelineError> {
        exact_len(tokens, 3, line, "subgraph_label SUBGRAPH DISPLAY_NAME")?;
        let id = tokens[1].clone();
        let label = tokens[2].trim().to_owned();
        if label.is_empty() || label.len() > 160 || label.chars().any(char::is_control) {
            return Err(TimelineError::at(
                line,
                1,
                "subgraph label must be 1 to 160 UTF-8 bytes without controls",
            ));
        }
        if self
            .subgraph_labels
            .insert(id.clone(), (label, line))
            .is_some()
        {
            return Err(TimelineError::at(
                line,
                1,
                format!("duplicate label for subgraph {id}"),
            ));
        }
        Ok(())
    }

    fn parse_subgraph_member(
        &mut self,
        tokens: &[String],
        line: usize,
    ) -> Result<(), TimelineError> {
        exact_len(tokens, 4, line, "subgraph_member SUBGRAPH segment SEGMENT")?;
        expect(tokens, 2, "segment", line)?;
        self.subgraph_members
            .push((tokens[1].clone(), tokens[3].clone(), line));
        Ok(())
    }

    fn parse_goal(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        if tokens.len() != 6 && tokens.len() != 8 {
            return Err(TimelineError::at(
                line,
                1,
                "expected goal GOAL_ID on SEGMENT predicate PREDICATE [source PATH]",
            ));
        }
        expect(tokens, 2, "on", line)?;
        expect(tokens, 4, "predicate", line)?;
        let predicate_source = if tokens.len() == 8 {
            expect(tokens, 6, "source", line)?;
            Some(parse_contained_relative_path(
                &tokens[7],
                line,
                "predicate source",
            )?)
        } else {
            None
        };
        let id = tokens[1].clone();
        let goal = Goal {
            id: id.clone(),
            segment: tokens[3].clone(),
            predicate: tokens[5].clone(),
            predicate_source,
            line,
        };
        if self.goals.insert(id.clone(), goal).is_some() {
            return Err(TimelineError::at(line, 1, format!("duplicate goal {id}")));
        }
        Ok(())
    }

    fn parse_proof(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        if tokens.len() != 8 && tokens.len() != 10 {
            return Err(TimelineError::at(
                line,
                1,
                "expected proof SEGMENT satisfies GOAL program SHA256 predicate SHA256 [ticks N]",
            ));
        }
        expect(tokens, 2, "satisfies", line)?;
        expect(tokens, 4, "program", line)?;
        expect(tokens, 6, "predicate", line)?;
        validate_sha256(&tokens[5], line, "predicate program")?;
        validate_sha256(&tokens[7], line, "predicate definition")?;
        let first_hit_tick = if tokens.len() == 10 {
            expect(tokens, 8, "ticks", line)?;
            Some(tokens[9].parse().map_err(|_| {
                TimelineError::at(line, 1, format!("invalid first-hit tick {:?}", tokens[9]))
            })?)
        } else {
            None
        };
        self.proofs.push(GoalProof {
            segment: tokens[1].clone(),
            goal: tokens[3].clone(),
            predicate_program_sha256: tokens[5].clone(),
            predicate_definition_sha256: tokens[7].clone(),
            first_hit_tick,
            line,
        });
        Ok(())
    }

    fn parse_continuation(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        exact_len(tokens, 4, line, "continuation NAME starts root@FINGERPRINT")?;
        expect(tokens, 2, "starts", line)?;
        let pin = parse_pin(&tokens[3], line)?;
        if pin.parent_segment != "root" {
            return Err(TimelineError::at(
                line,
                1,
                "continuation start must pin root@FINGERPRINT",
            ));
        }
        let name = tokens[1].clone();
        let continuation = Continuation {
            name: name.clone(),
            root_fingerprint: pin.checkpoint_fingerprint,
            steps: Vec::new(),
            line,
        };
        if self
            .continuations
            .insert(name.clone(), continuation)
            .is_some()
            || self.branches.contains_key(&name)
        {
            return Err(TimelineError::at(
                line,
                1,
                format!("duplicate lineage {name}"),
            ));
        }
        Ok(())
    }

    fn parse_branch(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        exact_len(tokens, 6, line, "branch NAME from LINEAGE after SEGMENT_ID")?;
        expect(tokens, 2, "from", line)?;
        expect(tokens, 4, "after", line)?;
        let name = tokens[1].clone();
        let branch = Branch {
            name: name.clone(),
            from_lineage: tokens[3].clone(),
            after_segment: tokens[5].clone(),
            steps: Vec::new(),
            line,
        };
        if self.branches.insert(name.clone(), branch).is_some()
            || self.continuations.contains_key(&name)
        {
            return Err(TimelineError::at(
                line,
                1,
                format!("duplicate lineage {name}"),
            ));
        }
        Ok(())
    }

    fn parse_continue(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        exact_len(
            tokens,
            6,
            line,
            "continue LINEAGE with SEGMENT after PARENT@FINGERPRINT",
        )?;
        expect(tokens, 2, "with", line)?;
        expect(tokens, 4, "after", line)?;
        let lineage = &tokens[1];
        let step = ContinuationStep {
            segment: tokens[3].clone(),
            after: parse_pin(&tokens[5], line)?,
            line,
        };
        if let Some(continuation) = self.continuations.get_mut(lineage) {
            continuation.steps.push(step);
        } else if let Some(branch) = self.branches.get_mut(lineage) {
            branch.steps.push(step);
        } else {
            return Err(TimelineError::at(
                line,
                1,
                format!("continue references undeclared lineage {lineage:?}"),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Subgraph {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub entry_segment: String,
    pub exit_segment: String,
    pub segments: BTreeSet<String>,
    #[serde(skip)]
    pub(super) line: usize,
}

fn parse_contained_relative_path(
    source: &str,
    line: usize,
    description: &str,
) -> Result<PathBuf, TimelineError> {
    let path = PathBuf::from(source);
    let windows_drive = source.as_bytes().get(1) == Some(&b':')
        && source
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
    let portable_components = source
        .split(['/', '\\'])
        .all(|component| !component.is_empty() && component != "." && component != "..");
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || windows_drive
        || !portable_components
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(TimelineError::at(
            line,
            1,
            format!("{description} must be a contained relative path"),
        ));
    }
    Ok(path)
}

fn parse_pin(token: &str, line: usize) -> Result<DependencyPin, TimelineError> {
    let (parent_segment, checkpoint_fingerprint) = token.rsplit_once('@').ok_or_else(|| {
        TimelineError::at(
            line,
            1,
            format!("dependency pin {token:?} must be PARENT@FINGERPRINT"),
        )
    })?;
    if parent_segment.is_empty() || checkpoint_fingerprint.is_empty() {
        return Err(TimelineError::at(
            line,
            1,
            format!("invalid dependency pin {token:?}"),
        ));
    }
    Ok(DependencyPin {
        parent_segment: parent_segment.into(),
        checkpoint_fingerprint: checkpoint_fingerprint.into(),
    })
}

/// Tokenizes one authored timeline line for syntax-aware workbench rewrites.
pub fn tokenize(line: &str, line_number: usize) -> Result<Vec<String>, TimelineError> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut quote_start = 0;
    for (column, character) in line.char_indices() {
        if quoted {
            match character {
                '"' => {
                    quoted = false;
                    output.push(std::mem::take(&mut current));
                }
                '\\' => {
                    return Err(TimelineError::at(
                        line_number,
                        column + 1,
                        "escape sequences are not supported in quoted timeline tokens",
                    ));
                }
                _ => current.push(character),
            }
            continue;
        }
        match character {
            '#' => break,
            '"' => {
                if !current.is_empty() {
                    return Err(TimelineError::at(
                        line_number,
                        column + 1,
                        "quote must start a new token",
                    ));
                }
                quoted = true;
                quote_start = column + 1;
            }
            value if value.is_whitespace() => {
                if !current.is_empty() {
                    output.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if quoted {
        return Err(TimelineError::at(
            line_number,
            quote_start,
            "unterminated quoted token",
        ));
    }
    if !current.is_empty() {
        output.push(current);
    }
    Ok(output)
}

fn exact_len(
    tokens: &[String],
    expected: usize,
    line: usize,
    usage: &str,
) -> Result<(), TimelineError> {
    if tokens.len() == expected {
        Ok(())
    } else {
        Err(TimelineError::at(line, 1, format!("expected {usage}")))
    }
}

fn expect(
    tokens: &[String],
    index: usize,
    expected: &str,
    line: usize,
) -> Result<(), TimelineError> {
    if tokens.get(index).map(String::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(TimelineError::at(
            line,
            1,
            format!("expected keyword {expected:?}"),
        ))
    }
}

fn required_token(
    tokens: &[String],
    index: usize,
    line: usize,
    description: &str,
) -> Result<String, TimelineError> {
    tokens
        .get(index)
        .cloned()
        .ok_or_else(|| TimelineError::at(line, 1, format!("missing {description}")))
}

fn validate_sha256(value: &str, line: usize, description: &str) -> Result<(), TimelineError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(TimelineError::at(
            line,
            1,
            format!("{description} SHA-256 must be 64 lowercase hexadecimal characters"),
        ))
    }
}

#[derive(Clone, Debug)]
pub struct TimelineError {
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

impl TimelineError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            line: None,
            column: None,
            message: message.into(),
        }
    }

    pub(super) fn at(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            column: Some(column),
            message: message.into(),
        }
    }
}

impl fmt::Display for TimelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => {
                write!(formatter, "timeline:{line}:{column}: {}", self.message)
            }
            _ => formatter.write_str(&self.message),
        }
    }
}

impl Error for TimelineError {}
