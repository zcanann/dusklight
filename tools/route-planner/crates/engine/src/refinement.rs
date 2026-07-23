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

fn apply_replacements(
    pack: &RefinementPack,
    facts: &mut FactCatalog,
    mechanics: &mut MechanicsCatalog,
) -> Result<(), PlannerContractError> {
    for rule in &pack.rules {
        let RefinementOperation::ReplaceRecord {
            target_id,
            replacement_kind,
            replacement_rule_id,
        } = &rule.operation
        else {
            continue;
        };
        let removed = remove_record(facts, mechanics, target_id);
        if removed == 0 {
            return Err(PlannerContractError::new(
                "rules.target_id",
                format!("references absent record {target_id}"),
            ));
        }
        if removed > 1 {
            return Err(PlannerContractError::new(
                "rules.target_id",
                format!("record ID {target_id} is ambiguous across catalogs"),
            ));
        }
        if matches!(
            replacement_kind,
            ReplacementKind::Replace | ReplacementKind::Supersede
        ) {
            let replacement_id = replacement_rule_id.as_ref().ok_or_else(|| {
                PlannerContractError::new(
                    "rules.replacement_rule_id",
                    "is required for replace or supersede",
                )
            })?;
            let replacement = pack
                .rules
                .iter()
                .find(|candidate| candidate.id == *replacement_id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "rules.replacement_rule_id",
                        "must reference a rule in the same pack",
                    )
                })?;
            if matches!(
                replacement.operation,
                RefinementOperation::ReplaceRecord { .. }
            ) {
                return Err(PlannerContractError::new(
                    "rules.replacement_rule_id",
                    "cannot reference another replacement operation",
                ));
            }
        }
    }
    Ok(())
}

fn apply_addition(
    pack: &RefinementPack,
    rule: &RefinementRule,
    facts: &mut FactCatalog,
    mechanics: &mut MechanicsCatalog,
) -> Result<(), PlannerContractError> {
    match &rule.operation {
        RefinementOperation::AddTransition { transition } => {
            mechanics.transitions.push(transition.clone())
        }
        RefinementOperation::AddObligation { obligation } => {
            mechanics.obligations.push(obligation.clone())
        }
        RefinementOperation::AddObstruction { obstruction } => {
            mechanics.obstructions.push(obstruction.clone())
        }
        RefinementOperation::BindObstruction { .. } => {}
        RefinementOperation::AddTechnique { technique } => {
            mechanics.techniques.push(technique.clone())
        }
        RefinementOperation::AddResolver { resolver } => mechanics.resolvers.push(resolver.clone()),
        RefinementOperation::AddWriter { writer } => mechanics.writers.push(writer.clone()),
        RefinementOperation::AddGate { gate } => mechanics.gates.push(gate.clone()),
        RefinementOperation::AddReader { reader } => mechanics.readers.push(reader.clone()),
        RefinementOperation::AddReconstructionRule {
            reconstruction_rule,
        } => mechanics
            .reconstruction_rules
            .push(reconstruction_rule.clone()),
        RefinementOperation::AddMicrotrace { microtrace } => {
            mechanics.microtraces.push(microtrace.clone())
        }
        RefinementOperation::AddGoal { goal } => mechanics.goals.push(goal.clone()),
        RefinementOperation::AddAlias { alias } => facts.aliases.push(alias.clone()),
        RefinementOperation::AddDerivedFact { fact } => facts.derived_facts.push(fact.clone()),
        RefinementOperation::ComponentTransform {
            prerequisite,
            operations,
        } => mechanics.techniques.push(Technique {
            id: rule.id.clone(),
            label: rule.label.clone(),
            scope: pack.manifest.scope.clone(),
            prerequisites: prerequisite.clone(),
            operations: operations.clone(),
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::new(),
            },
            evidence: rule.evidence.clone(),
        }),
        RefinementOperation::SuppressWriter { writer_id, when } => {
            mechanics.gates.push(GateRule {
                id: rule.id.clone(),
                scope: pack.manifest.scope.clone(),
                active_when: when.clone(),
                blocked_writer_ids: vec![writer_id.clone()],
                lifetime: SemanticLifetime::Unknown,
                evidence: rule.evidence.clone(),
            });
        }
        RefinementOperation::AssumeObstructionAbsent {
            obstruction_id,
            when,
        } => mechanics.resolvers.push(ObstructionResolver {
            id: rule.id.clone(),
            label: rule.label.clone(),
            scope: pack.manifest.scope.clone(),
            obstruction_id: obstruction_id.clone(),
            resolution_kind: ResolutionKind::AssumeAbsent,
            applicable_when: when.clone(),
            operations: Vec::new(),
            evidence: rule.evidence.clone(),
        }),
        RefinementOperation::ReplaceRecord { .. } => {}
    }
    Ok(())
}

fn validate_authored_obstruction(
    obstruction: &AuthoredObstruction,
) -> Result<(), PlannerContractError> {
    validate_stable_id("rules.obstruction.id", &obstruction.id)?;
    validate_label("rules.obstruction.label", &obstruction.label)?;
    obstruction.scope.validate("rules.obstruction.scope")?;
    obstruction.active_when.validate()?;
    validate_ids(
        "rules.obstruction.obligation_ids",
        &obstruction.obligation_ids,
        false,
    )?;
    obstruction
        .evidence
        .validate("rules.obstruction.evidence")?;
    validate_obstruction_action_selector(&obstruction.action_selector)
}

fn validate_obstruction_action_selector(
    selector: &ObstructionActionSelector,
) -> Result<(), PlannerContractError> {
    match selector {
        ObstructionActionSelector::ActionId { action_id } => {
            validate_stable_id("rules.obstruction.action_selector.action_id", action_id)
        }
        ObstructionActionSelector::Transition {
            transition_kind,
            approach_id,
            source,
            destination,
        } => {
            if transition_kind.is_none()
                && approach_id.is_none()
                && source.is_none()
                && destination.is_none()
            {
                return Err(PlannerContractError::new(
                    "rules.obstruction.action_selector",
                    "must contain at least one structural transition criterion",
                ));
            }
            if let Some(approach_id) = approach_id {
                validate_stable_id("rules.obstruction.action_selector.approach_id", approach_id)?;
            }
            if let Some(source) = source {
                validate_location_selector("rules.obstruction.action_selector.source", source)?;
            }
            if let Some(destination) = destination {
                validate_location_selector(
                    "rules.obstruction.action_selector.destination",
                    destination,
                )?;
            }
            Ok(())
        }
    }
}

