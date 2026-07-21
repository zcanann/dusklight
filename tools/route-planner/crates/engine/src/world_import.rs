//! Conservative import of immutable world inventories into planner facts.
//!
//! Authored SCLS destinations remain addressable even when no activation is
//! known. A candidate transition is emitted only for an extracted collision /
//! SCLS join, and that candidate still carries explicit activation-semantics
//! and physical-reachability obligations.

use crate::artifact::Digest;
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::state::{
    ComponentBinding, SceneLocation, StateValue, StaticWorldObject, validate_static_object,
};
use crate::transition::{
    ActivationContract, CandidateTransition, FeasibilityObligation, MECHANICS_CATALOG_SCHEMA,
    MechanicsCatalog, ObligationDetail, ObligationKind, StateOperation, TransitionKind,
    UnknownRequirement,
};
use crate::world_data::{PlacementKind, PlacementRecord, SourceKind, WorldContext, WorldInventory};
use crate::{PlannerContractError, canonical_json, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const EXTRACTED_WORLD_FACTS_SCHEMA: &str = "dusklight.route-planner.extracted-world-facts/v7";
pub const MAX_EXTRACTED_WORLD_RECORDS: usize = 2_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldInventoryFactSource {
    pub stage: String,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedSpawn {
    pub id: String,
    pub source_object_id: String,
    pub source_record_id: String,
    pub location: SceneLocation,
    pub position: [f32; 3],
    pub rotation: [i16; 3],
    pub parameters: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedEncodedExit {
    pub id: String,
    pub source_record_id: String,
    pub source_stage: String,
    pub source_room: Option<i8>,
    pub destination: SceneLocation,
    pub wipe: u8,
    pub wipe_time: u8,
    pub time_hour: i8,
    pub raw: Vec<u8>,
    pub candidate_transition_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedWorldFacts {
    pub schema: String,
    pub exact_context: ExactContext,
    pub world_context_sha256: Digest,
    pub inventories: Vec<WorldInventoryFactSource>,
    pub static_world_objects: Vec<StaticWorldObject>,
    pub spawns: Vec<ExtractedSpawn>,
    pub encoded_exits: Vec<ExtractedEncodedExit>,
    pub mechanics: MechanicsCatalog,
}

impl ExtractedWorldFacts {
    pub fn build(
        content: &ContentIdentity,
        runtime_configuration: &RuntimeConfiguration,
        world_context: &WorldContext,
        inventories: &[WorldInventory],
    ) -> Result<Self, PlannerContractError> {
        content.validate()?;
        runtime_configuration.validate()?;
        world_context
            .validate()
            .map_err(|error| world_error("world_context", error))?;
        let content_sha256 = content.digest()?;
        if runtime_configuration.content_sha256 != content_sha256 {
            return Err(PlannerContractError::new(
                "runtime_configuration.content_sha256",
                "does not name the supplied content identity",
            ));
        }
        if world_context.game_data_sha256 != content.fingerprint.game_data_sha256 {
            return Err(PlannerContractError::new(
                "world_context.game_data_sha256",
                "does not match the supplied content identity",
            ));
        }
        if inventories.len() != world_context.stages.len() {
            return Err(PlannerContractError::new(
                "inventories",
                "does not cover every world-context stage exactly once",
            ));
        }

        let mut inventory_by_stage = BTreeMap::new();
        for inventory in inventories {
            inventory
                .validate()
                .map_err(|error| world_error("inventories", error))?;
            if inventory_by_stage
                .insert(inventory.stage.as_str(), inventory)
                .is_some()
            {
                return Err(PlannerContractError::new(
                    "inventories",
                    "contains a duplicate stage",
                ));
            }
        }

        let exact_context = ExactContext {
            content_sha256,
            runtime_configuration_sha256: runtime_configuration.digest()?,
        };
        let scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: exact_context.clone(),
            }],
        };
        let mut sources = Vec::with_capacity(world_context.stages.len());
        let mut static_world_objects = Vec::new();
        let mut spawns = Vec::new();
        let mut encoded_exits = Vec::new();
        let mut transitions = Vec::new();
        let mut obligations = Vec::new();

        for stage in &world_context.stages {
            let inventory = inventory_by_stage
                .get(stage.stage.as_str())
                .ok_or_else(|| {
                    PlannerContractError::new("inventories", "is missing a world-context stage")
                })?;
            let inventory_sha256 = inventory
                .digest()
                .map_err(|error| world_error("inventories", error))?;
            if inventory_sha256 != stage.inventory_sha256 {
                return Err(PlannerContractError::new(
                    "inventories",
                    "digest does not match the world context",
                ));
            }
            sources.push(WorldInventoryFactSource {
                stage: stage.stage.clone(),
                inventory_sha256,
                spatial_index_sha256: stage.spatial_index_sha256,
            });

            for placement in inventory.placements.iter().chain(&inventory.player_spawns) {
                let object = import_static_object(&inventory.stage, placement)?;
                if placement.kind == PlacementKind::PlayerSpawn {
                    spawns.push(import_spawn(&inventory.stage, placement, &object.id)?);
                }
                static_world_objects.push(object);
            }

            let mut transition_ids_by_exit = BTreeMap::<&str, Vec<String>>::new();
            for trigger in &inventory.load_triggers {
                let token = stable_token(
                    "world.load-trigger",
                    &[inventory.stage.as_bytes(), trigger.stable_id.as_bytes()],
                );
                let transition_id = format!("transition.{token}");
                let approach_id = format!("approach.{token}");
                let obligation_id = format!("obligation.reach.{token}");
                let evidence =
                    extracted_evidence(inventory_sha256, &token, trigger.inferred_semantics);
                obligations.push(FeasibilityObligation {
                    id: obligation_id.clone(),
                    label: format!(
                        "Reach collision exit {} in {} room {}",
                        trigger.collision_exit_id, inventory.stage, trigger.room
                    ),
                    scope: scope.clone(),
                    obligation_kind: ObligationKind::Geometry,
                    detail: ObligationDetail::Geometry {
                        approach_id: approach_id.clone(),
                        source_region_id: stable_token(
                            "region.collision",
                            &[trigger.collision_id.as_bytes()],
                        ),
                        destination_region_id: stable_token(
                            "region.encoded-exit",
                            &[trigger.scls_id.as_bytes()],
                        ),
                    },
                    evidence: RuleEvidence {
                        truth: TruthStatus::Unknown,
                        records: evidence.records.clone(),
                    },
                });
                let unknown_requirements = trigger
                    .inferred_semantics
                    .then(|| UnknownRequirement {
                        id: "activation-semantics".into(),
                        description: "The collision-code/SCLS activation semantics are inferred and require source or trace confirmation.".into(),
                        evidence: RuleEvidence {
                            truth: TruthStatus::Unknown,
                            records: evidence.records.clone(),
                        },
                    })
                    .into_iter()
                    .collect();
                transitions.push(CandidateTransition {
                    id: transition_id.clone(),
                    label: format!(
                        "{} room {} exit {} to {} room {} point {}",
                        inventory.stage,
                        trigger.room,
                        trigger.collision_exit_id,
                        trigger.destination_stage,
                        trigger.destination_room,
                        trigger.destination_point
                    ),
                    scope: scope.clone(),
                    transition_kind: TransitionKind::EncodedMapExit,
                    approach_id,
                    activation: ActivationContract {
                        hard_guards: source_location_guard(&inventory.stage, trigger.room),
                        physical_obligation_ids: vec![obligation_id],
                        effects: vec![StateOperation::SetLocation {
                            location: SceneLocation {
                                stage: trigger.destination_stage.clone(),
                                room: trigger.destination_room,
                                layer: trigger.destination_layer,
                                spawn: trigger.destination_point,
                            },
                        }],
                        unknown_requirements,
                    },
                    evidence,
                });
                transition_ids_by_exit
                    .entry(trigger.scls_id.as_str())
                    .or_default()
                    .push(transition_id);
            }

            for exit in &inventory.exits {
                let id = stable_token(
                    "world.encoded-exit",
                    &[inventory.stage.as_bytes(), exit.stable_id.as_bytes()],
                );
                let mut candidate_transition_ids = transition_ids_by_exit
                    .remove(exit.stable_id.as_str())
                    .unwrap_or_default();
                candidate_transition_ids.sort();
                encoded_exits.push(ExtractedEncodedExit {
                    id,
                    source_record_id: exit.stable_id.clone(),
                    source_stage: inventory.stage.clone(),
                    source_room: exit.scope.room,
                    destination: SceneLocation {
                        stage: exit.destination_stage.clone(),
                        room: exit.destination_room,
                        layer: exit.destination_layer,
                        spawn: exit.destination_point,
                    },
                    wipe: exit.wipe,
                    wipe_time: exit.wipe_time,
                    time_hour: exit.time_hour,
                    raw: decode_hex(&exit.raw_hex)?,
                    candidate_transition_ids,
                });
            }
        }

        static_world_objects.sort_by(|left, right| left.id.cmp(&right.id));
        spawns.sort_by(|left, right| left.id.cmp(&right.id));
        encoded_exits.sort_by(|left, right| left.id.cmp(&right.id));
        obligations.sort_by(|left, right| left.id.cmp(&right.id));
        transitions.sort_by(|left, right| left.id.cmp(&right.id));
        let facts = Self {
            schema: EXTRACTED_WORLD_FACTS_SCHEMA.into(),
            exact_context,
            world_context_sha256: world_context
                .digest()
                .map_err(|error| world_error("world_context", error))?,
            inventories: sources,
            static_world_objects,
            spawns,
            encoded_exits,
            mechanics: MechanicsCatalog {
                schema: MECHANICS_CATALOG_SCHEMA.into(),
                transitions,
                obligations,
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
        };
        facts.validate()?;
        Ok(facts)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != EXTRACTED_WORLD_FACTS_SCHEMA || self.world_context_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "extracted_world_facts",
                "has an unsupported schema or zero world-context digest",
            ));
        }
        ContextSelector::Exact {
            context: self.exact_context.clone(),
        }
        .validate()?;
        if self.inventories.is_empty() || self.inventories.len() > 256 {
            return Err(PlannerContractError::new(
                "inventories",
                "must contain between 1 and 256 exact stage inventories",
            ));
        }
        validate_sorted("inventories", &self.inventories, |value| {
            value.stage.as_str()
        })?;
        for source in &self.inventories {
            validate_game_name("inventories.stage", &source.stage)?;
            if source.inventory_sha256 == Digest::ZERO
                || source.spatial_index_sha256 == Digest::ZERO
            {
                return Err(PlannerContractError::new(
                    "inventories",
                    "contains a zero digest",
                ));
            }
        }
        validate_sorted(
            "static_world_objects",
            &self.static_world_objects,
            |value| value.id.as_str(),
        )?;
        for object in &self.static_world_objects {
            validate_static_object(object)?;
        }
        validate_sorted("spawns", &self.spawns, |value| value.id.as_str())?;
        let object_ids = self
            .static_world_objects
            .iter()
            .map(|object| object.id.as_str())
            .collect::<BTreeSet<_>>();
        for spawn in &self.spawns {
            validate_stable_id("spawns.id", &spawn.id)?;
            validate_stable_id("spawns.source_object_id", &spawn.source_object_id)?;
            if !object_ids.contains(spawn.source_object_id.as_str()) {
                return Err(PlannerContractError::new(
                    "spawns.source_object_id",
                    "does not reference an imported static object",
                ));
            }
            spawn.location.validate()?;
            if !canonical_position(spawn.position) {
                return Err(PlannerContractError::new(
                    "spawns.position",
                    "must contain finite canonical coordinates",
                ));
            }
        }
        self.mechanics.validate()?;
        validate_sorted("encoded_exits", &self.encoded_exits, |value| {
            value.id.as_str()
        })?;
        let transition_ids = self
            .mechanics
            .transitions
            .iter()
            .map(|transition| transition.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut referenced_transition_ids = BTreeSet::new();
        for exit in &self.encoded_exits {
            validate_stable_id("encoded_exits.id", &exit.id)?;
            validate_game_name("encoded_exits.source_stage", &exit.source_stage)?;
            exit.destination.validate()?;
            if exit.raw.len() != 13
                || !strictly_sorted(&exit.candidate_transition_ids)
                || exit.candidate_transition_ids.iter().any(|id| {
                    !transition_ids.contains(id.as_str())
                        || !referenced_transition_ids.insert(id.as_str())
                })
            {
                return Err(PlannerContractError::new(
                    "encoded_exits",
                    "contains invalid raw data or transition references",
                ));
            }
        }
        if referenced_transition_ids != transition_ids {
            return Err(PlannerContractError::new(
                "mechanics.transitions",
                "must be referenced exactly once by an encoded exit",
            ));
        }
        let expected_scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: self.exact_context.clone(),
            }],
        };
        if self
            .mechanics
            .transitions
            .iter()
            .any(|transition| transition.scope != expected_scope)
            || self
                .mechanics
                .obligations
                .iter()
                .any(|obligation| obligation.scope != expected_scope)
        {
            return Err(PlannerContractError::new(
                "mechanics.scope",
                "does not match the exact imported context",
            ));
        }
        let total = self.static_world_objects.len()
            + self.spawns.len()
            + self.encoded_exits.len()
            + self.mechanics.transitions.len()
            + self.mechanics.obligations.len();
        if total > MAX_EXTRACTED_WORLD_RECORDS {
            return Err(PlannerContractError::new(
                "extracted_world_facts",
                "contains too many records",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let facts: Self = serde_json::from_slice(bytes)?;
        facts.validate()?;
        if facts.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "extracted_world_facts",
                "is not canonical JSON",
            ));
        }
        Ok(facts)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

