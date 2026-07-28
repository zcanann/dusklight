//! Import keyed doors, shutters, switches, and gates from GZ2E01.

use super::*;

pub(super) struct ImportedKeyedActorActions {
    pub(super) exit_record_id: Option<String>,
    pub(super) transitions: Vec<CandidateTransition>,
    pub(super) obligations: Vec<FeasibilityObligation>,
}

pub(super) fn import_gz2e01_keyed_actor_actions(
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
        "vshuter" => {
            import_gz2e01_external_switch_shutter(inventory, placement, scope, inventory_sha256)
        }
        "Wchain" => import_gz2e01_wolf_chain_switch(inventory, placement, scope, inventory_sha256),
        "K_Gate" => import_gz2e01_koki_gate(inventory, placement, scope, inventory_sha256),
        "R_Gate" => import_gz2e01_rider_gate(inventory, placement, scope, inventory_sha256),
        "CrvGate" => import_gz2e01_caravan_gate(inventory, placement, scope, inventory_sha256),
        _ => Ok(None),
    }
}

pub(super) fn import_gz2e01_keyed_mboss_door(
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

pub(super) fn import_gz2e01_key_shutter(
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

pub(super) fn import_gz2e01_external_switch_shutter(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let switch_id = placement.parameters as u8;
    let runtime_type = ((placement.parameters >> 8) as u8).wrapping_add(1);
    let checks_key = placement.parameters >> 31 != 0;
    if inventory.stage != "R_SP116"
        || room != 6
        || switch_id != 0xef
        || runtime_type != 4
        || checks_key
    {
        return Ok(None);
    }

    let family = "external-switch-shutter";
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: format!("evidence.source.actor.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "dca04961403031ef232059f5f9f8997d2f0a3965b111e97d9d72604e0014d14b",
                )),
                note: "d_a_obj_kshutter.cpp: runtime type 4 has no internal key check or switch writer and opens after its external switch becomes set.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.switch-dispatch.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "a275457390b8464750adaab345c769afa2dc0b295baba47a617ce6aad6fd26d3",
                )),
                note: "d_save.cpp: switch 0xef resolves through the current room's one-zone switch store.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.world.inventory.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(inventory_sha256),
                note: format!(
                    "Authenticated R_SP116 room-6 vshuter placement {} with external one-zone switch 0xef.",
                    placement.stable_id
                ),
            },
        ],
    };
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let actor_id = format!("obligation.actor-state.{token}");
    let passage_id = format!("obligation.interaction.{token}.passage");
    let obligations = vec![
        FeasibilityObligation {
            id: actor_id.clone(),
            label: format!("Observe {} respond to external switch 0xef", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm the loaded type-4 shutter observes the already-set external switch and completes its opening/collision-release phases.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: passage_id.clone(),
            label: format!("Traverse the externally opened {}", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Witness sufficient shutter opening and background-collision release for passage.".into(),
            },
            evidence: unknown_evidence,
        },
    ];
    let transition = keyed_actor_candidate(
        scope,
        placement,
        family,
        "set-switch-passage",
        "R_SP116 vshuter room 6 externally switched passage",
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, room),
                room_switch_label_guard(switch_id, true),
            ],
        },
        Vec::new(),
        &[actor_id, passage_id],
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![transition],
        obligations,
    }))
}

pub(super) fn import_gz2e01_wolf_chain_switch(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let switch_id = placement.parameters as u8;
    let authored_repeat = ((placement.parameters >> 8) & 0x0f) as u8;
    let repeatable = authored_repeat != 0 && authored_repeat != 0x0f;
    if inventory.stage != "R_SP116"
        || room != 6
        || switch_id != 0xef
        || authored_repeat != 0x0f
        || repeatable
    {
        return Ok(None);
    }

    let family = "wolf-chain-switch";
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: format!("evidence.source.actor.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "e72a2bfcc715f03d1fa934a2033e4360aa22fbfd2ffd4c962cb7a27c949b7fd0",
                )),
                note: "d_a_obj_wchain.cpp: low parameter byte selects the switch; authored repeat nibble 0xf normalizes to one-shot; onNowSwitch writes the clear switch on the next chain execute.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.player.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "b0c094b0c95144d7c5f89bc1d35d63fcde80f1f032a7772670a8142eb4dc9d8d",
                )),
                note: "d_a_alink_wolf.inc: wolf chain ready/wait state attaches to Wchain and raises onNowSwitch after pull length exceeds the exact 94-unit switch offset.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.switch-dispatch.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "a275457390b8464750adaab345c769afa2dc0b295baba47a617ce6aad6fd26d3",
                )),
                note: "d_save.cpp: switch 0xef resolves through the current room's one-zone switch store.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.world.inventory.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(inventory_sha256),
                note: format!(
                    "Authenticated R_SP116 room-6 Wchain placement {} with parameters 0x00000fef.",
                    placement.stable_id
                ),
            },
        ],
    };
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let interaction_id = format!("obligation.interaction.{token}");
    let effect_id = format!("obligation.actor-state.{token}");
    let obligations = vec![
        FeasibilityObligation {
            id: interaction_id.clone(),
            label: "Reach, bite, and pull the R_SP116 room-6 wolf chain".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Reach the chain as wolf, acquire its attention target, complete the ready jump without a wall hit, remain attached through the tension wait, and pull past the exact 94-unit switch offset.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: effect_id.clone(),
            label: "Commit the R_SP116 Wchain one-shot switch write".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm onNowSwitch survives into the chain actor's next execute, writes one-zone switch 0xef, and completes without an intervening unload.".into(),
            },
            evidence: unknown_evidence,
        },
    ];
    let transition = keyed_actor_candidate(
        scope,
        placement,
        family,
        "wolf-pull-switch",
        "R_SP116 room 6 wolf-chain pull sets one-zone switch 0xef",
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, room),
                PredicateExpression::Compare {
                    left: ValueReference::PlayerForm,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("wolf".into()),
                    },
                },
                room_switch_label_guard(switch_id, false),
            ],
        },
        vec![room_switch_label_write(switch_id, true)],
        &[effect_id, interaction_id],
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![transition],
        obligations,
    }))
}

