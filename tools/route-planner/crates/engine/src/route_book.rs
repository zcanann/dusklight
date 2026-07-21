//! Curated route preferences and collapsible plan regions.
//!
//! Route books reference mechanics; they never duplicate transition effects or
//! author derived losses. Removing a book therefore cannot change game truth.

use crate::artifact::Digest;
use crate::logic::{ContextScope, FactCatalog, PredicateExpression};
use crate::refinement::ComposedPlannerCatalog;
use crate::transition::{MechanicsCatalog, PathConstraint};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const ROUTE_BOOK_SCHEMA: &str = "dusklight.route-planner.route-book/v4";
pub const ROUTE_BOOK_EDIT_BATCH_SCHEMA: &str = "dusklight.route-planner.route-book-edit-batch/v4";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteBookManifest {
    pub id: String,
    pub version: String,
    pub label: String,
    pub author: String,
    pub source: String,
    pub scope: ContextScope,
    pub refinement_stack_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteBook {
    pub schema: String,
    pub manifest: RouteBookManifest,
    pub goal_ids: Vec<String>,
    pub constraints: Vec<RouteConstraint>,
    pub directives: Vec<RouteDirective>,
    pub steps: Vec<ReferenceStep>,
    pub methods: Vec<PlanMethod>,
    pub regions: Vec<PlanRegion>,
    pub annotations: Vec<RouteAnnotation>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConstraint {
    pub id: String,
    pub scope: ContextScope,
    pub constraint: PathConstraint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteDirective {
    pub id: String,
    pub scope: ContextScope,
    pub directive: RouteDirectiveKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouteDirectiveKind {
    PinAction { action: RouteActionRef },
    BanAction { action: RouteActionRef },
    PreferAction { action: RouteActionRef, weight: u32 },
    PinMethod { method_id: String },
    BanMethod { method_id: String },
    PreferMethod { method_id: String, weight: u32 },
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouteActionRef {
    Transition { transition_id: String },
    Technique { technique_id: String },
    Resolver { resolver_id: String },
    Writer { writer_id: String },
    Microtrace { microtrace_id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceStep {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub action: RouteActionRef,
    pub precondition: Option<PredicateExpression>,
    pub postcondition: Option<PredicateExpression>,
    pub region_id: Option<String>,
    pub annotation_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanMethod {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub region_id: String,
    pub step_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRegion {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub parent_region_id: Option<String>,
    pub entry_predicate: Option<PredicateExpression>,
    pub outcome_predicate: PredicateExpression,
    pub method_ids: Vec<String>,
    pub selected_method_id: Option<String>,
    pub collapse_policy: CollapsePolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CollapsePolicy {
    OnlyContinuationEquivalent,
    ShowResidualDifferences,
    Never,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteAnnotation {
    pub id: String,
    pub target: AnnotationTarget,
    pub body: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnnotationTarget {
    Goal { goal_id: String },
    Fact { fact_id: String },
    Action { action: RouteActionRef },
    Step { step_id: String },
    Method { method_id: String },
    Region { region_id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteBookEditBatch {
    pub schema: String,
    pub expected_route_book_sha256: Digest,
    pub edits: Vec<RouteBookEdit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouteBookEdit {
    SetGoalIds {
        goal_ids: Vec<String>,
    },
    UpsertConstraint {
        constraint: RouteConstraint,
    },
    RemoveConstraint {
        constraint_id: String,
    },
    UpsertDirective {
        directive: RouteDirective,
    },
    RemoveDirective {
        directive_id: String,
    },
    UpsertStep {
        step: ReferenceStep,
    },
    RemoveStep {
        step_id: String,
    },
    UpsertMethod {
        method: PlanMethod,
    },
    RemoveMethod {
        method_id: String,
    },
    UpsertRegion {
        region: PlanRegion,
    },
    RemoveRegion {
        region_id: String,
    },
    SetSelectedMethod {
        region_id: String,
        method_id: Option<String>,
    },
    SetCollapsePolicy {
        region_id: String,
        collapse_policy: CollapsePolicy,
    },
    UpsertAnnotation {
        annotation: RouteAnnotation,
    },
    RemoveAnnotation {
        annotation_id: String,
    },
}

impl RouteBookManifest {
    fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("manifest.id", &self.id)?;
        validate_version(&self.version)?;
        validate_label("manifest.label", &self.label)?;
        validate_label("manifest.author", &self.author)?;
        validate_label("manifest.source", &self.source)?;
        self.scope.validate("manifest.scope")?;
        if self.refinement_stack_sha256 == Some(Digest::ZERO) {
            return Err(PlannerContractError::new(
                "manifest.refinement_stack_sha256",
                "must be absent or nonzero",
            ));
        }
        Ok(())
    }
}

impl RouteBook {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != ROUTE_BOOK_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        self.manifest.validate()?;
        validate_sorted_ids("goal_ids", &self.goal_ids, false)?;
        validate_sorted_records(
            "constraints",
            &self.constraints,
            |record| &record.id,
            |record| {
                record.scope.validate("constraints.scope")?;
                require_scope_subset("constraints.scope", &record.scope, &self.manifest.scope)?;
                validate_constraint(&record.constraint)
            },
        )?;
        validate_sorted_records(
            "directives",
            &self.directives,
            |record| &record.id,
            |record| {
                record.scope.validate("directives.scope")?;
                require_scope_subset("directives.scope", &record.scope, &self.manifest.scope)?;
                validate_directive(&record.directive)
            },
        )?;
        let step_ids = validate_sorted_records(
            "steps",
            &self.steps,
            |record| &record.id,
            |step| {
                validate_label("steps.label", &step.label)?;
                step.scope.validate("steps.scope")?;
                require_scope_subset("steps.scope", &step.scope, &self.manifest.scope)?;
                validate_action(&step.action)?;
                if let Some(predicate) = &step.precondition {
                    predicate.validate()?;
                }
                if let Some(predicate) = &step.postcondition {
                    predicate.validate()?;
                }
                if let Some(region_id) = &step.region_id {
                    validate_stable_id("steps.region_id", region_id)?;
                }
                validate_sorted_ids("steps.annotation_ids", &step.annotation_ids, true)
            },
        )?;
        let method_ids = validate_sorted_records(
            "methods",
            &self.methods,
            |record| &record.id,
            |method| {
                validate_label("methods.label", &method.label)?;
                method.scope.validate("methods.scope")?;
                require_scope_subset("methods.scope", &method.scope, &self.manifest.scope)?;
                validate_stable_id("methods.region_id", &method.region_id)?;
                validate_ordered_references("methods.step_ids", &method.step_ids, &step_ids)
            },
        )?;
        let region_ids = validate_sorted_records(
            "regions",
            &self.regions,
            |record| &record.id,
            |region| {
                validate_label("regions.label", &region.label)?;
                region.scope.validate("regions.scope")?;
                require_scope_subset("regions.scope", &region.scope, &self.manifest.scope)?;
                if let Some(parent) = &region.parent_region_id {
                    validate_stable_id("regions.parent_region_id", parent)?;
                }
                if let Some(predicate) = &region.entry_predicate {
                    predicate.validate()?;
                }
                region.outcome_predicate.validate()?;
                validate_sorted_ids("regions.method_ids", &region.method_ids, false)?;
                if let Some(selected) = &region.selected_method_id {
                    validate_stable_id("regions.selected_method_id", selected)?;
                    if !region.method_ids.contains(selected) {
                        return Err(PlannerContractError::new(
                            "regions.selected_method_id",
                            "must name one of the region's methods",
                        ));
                    }
                }
                Ok(())
            },
        )?;
        let annotation_ids = validate_sorted_records(
            "annotations",
            &self.annotations,
            |record| &record.id,
            |annotation| {
                validate_annotation_target(&annotation.target)?;
                validate_label("annotations.body", &annotation.body)
            },
        )?;
        validate_region_hierarchy(&self.regions, &region_ids)?;
        for region in &self.regions {
            if let Some(parent_id) = &region.parent_region_id {
                let parent = self
                    .regions
                    .iter()
                    .find(|candidate| &candidate.id == parent_id)
                    .ok_or_else(|| {
                        PlannerContractError::new("regions.parent_region_id", "is unknown")
                    })?;
                require_scope_subset("regions.scope", &region.scope, &parent.scope)?;
            }
        }
        for method in &self.methods {
            if !region_ids.contains(method.region_id.as_str()) {
                return Err(PlannerContractError::new(
                    "methods.region_id",
                    format!("references unknown region {}", method.region_id),
                ));
            }
            let region = self
                .regions
                .iter()
                .find(|region| region.id == method.region_id)
                .ok_or_else(|| PlannerContractError::new("methods.region_id", "is unknown"))?;
            if !region.method_ids.contains(&method.id) {
                return Err(PlannerContractError::new(
                    "regions.method_ids",
                    format!("region {} omits method {}", region.id, method.id),
                ));
            }
            require_scope_subset("methods.scope", &method.scope, &region.scope)?;
            for step_id in &method.step_ids {
                let step = self
                    .steps
                    .iter()
                    .find(|step| &step.id == step_id)
                    .ok_or_else(|| PlannerContractError::new("methods.step_ids", "is unknown"))?;
                require_scope_subset("methods.scope", &method.scope, &step.scope)?;
            }
        }
        for region in &self.regions {
            for method_id in &region.method_ids {
                let method = self
                    .methods
                    .iter()
                    .find(|method| &method.id == method_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "regions.method_ids",
                            format!("references unknown method {method_id}"),
                        )
                    })?;
                if method.region_id != region.id {
                    return Err(PlannerContractError::new(
                        "regions.method_ids",
                        "method belongs to a different region",
                    ));
                }
            }
        }
        for step in &self.steps {
            if let Some(region) = &step.region_id
                && !region_ids.contains(region.as_str())
            {
                return Err(PlannerContractError::new(
                    "steps.region_id",
                    format!("references unknown region {region}"),
                ));
            }
            if let Some(region_id) = &step.region_id {
                let region = self
                    .regions
                    .iter()
                    .find(|region| &region.id == region_id)
                    .ok_or_else(|| PlannerContractError::new("steps.region_id", "is unknown"))?;
                require_scope_subset("steps.scope", &step.scope, &region.scope)?;
            }
            for annotation in &step.annotation_ids {
                if !annotation_ids.contains(annotation.as_str()) {
                    return Err(PlannerContractError::new(
                        "steps.annotation_ids",
                        format!("references unknown annotation {annotation}"),
                    ));
                }
            }
        }
        for annotation in &self.annotations {
            let (field, id, known) = match &annotation.target {
                AnnotationTarget::Step { step_id } => {
                    ("annotations.step_id", step_id.as_str(), &step_ids)
                }
                AnnotationTarget::Method { method_id } => {
                    ("annotations.method_id", method_id.as_str(), &method_ids)
                }
                AnnotationTarget::Region { region_id } => {
                    ("annotations.region_id", region_id.as_str(), &region_ids)
                }
                AnnotationTarget::Goal { .. }
                | AnnotationTarget::Fact { .. }
                | AnnotationTarget::Action { .. } => continue,
            };
            if !known.contains(id) {
                return Err(PlannerContractError::new(
                    field,
                    format!("references unknown ID {id}"),
                ));
            }
        }
        for directive in &self.directives {
            match &directive.directive {
                RouteDirectiveKind::PinMethod { method_id }
                | RouteDirectiveKind::BanMethod { method_id }
                | RouteDirectiveKind::PreferMethod { method_id, .. }
                    if !method_ids.contains(method_id.as_str()) =>
                {
                    return Err(PlannerContractError::new(
                        "directives.method_id",
                        format!("references unknown method {method_id}"),
                    ));
                }
                RouteDirectiveKind::PinMethod { method_id }
                | RouteDirectiveKind::BanMethod { method_id }
                | RouteDirectiveKind::PreferMethod { method_id, .. } => {
                    let method = self
                        .methods
                        .iter()
                        .find(|method| &method.id == method_id)
                        .ok_or_else(|| {
                            PlannerContractError::new("directives.method_id", "is unknown")
                        })?;
                    require_scope_subset("directives.scope", &directive.scope, &method.scope)?;
                }
                RouteDirectiveKind::PinAction { .. }
                | RouteDirectiveKind::BanAction { .. }
                | RouteDirectiveKind::PreferAction { .. } => {}
            }
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
    ) -> Result<(), PlannerContractError> {
        self.validate()?;
        facts.validate()?;
        mechanics.validate()?;
        let known_facts = facts
            .aliases
            .iter()
            .map(|record| record.id.as_str())
            .chain(facts.derived_facts.iter().map(|record| record.id.as_str()))
            .collect::<BTreeSet<_>>();
        let known_goals = mechanics
            .goals
            .iter()
            .map(|record| record.id.as_str())
            .collect::<BTreeSet<_>>();
        for goal in &self.goal_ids {
            require_known("goal_ids", goal, &known_goals)?;
        }
        for constraint in &self.constraints {
            validate_constraint_against(
                &constraint.constraint,
                &constraint.scope,
                &known_facts,
                facts,
                mechanics,
            )?;
        }
        for step in &self.steps {
            validate_action_against(&step.action, mechanics)?;
            require_scope_subset(
                "steps.scope",
                &step.scope,
                action_scope(&step.action, mechanics)?,
            )?;
            validate_predicate_facts(step.precondition.as_ref(), &known_facts)?;
            validate_predicate_facts(step.postcondition.as_ref(), &known_facts)?;
            validate_predicate_scopes(step.precondition.as_ref(), &step.scope, facts)?;
            validate_predicate_scopes(step.postcondition.as_ref(), &step.scope, facts)?;
        }
        for region in &self.regions {
            validate_predicate_facts(region.entry_predicate.as_ref(), &known_facts)?;
            validate_predicate_facts(Some(&region.outcome_predicate), &known_facts)?;
            validate_predicate_scopes(region.entry_predicate.as_ref(), &region.scope, facts)?;
            validate_predicate_scopes(Some(&region.outcome_predicate), &region.scope, facts)?;
        }
        for directive in &self.directives {
            match &directive.directive {
                RouteDirectiveKind::PinAction { action }
                | RouteDirectiveKind::BanAction { action }
                | RouteDirectiveKind::PreferAction { action, .. } => {
                    validate_action_against(action, mechanics)?;
                    require_scope_subset(
                        "directives.scope",
                        &directive.scope,
                        action_scope(action, mechanics)?,
                    )?;
                }
                RouteDirectiveKind::PinMethod { .. }
                | RouteDirectiveKind::BanMethod { .. }
                | RouteDirectiveKind::PreferMethod { .. } => {}
            }
        }
        for annotation in &self.annotations {
            match &annotation.target {
                AnnotationTarget::Goal { goal_id } => {
                    require_known("annotations.goal_id", goal_id, &known_goals)?
                }
                AnnotationTarget::Fact { fact_id } => {
                    require_known("annotations.fact_id", fact_id, &known_facts)?
                }
                AnnotationTarget::Action { action } => validate_action_against(action, mechanics)?,
                AnnotationTarget::Step { .. }
                | AnnotationTarget::Method { .. }
                | AnnotationTarget::Region { .. } => {}
            }
        }
        Ok(())
    }

    pub fn validate_against_composed(
        &self,
        catalog: &ComposedPlannerCatalog,
    ) -> Result<(), PlannerContractError> {
        catalog.validate()?;
        self.validate_against(&catalog.facts, &catalog.mechanics)?;
        if let Some(expected) = self.manifest.refinement_stack_sha256 {
            let actual = catalog.refinement_stack.digest()?;
            if expected != actual {
                return Err(PlannerContractError::new(
                    "manifest.refinement_stack_sha256",
                    "does not match the composed catalog",
                ));
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let book: Self = serde_json::from_slice(bytes)?;
        book.validate()?;
        if book.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "route_book",
                "is not canonical JSON",
            ));
        }
        Ok(book)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl RouteBookEditBatch {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != ROUTE_BOOK_EDIT_BATCH_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        if self.expected_route_book_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "expected_route_book_sha256",
                "must be nonzero",
            ));
        }
        if self.edits.is_empty() || self.edits.len() > 4_096 {
            return Err(PlannerContractError::new(
                "edits",
                "must contain between 1 and 4096 commands",
            ));
        }
        for edit in &self.edits {
            validate_edit(edit)?;
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let batch: Self = serde_json::from_slice(bytes)?;
        batch.validate()?;
        if batch.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "route_book_edit_batch",
                "is not canonical JSON",
            ));
        }
        Ok(batch)
    }

    pub fn apply(
        &self,
        book: &RouteBook,
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
    ) -> Result<RouteBook, PlannerContractError> {
        self.validate()?;
        book.validate_against(facts, mechanics)?;
        if book.digest()? != self.expected_route_book_sha256 {
            return Err(PlannerContractError::new(
                "expected_route_book_sha256",
                "does not match the current route book",
            ));
        }
        let mut edited = book.clone();
        for edit in &self.edits {
            apply_edit(&mut edited, edit)?;
        }
        sort_route_book(&mut edited);
        edited.validate_against(facts, mechanics)?;
        Ok(edited)
    }

    pub fn apply_composed(
        &self,
        book: &RouteBook,
        catalog: &ComposedPlannerCatalog,
    ) -> Result<RouteBook, PlannerContractError> {
        book.validate_against_composed(catalog)?;
        let edited = self.apply(book, &catalog.facts, &catalog.mechanics)?;
        edited.validate_against_composed(catalog)?;
        Ok(edited)
    }
}

fn validate_edit(edit: &RouteBookEdit) -> Result<(), PlannerContractError> {
    match edit {
        RouteBookEdit::SetGoalIds { goal_ids } => validate_sorted_ids("goal_ids", goal_ids, false),
        RouteBookEdit::UpsertConstraint { constraint } => {
            validate_stable_id("constraint.id", &constraint.id)?;
            constraint.scope.validate("constraint.scope")?;
            validate_constraint(&constraint.constraint)
        }
        RouteBookEdit::RemoveConstraint { constraint_id } => {
            validate_stable_id("constraint_id", constraint_id)
        }
        RouteBookEdit::UpsertDirective { directive } => {
            validate_stable_id("directive.id", &directive.id)?;
            directive.scope.validate("directive.scope")?;
            validate_directive(&directive.directive)
        }
        RouteBookEdit::RemoveDirective { directive_id } => {
            validate_stable_id("directive_id", directive_id)
        }
        RouteBookEdit::UpsertStep { step } => {
            validate_stable_id("step.id", &step.id)?;
            validate_label("step.label", &step.label)?;
            step.scope.validate("step.scope")?;
            validate_action(&step.action)?;
            if let Some(predicate) = &step.precondition {
                predicate.validate()?;
            }
            if let Some(predicate) = &step.postcondition {
                predicate.validate()?;
            }
            Ok(())
        }
        RouteBookEdit::RemoveStep { step_id } => validate_stable_id("step_id", step_id),
        RouteBookEdit::UpsertMethod { method } => {
            validate_stable_id("method.id", &method.id)?;
            validate_label("method.label", &method.label)?;
            method.scope.validate("method.scope")?;
            validate_stable_id("method.region_id", &method.region_id)?;
            if method.step_ids.is_empty() {
                return Err(PlannerContractError::new(
                    "method.step_ids",
                    "must not be empty",
                ));
            }
            Ok(())
        }
        RouteBookEdit::RemoveMethod { method_id } => validate_stable_id("method_id", method_id),
        RouteBookEdit::UpsertRegion { region } => {
            validate_stable_id("region.id", &region.id)?;
            validate_label("region.label", &region.label)?;
            region.scope.validate("region.scope")?;
            region.outcome_predicate.validate()
        }
        RouteBookEdit::RemoveRegion { region_id }
        | RouteBookEdit::SetSelectedMethod { region_id, .. }
        | RouteBookEdit::SetCollapsePolicy { region_id, .. } => {
            validate_stable_id("region_id", region_id)?;
            if let RouteBookEdit::SetSelectedMethod {
                method_id: Some(method_id),
                ..
            } = edit
            {
                validate_stable_id("method_id", method_id)?;
            }
            Ok(())
        }
        RouteBookEdit::UpsertAnnotation { annotation } => {
            validate_stable_id("annotation.id", &annotation.id)?;
            validate_annotation_target(&annotation.target)?;
            validate_label("annotation.body", &annotation.body)
        }
        RouteBookEdit::RemoveAnnotation { annotation_id } => {
            validate_stable_id("annotation_id", annotation_id)
        }
    }
}

fn apply_edit(book: &mut RouteBook, edit: &RouteBookEdit) -> Result<(), PlannerContractError> {
    match edit {
        RouteBookEdit::SetGoalIds { goal_ids } => book.goal_ids.clone_from(goal_ids),
        RouteBookEdit::UpsertConstraint { constraint } => {
            upsert(&mut book.constraints, constraint.clone(), |record| {
                &record.id
            })
        }
        RouteBookEdit::RemoveConstraint { constraint_id } => {
            remove(&mut book.constraints, constraint_id, |record| &record.id)?
        }
        RouteBookEdit::UpsertDirective { directive } => {
            upsert(&mut book.directives, directive.clone(), |record| &record.id)
        }
        RouteBookEdit::RemoveDirective { directive_id } => {
            remove(&mut book.directives, directive_id, |record| &record.id)?
        }
        RouteBookEdit::UpsertStep { step } => {
            upsert(&mut book.steps, step.clone(), |record| &record.id)
        }
        RouteBookEdit::RemoveStep { step_id } => {
            remove(&mut book.steps, step_id, |record| &record.id)?
        }
        RouteBookEdit::UpsertMethod { method } => {
            upsert(&mut book.methods, method.clone(), |record| &record.id)
        }
        RouteBookEdit::RemoveMethod { method_id } => {
            remove(&mut book.methods, method_id, |record| &record.id)?
        }
        RouteBookEdit::UpsertRegion { region } => {
            upsert(&mut book.regions, region.clone(), |record| &record.id)
        }
        RouteBookEdit::RemoveRegion { region_id } => {
            remove(&mut book.regions, region_id, |record| &record.id)?
        }
        RouteBookEdit::SetSelectedMethod {
            region_id,
            method_id,
        } => {
            let region = book
                .regions
                .iter_mut()
                .find(|region| &region.id == region_id)
                .ok_or_else(|| PlannerContractError::new("region_id", "is unknown"))?;
            region.selected_method_id.clone_from(method_id);
        }
        RouteBookEdit::SetCollapsePolicy {
            region_id,
            collapse_policy,
        } => {
            let region = book
                .regions
                .iter_mut()
                .find(|region| &region.id == region_id)
                .ok_or_else(|| PlannerContractError::new("region_id", "is unknown"))?;
            region.collapse_policy = *collapse_policy;
        }
        RouteBookEdit::UpsertAnnotation { annotation } => {
            upsert(&mut book.annotations, annotation.clone(), |record| {
                &record.id
            })
        }
        RouteBookEdit::RemoveAnnotation { annotation_id } => {
            remove(&mut book.annotations, annotation_id, |record| &record.id)?
        }
    }
    Ok(())
}

fn upsert<T, F>(records: &mut Vec<T>, value: T, get_id: F)
where
    F: Fn(&T) -> &String,
{
    let id = get_id(&value).clone();
    if let Some(existing) = records.iter_mut().find(|record| get_id(record) == &id) {
        *existing = value;
    } else {
        records.push(value);
    }
}

fn remove<T, F>(records: &mut Vec<T>, id: &str, get_id: F) -> Result<(), PlannerContractError>
where
    F: Fn(&T) -> &String,
{
    let before = records.len();
    records.retain(|record| get_id(record) != id);
    if records.len() == before {
        Err(PlannerContractError::new(
            "edit.id",
            format!("cannot remove unknown ID {id}"),
        ))
    } else {
        Ok(())
    }
}

fn sort_route_book(book: &mut RouteBook) {
    book.goal_ids.sort();
    book.constraints
        .sort_by(|left, right| left.id.cmp(&right.id));
    book.directives
        .sort_by(|left, right| left.id.cmp(&right.id));
    book.steps.sort_by(|left, right| left.id.cmp(&right.id));
    book.methods.sort_by(|left, right| left.id.cmp(&right.id));
    book.regions.sort_by(|left, right| left.id.cmp(&right.id));
    book.annotations
        .sort_by(|left, right| left.id.cmp(&right.id));
}

fn validate_constraint(constraint: &PathConstraint) -> Result<(), PlannerContractError> {
    match constraint {
        PathConstraint::RequirePredicate { predicate }
        | PathConstraint::ForbidPredicate { predicate } => predicate.validate(),
        PathConstraint::RequireTechnique { technique_id }
        | PathConstraint::ForbidTechnique { technique_id } => {
            validate_stable_id("constraints.technique_id", technique_id)
        }
        PathConstraint::EvidenceAtLeast { minimum } => {
            validate_stable_id("constraints.minimum", minimum)?;
            if matches!(
                minimum.as_str(),
                "established" | "contested" | "hypothetical"
            ) {
                Ok(())
            } else {
                Err(PlannerContractError::new(
                    "constraints.minimum",
                    "must be established, contested, or hypothetical",
                ))
            }
        }
        PathConstraint::CostAtMost { axis, .. } => validate_stable_id("constraints.axis", axis),
    }
}

fn validate_directive(directive: &RouteDirectiveKind) -> Result<(), PlannerContractError> {
    match directive {
        RouteDirectiveKind::PinAction { action }
        | RouteDirectiveKind::BanAction { action }
        | RouteDirectiveKind::PreferAction { action, .. } => validate_action(action)?,
        RouteDirectiveKind::PinMethod { method_id }
        | RouteDirectiveKind::BanMethod { method_id }
        | RouteDirectiveKind::PreferMethod { method_id, .. } => {
            validate_stable_id("directives.method_id", method_id)?
        }
    }
    match directive {
        RouteDirectiveKind::PreferAction { weight, .. }
        | RouteDirectiveKind::PreferMethod { weight, .. }
            if *weight == 0 =>
        {
            Err(PlannerContractError::new(
                "directives.weight",
                "must be greater than zero",
            ))
        }
        _ => Ok(()),
    }
}

fn validate_action(action: &RouteActionRef) -> Result<(), PlannerContractError> {
    let (field, id) = match action {
        RouteActionRef::Transition { transition_id } => ("action.transition_id", transition_id),
        RouteActionRef::Technique { technique_id } => ("action.technique_id", technique_id),
        RouteActionRef::Resolver { resolver_id } => ("action.resolver_id", resolver_id),
        RouteActionRef::Writer { writer_id } => ("action.writer_id", writer_id),
        RouteActionRef::Microtrace { microtrace_id } => ("action.microtrace_id", microtrace_id),
    };
    validate_stable_id(field, id)
}

fn validate_annotation_target(target: &AnnotationTarget) -> Result<(), PlannerContractError> {
    match target {
        AnnotationTarget::Goal { goal_id } => validate_stable_id("annotations.goal_id", goal_id),
        AnnotationTarget::Fact { fact_id } => validate_stable_id("annotations.fact_id", fact_id),
        AnnotationTarget::Action { action } => validate_action(action),
        AnnotationTarget::Step { step_id } => validate_stable_id("annotations.step_id", step_id),
        AnnotationTarget::Method { method_id } => {
            validate_stable_id("annotations.method_id", method_id)
        }
        AnnotationTarget::Region { region_id } => {
            validate_stable_id("annotations.region_id", region_id)
        }
    }
}

fn validate_action_against(
    action: &RouteActionRef,
    mechanics: &MechanicsCatalog,
) -> Result<(), PlannerContractError> {
    let (field, id, known) = match action {
        RouteActionRef::Transition { transition_id } => (
            "action.transition_id",
            transition_id.as_str(),
            mechanics
                .transitions
                .iter()
                .any(|record| record.id == *transition_id),
        ),
        RouteActionRef::Technique { technique_id } => (
            "action.technique_id",
            technique_id.as_str(),
            mechanics
                .techniques
                .iter()
                .any(|record| record.id == *technique_id),
        ),
        RouteActionRef::Resolver { resolver_id } => (
            "action.resolver_id",
            resolver_id.as_str(),
            mechanics
                .resolvers
                .iter()
                .any(|record| record.id == *resolver_id),
        ),
        RouteActionRef::Writer { writer_id } => (
            "action.writer_id",
            writer_id.as_str(),
            mechanics
                .writers
                .iter()
                .any(|record| record.id == *writer_id),
        ),
        RouteActionRef::Microtrace { microtrace_id } => (
            "action.microtrace_id",
            microtrace_id.as_str(),
            mechanics
                .microtraces
                .iter()
                .any(|record| record.id == *microtrace_id),
        ),
    };
    if known {
        Ok(())
    } else {
        Err(PlannerContractError::new(
            field,
            format!("references unknown action {id}"),
        ))
    }
}

fn action_scope<'a>(
    action: &RouteActionRef,
    mechanics: &'a MechanicsCatalog,
) -> Result<&'a ContextScope, PlannerContractError> {
    let scope = match action {
        RouteActionRef::Transition { transition_id } => mechanics
            .transitions
            .iter()
            .find(|record| record.id == *transition_id)
            .map(|record| &record.scope),
        RouteActionRef::Technique { technique_id } => mechanics
            .techniques
            .iter()
            .find(|record| record.id == *technique_id)
            .map(|record| &record.scope),
        RouteActionRef::Resolver { resolver_id } => mechanics
            .resolvers
            .iter()
            .find(|record| record.id == *resolver_id)
            .map(|record| &record.scope),
        RouteActionRef::Writer { writer_id } => mechanics
            .writers
            .iter()
            .find(|record| record.id == *writer_id)
            .map(|record| &record.scope),
        RouteActionRef::Microtrace { microtrace_id } => mechanics
            .microtraces
            .iter()
            .find(|record| record.id == *microtrace_id)
            .map(|record| &record.scope),
    };
    scope.ok_or_else(|| PlannerContractError::new("action", "references an unknown action"))
}

fn validate_constraint_against(
    constraint: &PathConstraint,
    required_scope: &ContextScope,
    known_facts: &BTreeSet<&str>,
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
) -> Result<(), PlannerContractError> {
    match constraint {
        PathConstraint::RequirePredicate { predicate }
        | PathConstraint::ForbidPredicate { predicate } => {
            validate_predicate_facts(Some(predicate), known_facts)?;
            validate_predicate_scopes(Some(predicate), required_scope, facts)
        }
        PathConstraint::RequireTechnique { technique_id }
        | PathConstraint::ForbidTechnique { technique_id } => {
            if mechanics
                .techniques
                .iter()
                .any(|record| record.id == *technique_id)
            {
                require_scope_subset(
                    "constraints.scope",
                    required_scope,
                    action_scope(
                        &RouteActionRef::Technique {
                            technique_id: technique_id.clone(),
                        },
                        mechanics,
                    )?,
                )
            } else {
                Err(PlannerContractError::new(
                    "constraints.technique_id",
                    format!("references unknown technique {technique_id}"),
                ))
            }
        }
        PathConstraint::EvidenceAtLeast { .. } | PathConstraint::CostAtMost { .. } => Ok(()),
    }
}

fn validate_predicate_scopes(
    predicate: Option<&PredicateExpression>,
    required_scope: &ContextScope,
    facts: &FactCatalog,
) -> Result<(), PlannerContractError> {
    if let Some(predicate) = predicate {
        let mut referenced = BTreeSet::new();
        predicate.referenced_facts(&mut referenced);
        for fact_id in referenced {
            let scope = facts
                .aliases
                .iter()
                .find(|record| record.id == fact_id)
                .map(|record| &record.scope)
                .or_else(|| {
                    facts
                        .derived_facts
                        .iter()
                        .find(|record| record.id == fact_id)
                        .map(|record| &record.scope)
                })
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "predicate.fact_id",
                        format!("references unknown fact {fact_id}"),
                    )
                })?;
            require_scope_subset("predicate.scope", required_scope, scope)?;
        }
    }
    Ok(())
}

fn validate_predicate_facts(
    predicate: Option<&PredicateExpression>,
    known_facts: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    if let Some(predicate) = predicate {
        let mut referenced = BTreeSet::new();
        predicate.referenced_facts(&mut referenced);
        for fact in referenced {
            require_known("predicate.fact_id", &fact, known_facts)?;
        }
    }
    Ok(())
}

fn require_known(
    field: &str,
    id: &str,
    known: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    if known.contains(id) {
        Ok(())
    } else {
        Err(PlannerContractError::new(
            field,
            format!("references unknown ID {id}"),
        ))
    }
}

fn validate_sorted_records<'a, T, F, V>(
    field: &str,
    records: &'a [T],
    get_id: F,
    validate: V,
) -> Result<BTreeSet<&'a str>, PlannerContractError>
where
    F: Fn(&'a T) -> &'a String,
    V: Fn(&T) -> Result<(), PlannerContractError>,
{
    let mut ids = BTreeSet::new();
    let mut previous = None;
    for record in records {
        let id = get_id(record);
        validate_stable_id(&format!("{field}.id"), id)?;
        validate(record)?;
        if !ids.insert(id.as_str()) || previous.is_some_and(|prior: &str| prior >= id.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted by ID",
            ));
        }
        previous = Some(id.as_str());
    }
    Ok(ids)
}

