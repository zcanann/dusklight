use super::*;
use std::format;

/// Parse, validate, and compile source in one operation.
pub fn compile_source(source: &str) -> Result<CompiledMilestones, DslError> {
    let program = parse(source)?;
    compile(&program).map_err(|error| DslError {
        line: 1,
        column: 1,
        message: error.to_string(),
    })
}

/// Return the canonical fact paths whose truth is consulted by one objective.
/// Reward features and proposer scores are intentionally outside this set.
pub fn required_query_facts(
    program: &MilestoneProgram,
    goal: &str,
) -> Result<Vec<String>, BinaryError> {
    let definition = program
        .definitions
        .iter()
        .find(|definition| definition.name == goal)
        .ok_or_else(|| BinaryError(format!("unknown milestone goal {goal}")))?;
    let mut facts = BTreeSet::new();
    collect_expression_facts(&definition.when, &mut facts);
    for expression in &definition.then {
        collect_expression_facts(expression, &mut facts);
    }
    Ok(facts.into_iter().collect())
}

pub(super) fn collect_expression_facts(expression: &Expression, facts: &mut BTreeSet<String>) {
    match expression {
        Expression::Compare { field, .. } => {
            facts.insert(field.path().into());
        }
        Expression::Query { fact, .. } => {
            facts.insert(fact.display_name().into());
        }
        Expression::Not(inner) => collect_expression_facts(inner, facts),
        Expression::And(left, right) | Expression::Or(left, right) => {
            collect_expression_facts(left, facts);
            collect_expression_facts(right, facts);
        }
    }
}

pub(super) fn definition_identity_bytes(
    name: &[u8],
    phase: EvaluationPhase,
    stable_ticks: u16,
    operation_count: u16,
    bytecode: &[u8],
) -> Result<Vec<u8>, BinaryError> {
    let mut identity = Vec::with_capacity(12 + name.len() + bytecode.len());
    push_u16(
        &mut identity,
        u16::try_from(name.len()).map_err(|_| BinaryError("milestone name too long".into()))?,
    );
    identity.extend_from_slice(name);
    identity.push(phase as u8);
    identity.push(0);
    push_u16(&mut identity, stable_ticks);
    push_u16(&mut identity, operation_count);
    push_u32(&mut identity, usize_u32(bytecode.len(), "bytecode")?);
    identity.extend_from_slice(bytecode);
    Ok(identity)
}

pub(super) fn encode_expression(
    expression: &Expression,
    output: &mut Vec<u8>,
    operations: &mut u16,
) -> Result<(), BinaryError> {
    match expression {
        Expression::Compare {
            field,
            operator,
            value,
        } => {
            output.extend_from_slice(&[0x01, *field as u8]);
            increment_ops(operations, 1)?;
            encode_value(value, output)?;
            increment_ops(operations, 1)?;
            output.push(*operator as u8);
            increment_ops(operations, 1)?;
        }
        Expression::Query {
            fact,
            operator,
            value,
        } => {
            encode_query_fact(fact, output)?;
            increment_ops(operations, 1)?;
            encode_value(value, output)?;
            increment_ops(operations, 1)?;
            output.push(*operator as u8);
            increment_ops(operations, 1)?;
        }
        Expression::Not(inner) => {
            encode_expression(inner, output, operations)?;
            output.push(0x30);
            increment_ops(operations, 1)?;
        }
        Expression::And(left, right) | Expression::Or(left, right) => {
            encode_expression(left, output, operations)?;
            encode_expression(right, output, operations)?;
            output.push(if matches!(expression, Expression::And(..)) {
                0x31
            } else {
                0x32
            });
            increment_ops(operations, 1)?;
        }
    }
    Ok(())
}

