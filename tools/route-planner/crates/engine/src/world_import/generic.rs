//! Import generic immutable objects, spawns, and source guards.

use super::*;

pub(super) fn import_static_object(
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

pub(super) fn import_spawn(
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

pub(super) fn source_location_guard(stage: &str, room: i8) -> PredicateExpression {
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
