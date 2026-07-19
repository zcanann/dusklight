//! Read-only gameplay-trace inspection and comparison adapters.

use crate::{option, usage_error};
use serde_json::{Value, json};
use std::error::Error;
use std::fs;

pub(crate) fn command_trace(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("inspect") if args.len() == 2 => {
            let summary = huntctl::trace::decode_and_summarize(&fs::read(&args[1])?)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("facts") if args.len() == 2 || args.len() == 4 => {
            if args.len() == 4 && args[2] != "--boundary-index" {
                return usage_error();
            }
            let decoded = huntctl::trace::decode(&fs::read(&args[1])?)?;
            let requested_boundary = option(&args[2..], "--boundary-index")
                .map(|value| value.parse::<u64>())
                .transpose()?;
            let facts = decoded
                .records
                .iter()
                .filter(|record| {
                    requested_boundary.is_none_or(|index| record.boundary_index == index)
                })
                .map(huntctl::trace_typed_facts::typed_facts_from_trace_record)
                .collect::<Vec<_>>();
            if requested_boundary.is_some() && facts.is_empty() {
                return Err("trace has no record at the requested boundary index".into());
            }
            for response in &facts {
                response.validate()?;
            }
            println!("{}", serde_json::to_string_pretty(&facts)?);
            Ok(())
        }
        Some("state-hashes") if args.len() == 2 || args.len() == 4 => {
            if args.len() == 4 && args[2] != "--output" {
                return usage_error();
            }
            let trace_bytes = fs::read(&args[1])?;
            let decoded = huntctl::trace::decode(&trace_bytes)?;
            let series = huntctl::semantic_state_hash::SemanticStateHashSeries::build(
                &decoded,
                &trace_bytes,
            )?;
            let output = serde_json::to_vec_pretty(&series)?;
            if let Some(path) = option(&args[2..], "--output") {
                fs::write(path, &output)?;
            } else {
                println!("{}", String::from_utf8(output)?);
            }
            Ok(())
        }
        Some("compare-state") if args.len() == 3 => {
            let left_bytes = fs::read(&args[1])?;
            let right_bytes = fs::read(&args[2])?;
            let left = huntctl::semantic_state_hash::SemanticStateHashSeries::build(
                &huntctl::trace::decode(&left_bytes)?,
                &left_bytes,
            )?;
            let right = huntctl::semantic_state_hash::SemanticStateHashSeries::build(
                &huntctl::trace::decode(&right_bytes)?,
                &right_bytes,
            )?;
            let comparison =
                huntctl::semantic_state_hash::SemanticStateHashComparison::compare(&left, &right)?;
            println!("{}", serde_json::to_string_pretty(&comparison)?);
            Ok(())
        }
        Some("timeline") if args.len() == 2 => {
            let decoded = huntctl::trace::decode(&fs::read(&args[1])?)?;
            let mut prior: Option<&huntctl::trace::TraceRecord> = None;
            let records: Vec<_> = decoded
                .records
                .iter()
                .filter(|record| {
                    let changed = prior.is_none_or(|previous| {
                        record.stage_name != previous.stage_name
                            || record.room != previous.room
                            || record.layer != previous.layer
                            || record.point != previous.point
                            || record.player_present() != previous.player_present()
                            || record.player_is_link() != previous.player_is_link()
                            || record.event_running() != previous.event_running()
                            || record.event_id != previous.event_id
                            || record.event_status != previous.event_status
                            || record.player_proc_id != previous.player_proc_id
                    });
                    let input = record.buttons != 0 || record.stick_x != 0 || record.stick_y != 0;
                    prior = Some(record);
                    changed || input
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&records)?);
            Ok(())
        }
        Some("compare") if args.len() >= 3 => {
            let mut rows: Vec<Value> = args[1..]
                .iter()
                .map(|path| {
                    let summary = huntctl::trace::decode_and_summarize(&fs::read(path)?)?;
                    let milestone_count = [
                        summary.first_playable.is_some(),
                        summary.route_control.is_some(),
                        summary.first_loading_trigger.is_some(),
                        summary.first_loading_transition.is_some(),
                        summary.post_load_playable.is_some(),
                        summary.first_post_load_event.is_some(),
                        summary.intro_cutscene.is_some(),
                    ]
                    .into_iter()
                    .filter(|reached| *reached)
                    .count();
                    let score_tick = summary
                        .intro_cutscene
                        .as_ref()
                        .or(summary.first_post_load_event.as_ref())
                        .or(summary.post_load_playable.as_ref())
                        .or(summary.first_loading_transition.as_ref())
                        .or(summary.first_loading_trigger.as_ref())
                        .or(summary.route_control.as_ref())
                        .or(summary.first_playable.as_ref())
                        .map(|milestone| milestone.simulation_tick)
                        .unwrap_or(u64::MAX);
                    Ok::<_, Box<dyn Error>>(json!({
                        "path": path,
                        "milestones_reached": milestone_count,
                        "score_tick": score_tick,
                        "summary": summary,
                    }))
                })
                .collect::<Result<_, _>>()?;
            rows.sort_by(|left, right| {
                let left_count = left["milestones_reached"].as_u64().unwrap();
                let right_count = right["milestones_reached"].as_u64().unwrap();
                right_count.cmp(&left_count).then_with(|| {
                    left["score_tick"]
                        .as_u64()
                        .unwrap()
                        .cmp(&right["score_tick"].as_u64().unwrap())
                })
            });
            println!("{}", serde_json::to_string_pretty(&rows)?);
            Ok(())
        }
        _ => usage_error(),
    }
}
