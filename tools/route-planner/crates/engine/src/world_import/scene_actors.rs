//! Import scene-actor actions whose semantics span placements and exits.

use super::*;

pub(super) struct ImportedSceneActorActions {
    pub(super) transitions: Vec<(String, CandidateTransition)>,
    pub(super) obligations: Vec<FeasibilityObligation>,
}

pub(super) fn import_gz2e01_l7_bridge_demo(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedSceneActorActions>, PlannerContractError> {
    if inventory.stage != "D_MN07"
        || placement.name != "dr"
        || placement.kind != PlacementKind::Actor
        || placement.scope.room != Some(6)
        || placement.parameters != 0x18
    {
        return Ok(None);
    }
    let matching_exit = |record_index| {
        inventory
            .exits
            .iter()
            .filter(|exit| exit.scope.room == Some(6) && exit.record_index == record_index)
            .collect::<Vec<_>>()
    };
    let pre_bridge_exits = matching_exit(6);
    let [pre_bridge_exit] = pre_bridge_exits.as_slice() else {
        return Ok(None);
    };
    let post_bridge_exits = matching_exit(7);
    let [post_bridge_exit] = post_bridge_exits.as_slice() else {
        return Ok(None);
    };

    let family = "l7-bridge-demo";
    let token = stable_token(
        "actor-scene",
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
                "7b350f2e3efa4ddb5907b38d4f1f8ceb91d37cc741dce7e4d7de67d436421b02",
            )),
            note: "d_a_L7demo_dr.cpp: layer-sensitive start guards, exact SCLS 6/7 scene requests, switch 0x18 write, and absence of a key decrement.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.name-map.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
            )),
            note: "d_stage.cpp: the exact `dr` placement name maps to the DR/L7 bridge-demo actor process.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.save-layout.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "fdac35e3d54a3c496dc20fd2f5e297fa9411a78fb7d09be607a62fa0cfa0c110",
            )),
            note: "d_save.h: current-stage memory switch 0x18 and the small-key count occupy distinct dSv_memBit_c fields.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.world.inventory.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(inventory_sha256),
            note: format!(
                "Authenticated D_MN07 room-6 placement {} and its exact room SCLS records.",
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
    let pre_interaction_id = format!("obligation.interaction.{token}.pre-bridge");
    let pre_effect_id = format!("obligation.actor-state.{token}.pre-bridge");
    let post_effect_id = format!("obligation.actor-state.{token}.post-bridge");
    let obligations = vec![
        FeasibilityObligation {
            id: pre_effect_id.clone(),
            label: "Complete the D_MN07 pre-bridge DR event and SCLS 6 request".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm event acceptance, camera/player demo ownership, the fixed walk, and the SCLS 6 scene request complete without interruption; no switch or key write belongs to this branch.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: post_effect_id.clone(),
            label: "Complete the D_MN07 bridge-destruction DR event and SCLS 7 request".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm the layer-3 DR event, both bridge-destruction phases, switch 0x18 write, event reset, and SCLS 7 scene request complete without interruption.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: pre_interaction_id.clone(),
            label: "Reach the D_MN07 DR pre-bridge start box with a key".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Reach world x in (-4480, -3730) and z in (-12800, -12100) outside layer 3 while the actor can acquire its potential event; the source applies no Y bound.".into(),
            },
            evidence: unknown_evidence,
        },
    ];

    let pre_bridge = keyed_actor_candidate(
        scope,
        placement,
        family,
        "enter-bridge-layer",
        "D_MN07 room 6 key-present DR event to bridge layer via SCLS 6",
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, 6),
                location_layer_guard(ComparisonOperator::NotEqual, 3),
                memory_switch_guard(0x18, false),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
            ],
        },
        vec![StateOperation::SetLocation {
            location: scene_location(pre_bridge_exit),
        }],
        &[pre_effect_id, pre_interaction_id],
        &evidence,
    );
    let post_bridge = keyed_actor_candidate(
        scope,
        placement,
        family,
        "destroy-bridge",
        "D_MN07 room 6 layer-3 DR bridge destruction via SCLS 7",
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, 6),
                location_layer_guard(ComparisonOperator::Equal, 3),
                memory_switch_guard(0x18, false),
            ],
        },
        vec![
            memory_switch_write(0x18),
            StateOperation::SetLocation {
                location: scene_location(post_bridge_exit),
            },
        ],
        &[post_effect_id],
        &evidence,
    );
    Ok(Some(ImportedSceneActorActions {
        transitions: vec![
            (pre_bridge_exit.stable_id.clone(), pre_bridge),
            (post_bridge_exit.stable_id.clone(), post_bridge),
        ],
        obligations,
    }))
}