fn import_static_object(
    stage: &str,
    placement: &PlacementRecord,
) -> Result<StaticWorldObject, PlannerContractError> {
    let raw = decode_hex(&placement.raw_hex)?;
    let id = stable_token(
        "world.object",
        &[stage.as_bytes(), placement.stable_id.as_bytes()],
    );
    let binding = match placement.scope {
        crate::world_data::SourceScope {
            kind: SourceKind::Stage,
            ..
        } => ComponentBinding::Stage {
            stage: stage.into(),
        },
        crate::world_data::SourceScope {
            kind: SourceKind::Room,
            room: Some(room),
        } => ComponentBinding::Room {
            stage: stage.into(),
            room,
        },
        _ => {
            return Err(PlannerContractError::new(
                "placement.scope",
                "has an invalid stage/room binding",
            ));
        }
    };
    let mut parameters = BTreeMap::new();
    parameters.insert("name".into(), StateValue::Text(placement.name.clone()));
    parameters.insert(
        "parameters".into(),
        StateValue::Unsigned(placement.parameters.into()),
    );
    parameters.insert(
        "set_id".into(),
        StateValue::Unsigned(placement.set_id.into()),
    );
    parameters.insert(
        "layer".into(),
        StateValue::Signed(placement.layer.map_or(-1, i64::from)),
    );
    parameters.insert(
        "position_f32_le".into(),
        StateValue::Bytes(f32_bytes([
            placement.position.x,
            placement.position.y,
            placement.position.z,
        ])),
    );
    parameters.insert(
        "angle_i16_le".into(),
        StateValue::Bytes(i16_bytes(placement.angle)),
    );
    parameters.insert("raw_record".into(), StateValue::Bytes(raw.clone()));
    parameters.insert(
        "source_record_id".into(),
        StateValue::Text(placement.stable_id.clone()),
    );
    Ok(StaticWorldObject {
        id,
        actor_type: actor_type(placement),
        placement_sha256: Digest(Sha256::digest(&raw).into()),
        binding,
        parameters,
    })
}

