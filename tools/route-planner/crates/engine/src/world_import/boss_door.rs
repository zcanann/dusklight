//! Import source-audited GZ2E01 boss-door semantics and evidence.

use super::*;

pub(super) struct ImportedBossDoor {
    pub(super) exit_record_id: String,
    pub(super) transition: CandidateTransition,
    pub(super) obligations: Vec<FeasibilityObligation>,
    pub(super) spatial_volumes: Vec<SpatialVolume>,
    pub(super) spatial_planes: Vec<SpatialPlane>,
}

#[derive(Clone, Copy)]
pub(super) enum Gz2e01BossDoorFamily {
    L1,
    L5,
}

pub(super) fn import_gz2e01_boss_door(
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
    let front_obligation_id = format!("obligation.front-side.{token}");
    let facing_obligation_id = format!("obligation.facing.{token}");
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
    let actor_question = match family {
        Gz2e01BossDoorFamily::L1 => {
            "Confirm the loaded boss-door resources and actor/event phases reach INIT, UNLOCK, collision release, CHG_SCENE, and restart handling without an intervening failure or interruption."
        }
        Gz2e01BossDoorFamily::L5 => {
            "Confirm the loaded L5 boss-door resources and event phases reach UNLOCK, key deletion when present, collision release, CHG_SCENE, close/end handling, and the restart-room write without an intervening failure or interruption."
        }
    };

    let object_id = stable_token(
        "world.object",
        &[inventory.stage.as_bytes(), placement.stable_id.as_bytes()],
    );
    let (spatial_volumes, spatial_planes, mut physical_obligation_ids, mut imported_obligations) =
        match family {
            Gz2e01BossDoorFamily::L1 => {
                let spatial_source_sha256 =
                    boss_door_spatial_source_digest(family, inventory_sha256, placement);
                let position = canonicalize_position([
                    placement.position.x,
                    placement.position.y,
                    placement.position.z,
                ]);
                let form_is = |form: &str| PredicateExpression::Compare {
                    left: ValueReference::PlayerForm,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text(form.into()),
                    },
                };
                let volume = |volume_id: &str, position: crate::transition::InteractionPosition| {
                    crate::transition::InteractionVolumeTest {
                        position,
                        volume: crate::transition::VolumeReference {
                            object_id: object_id.clone(),
                            volume_id: volume_id.into(),
                        },
                        must_be_inside: true,
                    }
                };
                (
                    vec![
                        SpatialVolume {
                            object_id: object_id.clone(),
                            volume_id: "boss-door-check-area".into(),
                            shape: SpatialVolumeShape::YawOrientedRectangle {
                                origin_xz: [position[0], position[2]],
                                yaw: placement.angle[1],
                                minimum_local_xz: [-200.0, -100.0],
                                maximum_local_xz: [200.0, 100.0],
                            },
                            source_sha256: spatial_source_sha256,
                        },
                        SpatialVolume {
                            object_id: object_id.clone(),
                            volume_id: "boss-door-wolf-current-x".into(),
                            shape: SpatialVolumeShape::YawOrientedStrip {
                                origin_xz: [position[0], position[2]],
                                yaw: placement.angle[1],
                                axis: crate::state::SpatialLocalAxis::X,
                                minimum: -130.0,
                                maximum: 130.0,
                            },
                            source_sha256: spatial_source_sha256,
                        },
                    ],
                    Vec::new(),
                    vec![
                        actor_obligation_id.clone(),
                        interaction_obligation_id.clone(),
                        facing_obligation_id.clone(),
                    ],
                    vec![
                        FeasibilityObligation {
                            id: interaction_obligation_id.clone(),
                            label: format!("Satisfy {} form-specific actor-local area checks", placement.name),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Interaction,
                            stage: crate::transition::ObligationStage::Activate,
                            detail: ObligationDetail::CompoundInteraction {
                                actor_instance_id: object_id.clone(),
                                interaction_mode: "door".into(),
                                branches: vec![
                                    crate::transition::InteractionBranch {
                                        when: form_is("human"),
                                        volume_tests: vec![volume(
                                            "boss-door-check-area",
                                            crate::transition::InteractionPosition::Player,
                                        )],
                                        pose_predicate: PredicateExpression::True,
                                    },
                                    crate::transition::InteractionBranch {
                                        when: form_is("wolf"),
                                        volume_tests: vec![
                                            volume(
                                                "boss-door-check-area",
                                                crate::transition::InteractionPosition::PlayerAttention,
                                            ),
                                            volume(
                                                "boss-door-wolf-current-x",
                                                crate::transition::InteractionPosition::Player,
                                            ),
                                        ],
                                        pose_predicate: PredicateExpression::True,
                                    },
                                ],
                                temporal_requirement: None,
                            },
                            evidence: evidence.clone(),
                        },
                        FeasibilityObligation {
                            id: facing_obligation_id.clone(),
                            label: format!("Face {} within binary-angle delta 0x4000", placement.name),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Interaction,
                            stage: crate::transition::ObligationStage::Activate,
                            detail: ObligationDetail::Facing {
                                yaw: ValueReference::PlayerRotationY,
                                target_yaw: placement.angle[1].wrapping_sub(0x7fff),
                                maximum_delta: 0x4000,
                            },
                            evidence: evidence.clone(),
                        },
                    ],
                )
            }
            Gz2e01BossDoorFamily::L5 => {
                let spatial_source_sha256 =
                    boss_door_spatial_source_digest(family, inventory_sha256, placement);
                let radians = f64::from(placement.angle[1]) * std::f64::consts::TAU / 65536.0;
                let (sin, cos) = radians.sin_cos();
                let normal = canonicalize_position([sin as f32, 0.0, cos as f32]);
                let position = canonicalize_position([
                    placement.position.x,
                    placement.position.y,
                    placement.position.z,
                ]);
                let offset =
                    canonicalize_scalar(-(normal[0] * position[0] + normal[2] * position[2]));
                let plane_id = format!("plane.front.{token}");
                (
                    vec![SpatialVolume {
                        object_id: object_id.clone(),
                        volume_id: "boss-door-check-area".into(),
                        shape: SpatialVolumeShape::YawOrientedRectangle {
                            origin_xz: [position[0], position[2]],
                            yaw: placement.angle[1],
                            minimum_local_xz: [-200.0, -100.0],
                            maximum_local_xz: [200.0, 100.0],
                        },
                        source_sha256: spatial_source_sha256,
                    }],
                    vec![SpatialPlane {
                        plane_id: plane_id.clone(),
                        normal,
                        offset,
                        source_sha256: spatial_source_sha256,
                    }],
                    vec![
                        actor_obligation_id.clone(),
                        interaction_obligation_id.clone(),
                        front_obligation_id.clone(),
                        facing_obligation_id.clone(),
                    ],
                    vec![
                        FeasibilityObligation {
                            id: interaction_obligation_id.clone(),
                            label: format!(
                                "Stand within {} actor-local checkArea rectangle",
                                placement.name
                            ),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Interaction,
                            stage: crate::transition::ObligationStage::Activate,
                            detail: ObligationDetail::Interaction {
                                actor_instance_id: object_id.clone(),
                                interaction_mode: "door".into(),
                                required_volumes: vec![crate::transition::VolumeReference {
                                    object_id: object_id.clone(),
                                    volume_id: "boss-door-check-area".into(),
                                }],
                                excluded_volumes: Vec::new(),
                                pose_predicate: PredicateExpression::True,
                                temporal_requirement: None,
                            },
                            evidence: evidence.clone(),
                        },
                        FeasibilityObligation {
                            id: front_obligation_id.clone(),
                            label: format!(
                                "Approach {} from positive actor-local Z",
                                placement.name
                            ),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Geometry,
                            stage: crate::transition::ObligationStage::Reach,
                            detail: ObligationDetail::PlaneSide {
                                plane_id,
                                relation: crate::state::PlaneRelation::Positive,
                            },
                            evidence: evidence.clone(),
                        },
                        FeasibilityObligation {
                            id: facing_obligation_id.clone(),
                            label: format!(
                                "Face {} within binary-angle delta 0x4000",
                                placement.name
                            ),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Interaction,
                            stage: crate::transition::ObligationStage::Activate,
                            detail: ObligationDetail::Facing {
                                yaw: ValueReference::PlayerRotationY,
                                target_yaw: placement.angle[1].wrapping_sub(0x7fff),
                                maximum_delta: 0x4000,
                            },
                            evidence: evidence.clone(),
                        },
                    ],
                )
            }
        };
    physical_obligation_ids.sort();
    imported_obligations.push(FeasibilityObligation {
        id: actor_obligation_id.clone(),
        label: format!(
            "Run the loaded {} keyhole, event, collision, and scene-change phases",
            placement.name
        ),
        scope: scope.clone(),
        obligation_kind: ObligationKind::ActorState,
        stage: crate::transition::ObligationStage::Effect,
        detail: ObligationDetail::Unresolved {
            research_question: actor_question.into(),
        },
        evidence: unknown_evidence.clone(),
    });
    imported_obligations.sort_by(|left, right| left.id.cmp(&right.id));

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
                physical_obligation_ids,
                effects,
                unknown_requirements: Vec::new(),
            },
            evidence,
        },
        obligations: imported_obligations,
        spatial_volumes,
        spatial_planes,
    }))
}

