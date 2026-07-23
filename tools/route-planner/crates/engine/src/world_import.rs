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
    ComponentBinding, ComponentBindingReference, ComponentKind, SceneLocation, StateValue,
    StaticWorldObject, validate_static_object,
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

pub const EXTRACTED_WORLD_FACTS_SCHEMA: &str = "dusklight.route-planner.extracted-world-facts/v9";
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

            let mut transition_ids_by_exit = BTreeMap::<String, Vec<String>>::new();
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
                    .entry(trigger.scls_id.clone())
                    .or_default()
                    .push(transition_id);
            }

            if is_source_audited_gz2e01(content) {
                for placement in &inventory.placements {
                    let Some(imported) =
                        import_gz2e01_boss_door(inventory, placement, &scope, inventory_sha256)?
                    else {
                        continue;
                    };
                    transition_ids_by_exit
                        .entry(imported.exit_record_id)
                        .or_default()
                        .push(imported.transition.id.clone());
                    obligations.extend(imported.obligations);
                    transitions.push(imported.transition);
                }
                for placement in &inventory.placements {
                    let Some(imported) = import_gz2e01_keyed_actor_actions(
                        inventory,
                        placement,
                        &scope,
                        inventory_sha256,
                    )?
                    else {
                        continue;
                    };
                    if let Some(exit_record_id) = imported.exit_record_id {
                        transition_ids_by_exit
                            .entry(exit_record_id)
                            .or_default()
                            .extend(
                                imported
                                    .transitions
                                    .iter()
                                    .map(|transition| transition.id.clone()),
                            );
                    }
                    obligations.extend(imported.obligations);
                    transitions.extend(imported.transitions);
                }
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
        let exit_transition_ids = self
            .mechanics
            .transitions
            .iter()
            .filter(|transition| {
                matches!(
                    transition.transition_kind,
                    TransitionKind::EncodedMapExit | TransitionKind::Door
                )
            })
            .map(|transition| transition.id.as_str())
            .collect::<BTreeSet<_>>();
        if referenced_transition_ids != exit_transition_ids {
            return Err(PlannerContractError::new(
                "mechanics.transitions",
                "encoded-map and door transitions must be referenced exactly once by an encoded exit, while other transition kinds must not be referenced by one",
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

struct ImportedBossDoor {
    exit_record_id: String,
    transition: CandidateTransition,
    obligations: Vec<FeasibilityObligation>,
}

#[derive(Clone, Copy)]
enum Gz2e01BossDoorFamily {
    L1,
    L5,
}

fn import_gz2e01_boss_door(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedBossDoor>, PlannerContractError> {
    if !matches!(
        placement.kind,
        PlacementKind::Actor | PlacementKind::ScaledActor
    ) {
        return Ok(None);
    }
    let Some(family) = gz2e01_boss_door_family(&placement.name) else {
        return Ok(None);
    };
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let write_unlock_switch = match family {
        Gz2e01BossDoorFamily::L1 => {
            if (inventory.stage == "D_MN08A" && room == 10)
                || (inventory.stage != "D_MN08A" && room == 50)
            {
                return Ok(None);
            }
            true
        }
        Gz2e01BossDoorFamily::L5 => match inventory.stage.as_str() {
            "D_MN11" => true,
            "D_MN11A" => false,
            _ => return Ok(None),
        },
    };

    let exit_index = ((placement.parameters >> 25) & 0x3f) as usize;
    let matching_exits = inventory
        .exits
        .iter()
        .filter(|exit| exit.record_index == exit_index && exit.scope.room == Some(room))
        .collect::<Vec<_>>();
    let [exit] = matching_exits.as_slice() else {
        return Ok(None);
    };

    // dSv_info_c routes switch IDs below 0x80 into dSv_memBit_c. Other
    // switch domains require their own backing-store import before their writes
    // can be claimed, so this importer deliberately leaves those placements as
    // encoded exits without a boss-door candidate.
    let switch_id = placement.angle[2] as u16 as u8;
    if switch_id >= 0x80 {
        return Ok(None);
    }
    let (switch_byte_offset, switch_mask) = memory_switch_raw_location(switch_id);
    let front_room = ((placement.parameters >> 13) & 0x3f) as u8;
    let back_room = ((placement.parameters >> 19) & 0x3f) as u8;
    let family_token = match family {
        Gz2e01BossDoorFamily::L1 => "l1",
        Gz2e01BossDoorFamily::L5 => "l5",
    };
    let token = stable_token(
        &format!("world.gz2e01.{family_token}-boss-door"),
        &[
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
            exit.stable_id.as_bytes(),
        ],
    );
    let transition_id = format!("transition.{token}");
    let approach_id = format!("approach.{token}");
    let interaction_obligation_id = format!("obligation.interaction.{token}");
    let actor_obligation_id = format!("obligation.actor-state.{token}");
    let evidence = gz2e01_boss_door_evidence(family, inventory_sha256, placement, &token);
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let boss_key_guard = PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: 0x1d,
            byte_width: 1,
            mask: 0x04,
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(0x04),
        },
    };
    let mut hard_guard_terms = vec![
        source_location_guard(&inventory.stage, room),
        boss_key_guard,
    ];
    if matches!(family, Gz2e01BossDoorFamily::L5) {
        hard_guard_terms.push(PredicateExpression::Compare {
            left: ValueReference::PlayerForm,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text("human".into()),
            },
        });
    }
    let destination = SceneLocation {
        stage: exit.destination_stage.clone(),
        room: exit.destination_room,
        layer: exit.destination_layer,
        spawn: exit.destination_point,
    };

    let mut effects = Vec::new();
    if write_unlock_switch {
        effects.push(StateOperation::WriteBoundRaw {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: switch_byte_offset,
            mask: vec![switch_mask],
            value: vec![switch_mask],
        });
    }
    effects.push(StateOperation::SetLocation {
        location: destination.clone(),
    });
    let (actor_question, interaction_question) = match family {
        Gz2e01BossDoorFamily::L1 => (
            "Confirm the loaded boss-door resources and actor/event phases reach INIT, UNLOCK, collision release, CHG_SCENE, and restart handling without an intervening failure or interruption.",
            "Derive the actor-local interaction volume and facing test: |x| <= 200, |z| <= 100, wolf attention/current-position constraints, and facing delta <= 0x4000, then connect them to extracted oriented geometry.",
        ),
        Gz2e01BossDoorFamily::L5 => (
            "Confirm the loaded L5 boss-door resources and event phases reach UNLOCK, key deletion when present, collision release, CHG_SCENE, close/end handling, and the restart-room write without an intervening failure or interruption.",
            "Derive the L5 actor-local usable side and interaction volume: local z must be positive, |x| <= 200, |z| <= 100, and facing delta <= 0x4000, then connect them to extracted oriented geometry.",
        ),
    };

    Ok(Some(ImportedBossDoor {
        exit_record_id: exit.stable_id.clone(),
        transition: CandidateTransition {
            id: transition_id,
            label: format!(
                "{} {} room {} boss door (front {}, back {}, exit {}) to {} room {} point {}",
                inventory.stage,
                placement.name,
                room,
                front_room,
                back_room,
                exit_index,
                destination.stage,
                destination.room,
                destination.spawn
            ),
            scope: scope.clone(),
            transition_kind: TransitionKind::Door,
            approach_id,
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: hard_guard_terms,
                },
                physical_obligation_ids: vec![
                    actor_obligation_id.clone(),
                    interaction_obligation_id.clone(),
                ],
                effects,
                unknown_requirements: Vec::new(),
            },
            evidence,
        },
        obligations: vec![
            FeasibilityObligation {
                id: actor_obligation_id,
                label: format!(
                    "Run the loaded {} keyhole, event, collision, and scene-change phases",
                    placement.name
                ),
                scope: scope.clone(),
                obligation_kind: ObligationKind::ActorState,
                detail: ObligationDetail::Unresolved {
                    research_question: actor_question.into(),
                },
                evidence: unknown_evidence.clone(),
            },
            FeasibilityObligation {
                id: interaction_obligation_id,
                label: format!("Reach and face {} from its usable side", placement.name),
                scope: scope.clone(),
                obligation_kind: ObligationKind::Interaction,
                detail: ObligationDetail::Unresolved {
                    research_question: interaction_question.into(),
                },
                evidence: unknown_evidence,
            },
        ],
    }))
}