fn validate_sorted_ids(
    field: &str,
    ids: &[String],
    allow_empty: bool,
) -> Result<(), PlannerContractError> {
    if !allow_empty && ids.is_empty() {
        return Err(PlannerContractError::new(field, "must not be empty"));
    }
    let mut previous = None;
    for id in ids {
        validate_stable_id(field, id)?;
        if previous.is_some_and(|prior: &str| prior >= id.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted",
            ));
        }
        previous = Some(id.as_str());
    }
    Ok(())
}

fn validate_ordered_references(
    field: &str,
    ids: &[String],
    known: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    if ids.is_empty() {
        return Err(PlannerContractError::new(field, "must not be empty"));
    }
    let mut seen = BTreeSet::new();
    for id in ids {
        validate_stable_id(field, id)?;
        if !seen.insert(id.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "contains a duplicate step",
            ));
        }
        require_known(field, id, known)?;
    }
    Ok(())
}

fn validate_region_hierarchy(
    regions: &[PlanRegion],
    known: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    let parents = regions
        .iter()
        .filter_map(|region| {
            region
                .parent_region_id
                .as_deref()
                .map(|parent| (region.id.as_str(), parent))
        })
        .collect::<BTreeMap<_, _>>();
    for (id, parent) in &parents {
        if !known.contains(parent) || id == parent {
            return Err(PlannerContractError::new(
                "regions.parent_region_id",
                "must reference a different known region",
            ));
        }
    }
    for start in known.iter().copied() {
        let mut seen = BTreeSet::new();
        let mut cursor = start;
        while let Some(parent) = parents.get(cursor) {
            if !seen.insert(cursor) {
                return Err(PlannerContractError::new(
                    "regions.parent_region_id",
                    "contains a cycle",
                ));
            }
            cursor = parent;
        }
    }
    Ok(())
}