fn validate_location_selector(
    field: &str,
    selector: &SceneLocationSelector,
) -> Result<(), PlannerContractError> {
    if selector.stage.is_none()
        && selector.room.is_none()
        && selector.layer.is_none()
        && selector.spawn.is_none()
    {
        return Err(PlannerContractError::new(
            field,
            "must constrain at least one location field",
        ));
    }
    if let Some(stage) = &selector.stage {
        validate_label(field, stage)?;
    }
    Ok(())
}

fn compile_obstruction_bindings(
    stack: &RefinementStack,
    packs: &BTreeMap<&str, &RefinementPack>,
    mechanics: &mut MechanicsCatalog,
) -> Result<Vec<CompiledObstructionBinding>, PlannerContractError> {
    let mut compiled_by_template = BTreeMap::<String, Vec<(String, String)>>::new();
    let mut binding_records = Vec::new();
    for entry in &stack.entries {
        let pack = packs[entry.pack_id.as_str()];
        for rule in &pack.rules {
            let RefinementOperation::BindObstruction { obstruction } = &rule.operation else {
                continue;
            };
            if compiled_by_template.contains_key(&obstruction.id) {
                return Err(PlannerContractError::new(
                    "rules.obstruction.id",
                    format!("duplicate authored obstruction template {}", obstruction.id),
                ));
            }
            let matches = mechanics
                .transitions
                .iter()
                .filter(|transition| transition_matches(&obstruction.action_selector, transition))
                .collect::<Vec<_>>();
            if matches.is_empty() {
                return Err(PlannerContractError::new(
                    "rules.obstruction.action_selector",
                    format!("matched no candidate actions for {}", obstruction.id),
                ));
            }
            if obstruction.match_cardinality == MatchCardinality::ExactlyOne && matches.len() != 1 {
                return Err(PlannerContractError::new(
                    "rules.obstruction.action_selector",
                    format!(
                        "expected exactly one candidate action for {}, matched {}",
                        obstruction.id,
                        matches.len()
                    ),
                ));
            }

            let plural = matches.len() > 1;
            let mut compiled = Vec::with_capacity(matches.len());
            for transition in matches {
                let id = if plural {
                    generated_binding_id("obstruction", &obstruction.id, &transition.id)
                } else {
                    obstruction.id.clone()
                };
                mechanics.obstructions.push(Obstruction {
                    id: id.clone(),
                    label: obstruction.label.clone(),
                    scope: obstruction.scope.clone(),
                    blocked_action_id: transition.id.clone(),
                    approach_id: transition.approach_id.clone(),
                    active_when: obstruction.active_when.clone(),
                    obligation_ids: obstruction.obligation_ids.clone(),
                    evidence: obstruction.evidence.clone(),
                });
                binding_records.push(CompiledObstructionBinding {
                    authored_obstruction_id: obstruction.id.clone(),
                    compiled_obstruction_id: id.clone(),
                    action_id: transition.id.clone(),
                    action_selector: obstruction.action_selector.clone(),
                    match_cardinality: obstruction.match_cardinality,
                    source_pack_id: pack.manifest.id.clone(),
                    source_rule_id: rule.id.clone(),
                });
                compiled.push((id, transition.id.clone()));
            }
            compiled_by_template.insert(obstruction.id.clone(), compiled);
        }
    }

    if compiled_by_template.is_empty() {
        return Ok(Vec::new());
    }
    let resolvers = std::mem::take(&mut mechanics.resolvers);
    for resolver in resolvers {
        let Some(bindings) = compiled_by_template.get(&resolver.obstruction_id) else {
            mechanics.resolvers.push(resolver);
            continue;
        };
        for (index, (obstruction_id, action_id)) in bindings.iter().enumerate() {
            let mut bound = resolver.clone();
            if bindings.len() > 1 {
                bound.id = generated_binding_id("resolver", &resolver.id, action_id);
            } else if index != 0 {
                unreachable!("a singular binding contains only one resolver target");
            }
            bound.obstruction_id = obstruction_id.clone();
            mechanics.resolvers.push(bound);
        }
    }
    binding_records.sort();
    Ok(binding_records)
}

