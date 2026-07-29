use super::*;

pub(super) fn command_suffix(command: &str, args: &[String]) -> Result<(), Box<dyn Error>> {
    match command {
        "evaluate-suffix-batch" => {
            let search_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(search_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request: OptimizationRequest =
                serde_json::from_slice(&fs::read(required_path(search_args, "--request")?)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(required_path(search_args, "--execution")?)?)?;
            execution.validate_files(&repository_root, &request)?;

            let input_tape = required_path(search_args, "--input-tape")?;
            let input_tape = if input_tape.is_absolute() {
                input_tape
            } else {
                repository_root.join(input_tape)
            };
            let batch = required_path(search_args, "--batch")?;
            let batch = if batch.is_absolute() {
                batch
            } else {
                repository_root.join(batch)
            };
            let output = required_path(search_args, "--output")?;
            let output = if output.is_absolute() {
                output
            } else {
                repository_root.join(output)
            };
            if output.exists() && fs::read_dir(&output)?.next().is_some() {
                return Err(format!(
                    "suffix-batch evaluation output must be new or empty: {}",
                    output.display()
                )
                .into());
            }
            fs::create_dir_all(&output)?;

            let terminal = NativeTerminalBinding {
                goal: request.terminal_predicate.goal.clone(),
                program_sha256: request.terminal_predicate.program_sha256,
                definition_sha256: request.terminal_predicate.definition_sha256,
            };
            let launch = NativeSuffixWorkerLaunch {
                executable: repository_root.join(&execution.executable.path),
                game_data: repository_root.join(&execution.game_data.path),
                input_tape,
                milestone_program: repository_root.join(&execution.milestone_program.path),
                card_fixture: execution.card_fixture_root(&repository_root, &request)?,
                card_fixture_sha256: execution.card_fixture_manifest.sha256,
                working_directory: repository_root.clone(),
                state_root: output.join("native-state"),
                world_context_sha256: execution.world_context.sha256,
                terminal,
                initial_batch: batch,
                initial_result: output.join("result.json"),
                initial_winner_tape: Some(output.join("winner.tape")),
            };
            let (worker, validated) = NativeSuffixWorkerSession::launch_with_prevalidated_files(
                &launch,
                NativeSuffixPrevalidatedFileIdentities {
                    executable_sha256: execution.executable.sha256,
                    game_data_sha256: execution.game_data.sha256,
                },
            )?;
            worker.shutdown()?;
            let successful_candidates = validated
                .candidates
                .iter()
                .filter(|candidate| candidate.first_hit_tick.is_some())
                .count();
            let best_first_hit_tick = validated
                .candidates
                .iter()
                .filter_map(|candidate| candidate.first_hit_tick)
                .min();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "huntctl-evaluate-suffix-batch/v1",
                    "result": output.join("result.json"),
                    "winner_tape": output.join("winner.tape"),
                    "candidate_count": validated.candidates.len(),
                    "successful_candidates": successful_candidates,
                    "best_first_hit_tick": best_first_hit_tick,
                    "simulated_ticks": validated.simulated_ticks,
                    "checkpoint_bytes": validated.checkpoint_bytes,
                    "restore_identity": validated.restore_identity,
                }))?
            );
            Ok(())
        }
        "suffix-select" => {
            let search_args = &args[1..];
            let candidate_path = required_path(search_args, "--candidate")?;
            let batch_path = required_path(search_args, "--batch")?;
            let selected_id = option(search_args, "--id").ok_or("missing required --id")?;
            let output = required_path(search_args, "--output")?;
            let seed: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            let batch: NativeSuffixBatch = serde_json::from_slice(&fs::read(batch_path)?)?;
            let selected = batch
                .candidates
                .iter()
                .find(|candidate| candidate.id == selected_id)
                .ok_or("selected suffix candidate is absent from its batch")?;
            let candidate = Candidate {
                schema: huntctl::search::CANDIDATE_SCHEMA.into(),
                segment: seed.segment,
                boot: seed.boot.clone(),
                actions: selected.actions.clone(),
                ancestry: huntctl::search::Ancestry {
                    generation: seed.ancestry.generation.saturating_add(1),
                    parent_id: Some(seed.id()?),
                    mutation: Some(format!("selected native suffix {selected_id}")),
                    intervention: None,
                },
            };
            candidate.validate()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&candidate)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!("selected {selected_id} into {}", output.display());
            Ok(())
        }
        "candidate-to-tape" => {
            let search_args = &args[1..];
            let input = required_path(search_args, "--input")?;
            let output = required_path(search_args, "--output")?;
            let candidate: Candidate = serde_json::from_slice(&fs::read(input)?)?;
            let tape = candidate.compile()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, tape.encode()?)?;
            println!(
                "wrote {} candidate frames to {}",
                tape.frames.len(),
                output.display()
            );
            Ok(())
        }
        "suffix-promote-failure" => {
            let search_args = &args[1..];
            let candidate_path = required_path(search_args, "--candidate")?;
            let batch_path = required_path(search_args, "--batch")?;
            let results_path = required_path(search_args, "--results")?;
            let output = required_path(search_args, "--output")?;
            let seed: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            let batch: NativeSuffixBatch = serde_json::from_slice(&fs::read(batch_path)?)?;
            let results: Value = serde_json::from_slice(&fs::read(results_path)?)?;
            if results.get("status").and_then(Value::as_str) != Some("passed") {
                return Err("failure promotion requires a passed native batch result".into());
            }
            let (winner_id, distance) = results
                .get("candidates")
                .and_then(Value::as_array)
                .ok_or("native batch result has no candidates")?
                .iter()
                .filter(|result| result.get("success").and_then(Value::as_bool) == Some(false))
                .filter_map(|result| {
                    let id = result.get("id")?.as_str()?;
                    let position = result
                        .pointer("/terminal_observation/position")?
                        .as_array()?;
                    let x = position.first()?.as_f64()?;
                    let z = position.get(2)?.as_f64()?;
                    (x.is_finite() && z.is_finite()).then_some((id, ordon_exit_edge_distance(x, z)))
                })
                .min_by(|left, right| left.1.total_cmp(&right.1).then_with(|| left.0.cmp(right.0)))
                .ok_or("native batch result has no finite failed terminal observations")?;
            let selected = batch
                .candidates
                .iter()
                .find(|candidate| candidate.id == winner_id)
                .ok_or("best failed result is absent from its batch")?;
            let promoted = Candidate {
                schema: huntctl::search::CANDIDATE_SCHEMA.into(),
                segment: seed.segment,
                boot: seed.boot.clone(),
                actions: selected.actions.clone(),
                ancestry: huntctl::search::Ancestry {
                    generation: seed.ancestry.generation.saturating_add(1),
                    parent_id: Some(seed.id()?),
                    mutation: Some(format!(
                        "native failure promotion {winner_id}; exit-edge distance {distance:.6}"
                    )),
                    intervention: None,
                },
            };
            promoted.validate()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&promoted)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!(
                "promoted failed candidate {winner_id} at signed exit-edge distance {distance:.6} to {}",
                output.display()
            );
            Ok(())
        }
        "suffix-refine" => {
            let search_args = &args[1..];
            let candidate_path = required_path(search_args, "--candidate")?;
            let batch_path = required_path(search_args, "--batch")?;
            let results_path = required_path(search_args, "--results")?;
            let output = required_path(search_args, "--output")?;
            let candidate: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            let batch: NativeSuffixBatch = serde_json::from_slice(&fs::read(batch_path)?)?;
            let results: Value = serde_json::from_slice(&fs::read(results_path)?)?;
            if results.get("status").and_then(Value::as_str) != Some("passed") {
                return Err(
                    "ranked suffix refinement requires a passed native batch result".into(),
                );
            }
            let terminal_observations = results
                .get("candidates")
                .and_then(Value::as_array)
                .ok_or("native batch result has no candidates")?
                .iter()
                .filter(|result| result.get("success").and_then(Value::as_bool) == Some(false))
                .map(|result| {
                    let id = result
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("native batch candidate has no id")?;
                    let position = result
                        .pointer("/terminal_observation/position")
                        .and_then(Value::as_array)
                        .ok_or("native batch candidate has no terminal position")?;
                    let x = position
                        .first()
                        .and_then(Value::as_f64)
                        .ok_or("native batch candidate terminal x is absent or non-finite")?;
                    let z = position
                        .get(2)
                        .and_then(Value::as_f64)
                        .ok_or("native batch candidate terminal z is absent or non-finite")?;
                    if !x.is_finite() || !z.is_finite() {
                        return Err("native batch candidate terminal position is non-finite");
                    }
                    Ok((id.to_owned(), x, z))
                })
                .collect::<Result<Vec<_>, &str>>()?;
            let refined = propose_ranked_suffix_refinement(
                &candidate,
                &batch,
                &terminal_observations,
                usize_option(search_args, "--candidate-budget", 107)?,
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&refined)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!(
                "wrote {} progress-ranked suffix refinements to {}",
                refined.candidates.len(),
                output.display()
            );
            Ok(())
        }
        "suffix-batch" => {
            let search_args = &args[1..];
            let candidate_path = required_path(search_args, "--candidate")?;
            let output = required_path(search_args, "--output")?;
            let candidate: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            let method = match option(search_args, "--method")
                .ok_or(
                    "missing required --method deletion|delete-hold|button-edge|heading|corner|corner-wide|collision|fine-heading|fine-terminal|lane-shift|fine-lane-shift|early-lane-shift|magnitude|asymmetric-lane-shift|post-collision|recovery-bias|timing|path|terminal",
                )?
                .as_str()
            {
                "deletion" => SuffixProposalMethod::Deletion,
                "delete-hold" => SuffixProposalMethod::DeleteHold,
                "button-edge" => SuffixProposalMethod::ButtonEdge,
                "heading" => SuffixProposalMethod::Heading,
                "corner" => SuffixProposalMethod::Corner,
                "corner-wide" => SuffixProposalMethod::CornerWide,
                "collision" => SuffixProposalMethod::Collision,
                "fine-heading" => SuffixProposalMethod::FineHeading,
                "fine-terminal" => SuffixProposalMethod::FineTerminal,
                "lane-shift" => SuffixProposalMethod::LaneShift,
                "fine-lane-shift" => SuffixProposalMethod::FineLaneShift,
                "early-lane-shift" => SuffixProposalMethod::EarlyLaneShift,
                "magnitude" => SuffixProposalMethod::Magnitude,
                "asymmetric-lane-shift" => SuffixProposalMethod::AsymmetricLaneShift,
                "post-collision" => SuffixProposalMethod::PostCollision,
                "recovery-bias" => SuffixProposalMethod::RecoveryBias,
                "timing" => SuffixProposalMethod::Timing,
                "path" => SuffixProposalMethod::Path,
                "terminal" => SuffixProposalMethod::Terminal,
                value => return Err(format!("unknown suffix proposal method {value:?}").into()),
            };
            let batch = propose_suffix_batch(
                &candidate,
                usize_option(search_args, "--source-frame", 440)?,
                &option(search_args, "--source-boundary-fingerprint")
                    .ok_or("missing required --source-boundary-fingerprint VALUE")?,
                usize_option(search_args, "--maximum-ticks", 125)?,
                usize_option(search_args, "--candidate-budget", 126)?,
                method,
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&batch)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!(
                "wrote {} native suffix candidates ({} ticks each) to {}",
                batch.candidates.len(),
                batch.maximum_ticks,
                output.display()
            );
            Ok(())
        }
        "candidate-from-tape" => {
            if args.len() < 2 {
                return usage_error();
            }
            let search_args = &args[1..];
            let input = required_path(search_args, "--input")?;
            let output = required_path(search_args, "--output")?;
            let segment = option(search_args, "--segment")
                .ok_or("missing required --segment PROFILE")?
                .parse::<SegmentProfile>()?;
            let start = usize_option(search_args, "--start", 0)?;
            let decoded = InputTape::decode(&fs::read(&input)?)?;
            let available = decoded
                .tape
                .frames
                .len()
                .checked_sub(start)
                .ok_or("candidate tape start exceeds input length")?;
            let frames = usize_option(search_args, "--frames", available)?;
            let end = start
                .checked_add(frames)
                .ok_or("candidate tape range overflows")?;
            if frames == 0 || end > decoded.tape.frames.len() {
                return Err("candidate tape range must be nonempty and inside the input".into());
            }
            let mut tape = InputTape {
                frames: decoded.tape.frames[start..end].to_vec(),
                ..decoded.tape
            };
            if flag(search_args, "--normalize-port-one") {
                let disconnected = huntctl::tape::RawPadState {
                    connected: false,
                    error: -1,
                    ..huntctl::tape::RawPadState::default()
                };
                for frame in &mut tape.frames {
                    frame.owned_ports = 0x01;
                    frame.pads[1..].fill(disconnected);
                }
            }
            let candidate = if flag(search_args, "--semantic-motion-paths") {
                Candidate::from_semantic_movement_tape(segment, &tape)?
            } else {
                Candidate::from_absolute_tape(segment, &tape)?
            };
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, serde_json::to_vec_pretty(&candidate)?)?;
            println!(
                "wrote {} frames as {} actions to {} (port-one-normalized: {}, semantic-motion-paths: {})",
                candidate.frame_count(),
                candidate.actions.len(),
                output.display(),
                flag(search_args, "--normalize-port-one"),
                flag(search_args, "--semantic-motion-paths")
            );
            Ok(())
        }
        _ => usage_error(),
    }
}