fn gz2e01_boss_door_family(name: &str) -> Option<Gz2e01BossDoorFamily> {
    match name {
        "L1Bdoor" | "L2Bdoor" | "L4Bdoor" | "L6Bdoor" | "L7Bdoor" | "L8Bdoor" | "L9Bdoor" => {
            Some(Gz2e01BossDoorFamily::L1)
        }
        "L5Bdoor" => Some(Gz2e01BossDoorFamily::L5),
        _ => None,
    }
}

fn memory_switch_raw_location(switch_id: u8) -> (u32, u8) {
    let word = u32::from(switch_id / 32);
    let bit_in_word = switch_id % 32;
    let byte_in_word = 3 - u32::from(bit_in_word / 8);
    let byte_offset = 0x08 + word * 4 + byte_in_word;
    (byte_offset, 1_u8 << (bit_in_word % 8))
}

fn is_source_audited_gz2e01(content: &ContentIdentity) -> bool {
    crate::orig_discovery::bundled_supported_build_registry()
        .ok()
        .and_then(|registry| {
            registry
                .identities
                .into_iter()
                .find(|identity| identity.id == "gcn-us-1.0-gz2e01")
        })
        .is_some_and(|identity| identity.fingerprint == content.fingerprint)
}

fn gz2e01_boss_door_evidence(
    family: Gz2e01BossDoorFamily,
    inventory_sha256: Digest,
    placement: &PlacementRecord,
    token: &str,
) -> RuleEvidence {
    let (actor_sha256, actor_note) = match family {
        Gz2e01BossDoorFamily::L1 => (
            "221c170e034cf90cc43b20dc737bebeb44d6f8b54111d4454024f2fea7069d79",
            "d_a_door_bossL1.cpp: boss-key/front/area offer guards; unlock switch, event phases, collision release, and scene-change behavior.",
        ),
        Gz2e01BossDoorFamily::L5 => (
            "9f649b99f027e39f1d39ce066d815a78032b536c4a9a83e0361681af2265102e",
            "d_a_door_bossL5.cpp: human/boss-key/front/area guards; first-unlock switch, keyhole/event phases, collision release, scene change, and restart behavior.",
        ),
    };
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: format!("evidence.source.actor.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(actor_sha256)),
                note: actor_note.into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.name-map.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
                )),
                note: "d_stage.cpp: boss-door placement names map to their exact actor process families.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.parameters.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "b0dacfc4b9c46786d73a840e55385e535364b9fee7de66cd0e2af18f25d1ca78",
                )),
                note: "d_door_param2.cpp: front/back room, exit number, and unlock-switch parameter decoding.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.save-layout.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "74a211e5d2ee2c0fe4ce259905fe1f479f373d5b2459d654871cbbd2f61e8756",
                )),
                note: "d_save.h: dSv_memBit_c switch array, key count, and dungeon-item backing layout.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.world.inventory.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(inventory_sha256),
                note: format!(
                    "Authenticated world inventory placement {} from resource {}.",
                    placement.stable_id, placement.source_sha256
                ),
            },
        ],
    }
}

struct ImportedKeyedActorActions {
    exit_record_id: Option<String>,
    transitions: Vec<CandidateTransition>,
    obligations: Vec<FeasibilityObligation>,
}

fn import_gz2e01_keyed_actor_actions(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    if !matches!(
        placement.kind,
        PlacementKind::Actor | PlacementKind::ScaledActor
    ) {
        return Ok(None);
    }
    match placement.name.as_str() {
        "L6Mdoor" | "L7door" | "L8Mdoor" => {
            import_gz2e01_keyed_mboss_door(inventory, placement, scope, inventory_sha256)
        }
        "kshtr00" | "L3Bdoor" => {
            import_gz2e01_key_shutter(inventory, placement, scope, inventory_sha256)
        }
        "K_Gate" => import_gz2e01_koki_gate(inventory, placement, scope, inventory_sha256),
        _ => Ok(None),
    }
}

