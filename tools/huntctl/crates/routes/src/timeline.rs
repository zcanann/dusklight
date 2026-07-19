//! Authored route timelines and immutable segment lineages.

use crate::milestone_dsl::{self, CompiledMilestones};
use crate::search::{Candidate, SegmentProfile};
use crate::tape::InputTape;
use crate::tape_dsl;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct Timeline {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate_program: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    pub segments: BTreeMap<String, Segment>,
    pub goals: BTreeMap<String, Goal>,
    pub proofs: Vec<GoalProof>,
    pub continuations: BTreeMap<String, Continuation>,
    pub branches: BTreeMap<String, Branch>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Origin {
    pub id: String,
    pub predicate: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate_source: Option<PathBuf>,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Segment {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub profile: SegmentProfile,
    pub artifact: ArtifactSource,
    pub start_fingerprint: String,
    pub end_fingerprint: String,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Goal {
    pub id: String,
    pub segment: String,
    pub predicate: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate_source: Option<PathBuf>,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct GoalProof {
    pub segment: String,
    pub goal: String,
    pub predicate_program_sha256: String,
    pub predicate_definition_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_hit_tick: Option<u64>,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ArtifactSource {
    Baseline(SegmentProfile),
    Candidate(PathBuf),
    Tas(PathBuf),
    Tape(PathBuf),
}

#[derive(Clone, Debug, Serialize)]
pub struct DependencyPin {
    pub parent_segment: String,
    pub checkpoint_fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContinuationStep {
    pub segment: String,
    pub after: DependencyPin,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Continuation {
    pub name: String,
    pub root_fingerprint: String,
    pub steps: Vec<ContinuationStep>,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Branch {
    pub name: String,
    pub from_lineage: String,
    pub after_segment: String,
    pub steps: Vec<ContinuationStep>,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct TimelineInspection {
    pub timeline: Timeline,
    pub lineages: Vec<ResolvedLineage>,
    pub frontiers: Vec<GoalFrontier>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResolvedLineage {
    pub name: String,
    pub kind: LineageKind,
    pub root_fingerprint: String,
    pub steps: Vec<ContinuationStep>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageKind {
    Continuation,
    Branch,
}

#[derive(Clone, Debug, Serialize)]
pub struct GoalFrontier {
    pub reference_segment: String,
    pub goal: String,
    pub segments: Vec<FrontierMember>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontierMember {
    pub segment: String,
    pub first_hit_tick: Option<u64>,
    pub relation_to_reference: DominanceRelation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DominanceRelation {
    Faster,
    Slower,
    Tied,
    Unscored,
    Incomparable,
}

#[derive(Clone, Debug, Serialize)]
pub struct TimelineStatus {
    pub timeline: String,
    pub immutable_lineages: Vec<LineageStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceStatus>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LineageStatus {
    pub name: String,
    pub kind: LineageKind,
    pub stale: bool,
    pub steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceStatus {
    pub base_lineage: String,
    pub selections: BTreeMap<String, String>,
    pub steps: Vec<WorkspaceStep>,
    pub stale_descendants: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceStep {
    pub original_segment: String,
    pub workspace_segment: String,
    pub state: WorkspaceStepState,
    pub rebase_compatible: bool,
    pub expected_start_fingerprint: String,
    pub actual_start_fingerprint: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStepState {
    Unchanged,
    Selected,
    Stale,
}

#[derive(Clone, Debug, Serialize)]
pub struct RebasePlan {
    pub base_lineage: String,
    pub new_continuation: String,
    pub old_lineage_preserved: bool,
    pub fully_compatible: bool,
    pub compatible_descendants: Vec<String>,
    pub stale_descendants: Vec<String>,
    pub steps: Vec<ContinuationStep>,
    pub authored_dsl: String,
}

impl Timeline {
    pub fn parse(source: &str) -> Result<Self, TimelineError> {
        Parser::new(source).parse()
    }

    pub fn inspect(&self) -> Result<TimelineInspection, TimelineError> {
        self.validate_artifacts(None)?;
        let mut lineages = Vec::new();
        for name in self.continuations.keys() {
            lineages.push(self.resolve_lineage(name)?);
        }
        for name in self.branches.keys() {
            lineages.push(self.resolve_lineage(name)?);
        }
        Ok(TimelineInspection {
            timeline: self.clone(),
            lineages,
            frontiers: self.frontiers(),
        })
    }

    pub fn compile_predicates(
        &self,
        root: &Path,
    ) -> Result<Option<CompiledMilestones>, TimelineError> {
        let Some(relative) = &self.predicate_program else {
            return Ok(None);
        };
        let root = if root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            root
        };
        let root = fs::canonicalize(root).map_err(|error| {
            TimelineError::new(format!(
                "cannot resolve timeline artifact root {}: {error}",
                root.display()
            ))
        })?;
        let path = fs::canonicalize(root.join(relative)).map_err(|error| {
            TimelineError::new(format!(
                "cannot resolve predicate program {}: {error}",
                root.join(relative).display()
            ))
        })?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(TimelineError::new(format!(
                "predicate program {} escapes the timeline artifact root",
                path.display()
            )));
        }
        let source = fs::read_to_string(&path).map_err(|error| {
            TimelineError::new(format!(
                "cannot read predicate program {}: {error}",
                path.display()
            ))
        })?;
        let program = milestone_dsl::parse(&source).map_err(|error| {
            TimelineError::new(format!(
                "invalid predicate program {}: {error}",
                path.display()
            ))
        })?;
        let authored = program
            .definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<HashSet<_>>();
        let mut referenced = self
            .goals
            .values()
            .map(|goal| (goal.line, goal.predicate.as_str(), "goal"))
            .collect::<Vec<_>>();
        if let Some(origin) = &self.origin {
            referenced.push((origin.line, origin.predicate.as_str(), "origin"));
        }
        for (line, predicate, owner) in referenced {
            if !authored.contains(predicate) {
                return Err(TimelineError::at(
                    line,
                    1,
                    format!(
                        "{owner} references predicate {predicate:?}, but predicate program {} does not define it",
                        path.display()
                    ),
                ));
            }
        }
        milestone_dsl::compile(&program).map(Some).map_err(|error| {
            TimelineError::new(format!(
                "cannot compile milestone program {}: {error}",
                path.display()
            ))
        })
    }

    pub fn origin_predicate_source(&self) -> Option<&Path> {
        self.origin
            .as_ref()
            .and_then(|origin| origin.predicate_source.as_deref())
            .or(self.predicate_program.as_deref())
    }

    pub fn goal_predicate_source(&self, goal_id: &str) -> Option<&Path> {
        self.goals
            .get(goal_id)
            .and_then(|goal| goal.predicate_source.as_deref())
            .or(self.predicate_program.as_deref())
    }

    pub fn compile_origin_predicate(
        &self,
        root: &Path,
    ) -> Result<Option<CompiledMilestones>, TimelineError> {
        let Some(origin) = &self.origin else {
            return Ok(None);
        };
        self.compile_owned_predicate(
            root,
            self.origin_predicate_source(),
            &origin.predicate,
            origin.line,
            "origin",
            origin.predicate_source.is_some(),
        )
        .map(Some)
    }

    pub fn compile_goal_predicate(
        &self,
        root: &Path,
        goal_id: &str,
    ) -> Result<CompiledMilestones, TimelineError> {
        let goal = self.goals.get(goal_id).ok_or_else(|| {
            TimelineError::new(format!("unknown goal {goal_id:?}"))
        })?;
        self.compile_owned_predicate(
            root,
            self.goal_predicate_source(goal_id),
            &goal.predicate,
            goal.line,
            &format!("goal {}", goal.id),
            goal.predicate_source.is_some(),
        )
    }

    fn compile_owned_predicate(
        &self,
        root: &Path,
        relative: Option<&Path>,
        expected: &str,
        line: usize,
        owner: &str,
        local: bool,
    ) -> Result<CompiledMilestones, TimelineError> {
        let relative = relative.ok_or_else(|| {
            TimelineError::at(line, 1, format!("{owner} has no predicate source"))
        })?;
        let root = fs::canonicalize(if root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            root
        })
        .map_err(|error| {
            TimelineError::new(format!(
                "cannot resolve timeline artifact root {}: {error}",
                root.display()
            ))
        })?;
        let unresolved = root.join(relative);
        let path = fs::canonicalize(&unresolved).map_err(|error| {
            TimelineError::new(format!(
                "cannot resolve predicate source {}: {error}",
                unresolved.display()
            ))
        })?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(TimelineError::new(format!(
                "predicate source {} escapes the timeline artifact root",
                path.display()
            )));
        }
        let source = fs::read_to_string(&path).map_err(|error| {
            TimelineError::new(format!(
                "cannot read predicate source {}: {error}",
                path.display()
            ))
        })?;
        let program = milestone_dsl::parse(&source).map_err(|error| {
            TimelineError::new(format!("invalid predicate source {}: {error}", path.display()))
        })?;
        if local
            && (program.definitions.len() != 1 || program.definitions[0].name != expected)
        {
            return Err(TimelineError::at(
                line,
                1,
                format!(
                    "{owner} predicate source {} must define exactly its own predicate {expected:?}",
                    path.display()
                ),
            ));
        }
        if !program
            .definitions
            .iter()
            .any(|definition| definition.name == expected)
        {
            return Err(TimelineError::at(
                line,
                1,
                format!(
                    "{owner} references predicate {expected:?}, but source {} does not define it",
                    path.display()
                ),
            ));
        }
        milestone_dsl::compile(&program).map_err(|error| {
            TimelineError::new(format!("cannot compile predicate source {}: {error}", path.display()))
        })
    }

    pub fn validate_artifacts(&self, root: Option<&Path>) -> Result<(), TimelineError> {
        self.validate_structure()?;
        let Some(root) = root else {
            return Ok(());
        };
        self.compile_origin_predicate(root)?;
        let mut compiled_goals = BTreeMap::new();
        for goal_id in self.goals.keys() {
            compiled_goals.insert(goal_id.as_str(), self.compile_goal_predicate(root, goal_id)?);
        }
        for proof in &self.proofs {
            let goal = &self.goals[&proof.goal];
            let compiled = &compiled_goals[goal.id.as_str()];
            let program_sha256 = compiled
                .program_sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let actual = compiled
                .definitions
                .iter()
                .find(|definition| definition.name == goal.predicate)
                .expect("compile_goal_predicate validates the owned definition");
            let actual = actual
                .sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if proof.predicate_program_sha256 != program_sha256 {
                return Err(TimelineError::at(
                    proof.line,
                    1,
                    format!(
                        "proof for segment {} and goal {} pins stale predicate source {}; current source program is {}",
                        proof.segment,
                        proof.goal,
                        proof.predicate_program_sha256,
                        program_sha256
                    ),
                ));
            }
            if proof.predicate_definition_sha256 != actual {
                return Err(TimelineError::at(
                    proof.line,
                    1,
                    format!(
                        "proof for segment {} and goal {} pins stale predicate {}; current predicate is {}",
                        proof.segment, proof.goal, proof.predicate_definition_sha256, actual
                    ),
                ));
            }
        }
        for segment in self.segments.values() {
            let profile = segment.profile;
            match &segment.artifact {
                ArtifactSource::Baseline(candidate_profile) => {
                    if *candidate_profile != profile {
                        return Err(TimelineError::at(
                            segment.line,
                            1,
                            format!(
                                "segment {} baseline profile {} does not match its declared profile {}",
                                segment.id,
                                candidate_profile.as_str(),
                                profile.as_str()
                            ),
                        ));
                    }
                    Candidate::baseline(*candidate_profile)
                        .validate()
                        .map_err(|error| TimelineError::at(segment.line, 1, error.to_string()))?;
                }
                ArtifactSource::Candidate(path) => {
                    let path = root.join(path);
                    let candidate: Candidate =
                        serde_json::from_slice(&fs::read(&path).map_err(|error| {
                            TimelineError::at(
                                segment.line,
                                1,
                                format!("cannot read candidate {}: {error}", path.display()),
                            )
                        })?)
                        .map_err(|error| TimelineError::at(segment.line, 1, error.to_string()))?;
                    candidate
                        .validate()
                        .map_err(|error| TimelineError::at(segment.line, 1, error.to_string()))?;
                    if candidate.segment != profile {
                        return Err(TimelineError::at(
                            segment.line,
                            1,
                            format!("candidate {} has the wrong segment profile", path.display()),
                        ));
                    }
                }
                ArtifactSource::Tas(path) => {
                    let path = root.join(path);
                    let source = fs::read_to_string(&path).map_err(|error| {
                        TimelineError::at(
                            segment.line,
                            1,
                            format!("cannot read TAS program {}: {error}", path.display()),
                        )
                    })?;
                    tape_dsl::parse(&source)
                        .map_err(|error| TimelineError::at(segment.line, 1, error.to_string()))?
                        .compile()
                        .map_err(|error| TimelineError::at(segment.line, 1, error.to_string()))?;
                }
                ArtifactSource::Tape(path) => {
                    let path = root.join(path);
                    InputTape::decode(&fs::read(&path).map_err(|error| {
                        TimelineError::at(
                            segment.line,
                            1,
                            format!("cannot read tape {}: {error}", path.display()),
                        )
                    })?)
                    .map_err(|error| TimelineError::at(segment.line, 1, error.to_string()))?;
                }
            }
        }
        Ok(())
    }

    pub fn status(
        &self,
        lineage: Option<&str>,
        selections: &BTreeMap<String, String>,
    ) -> Result<TimelineStatus, TimelineError> {
        self.validate_structure()?;
        let mut immutable_lineages = Vec::new();
        for name in self.continuations.keys().chain(self.branches.keys()) {
            let resolved = self.resolve_lineage(name)?;
            immutable_lineages.push(LineageStatus {
                name: resolved.name,
                kind: resolved.kind,
                stale: false,
                steps: resolved
                    .steps
                    .into_iter()
                    .map(|step| step.segment)
                    .collect(),
            });
        }
        let workspace = match lineage {
            Some(name) => Some(self.workspace(name, selections)?),
            None if selections.is_empty() => None,
            None => {
                return Err(TimelineError::new(
                    "workspace selections require --continuation NAME",
                ));
            }
        };
        Ok(TimelineStatus {
            timeline: self.name.clone(),
            immutable_lineages,
            workspace,
        })
    }

    pub fn rebase_compatible(
        &self,
        lineage: &str,
        selections: &BTreeMap<String, String>,
        new_name: &str,
    ) -> Result<RebasePlan, TimelineError> {
        if new_name.is_empty()
            || self.continuations.contains_key(new_name)
            || self.branches.contains_key(new_name)
        {
            return Err(TimelineError::new(format!(
                "new continuation name {new_name:?} is empty or already used"
            )));
        }
        let resolved = self.resolve_lineage(lineage)?;
        let workspace = self.workspace(lineage, selections)?;
        let mut steps = Vec::with_capacity(resolved.steps.len());
        let mut compatible_descendants = Vec::new();
        let mut stale_descendants = Vec::new();
        let mut previous_segment: Option<String> = None;
        let mut previous_fingerprint = resolved.root_fingerprint.clone();
        let mut chain_compatible = true;
        for (index, workspace_step) in workspace.steps.iter().enumerate() {
            let segment_id = &workspace_step.workspace_segment;
            let segment = &self.segments[segment_id];
            let compatible = chain_compatible
                && segment.parent == previous_segment
                && segment.start_fingerprint == previous_fingerprint;
            if index > 0 && workspace_step.state == WorkspaceStepState::Stale {
                if compatible {
                    compatible_descendants.push(segment_id.clone());
                } else {
                    stale_descendants.push(segment_id.clone());
                }
            }
            if !compatible {
                chain_compatible = false;
            }
            let parent_segment = previous_segment.clone().unwrap_or_else(|| "root".into());
            steps.push(ContinuationStep {
                segment: segment_id.clone(),
                after: DependencyPin {
                    parent_segment,
                    checkpoint_fingerprint: previous_fingerprint.clone(),
                },
                line: 0,
            });
            previous_segment = Some(segment_id.clone());
            previous_fingerprint = segment.end_fingerprint.clone();
        }
        let fully_compatible = chain_compatible;
        let authored_dsl = if fully_compatible {
            render_continuation(new_name, &resolved.root_fingerprint, &steps)
        } else {
            String::new()
        };
        Ok(RebasePlan {
            base_lineage: lineage.into(),
            new_continuation: new_name.into(),
            old_lineage_preserved: true,
            fully_compatible,
            compatible_descendants,
            stale_descendants,
            steps,
            authored_dsl,
        })
    }

    fn frontiers(&self) -> Vec<GoalFrontier> {
        let proof_ticks = self
            .proofs
            .iter()
            .map(|proof| {
                (
                    (proof.segment.as_str(), proof.goal.as_str()),
                    proof.first_hit_tick,
                )
            })
            .collect::<HashMap<_, _>>();
        let mut output = Vec::new();
        for goal in self.goals.values() {
            let reference_tick = proof_ticks
                .get(&(goal.segment.as_str(), goal.id.as_str()))
                .copied()
                .flatten();
            let segments = self
                .proofs
                .iter()
                .filter(|proof| proof.goal == goal.id)
                .map(|proof| FrontierMember {
                    segment: proof.segment.clone(),
                    first_hit_tick: proof.first_hit_tick,
                    relation_to_reference: dominance(proof.first_hit_tick, reference_tick),
                })
                .collect();
            output.push(GoalFrontier {
                reference_segment: goal.segment.clone(),
                goal: goal.id.clone(),
                segments,
            });
        }
        output
    }

    fn workspace(
        &self,
        lineage: &str,
        selections: &BTreeMap<String, String>,
    ) -> Result<WorkspaceStatus, TimelineError> {
        let resolved = self.resolve_lineage(lineage)?;
        let lineage_segments = resolved
            .steps
            .iter()
            .map(|step| step.segment.as_str())
            .collect::<HashSet<_>>();
        for (original_id, replacement_id) in selections {
            if !lineage_segments.contains(original_id.as_str()) {
                return Err(TimelineError::new(format!(
                    "selection segment {original_id:?} is not in lineage {lineage:?}"
                )));
            }
            let original = &self.segments[original_id];
            let replacement = self.segments.get(replacement_id).ok_or_else(|| {
                TimelineError::new(format!(
                    "selection references unknown segment {replacement_id:?}"
                ))
            })?;
            if replacement.profile != original.profile {
                return Err(TimelineError::new(format!(
                    "replacement segment {replacement_id} does not have the same profile as {original_id}"
                )));
            }
        }
        let mut changed_upstream = false;
        let mut chain_compatible = true;
        let mut expected_start = resolved.root_fingerprint.clone();
        let mut expected_parent: Option<String> = None;
        let mut steps = Vec::new();
        let mut stale_descendants = Vec::new();
        for original in &resolved.steps {
            let selected = selections
                .get(&original.segment)
                .cloned()
                .unwrap_or_else(|| original.segment.clone());
            let workspace_segment = &self.segments[&selected];
            let selected_changed = selected != original.segment;
            let state = if selected_changed {
                changed_upstream = true;
                WorkspaceStepState::Selected
            } else if changed_upstream {
                stale_descendants.push(selected.clone());
                WorkspaceStepState::Stale
            } else {
                WorkspaceStepState::Unchanged
            };
            let compatible = chain_compatible
                && workspace_segment.parent == expected_parent
                && workspace_segment.start_fingerprint == expected_start;
            if !compatible {
                chain_compatible = false;
            }
            steps.push(WorkspaceStep {
                original_segment: original.segment.clone(),
                workspace_segment: selected,
                state,
                rebase_compatible: compatible,
                expected_start_fingerprint: expected_start.clone(),
                actual_start_fingerprint: workspace_segment.start_fingerprint.clone(),
            });
            expected_start = workspace_segment.end_fingerprint.clone();
            expected_parent = Some(workspace_segment.id.clone());
        }
        Ok(WorkspaceStatus {
            base_lineage: lineage.into(),
            selections: selections.clone(),
            steps,
            stale_descendants,
        })
    }

    fn resolve_lineage(&self, name: &str) -> Result<ResolvedLineage, TimelineError> {
        if let Some(continuation) = self.continuations.get(name) {
            return Ok(ResolvedLineage {
                name: name.into(),
                kind: LineageKind::Continuation,
                root_fingerprint: continuation.root_fingerprint.clone(),
                steps: continuation.steps.clone(),
            });
        }
        let branch = self.branches.get(name).ok_or_else(|| {
            TimelineError::new(format!("unknown continuation or branch {name:?}"))
        })?;
        let base = self.resolve_lineage(&branch.from_lineage)?;
        let mut prefix = Vec::new();
        let mut reached = false;
        for step in base.steps {
            reached = step.segment == branch.after_segment;
            prefix.push(step);
            if reached {
                break;
            }
        }
        if !reached {
            return Err(TimelineError::at(
                branch.line,
                1,
                format!(
                    "branch {} fork segment {} is not reached by {}",
                    branch.name, branch.after_segment, branch.from_lineage
                ),
            ));
        }
        prefix.extend(branch.steps.clone());
        Ok(ResolvedLineage {
            name: name.into(),
            kind: LineageKind::Branch,
            root_fingerprint: base.root_fingerprint,
            steps: prefix,
        })
    }
}

fn dominance(first_hit_tick: Option<u64>, reference_tick: Option<u64>) -> DominanceRelation {
    let Some(reference_tick) = reference_tick else {
        return DominanceRelation::Incomparable;
    };
    match first_hit_tick {
        Some(left) if left < reference_tick => DominanceRelation::Faster,
        Some(left) if left > reference_tick => DominanceRelation::Slower,
        Some(_) => DominanceRelation::Tied,
        None => DominanceRelation::Unscored,
    }
}

fn render_continuation(name: &str, root_fingerprint: &str, steps: &[ContinuationStep]) -> String {
    let mut output = format!("continuation {name} starts root@{root_fingerprint}\n");
    for step in steps {
        output.push_str(&format!(
            "continue {name} with {} after {}@{}\n",
            step.segment, step.after.parent_segment, step.after.checkpoint_fingerprint
        ));
    }
    output
}

impl Timeline {
    fn validate_structure(&self) -> Result<(), TimelineError> {
        if self.name.is_empty() {
            return Err(TimelineError::new("timeline name is empty"));
        }
        if let Some(origin) = &self.origin
            && origin.predicate_source.is_none()
            && self.predicate_program.is_none()
        {
            return Err(TimelineError::at(
                origin.line,
                1,
                "origin requires its own predicate source (or a legacy predicate_program)",
            ));
        }
        for goal in self.goals.values() {
            if goal.predicate_source.is_none() && self.predicate_program.is_none() {
                return Err(TimelineError::at(
                    goal.line,
                    1,
                    format!(
                        "goal {} requires its own predicate source (or a legacy predicate_program)",
                        goal.id
                    ),
                ));
            }
        }
        for segment in self.segments.values() {
            if segment.parent.as_deref() == Some(segment.id.as_str()) {
                return Err(TimelineError::at(
                    segment.line,
                    1,
                    format!("segment {} cannot be its own parent", segment.id),
                ));
            }
            if let Some(parent) = &segment.parent
                && !self.segments.contains_key(parent)
            {
                return Err(TimelineError::at(
                    segment.line,
                    1,
                    format!("segment {} references unknown parent {parent}", segment.id),
                ));
            }
        }
        self.validate_segment_forest()?;
        for segment in self.segments.values() {
            if let Some(parent_id) = &segment.parent {
                let parent = &self.segments[parent_id];
                if segment.start_fingerprint != parent.end_fingerprint {
                    return Err(TimelineError::at(
                        segment.line,
                        1,
                        format!(
                            "segment {} starts at {}, but its parent {} ends at {}",
                            segment.id,
                            segment.start_fingerprint,
                            parent.id,
                            parent.end_fingerprint
                        ),
                    ));
                }
            }
        }

        for goal in self.goals.values() {
            if !self.segments.contains_key(&goal.segment) {
                return Err(TimelineError::at(
                    goal.line,
                    1,
                    format!(
                        "goal {} references unknown segment {}",
                        goal.id, goal.segment
                    ),
                ));
            }
        }

        for segment in self.segments.values() {
            if segment.start_fingerprint.is_empty() || segment.end_fingerprint.is_empty() {
                return Err(TimelineError::at(
                    segment.line,
                    1,
                    format!("segment {} has an empty boundary fingerprint", segment.id),
                ));
            }
        }
        let mut proof_pairs = HashSet::new();
        for proof in &self.proofs {
            let proving_segment = self.segments.get(&proof.segment).ok_or_else(|| {
                TimelineError::at(
                    proof.line,
                    1,
                    format!("proof references unknown segment {}", proof.segment),
                )
            })?;
            let goal = self.goals.get(&proof.goal).ok_or_else(|| {
                TimelineError::at(
                    proof.line,
                    1,
                    format!("proof references unknown goal {}", proof.goal),
                )
            })?;
            let reference_segment = &self.segments[&goal.segment];
            if proving_segment.id != reference_segment.id
                && proving_segment.parent != reference_segment.parent
            {
                return Err(TimelineError::at(
                    proof.line,
                    1,
                    format!(
                        "segment {} cannot satisfy goal {} on unrelated segment {}; proofs require the reference segment or one of its siblings",
                        proving_segment.id, goal.id, reference_segment.id
                    ),
                ));
            }
            if !proof_pairs.insert((proof.segment.as_str(), proof.goal.as_str())) {
                return Err(TimelineError::at(
                    proof.line,
                    1,
                    format!(
                        "duplicate proof for segment {} and goal {}",
                        proof.segment, proof.goal
                    ),
                ));
            }
        }
        for continuation in self.continuations.values() {
            if continuation.steps.is_empty() {
                return Err(TimelineError::at(
                    continuation.line,
                    1,
                    format!("continuation {} has no steps", continuation.name),
                ));
            }
            self.validate_steps(
                &continuation.name,
                &continuation.root_fingerprint,
                &continuation.steps,
            )?;
        }
        for branch in self.branches.values() {
            if !self.segments.contains_key(&branch.after_segment) {
                return Err(TimelineError::at(
                    branch.line,
                    1,
                    format!(
                        "branch {} references unknown fork segment {}",
                        branch.name, branch.after_segment
                    ),
                ));
            }
            if !self.continuations.contains_key(&branch.from_lineage)
                && !self.branches.contains_key(&branch.from_lineage)
            {
                return Err(TimelineError::at(
                    branch.line,
                    1,
                    format!(
                        "branch {} references unknown lineage {}",
                        branch.name, branch.from_lineage
                    ),
                ));
            }
        }
        self.validate_branch_cycles()?;
        for name in self.branches.keys() {
            let lineage = self.resolve_lineage(name)?;
            self.validate_steps(name, &lineage.root_fingerprint, &lineage.steps)?;
        }
        Ok(())
    }

    fn validate_steps(
        &self,
        name: &str,
        root_fingerprint: &str,
        steps: &[ContinuationStep],
    ) -> Result<(), TimelineError> {
        let mut prior_segment: Option<&Segment> = None;
        let mut expected_fingerprint = root_fingerprint;
        for step in steps {
            let segment = self.segments.get(&step.segment).ok_or_else(|| {
                TimelineError::at(
                    step.line,
                    1,
                    format!("{name} references unknown segment {}", step.segment),
                )
            })?;
            let expected_parent = prior_segment.map_or("root", |prior| prior.id.as_str());
            if step.after.parent_segment != expected_parent {
                return Err(TimelineError::at(
                    step.line,
                    1,
                    format!(
                        "{name} step {} pins parent {}, expected exact parent {}",
                        segment.id, step.after.parent_segment, expected_parent
                    ),
                ));
            }
            if step.after.checkpoint_fingerprint != expected_fingerprint
                || segment.start_fingerprint != expected_fingerprint
            {
                return Err(TimelineError::at(
                    step.line,
                    1,
                    format!(
                        "{name} step {} boundary mismatch: pin={}, segment starts={}, expected={}",
                        segment.id,
                        step.after.checkpoint_fingerprint,
                        segment.start_fingerprint,
                        expected_fingerprint
                    ),
                ));
            }
            if let Some(prior) = prior_segment {
                if segment.parent.as_deref() != Some(prior.id.as_str()) {
                    return Err(TimelineError::at(
                        step.line,
                        1,
                        format!(
                            "{name} is discontinuous: segment {} is not a child of {}",
                            segment.id, prior.id
                        ),
                    ));
                }
            } else if segment.parent.is_some() {
                return Err(TimelineError::at(
                    step.line,
                    1,
                    format!("{name} starts with non-root segment {}", segment.id),
                ));
            }
            expected_fingerprint = &segment.end_fingerprint;
            prior_segment = Some(segment);
        }
        Ok(())
    }

    fn validate_segment_forest(&self) -> Result<(), TimelineError> {
        for segment in self.segments.values() {
            let mut seen = HashSet::new();
            let mut current = Some(segment.id.as_str());
            while let Some(name) = current {
                if !seen.insert(name) {
                    return Err(TimelineError::at(
                        segment.line,
                        1,
                        format!("segment parent hierarchy contains a cycle at {name}"),
                    ));
                }
                current = self.segments[name].parent.as_deref();
            }
        }
        Ok(())
    }

    fn validate_branch_cycles(&self) -> Result<(), TimelineError> {
        for start in self.branches.keys() {
            let mut seen = HashSet::new();
            let mut current = start.as_str();
            while let Some(branch) = self.branches.get(current) {
                if !seen.insert(current) {
                    return Err(TimelineError::at(
                        branch.line,
                        1,
                        format!("branch ancestry contains a cycle at {current}"),
                    ));
                }
                current = &branch.from_lineage;
            }
        }
        Ok(())
    }
}

struct Parser<'a> {
    source: &'a str,
    timeline_name: Option<String>,
    predicate_program: Option<PathBuf>,
    origin: Option<Origin>,
    segments: BTreeMap<String, Segment>,
    segment_labels: BTreeMap<String, (String, usize)>,
    goals: BTreeMap<String, Goal>,
    proofs: Vec<GoalProof>,
    continuations: BTreeMap<String, Continuation>,
    branches: BTreeMap<String, Branch>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            timeline_name: None,
            predicate_program: None,
            origin: None,
            segments: BTreeMap::new(),
            segment_labels: BTreeMap::new(),
            goals: BTreeMap::new(),
            proofs: Vec::new(),
            continuations: BTreeMap::new(),
            branches: BTreeMap::new(),
        }
    }

    fn parse(mut self) -> Result<Timeline, TimelineError> {
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
        let timeline = Timeline {
            name: self
                .timeline_name
                .ok_or_else(|| TimelineError::new("missing timeline declaration"))?,
            predicate_program: self.predicate_program,
            origin: self.origin,
            segments: self.segments,
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
        if tokens.len() != 4 && tokens.len() != 6 {
            return Err(TimelineError::at(
                line,
                1,
                "expected origin boot predicate PREDICATE [source PATH]",
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
        let predicate_source = if tokens.len() == 6 {
            expect(tokens, 4, "source", line)?;
            Some(parse_contained_relative_path(
                &tokens[5],
                line,
                "predicate source",
            )?)
        } else {
            None
        };
        let origin = Origin {
            id: tokens[1].clone(),
            predicate: tokens[3].clone(),
            predicate_source,
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
    fn new(message: impl Into<String>) -> Self {
        Self {
            line: None,
            column: None,
            message: message.into(),
        }
    }

    fn at(line: usize, column: usize, message: impl Into<String>) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTE: &str = r#"
timeline intro
predicate_program route.milestones
origin boot predicate process_boot
segment boot_safe root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean-rng1 produces control-rng1
label boot_safe "Conservative boot"
segment boot_fast root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean-rng1 produces control-rng1
segment boot_other_rng root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean-rng2 produces control-rng2
segment exit_safe after boot_safe profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1
segment exit_rolls after boot_safe profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1
segment exit_repaired after boot_fast profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1
goal link_control on boot_safe predicate link_control
goal next_map on exit_safe predicate next_map
proof boot_safe satisfies link_control program 2222222222222222222222222222222222222222222222222222222222222222 predicate 1111111111111111111111111111111111111111111111111111111111111111 ticks 700
proof boot_fast satisfies link_control program 2222222222222222222222222222222222222222222222222222222222222222 predicate 1111111111111111111111111111111111111111111111111111111111111111 ticks 650
continuation main starts root@clean-rng1
continue main with boot_safe after root@clean-rng1
continue main with exit_safe after boot_safe@control-rng1
branch rolls from main after boot_safe
continue rolls with exit_rolls after boot_safe@control-rng1
"#;

    #[test]
    fn parses_segment_tree_continuations_branches_and_goal_frontiers() {
        let timeline = Timeline::parse(ROUTE).unwrap();
        let inspection = timeline.inspect().unwrap();
        assert_eq!(inspection.timeline.name, "intro");
        assert_eq!(inspection.lineages.len(), 2);
        assert_eq!(timeline.segments["boot_fast"].parent, None);
        assert_eq!(
            timeline.segments["boot_safe"].name.as_deref(),
            Some("Conservative boot")
        );
        assert_eq!(
            timeline.segments["exit_safe"].parent.as_deref(),
            Some("boot_safe")
        );
        let rolls = inspection
            .lineages
            .iter()
            .find(|lineage| lineage.name == "rolls")
            .unwrap();
        assert_eq!(rolls.steps[0].segment, "boot_safe");
        assert_eq!(rolls.steps[1].segment, "exit_rolls");
        let frontier = inspection
            .frontiers
            .iter()
            .find(|frontier| {
                frontier.reference_segment == "boot_safe"
                    && frontier.goal == "link_control"
                    && frontier.segments.len() == 2
            })
            .unwrap();
        assert_eq!(
            frontier
                .segments
                .iter()
                .find(|member| member.segment == "boot_fast")
                .unwrap()
                .relation_to_reference,
            DominanceRelation::Faster
        );
    }

    #[test]
    fn goal_proof_is_segment_metadata_and_may_be_satisfied_by_a_sibling() {
        let program_digest = "2".repeat(64);
        let digest = "1".repeat(64);
        let timeline = Timeline::parse(ROUTE).unwrap();
        let proof = &timeline.proofs[0];
        assert_eq!(proof.segment, "boot_safe");
        assert_eq!(proof.goal, "link_control");
        assert_eq!(proof.predicate_program_sha256, program_digest);
        assert_eq!(proof.predicate_definition_sha256, digest);
        assert_eq!(proof.first_hit_tick, Some(700));

        let invalid = ROUTE.replace(
            "program 2222222222222222222222222222222222222222222222222222222222222222",
            "program NOT-A-DIGEST",
        );
        assert!(
            Timeline::parse(&invalid)
                .unwrap_err()
                .to_string()
                .contains("64 lowercase hexadecimal")
        );

        assert!(timeline.proofs.iter().any(|proof| {
            proof.segment == "boot_fast"
                && proof.goal == "link_control"
                && timeline.segments["boot_fast"].parent == timeline.segments["boot_safe"].parent
        }));

        let duplicate = format!(
            "{ROUTE}\nproof boot_safe satisfies link_control program {} predicate {}",
            "2".repeat(64),
            "1".repeat(64)
        );
        assert!(
            Timeline::parse(&duplicate)
                .unwrap_err()
                .to_string()
                .contains("duplicate proof")
        );
    }

    #[test]
    fn rejects_goal_proof_from_an_unrelated_segment() {
        let unrelated = ROUTE.replace(
            "proof boot_fast satisfies link_control",
            "proof exit_safe satisfies link_control",
        );
        let error = Timeline::parse(&unrelated).unwrap_err();
        assert!(error.to_string().contains("unrelated segment boot_safe"));
        assert!(
            error
                .to_string()
                .contains("reference segment or one of its siblings")
        );
    }

    #[test]
    fn rejects_unpinned_parent_boundary_mismatch() {
        let mismatch = ROUTE.replace(
            "segment exit_repaired after boot_fast profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts control-rng1",
            "segment exit_repaired after boot_fast profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts wrong-parent-state",
        );
        let error = Timeline::parse(&mismatch).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("segment exit_repaired starts at wrong-parent-state")
        );
        assert!(
            error
                .to_string()
                .contains("parent boot_fast ends at control-rng1")
        );
    }

    #[test]
    fn adding_a_sibling_does_not_change_a_pinned_lineage() {
        let source = ROUTE.replace(
            "segment boot_other_rng root",
            "segment another_boot root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean-rng1 produces other\nsegment boot_other_rng root",
        );
        let timeline = Timeline::parse(&source).unwrap();
        let status = timeline.status(None, &BTreeMap::new()).unwrap();
        assert!(
            status
                .immutable_lineages
                .iter()
                .all(|lineage| !lineage.stale)
        );
        assert_eq!(status.immutable_lineages[0].steps[0], "boot_safe");
    }

    #[test]
    fn workspace_marks_descendants_stale_and_rebase_creates_new_lineage() {
        let timeline = Timeline::parse(ROUTE).unwrap();
        let selections = BTreeMap::from([("boot_safe".into(), "boot_fast".into())]);
        let status = timeline.status(Some("main"), &selections).unwrap();
        let workspace = status.workspace.unwrap();
        assert_eq!(workspace.steps[0].state, WorkspaceStepState::Selected);
        assert_eq!(workspace.steps[1].state, WorkspaceStepState::Stale);
        assert!(!workspace.steps[1].rebase_compatible);

        let poisoned = timeline
            .rebase_compatible("main", &selections, "main_fast")
            .unwrap();
        assert!(poisoned.old_lineage_preserved);
        assert!(!poisoned.fully_compatible);
        assert_eq!(poisoned.stale_descendants, vec!["exit_safe"]);

        let repaired_selections = BTreeMap::from([
            ("boot_safe".into(), "boot_fast".into()),
            ("exit_safe".into(), "exit_repaired".into()),
        ]);
        let repair = timeline
            .rebase_compatible("main", &repaired_selections, "main_repaired")
            .unwrap();
        assert!(repair.fully_compatible);
        assert!(
            repair
                .authored_dsl
                .contains("continue main_repaired with exit_repaired after boot_fast@control-rng1")
        );
    }

    #[test]
    fn rejects_cycles_bad_references_and_boundary_mismatches_with_lines() {
        let cycle = ROUTE.replace(
            "segment boot_safe root profile",
            "segment boot_safe after exit_safe profile",
        );
        assert!(
            Timeline::parse(&cycle)
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );

        let unknown = ROUTE.replace("with exit_safe", "with exit_missing");
        let error = Timeline::parse(&unknown).unwrap_err();
        assert!(error.to_string().contains("unknown segment"));
        assert!(error.line.is_some());

        let mismatch = ROUTE.replace("after boot_safe@control-rng1", "after boot_safe@wrong");
        assert!(
            Timeline::parse(&mismatch)
                .unwrap_err()
                .to_string()
                .contains("boundary mismatch")
        );

        let off_lineage = ROUTE.replace(
            "branch rolls from main after boot_safe",
            "branch rolls from main after boot_other_rng",
        );
        assert!(
            Timeline::parse(&off_lineage)
                .unwrap_err()
                .to_string()
                .contains("is not reached by main")
        );
    }

    #[test]
    fn parser_reports_quoted_token_diagnostics() {
        let error = Timeline::parse("timeline \"unterminated").unwrap_err();
        assert_eq!(error.line, Some(1));
        assert!(error.to_string().contains("unterminated"));
    }

    #[test]
    fn segment_labels_are_unique_bounded_metadata_for_existing_segments() {
        for source in [
            "timeline bad\nlabel missing \"Unknown\"",
            "timeline bad\nsegment root root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces end\nlabel root one\nlabel root two",
            "timeline bad\nsegment root root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces end\nlabel root \"\"",
        ] {
            assert!(Timeline::parse(source).is_err());
        }
    }

    #[test]
    fn rejects_the_removed_variant_entity() {
        let error = Timeline::parse(
            "timeline old\nvariant boot.safe uses baseline boot_to_fsp103 starts clean produces control",
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unknown timeline statement \"variant\"")
        );
    }

    #[test]
    fn parses_one_relative_predicate_program() {
        let timeline = Timeline::parse(ROUTE).unwrap();
        assert_eq!(
            timeline.predicate_program.as_deref(),
            Some(Path::new("route.milestones"))
        );

        let absolute =
            Timeline::parse("timeline bad\npredicate_program C:/bad/program").unwrap_err();
        assert!(absolute.to_string().contains("contained relative path"));
        let traversal = Timeline::parse("timeline bad\npredicate_program ../outside").unwrap_err();
        assert!(traversal.to_string().contains("contained relative path"));
        let windows_traversal =
            Timeline::parse("timeline bad\npredicate_program ..\\outside").unwrap_err();
        assert!(
            windows_traversal
                .to_string()
                .contains("contained relative path")
        );
        let duplicate =
            Timeline::parse("timeline bad\npredicate_program a\npredicate_program b").unwrap_err();
        assert!(
            duplicate
                .to_string()
                .contains("duplicate predicate_program")
        );
    }

    #[test]
    fn compiles_referenced_predicates_allows_unused_definitions_and_checks_proofs() {
        let root = std::env::temp_dir().join(format!(
            "huntctl-timeline-milestones-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("route.milestones"),
            r#"milestones 1.0
milestone process_boot {
  phase pre_input
  when boundary.kind == "boot" && boundary.index == 0
}
milestone link_control {
  phase post_sim
  when stage.name == "F_SP103" && player.exists
}
milestone unused_probe {
  phase post_sim
  when player.exists
}
"#,
        )
        .unwrap();
        let timeline = Timeline::parse(
            r#"timeline route
predicate_program route.milestones
origin boot predicate process_boot
segment boot root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces control
goal control on boot predicate link_control
continuation main starts root@clean
continue main with boot after root@clean
"#,
        )
        .unwrap();
        let compiled = timeline.compile_predicates(&root).unwrap().unwrap();
        assert_eq!(compiled.definitions.len(), 3);
        timeline.validate_artifacts(Some(&root)).unwrap();

        let program_digest = compiled
            .program_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let predicate_digest = compiled
            .definitions
            .iter()
            .find(|definition| definition.name == "link_control")
            .unwrap()
            .sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let pinned = Timeline::parse(&format!(
            r#"timeline route
predicate_program route.milestones
origin boot predicate process_boot
segment boot root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces control
goal control on boot predicate link_control
proof boot satisfies control program {program_digest} predicate {predicate_digest}
continuation main starts root@clean
continue main with boot after root@clean
"#
        ))
        .unwrap();
        pinned.validate_artifacts(Some(&root)).unwrap();

        let changed = fs::read_to_string(root.join("route.milestones"))
            .unwrap()
            .replace(
                "phase post_sim\n  when",
                "phase post_sim\n  stable 2\n  when",
            );
        fs::write(root.join("route.milestones"), changed).unwrap();
        assert!(
            pinned
                .validate_artifacts(Some(&root))
                .unwrap_err()
                .to_string()
                .contains("stale predicate source")
        );

        let missing = Timeline::parse(
            r#"timeline route
predicate_program route.milestones
segment boot root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces control
goal control on boot predicate not_defined
continuation main starts root@clean
continue main with boot after root@clean
"#,
        )
        .unwrap();
        assert!(
            missing
                .compile_predicates(&root)
                .unwrap_err()
                .to_string()
                .contains("does not define")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn goal_predicate_sources_are_local_and_independently_identified() {
        let root = std::env::temp_dir().join(format!(
            "huntctl-timeline-owned-predicates-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("first.milestones"),
            "milestones 1.0\nmilestone first { phase post_sim when player.exists }\n",
        )
        .unwrap();
        fs::write(
            root.join("second.milestones"),
            "milestones 1.0\nmilestone second { phase post_sim when event.running }\n",
        )
        .unwrap();
        let timeline = Timeline::parse(
            r#"timeline owned
segment root root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces one
segment child after root profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts one produces two
goal first_goal on root predicate first source first.milestones
goal second_goal on child predicate second source second.milestones
continuation main starts root@clean
continue main with root after root@clean
continue main with child after root@one
"#,
        )
        .unwrap();
        timeline.validate_artifacts(Some(&root)).unwrap();
        let first_before = timeline
            .compile_goal_predicate(&root, "first_goal")
            .unwrap()
            .program_sha256;
        let second_before = timeline
            .compile_goal_predicate(&root, "second_goal")
            .unwrap()
            .program_sha256;
        fs::write(
            root.join("second.milestones"),
            "milestones 1.0\nmilestone second { phase post_sim stable 2 when event.running }\n",
        )
        .unwrap();
        assert_eq!(
            timeline
                .compile_goal_predicate(&root, "first_goal")
                .unwrap()
                .program_sha256,
            first_before
        );
        assert_ne!(
            timeline
                .compile_goal_predicate(&root, "second_goal")
                .unwrap()
                .program_sha256,
            second_before
        );

        fs::write(
            root.join("second.milestones"),
            "milestones 1.0\nmilestone second { phase post_sim when event.running }\nmilestone historical_coupling { phase post_sim when player.exists }\n",
        )
        .unwrap();
        assert!(
            timeline
                .compile_goal_predicate(&root, "second_goal")
                .unwrap_err()
                .to_string()
                .contains("exactly its own predicate")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validates_git_tracked_tas_artifacts() {
        let root =
            std::env::temp_dir().join(format!("huntctl-timeline-tas-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("boot.tas"),
            "dusktape 1\nrate 30/1\nports 0x0f\nstate neutral {}\nframe neutral\n",
        )
        .unwrap();
        let timeline = Timeline::parse(
            r#"
timeline tas_route
segment boot_link root profile boot_to_fsp103 uses tas boot.tas starts clean produces control
continuation main starts root@clean
continue main with boot_link after root@clean
"#,
        )
        .unwrap();
        timeline.validate_artifacts(Some(&root)).unwrap();
        let _ = fs::remove_dir_all(root);
    }
}
