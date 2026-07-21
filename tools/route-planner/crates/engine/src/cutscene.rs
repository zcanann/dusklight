//! Planner-owned phase programs for cutscenes and exceptional load branches.
//!
//! A program compiles to ordinary candidate transitions. It does not grant a
//! special warp: confirmed operations execute in order, uncertain suffix
//! targets are invalidated, and any later savewarp still reads the retained
//! return-place component through the normal mechanics catalog.

use crate::artifact::Digest;
use crate::logic::{
    ComparisonOperator, ContextScope, PredicateExpression, RuleEvidence, ValueReference,
};
use crate::state::{ComponentSelector, StateValue};
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, StateOperation, TransitionKind,
    UnknownRequirement,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const CUTSCENE_PROGRAM_SCHEMA: &str = "dusklight.route-planner.cutscene-program/v6";
pub const COMPILED_CUTSCENE_PROGRAM_SCHEMA: &str =
    "dusklight.route-planner.compiled-cutscene-program/v6";
const MAX_CUTSCENE_PHASES: usize = 4096;
const MAX_PHASE_BRANCHES: usize = 256;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneProgram {
    pub schema: String,
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub flow_component_id: String,
    pub entry_phase_id: String,
    pub phases: Vec<CutscenePhase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledCutsceneProgram {
    pub schema: String,
    pub program_sha256: Digest,
    pub transitions: Vec<CandidateTransition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutscenePhase {
    pub id: String,
    pub label: String,
    pub resource_requests: Vec<CutsceneResourceRequest>,
    pub branches: Vec<CutsceneBranch>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneResourceRequest {
    /// Structured field on the flow component holding the requested archive ID.
    pub archive_field: String,
    pub archive_id: String,
    /// Structured field holding a build/source-specific result such as
    /// `pending`, `loaded`, `failed`, or an explicitly unknown value.
    pub result_field: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutsceneBranchKind {
    Advance,
    NormalCompletion,
    IntentionalSkip,
    Interruption,
    SceneChange,
    ResourceLoadFailure,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneBranch {
    pub transition_id: String,
    pub label: String,
    pub kind: CutsceneBranchKind,
    pub approach_id: String,
    pub hard_guards: PredicateExpression,
    pub physical_obligation_ids: Vec<String>,
    /// Writes proven to execute on this branch, in execution order.
    pub confirmed_operations: Vec<StateOperation>,
    /// Possible suffix writes whose execution/value is not established.
    pub uncertain_effects: Vec<UncertainCutsceneEffect>,
    pub unknown_requirements: Vec<UnknownRequirement>,
    pub destination_phase_id: Option<String>,
    pub terminal: bool,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UncertainCutsceneEffect {
    StructuredField {
        target: ComponentFieldTarget,
    },
    RawBits {
        component_id: String,
        byte_offset: u32,
        mask: Vec<u8>,
    },
}

impl CutsceneProgram {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != CUTSCENE_PROGRAM_SCHEMA {
            return Err(PlannerContractError::new(
                "cutscene_program.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("cutscene_program.id", &self.id)?;
        validate_label("cutscene_program.label", &self.label)?;
        self.scope.validate("cutscene_program.scope")?;
        validate_stable_id(
            "cutscene_program.flow_component_id",
            &self.flow_component_id,
        )?;
        validate_stable_id("cutscene_program.entry_phase_id", &self.entry_phase_id)?;
        if self.phases.is_empty() || self.phases.len() > MAX_CUTSCENE_PHASES {
            return Err(PlannerContractError::new(
                "cutscene_program.phases",
                format!("must contain between 1 and {MAX_CUTSCENE_PHASES} phases"),
            ));
        }
        let phase_ids = self
            .phases
            .iter()
            .map(|phase| phase.id.as_str())
            .collect::<BTreeSet<_>>();
        if phase_ids.len() != self.phases.len() || !phase_ids.contains(self.entry_phase_id.as_str())
        {
            return Err(PlannerContractError::new(
                "cutscene_program.phases",
                "must have unique IDs and contain the entry phase",
            ));
        }
        let mut prior_phase = None;
        let mut transition_ids = BTreeSet::new();
        for phase in &self.phases {
            validate_stable_id("cutscene_program.phase.id", &phase.id)?;
            validate_label("cutscene_program.phase.label", &phase.label)?;
            if prior_phase.is_some_and(|prior: &str| prior >= phase.id.as_str()) {
                return Err(PlannerContractError::new(
                    "cutscene_program.phases",
                    "must be sorted by phase ID",
                ));
            }
            prior_phase = Some(phase.id.as_str());
            validate_sorted_requests(&phase.resource_requests)?;
            if phase.branches.is_empty() || phase.branches.len() > MAX_PHASE_BRANCHES {
                return Err(PlannerContractError::new(
                    "cutscene_program.phase.branches",
                    format!("must contain between 1 and {MAX_PHASE_BRANCHES} branches"),
                ));
            }
            let mut prior_transition = None;
            for branch in &phase.branches {
                validate_stable_id(
                    "cutscene_program.branch.transition_id",
                    &branch.transition_id,
                )?;
                if !transition_ids.insert(branch.transition_id.as_str())
                    || prior_transition
                        .is_some_and(|prior: &str| prior >= branch.transition_id.as_str())
                {
                    return Err(PlannerContractError::new(
                        "cutscene_program.phase.branches",
                        "transition IDs must be globally unique and sorted within each phase",
                    ));
                }
                prior_transition = Some(branch.transition_id.as_str());
                validate_branch(branch, &phase_ids)?;
            }
        }
        Ok(())
    }

    pub fn compile(&self) -> Result<Vec<CandidateTransition>, PlannerContractError> {
        self.validate()?;
        let mut transitions = Vec::new();
        for phase in &self.phases {
            for branch in &phase.branches {
                let mut guards = vec![PredicateExpression::Compare {
                    left: ValueReference::FlowNode {
                        flow_component_id: self.flow_component_id.clone(),
                    },
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text(phase.id.clone()),
                    },
                }];
                guards.extend(phase.resource_requests.iter().map(|request| {
                    PredicateExpression::Compare {
                        left: ValueReference::ComponentField {
                            component_id: self.flow_component_id.clone(),
                            field: request.archive_field.clone(),
                        },
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Text(request.archive_id.clone()),
                        },
                    }
                }));
                guards.push(branch.hard_guards.clone());
                let mut effects = branch.confirmed_operations.clone();
                effects.extend(branch.uncertain_effects.iter().map(|effect| match effect {
                    UncertainCutsceneEffect::StructuredField { target } => {
                        StateOperation::InvalidateField {
                            target: target.clone(),
                        }
                    }
                    UncertainCutsceneEffect::RawBits {
                        component_id,
                        byte_offset,
                        mask,
                    } => StateOperation::InvalidateRaw {
                        component_id: component_id.clone(),
                        byte_offset: *byte_offset,
                        mask: mask.clone(),
                    },
                }));
                if let Some(destination_phase_id) = &branch.destination_phase_id {
                    effects.push(StateOperation::AdvanceFlow {
                        flow_component_id: self.flow_component_id.clone(),
                        node_id: destination_phase_id.clone(),
                    });
                } else {
                    effects.push(StateOperation::ClearComponent {
                        selector: ComponentSelector::Id {
                            component_id: self.flow_component_id.clone(),
                        },
                    });
                }
                let transition = CandidateTransition {
                    id: branch.transition_id.clone(),
                    label: branch.label.clone(),
                    scope: self.scope.clone(),
                    transition_kind: transition_kind(branch.kind),
                    approach_id: branch.approach_id.clone(),
                    activation: ActivationContract {
                        hard_guards: PredicateExpression::All { terms: guards },
                        physical_obligation_ids: branch.physical_obligation_ids.clone(),
                        effects,
                        unknown_requirements: branch.unknown_requirements.clone(),
                    },
                    evidence: branch.evidence.clone(),
                };
                transition.validate()?;
                transitions.push(transition);
            }
        }
        transitions.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(transitions)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let program: Self = serde_json::from_slice(bytes)?;
        program.validate()?;
        if program.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "cutscene_program",
                "is not canonical JSON",
            ));
        }
        Ok(program)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn compile_artifact(&self) -> Result<CompiledCutsceneProgram, PlannerContractError> {
        let artifact = CompiledCutsceneProgram {
            schema: COMPILED_CUTSCENE_PROGRAM_SCHEMA.into(),
            program_sha256: self.digest()?,
            transitions: self.compile()?,
        };
        artifact.validate()?;
        Ok(artifact)
    }
}

impl CompiledCutsceneProgram {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != COMPILED_CUTSCENE_PROGRAM_SCHEMA
            || self.program_sha256 == Digest::ZERO
            || self.transitions.is_empty()
        {
            return Err(PlannerContractError::new(
                "compiled_cutscene_program",
                "has an unsupported schema, zero source digest, or no transitions",
            ));
        }
        let mut prior = None;
        for transition in &self.transitions {
            transition.validate()?;
            if prior.is_some_and(|id: &str| id >= transition.id.as_str()) {
                return Err(PlannerContractError::new(
                    "compiled_cutscene_program.transitions",
                    "must be unique and sorted by transition ID",
                ));
            }
            prior = Some(transition.id.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let artifact: Self = serde_json::from_slice(bytes)?;
        artifact.validate()?;
        if artifact.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "compiled_cutscene_program",
                "is not canonical JSON",
            ));
        }
        Ok(artifact)
    }
}

fn validate_sorted_requests(
    requests: &[CutsceneResourceRequest],
) -> Result<(), PlannerContractError> {
    let mut prior = None;
    for request in requests {
        validate_stable_id(
            "cutscene_program.resource_request.archive_field",
            &request.archive_field,
        )?;
        validate_label(
            "cutscene_program.resource_request.archive_id",
            &request.archive_id,
        )?;
        validate_stable_id(
            "cutscene_program.resource_request.result_field",
            &request.result_field,
        )?;
        if prior.is_some_and(|field: &str| field >= request.archive_field.as_str()) {
            return Err(PlannerContractError::new(
                "cutscene_program.resource_requests",
                "must be unique and sorted by archive field",
            ));
        }
        prior = Some(request.archive_field.as_str());
    }
    Ok(())
}

fn validate_branch(
    branch: &CutsceneBranch,
    phase_ids: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    validate_label("cutscene_program.branch.label", &branch.label)?;
    validate_stable_id("cutscene_program.branch.approach_id", &branch.approach_id)?;
    branch.hard_guards.validate()?;
    validate_sorted_ids(
        "cutscene_program.branch.physical_obligation_ids",
        &branch.physical_obligation_ids,
    )?;
    for operation in &branch.confirmed_operations {
        operation.validate()?;
    }
    for effect in &branch.uncertain_effects {
        match effect {
            UncertainCutsceneEffect::StructuredField { target } => {
                StateOperation::InvalidateField {
                    target: target.clone(),
                }
                .validate()?;
            }
            UncertainCutsceneEffect::RawBits {
                component_id,
                byte_offset,
                mask,
            } => StateOperation::InvalidateRaw {
                component_id: component_id.clone(),
                byte_offset: *byte_offset,
                mask: mask.clone(),
            }
            .validate()?,
        }
    }
    let mut unknown_ids = BTreeSet::new();
    for unknown in &branch.unknown_requirements {
        validate_stable_id("cutscene_program.branch.unknown.id", &unknown.id)?;
        validate_label(
            "cutscene_program.branch.unknown.description",
            &unknown.description,
        )?;
        unknown
            .evidence
            .validate("cutscene_program.branch.unknown.evidence")?;
        if !unknown_ids.insert(unknown.id.as_str()) {
            return Err(PlannerContractError::new(
                "cutscene_program.branch.unknown_requirements",
                "contains duplicate IDs",
            ));
        }
    }
    branch
        .evidence
        .validate("cutscene_program.branch.evidence")?;
    if branch.kind == CutsceneBranchKind::SceneChange
        && !branch
            .confirmed_operations
            .iter()
            .any(|operation| matches!(operation, StateOperation::SetLocation { .. }))
    {
        return Err(PlannerContractError::new(
            "cutscene_program.branch.confirmed_operations",
            "a scene-change branch must contain a confirmed location operation",
        ));
    }
    if branch.kind == CutsceneBranchKind::Advance && branch.destination_phase_id.is_none() {
        return Err(PlannerContractError::new(
            "cutscene_program.branch.destination_phase_id",
            "an advance branch must target another phase",
        ));
    }
    match (&branch.destination_phase_id, branch.terminal) {
        (Some(destination), false) if phase_ids.contains(destination.as_str()) => Ok(()),
        (None, true) => Ok(()),
        (Some(_), false) => Err(PlannerContractError::new(
            "cutscene_program.branch.destination_phase_id",
            "references an unknown phase",
        )),
        _ => Err(PlannerContractError::new(
            "cutscene_program.branch",
            "must either target another phase or be terminal, but not both",
        )),
    }
}

fn validate_sorted_ids(field: &str, ids: &[String]) -> Result<(), PlannerContractError> {
    let mut prior = None;
    for id in ids {
        validate_stable_id(field, id)?;
        if prior.is_some_and(|value: &str| value >= id.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted",
            ));
        }
        prior = Some(id.as_str());
    }
    Ok(())
}

