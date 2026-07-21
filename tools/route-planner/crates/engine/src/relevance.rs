//! Goal-directed, catalog-only relevance expansion for stateful route search.
//!
//! This pass deliberately answers only "which mechanics could matter?". It does
//! not claim that a producer is reachable or that an obligation is satisfiable;
//! the forward solver remains responsible for those proofs.

use crate::PlannerContractError;
use crate::logic::{FactCatalog, PredicateExpression, ValueReference};
use crate::route_book::RouteActionRef;
use crate::state::{ComponentBinding, ComponentKind};
use crate::transition::{MechanicsCatalog, ObligationDetail, StateOperation};
use serde::Serialize;
use std::collections::BTreeSet;

pub const BACKWARD_RELEVANCE_SCHEMA: &str = "dusklight.route-planner.backward-relevance/v1";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateDependency {
    Fact {
        fact_id: String,
    },
    Component {
        component_id: String,
    },
    ComponentField {
        component_id: String,
        field: String,
    },
    BoundComponentField {
        component_kind: ComponentKind,
        binding: ComponentBinding,
        field: String,
    },
    RawBits {
        component_id: String,
        byte_offset: u32,
        byte_width: u8,
        mask: u64,
    },
    BoundRawBits {
        component_kind: ComponentKind,
        binding: ComponentBinding,
        byte_offset: u32,
        byte_width: u32,
    },
    RuntimeLanguage,
    RuntimeSetting {
        key: String,
    },
    RuntimeFileContext,
    LocationStage,
    LocationRoom,
    LocationLayer,
    LocationSpawn,
    PlayerForm,
    PlayerMount,
    PlayerControl,
    PlayerRotationX,
    PlayerRotationY,
    PlayerRotationZ,
    PlayerAction,
    ActorField {
        instance_id: String,
        field: String,
    },
    GateState {
        gate_id: String,
    },
    FlowNode {
        flow_component_id: String,
    },
    AnyState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackwardRelevance {
    pub schema: String,
    pub dependencies: Vec<StateDependency>,
    pub frontier_dependencies: Vec<StateDependency>,
    pub transition_ids: Vec<String>,
    pub writer_ids: Vec<String>,
    pub technique_ids: Vec<String>,
    pub obstruction_ids: Vec<String>,
    pub resolver_ids: Vec<String>,
    pub obligation_ids: Vec<String>,
    pub gate_ids: Vec<String>,
    pub reader_ids: Vec<String>,
    pub reconstruction_rule_ids: Vec<String>,
    pub microtrace_ids: Vec<String>,
}

impl BackwardRelevance {
    pub fn analyze(
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
        goal: &PredicateExpression,
    ) -> Result<Self, PlannerContractError> {
        Self::analyze_with_roots(facts, mechanics, goal, &[], &[])
    }

    pub fn analyze_with_roots(
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
        goal: &PredicateExpression,
        predicate_roots: &[PredicateExpression],
        action_roots: &[RouteActionRef],
    ) -> Result<Self, PlannerContractError> {
        facts.validate()?;
        mechanics.validate()?;
        goal.validate()?;

        let mut analysis = RelevanceBuilder::default();
        analysis.add_predicate(goal);
        for predicate in predicate_roots {
            predicate.validate()?;
            analysis.add_predicate(predicate);
        }
        for action in action_roots {
            analysis.add_action_root(mechanics, action)?;
        }
        loop {
            let before = analysis.cardinality();
            analysis.expand_facts(facts);
            analysis.expand_state_producers(mechanics);
            analysis.expand_relevant_transitions(mechanics);
            analysis.expand_relevant_writers(mechanics);
            if analysis.cardinality() == before {
                break;
            }
        }
        Ok(analysis.finish(facts, mechanics))
    }

    pub fn contains_transition(&self, id: &str) -> bool {
        self.transition_ids
            .binary_search_by(|candidate| candidate.as_str().cmp(id))
            .is_ok()
    }

    pub fn contains_writer(&self, id: &str) -> bool {
        self.writer_ids
            .binary_search_by(|candidate| candidate.as_str().cmp(id))
            .is_ok()
    }

    pub fn contains_technique(&self, id: &str) -> bool {
        self.technique_ids
            .binary_search_by(|candidate| candidate.as_str().cmp(id))
            .is_ok()
    }
}

#[derive(Default)]
struct RelevanceBuilder {
    dependencies: BTreeSet<StateDependency>,
    expanded_facts: BTreeSet<String>,
    transitions: BTreeSet<String>,
    writers: BTreeSet<String>,
    techniques: BTreeSet<String>,
    obstructions: BTreeSet<String>,
    resolvers: BTreeSet<String>,
    obligations: BTreeSet<String>,
    gates: BTreeSet<String>,
    readers: BTreeSet<String>,
    reconstructions: BTreeSet<String>,
    microtraces: BTreeSet<String>,
}

impl RelevanceBuilder {
    fn add_action_root(
        &mut self,
        mechanics: &MechanicsCatalog,
        action: &RouteActionRef,
    ) -> Result<(), PlannerContractError> {
        let known = match action {
            RouteActionRef::Transition { transition_id } => {
                self.transitions.insert(transition_id.clone());
                mechanics
                    .transitions
                    .iter()
                    .any(|transition| transition.id == *transition_id)
            }
            RouteActionRef::Technique { technique_id } => {
                self.techniques.insert(technique_id.clone());
                mechanics
                    .techniques
                    .iter()
                    .any(|technique| technique.id == *technique_id)
            }
            RouteActionRef::Writer { writer_id } => {
                self.writers.insert(writer_id.clone());
                mechanics
                    .writers
                    .iter()
                    .any(|writer| writer.id == *writer_id)
            }
            RouteActionRef::Resolver { resolver_id } => {
                self.resolvers.insert(resolver_id.clone());
                let Some(resolver) = mechanics
                    .resolvers
                    .iter()
                    .find(|resolver| resolver.id == *resolver_id)
                else {
                    return Err(PlannerContractError::new(
                        "backward_relevance.action_roots",
                        format!("references unknown resolver {resolver_id}"),
                    ));
                };
                if let Some(obstruction) = mechanics
                    .obstructions
                    .iter()
                    .find(|obstruction| obstruction.id == resolver.obstruction_id)
                {
                    self.obstructions.insert(obstruction.id.clone());
                    self.transitions
                        .insert(obstruction.blocked_action_id.clone());
                }
                true
            }
            RouteActionRef::Microtrace { microtrace_id } => {
                self.microtraces.insert(microtrace_id.clone());
                if let Some(microtrace) = mechanics
                    .microtraces
                    .iter()
                    .find(|microtrace| microtrace.id == *microtrace_id)
                {
                    self.add_predicate(&microtrace.precondition);
                    self.add_predicate(&microtrace.postcondition);
                    true
                } else {
                    false
                }
            }
        };
        if known {
            Ok(())
        } else {
            Err(PlannerContractError::new(
                "backward_relevance.action_roots",
                format!("references unknown action {action:?}"),
            ))
        }
    }

    fn cardinality(&self) -> usize {
        self.dependencies.len()
            + self.expanded_facts.len()
            + self.transitions.len()
            + self.writers.len()
            + self.techniques.len()
            + self.obstructions.len()
            + self.resolvers.len()
            + self.obligations.len()
            + self.gates.len()
            + self.readers.len()
            + self.reconstructions.len()
            + self.microtraces.len()
    }

    fn add_predicate(&mut self, predicate: &PredicateExpression) {
        match predicate {
            PredicateExpression::Compare { left, right, .. } => {
                self.add_value(left);
                self.add_value(right);
            }
            PredicateExpression::Fact { fact_id } => {
                self.dependencies.insert(StateDependency::Fact {
                    fact_id: fact_id.clone(),
                });
            }
            PredicateExpression::All { terms } | PredicateExpression::Any { terms } => {
                for term in terms {
                    self.add_predicate(term);
                }
            }
            PredicateExpression::Not { term } => self.add_predicate(term),
            PredicateExpression::True | PredicateExpression::False => {}
        }
    }

    fn add_value(&mut self, value: &ValueReference) {
        let dependency = match value {
            ValueReference::Literal { .. } => return,
            ValueReference::ComponentField {
                component_id,
                field,
            } => StateDependency::ComponentField {
                component_id: component_id.clone(),
                field: field.clone(),
            },
            ValueReference::BoundComponentField {
                component_kind,
                binding,
                field,
            } => StateDependency::BoundComponentField {
                component_kind: component_kind.clone(),
                binding: binding.clone(),
                field: field.clone(),
            },
            ValueReference::RawBits {
                component_id,
                byte_offset,
                byte_width,
                mask,
            } => StateDependency::RawBits {
                component_id: component_id.clone(),
                byte_offset: *byte_offset,
                byte_width: *byte_width,
                mask: *mask,
            },
            ValueReference::RuntimeLanguage => StateDependency::RuntimeLanguage,
            ValueReference::RuntimeSetting { key } => {
                StateDependency::RuntimeSetting { key: key.clone() }
            }
            ValueReference::LocationStage => StateDependency::LocationStage,
            ValueReference::LocationRoom => StateDependency::LocationRoom,
            ValueReference::LocationLayer => StateDependency::LocationLayer,
            ValueReference::LocationSpawn => StateDependency::LocationSpawn,
            ValueReference::PlayerForm => StateDependency::PlayerForm,
            ValueReference::PlayerMount => StateDependency::PlayerMount,
            ValueReference::PlayerControl => StateDependency::PlayerControl,
            ValueReference::PlayerRotationX => StateDependency::PlayerRotationX,
            ValueReference::PlayerRotationY => StateDependency::PlayerRotationY,
            ValueReference::PlayerRotationZ => StateDependency::PlayerRotationZ,
            ValueReference::PlayerAction => StateDependency::PlayerAction,
            ValueReference::ActorField { instance_id, field } => StateDependency::ActorField {
                instance_id: instance_id.clone(),
                field: field.clone(),
            },
            ValueReference::GateState { gate_id } => StateDependency::GateState {
                gate_id: gate_id.clone(),
            },
            ValueReference::FlowNode { flow_component_id } => StateDependency::FlowNode {
                flow_component_id: flow_component_id.clone(),
            },
        };
        self.dependencies.insert(dependency);
    }

    fn expand_facts(&mut self, facts: &FactCatalog) {
        let pending = self
            .dependencies
            .iter()
            .filter_map(|dependency| match dependency {
                StateDependency::Fact { fact_id } if !self.expanded_facts.contains(fact_id) => {
                    Some(fact_id.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for fact_id in pending {
            self.expanded_facts.insert(fact_id.clone());
            if let Some(alias) = facts.aliases.iter().find(|alias| alias.id == fact_id) {
                self.dependencies.insert(StateDependency::BoundRawBits {
                    component_kind: alias.raw.component_kind.clone(),
                    binding: alias.raw.binding.clone(),
                    byte_offset: alias.raw.byte_offset,
                    byte_width: alias.raw.mask.len() as u32,
                });
            } else if let Some(derived) = facts
                .derived_facts
                .iter()
                .find(|derived| derived.id == fact_id)
            {
                self.add_predicate(&derived.rule);
            }
        }
    }

    fn expand_state_producers(&mut self, mechanics: &MechanicsCatalog) {
        for transition in &mechanics.transitions {
            if transition
                .activation
                .effects
                .iter()
                .any(|operation| self.operation_is_relevant(operation))
            {
                self.transitions.insert(transition.id.clone());
            }
        }
        for writer in &mechanics.writers {
            if self.operation_is_relevant(&writer.operation) {
                self.writers.insert(writer.id.clone());
            }
        }
        for technique in &mechanics.techniques {
            if technique
                .operations
                .iter()
                .any(|operation| self.operation_is_relevant(operation))
            {
                self.techniques.insert(technique.id.clone());
            }
        }
        for reconstruction in &mechanics.reconstruction_rules {
            if reconstruction
                .initialization_operations
                .iter()
                .any(|operation| self.operation_is_relevant(operation))
            {
                self.reconstructions.insert(reconstruction.id.clone());
                self.add_predicate(&reconstruction.instantiate_when);
                for operation in &reconstruction.initialization_operations {
                    self.add_operation_inputs(operation);
                }
            }
        }
        for resolver in &mechanics.resolvers {
            if resolver
                .operations
                .iter()
                .any(|operation| self.operation_is_relevant(operation))
            {
                self.resolvers.insert(resolver.id.clone());
                if let Some(obstruction) = mechanics
                    .obstructions
                    .iter()
                    .find(|obstruction| obstruction.id == resolver.obstruction_id)
                {
                    self.obstructions.insert(obstruction.id.clone());
                    self.transitions
                        .insert(obstruction.blocked_action_id.clone());
                }
            }
        }
    }

    fn expand_relevant_transitions(&mut self, mechanics: &MechanicsCatalog) {
        let relevant_transitions = self.transitions.clone();
        for transition in mechanics
            .transitions
            .iter()
            .filter(|transition| relevant_transitions.contains(&transition.id))
        {
            self.add_predicate(&transition.activation.hard_guards);
            for operation in &transition.activation.effects {
                self.add_operation_inputs(operation);
            }
            for obligation_id in &transition.activation.physical_obligation_ids {
                self.obligations.insert(obligation_id.clone());
            }
            for reader in mechanics
                .readers
                .iter()
                .filter(|reader| reader.consuming_transition_id == transition.id)
            {
                self.readers.insert(reader.id.clone());
                self.add_value(&reader.source);
                if let Some(fact_id) = &reader.interpretation_fact_id {
                    self.dependencies.insert(StateDependency::Fact {
                        fact_id: fact_id.clone(),
                    });
                }
            }
            for obstruction in mechanics.obstructions.iter().filter(|obstruction| {
                obstruction.blocked_action_id == transition.id
                    && obstruction.approach_id == transition.approach_id
            }) {
                self.obstructions.insert(obstruction.id.clone());
                self.add_predicate(&obstruction.active_when);
                self.obligations
                    .extend(obstruction.obligation_ids.iter().cloned());
                for resolver in mechanics
                    .resolvers
                    .iter()
                    .filter(|resolver| resolver.obstruction_id == obstruction.id)
                {
                    self.resolvers.insert(resolver.id.clone());
                    self.add_predicate(&resolver.applicable_when);
                    for operation in &resolver.operations {
                        self.add_operation_inputs(operation);
                    }
                }
            }
        }

        let relevant_obligations = self.obligations.clone();
        for obligation in mechanics
            .obligations
            .iter()
            .filter(|obligation| relevant_obligations.contains(&obligation.id))
        {
            match &obligation.detail {
                ObligationDetail::Predicate { predicate } => self.add_predicate(predicate),
                ObligationDetail::Interaction {
                    pose_predicate,
                    temporal_requirement,
                    ..
                } => {
                    self.add_predicate(pose_predicate);
                    if let Some(requirement) = temporal_requirement {
                        self.add_temporal_witnesses(mechanics, &requirement.action_id);
                    }
                }
                ObligationDetail::Temporal {
                    requirement,
                    precondition,
                } => {
                    self.add_predicate(precondition);
                    self.add_temporal_witnesses(mechanics, &requirement.action_id);
                }
                ObligationDetail::Geometry { .. }
                | ObligationDetail::PlaneSide { .. }
                | ObligationDetail::Unresolved { .. } => {}
            }
        }
        for technique in &mechanics.techniques {
            if technique
                .discharged_obligation_ids
                .iter()
                .any(|id| relevant_obligations.contains(id))
            {
                self.techniques.insert(technique.id.clone());
            }
        }
        let relevant_techniques = self.techniques.clone();
        for technique in mechanics
            .techniques
            .iter()
            .filter(|technique| relevant_techniques.contains(&technique.id))
        {
            self.add_predicate(&technique.prerequisites);
            for operation in &technique.operations {
                self.add_operation_inputs(operation);
            }
            self.obligations
                .extend(technique.introduced_obligation_ids.iter().cloned());
        }
    }

    fn expand_relevant_writers(&mut self, mechanics: &MechanicsCatalog) {
        let relevant_writers = self.writers.clone();
        for writer in mechanics
            .writers
            .iter()
            .filter(|writer| relevant_writers.contains(&writer.id))
        {
            self.add_predicate(&writer.activation);
            self.add_operation_inputs(&writer.operation);
            for gate in mechanics.gates.iter().filter(|gate| {
                gate.blocked_writer_ids
                    .iter()
                    .any(|writer_id| writer_id == &writer.id)
            }) {
                self.gates.insert(gate.id.clone());
                self.add_predicate(&gate.active_when);
            }
        }
    }

    fn add_temporal_witnesses(&mut self, mechanics: &MechanicsCatalog, action_id: &str) {
        for microtrace in &mechanics.microtraces {
            if microtrace.operations.iter().any(|operation| {
                matches!(operation, StateOperation::Interrupt { action_id: candidate, .. } if candidate == action_id)
            }) {
                self.microtraces.insert(microtrace.id.clone());
                self.add_predicate(&microtrace.precondition);
                self.add_predicate(&microtrace.postcondition);
            }
        }
    }

    fn operation_is_relevant(&self, operation: &StateOperation) -> bool {
        operation_outputs(operation).iter().any(|output| {
            self.dependencies
                .iter()
                .any(|need| dependencies_overlap(need, output))
        })
    }

    fn add_operation_inputs(&mut self, operation: &StateOperation) {
        match operation {
            StateOperation::CopyValue { source, .. }
            | StateOperation::SetBitFromValue { source, .. } => {
                self.dependencies.insert(StateDependency::ComponentField {
                    component_id: source.component_id.clone(),
                    field: source.field.clone(),
                });
            }
            StateOperation::Copy { source, .. }
            | StateOperation::Move { source, .. }
            | StateOperation::Preserve { selector: source }
            | StateOperation::Serialize {
                selector: source, ..
            }
            | StateOperation::Bind {
                selector: source, ..
            }
            | StateOperation::Rebind {
                selector: source, ..
            } => {
                if let crate::state::ComponentSelector::Id { component_id } = source {
                    self.dependencies.insert(StateDependency::Component {
                        component_id: component_id.clone(),
                    });
                } else {
                    self.dependencies.insert(StateDependency::AnyState);
                }
            }
            StateOperation::Project { component_ids, .. } => {
                self.dependencies.extend(
                    component_ids
                        .iter()
                        .cloned()
                        .map(|component_id| StateDependency::Component { component_id }),
                );
            }
            StateOperation::CommitLoadStageBank { component_id, .. } => {
                self.dependencies.insert(StateDependency::Component {
                    component_id: component_id.clone(),
                });
                self.dependencies.insert(StateDependency::LocationStage);
            }
            StateOperation::Write { .. }
            | StateOperation::WriteRaw { .. }
            | StateOperation::InvalidateRaw { .. }
            | StateOperation::Adjust { .. }
            | StateOperation::ClearComponent { .. }
            | StateOperation::ClearField { .. }
            | StateOperation::InvalidateField { .. }
            | StateOperation::Initialize { .. }
            | StateOperation::Restore { .. }
            | StateOperation::SetActiveRuntimeFile { .. }
            | StateOperation::SetLocation { .. }
            | StateOperation::SetPlayerForm { .. }
            | StateOperation::SetPlayerMount { .. }
            | StateOperation::SetPlayerControl { .. }
            | StateOperation::SetPlayerAction { .. }
            | StateOperation::Consume { .. }
            | StateOperation::SetGate { .. }
            | StateOperation::ClearGate { .. }
            | StateOperation::AdvanceFlow { .. }
            | StateOperation::BranchFlow { .. }
            | StateOperation::ScheduleCleanup { .. }
            | StateOperation::CancelCleanup { .. }
            | StateOperation::Interrupt { .. } => {}
        }
    }

    fn finish(self, facts: &FactCatalog, mechanics: &MechanicsCatalog) -> BackwardRelevance {
        let frontier_dependencies = self
            .dependencies
            .iter()
            .filter(|dependency| !has_catalog_producer(dependency, facts, mechanics))
            .cloned()
            .collect();
        BackwardRelevance {
            schema: BACKWARD_RELEVANCE_SCHEMA.into(),
            dependencies: self.dependencies.into_iter().collect(),
            frontier_dependencies,
            transition_ids: self.transitions.into_iter().collect(),
            writer_ids: self.writers.into_iter().collect(),
            technique_ids: self.techniques.into_iter().collect(),
            obstruction_ids: self.obstructions.into_iter().collect(),
            resolver_ids: self.resolvers.into_iter().collect(),
            obligation_ids: self.obligations.into_iter().collect(),
            gate_ids: self.gates.into_iter().collect(),
            reader_ids: self.readers.into_iter().collect(),
            reconstruction_rule_ids: self.reconstructions.into_iter().collect(),
            microtrace_ids: self.microtraces.into_iter().collect(),
        }
    }
}

fn operation_outputs(operation: &StateOperation) -> Vec<StateDependency> {
    match operation {
        StateOperation::Write { target, .. }
        | StateOperation::Adjust { target, .. }
        | StateOperation::ClearField { target }
        | StateOperation::InvalidateField { target }
        | StateOperation::CopyValue { target, .. }
        | StateOperation::SetBitFromValue { target, .. } => {
            vec![StateDependency::ComponentField {
                component_id: target.component_id.clone(),
                field: target.field.clone(),
            }]
        }
        StateOperation::WriteRaw {
            component_id,
            byte_offset,
            mask,
            ..
        }
        | StateOperation::InvalidateRaw {
            component_id,
            byte_offset,
            mask,
        } => vec![StateDependency::RawBits {
            component_id: component_id.clone(),
            byte_offset: *byte_offset,
            byte_width: mask.len() as u8,
            mask: u64::MAX,
        }],
        StateOperation::ClearComponent { selector } => {
            if let crate::state::ComponentSelector::Id { component_id } = selector {
                vec![StateDependency::Component {
                    component_id: component_id.clone(),
                }]
            } else {
                vec![StateDependency::AnyState]
            }
        }
        StateOperation::Initialize { component } => vec![StateDependency::Component {
            component_id: component.id.clone(),
        }],
        StateOperation::Copy {
            destination_component_id,
            ..
        }
        | StateOperation::Move {
            destination_component_id,
            ..
        }
        | StateOperation::Restore {
            destination_component_id,
            ..
        } => vec![StateDependency::Component {
            component_id: destination_component_id.clone(),
        }],
        StateOperation::CommitLoadStageBank { component_id, .. } => vec![
            StateDependency::Component {
                component_id: component_id.clone(),
            },
            StateDependency::AnyState,
        ],
        StateOperation::SetLocation { .. } => vec![
            StateDependency::LocationStage,
            StateDependency::LocationRoom,
            StateDependency::LocationLayer,
            StateDependency::LocationSpawn,
        ],
        StateOperation::SetPlayerForm { .. } => vec![StateDependency::PlayerForm],
        StateOperation::SetPlayerMount { .. } => vec![StateDependency::PlayerMount],
        StateOperation::SetPlayerControl { .. } => vec![StateDependency::PlayerControl],
        StateOperation::SetPlayerAction { .. } => vec![StateDependency::PlayerAction],
        StateOperation::SetGate { gate_id } | StateOperation::ClearGate { gate_id } => {
            vec![StateDependency::GateState {
                gate_id: gate_id.clone(),
            }]
        }
        StateOperation::AdvanceFlow {
            flow_component_id, ..
        }
        | StateOperation::BranchFlow {
            flow_component_id, ..
        } => vec![StateDependency::FlowNode {
            flow_component_id: flow_component_id.clone(),
        }],
        StateOperation::Consume {
            pending_operation_id,
        } => vec![StateDependency::Component {
            component_id: pending_operation_id.clone(),
        }],
        StateOperation::Preserve { .. }
        | StateOperation::Serialize { .. }
        | StateOperation::Bind { .. }
        | StateOperation::Rebind { .. }
        | StateOperation::SetActiveRuntimeFile { .. }
        | StateOperation::Project { .. } => vec![StateDependency::AnyState],
        StateOperation::ScheduleCleanup { .. }
        | StateOperation::CancelCleanup { .. }
        | StateOperation::Interrupt { .. } => Vec::new(),
    }
}

fn dependencies_overlap(left: &StateDependency, right: &StateDependency) -> bool {
    use StateDependency as D;
    match (left, right) {
        (D::AnyState, _) | (_, D::AnyState) => true,
        (
            D::Component { component_id },
            D::Component {
                component_id: other,
            },
        ) => component_id == other,
        (
            D::Component { component_id },
            D::ComponentField {
                component_id: other,
                ..
            },
        )
        | (
            D::ComponentField {
                component_id: other,
                ..
            },
            D::Component { component_id },
        )
        | (
            D::Component { component_id },
            D::RawBits {
                component_id: other,
                ..
            },
        )
        | (
            D::RawBits {
                component_id: other,
                ..
            },
            D::Component { component_id },
        ) => component_id == other,
        (
            D::ComponentField {
                component_id,
                field,
            },
            D::ComponentField {
                component_id: other,
                field: other_field,
            },
        ) => component_id == other && field == other_field,
        (D::BoundComponentField { field, .. }, D::ComponentField { field: other, .. })
        | (D::ComponentField { field: other, .. }, D::BoundComponentField { field, .. }) => {
            field == other
        }
        (
            D::RawBits {
                component_id,
                byte_offset,
                byte_width,
                ..
            },
            D::RawBits {
                component_id: other,
                byte_offset: other_offset,
                byte_width: other_width,
                ..
            },
        ) => {
            component_id == other
                && ranges_overlap(
                    *byte_offset,
                    u32::from(*byte_width),
                    *other_offset,
                    u32::from(*other_width),
                )
        }
        (
            D::BoundRawBits {
                byte_offset,
                byte_width,
                ..
            },
            D::RawBits {
                byte_offset: other_offset,
                byte_width: other_width,
                ..
            },
        )
        | (
            D::RawBits {
                byte_offset: other_offset,
                byte_width: other_width,
                ..
            },
            D::BoundRawBits {
                byte_offset,
                byte_width,
                ..
            },
        ) => ranges_overlap(
            *byte_offset,
            *byte_width,
            *other_offset,
            u32::from(*other_width),
        ),
        _ => left == right,
    }
}

fn ranges_overlap(left: u32, left_width: u32, right: u32, right_width: u32) -> bool {
    left < right.saturating_add(right_width) && right < left.saturating_add(left_width)
}

fn has_catalog_producer(
    dependency: &StateDependency,
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
) -> bool {
    if let StateDependency::Fact { fact_id } = dependency {
        return facts.aliases.iter().any(|fact| fact.id == *fact_id)
            || facts.derived_facts.iter().any(|fact| fact.id == *fact_id);
    }
    mechanics
        .transitions
        .iter()
        .flat_map(|transition| &transition.activation.effects)
        .chain(mechanics.writers.iter().map(|writer| &writer.operation))
        .chain(
            mechanics
                .techniques
                .iter()
                .flat_map(|technique| &technique.operations),
        )
        .chain(
            mechanics
                .resolvers
                .iter()
                .flat_map(|resolver| &resolver.operations),
        )
        .chain(
            mechanics
                .reconstruction_rules
                .iter()
                .flat_map(|rule| &rule.initialization_operations),
        )
        .any(|operation| {
            operation_outputs(operation)
                .iter()
                .any(|output| dependencies_overlap(dependency, output))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::identity::{ContextSelector, ExactContext};
    use crate::logic::{
        ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA,
        RuleEvidence, TruthStatus,
    };
    use crate::state::{SemanticLifetime, StateValue};
    use crate::transition::{
        ActivationContract, CandidateTransition, ComponentFieldTarget, FeasibilityObligation,
        GateRule, MECHANICS_CATALOG_SCHEMA, ObligationDetail, ObligationKind, Obstruction,
        ObstructionResolver, ReaderRule, ResolutionKind, RouteCost, Technique, TemporalRequirement,
        TemporalWindow, TransitionKind, WitnessedMicrotrace, WriterRule,
    };

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
                id: "test.evidence".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(Digest([3; 32])),
                note: "Synthetic relevance acceptance evidence.".into(),
            }],
        }
    }

    fn field(name: &str) -> ValueReference {
        ValueReference::ComponentField {
            component_id: "state.route".into(),
            field: name.into(),
        }
    }

    fn equals(name: &str, value: u64) -> PredicateExpression {
        PredicateExpression::Compare {
            left: field(name),
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Unsigned(value),
            },
        }
    }

    fn write(name: &str, value: u64) -> StateOperation {
        StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: "state.route".into(),
                field: name.into(),
            },
            value: StateValue::Unsigned(value),
        }
    }

    fn transition(
        id: &str,
        guard: PredicateExpression,
        effects: Vec<StateOperation>,
    ) -> CandidateTransition {
        CandidateTransition {
            id: id.into(),
            label: id.into(),
            scope: scope(),
            transition_kind: TransitionKind::Other,
            approach_id: format!("approach.{id}"),
            activation: ActivationContract {
                hard_guards: guard,
                physical_obligation_ids: Vec::new(),
                effects,
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(),
        }
    }

    #[test]
    fn expands_all_producers_through_cycles_readers_and_writer_gates() {
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let goal_guard = PredicateExpression::Any {
            terms: vec![equals("alternate", 1), equals("middle", 1)],
        };
        let mut mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: vec![
                transition(
                    "transition.alternate",
                    equals("root", 1),
                    vec![write("alternate", 1)],
                ),
                transition(
                    "transition.cycle",
                    equals("middle", 1),
                    vec![write("root", 1)],
                ),
                transition("transition.goal", goal_guard, vec![write("final", 1)]),
                transition(
                    "transition.middle",
                    equals("root", 1),
                    vec![write("middle", 1)],
                ),
                transition(
                    "transition.noise",
                    PredicateExpression::True,
                    vec![write("noise", 1)],
                ),
                transition(
                    "transition.unlock-writer",
                    PredicateExpression::True,
                    vec![write("writer_blocked", 0)],
                ),
            ],
            obligations: Vec::new(),
            writers: vec![WriterRule {
                id: "writer.recent-item".into(),
                scope: scope(),
                activation: PredicateExpression::True,
                operation: write("recent_item", 0x4a),
                evidence: evidence(),
            }],
            gates: vec![GateRule {
                id: "gate.recent-item-writer".into(),
                scope: scope(),
                active_when: equals("writer_blocked", 1),
                blocked_writer_ids: vec!["writer.recent-item".into()],
                lifetime: SemanticLifetime::Session,
                evidence: evidence(),
            }],
            readers: vec![ReaderRule {
                id: "reader.goal-recent-item".into(),
                scope: scope(),
                source: field("recent_item"),
                consuming_transition_id: "transition.goal".into(),
                interpretation_fact_id: None,
                evidence: evidence(),
            }],
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: Vec::new(),
        };
        mechanics
            .transitions
            .sort_by(|left, right| left.id.cmp(&right.id));

        let relevance = BackwardRelevance::analyze(&facts, &mechanics, &equals("final", 1))
            .expect("catalog should expand");
        assert_eq!(
            relevance.transition_ids,
            vec![
                "transition.alternate",
                "transition.cycle",
                "transition.goal",
                "transition.middle",
                "transition.unlock-writer",
            ]
        );
        assert_eq!(relevance.writer_ids, vec!["writer.recent-item"]);
        assert_eq!(relevance.gate_ids, vec!["gate.recent-item-writer"]);
        assert_eq!(relevance.reader_ids, vec!["reader.goal-recent-item"]);
        assert!(!relevance.contains_transition("transition.noise"));
        assert!(relevance.frontier_dependencies.is_empty());
    }

    #[test]
    fn pulls_feasibility_alternatives_and_exact_temporal_witnesses_inward() {
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let timing = TemporalWindow {
            earliest_frame: 12,
            latest_frame: 12,
            required_input: Some("sidehop".into()),
        };
        let mut target = transition(
            "transition.target",
            PredicateExpression::True,
            vec![write("final", 1)],
        );
        target.approach_id = "approach.target".into();
        target.activation.physical_obligation_ids = vec!["obligation.timing".into()];
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: vec![target],
            obligations: vec![FeasibilityObligation {
                id: "obligation.timing".into(),
                label: "Witness the exact interruption".into(),
                scope: scope(),
                obligation_kind: ObligationKind::Timing,
                detail: ObligationDetail::Temporal {
                    requirement: TemporalRequirement {
                        action_id: "dialogue.overwrite".into(),
                        window: timing.clone(),
                    },
                    precondition: equals("armed", 1),
                },
                evidence: evidence(),
            }],
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: vec![Obstruction {
                id: "obstruction.target".into(),
                label: "Target is physically obstructed".into(),
                scope: scope(),
                blocked_action_id: "transition.target".into(),
                approach_id: "approach.target".into(),
                active_when: equals("blocked", 1),
                obligation_ids: vec!["obligation.timing".into()],
                evidence: evidence(),
            }],
            resolvers: vec![ObstructionResolver {
                id: "resolver.target".into(),
                label: "Bypass the target obstruction".into(),
                scope: scope(),
                obstruction_id: "obstruction.target".into(),
                resolution_kind: ResolutionKind::Bypass,
                applicable_when: equals("resolver_ready", 1),
                operations: Vec::new(),
                evidence: evidence(),
            }],
            techniques: vec![Technique {
                id: "technique.timing".into(),
                label: "Perform the timing setup".into(),
                scope: scope(),
                prerequisites: equals("technique_ready", 1),
                operations: Vec::new(),
                discharged_obligation_ids: vec!["obligation.timing".into()],
                introduced_obligation_ids: Vec::new(),
                cost: RouteCost {
                    axes: Default::default(),
                },
                evidence: evidence(),
            }],
            microtraces: vec![WitnessedMicrotrace {
                id: "microtrace.timing".into(),
                scope: scope(),
                precondition: equals("trace_ready", 1),
                operations: vec![StateOperation::Interrupt {
                    action_id: "dialogue.overwrite".into(),
                    window: timing.clone(),
                }],
                postcondition: PredicateExpression::True,
                timing,
                evidence: evidence(),
            }],
            goals: Vec::new(),
        };

        let relevance = BackwardRelevance::analyze(&facts, &mechanics, &equals("final", 1))
            .expect("catalog should expand");
        assert_eq!(relevance.obligation_ids, vec!["obligation.timing"]);
        assert_eq!(relevance.obstruction_ids, vec!["obstruction.target"]);
        assert_eq!(relevance.resolver_ids, vec!["resolver.target"]);
        assert_eq!(relevance.technique_ids, vec!["technique.timing"]);
        assert_eq!(relevance.microtrace_ids, vec!["microtrace.timing"]);
        for field in [
            "armed",
            "blocked",
            "resolver_ready",
            "technique_ready",
            "trace_ready",
        ] {
            assert!(
                relevance
                    .dependencies
                    .contains(&StateDependency::ComponentField {
                        component_id: "state.route".into(),
                        field: field.into(),
                    })
            );
        }
    }
}
