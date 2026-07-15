//! Authored route timelines and immutable variant lineages.

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
    pub variants: BTreeMap<String, Variant>,
    pub proofs: Vec<GoalProof>,
    pub continuations: BTreeMap<String, Continuation>,
    pub branches: BTreeMap<String, Branch>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Origin {
    pub id: String,
    pub predicate: String,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Segment {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub profile: SegmentProfile,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Goal {
    pub id: String,
    pub segment: String,
    pub predicate: String,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Variant {
    pub id: String,
    pub segment: String,
    pub name: String,
    pub incumbent: bool,
    pub artifact: ArtifactSource,
    pub start_fingerprint: String,
    pub boundary_fingerprint: String,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct GoalProof {
    pub variant: String,
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
    pub parent_variant: String,
    pub checkpoint_fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContinuationStep {
    pub variant: String,
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
    pub after_variant: String,
    pub steps: Vec<ContinuationStep>,
    #[serde(skip)]
    line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct TimelineInspection {
    pub timeline: Timeline,
    pub incumbents: BTreeMap<String, String>,
    pub lineages: Vec<ResolvedLineage>,
    pub frontiers: Vec<VariantFrontier>,
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
pub struct VariantFrontier {
    pub segment: String,
    pub goal: String,
    pub start_fingerprint: String,
    pub boundary_fingerprint: String,
    pub variants: Vec<FrontierMember>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontierMember {
    pub variant: String,
    pub incumbent: bool,
    pub first_hit_tick: Option<u64>,
    pub relation_to_incumbent: DominanceRelation,
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
    pub incumbents: BTreeMap<String, String>,
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
    pub original_variant: String,
    pub workspace_variant: String,
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
        let incumbents = self.incumbents();
        let mut lineages = Vec::new();
        for name in self.continuations.keys() {
            lineages.push(self.resolve_lineage(name)?);
        }
        for name in self.branches.keys() {
            lineages.push(self.resolve_lineage(name)?);
        }
        Ok(TimelineInspection {
            timeline: self.clone(),
            incumbents,
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

    pub fn validate_artifacts(&self, root: Option<&Path>) -> Result<(), TimelineError> {
        self.validate_structure()?;
        let Some(root) = root else {
            return Ok(());
        };
        if let Some(compiled) = self.compile_predicates(root)? {
            let program_sha256 = compiled
                .program_sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            for proof in &self.proofs {
                let goal = &self.goals[&proof.goal];
                let actual = compiled
                    .definitions
                    .iter()
                    .find(|definition| definition.name == goal.predicate)
                    .expect("compile_predicates validates every referenced goal predicate");
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
                            "proof for variant {} and goal {} pins stale predicate program {}; current program is {}",
                            proof.variant,
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
                            "proof for variant {} and goal {} pins stale predicate {}; current predicate is {}",
                            proof.variant, proof.goal, proof.predicate_definition_sha256, actual
                        ),
                    ));
                }
            }
        } else if let Some(proof) = self.proofs.first() {
            return Err(TimelineError::at(
                proof.line,
                1,
                format!(
                    "proof for variant {} and goal {} has no predicate_program declaration",
                    proof.variant, proof.goal
                ),
            ));
        }
        for variant in self.variants.values() {
            let profile = self.segments[&variant.segment].profile;
            match &variant.artifact {
                ArtifactSource::Baseline(candidate_profile) => {
                    if *candidate_profile != profile {
                        return Err(TimelineError::at(
                            variant.line,
                            1,
                            format!(
                                "variant {} baseline profile {} does not match segment profile {}",
                                variant.id,
                                candidate_profile.as_str(),
                                profile.as_str()
                            ),
                        ));
                    }
                    Candidate::baseline(*candidate_profile)
                        .validate()
                        .map_err(|error| TimelineError::at(variant.line, 1, error.to_string()))?;
                }
                ArtifactSource::Candidate(path) => {
                    let path = root.join(path);
                    let candidate: Candidate =
                        serde_json::from_slice(&fs::read(&path).map_err(|error| {
                            TimelineError::at(
                                variant.line,
                                1,
                                format!("cannot read candidate {}: {error}", path.display()),
                            )
                        })?)
                        .map_err(|error| TimelineError::at(variant.line, 1, error.to_string()))?;
                    candidate
                        .validate()
                        .map_err(|error| TimelineError::at(variant.line, 1, error.to_string()))?;
                    if candidate.segment != profile {
                        return Err(TimelineError::at(
                            variant.line,
                            1,
                            format!("candidate {} has the wrong segment profile", path.display()),
                        ));
                    }
                }
                ArtifactSource::Tas(path) => {
                    let path = root.join(path);
                    let source = fs::read_to_string(&path).map_err(|error| {
                        TimelineError::at(
                            variant.line,
                            1,
                            format!("cannot read TAS program {}: {error}", path.display()),
                        )
                    })?;
                    tape_dsl::parse(&source)
                        .map_err(|error| TimelineError::at(variant.line, 1, error.to_string()))?
                        .compile()
                        .map_err(|error| TimelineError::at(variant.line, 1, error.to_string()))?;
                }
                ArtifactSource::Tape(path) => {
                    let path = root.join(path);
                    InputTape::decode(&fs::read(&path).map_err(|error| {
                        TimelineError::at(
                            variant.line,
                            1,
                            format!("cannot read tape {}: {error}", path.display()),
                        )
                    })?)
                    .map_err(|error| TimelineError::at(variant.line, 1, error.to_string()))?;
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
                    .map(|step| step.variant)
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
            incumbents: self.incumbents(),
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
        let mut previous_variant: Option<String> = None;
        let mut previous_fingerprint = resolved.root_fingerprint.clone();
        let mut chain_compatible = true;
        for (index, workspace_step) in workspace.steps.iter().enumerate() {
            let variant_id = &workspace_step.workspace_variant;
            let variant = &self.variants[variant_id];
            let compatible = chain_compatible && variant.start_fingerprint == previous_fingerprint;
            if index > 0 && workspace_step.state == WorkspaceStepState::Stale {
                if compatible {
                    compatible_descendants.push(variant_id.clone());
                } else {
                    stale_descendants.push(variant_id.clone());
                }
            }
            if !compatible {
                chain_compatible = false;
            }
            let parent_variant = previous_variant.clone().unwrap_or_else(|| "root".into());
            steps.push(ContinuationStep {
                variant: variant_id.clone(),
                after: DependencyPin {
                    parent_variant,
                    checkpoint_fingerprint: previous_fingerprint.clone(),
                },
                line: 0,
            });
            previous_variant = Some(variant_id.clone());
            previous_fingerprint = variant.boundary_fingerprint.clone();
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

    fn incumbents(&self) -> BTreeMap<String, String> {
        self.variants
            .values()
            .filter(|variant| variant.incumbent)
            .map(|variant| (variant.segment.clone(), variant.id.clone()))
            .collect()
    }

    fn frontiers(&self) -> Vec<VariantFrontier> {
        let proof_ticks = self
            .proofs
            .iter()
            .map(|proof| {
                (
                    (proof.variant.as_str(), proof.goal.as_str()),
                    proof.first_hit_tick,
                )
            })
            .collect::<HashMap<_, _>>();
        let mut output = Vec::new();
        for goal in self.goals.values() {
            let mut grouped: BTreeMap<(String, String), Vec<&Variant>> = BTreeMap::new();
            for variant in self
                .variants
                .values()
                .filter(|variant| variant.segment == goal.segment)
            {
                grouped
                    .entry((
                        variant.start_fingerprint.clone(),
                        variant.boundary_fingerprint.clone(),
                    ))
                    .or_default()
                    .push(variant);
            }
            for ((start_fingerprint, boundary_fingerprint), variants) in grouped {
                let incumbent = variants.iter().find(|variant| variant.incumbent).copied();
                let incumbent_tick = incumbent.and_then(|variant| {
                    proof_ticks
                        .get(&(variant.id.as_str(), goal.id.as_str()))
                        .copied()
                        .flatten()
                });
                let members = variants
                    .into_iter()
                    .map(|variant| {
                        let first_hit_tick = proof_ticks
                            .get(&(variant.id.as_str(), goal.id.as_str()))
                            .copied()
                            .flatten();
                        FrontierMember {
                            variant: variant.id.clone(),
                            incumbent: variant.incumbent,
                            first_hit_tick,
                            relation_to_incumbent: dominance(first_hit_tick, incumbent_tick),
                        }
                    })
                    .collect();
                output.push(VariantFrontier {
                    segment: goal.segment.clone(),
                    goal: goal.id.clone(),
                    start_fingerprint,
                    boundary_fingerprint,
                    variants: members,
                });
            }
        }
        output
    }

    fn workspace(
        &self,
        lineage: &str,
        selections: &BTreeMap<String, String>,
    ) -> Result<WorkspaceStatus, TimelineError> {
        let resolved = self.resolve_lineage(lineage)?;
        let lineage_segments: HashSet<_> = resolved
            .steps
            .iter()
            .map(|step| self.variants[&step.variant].segment.as_str())
            .collect();
        for (segment, variant_id) in selections {
            if !lineage_segments.contains(segment.as_str()) {
                return Err(TimelineError::new(format!(
                    "selection segment {segment:?} is not in lineage {lineage:?}"
                )));
            }
            let variant = self.variants.get(variant_id).ok_or_else(|| {
                TimelineError::new(format!(
                    "selection references unknown variant {variant_id:?}"
                ))
            })?;
            if variant.segment != *segment {
                return Err(TimelineError::new(format!(
                    "variant {variant_id} belongs to segment {}, not {segment}",
                    variant.segment
                )));
            }
        }
        let mut changed_upstream = false;
        let mut chain_compatible = true;
        let mut expected_start = resolved.root_fingerprint.clone();
        let mut steps = Vec::new();
        let mut stale_descendants = Vec::new();
        for original in &resolved.steps {
            let original_variant = &self.variants[&original.variant];
            let selected = selections
                .get(&original_variant.segment)
                .cloned()
                .unwrap_or_else(|| original.variant.clone());
            let workspace_variant = &self.variants[&selected];
            let selected_changed = selected != original.variant;
            let state = if selected_changed {
                changed_upstream = true;
                WorkspaceStepState::Selected
            } else if changed_upstream {
                stale_descendants.push(selected.clone());
                WorkspaceStepState::Stale
            } else {
                WorkspaceStepState::Unchanged
            };
            let compatible =
                chain_compatible && workspace_variant.start_fingerprint == expected_start;
            if !compatible {
                chain_compatible = false;
            }
            steps.push(WorkspaceStep {
                original_variant: original.variant.clone(),
                workspace_variant: selected,
                state,
                rebase_compatible: compatible,
                expected_start_fingerprint: expected_start.clone(),
                actual_start_fingerprint: workspace_variant.start_fingerprint.clone(),
            });
            expected_start = workspace_variant.boundary_fingerprint.clone();
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
            reached = step.variant == branch.after_variant;
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
                    "branch {} fork variant {} is not reached by {}",
                    branch.name, branch.after_variant, branch.from_lineage
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

fn dominance(first_hit_tick: Option<u64>, incumbent_tick: Option<u64>) -> DominanceRelation {
    let Some(incumbent_tick) = incumbent_tick else {
        return DominanceRelation::Incomparable;
    };
    match first_hit_tick {
        Some(left) if left < incumbent_tick => DominanceRelation::Faster,
        Some(left) if left > incumbent_tick => DominanceRelation::Slower,
        Some(_) => DominanceRelation::Tied,
        None => DominanceRelation::Unscored,
    }
}

fn render_continuation(name: &str, root_fingerprint: &str, steps: &[ContinuationStep]) -> String {
    let mut output = format!("continuation {name} starts root@{root_fingerprint}\n");
    for step in steps {
        output.push_str(&format!(
            "continue {name} with {} after {}@{}\n",
            step.variant, step.after.parent_variant, step.after.checkpoint_fingerprint
        ));
    }
    output
}

impl Timeline {
    fn validate_structure(&self) -> Result<(), TimelineError> {
        if self.name.is_empty() {
            return Err(TimelineError::new("timeline name is empty"));
        }
        if (self.origin.is_some() || !self.goals.is_empty()) && self.predicate_program.is_none() {
            return Err(TimelineError::new(
                "origin and goal declarations require predicate_program",
            ));
        }
        for segment in self.segments.values() {
            if segment.parent.as_deref() == Some(segment.name.as_str()) {
                return Err(TimelineError::at(
                    segment.line,
                    1,
                    format!("segment {} cannot be its own parent", segment.name),
                ));
            }
            if let Some(parent) = &segment.parent
                && !self.segments.contains_key(parent)
            {
                return Err(TimelineError::at(
                    segment.line,
                    1,
                    format!(
                        "segment {} references unknown parent {parent}",
                        segment.name
                    ),
                ));
            }
        }
        self.validate_segment_forest()?;

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

        let mut incumbent_counts: HashMap<&str, usize> = HashMap::new();
        for variant in self.variants.values() {
            if !self.segments.contains_key(&variant.segment) {
                return Err(TimelineError::at(
                    variant.line,
                    1,
                    format!(
                        "variant {} references unknown segment {}",
                        variant.id, variant.segment
                    ),
                ));
            }
            if variant.start_fingerprint.is_empty() || variant.boundary_fingerprint.is_empty() {
                return Err(TimelineError::at(
                    variant.line,
                    1,
                    format!("variant {} has an empty boundary fingerprint", variant.id),
                ));
            }
            if variant.incumbent {
                *incumbent_counts.entry(&variant.segment).or_default() += 1;
            }
        }
        let mut proof_pairs = HashSet::new();
        for proof in &self.proofs {
            let variant = self.variants.get(&proof.variant).ok_or_else(|| {
                TimelineError::at(
                    proof.line,
                    1,
                    format!("proof references unknown variant {}", proof.variant),
                )
            })?;
            let goal = self.goals.get(&proof.goal).ok_or_else(|| {
                TimelineError::at(
                    proof.line,
                    1,
                    format!("proof references unknown goal {}", proof.goal),
                )
            })?;
            if goal.segment != variant.segment {
                return Err(TimelineError::at(
                    proof.line,
                    1,
                    format!(
                        "proof goal {} belongs to segment {}, but variant {} belongs to {}",
                        goal.id, goal.segment, variant.id, variant.segment
                    ),
                ));
            }
            if !proof_pairs.insert((proof.variant.as_str(), proof.goal.as_str())) {
                return Err(TimelineError::at(
                    proof.line,
                    1,
                    format!(
                        "duplicate proof for variant {} and goal {}",
                        proof.variant, proof.goal
                    ),
                ));
            }
        }
        for segment in self.segments.keys() {
            let count = incumbent_counts.get(segment.as_str()).copied().unwrap_or(0);
            if count != 1 {
                return Err(TimelineError::new(format!(
                    "segment {segment} must have exactly one incumbent variant; found {count}"
                )));
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
            if !self.variants.contains_key(&branch.after_variant) {
                return Err(TimelineError::at(
                    branch.line,
                    1,
                    format!(
                        "branch {} references unknown fork variant {}",
                        branch.name, branch.after_variant
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
        let mut prior_variant: Option<&Variant> = None;
        let mut expected_fingerprint = root_fingerprint;
        for step in steps {
            let variant = self.variants.get(&step.variant).ok_or_else(|| {
                TimelineError::at(
                    step.line,
                    1,
                    format!("{name} references unknown variant {}", step.variant),
                )
            })?;
            let expected_parent = prior_variant.map_or("root", |prior| prior.id.as_str());
            if step.after.parent_variant != expected_parent {
                return Err(TimelineError::at(
                    step.line,
                    1,
                    format!(
                        "{name} step {} pins parent {}, expected exact parent {}",
                        variant.id, step.after.parent_variant, expected_parent
                    ),
                ));
            }
            if step.after.checkpoint_fingerprint != expected_fingerprint
                || variant.start_fingerprint != expected_fingerprint
            {
                return Err(TimelineError::at(
                    step.line,
                    1,
                    format!(
                        "{name} step {} boundary mismatch: pin={}, variant starts={}, expected={}",
                        variant.id,
                        step.after.checkpoint_fingerprint,
                        variant.start_fingerprint,
                        expected_fingerprint
                    ),
                ));
            }
            if let Some(prior) = prior_variant {
                let segment = &self.segments[&variant.segment];
                if segment.parent.as_deref() != Some(prior.segment.as_str()) {
                    return Err(TimelineError::at(
                        step.line,
                        1,
                        format!(
                            "{name} is discontinuous: segment {} is not a child of {}",
                            segment.name, prior.segment
                        ),
                    ));
                }
            } else if self.segments[&variant.segment].parent.is_some() {
                return Err(TimelineError::at(
                    step.line,
                    1,
                    format!("{name} starts with non-root segment {}", variant.segment),
                ));
            }
            expected_fingerprint = &variant.boundary_fingerprint;
            prior_variant = Some(variant);
        }
        Ok(())
    }

    fn validate_segment_forest(&self) -> Result<(), TimelineError> {
        for segment in self.segments.values() {
            let mut seen = HashSet::new();
            let mut current = Some(segment.name.as_str());
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
    goals: BTreeMap<String, Goal>,
    variants: BTreeMap<String, Variant>,
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
            goals: BTreeMap::new(),
            variants: BTreeMap::new(),
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
                "goal" => self.parse_goal(&tokens, line_number)?,
                "variant" => self.parse_variant(&tokens, line_number)?,
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
        let timeline = Timeline {
            name: self
                .timeline_name
                .ok_or_else(|| TimelineError::new("missing timeline declaration"))?,
            predicate_program: self.predicate_program,
            origin: self.origin,
            segments: self.segments,
            goals: self.goals,
            variants: self.variants,
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
        let path = PathBuf::from(&tokens[1]);
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(TimelineError::at(
                line,
                1,
                "predicate program must be a contained relative path",
            ));
        }
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
        exact_len(tokens, 4, line, "origin boot predicate PREDICATE")?;
        if tokens[1] != "boot" {
            return Err(TimelineError::at(
                line,
                1,
                "the only supported origin is boot",
            ));
        }
        expect(tokens, 2, "predicate", line)?;
        let origin = Origin {
            id: tokens[1].clone(),
            predicate: tokens[3].clone(),
            line,
        };
        if self.origin.replace(origin).is_some() {
            return Err(TimelineError::at(line, 1, "duplicate origin declaration"));
        }
        Ok(())
    }

    fn parse_segment(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        let (parent, profile_index) = match tokens.get(2).map(String::as_str) {
            Some("root") => {
                exact_len(tokens, 5, line, "segment NAME root profile PROFILE")?;
                (None, 4)
            }
            Some("after") => {
                exact_len(
                    tokens,
                    6,
                    line,
                    "segment NAME after PARENT_SEGMENT profile PROFILE",
                )?;
                (Some(tokens[3].clone()), 5)
            }
            _ => {
                return Err(TimelineError::at(
                    line,
                    1,
                    "expected segment NAME root profile PROFILE or segment NAME after PARENT_SEGMENT profile PROFILE",
                ));
            }
        };
        expect(tokens, profile_index - 1, "profile", line)?;
        let name = tokens[1].clone();
        let segment = Segment {
            name: name.clone(),
            parent,
            profile: tokens[profile_index].parse().map_err(
                |error: crate::search::SearchError| TimelineError::at(line, 1, error.to_string()),
            )?,
            line,
        };
        if self.segments.insert(name.clone(), segment).is_some() {
            return Err(TimelineError::at(
                line,
                1,
                format!("duplicate segment {name}"),
            ));
        }
        Ok(())
    }

    fn parse_goal(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        exact_len(
            tokens,
            6,
            line,
            "goal GOAL_ID on SEGMENT predicate PREDICATE",
        )?;
        expect(tokens, 2, "on", line)?;
        expect(tokens, 4, "predicate", line)?;
        let id = tokens[1].clone();
        let goal = Goal {
            id: id.clone(),
            segment: tokens[3].clone(),
            predicate: tokens[5].clone(),
            line,
        };
        if self.goals.insert(id.clone(), goal).is_some() {
            return Err(TimelineError::at(line, 1, format!("duplicate goal {id}")));
        }
        Ok(())
    }

    fn parse_variant(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        if tokens.len() < 9 {
            return Err(TimelineError::at(
                line,
                1,
                "expected variant SEGMENT.NAME [incumbent] uses KIND VALUE starts FINGERPRINT produces FINGERPRINT",
            ));
        }
        let id = tokens[1].clone();
        let (segment, name) = split_variant_id(&id, line)?;
        let mut cursor = 2;
        let incumbent = tokens.get(cursor).is_some_and(|token| token == "incumbent");
        if incumbent {
            cursor += 1;
        }
        expect(tokens, cursor, "uses", line)?;
        let kind = tokens
            .get(cursor + 1)
            .ok_or_else(|| TimelineError::at(line, 1, "missing artifact kind"))?;
        let value = tokens
            .get(cursor + 2)
            .ok_or_else(|| TimelineError::at(line, 1, "missing artifact value"))?;
        let artifact = match kind.as_str() {
            "baseline" => ArtifactSource::Baseline(value.parse().map_err(
                |error: crate::search::SearchError| TimelineError::at(line, 1, error.to_string()),
            )?),
            "candidate" => ArtifactSource::Candidate(PathBuf::from(value)),
            "tas" => ArtifactSource::Tas(PathBuf::from(value)),
            "tape" => ArtifactSource::Tape(PathBuf::from(value)),
            _ => {
                return Err(TimelineError::at(
                    line,
                    1,
                    format!("unknown variant artifact kind {kind:?}"),
                ));
            }
        };
        cursor += 3;
        expect(tokens, cursor, "starts", line)?;
        let start_fingerprint = required_token(tokens, cursor + 1, line, "start fingerprint")?;
        cursor += 2;
        expect(tokens, cursor, "produces", line)?;
        let boundary_fingerprint =
            required_token(tokens, cursor + 1, line, "boundary fingerprint")?;
        cursor += 2;
        if cursor != tokens.len() {
            return Err(TimelineError::at(
                line,
                1,
                format!("unexpected variant token {:?}", tokens[cursor]),
            ));
        }
        let variant = Variant {
            id: id.clone(),
            segment,
            name,
            incumbent,
            artifact,
            start_fingerprint,
            boundary_fingerprint,
            line,
        };
        if self.variants.insert(id.clone(), variant).is_some() {
            return Err(TimelineError::at(
                line,
                1,
                format!("duplicate variant {id}"),
            ));
        }
        Ok(())
    }

    fn parse_proof(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        if tokens.len() != 8 && tokens.len() != 10 {
            return Err(TimelineError::at(
                line,
                1,
                "expected proof VARIANT satisfies GOAL program SHA256 predicate SHA256 [ticks N]",
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
            variant: tokens[1].clone(),
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
        if pin.parent_variant != "root" {
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
        exact_len(tokens, 6, line, "branch NAME from LINEAGE after VARIANT_ID")?;
        expect(tokens, 2, "from", line)?;
        expect(tokens, 4, "after", line)?;
        let name = tokens[1].clone();
        let branch = Branch {
            name: name.clone(),
            from_lineage: tokens[3].clone(),
            after_variant: tokens[5].clone(),
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
            "continue LINEAGE with SEGMENT.VARIANT after PARENT@FINGERPRINT",
        )?;
        expect(tokens, 2, "with", line)?;
        expect(tokens, 4, "after", line)?;
        let lineage = &tokens[1];
        let step = ContinuationStep {
            variant: tokens[3].clone(),
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

fn split_variant_id(id: &str, line: usize) -> Result<(String, String), TimelineError> {
    let (segment, name) = id.rsplit_once('.').ok_or_else(|| {
        TimelineError::at(line, 1, format!("variant ID {id:?} must be SEGMENT.NAME"))
    })?;
    if segment.is_empty() || name.is_empty() {
        return Err(TimelineError::at(
            line,
            1,
            format!("invalid variant ID {id:?}"),
        ));
    }
    Ok((segment.into(), name.into()))
}

fn parse_pin(token: &str, line: usize) -> Result<DependencyPin, TimelineError> {
    let (parent_variant, checkpoint_fingerprint) = token.rsplit_once('@').ok_or_else(|| {
        TimelineError::at(
            line,
            1,
            format!("dependency pin {token:?} must be PARENT@FINGERPRINT"),
        )
    })?;
    if parent_variant.is_empty() || checkpoint_fingerprint.is_empty() {
        return Err(TimelineError::at(
            line,
            1,
            format!("invalid dependency pin {token:?}"),
        ));
    }
    Ok(DependencyPin {
        parent_variant: parent_variant.into(),
        checkpoint_fingerprint: checkpoint_fingerprint.into(),
    })
}

fn tokenize(line: &str, line_number: usize) -> Result<Vec<String>, TimelineError> {
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
segment boot_link root profile boot_to_fsp103
segment exit after boot_link profile fsp103_to_fsp104
goal link_control on boot_link predicate link_control
goal next_map on exit predicate next_map
variant boot_link.safe incumbent uses baseline boot_to_fsp103 starts clean-rng1 produces control-rng1
variant boot_link.fast uses baseline boot_to_fsp103 starts clean-rng1 produces control-rng1
variant boot_link.other_rng uses baseline boot_to_fsp103 starts clean-rng2 produces control-rng2
variant exit.safe incumbent uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1
variant exit.rolls uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1
proof boot_link.safe satisfies link_control program 2222222222222222222222222222222222222222222222222222222222222222 predicate 1111111111111111111111111111111111111111111111111111111111111111 ticks 700
proof boot_link.fast satisfies link_control program 2222222222222222222222222222222222222222222222222222222222222222 predicate 1111111111111111111111111111111111111111111111111111111111111111 ticks 650
continuation main starts root@clean-rng1
continue main with boot_link.safe after root@clean-rng1
continue main with exit.safe after boot_link.safe@control-rng1
branch rolls from main after boot_link.safe
continue rolls with exit.rolls after boot_link.safe@control-rng1
"#;

    #[test]
    fn parses_variants_continuations_branches_and_frontiers() {
        let timeline = Timeline::parse(ROUTE).unwrap();
        let inspection = timeline.inspect().unwrap();
        assert_eq!(inspection.timeline.name, "intro");
        assert_eq!(inspection.lineages.len(), 2);
        assert_eq!(inspection.incumbents["boot_link"], "boot_link.safe");
        let rolls = inspection
            .lineages
            .iter()
            .find(|lineage| lineage.name == "rolls")
            .unwrap();
        assert_eq!(rolls.steps[0].variant, "boot_link.safe");
        assert_eq!(rolls.steps[1].variant, "exit.rolls");
        let frontier = inspection
            .frontiers
            .iter()
            .find(|frontier| {
                frontier.segment == "boot_link"
                    && frontier.goal == "link_control"
                    && frontier.variants.len() == 2
            })
            .unwrap();
        assert_eq!(
            frontier
                .variants
                .iter()
                .find(|member| member.variant == "boot_link.fast")
                .unwrap()
                .relation_to_incumbent,
            DominanceRelation::Faster
        );
        assert!(inspection.frontiers.iter().any(|frontier| {
            frontier.segment == "boot_link"
                && frontier.start_fingerprint == "clean-rng2"
                && frontier.variants[0].relation_to_incumbent == DominanceRelation::Incomparable
        }));
    }

    #[test]
    fn goal_proof_is_separate_from_variant_and_scoped_to_its_segment() {
        let program_digest = "2".repeat(64);
        let digest = "1".repeat(64);
        let timeline = Timeline::parse(ROUTE).unwrap();
        let proof = &timeline.proofs[0];
        assert_eq!(proof.variant, "boot_link.safe");
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

        let wrong_segment = ROUTE.replace("satisfies link_control", "satisfies next_map");
        assert!(
            Timeline::parse(&wrong_segment)
                .unwrap_err()
                .to_string()
                .contains("belongs to segment exit")
        );

        let duplicate = format!(
            "{ROUTE}\nproof boot_link.safe satisfies link_control program {} predicate {}",
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
    fn incumbent_change_does_not_stale_immutable_lineage() {
        let mut source = ROUTE.replace("boot_link.safe incumbent", "boot_link.safe");
        source = source.replace("boot_link.fast uses", "boot_link.fast incumbent uses");
        let timeline = Timeline::parse(&source).unwrap();
        let status = timeline.status(None, &BTreeMap::new()).unwrap();
        assert!(
            status
                .immutable_lineages
                .iter()
                .all(|lineage| !lineage.stale)
        );
        assert_eq!(status.immutable_lineages[0].steps[0], "boot_link.safe");
    }

    #[test]
    fn workspace_marks_descendants_stale_and_rebase_creates_new_lineage() {
        let timeline = Timeline::parse(ROUTE).unwrap();
        let selections = BTreeMap::from([("boot_link".into(), "boot_link.fast".into())]);
        let status = timeline.status(Some("main"), &selections).unwrap();
        let workspace = status.workspace.unwrap();
        assert_eq!(workspace.steps[0].state, WorkspaceStepState::Selected);
        assert_eq!(workspace.steps[1].state, WorkspaceStepState::Stale);
        assert!(workspace.steps[1].rebase_compatible);

        let repair = timeline
            .rebase_compatible("main", &selections, "main_fast")
            .unwrap();
        assert!(repair.old_lineage_preserved);
        assert!(repair.fully_compatible);
        assert_eq!(repair.compatible_descendants, vec!["exit.safe"]);
        assert!(
            repair
                .authored_dsl
                .contains("continue main_fast with exit.safe after boot_link.fast@control-rng1")
        );
    }

    #[test]
    fn rejects_cycles_bad_references_and_boundary_mismatches_with_lines() {
        let cycle = ROUTE.replace(
            "segment boot_link root profile boot_to_fsp103",
            "segment boot_link after exit profile boot_to_fsp103",
        );
        assert!(
            Timeline::parse(&cycle)
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );

        let unknown = ROUTE.replace("with exit.safe", "with exit.missing");
        let error = Timeline::parse(&unknown).unwrap_err();
        assert!(error.to_string().contains("unknown variant"));
        assert!(error.line.is_some());

        let mismatch = ROUTE.replace(
            "after boot_link.safe@control-rng1",
            "after boot_link.safe@wrong",
        );
        assert!(
            Timeline::parse(&mismatch)
                .unwrap_err()
                .to_string()
                .contains("boundary mismatch")
        );

        let off_lineage = ROUTE.replace(
            "branch rolls from main after boot_link.safe",
            "branch rolls from main after boot_link.other_rng",
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
segment boot root profile boot_to_fsp103
goal control on boot predicate link_control
variant boot.test incumbent uses baseline boot_to_fsp103 starts clean produces control
continuation main starts root@clean
continue main with boot.test after root@clean
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
segment boot root profile boot_to_fsp103
goal control on boot predicate link_control
variant boot.test incumbent uses baseline boot_to_fsp103 starts clean produces control
proof boot.test satisfies control program {program_digest} predicate {predicate_digest}
continuation main starts root@clean
continue main with boot.test after root@clean
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
                .contains("stale predicate program")
        );

        let missing = Timeline::parse(
            r#"timeline route
predicate_program route.milestones
segment boot root profile boot_to_fsp103
goal control on boot predicate not_defined
variant boot.test incumbent uses baseline boot_to_fsp103 starts clean produces control
continuation main starts root@clean
continue main with boot.test after root@clean
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
segment boot_link root profile boot_to_fsp103
variant boot_link.tas incumbent uses tas boot.tas starts clean produces control
continuation main starts root@clean
continue main with boot_link.tas after root@clean
"#,
        )
        .unwrap();
        timeline.validate_artifacts(Some(&root)).unwrap();
        let _ = fs::remove_dir_all(root);
    }
}
