//! Versioned refinement packs and deterministic theorycraft overlays.

use crate::artifact::Digest;
use crate::logic::{
    ComparisonOperator, ContextScope, DerivedFact, FactCatalog, FriendlyAlias, PredicateExpression,
    RuleEvidence, ValueReference,
};
use crate::state::SemanticLifetime;
use crate::state::{SceneLocation, StateValue};
use crate::transition::{
    ActorReconstructionRule, CandidateTransition, FeasibilityObligation, GateRule, Goal,
    MechanicsCatalog, Obstruction, ObstructionResolver, ReaderRule, ResolutionKind, RouteCost,
    StateOperation, Technique, TransitionKind, WitnessedMicrotrace, WriterRule,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

mod composition;
use composition::*;

pub const REFINEMENT_PACK_SCHEMA: &str = "dusklight.route-planner.refinement-pack/v15";
pub const REFINEMENT_STACK_SCHEMA: &str = "dusklight.route-planner.refinement-stack/v2";
pub const COMPOSED_CATALOG_SCHEMA: &str = "dusklight.route-planner.composed-catalog/v16";
pub const REFINEMENT_DIAGNOSTIC_REPORT_SCHEMA: &str =
    "dusklight.route-planner.refinement-diagnostic-report/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementDiagnosticReport {
    pub schema: String,
    pub valid: bool,
    pub diagnostics: Vec<RefinementDiagnostic>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementDiagnostic {
    pub pack_id: Option<String>,
    pub field: String,
    pub detail: String,
    pub suggestion: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackDependency {
    pub pack_id: String,
    pub pack_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementPackManifest {
    pub id: String,
    pub version: String,
    pub author: String,
    pub source: String,
    pub scope: ContextScope,
    pub precedence: i32,
    pub dependencies: Vec<PackDependency>,
    pub conflicts: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplacementKind {
    Replace,
    Supersede,
    Disable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchCardinality {
    ExactlyOne,
    OneOrMore,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneLocationSelector {
    pub stage: Option<String>,
    pub room: Option<i8>,
    pub layer: Option<i8>,
    pub spawn: Option<i16>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObstructionActionSelector {
    ActionId {
        action_id: String,
    },
    Transition {
        transition_kind: Option<TransitionKind>,
        approach_id: Option<String>,
        source: Option<SceneLocationSelector>,
        destination: Option<SceneLocationSelector>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredObstruction {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub action_selector: ObstructionActionSelector,
    pub match_cardinality: MatchCardinality,
    pub active_when: PredicateExpression,
    pub obligation_ids: Vec<String>,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledObstructionBinding {
    pub authored_obstruction_id: String,
    pub compiled_obstruction_id: String,
    pub action_id: String,
    pub action_selector: ObstructionActionSelector,
    pub match_cardinality: MatchCardinality,
    pub source_pack_id: String,
    pub source_rule_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RefinementOperation {
    AddTransition {
        transition: CandidateTransition,
    },
    AddObligation {
        obligation: FeasibilityObligation,
    },
    AddObstruction {
        obstruction: Obstruction,
    },
    BindObstruction {
        obstruction: AuthoredObstruction,
    },
    AddTechnique {
        technique: Technique,
    },
    AddResolver {
        resolver: ObstructionResolver,
    },
    AddWriter {
        writer: WriterRule,
    },
    AddGate {
        gate: GateRule,
    },
    AddReader {
        reader: ReaderRule,
    },
    AddReconstructionRule {
        reconstruction_rule: ActorReconstructionRule,
    },
    AddMicrotrace {
        microtrace: WitnessedMicrotrace,
    },
    AddGoal {
        goal: Goal,
    },
    AddAlias {
        alias: FriendlyAlias,
    },
    AddDerivedFact {
        fact: DerivedFact,
    },
    ComponentTransform {
        prerequisite: PredicateExpression,
        operations: Vec<StateOperation>,
    },
    SuppressWriter {
        writer_id: String,
        when: PredicateExpression,
    },
    AssumeObstructionAbsent {
        obstruction_id: String,
        when: PredicateExpression,
    },
    ReplaceRecord {
        target_id: String,
        replacement_kind: ReplacementKind,
        replacement_rule_id: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementRule {
    pub id: String,
    pub label: String,
    pub operation: RefinementOperation,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementPack {
    pub schema: String,
    pub manifest: RefinementPackManifest,
    pub rules: Vec<RefinementRule>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementStackEntry {
    pub layer: RefinementLayer,
    pub precedence: i32,
    pub pack_id: String,
    pub pack_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefinementLayer {
    EnabledPack,
    RouteLocal,
    EphemeralWhatIf,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementLayers {
    pub enabled_packs: Vec<RefinementPack>,
    pub route_local_overlays: Vec<RefinementPack>,
    pub ephemeral_what_if_overlays: Vec<RefinementPack>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementStack {
    pub schema: String,
    pub entries: Vec<RefinementStackEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComposedPlannerCatalog {
    pub schema: String,
    pub base_fact_catalog_sha256: Digest,
    pub base_mechanics_catalog_sha256: Digest,
    pub facts: FactCatalog,
    pub mechanics: MechanicsCatalog,
    pub refinement_stack: RefinementStack,
    pub obstruction_bindings: Vec<CompiledObstructionBinding>,
}

impl RefinementPackManifest {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("manifest.id", &self.id)?;
        validate_version(&self.version)?;
        validate_label("manifest.author", &self.author)?;
        validate_label("manifest.source", &self.source)?;
        self.scope.validate("manifest.scope")?;
        validate_dependencies(&self.dependencies)?;
        validate_ids("manifest.conflicts", &self.conflicts, true)?;
        if self.conflicts.iter().any(|id| id == &self.id)
            || self.dependencies.iter().any(|item| item.pack_id == self.id)
        {
            return Err(PlannerContractError::new(
                "manifest",
                "a pack cannot depend on or conflict with itself",
            ));
        }
        Ok(())
    }
}

impl RefinementRule {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("rules.id", &self.id)?;
        validate_label("rules.label", &self.label)?;
        self.evidence.validate("rules.evidence")?;
        match &self.operation {
            RefinementOperation::AddTransition { transition } => {
                // Full cross-reference validation occurs after the pack is composed.
                validate_stable_id("rules.transition.id", &transition.id)?;
                transition.scope.validate("rules.transition.scope")?;
                transition.activation.hard_guards.validate()
            }
            RefinementOperation::AddObligation { obligation } => {
                validate_stable_id("rules.obligation.id", &obligation.id)?;
                obligation.scope.validate("rules.obligation.scope")?;
                obligation.evidence.validate("rules.obligation.evidence")
            }
            RefinementOperation::AddObstruction { obstruction } => {
                validate_stable_id("rules.obstruction.id", &obstruction.id)?;
                validate_stable_id(
                    "rules.obstruction.blocked_action_id",
                    &obstruction.blocked_action_id,
                )?;
                obstruction.scope.validate("rules.obstruction.scope")?;
                obstruction.active_when.validate()?;
                obstruction.evidence.validate("rules.obstruction.evidence")
            }
            RefinementOperation::BindObstruction { obstruction } => {
                validate_authored_obstruction(obstruction)
            }
            RefinementOperation::AddTechnique { technique } => {
                validate_stable_id("rules.technique.id", &technique.id)?;
                technique.scope.validate("rules.technique.scope")?;
                technique.prerequisites.validate()?;
                validate_operations(&technique.operations)
            }
            RefinementOperation::AddResolver { resolver } => {
                validate_stable_id("rules.resolver.id", &resolver.id)?;
                validate_stable_id("rules.resolver.obstruction_id", &resolver.obstruction_id)?;
                resolver.scope.validate("rules.resolver.scope")?;
                resolver.applicable_when.validate()?;
                validate_operations(&resolver.operations)
            }
            RefinementOperation::AddWriter { writer } => {
                validate_stable_id("rules.writer.id", &writer.id)?;
                writer.scope.validate("rules.writer.scope")?;
                writer.activation.validate()?;
                writer.operation.validate()
            }
            RefinementOperation::AddGate { gate } => {
                validate_stable_id("rules.gate.id", &gate.id)?;
                gate.scope.validate("rules.gate.scope")?;
                gate.active_when.validate()?;
                gate.evidence.validate("rules.gate.evidence")
            }
            RefinementOperation::AddReader { reader } => {
                validate_stable_id("rules.reader.id", &reader.id)?;
                reader.scope.validate("rules.reader.scope")?;
                reader.evidence.validate("rules.reader.evidence")
            }
            RefinementOperation::AddReconstructionRule {
                reconstruction_rule,
            } => {
                validate_stable_id("rules.reconstruction_rule.id", &reconstruction_rule.id)?;
                reconstruction_rule
                    .scope
                    .validate("rules.reconstruction_rule.scope")?;
                reconstruction_rule.instantiate_when.validate()?;
                validate_operations(&reconstruction_rule.initialization_operations)?;
                reconstruction_rule
                    .evidence
                    .validate("rules.reconstruction_rule.evidence")
            }
            RefinementOperation::AddMicrotrace { microtrace } => {
                validate_stable_id("rules.microtrace.id", &microtrace.id)?;
                microtrace.scope.validate("rules.microtrace.scope")?;
                microtrace.precondition.validate()?;
                validate_operations(&microtrace.operations)?;
                microtrace.postcondition.validate()
            }
            RefinementOperation::AddGoal { goal } => {
                validate_stable_id("rules.goal.id", &goal.id)?;
                goal.predicate.validate()
            }
            RefinementOperation::AddAlias { alias } => {
                validate_stable_id("rules.alias.id", &alias.id)?;
                alias.scope.validate("rules.alias.scope")
            }
            RefinementOperation::AddDerivedFact { fact } => {
                validate_stable_id("rules.fact.id", &fact.id)?;
                fact.scope.validate("rules.fact.scope")?;
                fact.rule.validate()
            }
            RefinementOperation::ComponentTransform {
                prerequisite,
                operations,
            } => {
                prerequisite.validate()?;
                if operations.is_empty() {
                    return Err(PlannerContractError::new(
                        "rules.operations",
                        "component transform must contain at least one operation",
                    ));
                }
                validate_operations(operations)
            }
            RefinementOperation::SuppressWriter { writer_id, when } => {
                validate_stable_id("rules.writer_id", writer_id)?;
                when.validate()
            }
            RefinementOperation::AssumeObstructionAbsent {
                obstruction_id,
                when,
            } => {
                validate_stable_id("rules.obstruction_id", obstruction_id)?;
                when.validate()
            }
            RefinementOperation::ReplaceRecord {
                target_id,
                replacement_kind,
                replacement_rule_id,
            } => {
                validate_stable_id("rules.target_id", target_id)?;
                match replacement_kind {
                    ReplacementKind::Replace | ReplacementKind::Supersede => {
                        let replacement = replacement_rule_id.as_ref().ok_or_else(|| {
                            PlannerContractError::new(
                                "rules.replacement_rule_id",
                                "is required for replace or supersede",
                            )
                        })?;
                        validate_stable_id("rules.replacement_rule_id", replacement)
                    }
                    ReplacementKind::Disable if replacement_rule_id.is_some() => {
                        Err(PlannerContractError::new(
                            "rules.replacement_rule_id",
                            "must be absent when disabling a record",
                        ))
                    }
                    ReplacementKind::Disable => Ok(()),
                }
            }
        }
    }
}

impl RefinementPack {
    pub fn diagnose(&self) -> RefinementDiagnosticReport {
        diagnose_refinement_packs(std::slice::from_ref(self))
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != REFINEMENT_PACK_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        self.manifest.validate()?;
        if self.rules.is_empty() || self.rules.len() > 16_384 {
            return Err(PlannerContractError::new(
                "rules",
                "must contain between 1 and 16384 records",
            ));
        }
        let mut previous = None;
        let mut ids = BTreeSet::new();
        for rule in &self.rules {
            rule.validate()?;
            if !ids.insert(rule.id.as_str())
                || previous.is_some_and(|prior: &str| prior >= rule.id.as_str())
            {
                return Err(PlannerContractError::new(
                    "rules",
                    "must be unique and sorted by ID",
                ));
            }
            previous = Some(rule.id.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let pack: Self = serde_json::from_slice(bytes)?;
        pack.validate()?;
        if pack.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "refinement_pack",
                "is not canonical JSON",
            ));
        }
        Ok(pack)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

pub fn diagnose_refinement_packs(packs: &[RefinementPack]) -> RefinementDiagnosticReport {
    let mut diagnostics = Vec::new();
    let mut ids = BTreeMap::<&str, Vec<usize>>::new();
    let mut valid_digests = BTreeMap::new();
    for (pack_index, pack) in packs.iter().enumerate() {
        let pack_id = (!pack.manifest.id.is_empty()).then(|| pack.manifest.id.clone());
        if pack.schema != REFINEMENT_PACK_SCHEMA {
            diagnostics.push(diagnostic(pack_id.clone(), "schema", "is unsupported"));
        }
        if let Err(error) = pack.manifest.validate() {
            diagnostics.push(diagnostic_from_error(pack_id.clone(), error));
        }
        if pack.rules.is_empty() || pack.rules.len() > 16_384 {
            diagnostics.push(diagnostic(
                pack_id.clone(),
                "rules",
                "must contain between 1 and 16384 records",
            ));
        }
        let mut prior = None;
        let mut rule_ids = BTreeMap::<&str, Vec<usize>>::new();
        for (rule_index, rule) in pack.rules.iter().enumerate() {
            if let Err(error) = rule.validate() {
                let mut row = diagnostic_from_error(pack_id.clone(), error);
                row.field = format!("rules[{rule_index}].{}", row.field);
                diagnostics.push(row);
            }
            rule_ids
                .entry(rule.id.as_str())
                .or_default()
                .push(rule_index);
            if prior.is_some_and(|prior: &str| prior >= rule.id.as_str()) {
                diagnostics.push(diagnostic(
                    pack_id.clone(),
                    format!("rules[{rule_index}].id"),
                    "is not strictly sorted after the preceding rule ID",
                ));
            }
            prior = Some(rule.id.as_str());
        }
        for (id, indexes) in rule_ids {
            if indexes.len() > 1 {
                diagnostics.push(diagnostic(
                    pack_id.clone(),
                    "rules.id",
                    format!("duplicate rule ID {id} at indexes {indexes:?}"),
                ));
            }
        }
        ids.entry(pack.manifest.id.as_str())
            .or_default()
            .push(pack_index);
        if pack.validate().is_ok() {
            if let Ok(digest) = pack.digest() {
                valid_digests.insert(pack.manifest.id.as_str(), digest);
            }
        }
    }
    for (id, indexes) in &ids {
        if indexes.len() > 1 {
            diagnostics.push(diagnostic(
                Some((*id).into()),
                "manifest.id",
                format!("duplicate pack ID occurs at indexes {indexes:?}"),
            ));
        }
    }
    for pack in packs {
        let pack_id = Some(pack.manifest.id.clone());
        for dependency in &pack.manifest.dependencies {
            match valid_digests.get(dependency.pack_id.as_str()) {
                None => diagnostics.push(diagnostic(
                    pack_id.clone(),
                    "manifest.dependencies",
                    format!("missing valid pack {}", dependency.pack_id),
                )),
                Some(actual) if *actual != dependency.pack_sha256 => diagnostics.push(diagnostic(
                    pack_id.clone(),
                    "manifest.dependencies",
                    format!("digest mismatch for pack {}", dependency.pack_id),
                )),
                Some(_) => {}
            }
        }
        for conflict in &pack.manifest.conflicts {
            if ids.contains_key(conflict.as_str()) {
                diagnostics.push(diagnostic(
                    pack_id.clone(),
                    "manifest.conflicts",
                    format!("packs {} and {conflict} conflict", pack.manifest.id),
                ));
            }
        }
    }
    if diagnostics.is_empty() {
        if let Err(error) = RefinementStack::build(packs) {
            diagnostics.push(diagnostic_from_error(None, error));
        }
    }
    diagnostics.sort();
    diagnostics.dedup();
    RefinementDiagnosticReport {
        schema: REFINEMENT_DIAGNOSTIC_REPORT_SCHEMA.into(),
        valid: diagnostics.is_empty(),
        diagnostics,
    }
}

fn diagnostic_from_error(
    pack_id: Option<String>,
    error: PlannerContractError,
) -> RefinementDiagnostic {
    diagnostic(pack_id, error.field(), error.detail())
}

fn diagnostic(
    pack_id: Option<String>,
    field: impl Into<String>,
    detail: impl Into<String>,
) -> RefinementDiagnostic {
    let field = field.into();
    let detail = detail.into();
    let suggestion = diagnostic_suggestion(&field, &detail);
    RefinementDiagnostic {
        pack_id,
        field,
        detail,
        suggestion,
    }
}

fn diagnostic_suggestion(field: &str, detail: &str) -> String {
    if detail.contains("sorted") || detail.contains("duplicate") {
        "Sort records by stable ID and rename or remove duplicates before exporting.".into()
    } else if detail.contains("missing") || detail.contains("absent") {
        "Add the referenced record or dependency with its exact canonical digest.".into()
    } else if detail.contains("digest mismatch") {
        "Re-export the dependency and update this reference to its exact canonical digest.".into()
    } else if field.contains("conflicts") {
        "Disable one conflicting pack or author an explicit replacement pack.".into()
    } else if field.contains("scope") {
        "Select an exact supported context or an explicitly evidenced equivalence selector.".into()
    } else if field.contains("evidence") {
        "Attach a typed evidence record appropriate to the declared truth status.".into()
    } else if field.contains("schema") {
        format!("Set schema to {REFINEMENT_PACK_SCHEMA} before canonical export.")
    } else {
        "Correct this field according to the refinement-pack contract and diagnose again.".into()
    }
}

impl RefinementStack {
    pub fn build(packs: &[RefinementPack]) -> Result<Self, PlannerContractError> {
        Self::build_layered(&RefinementLayers {
            enabled_packs: packs.to_vec(),
            ..RefinementLayers::default()
        })
    }

    pub fn build_layered(layers: &RefinementLayers) -> Result<Self, PlannerContractError> {
        let layered_packs = layers.iter().collect::<Vec<_>>();
        let mut by_id = BTreeMap::new();
        let mut digests = BTreeMap::new();
        let mut pack_layers = BTreeMap::new();
        for &(layer, pack) in &layered_packs {
            pack.validate()?;
            if by_id.insert(pack.manifest.id.as_str(), pack).is_some() {
                return Err(PlannerContractError::new(
                    "refinement_layers",
                    "contains duplicate pack IDs across layers",
                ));
            }
            pack_layers.insert(pack.manifest.id.as_str(), layer);
            digests.insert(pack.manifest.id.as_str(), pack.digest()?);
        }
        for &(layer, pack) in &layered_packs {
            for dependency in &pack.manifest.dependencies {
                let actual = digests.get(dependency.pack_id.as_str()).ok_or_else(|| {
                    PlannerContractError::new(
                        "manifest.dependencies",
                        format!("missing pack {}", dependency.pack_id),
                    )
                })?;
                if *actual != dependency.pack_sha256 {
                    return Err(PlannerContractError::new(
                        "manifest.dependencies",
                        format!("digest mismatch for pack {}", dependency.pack_id),
                    ));
                }
                if pack_layers[dependency.pack_id.as_str()] > layer {
                    return Err(PlannerContractError::new(
                        "manifest.dependencies",
                        format!(
                            "pack {} cannot depend on later-layer pack {}",
                            pack.manifest.id, dependency.pack_id
                        ),
                    ));
                }
            }
            for conflict in &pack.manifest.conflicts {
                if by_id.contains_key(conflict.as_str()) {
                    return Err(PlannerContractError::new(
                        "manifest.conflicts",
                        format!("packs {} and {conflict} conflict", pack.manifest.id),
                    ));
                }
            }
        }
        reject_dependency_cycles(&by_id)?;
        let mut entries = layered_packs
            .iter()
            .map(|(layer, pack)| RefinementStackEntry {
                layer: *layer,
                precedence: pack.manifest.precedence,
                pack_id: pack.manifest.id.clone(),
                pack_sha256: digests[pack.manifest.id.as_str()],
            })
            .collect::<Vec<_>>();
        entries.sort();
        let positions = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.pack_id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        for (_, pack) in &layered_packs {
            for dependency in &pack.manifest.dependencies {
                if positions[dependency.pack_id.as_str()] >= positions[pack.manifest.id.as_str()] {
                    return Err(PlannerContractError::new(
                        "manifest.dependencies",
                        format!(
                            "pack {} dependency {} must sort earlier by layer and precedence",
                            pack.manifest.id, dependency.pack_id
                        ),
                    ));
                }
            }
        }
        let stack = Self {
            schema: REFINEMENT_STACK_SCHEMA.into(),
            entries,
        };
        stack.validate()?;
        Ok(stack)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != REFINEMENT_STACK_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        let mut previous = None;
        for entry in &self.entries {
            validate_stable_id("entries.pack_id", &entry.pack_id)?;
            if entry.pack_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "entries.pack_sha256",
                    "must be nonzero",
                ));
            }
            if previous.is_some_and(|prior: &RefinementStackEntry| prior >= entry) {
                return Err(PlannerContractError::new(
                    "entries",
                    "must be unique and sorted by layer, precedence, ID, and digest",
                ));
            }
            previous = Some(entry);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let stack: Self = serde_json::from_slice(bytes)?;
        stack.validate()?;
        if stack.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "refinement_stack",
                "is not canonical JSON",
            ));
        }
        Ok(stack)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl RefinementLayers {
    pub fn iter(&self) -> impl Iterator<Item = (RefinementLayer, &RefinementPack)> {
        self.enabled_packs
            .iter()
            .map(|pack| (RefinementLayer::EnabledPack, pack))
            .chain(
                self.route_local_overlays
                    .iter()
                    .map(|pack| (RefinementLayer::RouteLocal, pack)),
            )
            .chain(
                self.ephemeral_what_if_overlays
                    .iter()
                    .map(|pack| (RefinementLayer::EphemeralWhatIf, pack)),
            )
    }
}

impl ComposedPlannerCatalog {
    pub fn compose(
        base_facts: &FactCatalog,
        base_mechanics: &MechanicsCatalog,
        packs: &[RefinementPack],
    ) -> Result<Self, PlannerContractError> {
        Self::compose_layered(
            base_facts,
            base_mechanics,
            &RefinementLayers {
                enabled_packs: packs.to_vec(),
                ..RefinementLayers::default()
            },
        )
    }

    pub fn compose_layered(
        base_facts: &FactCatalog,
        base_mechanics: &MechanicsCatalog,
        layers: &RefinementLayers,
    ) -> Result<Self, PlannerContractError> {
        base_facts.validate()?;
        base_mechanics.validate()?;
        let refinement_stack = RefinementStack::build_layered(layers)?;
        let by_id = layers
            .iter()
            .map(|(_, pack)| (pack.manifest.id.as_str(), pack))
            .collect::<BTreeMap<_, _>>();
        let mut facts = base_facts.clone();
        let mut mechanics = base_mechanics.clone();

        for entry in &refinement_stack.entries {
            let pack = by_id[entry.pack_id.as_str()];
            apply_replacements(pack, &mut facts, &mut mechanics)?;
            for rule in &pack.rules {
                apply_addition(pack, rule, &mut facts, &mut mechanics)?;
            }
        }
        let obstruction_bindings =
            compile_obstruction_bindings(&refinement_stack, &by_id, &mut mechanics)?;
        sort_catalogs(&mut facts, &mut mechanics);
        let composed = Self {
            schema: COMPOSED_CATALOG_SCHEMA.into(),
            base_fact_catalog_sha256: base_facts.digest()?,
            base_mechanics_catalog_sha256: base_mechanics.digest()?,
            facts,
            mechanics,
            refinement_stack,
            obstruction_bindings,
        };
        composed.validate()?;
        Ok(composed)
    }

    /// Extends an already composed catalog with additive, explicitly
    /// hypothetical editor overlays. This deliberately does not attempt to
    /// reconstruct the source packs behind the existing stack: the current
    /// catalog is the immutable base, and only the two bounded theorycraft
    /// operations accepted by the workbench may be appended.
    pub fn extend_ephemeral_what_if(
        &self,
        packs: &[RefinementPack],
    ) -> Result<Self, PlannerContractError> {
        self.validate()?;
        let mut composed = self.clone();
        for pack in packs {
            pack.validate()?;
            for rule in &pack.rules {
                if !matches!(
                    rule.operation,
                    RefinementOperation::ComponentTransform { .. }
                        | RefinementOperation::AssumeObstructionAbsent { .. }
                ) {
                    return Err(PlannerContractError::new(
                        "rules.operation",
                        "ephemeral editor overlays may only transform components or assume an obstruction absent",
                    ));
                }
            }

            let entries = &composed.refinement_stack.entries;
            if entries
                .iter()
                .any(|entry| entry.pack_id == pack.manifest.id)
            {
                return Err(PlannerContractError::new(
                    "manifest.id",
                    format!("duplicate pack ID {}", pack.manifest.id),
                ));
            }
            if let Some(conflict) = pack.manifest.conflicts.iter().find(|id| {
                entries
                    .iter()
                    .any(|entry| entry.pack_id.as_str() == id.as_str())
            }) {
                return Err(PlannerContractError::new(
                    "manifest.conflicts",
                    format!(
                        "pack {} conflicts with active pack {conflict}",
                        pack.manifest.id
                    ),
                ));
            }
            for dependency in &pack.manifest.dependencies {
                let entry = entries
                    .iter()
                    .find(|entry| entry.pack_id == dependency.pack_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "manifest.dependencies",
                            format!("missing pack {}", dependency.pack_id),
                        )
                    })?;
                if entry.pack_sha256 != dependency.pack_sha256 {
                    return Err(PlannerContractError::new(
                        "manifest.dependencies",
                        format!("digest mismatch for pack {}", dependency.pack_id),
                    ));
                }
            }
            if composed
                .refinement_stack
                .entries
                .iter()
                .filter(|entry| entry.layer == RefinementLayer::EphemeralWhatIf)
                .any(|entry| entry.precedence >= pack.manifest.precedence)
            {
                return Err(PlannerContractError::new(
                    "manifest.precedence",
                    "must be greater than every active ephemeral what-if overlay",
                ));
            }

            for rule in &pack.rules {
                apply_addition(pack, rule, &mut composed.facts, &mut composed.mechanics)?;
            }
            composed
                .refinement_stack
                .entries
                .push(RefinementStackEntry {
                    layer: RefinementLayer::EphemeralWhatIf,
                    precedence: pack.manifest.precedence,
                    pack_id: pack.manifest.id.clone(),
                    pack_sha256: pack.digest()?,
                });
            composed.refinement_stack.entries.sort();
            sort_catalogs(&mut composed.facts, &mut composed.mechanics);
            composed.validate()?;
        }
        Ok(composed)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != COMPOSED_CATALOG_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        if self.base_fact_catalog_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "base_fact_catalog_sha256",
                "must be nonzero",
            ));
        }
        if self.base_mechanics_catalog_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "base_mechanics_catalog_sha256",
                "must be nonzero",
            ));
        }
        self.facts.validate()?;
        self.mechanics.validate()?;
        self.refinement_stack.validate()?;
        validate_compiled_obstruction_bindings(self)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let catalog: Self = serde_json::from_slice(bytes)?;
        catalog.validate()?;
        if catalog.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "composed_catalog",
                "is not canonical JSON",
            ));
        }
        Ok(catalog)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

#[cfg(test)]
#[path = "refinement_tests.rs"]
mod tests;
