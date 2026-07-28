//! Canonicalize imported values and validate stable extracted collections.

use super::*;

pub(super) fn static_digest(value: &str) -> Digest {
    value.parse().expect("source-audit digest literal is valid")
}

pub(super) fn extracted_evidence(
    inventory_sha256: Digest,
    token: &str,
    inferred: bool,
) -> RuleEvidence {
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

pub(super) fn actor_type(placement: &PlacementRecord) -> String {
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

pub(super) fn stable_token(domain: &str, parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update([0]);
        hasher.update(part);
    }
    format!("{domain}.{}", Digest(hasher.finalize().into()))
}

pub(super) fn decode_hex(value: &str) -> Result<Vec<u8>, PlannerContractError> {
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

pub(super) fn f32_bytes(values: [f32; 3]) -> Vec<u8> {
    canonicalize_position(values)
        .into_iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .collect()
}

pub(super) fn i16_bytes(values: [i16; 3]) -> Vec<u8> {
    values.into_iter().flat_map(i16::to_le_bytes).collect()
}

pub(super) fn canonicalize_position(mut values: [f32; 3]) -> [f32; 3] {
    for value in &mut values {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    values
}

pub(super) fn canonicalize_scalar(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

pub(super) fn triangle_bounds(triangle: &[[f32; 3]; 3]) -> ([f32; 3], [f32; 3]) {
    let mut minimum = triangle[0];
    let mut maximum = triangle[0];
    for point in &triangle[1..] {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    (
        canonicalize_position(minimum),
        canonicalize_position(maximum),
    )
}

pub(super) fn validate_approach_shape(
    shape: &ExtractedApproachShape,
) -> Result<(), PlannerContractError> {
    match shape {
        ExtractedApproachShape::Reconstructed {
            triangle,
            plane_normal,
            plane_offset,
            minimum,
            maximum,
        } => {
            if triangle.iter().any(|point| !canonical_position(*point))
                || !canonical_position(*plane_normal)
                || !plane_offset.is_finite()
                || plane_offset.to_bits() == (-0.0_f32).to_bits()
                || plane_normal.iter().all(|value| *value == 0.0)
                || !canonical_position(*minimum)
                || !canonical_position(*maximum)
                || minimum
                    .iter()
                    .zip(maximum)
                    .any(|(minimum, maximum)| minimum > maximum)
                || triangle_bounds(triangle) != (*minimum, *maximum)
            {
                return Err(PlannerContractError::new(
                    "approach_geometries.shape",
                    "has invalid reconstructed triangle, plane, or exact bounds",
                ));
            }
            Ok(())
        }
        ExtractedApproachShape::Unavailable { reason } => {
            validate_label("approach_geometries.shape.reason", reason)
        }
    }
}

pub(super) fn canonical_position(values: [f32; 3]) -> bool {
    values
        .iter()
        .all(|value| value.is_finite() && value.to_bits() != (-0.0_f32).to_bits())
}

pub(super) fn validate_game_name(field: &str, value: &str) -> Result<(), PlannerContractError> {
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

pub(super) fn validate_sorted<T>(
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

pub(super) fn strictly_sorted(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

pub(super) fn world_error(field: &str, error: impl std::fmt::Display) -> PlannerContractError {
    PlannerContractError::new(field, error.to_string())
}