fn require_scope_subset(
    field: &str,
    required: &ContextScope,
    available: &ContextScope,
) -> Result<(), PlannerContractError> {
    if required
        .selectors
        .iter()
        .all(|selector| available.selectors.contains(selector))
    {
        Ok(())
    } else {
        Err(PlannerContractError::new(
            field,
            "contains a context outside its parent scope",
        ))
    }
}

fn validate_version(value: &str) -> Result<(), PlannerContractError> {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || part.parse::<u32>().is_err())
    {
        return Err(PlannerContractError::new(
            "manifest.version",
            "must be a numeric major.minor.patch version",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ContextSelector, ExactContext};
    use crate::logic::{
        DerivedFact, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, RuleEvidence, TruthStatus,
    };
    use crate::transition::{Goal, MECHANICS_CATALOG_SCHEMA, RouteCost, Technique};

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

    fn alternate_scope() -> ContextScope {
        ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([9; 32]),
                    runtime_configuration_sha256: Digest([8; 32]),
                },
            }],
        }
    }

    fn evidence() -> RuleEvidence {
        RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "source.route-book".into(),
                kind: EvidenceKind::CommunityReported,
                source_sha256: Some(Digest([3; 32])),
                note: "Documented route method.".into(),
            }],
        }
    }

    fn catalogs() -> (FactCatalog, MechanicsCatalog) {
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: vec![DerivedFact {
                id: "inventory.fishing-rod".into(),
                label: "Fishing Rod obtained".into(),
                scope: scope(),
                rule: PredicateExpression::True,
                evidence: evidence(),
            }],
        };
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: Vec::new(),
            obligations: Vec::new(),
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: vec![
                Technique {
                    id: "technique.chicken-bypass".into(),
                    label: "Chicken vine bypass".into(),
                    scope: scope(),
                    prerequisites: PredicateExpression::True,
                    operations: Vec::new(),
                    discharged_obligation_ids: Vec::new(),
                    introduced_obligation_ids: Vec::new(),
                    cost: RouteCost {
                        axes: BTreeMap::new(),
                    },
                    evidence: evidence(),
                },
                Technique {
                    id: "technique.ordinary-rod-quest".into(),
                    label: "Ordinary rod quest".into(),
                    scope: scope(),
                    prerequisites: PredicateExpression::True,
                    operations: Vec::new(),
                    discharged_obligation_ids: Vec::new(),
                    introduced_obligation_ids: Vec::new(),
                    cost: RouteCost {
                        axes: BTreeMap::new(),
                    },
                    evidence: evidence(),
                },
            ],
            microtraces: Vec::new(),
            goals: vec![Goal {
                id: "goal.obtain-fishing-rod".into(),
                label: "Obtain Fishing Rod".into(),
                predicate: PredicateExpression::Fact {
                    fact_id: "inventory.fishing-rod".into(),
                },
            }],
        };
        (facts, mechanics)
    }

    fn route_book_fixture() -> RouteBook {
        RouteBook {
            schema: ROUTE_BOOK_SCHEMA.into(),
            manifest: RouteBookManifest {
                id: "route-book.rod-research".into(),
                version: "1.0.0".into(),
                label: "Fishing Rod research".into(),
                author: "Route researchers".into(),
                source: "Curated route references".into(),
                scope: scope(),
                refinement_stack_sha256: None,
            },
            goal_ids: vec!["goal.obtain-fishing-rod".into()],
            constraints: Vec::new(),
            directives: Vec::new(),
            steps: vec![
                ReferenceStep {
                    id: "step.chicken-bypass".into(),
                    label: "Bypass vine man with chicken".into(),
                    scope: scope(),
                    action: RouteActionRef::Technique {
                        technique_id: "technique.chicken-bypass".into(),
                    },
                    precondition: None,
                    postcondition: None,
                    region_id: Some("region.obtain-rod".into()),
                    annotation_ids: Vec::new(),
                },
                ReferenceStep {
                    id: "step.ordinary-quest".into(),
                    label: "Complete ordinary rod quest".into(),
                    scope: scope(),
                    action: RouteActionRef::Technique {
                        technique_id: "technique.ordinary-rod-quest".into(),
                    },
                    precondition: None,
                    postcondition: Some(PredicateExpression::Fact {
                        fact_id: "inventory.fishing-rod".into(),
                    }),
                    region_id: Some("region.obtain-rod".into()),
                    annotation_ids: Vec::new(),
                },
            ],
            methods: vec![
                PlanMethod {
                    id: "method.chicken-mix".into(),
                    label: "Chicken bypass plus ordinary finish".into(),
                    scope: scope(),
                    region_id: "region.obtain-rod".into(),
                    step_ids: vec!["step.chicken-bypass".into(), "step.ordinary-quest".into()],
                },
                PlanMethod {
                    id: "method.ordinary".into(),
                    label: "Ordinary quest".into(),
                    scope: scope(),
                    region_id: "region.obtain-rod".into(),
                    step_ids: vec!["step.ordinary-quest".into()],
                },
            ],
            regions: vec![PlanRegion {
                id: "region.obtain-rod".into(),
                label: "Obtain Fishing Rod".into(),
                scope: scope(),
                parent_region_id: None,
                entry_predicate: None,
                outcome_predicate: PredicateExpression::Fact {
                    fact_id: "inventory.fishing-rod".into(),
                },
                method_ids: vec!["method.chicken-mix".into(), "method.ordinary".into()],
                selected_method_id: None,
                collapse_policy: CollapsePolicy::OnlyContinuationEquivalent,
            }],
            annotations: Vec::new(),
        }
    }

    #[test]
    fn route_book_collapses_interchangeable_methods_without_authoring_effects() {
        let (facts, mechanics) = catalogs();
        let book = route_book_fixture();
        book.validate_against(&facts, &mechanics).unwrap();
        assert_eq!(book.regions[0].method_ids.len(), 2);
        assert_eq!(
            book.regions[0].collapse_policy,
            CollapsePolicy::OnlyContinuationEquivalent
        );
        let bytes = book.canonical_bytes().unwrap();
        assert_eq!(RouteBook::decode_canonical(&bytes).unwrap(), book);
    }

    #[test]
    fn unknown_actions_fail_against_catalog_without_becoming_mechanics() {
        let (facts, mechanics) = catalogs();
        let mut book = route_book_fixture();
        book.steps[0].action = RouteActionRef::Technique {
            technique_id: "technique.imaginary".into(),
        };
        assert_eq!(
            book.validate_against(&facts, &mechanics)
                .unwrap_err()
                .field(),
            "action.technique_id"
        );
    }

    #[test]
    fn region_cycles_and_zero_weight_preferences_fail_closed() {
        let mut book = route_book_fixture();
        book.regions[0].parent_region_id = Some("region.obtain-rod".into());
        assert_eq!(
            book.validate().unwrap_err().field(),
            "regions.parent_region_id"
        );

        let mut book = route_book_fixture();
        book.directives.push(RouteDirective {
            id: "directive.prefer".into(),
            scope: scope(),
            directive: RouteDirectiveKind::PreferMethod {
                method_id: "method.ordinary".into(),
                weight: 0,
            },
        });
        assert_eq!(book.validate().unwrap_err().field(), "directives.weight");
    }

    #[test]
    fn step_context_cannot_leak_an_action_into_an_unsupported_build() {
        let (facts, mechanics) = catalogs();
        let mut book = route_book_fixture();
        book.manifest
            .scope
            .selectors
            .extend(alternate_scope().selectors);
        book.regions[0].scope = book.manifest.scope.clone();
        book.methods.remove(0);
        book.regions[0].method_ids = vec!["method.ordinary".into()];
        book.steps[0].scope = alternate_scope();
        book.validate().unwrap();
        assert_eq!(
            book.validate_against(&facts, &mechanics)
                .unwrap_err()
                .field(),
            "steps.scope"
        );
    }

    #[test]
    fn revision_checked_edit_batches_are_atomic_and_revalidated() {
        let (facts, mechanics) = catalogs();
        let book = route_book_fixture();
        let batch = RouteBookEditBatch {
            schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
            expected_route_book_sha256: book.digest().unwrap(),
            edits: vec![
                RouteBookEdit::SetSelectedMethod {
                    region_id: "region.obtain-rod".into(),
                    method_id: Some("method.ordinary".into()),
                },
                RouteBookEdit::SetCollapsePolicy {
                    region_id: "region.obtain-rod".into(),
                    collapse_policy: CollapsePolicy::ShowResidualDifferences,
                },
            ],
        };
        let edited = batch.apply(&book, &facts, &mechanics).unwrap();
        assert_eq!(
            edited.regions[0].selected_method_id.as_deref(),
            Some("method.ordinary")
        );
        assert_eq!(
            edited.regions[0].collapse_policy,
            CollapsePolicy::ShowResidualDifferences
        );
        assert_ne!(edited.digest().unwrap(), book.digest().unwrap());

        let mut stale = batch;
        stale.expected_route_book_sha256 = Digest([9; 32]);
        assert_eq!(
            stale.apply(&book, &facts, &mechanics).unwrap_err().field(),
            "expected_route_book_sha256"
        );
    }

    #[test]
    fn invalid_edit_batch_does_not_partially_mutate_the_source_book() {
        let (facts, mechanics) = catalogs();
        let book = route_book_fixture();
        let original_digest = book.digest().unwrap();
        let batch = RouteBookEditBatch {
            schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
            expected_route_book_sha256: original_digest,
            edits: vec![RouteBookEdit::RemoveStep {
                step_id: "step.ordinary-quest".into(),
            }],
        };
        assert_eq!(
            batch.apply(&book, &facts, &mechanics).unwrap_err().field(),
            "methods.step_ids"
        );
        assert_eq!(book.digest().unwrap(), original_digest);
    }
}
