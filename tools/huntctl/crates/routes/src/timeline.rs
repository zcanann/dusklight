//! Authored route timelines and immutable segment lineages.

use crate::milestone_dsl::{self, CompiledMilestones};
use crate::search::{Candidate, SegmentProfile};
use crate::tape::InputTape;
use crate::tape_dsl;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

mod parser;
use parser::Parser;
pub use parser::{Subgraph, TimelineError, tokenize};

#[derive(Clone, Debug, Serialize)]
pub struct Timeline {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate_program: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    pub segments: BTreeMap<String, Segment>,
    pub subgraphs: BTreeMap<String, Subgraph>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_fixture: Option<PathBuf>,
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
        let goal = self
            .goals
            .get(goal_id)
            .ok_or_else(|| TimelineError::new(format!("unknown goal {goal_id:?}")))?;
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
            TimelineError::new(format!(
                "invalid predicate source {}: {error}",
                path.display()
            ))
        })?;
        if local && (program.definitions.len() != 1 || program.definitions[0].name != expected) {
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
            TimelineError::new(format!(
                "cannot compile predicate source {}: {error}",
                path.display()
            ))
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
            compiled_goals.insert(
                goal_id.as_str(),
                self.compile_goal_predicate(root, goal_id)?,
            );
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
                        proof.segment, proof.goal, proof.predicate_program_sha256, program_sha256
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
        self.validate_subgraphs()?;
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

    fn validate_subgraphs(&self) -> Result<(), TimelineError> {
        let mut owners = HashMap::<&str, &str>::new();
        for subgraph in self.subgraphs.values() {
            if subgraph.parent.as_deref() == Some(subgraph.id.as_str()) {
                return Err(TimelineError::at(
                    subgraph.line,
                    1,
                    format!("subgraph {} cannot contain itself", subgraph.id),
                ));
            }
            if let Some(parent) = &subgraph.parent
                && !self.subgraphs.contains_key(parent)
            {
                return Err(TimelineError::at(
                    subgraph.line,
                    1,
                    format!(
                        "subgraph {} references unknown parent {parent}",
                        subgraph.id
                    ),
                ));
            }
            for segment in &subgraph.segments {
                if !self.segments.contains_key(segment) {
                    return Err(TimelineError::at(
                        subgraph.line,
                        1,
                        format!(
                            "subgraph {} references unknown segment {segment}",
                            subgraph.id
                        ),
                    ));
                }
                if let Some(previous) = owners.insert(segment, &subgraph.id) {
                    return Err(TimelineError::at(
                        subgraph.line,
                        1,
                        format!(
                            "segment {segment} belongs directly to both {previous} and {}",
                            subgraph.id
                        ),
                    ));
                }
            }
        }

        for start in self.subgraphs.values() {
            let mut seen = HashSet::new();
            let mut current = Some(start.id.as_str());
            while let Some(id) = current {
                if !seen.insert(id) {
                    return Err(TimelineError::at(
                        start.line,
                        1,
                        format!("subgraph hierarchy contains a cycle at {id}"),
                    ));
                }
                current = self.subgraphs[id].parent.as_deref();
            }
        }

        for subgraph in self.subgraphs.values() {
            let closure = self.subgraph_segment_closure(&subgraph.id);
            if !closure.contains(&subgraph.entry_segment) {
                return Err(TimelineError::at(
                    subgraph.line,
                    1,
                    format!(
                        "subgraph {} entry {} is not contained by it",
                        subgraph.id, subgraph.entry_segment
                    ),
                ));
            }
            if !closure.contains(&subgraph.exit_segment) {
                return Err(TimelineError::at(
                    subgraph.line,
                    1,
                    format!(
                        "subgraph {} exit {} is not contained by it",
                        subgraph.id, subgraph.exit_segment
                    ),
                ));
            }
            for segment_id in &closure {
                let segment = &self.segments[segment_id];
                let parent_inside = segment
                    .parent
                    .as_ref()
                    .is_some_and(|parent| closure.contains(parent));
                if segment_id == &subgraph.entry_segment {
                    if parent_inside {
                        return Err(TimelineError::at(
                            subgraph.line,
                            1,
                            format!(
                                "subgraph {} entry {} has a parent inside the subgraph",
                                subgraph.id, segment_id
                            ),
                        ));
                    }
                } else if !parent_inside {
                    return Err(TimelineError::at(
                        subgraph.line,
                        1,
                        format!(
                            "subgraph {} has a second entry at segment {segment_id}",
                            subgraph.id
                        ),
                    ));
                }
                for child in self
                    .segments
                    .values()
                    .filter(|child| child.parent.as_deref() == Some(segment_id))
                {
                    if !closure.contains(&child.id) && segment_id != &subgraph.exit_segment {
                        return Err(TimelineError::at(
                            subgraph.line,
                            1,
                            format!(
                                "subgraph {} leaks from non-exit segment {segment_id}",
                                subgraph.id
                            ),
                        ));
                    }
                }
            }
            if self.segments.values().any(|segment| {
                segment.parent.as_deref() == Some(&subgraph.exit_segment)
                    && closure.contains(&segment.id)
            }) {
                return Err(TimelineError::at(
                    subgraph.line,
                    1,
                    format!(
                        "subgraph {} exit {} has a child inside the subgraph",
                        subgraph.id, subgraph.exit_segment
                    ),
                ));
            }
        }
        Ok(())
    }

    pub fn subgraph_segment_closure(&self, id: &str) -> BTreeSet<String> {
        let mut result = BTreeSet::new();
        let mut pending = vec![id];
        while let Some(current) = pending.pop() {
            let Some(subgraph) = self.subgraphs.get(current) else {
                continue;
            };
            result.extend(subgraph.segments.iter().cloned());
            pending.extend(
                self.subgraphs
                    .values()
                    .filter(|child| child.parent.as_deref() == Some(current))
                    .map(|child| child.id.as_str()),
            );
        }
        result
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

#[cfg(test)]
mod tests;