fn import_spawn(
    stage: &str,
    placement: &PlacementRecord,
    source_object_id: &str,
) -> Result<ExtractedSpawn, PlannerContractError> {
    let position = canonicalize_position([
        placement.position.x,
        placement.position.y,
        placement.position.z,
    ]);
    Ok(ExtractedSpawn {
        id: stable_token(
            "world.spawn",
            &[stage.as_bytes(), placement.stable_id.as_bytes()],
        ),
        source_object_id: source_object_id.into(),
        source_record_id: placement.stable_id.clone(),
        location: SceneLocation {
            stage: stage.into(),
            room: placement.scope.room.unwrap_or(-1),
            layer: placement.layer.map_or(-1, |layer| layer as i8),
            spawn: (placement.angle[2] as u16 & 0xff) as i16,
        },
        position,
        rotation: placement.angle,
        parameters: placement.parameters,
    })
}

fn source_location_guard(stage: &str, room: i8) -> PredicateExpression {
    PredicateExpression::All {
        terms: vec![
            PredicateExpression::Compare {
                left: ValueReference::LocationStage,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text(stage.into()),
                },
            },
            PredicateExpression::Compare {
                left: ValueReference::LocationRoom,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Signed(room.into()),
                },
            },
        ],
    }
}

fn extracted_evidence(inventory_sha256: Digest, token: &str, inferred: bool) -> RuleEvidence {
    RuleEvidence {
        truth: if inferred {
            TruthStatus::Contested
        } else {
            TruthStatus::Established
        },
        records: vec![EvidenceRecord {
            id: format!("evidence.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(inventory_sha256),
            note: if inferred {
                "Extracted collision exit code joined to the room SCLS index; activation semantics remain inferred.".into()
            } else {
                "Extracted from an immutable world inventory.".into()
            },
        }],
    }
}

fn actor_type(placement: &PlacementRecord) -> String {
    if placement.kind == PlacementKind::PlayerSpawn {
        return "player-spawn".into();
    }
    let normalized = placement
        .name
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                byte.to_ascii_lowercase() as char
            } else if matches!(byte, b'_' | b'-') {
                byte as char
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("actor.{normalized}")
}

fn stable_token(domain: &str, parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update([0]);
        hasher.update(part);
    }
    format!("{domain}.{}", Digest(hasher.finalize().into()))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, PlannerContractError> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PlannerContractError::new("raw_hex", "is not canonical hex"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII was validated");
            u8::from_str_radix(pair, 16)
                .map_err(|_| PlannerContractError::new("raw_hex", "is not canonical hex"))
        })
        .collect()
}

