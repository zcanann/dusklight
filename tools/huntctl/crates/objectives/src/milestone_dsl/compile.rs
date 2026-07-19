use super::*;
use std::format;

/// Compile a validated AST to canonical DMSP v1 bytes.
pub fn compile(program: &MilestoneProgram) -> Result<CompiledMilestones, BinaryError> {
    validate_program(program).map_err(BinaryError)?;
    let mut records = Vec::new();
    let mut identities = Vec::with_capacity(program.definitions.len());
    for definition in &program.definitions {
        let mut bytecode = Vec::new();
        let mut operation_count = 0_u16;
        if definition.then.is_empty() {
            encode_expression(&definition.when, &mut bytecode, &mut operation_count)?;
        } else {
            bytecode.push(0x40);
            push_u16(&mut bytecode, definition.within_ticks.unwrap());
            bytecode.push((definition.then.len() + 1) as u8);
            increment_ops(&mut operation_count, 1)?;
            for step in std::iter::once(&definition.when).chain(&definition.then) {
                encode_expression(step, &mut bytecode, &mut operation_count)?;
                bytecode.push(0x41);
                increment_ops(&mut operation_count, 1)?;
            }
        }
        for projection in &definition.projections {
            encode_value_projection(projection, &mut bytecode, &mut operation_count)?;
        }
        let name = definition.name.as_bytes();
        let identity_bytes = definition_identity_bytes(
            name,
            definition.phase,
            definition.stable_ticks,
            operation_count,
            &bytecode,
        )?;
        let definition_sha256: [u8; 32] = Sha256::new()
            .chain_update(DEFINITION_DOMAIN)
            .chain_update(&identity_bytes)
            .finalize()
            .into();
        let record_len = RECORD_FIXED_BYTES
            .checked_add(name.len())
            .and_then(|length| length.checked_add(bytecode.len()))
            .ok_or_else(|| BinaryError("milestone record length overflow".into()))?;
        push_u32(&mut records, usize_u32(record_len, "milestone record")?);
        records.extend_from_slice(&identity_bytes[..identity_bytes.len() - bytecode.len()]);
        records.extend_from_slice(&definition_sha256);
        records.extend_from_slice(&bytecode);
        identities.push(CompiledDefinitionIdentity {
            name: definition.name.clone(),
            sha256: definition_sha256,
        });
    }

    let mut bytes = Vec::with_capacity(HEADER_BYTES + records.len());
    bytes.extend_from_slice(&MAGIC);
    push_u16(&mut bytes, WIRE_VERSION.0);
    push_u16(&mut bytes, program.version.minor);
    push_u16(&mut bytes, program.version.major);
    push_u16(&mut bytes, program.version.minor);
    push_u16(
        &mut bytes,
        u16::try_from(program.definitions.len())
            .map_err(|_| BinaryError("too many milestone definitions".into()))?,
    );
    push_u16(&mut bytes, 0);
    push_u32(&mut bytes, usize_u32(records.len(), "program payload")?);
    bytes.extend_from_slice(&[0; 32]);
    bytes.extend_from_slice(&records);
    if bytes.len() > MAX_BINARY_BYTES {
        return Err(BinaryError(format!(
            "compiled milestone program exceeds {MAX_BINARY_BYTES} bytes"
        )));
    }
    let program_sha256 = program_digest(&bytes);
    bytes[20..52].copy_from_slice(&program_sha256);
    Ok(CompiledMilestones {
        bytes,
        program_sha256,
        definitions: identities,
    })
}

fn encode_value_projection(
    projection: &ValueProjection,
    output: &mut Vec<u8>,
    operations: &mut u16,
) -> Result<(), BinaryError> {
    output.push(0x50);
    output.push(
        u8::try_from(projection.name.len())
            .map_err(|_| BinaryError("projection name is too long".into()))?,
    );
    output.extend_from_slice(projection.name.as_bytes());
    output.push(
        u8::try_from(projection.items.len())
            .map_err(|_| BinaryError("projection has too many items".into()))?,
    );
    increment_ops(operations, 1)?;
    for item in &projection.items {
        match item {
            ValueProjectionItem::Rng { stream } => {
                output.extend_from_slice(&[0x51, *stream as u8]);
            }
            ValueProjectionItem::ActorPopulation { stage, room } => {
                output.push(0x52);
                let mut fixed_stage = [0_u8; 8];
                fixed_stage[..stage.len()].copy_from_slice(stage.as_bytes());
                output.extend_from_slice(&fixed_stage);
                output.push(*room as u8);
            }
            ValueProjectionItem::Flag { selector } => {
                output.extend_from_slice(&[0x53, selector.domain as u8, selector.room as u8]);
                push_u16(output, selector.index);
            }
        }
        increment_ops(operations, 1)?;
    }
    Ok(())
}

/// Stable identity for one named projection, independent of milestone topology.
pub fn value_projection_identity(projection: &ValueProjection) -> Result<[u8; 32], BinaryError> {
    validate_text(&projection.name, MAX_NAME_BYTES, false).map_err(BinaryError)?;
    if projection.items.is_empty() || projection.items.len() > MAX_PROJECTION_ITEMS {
        return Err(BinaryError("invalid projection item count".into()));
    }
    for item in &projection.items {
        match item {
            ValueProjectionItem::Rng { .. } => {}
            ValueProjectionItem::ActorPopulation { stage, room }
                if valid_stage_name(stage) && (-1..=63).contains(room) => {}
            ValueProjectionItem::Flag { selector } => {
                validate_query_fact(&QueryFact::Flag {
                    selector: selector.clone(),
                })
                .map_err(BinaryError)?;
            }
            _ => {
                return Err(BinaryError(
                    "invalid actor population projection scope".into(),
                ));
            }
        }
    }
    let mut bytes = Vec::new();
    let mut operations = 0;
    encode_value_projection(projection, &mut bytes, &mut operations)?;
    Ok(Sha256::new()
        .chain_update(PROJECTION_DOMAIN)
        .chain_update(bytes)
        .finalize()
        .into())
}