pub(super) fn import_gz2e01_koki_gate(
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

pub(super) fn import_gz2e01_rider_gate(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let switch_id = placement.parameters as u8;
    let (switch_guard, switch_write, switch_description) =
        match (inventory.stage.as_str(), room, switch_id) {
            ("F_SP109", 0, 0x6b) => (
                memory_switch_guard as fn(u8, bool) -> PredicateExpression,
                memory_switch_write as fn(u8) -> StateOperation,
                "memory switch 0x6b",
            ),
            ("F_SP121", 3, 0x82) | ("F_SP121", 15, 0x81) => (
                dungeon_session_switch_guard as fn(u8, bool) -> PredicateExpression,
                dungeon_session_switch_write as fn(u8) -> StateOperation,
                if switch_id == 0x82 {
                    "dungeon-session switch 0x82"
                } else {
                    "dungeon-session switch 0x81"
                },
            ),
            _ => return Ok(None),
        };
    let family = "rider-gate";
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let mut evidence_records = vec![
        EvidenceRecord {
            id: format!("evidence.source.actor.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "eb644962c9c9596514d552e2f87015f1c68786bf998ff79d41a606276750bffb",
            )),
            note: "d_a_obj_rgate.cpp: key/facing/box offer guard, queued decrement, switch writer, M_035 bypass, event completion, and post-unlock pushing.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.event.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(static_digest(
                "95582d74d858aeb5b01a9f1beb6c0c1bd6761b619b75f57d1d60d906f73ea856",
            )),
            note: "M_RGate00/event_list.dat: RIDER_GATE_OPEN00 contains UNLOCK before OPEN.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.event-label.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "8804d987bb1da08281c143d96d46a2832f02650f4b9777b010f196ed20847a14",
            )),
            note: "d_save_bit_labels.inc: saveBitLabels[68] is M_035 at packed persistent-event coordinate 0x0810.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.name-map.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
            )),
            note: "d_stage.cpp: R_Gate maps to the rider-gate actor process.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.switch-dispatch.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "a275457390b8464750adaab345c769afa2dc0b295baba47a617ce6aad6fd26d3",
            )),
            note: "d_save.cpp: switches 0x80 through 0xbf resolve to the 64-bit dungeon-session switch store; lower switches resolve to loaded stage memory.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.world.inventory.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(inventory_sha256),
            note: format!(
                "Authenticated {} room-{} layered rider-gate placement {} with {}.",
                inventory.stage, room, placement.stable_id, switch_description
            ),
        },
    ];
    evidence_records.sort_by(|left, right| left.id.cmp(&right.id));
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: evidence_records,
    };
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let actor_id = format!("obligation.actor-state.{token}");
    let interaction_id = format!("obligation.interaction.{token}");
    let passage_id = format!("obligation.interaction.{token}.passage");
    let obligations = vec![
        FeasibilityObligation {
            id: actor_id.clone(),
            label: format!("Complete {} keyed event and committed key delta", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm the accepted door command, RIDER_GATE_OPEN00 cuts, event reset, and uncontended queued key-delta commit complete without interruption.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: interaction_id.clone(),
            label: format!("Reach and activate {} keyed side", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Reach actor-local x in [-100, 100] and z in [0, 100] with the required facing while the gate owns its door event.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: passage_id.clone(),
            label: format!("Traverse the physically open {}", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Witness sufficient leaf opening and collision clearance under either the set-switch push behavior or the M_035 forced-open bypass.".into(),
            },
            evidence: unknown_evidence,
        },
    ];
    let location = placement_location_guard(inventory, placement, room);
    let locked = PredicateExpression::All {
        terms: vec![
            location.clone(),
            switch_guard(switch_id, false),
            persistent_event_bit_guard(0x0810, false),
        ],
    };
    let ordinary = keyed_actor_candidate(
        scope,
        placement,
        family,
        "unlock",
        &format!(
            "{} {} layer {} keyed rider-gate unlock",
            inventory.stage,
            placement.name,
            placement.layer.unwrap_or_default()
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                locked.clone(),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
                small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
            ],
        },
        vec![small_key_adjust(-1), switch_write(switch_id)],
        &[actor_id.clone(), interaction_id.clone()],
        &evidence,
    );
    let high = keyed_actor_candidate(
        scope,
        placement,
        family,
        "unlock-high-key-clamp",
        &format!(
            "{} {} layer {} rider-gate unlock with high raw keys clamped to 99",
            inventory.stage,
            placement.name,
            placement.layer.unwrap_or_default()
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                locked,
                small_key_guard(ComparisonOperator::GreaterThan, 100),
            ],
        },
        vec![small_key_write(99), switch_write(switch_id)],
        &[actor_id, interaction_id],
        &evidence,
    );
    let unlocked_passage = keyed_actor_candidate(
        scope,
        placement,
        family,
        "set-switch-passage",
        &format!(
            "{} {} layer {} set-switch physical passage",
            inventory.stage,
            placement.name,
            placement.layer.unwrap_or_default()
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                location.clone(),
                switch_guard(switch_id, true),
                persistent_event_bit_guard(0x0810, false),
            ],
        },
        Vec::new(),
        std::slice::from_ref(&passage_id),
        &evidence,
    );
    let event_bypass = keyed_actor_candidate(
        scope,
        placement,
        family,
        "m035-forced-open-passage",
        &format!(
            "{} {} layer {} M_035 forced-open passage",
            inventory.stage,
            placement.name,
            placement.layer.unwrap_or_default()
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![location, persistent_event_bit_guard(0x0810, true)],
        },
        Vec::new(),
        &[passage_id],
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![ordinary, high, unlocked_passage, event_bypass],
        obligations,
    }))
}

