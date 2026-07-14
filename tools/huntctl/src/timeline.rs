//! Authored route timelines and immutable variant lineages.

use crate::search::{Candidate, SegmentProfile};
use crate::tape::InputTape;
use crate::tape_dsl;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct Timeline {
    pub name: String,
    pub milestones: Vec<String>,
    pub segments: BTreeMap<String, Segment>,
    pub variants: BTreeMap<String, Variant>,
    pub continuations: BTreeMap<String, Continuation>,
    pub branches: BTreeMap<String, Branch>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Segment {
    pub name: String,
    pub from: String,
    pub to: String,
    pub profile: SegmentProfile,
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
    pub from_continuation: String,
    pub at_milestone: String,
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

    pub fn validate_artifacts(&self, root: Option<&Path>) -> Result<(), TimelineError> {
        self.validate_structure()?;
        let Some(root) = root else {
            return Ok(());
        };
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
        let mut grouped: BTreeMap<(String, String, String), Vec<&Variant>> = BTreeMap::new();
        for variant in self.variants.values() {
            grouped
                .entry((
                    variant.segment.clone(),
                    variant.start_fingerprint.clone(),
                    variant.boundary_fingerprint.clone(),
                ))
                .or_default()
                .push(variant);
        }
        grouped
            .into_iter()
            .map(
                |((segment, start_fingerprint, boundary_fingerprint), variants)| {
                    let incumbent = variants.iter().find(|variant| variant.incumbent).copied();
                    let members = variants
                        .into_iter()
                        .map(|variant| FrontierMember {
                            variant: variant.id.clone(),
                            incumbent: variant.incumbent,
                            first_hit_tick: variant.first_hit_tick,
                            relation_to_incumbent: dominance(variant, incumbent),
                        })
                        .collect();
                    VariantFrontier {
                        segment,
                        start_fingerprint,
                        boundary_fingerprint,
                        variants: members,
                    }
                },
            )
            .collect()
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
        let base = self.resolve_lineage(&branch.from_continuation)?;
        let mut prefix = Vec::new();
        let mut reached = false;
        for step in base.steps {
            let variant = &self.variants[&step.variant];
            prefix.push(step);
            if self.segments[&variant.segment].to == branch.at_milestone {
                reached = true;
                break;
            }
        }
        if !reached {
            return Err(TimelineError::at(
                branch.line,
                1,
                format!(
                    "branch {} milestone {} is not reached by {}",
                    branch.name, branch.at_milestone, branch.from_continuation
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

fn dominance(variant: &Variant, incumbent: Option<&Variant>) -> DominanceRelation {
    let Some(incumbent) = incumbent else {
        return DominanceRelation::Incomparable;
    };
    if variant.start_fingerprint != incumbent.start_fingerprint
        || variant.boundary_fingerprint != incumbent.boundary_fingerprint
    {
        return DominanceRelation::Incomparable;
    }
    match (variant.first_hit_tick, incumbent.first_hit_tick) {
        (Some(left), Some(right)) if left < right => DominanceRelation::Faster,
        (Some(left), Some(right)) if left > right => DominanceRelation::Slower,
        (Some(_), Some(_)) => DominanceRelation::Tied,
        _ => DominanceRelation::Unscored,
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
        let milestones: HashSet<_> = self.milestones.iter().map(String::as_str).collect();
        for segment in self.segments.values() {
            if !milestones.contains(segment.from.as_str())
                || !milestones.contains(segment.to.as_str())
            {
                return Err(TimelineError::at(
                    segment.line,
                    1,
                    format!(
                        "segment {} references unknown milestone {} or {}",
                        segment.name, segment.from, segment.to
                    ),
                ));
            }
            if segment.from == segment.to {
                return Err(TimelineError::at(
                    segment.line,
                    1,
                    format!(
                        "segment {} starts and ends at the same milestone",
                        segment.name
                    ),
                ));
            }
        }
        self.validate_segment_dag()?;

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
            if !milestones.contains(branch.at_milestone.as_str()) {
                return Err(TimelineError::at(
                    branch.line,
                    1,
                    format!(
                        "branch {} references unknown milestone {}",
                        branch.name, branch.at_milestone
                    ),
                ));
            }
            if !self.continuations.contains_key(&branch.from_continuation)
                && !self.branches.contains_key(&branch.from_continuation)
            {
                return Err(TimelineError::at(
                    branch.line,
                    1,
                    format!(
                        "branch {} references unknown lineage {}",
                        branch.name, branch.from_continuation
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
                let prior_segment = &self.segments[&prior.segment];
                let segment = &self.segments[&variant.segment];
                if prior_segment.to != segment.from {
                    return Err(TimelineError::at(
                        step.line,
                        1,
                        format!(
                            "{name} is discontinuous: {} ends at {}, {} starts at {}",
                            prior.id, prior_segment.to, variant.id, segment.from
                        ),
                    ));
                }
            }
            expected_fingerprint = &variant.boundary_fingerprint;
            prior_variant = Some(variant);
        }
        Ok(())
    }

    fn validate_segment_dag(&self) -> Result<(), TimelineError> {
        let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut indegree: HashMap<&str, usize> = self
            .milestones
            .iter()
            .map(|milestone| (milestone.as_str(), 0))
            .collect();
        for segment in self.segments.values() {
            outgoing
                .entry(segment.from.as_str())
                .or_default()
                .push(segment.to.as_str());
            *indegree.entry(segment.to.as_str()).or_default() += 1;
        }
        let mut ready: Vec<_> = indegree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(node, _)| *node)
            .collect();
        let mut visited = 0;
        while let Some(node) = ready.pop() {
            visited += 1;
            if let Some(next) = outgoing.get(node) {
                for destination in next {
                    let degree = indegree.get_mut(destination).unwrap();
                    *degree -= 1;
                    if *degree == 0 {
                        ready.push(destination);
                    }
                }
            }
        }
        if visited != indegree.len() {
            return Err(TimelineError::new(
                "timeline segment graph contains a cycle",
            ));
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
                current = &branch.from_continuation;
            }
        }
        Ok(())
    }
}

struct Parser<'a> {
    source: &'a str,
    timeline_name: Option<String>,
    milestones: Vec<String>,
    segments: BTreeMap<String, Segment>,
    variants: BTreeMap<String, Variant>,
    continuations: BTreeMap<String, Continuation>,
    branches: BTreeMap<String, Branch>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            timeline_name: None,
            milestones: Vec::new(),
            segments: BTreeMap::new(),
            variants: BTreeMap::new(),
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
                "milestone" => self.parse_milestone(&tokens, line_number)?,
                "segment" => self.parse_segment(&tokens, line_number)?,
                "variant" => self.parse_variant(&tokens, line_number)?,
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
            milestones: self.milestones,
            segments: self.segments,
            variants: self.variants,
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

    fn parse_milestone(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        exact_len(tokens, 2, line, "milestone NAME")?;
        if self.milestones.contains(&tokens[1]) {
            return Err(TimelineError::at(
                line,
                1,
                format!("duplicate milestone {}", tokens[1]),
            ));
        }
        self.milestones.push(tokens[1].clone());
        Ok(())
    }

    fn parse_segment(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        exact_len(
            tokens,
            8,
            line,
            "segment NAME from MILESTONE to MILESTONE profile PROFILE",
        )?;
        expect(tokens, 2, "from", line)?;
        expect(tokens, 4, "to", line)?;
        expect(tokens, 6, "profile", line)?;
        let name = tokens[1].clone();
        let segment = Segment {
            name: name.clone(),
            from: tokens[3].clone(),
            to: tokens[5].clone(),
            profile: tokens[7]
                .parse()
                .map_err(|error: crate::search::SearchError| {
                    TimelineError::at(line, 1, error.to_string())
                })?,
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

    fn parse_variant(&mut self, tokens: &[String], line: usize) -> Result<(), TimelineError> {
        if tokens.len() < 9 {
            return Err(TimelineError::at(
                line,
                1,
                "expected variant SEGMENT.NAME [incumbent] uses KIND VALUE starts FINGERPRINT produces FINGERPRINT [ticks N]",
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
        let first_hit_tick = if cursor < tokens.len() {
            expect(tokens, cursor, "ticks", line)?;
            let value = required_token(tokens, cursor + 1, line, "tick count")?;
            cursor += 2;
            Some(value.parse().map_err(|_| {
                TimelineError::at(line, 1, format!("invalid first-hit tick {value:?}"))
            })?)
        } else {
            None
        };
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
            first_hit_tick,
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
        exact_len(
            tokens,
            6,
            line,
            "branch NAME from CONTINUATION at MILESTONE",
        )?;
        expect(tokens, 2, "from", line)?;
        expect(tokens, 4, "at", line)?;
        let name = tokens[1].clone();
        let branch = Branch {
            name: name.clone(),
            from_continuation: tokens[3].clone(),
            at_milestone: tokens[5].clone(),
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
milestone boot
milestone control
milestone next_map
segment boot_link from boot to control profile boot_to_fsp103
segment exit from control to next_map profile fsp103_to_fsp104
variant boot_link.safe incumbent uses baseline boot_to_fsp103 starts clean-rng1 produces control-rng1 ticks 700
variant boot_link.fast uses baseline boot_to_fsp103 starts clean-rng1 produces control-rng1 ticks 650
variant boot_link.other_rng uses baseline boot_to_fsp103 starts clean-rng2 produces control-rng2 ticks 600
variant exit.safe incumbent uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1 ticks 603
variant exit.rolls uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1 ticks 580
continuation main starts root@clean-rng1
continue main with boot_link.safe after root@clean-rng1
continue main with exit.safe after boot_link.safe@control-rng1
branch rolls from main at control
continue rolls with exit.rolls after boot_link.safe@control-rng1
"#;

    #[test]
    fn parses_variants_continuations_branches_and_frontiers() {
        let timeline = Timeline::parse(ROUTE).unwrap();
        let inspection = timeline.inspect().unwrap();
        assert_eq!(inspection.timeline.name, "intro");
        assert_eq!(inspection.lineages.len(), 2);
        assert_eq!(inspection.incumbents["boot_link"], "boot_link.safe");
        let frontier = inspection
            .frontiers
            .iter()
            .find(|frontier| frontier.segment == "boot_link" && frontier.variants.len() == 2)
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
            "segment exit from control to next_map",
            "segment exit from control to boot",
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
    }

    #[test]
    fn parser_reports_quoted_token_diagnostics() {
        let error = Timeline::parse("timeline \"unterminated").unwrap_err();
        assert_eq!(error.line, Some(1));
        assert!(error.to_string().contains("unterminated"));
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
milestone boot
milestone control
segment boot_link from boot to control profile boot_to_fsp103
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