fn transition_kind(kind: CutsceneBranchKind) -> TransitionKind {
    match kind {
        CutsceneBranchKind::SceneChange => TransitionKind::CutsceneSceneChange,
        CutsceneBranchKind::ResourceLoadFailure => TransitionKind::ResourceLoadFailure,
        CutsceneBranchKind::Advance
        | CutsceneBranchKind::NormalCompletion
        | CutsceneBranchKind::IntentionalSkip
        | CutsceneBranchKind::Interruption => TransitionKind::Cutscene,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::identity::{ContextSelector, ExactContext};
    use crate::logic::{EvidenceKind, EvidenceRecord, TruthStatus};

    fn scope() -> ContextScope {
        ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([1; 32]),
                    runtime_configuration_sha256: Digest([2; 32]),
                },
            }],
        }
    }

    fn evidence() -> RuleEvidence {
        RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "trace.cutscene-prefix".into(),
                kind: EvidenceKind::TraceObserved,
                source_sha256: Some(Digest([3; 32])),
                note: "Observed through the branch point.".into(),
            }],
        }
    }

    fn branch(transition_id: &str, kind: CutsceneBranchKind, guard_value: &str) -> CutsceneBranch {
        let mut branch = CutsceneBranch {
            transition_id: transition_id.into(),
            label: transition_id.into(),
            kind,
            approach_id: "cutscene.load-phase".into(),
            hard_guards: PredicateExpression::Compare {
                left: ValueReference::ComponentField {
                    component_id: "cutscene.zelda".into(),
                    field: "load_result".into(),
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text(guard_value.into()),
                },
            },
            physical_obligation_ids: Vec::new(),
            confirmed_operations: vec![StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: "event.flags".into(),
                    field: "confirmed_prefix".into(),
                },
                value: StateValue::Boolean(true),
            }],
            uncertain_effects: Vec::new(),
            unknown_requirements: Vec::new(),
            destination_phase_id: None,
            terminal: true,
            evidence: evidence(),
        };
        if kind == CutsceneBranchKind::SceneChange {
            branch
                .confirmed_operations
                .push(StateOperation::SetLocation {
                    location: crate::state::SceneLocation {
                        stage: "R_SP109".into(),
                        room: 0,
                        layer: 0,
                        spawn: 0,
                    },
                });
        }
        branch
    }

    fn program() -> CutsceneProgram {
        let normal = branch(
            "transition.zelda-cutscene.normal",
            CutsceneBranchKind::SceneChange,
            "loaded",
        );
        let mut failure = branch(
            "transition.zelda-cutscene.resource-failure",
            CutsceneBranchKind::ResourceLoadFailure,
            "failed",
        );
        failure.uncertain_effects = vec![UncertainCutsceneEffect::StructuredField {
            target: ComponentFieldTarget {
                component_id: "event.flags".into(),
                field: "unaudited_suffix".into(),
            },
        }];
        CutsceneProgram {
            schema: CUTSCENE_PROGRAM_SCHEMA.into(),
            id: "zelda-post-meeting".into(),
            label: "Post-meeting cutscene fixture".into(),
            scope: scope(),
            flow_component_id: "cutscene.zelda".into(),
            entry_phase_id: "load-actors".into(),
            phases: vec![CutscenePhase {
                id: "load-actors".into(),
                label: "Load actors".into(),
                resource_requests: vec![CutsceneResourceRequest {
                    archive_field: "requested_archive".into(),
                    archive_id: "zelda-tower.arc".into(),
                    result_field: "load_result".into(),
                }],
                branches: vec![normal, failure],
            }],
        }
    }

    #[test]
    fn compiles_normal_and_failure_branches_without_inventing_suffix_writes() {
        let program = program();
        let transitions = program.compile().unwrap();
        assert_eq!(transitions.len(), 2);
        let normal = transitions
            .iter()
            .find(|transition| transition.id == "transition.zelda-cutscene.normal")
            .unwrap();
        assert_eq!(normal.transition_kind, TransitionKind::CutsceneSceneChange);
        let failure = transitions
            .iter()
            .find(|transition| transition.id == "transition.zelda-cutscene.resource-failure")
            .unwrap();
        assert_eq!(failure.transition_kind, TransitionKind::ResourceLoadFailure);
        assert!(matches!(
            failure.activation.effects.as_slice(),
            [
                StateOperation::Write { .. },
                StateOperation::InvalidateField { .. },
                StateOperation::ClearComponent { .. }
            ]
        ));
        assert!(failure.activation.effects.iter().all(|operation| !matches!(
            operation,
            StateOperation::Write { target, .. }
                if target.component_id == "save.return-place"
        )));
        assert_eq!(
            CutsceneProgram::decode_canonical(&program.canonical_bytes().unwrap()).unwrap(),
            program
        );
        let artifact = program.compile_artifact().unwrap();
        assert_eq!(artifact.transitions, transitions);
        assert_eq!(
            CompiledCutsceneProgram::decode_canonical(&artifact.canonical_bytes().unwrap())
                .unwrap(),
            artifact
        );
    }

    #[test]
    fn rejects_unknown_phase_targets_and_noncanonical_branch_order() {
        let mut invalid_target = program();
        invalid_target.phases[0].branches[0].terminal = false;
        invalid_target.phases[0].branches[0].destination_phase_id = Some("missing".into());
        assert!(invalid_target.validate().is_err());

        let mut reversed = program();
        reversed.phases[0].branches.reverse();
        assert!(reversed.validate().is_err());
    }
}