fn validate_compiled_obstruction_bindings(
    catalog: &ComposedPlannerCatalog,
) -> Result<(), PlannerContractError> {
    let transition_by_id = catalog
        .mechanics
        .transitions
        .iter()
        .map(|transition| (transition.id.as_str(), transition))
        .collect::<BTreeMap<_, _>>();
    let obstruction_by_id = catalog
        .mechanics
        .obstructions
        .iter()
        .map(|obstruction| (obstruction.id.as_str(), obstruction))
        .collect::<BTreeMap<_, _>>();
    let pack_ids = catalog
        .refinement_stack
        .entries
        .iter()
        .map(|entry| entry.pack_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut previous = None;
    let mut pairs = BTreeSet::new();
    let mut groups = BTreeMap::<&str, Vec<&CompiledObstructionBinding>>::new();
    for binding in &catalog.obstruction_bindings {
        validate_obstruction_action_selector(&binding.action_selector)?;
        for (field, id) in [
            (
                "obstruction_bindings.authored_obstruction_id",
                &binding.authored_obstruction_id,
            ),
            (
                "obstruction_bindings.compiled_obstruction_id",
                &binding.compiled_obstruction_id,
            ),
            ("obstruction_bindings.action_id", &binding.action_id),
            (
                "obstruction_bindings.source_pack_id",
                &binding.source_pack_id,
            ),
            (
                "obstruction_bindings.source_rule_id",
                &binding.source_rule_id,
            ),
        ] {
            validate_stable_id(field, id)?;
        }
        if previous.is_some_and(|prior: &CompiledObstructionBinding| prior >= binding) {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "must be unique and sorted",
            ));
        }
        if !pairs.insert((
            binding.authored_obstruction_id.as_str(),
            binding.action_id.as_str(),
        )) {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "contains a duplicate authored-obstruction/action pair",
            ));
        }
        let transition = transition_by_id
            .get(binding.action_id.as_str())
            .ok_or_else(|| {
                PlannerContractError::new(
                    "obstruction_bindings.action_id",
                    "references an unknown transition",
                )
            })?;
        let obstruction = obstruction_by_id
            .get(binding.compiled_obstruction_id.as_str())
            .ok_or_else(|| {
                PlannerContractError::new(
                    "obstruction_bindings.compiled_obstruction_id",
                    "references an unknown obstruction",
                )
            })?;
        if obstruction.blocked_action_id != binding.action_id
            || obstruction.approach_id != transition.approach_id
            || !transition_matches(&binding.action_selector, transition)
        {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "does not agree with its compiled obstruction and transition",
            ));
        }
        if !pack_ids.contains(binding.source_pack_id.as_str()) {
            return Err(PlannerContractError::new(
                "obstruction_bindings.source_pack_id",
                "references a pack absent from the refinement stack",
            ));
        }
        groups
            .entry(binding.authored_obstruction_id.as_str())
            .or_default()
            .push(binding);
        previous = Some(binding);
    }
    for bindings in groups.values() {
        let first = bindings[0];
        if bindings.iter().any(|binding| {
            binding.action_selector != first.action_selector
                || binding.match_cardinality != first.match_cardinality
                || binding.source_pack_id != first.source_pack_id
                || binding.source_rule_id != first.source_rule_id
        }) {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "one authored obstruction has inconsistent selector provenance",
            ));
        }
        if first.match_cardinality == MatchCardinality::ExactlyOne && bindings.len() != 1 {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "an exactly-one selector must have exactly one compiled binding",
            ));
        }
    }
    Ok(())
}

fn transition_matches(
    selector: &ObstructionActionSelector,
    transition: &CandidateTransition,
) -> bool {
    match selector {
        ObstructionActionSelector::ActionId { action_id } => transition.id == *action_id,
        ObstructionActionSelector::Transition {
            transition_kind,
            approach_id,
            source,
            destination,
        } => {
            transition_kind.is_none_or(|kind| transition.transition_kind == kind)
                && approach_id
                    .as_ref()
                    .is_none_or(|approach| transition.approach_id == *approach)
                && source.as_ref().is_none_or(|source| {
                    source_matches_guard(source, &transition.activation.hard_guards)
                })
                && destination.as_ref().is_none_or(|destination| {
                    transition
                        .activation
                        .effects
                        .iter()
                        .rev()
                        .find_map(|operation| match operation {
                            StateOperation::SetLocation { location } => Some(location),
                            _ => None,
                        })
                        .is_some_and(|location| location_matches(destination, location))
                })
        }
    }
}

fn source_matches_guard(selector: &SceneLocationSelector, guard: &PredicateExpression) -> bool {
    selector.stage.as_ref().is_none_or(|stage| {
        guard_contains_location_equality(
            guard,
            &ValueReference::LocationStage,
            &StateValue::Text(stage.clone()),
        )
    }) && selector.room.is_none_or(|room| {
        guard_contains_location_equality(
            guard,
            &ValueReference::LocationRoom,
            &StateValue::Signed(room.into()),
        )
    }) && selector.layer.is_none_or(|layer| {
        guard_contains_location_equality(
            guard,
            &ValueReference::LocationLayer,
            &StateValue::Signed(layer.into()),
        )
    }) && selector.spawn.is_none_or(|spawn| {
        guard_contains_location_equality(
            guard,
            &ValueReference::LocationSpawn,
            &StateValue::Signed(spawn.into()),
        )
    })
}

fn guard_contains_location_equality(
    expression: &PredicateExpression,
    reference: &ValueReference,
    value: &StateValue,
) -> bool {
    match expression {
        PredicateExpression::Compare {
            left,
            operator: ComparisonOperator::Equal,
            right,
        } => {
            (left == reference
                && right
                    == &ValueReference::Literal {
                        value: value.clone(),
                    })
                || (right == reference
                    && left
                        == &ValueReference::Literal {
                            value: value.clone(),
                        })
        }
        PredicateExpression::All { terms } => terms
            .iter()
            .any(|term| guard_contains_location_equality(term, reference, value)),
        PredicateExpression::True
        | PredicateExpression::False
        | PredicateExpression::Fact { .. }
        | PredicateExpression::Any { .. }
        | PredicateExpression::Not { .. }
        | PredicateExpression::Compare { .. } => false,
    }
}

fn location_matches(selector: &SceneLocationSelector, location: &SceneLocation) -> bool {
    selector
        .stage
        .as_ref()
        .is_none_or(|stage| location.stage == *stage)
        && selector.room.is_none_or(|room| location.room == room)
        && selector.layer.is_none_or(|layer| location.layer == layer)
        && selector.spawn.is_none_or(|spawn| location.spawn == spawn)
}

fn generated_binding_id(kind: &str, template_id: &str, action_id: &str) -> String {
    let digest =
        Digest(Sha256::digest(format!("{kind}\0{template_id}\0{action_id}").as_bytes()).into());
    format!("binding.{kind}.{digest}")
}