fn import_gz2e01_keyed_mboss_door(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let front_option = ((placement.parameters >> 8) & 0x03) as u8;
    let front_room = ((placement.parameters >> 13) & 0x3f) as u8;
    if front_option != 2 || room == 51 || room == 52 || front_room != room as u8 {
        return Ok(None);
    }
    let switch_id = placement.angle[2] as u16 as u8;
    if switch_id >= 0x80 {
        return Ok(None);
    }
    let exit_index = ((placement.parameters >> 25) & 0x3f) as usize;
    let matching_exits = inventory
        .exits
        .iter()
        .filter(|exit| exit.record_index == exit_index && exit.scope.room == Some(room))
        .collect::<Vec<_>>();
    let [exit] = matching_exits.as_slice() else {
        return Ok(None);
    };
    let (event_sha256, event_note) = match placement.name.as_str() {
        "L6Mdoor" => (
            "fd5570eca9bd29ee1b433236a10945872930fbf52c2508af9ff2c3f7ea9386fe",
            "L3MBdoor/event_list.dat: DEFAULT_MBS_SHUTTER_L3_F reaches UNLOCK before OPEN and CHG_SCENE.",
        ),
        "L7door" => (
            "7de6bfac10e3ca6c3f6bc88a83815972d3397fd3488b067398cdd8cb0ea0cce4",
            "L7MBdoor/event_list.dat: DEFAULT_MBS_SHUTTER_L7_F reaches UNLOCK before OPEN and CHG_SCENE.",
        ),
        "L8Mdoor" => (
            "b079b8b284208582d9a37b50bd94f13400530abca75db0771147a646a8d83627",
            "L8MBdoor/event_list.dat: DEFAULT_MBS_SHUTTER_L8_F reaches UNLOCK before OPEN and CHG_SCENE.",
        ),
        _ => return Ok(None),
    };
    let family = "keyed-mboss-door";
    let base_token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = keyed_actor_evidence(
        family,
        inventory_sha256,
        placement,
        "94b00ab791e96a5738a0c2ef94945461c4e930b6128fc5a16d13630da9d1dff2",
        "d_a_door_mbossL1.cpp: front-side option-2 key guard, one-time switch write/key decrement, event/collision phases, and scene change.",
        event_sha256,
        event_note,
        true,
        &base_token,
    );
    let (obligations, obligation_ids) = keyed_actor_obligations(
        scope,
        placement,
        &base_token,
        &evidence,
        "Confirm resources, keyhole when present, the selected retail event cuts, collision release, CHG_SCENE, restart handling, and an uncontended queued key-delta commit complete without interruption.",
        "Reach the authored front side inside |x| <= 130 and |z| <= 110 with the required facing; wolf attention/current-position checks remain part of the physical witness.",
    );
    let location_guard = placement_location_guard(inventory, placement, room);
    let destination = SceneLocation {
        stage: exit.destination_stage.clone(),
        room: exit.destination_room,
        layer: exit.destination_layer,
        spawn: exit.destination_point,
    };
    let first_open = keyed_actor_candidate(
        scope,
        placement,
        family,
        "first-open",
        &format!(
            "{} {} room {} first keyed opening to {} room {} point {}",
            inventory.stage,
            placement.name,
            room,
            destination.stage,
            destination.room,
            destination.spawn
        ),
        TransitionKind::Door,
        PredicateExpression::All {
            terms: vec![
                location_guard.clone(),
                memory_switch_guard(switch_id, false),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
                small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
            ],
        },
        vec![
            memory_switch_write(switch_id),
            small_key_adjust(-1),
            StateOperation::SetLocation {
                location: destination.clone(),
            },
        ],
        &obligation_ids,
        &evidence,
    );
    let first_open_high_key = keyed_actor_candidate(
        scope,
        placement,
        family,
        "first-open-high-key-clamp",
        &format!(
            "{} {} room {} first keyed opening from a high raw key count to {} room {} point {}",
            inventory.stage,
            placement.name,
            room,
            destination.stage,
            destination.room,
            destination.spawn
        ),
        TransitionKind::Door,
        PredicateExpression::All {
            terms: vec![
                location_guard.clone(),
                memory_switch_guard(switch_id, false),
                small_key_guard(ComparisonOperator::GreaterThan, 100),
            ],
        },
        vec![
            memory_switch_write(switch_id),
            small_key_write(99),
            StateOperation::SetLocation {
                location: destination.clone(),
            },
        ],
        &obligation_ids,
        &evidence,
    );
    let reopen = keyed_actor_candidate(
        scope,
        placement,
        family,
        "reopen",
        &format!(
            "{} {} room {} already-unlocked opening to {} room {} point {}",
            inventory.stage,
            placement.name,
            room,
            destination.stage,
            destination.room,
            destination.spawn
        ),
        TransitionKind::Door,
        PredicateExpression::All {
            terms: vec![location_guard, memory_switch_guard(switch_id, true)],
        },
        vec![StateOperation::SetLocation {
            location: destination,
        }],
        &obligation_ids,
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: Some(exit.stable_id.clone()),
        transitions: vec![first_open, first_open_high_key, reopen],
        obligations,
    }))
}

fn import_gz2e01_key_shutter(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let checks_key = placement.parameters >> 31 != 0;
    let authored_type = ((placement.parameters >> 8) & 0xff) as u8;
    let runtime_type = authored_type.wrapping_add(1);
    let supported_type = match (placement.name.as_str(), runtime_type) {
        ("kshtr00", 0) => Some((
            false,
            "8676effbd561ba65f8e4a8b9493aa6b60072d40f72a8e240b2ffa9c5550b40fa",
            "S_shut00/event_list.dat: KEY_JAIL_00 and its wolf variant both contain UNLOCK before OPEN.",
        )),
        ("kshtr00", 2) => Some((
            false,
            "3bff3ce52a0c1660d5ccf0bdcae24b672e50013317b3469698c51e32336c159a",
            "Lv3shut00/event_list.dat: KEY_JAIL_01 and its wolf variant both contain UNLOCK before OPEN.",
        )),
        ("L3Bdoor", 3) => Some((
            true,
            "2184efba5db7b458f01c50534e29ba072fcb58be5e3b6df8f92e35b758726440",
            "K_l3bdoor/event_list.dat: DEFAULT_BS_SHUTTER_L3_F contains UNLOCK before OPEN.",
        )),
        _ => None,
    };
    let Some((uses_boss_key, event_sha256, event_note)) = supported_type else {
        return Ok(None);
    };
    if !checks_key {
        return Ok(None);
    }
    let switch_id = placement.parameters as u8;
    if switch_id >= 0x80 {
        return Ok(None);
    }
    let family = if uses_boss_key {
        "lakebed-boss-key-shutter"
    } else {
        "key-shutter"
    };
    let base_token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = keyed_actor_evidence(
        family,
        inventory_sha256,
        placement,
        "dca04961403031ef232059f5f9f8997d2f0a3965b111e97d9d72604e0014d14b",
        "d_a_obj_kshutter.cpp: type/check-key decoder, small-key or boss-key offer guard, acceptance switch write, UNLOCK key delta, and collision/open phases.",
        event_sha256,
        event_note,
        false,
        &base_token,
    );
    let (obligations, obligation_ids) = keyed_actor_obligations(
        scope,
        placement,
        &base_token,
        &evidence,
        "Confirm resources, keyhole when present, accepted event, UNLOCK/OPEN cuts, collision release, and an uncontended queued key-delta commit complete without interruption.",
        "Reach the actor's bounded interaction area with the required facing; retain the human/wolf event choice as part of the witness.",
    );
    let location_guard = placement_location_guard(inventory, placement, room);
    let mut transitions = Vec::new();
    if uses_boss_key {
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "open-with-small-key",
            &format!(
                "{} {} room {} boss-key opening with incidental small-key decrement",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    location_guard.clone(),
                    memory_switch_guard(switch_id, false),
                    boss_key_guard(),
                    small_key_guard(ComparisonOperator::GreaterThan, 0),
                    small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
                ],
            },
            vec![memory_switch_write(switch_id), small_key_adjust(-1)],
            &obligation_ids,
            &evidence,
        ));
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "open-with-high-small-key-clamp",
            &format!(
                "{} {} room {} boss-key opening with high raw small keys clamped to 99",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    location_guard.clone(),
                    memory_switch_guard(switch_id, false),
                    boss_key_guard(),
                    small_key_guard(ComparisonOperator::GreaterThan, 100),
                ],
            },
            vec![memory_switch_write(switch_id), small_key_write(99)],
            &obligation_ids,
            &evidence,
        ));
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "open-with-zero-small-keys",
            &format!(
                "{} {} room {} boss-key opening with clamped zero small keys",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    location_guard,
                    memory_switch_guard(switch_id, false),
                    boss_key_guard(),
                    small_key_guard(ComparisonOperator::Equal, 0),
                ],
            },
            vec![memory_switch_write(switch_id)],
            &obligation_ids,
            &evidence,
        ));
    } else {
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "unlock",
            &format!(
                "{} {} room {} keyed shutter unlock",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    location_guard,
                    memory_switch_guard(switch_id, false),
                    small_key_guard(ComparisonOperator::GreaterThan, 0),
                    small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
                ],
            },
            vec![memory_switch_write(switch_id), small_key_adjust(-1)],
            &obligation_ids,
            &evidence,
        ));
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "unlock-high-key-clamp",
            &format!(
                "{} {} room {} keyed shutter unlock with high raw keys clamped to 99",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    placement_location_guard(inventory, placement, room),
                    memory_switch_guard(switch_id, false),
                    small_key_guard(ComparisonOperator::GreaterThan, 100),
                ],
            },
            vec![memory_switch_write(switch_id), small_key_write(99)],
            &obligation_ids,
            &evidence,
        ));
    }
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions,
        obligations,
    }))
}