pub(super) fn import_gz2e01_caravan_gate(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room @ (1 | 2)) = placement.scope.room else {
        return Ok(None);
    };
    if inventory.stage != "F_SP118" || placement.parameters != u32::MAX {
        return Ok(None);
    }
    let family = "caravan-gate";
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let mut evidence_records = vec![
        EvidenceRecord {
            id: format!("evidence.source.actor.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "f0916a79d3b157454dd2263307567e472d4f394d61ad8ece9153500d91943697",
            )),
            note: "d_a_obj_crvgate.cpp: parent/child creation, key/facing/distance offer guard, queued decrement, transient paired opening, and boar/event destruction path.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.name-map.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
            )),
            note: "d_stage.cpp: CrvGate maps to the caravan-gate actor process.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.world.inventory.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(inventory_sha256),
            note: format!(
                "Authenticated F_SP118 room-{room} caravan-gate parent placement {}.",
                placement.stable_id
            ),
        },
    ];
    evidence_records.sort_by(|left, right| left.id.cmp(&right.id));
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: evidence_records,
    };
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let actor_id = format!("obligation.actor-state.{token}");
    let interaction_id = format!("obligation.interaction.{token}");
    let boar_id = format!("obligation.interaction.{token}.boar-destruction");
    let obligations = vec![
        FeasibilityObligation {
            id: actor_id.clone(),
            label: format!("Complete the paired room-{room} caravan-gate key event"),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm parent creation, child lookup, accepted demo command, uncontended queued key-delta commit, camera reset, and transient SetOpen on both leaves complete without interruption.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: interaction_id.clone(),
            label: format!("Reach and activate the room-{room} caravan gate"),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Reach within 200 world-XZ units with player/gate facing delta at least 0x5000 while the parent owns its door command.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: boar_id.clone(),
            label: format!("Destroy the room-{room} caravan-gate pair with the running boar"),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "During a running event, collide the ridden E_WB boar with a gate sphere inside the 490-unit bound at nonzero speed, then witness both paired leaves enter and complete their destruction/open state.".into(),
            },
            evidence: unknown_evidence,
        },
    ];
    let location = placement_location_guard(inventory, placement, room);
    let ordinary = keyed_actor_candidate(
        scope,
        placement,
        family,
        "key-open",
        &format!("F_SP118 room {room} caravan-gate transient key opening"),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                location.clone(),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
                small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
            ],
        },
        vec![small_key_adjust(-1)],
        &[actor_id.clone(), interaction_id.clone()],
        &evidence,
    );
    let high = keyed_actor_candidate(
        scope,
        placement,
        family,
        "key-open-high-key-clamp",
        &format!(
            "F_SP118 room {room} caravan-gate transient opening with high raw keys clamped to 99"
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                location.clone(),
                small_key_guard(ComparisonOperator::GreaterThan, 100),
            ],
        },
        vec![small_key_write(99)],
        &[actor_id, interaction_id],
        &evidence,
    );
    let boar_bypass = keyed_actor_candidate(
        scope,
        placement,
        family,
        "boar-destruction-bypass",
        &format!("F_SP118 room {room} caravan-gate boar destruction bypass"),
        TransitionKind::ActorDriven,
        location,
        Vec::new(),
        &[boar_id],
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![ordinary, high, boar_bypass],
        obligations,
    }))
}