fn remove_record(facts: &mut FactCatalog, mechanics: &mut MechanicsCatalog, id: &str) -> usize {
    let mut removed = 0;
    removed += remove_where(&mut facts.aliases, id, |record| &record.id);
    removed += remove_where(&mut facts.derived_facts, id, |record| &record.id);
    removed += remove_where(&mut mechanics.transitions, id, |record| &record.id);
    removed += remove_where(&mut mechanics.obligations, id, |record| &record.id);
    removed += remove_where(&mut mechanics.writers, id, |record| &record.id);
    removed += remove_where(&mut mechanics.gates, id, |record| &record.id);
    removed += remove_where(&mut mechanics.readers, id, |record| &record.id);
    removed += remove_where(&mut mechanics.reconstruction_rules, id, |record| &record.id);
    removed += remove_where(&mut mechanics.obstructions, id, |record| &record.id);
    removed += remove_where(&mut mechanics.resolvers, id, |record| &record.id);
    removed += remove_where(&mut mechanics.techniques, id, |record| &record.id);
    removed += remove_where(&mut mechanics.microtraces, id, |record| &record.id);
    removed += remove_where(&mut mechanics.goals, id, |record| &record.id);
    removed
}

fn remove_where<T, F>(records: &mut Vec<T>, id: &str, get_id: F) -> usize
where
    F: Fn(&T) -> &String,
{
    let before = records.len();
    records.retain(|record| get_id(record) != id);
    before - records.len()
}

fn sort_catalogs(facts: &mut FactCatalog, mechanics: &mut MechanicsCatalog) {
    facts.aliases.sort_by(|left, right| left.id.cmp(&right.id));
    facts
        .derived_facts
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .transitions
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .obligations
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .writers
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .gates
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .readers
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .reconstruction_rules
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .obstructions
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .resolvers
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .techniques
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .microtraces
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .goals
        .sort_by(|left, right| left.id.cmp(&right.id));
}

fn validate_operations(operations: &[StateOperation]) -> Result<(), PlannerContractError> {
    if operations.len() > 4_096 {
        return Err(PlannerContractError::new(
            "rules.operations",
            "must contain at most 4096 operations",
        ));
    }
    for operation in operations {
        operation.validate()?;
    }
    Ok(())
}

fn validate_dependencies(dependencies: &[PackDependency]) -> Result<(), PlannerContractError> {
    if dependencies.len() > 256 {
        return Err(PlannerContractError::new(
            "manifest.dependencies",
            "must contain at most 256 records",
        ));
    }
    let mut previous = None;
    for dependency in dependencies {
        validate_stable_id("manifest.dependencies.pack_id", &dependency.pack_id)?;
        if dependency.pack_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "manifest.dependencies.pack_sha256",
                "must be nonzero",
            ));
        }
        if previous.is_some_and(|prior: &str| prior >= dependency.pack_id.as_str()) {
            return Err(PlannerContractError::new(
                "manifest.dependencies",
                "must be unique and sorted by pack ID",
            ));
        }
        previous = Some(dependency.pack_id.as_str());
    }
    Ok(())
}