fn import_gz2e01_koki_gate(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let name_argument = ((placement.parameters >> 16) & 0x0f) as u8;
    let switch_id = placement.parameters as u8;
    if name_argument != 0 || switch_id >= 0x80 {
        return Ok(None);
    }
    let family = "koki-gate";
    let base_token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = keyed_actor_evidence(
        family,
        inventory_sha256,
        placement,
        "55696f32a444f9fde4b446442211cc3bed8b2872c8b05d7646001bd3659879e8",
        "d_a_obj_kgate.cpp: type-0 switch/key offer guard, accepted-door key delta and switch write, live push/open behavior, and set-switch reload reconstruction.",
        "c8684156665423d1a133dc0b102098d8ec3be838dd6abe117dbafedf0144ab83",
        "D_KGate00/event_list.dat: KOKI_GATE_OPEN00 contains UNLOCK before OPEN.",
        false,
        &base_token,
    );
    let (obligations, obligation_ids) = keyed_actor_obligations(
        scope,
        placement,
        &base_token,
        &evidence,
        "Confirm resources, accepted door command, event cuts, an uncontended queued key-delta commit, and the unlocked gate's physical open/push behavior complete without interruption.",
        "Reach local x in [-100, 100], z in [0, 100], with the actor/player facing delta required by checkOpen().",
    );
    let transition = keyed_actor_candidate(
        scope,
        placement,
        family,
        "unlock",
        &format!(
            "{} {} room {} keyed gate unlock",
            inventory.stage, placement.name, room
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, room),
                memory_switch_guard(switch_id, false),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
                small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
            ],
        },
        vec![small_key_adjust(-1), memory_switch_write(switch_id)],
        &obligation_ids,
        &evidence,
    );
    let high_key_transition = keyed_actor_candidate(
        scope,
        placement,
        family,
        "unlock-high-key-clamp",
        &format!(
            "{} {} room {} keyed gate unlock with high raw keys clamped to 99",
            inventory.stage, placement.name, room
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, room),
                memory_switch_guard(switch_id, false),
                small_key_guard(ComparisonOperator::GreaterThan, 100),
            ],
        },
        vec![small_key_write(99), memory_switch_write(switch_id)],
        &obligation_ids,
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![transition, high_key_transition],
        obligations,
    }))
}

fn placement_location_guard(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    room: i8,
) -> PredicateExpression {
    let PredicateExpression::All { mut terms } = source_location_guard(&inventory.stage, room)
    else {
        unreachable!("source_location_guard always returns an all predicate")
    };
    if let Some(layer) = placement.layer {
        terms.push(PredicateExpression::Compare {
            left: ValueReference::LocationLayer,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Signed(layer.into()),
            },
        });
    }
    PredicateExpression::All { terms }
}

fn small_key_guard(operator: ComparisonOperator, value: u64) -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: 0x1c,
            byte_width: 1,
            mask: 0xff,
        },
        operator,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(value),
        },
    }
}

fn boss_key_guard() -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: 0x1d,
            byte_width: 1,
            mask: 0x04,
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(0x04),
        },
    }
}

fn memory_switch_guard(switch_id: u8, set: bool) -> PredicateExpression {
    let (byte_offset, mask) = memory_switch_raw_location(switch_id);
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset,
            byte_width: 1,
            mask: u64::from(mask),
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(if set { u64::from(mask) } else { 0 }),
        },
    }
}

fn memory_switch_write(switch_id: u8) -> StateOperation {
    let (byte_offset, mask) = memory_switch_raw_location(switch_id);
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset,
        mask: vec![mask],
        value: vec![mask],
    }
}

fn small_key_adjust(delta: i64) -> StateOperation {
    StateOperation::AdjustBoundRawUnsigned {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: 0x1c,
        byte_width: 1,
        delta,
    }
}

fn small_key_write(value: u8) -> StateOperation {
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: 0x1c,
        mask: vec![0xff],
        value: vec![value],
    }
}

#[allow(clippy::too_many_arguments)]
fn keyed_actor_candidate(
    scope: &ContextScope,
    placement: &PlacementRecord,
    family: &str,
    branch: &str,
    label: &str,
    transition_kind: TransitionKind,
    hard_guards: PredicateExpression,
    effects: Vec<StateOperation>,
    obligation_ids: &[String],
    evidence: &RuleEvidence,
) -> CandidateTransition {
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            branch.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    CandidateTransition {
        id: format!("transition.{token}"),
        label: label.into(),
        scope: scope.clone(),
        transition_kind,
        approach_id: format!("approach.{token}"),
        activation: ActivationContract {
            hard_guards,
            physical_obligation_ids: obligation_ids.to_vec(),
            effects,
            unknown_requirements: Vec::new(),
        },
        evidence: evidence.clone(),
    }
}

fn keyed_actor_obligations(
    scope: &ContextScope,
    placement: &PlacementRecord,
    token: &str,
    evidence: &RuleEvidence,
    actor_question: &str,
    interaction_question: &str,
) -> (Vec<FeasibilityObligation>, Vec<String>) {
    let actor_id = format!("obligation.actor-state.{token}");
    let interaction_id = format!("obligation.interaction.{token}");
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    (
        vec![
            FeasibilityObligation {
                id: actor_id.clone(),
                label: format!("Run the loaded {} unlock/open phases", placement.name),
                scope: scope.clone(),
                obligation_kind: ObligationKind::ActorState,
                detail: ObligationDetail::Unresolved {
                    research_question: actor_question.into(),
                },
                evidence: unknown_evidence.clone(),
            },
            FeasibilityObligation {
                id: interaction_id.clone(),
                label: format!("Reach and activate {}", placement.name),
                scope: scope.clone(),
                obligation_kind: ObligationKind::Interaction,
                detail: ObligationDetail::Unresolved {
                    research_question: interaction_question.into(),
                },
                evidence: unknown_evidence,
            },
        ],
        vec![actor_id, interaction_id],
    )
}

