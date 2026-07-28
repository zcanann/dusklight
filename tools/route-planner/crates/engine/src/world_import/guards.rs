//! Construct typed planner guards, writes, obligations, and evidence.

use super::*;

pub(super) fn scene_location(exit: &crate::world_data::StageExitRecord) -> SceneLocation {
    SceneLocation {
        stage: exit.destination_stage.clone(),
        room: exit.destination_room,
        layer: exit.destination_layer,
        spawn: exit.destination_point,
    }
}

pub(super) fn location_layer_guard(
    operator: ComparisonOperator,
    layer: i64,
) -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::LocationLayer,
        operator,
        right: ValueReference::Literal {
            value: StateValue::Signed(layer),
        },
    }
}

pub(super) fn placement_location_guard(
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

pub(super) fn small_key_guard(operator: ComparisonOperator, value: u64) -> PredicateExpression {
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

pub(super) fn boss_key_guard() -> PredicateExpression {
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

pub(super) fn memory_switch_guard(switch_id: u8, set: bool) -> PredicateExpression {
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

pub(super) fn dungeon_session_switch_guard(switch_id: u8, set: bool) -> PredicateExpression {
    debug_assert!((0x80..0xc0).contains(&switch_id));
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::Custom {
                id: DUNGEON_SESSION_SWITCH_LABEL_KIND.into(),
            },
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: u32::from(switch_id - 0x80),
            byte_width: 1,
            mask: 1,
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(u64::from(set)),
        },
    }
}

pub(super) fn room_switch_label_guard(switch_id: u8, set: bool) -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::Custom {
                id: ROOM_SWITCH_LABEL_KIND.into(),
            },
            binding: ComponentBindingReference::CurrentRoom,
            byte_offset: u32::from(switch_id),
            byte_width: 1,
            mask: 1,
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(u64::from(set)),
        },
    }
}

pub(super) fn persistent_event_bit_guard(packed_coordinate: u16, set: bool) -> PredicateExpression {
    let byte_offset = u32::from(packed_coordinate >> 8);
    let mask = packed_coordinate as u8;
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::Custom {
                id: "persistent-event-registers".into(),
            },
            binding: ComponentBindingReference::ActiveRuntimeFile,
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

pub(super) fn memory_switch_write(switch_id: u8) -> StateOperation {
    let (byte_offset, mask) = memory_switch_raw_location(switch_id);
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset,
        mask: vec![mask],
        value: vec![mask],
    }
}

pub(super) fn dungeon_session_switch_write(switch_id: u8) -> StateOperation {
    debug_assert!((0x80..0xc0).contains(&switch_id));
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::Custom {
            id: DUNGEON_SESSION_SWITCH_LABEL_KIND.into(),
        },
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: u32::from(switch_id - 0x80),
        mask: vec![1],
        value: vec![1],
    }
}

pub(super) fn room_switch_label_write(switch_id: u8, set: bool) -> StateOperation {
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::Custom {
            id: ROOM_SWITCH_LABEL_KIND.into(),
        },
        binding: ComponentBindingReference::CurrentRoom,
        byte_offset: u32::from(switch_id),
        mask: vec![1],
        value: vec![u8::from(set)],
    }
}

pub(super) fn small_key_adjust(delta: i64) -> StateOperation {
    StateOperation::AdjustBoundRawUnsigned {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: 0x1c,
        byte_width: 1,
        delta,
    }
}

pub(super) fn small_key_write(value: u8) -> StateOperation {
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: 0x1c,
        mask: vec![0xff],
        value: vec![value],
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn keyed_actor_candidate(
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

pub(super) fn keyed_actor_obligations(
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
                stage: crate::transition::ObligationStage::Effect,
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
                stage: crate::transition::ObligationStage::Activate,
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
pub(super) fn keyed_actor_evidence(
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
