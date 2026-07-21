//! Versioned refinement packs and deterministic theorycraft overlays.

use crate::artifact::Digest;
use crate::logic::{
    ContextScope, DerivedFact, FactCatalog, FriendlyAlias, PredicateExpression, RuleEvidence,
};
use crate::state::SemanticLifetime;
use crate::transition::{
    ActorReconstructionRule, CandidateTransition, FeasibilityObligation, GateRule, Goal,
    MechanicsCatalog, Obstruction, ObstructionResolver, ReaderRule, ResolutionKind, RouteCost,
    StateOperation, Technique, WitnessedMicrotrace, WriterRule,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const REFINEMENT_PACK_SCHEMA: &str = "dusklight.route-planner.refinement-pack/v2";
pub const REFINEMENT_STACK_SCHEMA: &str = "dusklight.route-planner.refinement-stack/v1";
pub const COMPOSED_CATALOG_SCHEMA: &str = "dusklight.route-planner.composed-catalog/v2";

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
    pub precedence: i32,
    pub pack_id: String,
    pub pack_sha256: Digest,
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

impl RefinementStack {
    pub fn build(packs: &[RefinementPack]) -> Result<Self, PlannerContractError> {
        let mut by_id = BTreeMap::new();
        let mut digests = BTreeMap::new();
        for pack in packs {
            pack.validate()?;
            if by_id.insert(pack.manifest.id.as_str(), pack).is_some() {
                return Err(PlannerContractError::new(
                    "packs",
                    "contains duplicate pack IDs",
                ));
            }
            digests.insert(pack.manifest.id.as_str(), pack.digest()?);
        }
        for pack in packs {
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
        let mut entries = packs
            .iter()
            .map(|pack| RefinementStackEntry {
                precedence: pack.manifest.precedence,
                pack_id: pack.manifest.id.clone(),
                pack_sha256: digests[pack.manifest.id.as_str()],
            })
            .collect::<Vec<_>>();
        entries.sort();
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
                    "must be unique and sorted by precedence, ID, and digest",
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

impl ComposedPlannerCatalog {
    pub fn compose(
        base_facts: &FactCatalog,
        base_mechanics: &MechanicsCatalog,
        packs: &[RefinementPack],
    ) -> Result<Self, PlannerContractError> {
        base_facts.validate()?;
        base_mechanics.validate()?;
        let refinement_stack = RefinementStack::build(packs)?;
        let by_id = packs
            .iter()
            .map(|pack| (pack.manifest.id.as_str(), pack))
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
        sort_catalogs(&mut facts, &mut mechanics);
        let composed = Self {
            schema: COMPOSED_CATALOG_SCHEMA.into(),
            base_fact_catalog_sha256: base_facts.digest()?,
            base_mechanics_catalog_sha256: base_mechanics.digest()?,
            facts,
            mechanics,
            refinement_stack,
        };
        composed.validate()?;
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
        self.refinement_stack.validate()
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
}