#[allow(clippy::too_many_arguments)]
fn keyed_actor_evidence(
    family: &str,
    inventory_sha256: Digest,
    placement: &PlacementRecord,
    actor_sha256: &str,
    actor_note: &str,
    event_sha256: &str,
    event_note: &str,
    include_mboss_parameters: bool,
    token: &str,
) -> RuleEvidence {
    let mut records = vec![
            EvidenceRecord {
                id: format!("evidence.source.actor.{family}.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(actor_sha256)),
                note: actor_note.into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.event.{family}.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(static_digest(event_sha256)),
                note: event_note.into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.name-map.{family}.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
                )),
                note: "d_stage.cpp: exact placement names map to their distinct actor process families and mini-boss level arguments.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.key-commit.{family}.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "b58ed135700865df0f0cb9ce0e4115de6ec1f9f6dbb8fff8cc1ff99b437d5569",
                )),
                note: "d_meter2.cpp: queued key deltas clamp to [0, 99], update dSv_memBit_c::mKeyNum, and clear the pending delta.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.save-layout.{family}.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "74a211e5d2ee2c0fe4ce259905fe1f479f373d5b2459d654871cbbd2f61e8756",
                )),
                note: "d_save.h: dSv_memBit_c memory switches, key count byte 0x1c, and dungeon-item byte 0x1d share the active stage bank.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.world.inventory.{family}.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(inventory_sha256),
                note: format!(
                    "Authenticated world inventory placement {} from resource {}.",
                    placement.stable_id, placement.source_sha256
                ),
            },
        ];
    if include_mboss_parameters {
        records.push(EvidenceRecord {
            id: format!("evidence.source.parameters.{family}.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "b0dacfc4b9c46786d73a840e55385e535364b9fee7de66cd0e2af18f25d1ca78",
            )),
            note: "d_door_param2.cpp: mini-boss-door front/back room, exit number, option, and unlock-switch parameter decoding.".into(),
        });
        records.sort_by(|left, right| left.id.cmp(&right.id));
    }
    RuleEvidence {
        truth: TruthStatus::Established,
        records,
    }
}