pub(super) fn encode_query_fact(fact: &QueryFact, output: &mut Vec<u8>) -> Result<(), BinaryError> {
    validate_query_fact(fact).map_err(BinaryError)?;
    output.push(0x02);
    match fact {
        QueryFact::PlacedActor { selector, field } => {
            output.push(1);
            output.push(*field as u8);
            let mut stage = [0_u8; 8];
            stage[..selector.stage.len()].copy_from_slice(selector.stage.as_bytes());
            output.extend_from_slice(&stage);
            output.push(selector.home_room as u8);
            push_u16(output, selector.set_id);
            output.extend_from_slice(&selector.actor_name.to_le_bytes());
        }
        QueryFact::Flag { selector } => {
            output.push(2);
            output.push(selector.domain as u8);
            output.push(selector.room as u8);
            push_u16(output, selector.index);
        }
        QueryFact::PlayerInAabb { minimum, maximum } => {
            output.push(3);
            for value in minimum.iter().chain(maximum) {
                output.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        QueryFact::PlayerPlaneSignedDistance { point, normal } => {
            output.push(4);
            for value in point.iter().chain(normal) {
                output.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
    }
    Ok(())
}

pub(super) fn encode_value(value: &Value, output: &mut Vec<u8>) -> Result<(), BinaryError> {
    match value {
        Value::Bool(value) => output.extend_from_slice(&[0x10, u8::from(*value)]),
        Value::U32(value) => {
            output.push(0x11);
            push_u32(output, *value);
        }
        Value::U64(value) => {
            output.push(0x12);
            output.extend_from_slice(&value.to_le_bytes());
        }
        Value::I32(value) => {
            output.push(0x13);
            output.extend_from_slice(&value.to_le_bytes());
        }
        Value::F32(value) => {
            if !value.is_finite() || value.to_bits() != canonical_float(*value).to_bits() {
                return Err(BinaryError("noncanonical floating-point constant".into()));
            }
            output.push(0x14);
            output.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        Value::Symbol(value) => encode_string_value(0x15, value, output)?,
        Value::ProcedureNumber(value) => {
            output.push(0x16);
            push_u32(output, *value);
        }
        Value::ProcedureSymbol(value) => encode_string_value(0x17, value, output)?,
    }
    Ok(())
}

pub(super) fn encode_string_value(
    opcode: u8,
    value: &str,
    output: &mut Vec<u8>,
) -> Result<(), BinaryError> {
    validate_text(value, MAX_SYMBOL_BYTES, false).map_err(BinaryError)?;
    output.push(opcode);
    output.push(u8::try_from(value.len()).map_err(|_| BinaryError("symbol is too long".into()))?);
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

pub(super) fn increment_ops(operations: &mut u16, amount: u16) -> Result<(), BinaryError> {
    *operations = operations
        .checked_add(amount)
        .ok_or_else(|| BinaryError("operation count overflow".into()))?;
    if usize::from(*operations) > MAX_OPS {
        return Err(BinaryError(format!(
            "expression exceeds {MAX_OPS} operations"
        )));
    }
    Ok(())
}

pub(super) fn program_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::new()
        .chain_update(PROGRAM_DOMAIN)
        .chain_update(&bytes[..20])
        .chain_update(&bytes[HEADER_BYTES..])
        .finalize()
        .into()
}

pub(super) fn usize_u32(value: usize, context: &str) -> Result<u32, BinaryError> {
    u32::try_from(value).map_err(|_| BinaryError(format!("{context} is too large")))
}

pub(super) fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

/// Strictly decode canonical DMSP v1 bytes and verify all embedded identities.
pub fn decode(bytes: &[u8]) -> Result<DecodedMilestones, BinaryError> {
    if bytes.len() < HEADER_BYTES || bytes.len() > MAX_BINARY_BYTES {
        return Err(BinaryError("invalid milestone program size".into()));
    }
    let mut cursor = Cursor::new(bytes);
    if cursor.take(4)? != MAGIC {
        return Err(BinaryError("invalid milestone program magic".into()));
    }
    let wire = (cursor.u16()?, cursor.u16()?);
    if wire.0 != WIRE_VERSION.0 || wire.1 > WIRE_VERSION.1 {
        return Err(BinaryError(format!(
            "unsupported milestone wire version {}.{}",
            wire.0, wire.1
        )));
    }
    let version = LanguageVersion {
        major: cursor.u16()?,
        minor: cursor.u16()?,
    };
    if version.major != LANGUAGE_VERSION.0
        || version.minor > LANGUAGE_VERSION.1
        || version.minor != wire.1
    {
        return Err(BinaryError("unsupported milestone language version".into()));
    }
    let definition_count = usize::from(cursor.u16()?);
    if definition_count == 0 || definition_count > MAX_DEFINITIONS {
        return Err(BinaryError("invalid milestone definition count".into()));
    }
    if cursor.u16()? != 0 {
        return Err(BinaryError("nonzero milestone header reservation".into()));
    }
    let payload_len = cursor.u32()? as usize;
    let expected_program_digest = cursor.array32()?;
    if payload_len != cursor.remaining() {
        return Err(BinaryError("milestone payload length mismatch".into()));
    }
    let actual_program_digest = program_digest(bytes);
    if expected_program_digest != actual_program_digest {
        return Err(BinaryError("milestone program digest mismatch".into()));
    }

    let mut definitions = Vec::with_capacity(definition_count);
    let mut identities = Vec::with_capacity(definition_count);
    for _ in 0..definition_count {
        let record_len = cursor.u32()? as usize;
        if record_len < RECORD_FIXED_BYTES || record_len > cursor.remaining() {
            return Err(BinaryError("invalid milestone record length".into()));
        }
        let record_bytes = cursor.take(record_len)?;
        let (definition, identity) = decode_definition(record_bytes)?;
        definitions.push(definition);
        identities.push(identity);
    }
    if cursor.remaining() != 0 {
        return Err(BinaryError("trailing milestone program data".into()));
    }
    let program = MilestoneProgram {
        version,
        definitions,
    };
    validate_program(&program).map_err(BinaryError)?;
    let canonical = compile(&program)?;
    if canonical.bytes != bytes {
        return Err(BinaryError(
            "noncanonical milestone program encoding".into(),
        ));
    }
    Ok(DecodedMilestones {
        program,
        program_sha256: actual_program_digest,
        definitions: identities,
    })
}

pub(super) fn decode_definition(
    bytes: &[u8],
) -> Result<(MilestoneDefinition, CompiledDefinitionIdentity), BinaryError> {
    let mut cursor = Cursor::new(bytes);
    let name_len = usize::from(cursor.u16()?);
    if name_len == 0 || name_len > MAX_NAME_BYTES {
        return Err(BinaryError("invalid milestone name length".into()));
    }
    let name = cursor.string(name_len)?;
    validate_text(&name, MAX_NAME_BYTES, false).map_err(BinaryError)?;
    let phase = match cursor.u8()? {
        0 => EvaluationPhase::PreInput,
        1 => EvaluationPhase::PostSim,
        _ => return Err(BinaryError("invalid milestone evaluation phase".into())),
    };
    if cursor.u8()? != 0 {
        return Err(BinaryError("nonzero milestone record reservation".into()));
    }
    let stable_ticks = cursor.u16()?;
    if stable_ticks == 0 {
        return Err(BinaryError("zero milestone stable count".into()));
    }
    let operation_count = cursor.u16()?;
    if operation_count == 0 || usize::from(operation_count) > MAX_OPS {
        return Err(BinaryError("invalid milestone operation count".into()));
    }
    let bytecode_len = cursor.u32()? as usize;
    let expected_digest = cursor.array32()?;
    if bytecode_len != cursor.remaining() {
        return Err(BinaryError("milestone bytecode length mismatch".into()));
    }
    let bytecode = cursor.take(bytecode_len)?;
    let identity_bytes = definition_identity_bytes(
        name.as_bytes(),
        phase,
        stable_ticks,
        operation_count,
        bytecode,
    )?;
    let actual_digest: [u8; 32] = Sha256::new()
        .chain_update(DEFINITION_DOMAIN)
        .chain_update(identity_bytes)
        .finalize()
        .into();
    if actual_digest != expected_digest {
        return Err(BinaryError("milestone definition digest mismatch".into()));
    }
    let (when, then, within_ticks, projections) = decode_expression(bytecode, operation_count)?;
    Ok((
        MilestoneDefinition {
            name: name.clone(),
            phase,
            stable_ticks,
            when,
            then,
            within_ticks,
            projections,
        },
        CompiledDefinitionIdentity {
            name,
            sha256: actual_digest,
        },
    ))
}

#[derive(Clone, Debug)]
pub(super) enum StackItem {
    Field(Field),
    Query(QueryFact),
    Value(Value),
    Expression(Expression),
}

pub(super) type DecodedExpression = (
    Expression,
    Vec<Expression>,
    Option<u16>,
    Vec<ValueProjection>,
);

pub(super) fn decode_expression(
    bytes: &[u8],
    operation_count: u16,
) -> Result<DecodedExpression, BinaryError> {
    let mut cursor = Cursor::new(bytes);
    let mut stack = Vec::new();
    let mut sequence_within = None;
    let mut expected_steps = 0_usize;
    let mut sequence_steps = Vec::new();
    let mut projections = Vec::new();
    let mut current_projection: Option<ValueProjection> = None;
    let mut projection_items_remaining = 0_usize;
    let mut metadata_started = false;
    for operation_index in 0..operation_count {
        let opcode = cursor.u8()?;
        if metadata_started && !(0x50..=0x53).contains(&opcode) {
            return Err(BinaryError(
                "predicate opcodes may not follow value projections".into(),
            ));
        }
        match opcode {
            0x40 => {
                if operation_index != 0 || sequence_within.is_some() || !stack.is_empty() {
                    return Err(BinaryError("invalid sequence start opcode".into()));
                }
                let within = cursor.u16()?;
                expected_steps = usize::from(cursor.u8()?);
                if within == 0 || !(2..=16).contains(&expected_steps) {
                    return Err(BinaryError("invalid bounded sequence descriptor".into()));
                }
                sequence_within = Some(within);
            }
            0x41 => {
                if sequence_within.is_none() || sequence_steps.len() == expected_steps {
                    return Err(BinaryError("unexpected sequence step terminator".into()));
                }
                let step = pop_expression(&mut stack, "sequence step")?;
                if !stack.is_empty() {
                    return Err(BinaryError(
                        "sequence step does not yield exactly one boolean".into(),
                    ));
                }
                sequence_steps.push(step);
            }
            0x50 => {
                let expression_complete = if sequence_within.is_some() {
                    stack.is_empty() && sequence_steps.len() == expected_steps
                } else {
                    matches!(stack.as_slice(), [StackItem::Expression(_)])
                };
                if !expression_complete
                    || current_projection.is_some()
                    || projections.len() == MAX_PROJECTIONS
                {
                    return Err(BinaryError("invalid value projection start".into()));
                }
                metadata_started = true;
                let name_len = usize::from(cursor.u8()?);
                if name_len == 0 || name_len > MAX_NAME_BYTES {
                    return Err(BinaryError("invalid projection name length".into()));
                }
                let name = cursor.string(name_len)?;
                validate_text(&name, MAX_NAME_BYTES, false).map_err(BinaryError)?;
                projection_items_remaining = usize::from(cursor.u8()?);
                if projection_items_remaining == 0
                    || projection_items_remaining > MAX_PROJECTION_ITEMS
                {
                    return Err(BinaryError("invalid projection item count".into()));
                }
                current_projection = Some(ValueProjection {
                    name,
                    items: Vec::with_capacity(projection_items_remaining),
                });
            }
            0x51..=0x53 => {
                if !metadata_started || projection_items_remaining == 0 {
                    return Err(BinaryError(
                        "projection item has no active projection".into(),
                    ));
                }
                let item = match opcode {
                    0x51 => ValueProjectionItem::Rng {
                        stream: match cursor.u8()? {
                            0 => RngStream::Primary,
                            1 => RngStream::Secondary,
                            _ => return Err(BinaryError("invalid projected RNG stream".into())),
                        },
                    },
                    0x52 => {
                        let stage_bytes = cursor.take(8)?;
                        let stage_len = stage_bytes
                            .iter()
                            .position(|byte| *byte == 0)
                            .unwrap_or(stage_bytes.len());
                        if stage_bytes[stage_len..].iter().any(|byte| *byte != 0) {
                            return Err(BinaryError("noncanonical projected stage padding".into()));
                        }
                        ValueProjectionItem::ActorPopulation {
                            stage: String::from_utf8(stage_bytes[..stage_len].to_vec())
                                .map_err(|_| BinaryError("invalid projected stage".into()))?,
                            room: cursor.u8()? as i8,
                        }
                    }
                    _ => ValueProjectionItem::Flag {
                        selector: FlagSelector {
                            domain: FlagDomain::from_id(cursor.u8()?).ok_or_else(|| {
                                BinaryError("invalid projected flag domain".into())
                            })?,
                            room: cursor.u8()? as i8,
                            index: cursor.u16()?,
                        },
                    },
                };
                current_projection.as_mut().unwrap().items.push(item);
                projection_items_remaining -= 1;
                if projection_items_remaining == 0 {
                    projections.push(current_projection.take().unwrap());
                }
            }
            0x01 => {
                let id = cursor.u8()?;
                stack.push(StackItem::Field(Field::from_id(id).ok_or_else(|| {
                    BinaryError(format!("unknown milestone field ID {id}"))
                })?));
            }
            0x02 => {
                let kind = cursor.u8()?;
                let fact = match kind {
                    1 => {
                        let field_id = cursor.u8()?;
                        let field = ActorFact::from_id(field_id).ok_or_else(|| {
                            BinaryError(format!("unknown actor fact ID {field_id}"))
                        })?;
                        let stage_bytes = cursor.take(8)?;
                        let stage_len = stage_bytes
                            .iter()
                            .position(|byte| *byte == 0)
                            .unwrap_or(stage_bytes.len());
                        if stage_bytes[stage_len..].iter().any(|byte| *byte != 0) {
                            return Err(BinaryError(
                                "noncanonical placed-actor stage padding".into(),
                            ));
                        }
                        let stage = String::from_utf8(stage_bytes[..stage_len].to_vec())
                            .map_err(|_| BinaryError("invalid placed-actor stage".into()))?;
                        QueryFact::PlacedActor {
                            selector: PlacedActorSelector {
                                stage,
                                home_room: cursor.u8()? as i8,
                                set_id: cursor.u16()?,
                                actor_name: cursor.i16()?,
                            },
                            field,
                        }
                    }
                    2 => QueryFact::Flag {
                        selector: FlagSelector {
                            domain: FlagDomain::from_id(cursor.u8()?)
                                .ok_or_else(|| BinaryError("unknown flag fact domain".into()))?,
                            room: cursor.u8()? as i8,
                            index: cursor.u16()?,
                        },
                    },
                    3 | 4 => {
                        let mut values = [0.0_f32; 6];
                        for value in &mut values {
                            *value = f32::from_bits(cursor.u32()?);
                        }
                        if kind == 3 {
                            QueryFact::PlayerInAabb {
                                minimum: values[..3].try_into().unwrap(),
                                maximum: values[3..].try_into().unwrap(),
                            }
                        } else {
                            QueryFact::PlayerPlaneSignedDistance {
                                point: values[..3].try_into().unwrap(),
                                normal: values[3..].try_into().unwrap(),
                            }
                        }
                    }
                    _ => return Err(BinaryError(format!("unknown query fact kind {kind}"))),
                };
                validate_query_fact(&fact).map_err(BinaryError)?;
                stack.push(StackItem::Query(fact));
            }
            0x10 => match cursor.u8()? {
                0 => stack.push(StackItem::Value(Value::Bool(false))),
                1 => stack.push(StackItem::Value(Value::Bool(true))),
                _ => return Err(BinaryError("noncanonical boolean constant".into())),
            },
            0x11 => stack.push(StackItem::Value(Value::U32(cursor.u32()?))),
            0x12 => stack.push(StackItem::Value(Value::U64(cursor.u64()?))),
            0x13 => stack.push(StackItem::Value(Value::I32(cursor.i32()?))),
            0x14 => {
                let value = f32::from_bits(cursor.u32()?);
                if !value.is_finite() || value.to_bits() != canonical_float(value).to_bits() {
                    return Err(BinaryError("noncanonical floating-point constant".into()));
                }
                stack.push(StackItem::Value(Value::F32(value)));
            }
            0x15 => stack.push(StackItem::Value(Value::Symbol(cursor.symbol()?))),
            0x16 => stack.push(StackItem::Value(Value::ProcedureNumber(cursor.u32()?))),
            0x17 => stack.push(StackItem::Value(Value::ProcedureSymbol(cursor.symbol()?))),
            0x20..=0x27 => {
                let operator = match opcode {
                    0x20 => Comparison::Equal,
                    0x21 => Comparison::NotEqual,
                    0x22 => Comparison::Less,
                    0x23 => Comparison::LessEqual,
                    0x24 => Comparison::Greater,
                    0x25 => Comparison::GreaterEqual,
                    0x26 => Comparison::HasAll,
                    _ => Comparison::HasAny,
                };
                let value = match stack.pop() {
                    Some(StackItem::Value(value)) => value,
                    _ => return Err(BinaryError("comparison requires a literal value".into())),
                };
                let expression = match stack.pop() {
                    Some(StackItem::Field(field)) => {
                        validate_comparison(field, operator, &value).map_err(BinaryError)?;
                        Expression::Compare {
                            field,
                            operator,
                            value,
                        }
                    }
                    Some(StackItem::Query(fact)) => {
                        validate_query_comparison(&fact, operator, &value).map_err(BinaryError)?;
                        Expression::Query {
                            fact,
                            operator,
                            value,
                        }
                    }
                    _ => return Err(BinaryError("comparison requires a field or query".into())),
                };
                stack.push(StackItem::Expression(expression));
            }
            0x30 => {
                let inner = pop_expression(&mut stack, "not")?;
                stack.push(StackItem::Expression(Expression::Not(Box::new(inner))));
            }
            0x31 | 0x32 => {
                let right = pop_expression(&mut stack, "boolean operator")?;
                let left = pop_expression(&mut stack, "boolean operator")?;
                stack.push(StackItem::Expression(if opcode == 0x31 {
                    Expression::And(Box::new(left), Box::new(right))
                } else {
                    Expression::Or(Box::new(left), Box::new(right))
                }));
            }
            _ => {
                return Err(BinaryError(format!(
                    "unknown milestone opcode 0x{opcode:02x}"
                )));
            }
        }
        if stack.len() > MAX_OPS {
            return Err(BinaryError("milestone expression stack overflow".into()));
        }
    }
    if cursor.remaining() != 0 {
        return Err(BinaryError("trailing milestone bytecode".into()));
    }
    if current_projection.is_some() {
        return Err(BinaryError("incomplete value projection".into()));
    }
    if let Some(within) = sequence_within {
        if !stack.is_empty() || sequence_steps.len() != expected_steps {
            return Err(BinaryError(
                "bounded sequence does not contain its declared steps".into(),
            ));
        }
        let mut steps = sequence_steps.into_iter();
        let when = steps.next().unwrap();
        return Ok((when, steps.collect(), Some(within), projections));
    }
    if stack.len() != 1 {
        return Err(BinaryError(
            "milestone bytecode does not yield one boolean".into(),
        ));
    }
    Ok((
        pop_expression(&mut stack, "program result")?,
        Vec::new(),
        None,
        projections,
    ))
}

pub(super) fn pop_expression(
    stack: &mut Vec<StackItem>,
    context: &str,
) -> Result<Expression, BinaryError> {
    match stack.pop() {
        Some(StackItem::Expression(expression)) => Ok(expression),
        _ => Err(BinaryError(format!(
            "{context} requires a boolean expression"
        ))),
    }
}

pub(super) struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], BinaryError> {
        let end = self
            .at
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| BinaryError("truncated milestone program".into()))?;
        let value = &self.bytes[self.at..end];
        self.at = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, BinaryError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, BinaryError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn i16(&mut self) -> Result<i16, BinaryError> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, BinaryError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, BinaryError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, BinaryError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn array32(&mut self) -> Result<[u8; 32], BinaryError> {
        Ok(self.take(32)?.try_into().unwrap())
    }

    fn string(&mut self, length: usize) -> Result<String, BinaryError> {
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| BinaryError("invalid UTF-8 in milestone program".into()))
    }

    fn symbol(&mut self) -> Result<String, BinaryError> {
        let length = usize::from(self.u8()?);
        if length == 0 || length > MAX_SYMBOL_BYTES {
            return Err(BinaryError("invalid milestone symbol length".into()));
        }
        self.string(length)
    }
}
