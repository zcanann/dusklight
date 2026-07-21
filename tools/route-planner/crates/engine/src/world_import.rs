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

struct ImportedBossDoor {
    exit_record_id: String,
    transition: CandidateTransition,
    obligations: Vec<FeasibilityObligation>,
}

fn import_gz2e01_boss_door(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedBossDoor>, PlannerContractError> {
    if placement.kind != PlacementKind::Actor || !is_l1_boss_door_name(&placement.name) {
        return Ok(None);
    }
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    if (inventory.stage == "D_MN08A" && room == 10) || (inventory.stage != "D_MN08A" && room == 50)
    {
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
    let token = stable_token(
        "world.gz2e01.l1-boss-door",
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
    let evidence = gz2e01_boss_door_evidence(inventory_sha256, placement, &token);
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
    let destination = SceneLocation {
        stage: exit.destination_stage.clone(),
        room: exit.destination_room,
        layer: exit.destination_layer,
        spawn: exit.destination_point,
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
                    terms: vec![source_location_guard(&inventory.stage, room), boss_key_guard],
                },
                physical_obligation_ids: vec![
                    actor_obligation_id.clone(),
                    interaction_obligation_id.clone(),
                ],
                effects: vec![
                    StateOperation::WriteBoundRaw {
                        component_kind: ComponentKind::DungeonMemory,
                        binding: ComponentBindingReference::CurrentStage,
                        byte_offset: switch_byte_offset,
                        mask: vec![switch_mask],
                        value: vec![switch_mask],
                    },
                    StateOperation::SetLocation {
                        location: destination,
                    },
                ],
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
                    research_question: "Confirm the loaded boss-door resources and actor/event phases reach INIT, UNLOCK, collision release, CHG_SCENE, and restart handling without an intervening failure or interruption.".into(),
                },
                evidence: unknown_evidence.clone(),
            },
            FeasibilityObligation {
                id: interaction_obligation_id,
                label: format!("Reach and face {} from its usable side", placement.name),
                scope: scope.clone(),
                obligation_kind: ObligationKind::Interaction,
                detail: ObligationDetail::Unresolved {
                    research_question: "Derive the actor-local interaction volume and facing test: |x| <= 200, |z| <= 100, wolf attention/current-position constraints, and facing delta <= 0x4000, then connect them to extracted oriented geometry.".into(),
                },
                evidence: unknown_evidence,
            },
        ],
    }))
}

fn is_l1_boss_door_name(name: &str) -> bool {
    matches!(
        name,
        "L1Bdoor" | "L2Bdoor" | "L4Bdoor" | "L6Bdoor" | "L7Bdoor" | "L8Bdoor" | "L9Bdoor"
    )
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
    inventory_sha256: Digest,
    placement: &PlacementRecord,
    token: &str,
) -> RuleEvidence {
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: format!("evidence.source.actor.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "221c170e034cf90cc43b20dc737bebeb44d6f8b54111d4454024f2fea7069d79",
                )),
                note: "d_a_door_bossL1.cpp: boss-key/front/area offer guards; unlock switch, event phases, collision release, and scene-change behavior.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.name-map.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
                )),
                note: "d_stage.cpp: L1/L2/L4/L6/L7/L8/L9 boss-door names map to fpcNm_L1BOSS_DOOR_e.".into(),
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
        ContentIdentity {
            schema: CONTENT_IDENTITY_SCHEMA.into(),
            id: "gcn-us-1.0-gz2e01".into(),
            fingerprint: ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "1.0".into(),
                product_id: "GZ2E01".into(),
                executable_sha256: static_digest(
                    "e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8",
                ),
                game_data_sha256: static_digest(
                    "0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772",
                ),
                resource_manifest_sha256: static_digest(
                    "2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1",
                ),
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
                kind: PlacementKind::Actor,
                name: "L1Bdoor".into(),
                parameters: 0x0191_8000,
                position: Vec3 {
                    x: 7283.0,
                    y: 3302.0,
                    z: -16430.0,
                },
                angle: [-211, 0, 0x1717],
                set_id: 0xff,
                scale_raw: None,
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