fn static_digest(value: &str) -> Digest {
    value.parse().expect("source-audit digest literal is valid")
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
    use crate::orig_discovery::bundled_supported_build_registry;
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

    fn audited_content() -> ContentIdentity {
        bundled_supported_build_registry()
            .unwrap()
            .identities
            .into_iter()
            .find(|identity| identity.id == "gcn-us-1.0-gz2e01")
            .unwrap()
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

    fn boss_door_inventory(room: i8) -> WorldInventory {
        let room_resource =
            static_digest("9336aabaee513b635d6d0d3db3f5f3b67f5c6bd6643581ebd1a8f7b779fa8e7a");
        let exit_id = "dzr-sha256:boss-room/chunk/SCLS/record/0".to_string();
        WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: "D_MN05".into(),
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
                        room: Some(room),
                    },
                    archive_sha256: static_digest(
                        "5b495a915c1539b92f57e84f7cbcf0b5662a8caeaf7ecf0503ac15af7a6e6a77",
                    ),
                    stage_data_path: "room.dzr".into(),
                    stage_data_sha256: room_resource,
                    kcl_path: None,
                    kcl_sha256: None,
                    plc_path: None,
                    plc_sha256: None,
                    addressable_prisms: 0,
                },
            ],
            chunks: Vec::new(),
            placements: vec![PlacementRecord {
                stable_id: "dzr-sha256:boss-room/chunk/ACTR/record/0".into(),
                source_sha256: room_resource,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(room),
                },
                chunk_tag: "ACTR".into(),
                record_index: 0,
                layer: None,
                kind: PlacementKind::ScaledActor,
                name: "L1Bdoor".into(),
                parameters: 0x0191_8000,
                position: Vec3 {
                    x: 7283.0,
                    y: 3302.0,
                    z: -16430.0,
                },
                angle: [-211, 0, 0x1717],
                set_id: 0xff,
                scale_raw: Some([10, 10, 10]),
                raw_hex: "4c3142646f6f72000191800045e39800454e6000c6805c00ff2d0000171700ff0a0a0aff"
                    .into(),
            }],
            player_spawns: Vec::new(),
            exits: vec![StageExitRecord {
                stable_id: exit_id,
                source_sha256: room_resource,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(room),
                },
                chunk_tag: "SCLS".into(),
                record_index: 0,
                destination_stage: "D_MN05A".into(),
                destination_point: 0,
                destination_room: 50,
                destination_layer: -1,
                wipe: 0,
                wipe_time: 0,
                time_hour: -1,
                raw_start: 0,
                raw_field_a: 0,
                raw_field_b: 0,
                raw_wipe: 0,
                raw_hex: "00".repeat(13),
            }],
            collisions: Vec::new(),
            load_triggers: Vec::new(),
        }
    }

    fn l5_boss_door_inventory(boss_room: bool) -> WorldInventory {
        let room = if boss_room { 50 } else { 4 };
        let mut inventory = boss_door_inventory(room);
        inventory.stage = if boss_room { "D_MN11A" } else { "D_MN11" }.into();
        let (archive_sha256, resource_sha256, parameters, position, destination) = if boss_room {
            (
                static_digest("4acd3b8ce5ac24820364314c1cbec9569bf0faad2d4f0e6688e974616d8c7889"),
                static_digest("106533086f77371b6abd4cfea2d0d2c14fd88f5ce1a2569bfc8020848d2519a6"),
                0x0390_8200,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 2100.0,
                },
                ("F_SP114", 1, 2, 11, 1),
            )
        } else {
            (
                static_digest("6ee1274731222f3abe62c50de686fbae60663ba11e911f79c54fe886e221cd55"),
                static_digest("cd32b1ac737b8cfe6f92fa35a18ba4e24a5ccbfe8b466ae403b6c269dcbfc5c3"),
                0x0590_8200,
                Vec3 {
                    x: 0.0,
                    y: 2109.0,
                    z: -5237.0,
                },
                ("D_MN11A", 50, -1, 1, 2),
            )
        };
        inventory.sources[1].archive_sha256 = archive_sha256;
        inventory.sources[1].stage_data_sha256 = resource_sha256;
        let placement = &mut inventory.placements[0];
        placement.stable_id = format!("dzr-sha256:l5-{room}/chunk/Door/record/0");
        placement.source_sha256 = resource_sha256;
        placement.chunk_tag = "Door".into();
        placement.name = "L5Bdoor".into();
        placement.parameters = parameters;
        placement.position = position;
        placement.angle = [-1, 0, -248];
        placement.raw_hex = if boss_room {
            "4c3542646f6f720003908200000000000000000045034000ffff0000ff0800ff0a0a0aff"
        } else {
            "4c3542646f6f720005908200000000004503d000c5a3a800ffff0000ff0800ff0a0a0aff"
        }
        .into();
        let exit = &mut inventory.exits[0];
        exit.stable_id = format!("dzr-sha256:l5-{room}/chunk/SCLS/record/{}", destination.4);
        exit.source_sha256 = resource_sha256;
        exit.record_index = destination.4;
        exit.destination_stage = destination.0.into();
        exit.destination_room = destination.1;
        exit.destination_layer = destination.2;
        exit.destination_point = destination.3;
        inventory
    }

    fn replace_room_actor(
        mut inventory: WorldInventory,
        stage: &str,
        room: i8,
        placement: PlacementRecord,
        keep_exit: bool,
    ) -> WorldInventory {
        inventory.stage = stage.into();
        inventory.sources[1].scope.room = Some(room);
        inventory.sources[1].stage_data_sha256 = placement.source_sha256;
        inventory.placements = vec![placement];
        if keep_exit {
            inventory.exits[0].scope.room = Some(room);
            inventory.exits[0].source_sha256 = inventory.sources[1].stage_data_sha256;
        } else {
            inventory.exits.clear();
        }
        inventory
    }

    fn keyed_mboss_inventory() -> WorldInventory {
        let source =
            static_digest("2756f041cd797b24e2794983d3c6e0b370aa1dd50a57a08a2e437585688a268c");
        let mut inventory = replace_room_actor(
            boss_door_inventory(7),
            "D_MN06",
            7,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/Door/record/0"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(7),
                },
                chunk_tag: "Door".into(),
                record_index: 0,
                layer: None,
                kind: PlacementKind::ScaledActor,
                name: "L6Mdoor".into(),
                parameters: 0x01b0_e600,
                position: Vec3 {
                    x: 1580.0,
                    y: 8250.0,
                    z: 700.0,
                },
                angle: [-1, 0x4000, -227],
                set_id: 0xff,
                scale_raw: Some([10, 10, 10]),
                raw_hex: "4c364d646f6f720001b0e60044c580004600e800442f0000ffff4000ff1d00ff0a0a0aff"
                    .into(),
            },
            true,
        );
        let exit = &mut inventory.exits[0];
        exit.stable_id = format!("dzr-sha256:{source}/chunk/SCLS/record/0");
        exit.destination_stage = "D_MN06B".into();
        exit.destination_room = 51;
        exit.destination_layer = -1;
        exit.destination_point = 0;
        inventory
    }

    fn regular_key_shutter_inventory() -> WorldInventory {
        let source =
            static_digest("f4a05b52105afd1dacb6b5b4b8e51706a922be24ea1b15fc198b47fd8aefb578");
        replace_room_actor(
            boss_door_inventory(9),
            "D_MN01",
            9,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/ACTR/record/4"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(9),
                },
                chunk_tag: "ACTR".into(),
                record_index: 4,
                layer: None,
                kind: PlacementKind::Actor,
                name: "kshtr00".into(),
                parameters: 0x80ff_0123,
                position: Vec3 {
                    x: 15185.0,
                    y: -50.0,
                    z: -570.0,
                },
                angle: [0, -5461, 255],
                set_id: 0xffff,
                scale_raw: None,
                raw_hex: "6b7368747230300080ff0123466d4400c2480000c40e80000000eaab00ffffff".into(),
            },
            false,
        )
    }

    fn lakebed_boss_key_shutter_inventory() -> WorldInventory {
        let source =
            static_digest("9336aabaee513b635d6d0d3db3f5f3b67f5c6bd6643581ebd1a8f7b779fa8e7a");
        replace_room_actor(
            boss_door_inventory(3),
            "D_MN01",
            3,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/Door/record/0"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(3),
                },
                chunk_tag: "Door".into(),
                record_index: 0,
                layer: None,
                kind: PlacementKind::ScaledActor,
                name: "L3Bdoor".into(),
                parameters: 0x80ff_0255,
                position: Vec3 {
                    x: 0.0,
                    y: -320.0,
                    z: 325.31067,
                },
                angle: [0, 0, 0],
                set_id: 0xff,
                scale_raw: Some([10, 10, 10]),
                raw_hex: "4c3342646f6f720080ff025500000000c3a0000043a2a7c400000000000000ff0a0a0aff"
                    .into(),
            },
            false,
        )
    }

    fn koki_gate_inventory(switch_id: u8) -> WorldInventory {
        let source =
            static_digest("5c2208b4088c8ac55dabca200f7bd7eedac3cf2c93364eb49fcfae2216513e21");
        let parameters = 0x0ff0_ff00 | u32::from(switch_id);
        let mut raw = "4b5f4761746500000ff0ff0cc688e00043480000c62eec00000026660000ffff".to_owned();
        raw.replace_range(22..24, &format!("{switch_id:02x}"));
        replace_room_actor(
            boss_door_inventory(3),
            "F_SP108",
            3,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/ACT0/record/36"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(3),
                },
                chunk_tag: "ACT0".into(),
                record_index: 36,
                layer: Some(0),
                kind: PlacementKind::Actor,
                name: "K_Gate".into(),
                parameters,
                position: Vec3 {
                    x: -17520.0,
                    y: 200.0,
                    z: -11195.0,
                },
                angle: [0, 9830, 0],
                set_id: 0xffff,
                scale_raw: None,
                raw_hex: raw,
            },
            false,
        )
    }

    fn has_small_key_comparison(
        transition: &CandidateTransition,
        expected_operator: ComparisonOperator,
        expected_value: u64,
    ) -> bool {
        let PredicateExpression::All { terms } = &transition.activation.hard_guards else {
            return false;
        };
        terms.iter().any(|term| {
            matches!(
                term,
                PredicateExpression::Compare {
                    left: ValueReference::BoundRawBits {
                        byte_offset: 0x1c,
                        byte_width: 1,
                        mask: 0xff,
                        ..
                    },
                    operator,
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(value),
                    },
                } if *operator == expected_operator && *value == expected_value
            )
        })
    }

    fn writes_small_key(transition: &CandidateTransition, expected_value: u8) -> bool {
        transition.activation.effects.iter().any(|effect| {
            matches!(
                effect,
                StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x1c,
                    mask,
                    value,
                } if mask == &[0xff] && value == &[expected_value]
            )
        })
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
    fn imports_audited_gz2e01_boss_door_guard_write_and_destination() {
        assert_eq!(memory_switch_raw_location(0x00), (0x0b, 0x01));
        assert_eq!(memory_switch_raw_location(0x17), (0x09, 0x80));
        assert_eq!(memory_switch_raw_location(0x1f), (0x08, 0x80));
        assert_eq!(memory_switch_raw_location(0x20), (0x0f, 0x01));
        assert_eq!(memory_switch_raw_location(0x7f), (0x14, 0x80));
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = boss_door_inventory(12);
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 1);
        assert_eq!(facts.mechanics.obligations.len(), 2);
        assert_eq!(facts.encoded_exits[0].candidate_transition_ids.len(), 1);
        let transition = &facts.mechanics.transitions[0];
        assert_eq!(transition.transition_kind, TransitionKind::Door);
        assert_eq!(transition.evidence.truth, TruthStatus::Established);
        assert_eq!(transition.activation.physical_obligation_ids.len(), 2);
        let PredicateExpression::All { terms } = &transition.activation.hard_guards else {
            panic!("boss door must retain source location and boss-key guards")
        };
        assert!(terms.iter().any(|term| {
            matches!(
                term,
                PredicateExpression::Compare {
                    left: ValueReference::BoundRawBits {
                        component_kind: ComponentKind::DungeonMemory,
                        binding: ComponentBindingReference::CurrentStage,
                        byte_offset: 0x1d,
                        byte_width: 1,
                        mask: 0x04,
                    },
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(0x04),
                    },
                }
            )
        }));
        assert!(matches!(
            transition.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x09,
                    mask,
                    value,
                },
                StateOperation::SetLocation { location },
            ] if mask == &[0x80]
                && value == &[0x80]
                && location.stage == "D_MN05A"
                && location.room == 50
                && location.spawn == 0
        ));
    }

    #[test]
    fn does_not_generalize_boss_door_source_semantics_or_reverse_side() {
        let inventory = boss_door_inventory(12);
        let mut unaudited_content = audited_content();
        unaudited_content.fingerprint.executable_sha256 = Digest([0x55; 32]);
        let unaudited_runtime = runtime(&unaudited_content);
        let context = world_context(unaudited_content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &unaudited_content,
            &unaudited_runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();
        assert!(facts.mechanics.transitions.is_empty());
        assert!(facts.encoded_exits[0].candidate_transition_ids.is_empty());

        let content = audited_content();
        let runtime = runtime(&content);
        let reverse_inventory = boss_door_inventory(50);
        let context = world_context(content.fingerprint.game_data_sha256, &reverse_inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&reverse_inventory),
        )
        .unwrap();
        assert!(facts.mechanics.transitions.is_empty());
        assert!(facts.encoded_exits[0].candidate_transition_ids.is_empty());
    }

    #[test]
    fn imports_l5_human_guard_and_distinguishes_dungeon_from_boss_room_unlock() {
        let content = audited_content();
        let runtime = runtime(&content);
        let dungeon_inventory = l5_boss_door_inventory(false);
        let context = world_context(content.fingerprint.game_data_sha256, &dungeon_inventory);
        let dungeon = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&dungeon_inventory),
        )
        .unwrap();
        let transition = &dungeon.mechanics.transitions[0];
        let PredicateExpression::All { terms } = &transition.activation.hard_guards else {
            panic!("L5 boss door must retain location, boss-key, and form guards")
        };
        assert!(terms.iter().any(|term| {
            matches!(
                term,
                PredicateExpression::Compare {
                    left: ValueReference::PlayerForm,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text(form),
                    },
                } if form == "human"
            )
        }));
        assert!(matches!(
            transition.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x0a,
                    mask,
                    value,
                },
                StateOperation::SetLocation { location },
            ] if mask == &[0x01]
                && value == &[0x01]
                && location.stage == "D_MN11A"
                && location.room == 50
                && location.spawn == 1
        ));
        assert!(transition.evidence.records.iter().any(|record| {
            record.source_sha256
                == Some(static_digest(
                    "9f649b99f027e39f1d39ce066d815a78032b536c4a9a83e0361681af2265102e",
                ))
        }));

        let boss_room_inventory = l5_boss_door_inventory(true);
        let context = world_context(content.fingerprint.game_data_sha256, &boss_room_inventory);
        let boss_room = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&boss_room_inventory),
        )
        .unwrap();
        assert!(matches!(
            boss_room.mechanics.transitions[0]
                .activation
                .effects
                .as_slice(),
            [StateOperation::SetLocation { location }]
                if location.stage == "F_SP114"
                    && location.room == 1
                    && location.layer == 2
                    && location.spawn == 11
        ));
    }

    #[test]
    fn imports_keyed_mboss_first_open_and_reopen_as_distinct_branches() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = keyed_mboss_inventory();
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.schema, EXTRACTED_WORLD_FACTS_SCHEMA);
        assert_eq!(facts.mechanics.transitions.len(), 3);
        assert_eq!(facts.mechanics.obligations.len(), 2);
        assert_eq!(facts.encoded_exits[0].candidate_transition_ids.len(), 3);
        let first_open =
            facts
                .mechanics
                .transitions
                .iter()
                .find(|transition| {
                    transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
                })
                .expect("first keyed opening branch");
        assert_eq!(first_open.transition_kind, TransitionKind::Door);
        assert!(matches!(
            first_open.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x08,
                    mask,
                    value,
                },
                StateOperation::AdjustBoundRawUnsigned {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x1c,
                    byte_width: 1,
                    delta: -1,
                },
                StateOperation::SetLocation { location },
            ] if mask == &[0x20]
                && value == &[0x20]
                && location.stage == "D_MN06B"
                && location.room == 51
        ));
        let PredicateExpression::All { terms } = &first_open.activation.hard_guards else {
            panic!("first opening must retain location, switch, and key guards")
        };
        assert!(terms.iter().any(|term| matches!(
            term,
            PredicateExpression::Compare {
                left: ValueReference::BoundRawBits {
                    byte_offset: 0x1c,
                    byte_width: 1,
                    mask: 0xff,
                    ..
                },
                operator: ComparisonOperator::GreaterThan,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(0),
                },
            }
        )));
        assert!(has_small_key_comparison(
            first_open,
            ComparisonOperator::LessThanOrEqual,
            100,
        ));
        let reopen = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.activation.effects.len() == 1)
            .expect("already-unlocked reopening branch");
        assert!(matches!(
            reopen.activation.effects.as_slice(),
            [StateOperation::SetLocation { location }]
                if location.stage == "D_MN06B" && location.room == 51
        ));
        let high = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| writes_small_key(transition, 99))
            .expect("high raw key-count clamp branch");
        assert!(has_small_key_comparison(
            high,
            ComparisonOperator::GreaterThan,
            100,
        ));
    }

    #[test]
    fn imports_regular_key_shutter_switch_and_small_key_mutation() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = regular_key_shutter_inventory();
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 2);
        assert_eq!(facts.mechanics.obligations.len(), 2);
        let transition =
            facts
                .mechanics
                .transitions
                .iter()
                .find(|transition| {
                    transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
                })
                .expect("ordinary key decrement branch");
        assert_eq!(transition.transition_kind, TransitionKind::ActorDriven);
        assert!(matches!(
            transition.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    byte_offset: 0x0f,
                    mask,
                    value,
                    ..
                },
                StateOperation::AdjustBoundRawUnsigned {
                    byte_offset: 0x1c,
                    byte_width: 1,
                    delta: -1,
                    ..
                },
            ] if mask == &[0x08] && value == &[0x08]
        ));
        assert!(has_small_key_comparison(
            transition,
            ComparisonOperator::LessThanOrEqual,
            100,
        ));
        assert!(transition.evidence.records.iter().any(|record| {
            record.source_sha256
                == Some(static_digest(
                    "3bff3ce52a0c1660d5ccf0bdcae24b672e50013317b3469698c51e32336c159a",
                ))
        }));
        let high = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| writes_small_key(transition, 99))
            .expect("high raw key-count clamp branch");
        assert!(has_small_key_comparison(
            high,
            ComparisonOperator::GreaterThan,
            100,
        ));
    }

    #[test]
    fn imports_lakebed_boss_shutter_zero_normal_and_high_small_key_outcomes() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = lakebed_boss_key_shutter_inventory();
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 3);
        assert_eq!(facts.mechanics.obligations.len(), 2);
        let positive =
            facts
                .mechanics
                .transitions
                .iter()
                .find(|transition| {
                    transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
                })
                .expect("boss-key opening with a small key");
        assert!(matches!(
            positive.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    byte_offset: 0x11,
                    mask,
                    value,
                    ..
                },
                StateOperation::AdjustBoundRawUnsigned {
                    byte_offset: 0x1c,
                    byte_width: 1,
                    delta: -1,
                    ..
                },
            ] if mask == &[0x20] && value == &[0x20]
        ));
        assert!(has_small_key_comparison(
            positive,
            ComparisonOperator::LessThanOrEqual,
            100,
        ));
        let zero = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.activation.effects.len() == 1)
            .expect("boss-key opening with zero small keys");
        let PredicateExpression::All { terms } = &zero.activation.hard_guards else {
            panic!("zero-key branch must retain boss-key and zero-key guards")
        };
        assert!(terms.iter().any(|term| matches!(
            term,
            PredicateExpression::Compare {
                left: ValueReference::BoundRawBits {
                    byte_offset: 0x1d,
                    mask: 0x04,
                    ..
                },
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(0x04),
                },
                ..
            }
        )));
        let high = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| writes_small_key(transition, 99))
            .expect("high raw key-count clamp branch");
        assert!(has_small_key_comparison(
            high,
            ComparisonOperator::GreaterThan,
            100,
        ));
        assert!(terms.iter().any(|term| matches!(
            term,
            PredicateExpression::Compare {
                left: ValueReference::BoundRawBits {
                    byte_offset: 0x1c,
                    mask: 0xff,
                    ..
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(0),
                },
            }
        )));
    }

    #[test]
    fn imports_only_memory_switch_backed_type_zero_koki_gate_unlocks() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = koki_gate_inventory(0x0c);
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 2);
        let ordinary =
            facts
                .mechanics
                .transitions
                .iter()
                .find(|transition| {
                    transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
                })
                .expect("ordinary key decrement branch");
        assert!(matches!(
            ordinary.activation.effects.as_slice(),
            [
                StateOperation::AdjustBoundRawUnsigned {
                    byte_offset: 0x1c,
                    byte_width: 1,
                    delta: -1,
                    ..
                },
                StateOperation::WriteBoundRaw {
                    byte_offset: 0x0a,
                    mask,
                    value,
                    ..
                },
            ] if mask == &[0x10] && value == &[0x10]
        ));
        assert!(has_small_key_comparison(
            ordinary,
            ComparisonOperator::LessThanOrEqual,
            100,
        ));
        let PredicateExpression::All { terms } = &ordinary.activation.hard_guards else {
            panic!("gate must retain location, switch, and key guards")
        };
        assert!(terms.iter().any(|term| matches!(
            term,
            PredicateExpression::All { terms }
                if terms.iter().any(|nested| matches!(
                    nested,
                    PredicateExpression::Compare {
                        left: ValueReference::LocationLayer,
                        right: ValueReference::Literal {
                            value: StateValue::Signed(0),
                        },
                        ..
                    }
                ))
        )));
        let high = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| writes_small_key(transition, 99))
            .expect("high raw key-count clamp branch");
        assert!(has_small_key_comparison(
            high,
            ComparisonOperator::GreaterThan,
            100,
        ));

        let absent_switch_inventory = koki_gate_inventory(0xff);
        absent_switch_inventory.validate().unwrap();
        let context = world_context(
            content.fingerprint.game_data_sha256,
            &absent_switch_inventory,
        );
        let absent_switch = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&absent_switch_inventory),
        )
        .unwrap();
        assert!(absent_switch.mechanics.transitions.is_empty());
        assert!(absent_switch.mechanics.obligations.is_empty());
    }

    #[test]
    fn imports_wrapped_type_zero_shutter_and_each_mboss_event_resource() {
        let scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([0x11; 32]),
                    runtime_configuration_sha256: Digest([0x22; 32]),
                },
            }],
        };
        let inventory = regular_key_shutter_inventory();
        let mut shutter = inventory.placements[0].clone();
        shutter.parameters = 0x80ff_ff2b;
        let imported =
            import_gz2e01_keyed_actor_actions(&inventory, &shutter, &scope, Digest([0x33; 32]))
                .unwrap()
                .expect("authored 0xff wraps to the supported runtime type zero");
        assert_eq!(imported.transitions.len(), 2);
        assert!(
            imported.transitions[0]
                .evidence
                .records
                .iter()
                .any(|record| {
                    record.source_sha256
                        == Some(static_digest(
                            "8676effbd561ba65f8e4a8b9493aa6b60072d40f72a8e240b2ffa9c5550b40fa",
                        ))
                })
        );

        let inventory = keyed_mboss_inventory();
        for (name, event_sha256) in [
            (
                "L7door",
                "7de6bfac10e3ca6c3f6bc88a83815972d3397fd3488b067398cdd8cb0ea0cce4",
            ),
            (
                "L8Mdoor",
                "b079b8b284208582d9a37b50bd94f13400530abca75db0771147a646a8d83627",
            ),
        ] {
            let mut placement = inventory.placements[0].clone();
            placement.name = name.into();
            let imported = import_gz2e01_keyed_actor_actions(
                &inventory,
                &placement,
                &scope,
                Digest([0x33; 32]),
            )
            .unwrap()
            .expect("audited keyed mini-boss alias");
            assert_eq!(imported.transitions.len(), 3);
            assert!(
                imported.transitions[0]
                    .evidence
                    .records
                    .iter()
                    .any(|record| record.source_sha256 == Some(static_digest(event_sha256)))
            );
        }
    }

    #[test]
    fn excludes_unaudited_keyed_family_bypasses_and_non_memory_switches() {
        let scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([0x11; 32]),
                    runtime_configuration_sha256: Digest([0x22; 32]),
                },
            }],
        };
        let inventory = regular_key_shutter_inventory();
        let mut placement = inventory.placements[0].clone();
        for name in ["R_Gate", "CrvGate", "vshuter"] {
            placement.name = name.into();
            assert!(
                import_gz2e01_keyed_actor_actions(
                    &inventory,
                    &placement,
                    &scope,
                    Digest([0x33; 32]),
                )
                .unwrap()
                .is_none()
            );
        }

        placement = inventory.placements[0].clone();
        placement.parameters &= 0x7fff_ffff;
        assert!(
            import_gz2e01_keyed_actor_actions(&inventory, &placement, &scope, Digest([0x33; 32]),)
                .unwrap()
                .is_none()
        );

        let gate_inventory = koki_gate_inventory(0x82);
        let mut gate = gate_inventory.placements[0].clone();
        assert!(
            import_gz2e01_keyed_actor_actions(&gate_inventory, &gate, &scope, Digest([0x33; 32]),)
                .unwrap()
                .is_none()
        );
        gate.parameters = (gate.parameters & !(0x0f << 16)) | (1 << 16);
        assert!(
            import_gz2e01_keyed_actor_actions(&gate_inventory, &gate, &scope, Digest([0x33; 32]),)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn encoded_exits_reference_only_map_and_door_transitions() {
        let content = audited_content();
        let runtime = runtime(&content);

        let gate_inventory = koki_gate_inventory(0x0c);
        let context = world_context(content.fingerprint.game_data_sha256, &gate_inventory);
        let mut actor_facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&gate_inventory),
        )
        .unwrap();
        actor_facts.mechanics.transitions[0].transition_kind = TransitionKind::Door;
        assert_eq!(
            actor_facts.validate().unwrap_err().field(),
            "mechanics.transitions"
        );

        let door_inventory = keyed_mboss_inventory();
        let context = world_context(content.fingerprint.game_data_sha256, &door_inventory);
        let mut door_facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&door_inventory),
        )
        .unwrap();
        door_facts.mechanics.transitions[0].transition_kind = TransitionKind::ActorDriven;
        assert_eq!(
            door_facts.validate().unwrap_err().field(),
            "mechanics.transitions"
        );
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