fn validate_ids(
    field: &str,
    ids: &[String],
    allow_empty: bool,
) -> Result<(), PlannerContractError> {
    if (!allow_empty && ids.is_empty()) || ids.len() > 256 {
        return Err(PlannerContractError::new(
            field,
            "contains an invalid number of IDs",
        ));
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

fn validate_version(version: &str) -> Result<(), PlannerContractError> {
    let parts = version.split('.').collect::<Vec<_>>();
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

fn reject_dependency_cycles(
    packs: &BTreeMap<&str, &RefinementPack>,
) -> Result<(), PlannerContractError> {
    fn visit<'a>(
        id: &'a str,
        packs: &BTreeMap<&'a str, &'a RefinementPack>,
        visiting: &mut BTreeSet<&'a str>,
        complete: &mut BTreeSet<&'a str>,
    ) -> Result<(), PlannerContractError> {
        if complete.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(PlannerContractError::new(
                "manifest.dependencies",
                format!("dependency cycle at {id}"),
            ));
        }
        for dependency in &packs[id].manifest.dependencies {
            if let Some((canonical, _)) = packs.get_key_value(dependency.pack_id.as_str()) {
                visit(canonical, packs, visiting, complete)?;
            }
        }
        visiting.remove(id);
        complete.insert(id);
        Ok(())
    }

    let mut complete = BTreeSet::new();
    for id in packs.keys().copied() {
        visit(id, packs, &mut BTreeSet::new(), &mut complete)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{PlannerGraph, PlannerGraphRelation};
    use crate::identity::{ContextSelector, ExactContext};
    use crate::logic::{EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, TruthStatus};
    use crate::transition::{MECHANICS_CATALOG_SCHEMA, ObligationDetail, ObligationKind};

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

    fn evidence(truth: TruthStatus) -> RuleEvidence {
        RuleEvidence {
            truth,
            records: vec![EvidenceRecord {
                id: "source.refinement".into(),
                kind: EvidenceKind::Theorycraft,
                source_sha256: Some(Digest([3; 32])),
                note: "Explicit theorycraft assumption.".into(),
            }],
        }
    }

    fn pack(id: &str, precedence: i32, operation: RefinementOperation) -> RefinementPack {
        RefinementPack {
            schema: REFINEMENT_PACK_SCHEMA.into(),
            manifest: RefinementPackManifest {
                id: id.into(),
                version: "1.0.0".into(),
                author: "Route research".into(),
                source: "Local theorycraft".into(),
                scope: scope(),
                precedence,
                dependencies: Vec::new(),
                conflicts: Vec::new(),
            },
            rules: vec![RefinementRule {
                id: format!("{id}.rule"),
                label: "Test rule".into(),
                operation,
                evidence: evidence(TruthStatus::Hypothetical),
            }],
        }
    }

    fn empty_catalogs() -> (FactCatalog, MechanicsCatalog) {
        (
            FactCatalog {
                schema: FACT_CATALOG_SCHEMA.into(),
                aliases: Vec::new(),
                derived_facts: Vec::new(),
            },
            MechanicsCatalog {
                schema: MECHANICS_CATALOG_SCHEMA.into(),
                transitions: Vec::new(),
                obligations: Vec::new(),
                writers: Vec::new(),
                gates: Vec::new(),
                readers: Vec::new(),
                reconstruction_rules: Vec::new(),
                obstructions: Vec::new(),
                resolvers: Vec::new(),
                techniques: Vec::new(),
                microtraces: Vec::new(),
                goals: Vec::new(),
            },
        )
    }

    fn rule(id: &str, operation: RefinementOperation) -> RefinementRule {
        RefinementRule {
            id: id.into(),
            label: format!("Rule {id}"),
            operation,
            evidence: evidence(TruthStatus::Hypothetical),
        }
    }

    fn map_transition(
        id: &str,
        source_stage: &str,
        destination_stage: &str,
    ) -> CandidateTransition {
        CandidateTransition {
            id: id.into(),
            label: format!("{source_stage} to {destination_stage}"),
            scope: scope(),
            transition_kind: TransitionKind::EncodedMapExit,
            approach_id: format!("approach.{id}"),
            activation: crate::transition::ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        PredicateExpression::Compare {
                            left: ValueReference::LocationStage,
                            operator: ComparisonOperator::Equal,
                            right: ValueReference::Literal {
                                value: StateValue::Text(source_stage.into()),
                            },
                        },
                        PredicateExpression::Compare {
                            left: ValueReference::LocationRoom,
                            operator: ComparisonOperator::Equal,
                            right: ValueReference::Literal {
                                value: StateValue::Signed(0),
                            },
                        },
                    ],
                },
                physical_obligation_ids: vec!["obligation.reach-exit".into()],
                effects: vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: destination_stage.into(),
                        room: 1,
                        layer: 0,
                        spawn: 2,
                    },
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(TruthStatus::Established),
        }
    }

    fn exit_obligation() -> FeasibilityObligation {
        FeasibilityObligation {
            id: "obligation.reach-exit".into(),
            label: "Reach the exit".into(),
            scope: scope(),
            obligation_kind: ObligationKind::Geometry,
            detail: ObligationDetail::Unresolved {
                research_question: "Can the exit be reached?".into(),
            },
            evidence: evidence(TruthStatus::Established),
        }
    }

    fn bound_obstruction(cardinality: MatchCardinality) -> AuthoredObstruction {
        AuthoredObstruction {
            id: "obstruction.bound-wall".into(),
            label: "Bound wall".into(),
            scope: scope(),
            action_selector: ObstructionActionSelector::Transition {
                transition_kind: Some(TransitionKind::EncodedMapExit),
                approach_id: None,
                source: None,
                destination: Some(SceneLocationSelector {
                    stage: Some("DEST".into()),
                    room: Some(1),
                    layer: None,
                    spawn: None,
                }),
            },
            match_cardinality: cardinality,
            active_when: PredicateExpression::True,
            obligation_ids: vec!["obligation.reach-exit".into()],
            evidence: evidence(TruthStatus::Established),
        }
    }

    #[test]
    fn theorycraft_absence_is_explicit_and_remains_hypothetical() {
        let pack = pack(
            "what-if.no-wall",
            50,
            RefinementOperation::AssumeObstructionAbsent {
                obstruction_id: "obstruction.ordon-wall".into(),
                when: PredicateExpression::True,
            },
        );
        pack.validate().unwrap();
        assert_eq!(pack.rules[0].evidence.truth, TruthStatus::Hypothetical);
        assert_ne!(pack.digest().unwrap(), Digest::ZERO);
    }

    #[test]
    fn stack_precedence_is_deterministic_independent_of_input_order() {
        let low = pack(
            "community.base",
            10,
            RefinementOperation::SuppressWriter {
                writer_id: "writer.savmem".into(),
                when: PredicateExpression::False,
            },
        );
        let high = pack(
            "route.local",
            100,
            RefinementOperation::AssumeObstructionAbsent {
                obstruction_id: "obstruction.wall".into(),
                when: PredicateExpression::True,
            },
        );
        let first = RefinementStack::build(&[high.clone(), low.clone()]).unwrap();
        let second = RefinementStack::build(&[low, high]).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.entries[0].pack_id, "community.base");
    }

    #[test]
    fn conflicts_and_dependency_digest_mismatches_fail_closed() {
        let mut left = pack(
            "left",
            1,
            RefinementOperation::SuppressWriter {
                writer_id: "writer.a".into(),
                when: PredicateExpression::True,
            },
        );
        let right = pack(
            "right",
            2,
            RefinementOperation::SuppressWriter {
                writer_id: "writer.b".into(),
                when: PredicateExpression::True,
            },
        );
        left.manifest.conflicts = vec!["right".into()];
        assert_eq!(
            RefinementStack::build(&[left.clone(), right.clone()])
                .unwrap_err()
                .field(),
            "manifest.conflicts"
        );

        left.manifest.conflicts.clear();
        left.manifest.dependencies = vec![PackDependency {
            pack_id: "right".into(),
            pack_sha256: Digest([9; 32]),
        }];
        assert_eq!(
            RefinementStack::build(&[left, right]).unwrap_err().field(),
            "manifest.dependencies"
        );
    }

    #[test]
    fn canonical_decode_rejects_browser_or_editor_junk_fields() {
        let pack = pack(
            "clean",
            1,
            RefinementOperation::AssumeObstructionAbsent {
                obstruction_id: "obstruction.wall".into(),
                when: PredicateExpression::True,
            },
        );
        let bytes = pack.canonical_bytes().unwrap();
        assert_eq!(RefinementPack::decode_canonical(&bytes).unwrap(), pack);
        let mut value = serde_json::to_value(pack).unwrap();
        value["browser_only"] = serde_json::json!(true);
        assert!(serde_json::from_value::<RefinementPack>(value).is_err());
    }

    #[test]
    fn composed_catalog_accepts_only_additive_ephemeral_editor_overlays() {
        let (facts, mechanics) = empty_catalogs();
        let base = ComposedPlannerCatalog::compose(&facts, &mechanics, &[]).unwrap();
        let first = pack(
            "what-if.rebind",
            1_000,
            RefinementOperation::ComponentTransform {
                prerequisite: PredicateExpression::True,
                operations: vec![StateOperation::Preserve {
                    selector: crate::state::ComponentSelector::Id {
                        component_id: "component.stage-bank".into(),
                    },
                }],
            },
        );
        let extended = base
            .extend_ephemeral_what_if(std::slice::from_ref(&first))
            .unwrap();
        assert!(base.mechanics.techniques.is_empty());
        assert_eq!(extended.mechanics.techniques[0].id, "what-if.rebind.rule");
        assert_eq!(
            extended.refinement_stack.entries[0].layer,
            RefinementLayer::EphemeralWhatIf
        );

        let mut second = pack(
            "what-if.copy",
            1_001,
            RefinementOperation::ComponentTransform {
                prerequisite: PredicateExpression::True,
                operations: vec![StateOperation::Preserve {
                    selector: crate::state::ComponentSelector::Id {
                        component_id: "component.session".into(),
                    },
                }],
            },
        );
        second.manifest.dependencies.push(PackDependency {
            pack_id: first.manifest.id.clone(),
            pack_sha256: first.digest().unwrap(),
        });
        assert_eq!(
            extended
                .extend_ephemeral_what_if(std::slice::from_ref(&second))
                .unwrap()
                .mechanics
                .techniques
                .len(),
            2
        );

        let forbidden = pack(
            "what-if.writer",
            1_002,
            RefinementOperation::SuppressWriter {
                writer_id: "writer.any".into(),
                when: PredicateExpression::True,
            },
        );
        assert_eq!(
            extended
                .extend_ephemeral_what_if(&[forbidden])
                .unwrap_err()
                .field(),
            "rules.operation"
        );
    }

    #[test]
    fn composition_compiles_obstructions_transforms_and_writer_suppression() {
        let (facts, mut mechanics) = empty_catalogs();
        mechanics.writers.push(WriterRule {
            id: "writer.savmem".into(),
            scope: scope(),
            activation: PredicateExpression::True,
            operation: StateOperation::SetGate {
                gate_id: "gate.return-place".into(),
            },
            evidence: evidence(TruthStatus::Established),
        });
        let pack = RefinementPack {
            schema: REFINEMENT_PACK_SCHEMA.into(),
            manifest: RefinementPackManifest {
                id: "research.ordon-wall".into(),
                version: "1.0.0".into(),
                author: "Route research".into(),
                source: "Local theorycraft".into(),
                scope: scope(),
                precedence: 10,
                dependencies: Vec::new(),
                conflicts: Vec::new(),
            },
            rules: vec![
                rule(
                    "a.obligation",
                    RefinementOperation::AddObligation {
                        obligation: FeasibilityObligation {
                            id: "obligation.reach-wall".into(),
                            label: "Reach the far side of the wall".into(),
                            scope: scope(),
                            obligation_kind: ObligationKind::Geometry,
                            detail: ObligationDetail::Unresolved {
                                research_question: "Can the wall be crossed?".into(),
                            },
                            evidence: evidence(TruthStatus::Established),
                        },
                    },
                ),
                rule(
                    "b.obstruction",
                    RefinementOperation::AddObstruction {
                        obstruction: Obstruction {
                            id: "obstruction.ordon-wall".into(),
                            label: "Ordon wall".into(),
                            scope: scope(),
                            blocked_action_id: "transition.ordon-return".into(),
                            approach_id: "approach.ordon-wall".into(),
                            active_when: PredicateExpression::True,
                            obligation_ids: vec!["obligation.reach-wall".into()],
                            evidence: evidence(TruthStatus::Established),
                        },
                    },
                ),
                rule(
                    "c.assume-absent",
                    RefinementOperation::AssumeObstructionAbsent {
                        obstruction_id: "obstruction.ordon-wall".into(),
                        when: PredicateExpression::True,
                    },
                ),
                rule(
                    "d.component-transform",
                    RefinementOperation::ComponentTransform {
                        prerequisite: PredicateExpression::True,
                        operations: vec![StateOperation::SetGate {
                            gate_id: "gate.what-if-transfer".into(),
                        }],
                    },
                ),
                rule(
                    "e.suppress-writer",
                    RefinementOperation::SuppressWriter {
                        writer_id: "writer.savmem".into(),
                        when: PredicateExpression::True,
                    },
                ),
            ],
        };

        let composed = ComposedPlannerCatalog::compose(&facts, &mechanics, &[pack]).unwrap();
        assert_eq!(composed.mechanics.obligations.len(), 1);
        assert_eq!(composed.mechanics.obstructions.len(), 1);
        assert_eq!(composed.mechanics.resolvers.len(), 1);
        assert_eq!(
            composed.mechanics.resolvers[0].resolution_kind,
            ResolutionKind::AssumeAbsent
        );
        assert_eq!(composed.mechanics.techniques.len(), 1);
        assert_eq!(
            composed.mechanics.gates[0].blocked_writer_ids,
            ["writer.savmem"]
        );
        let bytes = composed.canonical_bytes().unwrap();
        assert_eq!(
            ComposedPlannerCatalog::decode_canonical(&bytes).unwrap(),
            composed
        );
    }

    #[test]
    fn authored_obstruction_selector_binds_and_projects_the_block_dependency() {
        let (facts, mut mechanics) = empty_catalogs();
        mechanics.obligations.push(exit_obligation());
        mechanics
            .transitions
            .push(map_transition("transition.a", "SOURCE_A", "DEST"));
        let mut obstruction = bound_obstruction(MatchCardinality::ExactlyOne);
        obstruction.action_selector = ObstructionActionSelector::Transition {
            transition_kind: Some(TransitionKind::EncodedMapExit),
            approach_id: None,
            source: Some(SceneLocationSelector {
                stage: Some("SOURCE_A".into()),
                room: Some(0),
                layer: None,
                spawn: None,
            }),
            destination: Some(SceneLocationSelector {
                stage: Some("DEST".into()),
                room: Some(1),
                layer: None,
                spawn: None,
            }),
        };
        let pack = RefinementPack {
            schema: REFINEMENT_PACK_SCHEMA.into(),
            manifest: RefinementPackManifest {
                id: "binding.wall".into(),
                version: "1.0.0".into(),
                author: "Route research".into(),
                source: "Local theorycraft".into(),
                scope: scope(),
                precedence: 1,
                dependencies: Vec::new(),
                conflicts: Vec::new(),
            },
            rules: vec![
                rule(
                    "a.bind",
                    RefinementOperation::BindObstruction { obstruction },
                ),
                rule(
                    "b.resolve",
                    RefinementOperation::AssumeObstructionAbsent {
                        obstruction_id: "obstruction.bound-wall".into(),
                        when: PredicateExpression::True,
                    },
                ),
            ],
        };

        let composed =
            ComposedPlannerCatalog::compose(&facts, &mechanics, std::slice::from_ref(&pack))
                .unwrap();
        assert_eq!(composed.mechanics.obstructions.len(), 1);
        assert_eq!(
            composed.mechanics.obstructions[0].blocked_action_id,
            "transition.a"
        );
        assert_eq!(
            composed.mechanics.obstructions[0].approach_id,
            "approach.transition.a"
        );
        assert_eq!(
            composed.mechanics.resolvers[0].obstruction_id,
            "obstruction.bound-wall"
        );
        assert_eq!(composed.obstruction_bindings.len(), 1);
        assert_eq!(
            composed.obstruction_bindings[0].source_pack_id,
            "binding.wall"
        );
        assert_eq!(composed.obstruction_bindings[0].source_rule_id, "a.bind");
        let graph = PlannerGraph::project_composed(&composed).unwrap();
        assert!(graph.edges.iter().any(|edge| {
            edge.source_node_id == "obstruction/obstruction.bound-wall"
                && edge.target_node_id == "transition/transition.a"
                && edge.relation == PlannerGraphRelation::Blocks
        }));

        mechanics.transitions.clear();
        let error = ComposedPlannerCatalog::compose(&facts, &mechanics, &[pack]).unwrap_err();
        assert_eq!(error.field(), "rules.obstruction.action_selector");
        assert!(error.detail().contains("matched no candidate actions"));
    }

    #[test]
    fn plural_obstruction_selector_expands_bindings_and_resolvers_deterministically() {
        let (facts, mut mechanics) = empty_catalogs();
        mechanics.obligations.push(exit_obligation());
        mechanics
            .transitions
            .push(map_transition("transition.a", "SOURCE_A", "DEST"));
        mechanics
            .transitions
            .push(map_transition("transition.b", "SOURCE_B", "DEST"));
        let plural_pack = RefinementPack {
            schema: REFINEMENT_PACK_SCHEMA.into(),
            manifest: RefinementPackManifest {
                id: "binding.plural-wall".into(),
                version: "1.0.0".into(),
                author: "Route research".into(),
                source: "Local theorycraft".into(),
                scope: scope(),
                precedence: 1,
                dependencies: Vec::new(),
                conflicts: Vec::new(),
            },
            rules: vec![
                rule(
                    "a.bind",
                    RefinementOperation::BindObstruction {
                        obstruction: bound_obstruction(MatchCardinality::OneOrMore),
                    },
                ),
                rule(
                    "b.resolve",
                    RefinementOperation::AssumeObstructionAbsent {
                        obstruction_id: "obstruction.bound-wall".into(),
                        when: PredicateExpression::True,
                    },
                ),
            ],
        };

        let first =
            ComposedPlannerCatalog::compose(&facts, &mechanics, std::slice::from_ref(&plural_pack))
                .unwrap();
        let second =
            ComposedPlannerCatalog::compose(&facts, &mechanics, std::slice::from_ref(&plural_pack))
                .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.mechanics.obstructions.len(), 2);
        assert_eq!(first.mechanics.resolvers.len(), 2);
        assert_eq!(first.obstruction_bindings.len(), 2);
        assert!(
            first
                .mechanics
                .obstructions
                .iter()
                .all(|record| record.id.starts_with("binding.obstruction."))
        );
        for obstruction in &first.mechanics.obstructions {
            assert!(first.mechanics.resolvers.iter().any(|resolver| {
                resolver.obstruction_id == obstruction.id
                    && resolver.id.starts_with("binding.resolver.")
            }));
        }

        let singular_pack = pack(
            "binding.ambiguous-wall",
            1,
            RefinementOperation::BindObstruction {
                obstruction: bound_obstruction(MatchCardinality::ExactlyOne),
            },
        );
        let error =
            ComposedPlannerCatalog::compose(&facts, &mechanics, &[singular_pack]).unwrap_err();
        assert!(error.detail().contains("expected exactly one"));
    }

    #[test]
    fn duplicate_additions_require_an_explicit_replacement() {
        let (facts, mut mechanics) = empty_catalogs();
        let writer = WriterRule {
            id: "writer.savmem".into(),
            scope: scope(),
            activation: PredicateExpression::True,
            operation: StateOperation::SetGate {
                gate_id: "gate.return-place".into(),
            },
            evidence: evidence(TruthStatus::Established),
        };
        mechanics.writers.push(writer.clone());
        let duplicate = pack(
            "duplicate.writer",
            10,
            RefinementOperation::AddWriter { writer },
        );
        assert_eq!(
            ComposedPlannerCatalog::compose(&facts, &mechanics, &[duplicate])
                .unwrap_err()
                .field(),
            "writers"
        );
    }

    #[test]
    fn replacement_and_disable_precedence_is_deterministic() {
        let (facts, mut mechanics) = empty_catalogs();
        mechanics.goals.push(Goal {
            id: "goal.original".into(),
            label: "Original goal".into(),
            predicate: PredicateExpression::True,
        });
        let replacement = RefinementPack {
            schema: REFINEMENT_PACK_SCHEMA.into(),
            manifest: RefinementPackManifest {
                id: "replace.goal".into(),
                version: "1.0.0".into(),
                author: "Route research".into(),
                source: "Local theorycraft".into(),
                scope: scope(),
                precedence: 10,
                dependencies: Vec::new(),
                conflicts: Vec::new(),
            },
            rules: vec![
                rule(
                    "a.replace",
                    RefinementOperation::ReplaceRecord {
                        target_id: "goal.original".into(),
                        replacement_kind: ReplacementKind::Replace,
                        replacement_rule_id: Some("b.goal".into()),
                    },
                ),
                rule(
                    "b.goal",
                    RefinementOperation::AddGoal {
                        goal: Goal {
                            id: "goal.replacement".into(),
                            label: "Replacement goal".into(),
                            predicate: PredicateExpression::True,
                        },
                    },
                ),
            ],
        };
        let disable = pack(
            "disable.goal",
            20,
            RefinementOperation::ReplaceRecord {
                target_id: "goal.replacement".into(),
                replacement_kind: ReplacementKind::Disable,
                replacement_rule_id: None,
            },
        );

        let first = ComposedPlannerCatalog::compose(
            &facts,
            &mechanics,
            &[disable.clone(), replacement.clone()],
        )
        .unwrap();
        let second =
            ComposedPlannerCatalog::compose(&facts, &mechanics, &[replacement, disable]).unwrap();
        assert_eq!(first, second);
        assert!(first.mechanics.goals.is_empty());
    }

    #[test]
    fn route_local_and_ephemeral_layers_override_precedence_and_remove_cleanly() {
        let (facts, mut mechanics) = empty_catalogs();
        mechanics.goals.push(Goal {
            id: "goal.base".into(),
            label: "Base goal".into(),
            predicate: PredicateExpression::True,
        });
        let replacement_pack =
            |pack_id: &str, precedence: i32, from: &str, to: &str| RefinementPack {
                schema: REFINEMENT_PACK_SCHEMA.into(),
                manifest: RefinementPackManifest {
                    id: pack_id.into(),
                    version: "1.0.0".into(),
                    author: "Route research".into(),
                    source: "Layering regression fixture".into(),
                    scope: scope(),
                    precedence,
                    dependencies: Vec::new(),
                    conflicts: Vec::new(),
                },
                rules: vec![
                    rule(
                        &format!("{pack_id}.a-replace"),
                        RefinementOperation::ReplaceRecord {
                            target_id: from.into(),
                            replacement_kind: ReplacementKind::Replace,
                            replacement_rule_id: Some(format!("{pack_id}.b-goal")),
                        },
                    ),
                    rule(
                        &format!("{pack_id}.b-goal"),
                        RefinementOperation::AddGoal {
                            goal: Goal {
                                id: to.into(),
                                label: format!("Goal from {pack_id}"),
                                predicate: PredicateExpression::True,
                            },
                        },
                    ),
                ],
            };
        let enabled = replacement_pack("enabled.goal", 10_000, "goal.base", "goal.enabled");
        let mut route = replacement_pack("route.goal", -10_000, "goal.enabled", "goal.route");
        route.manifest.dependencies = vec![PackDependency {
            pack_id: enabled.manifest.id.clone(),
            pack_sha256: enabled.digest().unwrap(),
        }];
        let ephemeral = replacement_pack("what-if.goal", -20_000, "goal.route", "goal.what-if");
        let layers = RefinementLayers {
            enabled_packs: vec![enabled.clone()],
            route_local_overlays: vec![route.clone()],
            ephemeral_what_if_overlays: vec![ephemeral.clone()],
        };
        let composed =
            ComposedPlannerCatalog::compose_layered(&facts, &mechanics, &layers).unwrap();
        assert_eq!(
            composed
                .refinement_stack
                .entries
                .iter()
                .map(|entry| (entry.layer, entry.pack_id.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (RefinementLayer::EnabledPack, "enabled.goal"),
                (RefinementLayer::RouteLocal, "route.goal"),
                (RefinementLayer::EphemeralWhatIf, "what-if.goal"),
            ]
        );
        assert_eq!(composed.mechanics.goals[0].id, "goal.what-if");

        let without_what_if = ComposedPlannerCatalog::compose_layered(
            &facts,
            &mechanics,
            &RefinementLayers {
                enabled_packs: vec![enabled.clone()],
                route_local_overlays: vec![route],
                ephemeral_what_if_overlays: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(without_what_if.mechanics.goals[0].id, "goal.route");
        let enabled_only = ComposedPlannerCatalog::compose(&facts, &mechanics, &[enabled]).unwrap();
        assert_eq!(enabled_only.mechanics.goals[0].id, "goal.enabled");

        let mut invalid_dependency =
            replacement_pack("enabled.depends-on-what-if", 0, "goal.base", "goal.invalid");
        invalid_dependency.manifest.dependencies = vec![PackDependency {
            pack_id: ephemeral.manifest.id.clone(),
            pack_sha256: ephemeral.digest().unwrap(),
        }];
        let error = RefinementStack::build_layered(&RefinementLayers {
            enabled_packs: vec![invalid_dependency],
            route_local_overlays: Vec::new(),
            ephemeral_what_if_overlays: vec![ephemeral],
        })
        .unwrap_err();
        assert_eq!(error.field(), "manifest.dependencies");
        assert!(error.detail().contains("later-layer"));
    }

    #[test]
    fn diagnostics_accumulate_rule_shape_dependency_and_conflict_fixes() {
        let operation = RefinementOperation::AddGoal {
            goal: Goal {
                id: "goal.diagnostic".into(),
                label: "Diagnostic goal".into(),
                predicate: PredicateExpression::True,
            },
        };
        let mut malformed = pack("diagnostic.malformed", 0, operation.clone());
        malformed.schema = "old-schema".into();
        let mut duplicate = malformed.rules[0].clone();
        duplicate.label.clear();
        malformed.rules.push(duplicate);
        malformed.manifest.dependencies.push(PackDependency {
            pack_id: "diagnostic.missing".into(),
            pack_sha256: Digest([7; 32]),
        });
        let mut conflicting = pack("diagnostic.conflicting", 1, operation);
        conflicting.manifest.conflicts = vec![malformed.manifest.id.clone()];

        let report = diagnose_refinement_packs(&[malformed, conflicting]);
        assert!(!report.valid);
        assert!(report.diagnostics.len() >= 5);
        assert!(report.diagnostics.iter().any(|row| row.field == "schema"));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|row| row.detail.contains("duplicate rule ID"))
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|row| row.detail.contains("missing valid pack"))
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|row| row.detail.contains("conflict"))
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|row| !row.suggestion.is_empty())
        );
    }
}