pub(super) fn boss_door_spatial_source_digest(
    family: Gz2e01BossDoorFamily,
    inventory_sha256: Digest,
    placement: &PlacementRecord,
) -> Digest {
    let source_sha256 = match family {
        Gz2e01BossDoorFamily::L1 => {
            static_digest("221c170e034cf90cc43b20dc737bebeb44d6f8b54111d4454024f2fea7069d79")
        }
        Gz2e01BossDoorFamily::L5 => {
            static_digest("9f649b99f027e39f1d39ce066d815a78032b536c4a9a83e0361681af2265102e")
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.route-planner.boss-door-spatial-source/v1");
    hasher.update(inventory_sha256.0);
    hasher.update(source_sha256.0);
    hasher.update(placement.stable_id.as_bytes());
    Digest(hasher.finalize().into())
}

pub(super) fn gz2e01_boss_door_family(name: &str) -> Option<Gz2e01BossDoorFamily> {
    match name {
        "L1Bdoor" | "L2Bdoor" | "L4Bdoor" | "L6Bdoor" | "L7Bdoor" | "L8Bdoor" | "L9Bdoor" => {
            Some(Gz2e01BossDoorFamily::L1)
        }
        "L5Bdoor" => Some(Gz2e01BossDoorFamily::L5),
        _ => None,
    }
}

pub(super) fn memory_switch_raw_location(switch_id: u8) -> (u32, u8) {
    let word = u32::from(switch_id / 32);
    let bit_in_word = switch_id % 32;
    let byte_in_word = 3 - u32::from(bit_in_word / 8);
    let byte_offset = 0x08 + word * 4 + byte_in_word;
    (byte_offset, 1_u8 << (bit_in_word % 8))
}

pub(super) fn is_source_audited_gz2e01(content: &ContentIdentity) -> bool {
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

pub(super) fn gz2e01_boss_door_evidence(
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
