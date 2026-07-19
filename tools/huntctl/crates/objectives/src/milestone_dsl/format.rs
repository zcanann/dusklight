use super::*;
use std::format;

/// Canonically format an AST for source control or visual-editor export.
pub fn format(program: &MilestoneProgram) -> Result<String, BinaryError> {
    validate_program(program).map_err(BinaryError)?;
    let mut output = format!(
        "milestones {}.{}\n\n",
        program.version.major, program.version.minor
    );
    for (index, definition) in program.definitions.iter().enumerate() {
        output.push_str("milestone ");
        output.push_str(&quoted(&definition.name));
        output.push_str(" {\n  phase ");
        output.push_str(match definition.phase {
            EvaluationPhase::PreInput => "pre_input",
            EvaluationPhase::PostSim => "post_sim",
        });
        output.push('\n');
        if definition.stable_ticks != 1 {
            output.push_str(&format!("  stable {}\n", definition.stable_ticks));
        }
        if let Some(within_ticks) = definition.within_ticks {
            output.push_str(&format!("  within {within_ticks}\n"));
        }
        output.push_str("  when ");
        format_expression(&definition.when, 0, &mut output);
        for step in &definition.then {
            output.push_str("\n  then ");
            format_expression(step, 0, &mut output);
        }
        for projection in &definition.projections {
            output.push_str("\n  projection ");
            output.push_str(&quoted(&projection.name));
            output.push_str(" {");
            for item in &projection.items {
                output.push_str("\n    ");
                match item {
                    ValueProjectionItem::Rng { stream } => {
                        output.push_str("rng ");
                        output.push_str(match stream {
                            RngStream::Primary => "primary",
                            RngStream::Secondary => "secondary",
                        });
                    }
                    ValueProjectionItem::ActorPopulation { stage, room } => {
                        output.push_str("actor_population ");
                        output.push_str(&quoted(stage));
                        output.push(' ');
                        output.push_str(&room.to_string());
                    }
                    ValueProjectionItem::Flag { selector } => {
                        output.push_str("flag ");
                        output.push_str(match selector.domain {
                            FlagDomain::Event => "event ",
                            FlagDomain::Temporary => "temporary ",
                            FlagDomain::Dungeon => "dungeon ",
                            FlagDomain::Switch => "switch ",
                        });
                        if selector.domain == FlagDomain::Switch {
                            output.push_str(&selector.room.to_string());
                            output.push(' ');
                        }
                        output.push_str(&selector.index.to_string());
                    }
                }
            }
            output.push_str("\n  }");
        }
        output.push_str("\n}");
        if index + 1 != program.definitions.len() {
            output.push_str("\n\n");
        } else {
            output.push('\n');
        }
    }
    Ok(output)
}

fn format_expression(expression: &Expression, parent_precedence: u8, output: &mut String) {
    let precedence = match expression {
        Expression::Or(..) => 1,
        Expression::And(..) => 2,
        Expression::Not(..) => 3,
        Expression::Compare { .. } | Expression::Query { .. } => 4,
    };
    let parentheses = precedence < parent_precedence;
    if parentheses {
        output.push('(');
    }
    match expression {
        Expression::Compare {
            field,
            operator,
            value,
        } => {
            output.push_str(field.path());
            output.push(' ');
            output.push_str(match operator {
                Comparison::Equal => "==",
                Comparison::NotEqual => "!=",
                Comparison::Less => "<",
                Comparison::LessEqual => "<=",
                Comparison::Greater => ">",
                Comparison::GreaterEqual => ">=",
                Comparison::HasAll => "has_all",
                Comparison::HasAny => "has_any",
            });
            output.push(' ');
            format_value(value, output);
        }
        Expression::Query {
            fact,
            operator,
            value,
        } => {
            format_query_fact(fact, output);
            output.push(' ');
            output.push_str(match operator {
                Comparison::Equal => "==",
                Comparison::NotEqual => "!=",
                Comparison::Less => "<",
                Comparison::LessEqual => "<=",
                Comparison::Greater => ">",
                Comparison::GreaterEqual => ">=",
                Comparison::HasAll => "has_all",
                Comparison::HasAny => "has_any",
            });
            output.push(' ');
            format_value(value, output);
        }
        Expression::Not(inner) => {
            output.push('!');
            format_expression(inner, precedence, output);
        }
        Expression::And(left, right) => {
            format_expression(left, precedence, output);
            output.push_str(" && ");
            format_expression(right, precedence + 1, output);
        }
        Expression::Or(left, right) => {
            format_expression(left, precedence, output);
            output.push_str(" || ");
            format_expression(right, precedence + 1, output);
        }
    }
    if parentheses {
        output.push(')');
    }
}

fn format_query_fact(fact: &QueryFact, output: &mut String) {
    match fact {
        QueryFact::PlacedActor { selector, field } => {
            output.push_str(field.path());
            output.push('(');
            output.push_str(&quoted(&selector.stage));
            output.push_str(", ");
            output.push_str(&selector.home_room.to_string());
            output.push_str(", ");
            output.push_str(&selector.set_id.to_string());
            output.push_str(", ");
            output.push_str(&selector.actor_name.to_string());
            output.push(')');
        }
        QueryFact::Flag { selector } => {
            output.push_str(selector.domain.path());
            output.push('(');
            if selector.domain == FlagDomain::Switch {
                output.push_str(&selector.room.to_string());
                output.push_str(", ");
            }
            output.push_str(&selector.index.to_string());
            output.push(')');
        }
        QueryFact::PlayerInAabb { minimum, maximum } => {
            output.push_str("player.in_aabb(");
            format_f32_arguments(minimum.iter().chain(maximum), output);
            output.push(')');
        }
        QueryFact::PlayerPlaneSignedDistance { point, normal } => {
            output.push_str("player.plane_signed_distance(");
            format_f32_arguments(point.iter().chain(normal), output);
            output.push(')');
        }
    }
}

fn format_f32_arguments<'a>(values: impl Iterator<Item = &'a f32>, output: &mut String) {
    for (index, value) in values.enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        format_value(&Value::F32(*value), output);
    }
}

fn format_value(value: &Value, output: &mut String) {
    match value {
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::U32(value) | Value::ProcedureNumber(value) => output.push_str(&value.to_string()),
        Value::U64(value) => output.push_str(&value.to_string()),
        Value::I32(value) => output.push_str(&value.to_string()),
        Value::F32(value) => {
            let mut rendered = value.to_string();
            if !rendered.contains(['.', 'e', 'E']) {
                rendered.push_str(".0");
            }
            output.push_str(&rendered);
        }
        Value::Symbol(value) | Value::ProcedureSymbol(value) => {
            output.push_str(&quoted(value));
        }
    }
}

fn quoted(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}