fn f32_bytes(values: [f32; 3]) -> Vec<u8> {
    canonicalize_position(values)
        .into_iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .collect()
}

fn i16_bytes(values: [i16; 3]) -> Vec<u8> {
    values.into_iter().flat_map(i16::to_le_bytes).collect()
}

fn canonicalize_position(mut values: [f32; 3]) -> [f32; 3] {
    for value in &mut values {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    values
}

fn canonical_position(values: [f32; 3]) -> bool {
    values
        .iter()
        .all(|value| value.is_finite() && value.to_bits() != (-0.0_f32).to_bits())
}

fn validate_game_name(field: &str, value: &str) -> Result<(), PlannerContractError> {
    if value.is_empty()
        || value.len() > 8
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(PlannerContractError::new(field, "is not a valid game name"));
    }
    Ok(())
}

fn validate_sorted<T>(
    field: &str,
    values: &[T],
    key: impl Fn(&T) -> &str,
) -> Result<(), PlannerContractError> {
    if values.windows(2).any(|pair| key(&pair[0]) >= key(&pair[1])) {
        return Err(PlannerContractError::new(
            field,
            "must be unique and sorted",
        ));
    }
    Ok(())
}

fn strictly_sorted(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn world_error(field: &str, error: impl std::fmt::Display) -> PlannerContractError {
    PlannerContractError::new(field, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{
        CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
        RUNTIME_CONFIGURATION_SCHEMA,
    };
    use crate::world_data::{
        CollisionCode, CollisionInventoryRecord, CollisionLoadTrigger, KclAuthoredPrism,
        KclInventoryPrism, KclReconstruction, KclSourceIndices, PlacementKind, PlacementRecord,
        SourceKind, SourceScope, StageExitRecord, Vec3, WORLD_INVENTORY_SCHEMA, WorldContext,
        WorldInventory, WorldSource,
    };

    fn content() -> ContentIdentity {
        ContentIdentity {
            schema: CONTENT_IDENTITY_SCHEMA.into(),
            id: "gcn-us-test".into(),
            fingerprint: ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "test".into(),
                product_id: "GZ2E01".into(),
                executable_sha256: Digest([1; 32]),
                game_data_sha256: Digest([2; 32]),
                resource_manifest_sha256: Digest([3; 32]),
            },
        }
    }

    fn runtime(content: &ContentIdentity) -> RuntimeConfiguration {
        RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: content.digest().unwrap(),
            language: "en".into(),
            settings: BTreeMap::new(),
        }
    }

    fn placement(
        stable_id: &str,
        kind: PlacementKind,
        name: &str,
        angle_z: i16,
    ) -> PlacementRecord {
        PlacementRecord {
            stable_id: stable_id.into(),
            source_sha256: Digest([12; 32]),
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(0),
            },
            chunk_tag: if kind == PlacementKind::PlayerSpawn {
                "PLYR".into()
            } else {
                "ACTR".into()
            },
            record_index: 0,
            layer: None,
            kind,
            name: name.into(),
            parameters: 0x1234,
            position: Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
            angle: [0, 0x1000, angle_z],
            set_id: 7,
            scale_raw: None,
            raw_hex: "00".repeat(32),
        }
    }

    fn inventory(linked: bool) -> WorldInventory {
        let kcl = Digest([14; 32]);
        let plc = Digest([15; 32]);
        let collision_id = format!("kcl-sha256:{kcl}/plc-sha256:{plc}/prism/1");
        let exit_id = "dzr-sha256:fixture/chunk/SCLS/record/0".to_string();
        let raw_code = [if linked { 0 } else { 0x3f }, 0, 0, 0, 0];
        let collision = CollisionInventoryRecord {
            room: 0,
            prism: KclInventoryPrism {
                authored: KclAuthoredPrism {
                    stable_id: collision_id.clone(),
                    prism_index: 1,
                    height: 1.0,
                    source_indices: KclSourceIndices {
                        position: 0,
                        face_normal: 0,
                        edge_normal_1: 0,
                        edge_normal_2: 0,
                        edge_normal_3: 0,
                    },
                    attribute: 0,
                    code: CollisionCode {
                        raw: raw_code,
                        exit_id: if linked { 0 } else { 0x3f },
                        polygon_color: 0,
                        special_code: 0,
                        link_no: 0,
                        wall_code: 0,
                        attribute_0: 0,
                        attribute_1: 0,
                        ground_code: 0,
                        camera_move_background: 0,
                        room_camera: 0,
                        room_path: 0,
                        room_path_point: 0,
                        room_info: 0,
                        sound_id: 0,
                        room: 0,
                    },
                },
                reconstruction: KclReconstruction::Degenerate {
                    reason: "fixture".into(),
                },
            },
        };
        let exit = StageExitRecord {
            stable_id: exit_id.clone(),
            source_sha256: Digest([12; 32]),
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(0),
            },
            chunk_tag: "SCLS".into(),
            record_index: 0,
            destination_stage: "F_SP104".into(),
            destination_point: 2,
            destination_room: 1,
            destination_layer: -1,
            wipe: 0,
            wipe_time: 0,
            time_hour: -1,
            raw_start: 2,
            raw_field_a: 0,
            raw_field_b: 0x0f,
            raw_wipe: 15,
            raw_hex: "00".repeat(13),
        };
        let load_triggers = if linked {
            let mut hasher = Sha256::new();
            hasher.update(b"dusklight.collision-load-trigger/v1\0");
            hasher.update(collision_id.as_bytes());
            hasher.update([0]);
            hasher.update(exit_id.as_bytes());
            let digest = Digest(hasher.finalize().into());
            vec![CollisionLoadTrigger {
                stable_id: format!("load-trigger-sha256:{digest}"),
                room: 0,
                collision_id,
                collision_exit_id: 0,
                scls_id: exit_id,
                destination_stage: "F_SP104".into(),
                destination_room: 1,
                destination_layer: -1,
                destination_point: 2,
                inferred_semantics: true,
            }]
        } else {
            Vec::new()
        };
        WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: "F_SP103".into(),
            sources: vec![
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Stage,
                        room: None,
                    },
                    archive_sha256: Digest([10; 32]),
                    stage_data_path: "stage.dzs".into(),
                    stage_data_sha256: Digest([11; 32]),
                    kcl_path: None,
                    kcl_sha256: None,
                    plc_path: None,
                    plc_sha256: None,
                    addressable_prisms: 0,
                },
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Room,
                        room: Some(0),
                    },
                    archive_sha256: Digest([13; 32]),
                    stage_data_path: "room.dzr".into(),
                    stage_data_sha256: Digest([12; 32]),
                    kcl_path: Some("room.kcl".into()),
                    kcl_sha256: Some(kcl),
                    plc_path: Some("room.plc".into()),
                    plc_sha256: Some(plc),
                    addressable_prisms: 1,
                },
            ],
            chunks: Vec::new(),
            placements: vec![placement(
                "actor-record",
                PlacementKind::Actor,
                "kytag14",
                0,
            )],
            player_spawns: vec![placement(
                "spawn-record",
                PlacementKind::PlayerSpawn,
                "start",
                5,
            )],
            exits: vec![exit],
            collisions: vec![collision],
            load_triggers,
        }
    }

    fn world_context(game_data_sha256: Digest, inventory: &WorldInventory) -> WorldContext {
        let context = WorldContext {
            schema: crate::world_data::WORLD_CONTEXT_SCHEMA.into(),
            game_data_sha256,
            stages: vec![crate::world_data::WorldContextStage {
                stage: inventory.stage.clone(),
                inventory_sha256: inventory.digest().unwrap(),
                spatial_index_sha256: Digest([99; 32]),
            }],
        };
        context.validate().unwrap();
        context
    }

    #[test]
    fn imports_joined_exit_as_obstructed_candidate_without_claiming_feasibility() {
        let content = content();
        let runtime = runtime(&content);
        let inventory = inventory(true);
        inventory.validate().unwrap();
        let context = world_context(Digest([2; 32]), &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();
        assert_eq!(facts.static_world_objects.len(), 2);
        assert_eq!(facts.spawns[0].location.spawn, 5);
        assert_eq!(facts.mechanics.transitions.len(), 1);
        assert_eq!(facts.mechanics.obligations.len(), 1);
        assert_eq!(facts.encoded_exits[0].candidate_transition_ids.len(), 1);
        let transition = &facts.mechanics.transitions[0];
        assert_eq!(transition.evidence.truth, TruthStatus::Contested);
        assert_eq!(transition.activation.unknown_requirements.len(), 1);
        assert_eq!(transition.activation.physical_obligation_ids.len(), 1);
        assert!(matches!(
            transition.activation.effects.as_slice(),
            [StateOperation::SetLocation { location }]
                if location.stage == "F_SP104" && location.room == 1 && location.spawn == 2
        ));
        let bytes = facts.canonical_bytes().unwrap();
        assert_eq!(
            ExtractedWorldFacts::decode_canonical(&bytes).unwrap(),
            facts
        );
        assert_ne!(facts.digest().unwrap(), Digest::ZERO);
    }

    #[test]
    fn keeps_unjoined_scls_as_encoded_fact_without_inventing_transition() {
        let content = content();
        let runtime = runtime(&content);
        let inventory = inventory(false);
        inventory.validate().unwrap();
        let context = world_context(Digest([2; 32]), &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();
        assert!(facts.mechanics.transitions.is_empty());
        assert!(facts.mechanics.obligations.is_empty());
        assert!(facts.encoded_exits[0].candidate_transition_ids.is_empty());
    }

    #[test]
    fn exact_content_and_world_context_must_agree() {
        let content = content();
        let runtime = runtime(&content);
        let inventory = inventory(false);
        let context = world_context(Digest([9; 32]), &inventory);
        assert_eq!(
            ExtractedWorldFacts::build(
                &content,
                &runtime,
                &context,
                std::slice::from_ref(&inventory),
            )
            .unwrap_err()
            .field(),
            "world_context.game_data_sha256"
        );
    }
}
