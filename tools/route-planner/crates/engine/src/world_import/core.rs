//! Build and validate the complete extracted world-facts artifact.

use super::*;

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

        let stages = world_context
            .stages
            .iter()
            .map(|stage| {
                let inventory = inventory_by_stage
                    .get(stage.stage.as_str())
                    .copied()
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
                Ok(WorldImportStage {
                    inventory,
                    inventory_sha256,
                    spatial_index_sha256: Some(stage.spatial_index_sha256),
                })
            })
            .collect::<Result<Vec<_>, PlannerContractError>>()?;
        Self::build_validated(
            content,
            runtime_configuration,
            Some(
                world_context
                    .digest()
                    .map_err(|error| world_error("world_context", error))?,
            ),
            None,
            stages,
            Vec::new(),
        )
    }

    fn build_validated(
        content: &ContentIdentity,
        runtime_configuration: &RuntimeConfiguration,
        world_context_sha256: Option<Digest>,
        native_inventory_set_sha256: Option<Digest>,
        stages: Vec<WorldImportStage<'_>>,
        native_stage_metadata: Vec<NativeStageMetadata>,
    ) -> Result<Self, PlannerContractError> {
        let content_sha256 = content.digest()?;
        let exact_context = ExactContext {
            content_sha256,
            runtime_configuration_sha256: runtime_configuration.digest()?,
        };
        let scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: exact_context.clone(),
            }],
        };
        let mut sources = Vec::with_capacity(stages.len());
        let mut static_world_objects = Vec::new();
        let mut spatial_volumes = Vec::new();
        let mut spatial_planes = Vec::new();
        let mut spawns = Vec::new();
        let mut encoded_exits = Vec::new();
        let mut approach_geometries = Vec::new();
        let mut transitions = Vec::new();
        let mut obligations = Vec::new();

        for stage in stages {
            let inventory = stage.inventory;
            let inventory_sha256 = stage.inventory_sha256;
            sources.push(WorldInventoryFactSource {
                stage: inventory.stage.clone(),
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
                let collision = inventory
                    .collisions
                    .iter()
                    .find(|collision| collision.prism.authored.stable_id == trigger.collision_id)
                    .expect("validated load trigger references a collision");
                let mut candidate_spawn_ids = inventory
                    .player_spawns
                    .iter()
                    .filter(|spawn| spawn.scope.room == Some(trigger.room))
                    .map(|spawn| {
                        stable_token(
                            "world.spawn",
                            &[inventory.stage.as_bytes(), spawn.stable_id.as_bytes()],
                        )
                    })
                    .collect::<Vec<_>>();
                candidate_spawn_ids.sort();
                let shape = match &collision.prism.reconstruction {
                    KclReconstruction::Reconstructed { plane, triangle } => {
                        let triangle = triangle
                            .map(|point| canonicalize_position([point.x, point.y, point.z]));
                        let (minimum, maximum) = triangle_bounds(&triangle);
                        ExtractedApproachShape::Reconstructed {
                            triangle,
                            plane_normal: canonicalize_position([
                                plane.normal.x,
                                plane.normal.y,
                                plane.normal.z,
                            ]),
                            plane_offset: canonicalize_scalar(plane.d),
                            minimum,
                            maximum,
                        }
                    }
                    KclReconstruction::Degenerate { reason } => {
                        ExtractedApproachShape::Unavailable {
                            reason: reason.clone(),
                        }
                    }
                };
                approach_geometries.push(ExtractedApproachGeometry {
                    id: format!("approach-geometry.{token}"),
                    transition_id: transition_id.clone(),
                    approach_id: approach_id.clone(),
                    source_stage: inventory.stage.clone(),
                    source_room: trigger.room,
                    source_collision_id: trigger.collision_id.clone(),
                    source_inventory_sha256: inventory_sha256,
                    candidate_spawn_ids,
                    shape,
                });
                obligations.push(FeasibilityObligation {
                    id: obligation_id.clone(),
                    label: format!(
                        "Reach collision exit {} in {} room {}",
                        trigger.collision_exit_id, inventory.stage, trigger.room
                    ),
                    scope: scope.clone(),
                    obligation_kind: ObligationKind::Geometry,
                    stage: crate::transition::ObligationStage::Reach,
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
                    spatial_volumes.extend(imported.spatial_volumes);
                    spatial_planes.extend(imported.spatial_planes);
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
                for placement in &inventory.placements {
                    let Some(imported) = import_gz2e01_l7_bridge_demo(
                        inventory,
                        placement,
                        &scope,
                        inventory_sha256,
                    )?
                    else {
                        continue;
                    };
                    for (exit_record_id, transition) in imported.transitions {
                        transition_ids_by_exit
                            .entry(exit_record_id)
                            .or_default()
                            .push(transition.id.clone());
                        transitions.push(transition);
                    }
                    obligations.extend(imported.obligations);
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
        spatial_volumes.sort_by(|left, right| {
            (&left.object_id, &left.volume_id).cmp(&(&right.object_id, &right.volume_id))
        });
        spatial_planes.sort_by(|left, right| left.plane_id.cmp(&right.plane_id));
        spawns.sort_by(|left, right| left.id.cmp(&right.id));
        encoded_exits.sort_by(|left, right| left.id.cmp(&right.id));
        approach_geometries.sort_by(|left, right| left.id.cmp(&right.id));
        obligations.sort_by(|left, right| left.id.cmp(&right.id));
        transitions.sort_by(|left, right| left.id.cmp(&right.id));
        let facts = Self {
            schema: EXTRACTED_WORLD_FACTS_SCHEMA.into(),
            exact_context,
            world_context_sha256,
            native_inventory_set_sha256,
            inventories: sources,
            native_stage_metadata,
            static_world_objects,
            spatial_volumes,
            spatial_planes,
            spawns,
            encoded_exits,
            approach_geometries,
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

    /// Imports planner-native stage records without manufacturing a compatible
    /// world-context or spatial-index identity. Collision-backed transitions
    /// remain absent because the v4 native inventory set marks that domain
    /// unavailable; placement/SCLS-backed actor rules still import normally.
    pub fn build_from_orig_world_inventories(
        content: &ContentIdentity,
        runtime_configuration: &RuntimeConfiguration,
        native: &ExtractedOrigWorldInventories,
    ) -> Result<Self, PlannerContractError> {
        content.validate()?;
        runtime_configuration.validate()?;
        native.validate()?;
        let content_sha256 = content.digest()?;
        if native.content_sha256 != content_sha256
            || native.game_data_sha256 != content.fingerprint.game_data_sha256
        {
            return Err(PlannerContractError::new(
                "native_world.identity",
                "does not match the supplied content identity",
            ));
        }
        if runtime_configuration.content_sha256 != content_sha256 {
            return Err(PlannerContractError::new(
                "runtime_configuration.content_sha256",
                "does not name the supplied content identity",
            ));
        }
        let native_sha256 = native.digest()?;
        let stages = native
            .inventories
            .iter()
            .map(|inventory| {
                Ok(WorldImportStage {
                    inventory,
                    inventory_sha256: inventory.digest()?,
                    spatial_index_sha256: None,
                })
            })
            .collect::<Result<Vec<_>, PlannerContractError>>()?;
        Self::build_validated(
            content,
            runtime_configuration,
            None,
            Some(native_sha256),
            stages,
            native.stage_metadata.clone(),
        )
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != EXTRACTED_WORLD_FACTS_SCHEMA {
            return Err(PlannerContractError::new(
                "extracted_world_facts",
                "has an unsupported schema",
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
        let compatible_provenance = matches!(
            (self.world_context_sha256, self.native_inventory_set_sha256),
            (Some(context), None) if context != Digest::ZERO
        );
        let native_provenance = matches!(
            (self.world_context_sha256, self.native_inventory_set_sha256),
            (None, Some(native)) if native != Digest::ZERO
        );
        if !compatible_provenance && !native_provenance {
            return Err(PlannerContractError::new(
                "extracted_world_facts.provenance",
                "must name exactly one nonzero world context or native inventory set",
            ));
        }
        if compatible_provenance && !self.native_stage_metadata.is_empty()
            || native_provenance && self.native_stage_metadata.len() != self.inventories.len()
        {
            return Err(PlannerContractError::new(
                "native_stage_metadata",
                "must be empty for compatible provenance and complete for planner-native provenance",
            ));
        }
        for (source, metadata) in self.inventories.iter().zip(&self.native_stage_metadata) {
            if source.stage != metadata.stage {
                return Err(PlannerContractError::new(
                    "native_stage_metadata.stage",
                    "does not match its inventory fact source",
                ));
            }
            metadata.validate_records()?;
        }
        for source in &self.inventories {
            validate_game_name("inventories.stage", &source.stage)?;
            if source.inventory_sha256 == Digest::ZERO
                || compatible_provenance
                    && !matches!(source.spatial_index_sha256, Some(digest) if digest != Digest::ZERO)
                || native_provenance && source.spatial_index_sha256.is_some()
            {
                return Err(PlannerContractError::new(
                    "inventories",
                    "does not match its compatible or planner-native spatial provenance",
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
        if self.spatial_volumes.windows(2).any(|pair| {
            (&pair[0].object_id, &pair[0].volume_id) >= (&pair[1].object_id, &pair[1].volume_id)
        }) {
            return Err(PlannerContractError::new(
                "spatial_volumes",
                "must be unique and sorted by object and volume ID",
            ));
        }
        for volume in &self.spatial_volumes {
            validate_spatial_volume(volume)?;
            if !self
                .static_world_objects
                .iter()
                .any(|object| object.id == volume.object_id)
            {
                return Err(PlannerContractError::new(
                    "spatial_volumes.object_id",
                    "does not reference an imported static object",
                ));
            }
        }
        validate_sorted("spatial_planes", &self.spatial_planes, |value| {
            value.plane_id.as_str()
        })?;
        for plane in &self.spatial_planes {
            validate_spatial_plane(plane)?;
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
        if native_provenance
            && (!self.approach_geometries.is_empty()
                || self
                    .mechanics
                    .transitions
                    .iter()
                    .any(|transition| transition.transition_kind == TransitionKind::EncodedMapExit))
        {
            return Err(PlannerContractError::new(
                "extracted_world_facts.native_collision",
                "native inventory-set provenance cannot contain unavailable collision approaches or encoded-map transitions",
            ));
        }
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
        validate_sorted("approach_geometries", &self.approach_geometries, |value| {
            value.id.as_str()
        })?;
        let spawn_ids = self
            .spawns
            .iter()
            .map(|spawn| spawn.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut approach_transition_ids = BTreeSet::new();
        for geometry in &self.approach_geometries {
            validate_stable_id("approach_geometries.id", &geometry.id)?;
            validate_stable_id("approach_geometries.transition_id", &geometry.transition_id)?;
            validate_stable_id("approach_geometries.approach_id", &geometry.approach_id)?;
            validate_game_name("approach_geometries.source_stage", &geometry.source_stage)?;
            if geometry.source_collision_id.is_empty()
                || geometry.source_collision_id.len() > 2048
                || geometry.source_inventory_sha256 == Digest::ZERO
                || !strictly_sorted(&geometry.candidate_spawn_ids)
                || geometry
                    .candidate_spawn_ids
                    .iter()
                    .any(|id| !spawn_ids.contains(id.as_str()))
            {
                return Err(PlannerContractError::new(
                    "approach_geometries",
                    "contains an invalid source or spawn reference",
                ));
            }
            if geometry.candidate_spawn_ids.iter().any(|id| {
                self.spawns
                    .iter()
                    .find(|spawn| spawn.id == *id)
                    .is_none_or(|spawn| {
                        spawn.location.stage != geometry.source_stage
                            || spawn.location.room != geometry.source_room
                    })
            }) {
                return Err(PlannerContractError::new(
                    "approach_geometries.candidate_spawn_ids",
                    "must remain in the geometry's exact source stage and room",
                ));
            }
            let Some(transition) = self
                .mechanics
                .transitions
                .iter()
                .find(|transition| transition.id == geometry.transition_id)
            else {
                return Err(PlannerContractError::new(
                    "approach_geometries.transition_id",
                    "references an unknown transition",
                ));
            };
            if transition.transition_kind != TransitionKind::EncodedMapExit
                || transition.approach_id != geometry.approach_id
                || !approach_transition_ids.insert(geometry.transition_id.as_str())
            {
                return Err(PlannerContractError::new(
                    "approach_geometries.transition_id",
                    "must uniquely reference its encoded-map transition and exact approach",
                ));
            }
            validate_approach_shape(&geometry.shape)?;
        }
        let encoded_map_transition_ids = self
            .mechanics
            .transitions
            .iter()
            .filter(|transition| transition.transition_kind == TransitionKind::EncodedMapExit)
            .map(|transition| transition.id.as_str())
            .collect::<BTreeSet<_>>();
        if approach_transition_ids != encoded_map_transition_ids {
            return Err(PlannerContractError::new(
                "approach_geometries",
                "must cover every collision-derived encoded-map transition exactly once",
            ));
        }
        let exit_transition_ids = self
            .mechanics
            .transitions
            .iter()
            .filter(|transition| {
                transition
                    .activation
                    .effects
                    .iter()
                    .any(|effect| matches!(effect, StateOperation::SetLocation { .. }))
            })
            .map(|transition| transition.id.as_str())
            .collect::<BTreeSet<_>>();
        if self.mechanics.transitions.iter().any(|transition| {
            matches!(
                transition.transition_kind,
                TransitionKind::EncodedMapExit | TransitionKind::Door
            ) && !exit_transition_ids.contains(transition.id.as_str())
        }) {
            return Err(PlannerContractError::new(
                "mechanics.transitions",
                "encoded-map and door transitions must contain an encoded location change",
            ));
        }
        if referenced_transition_ids != exit_transition_ids {
            return Err(PlannerContractError::new(
                "mechanics.transitions",
                "every location-changing transition must be referenced exactly once by its encoded exit, while transitions without a location change must not be referenced by one",
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
            + self
                .native_stage_metadata
                .iter()
                .map(|metadata| {
                    metadata.room_transforms.len()
                        + metadata.file_lists.len()
                        + metadata.room_reads.len()
                        + metadata.cameras.len()
                        + metadata.camera_arrows.len()
                        + metadata.paths.len()
                        + metadata.path_points.len()
                })
                .sum::<usize>()
            + self.spatial_volumes.len()
            + self.spatial_planes.len()
            + self.spawns.len()
            + self.encoded_exits.len()
            + self.approach_geometries.len()
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
