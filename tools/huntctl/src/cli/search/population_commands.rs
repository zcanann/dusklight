use super::*;

pub(super) fn command_population(command: &str, args: &[String]) -> Result<(), Box<dyn Error>> {
    match command {
        "import-tape" => {
            let search_args = &args[1..];
            let segment: SegmentProfile = option(search_args, "--segment")
                .ok_or("missing required --segment ID")?
                .parse()?;
            let tape_path = required_path(search_args, "--tape")?;
            let output = required_path(search_args, "--output")?;
            let tape = InputTape::decode(&fs::read(&tape_path)?)?.tape;
            let candidate = Candidate::from_absolute_tape(segment, &tape)?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, serde_json::to_vec_pretty(&candidate)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "candidate_id": candidate.id()?,
                    "candidate": output,
                    "source_tape": tape_path,
                    "frames": candidate.frame_count(),
                    "lossless": candidate.compile()? == tape,
                }))?
            );
            Ok(())
        }
        "seed" => {
            let search_args = &args[1..];
            let segment: SegmentProfile = option(search_args, "--segment")
                .ok_or("missing required --segment ID")?
                .parse()?;
            let output = required_path(search_args, "--output")?;
            let size = usize_option(search_args, "--size", 16)?;
            let rng_seed = u64_option(search_args, "--rng-seed", 1)?;
            let candidate = if let Some(path) = option(search_args, "--candidate") {
                let candidate: Candidate = serde_json::from_slice(&fs::read(path)?)?;
                if candidate.segment != segment {
                    return Err("candidate segment does not match --segment".into());
                }
                candidate
            } else {
                Candidate::baseline(segment)
            };
            let manifest = write_seed_population(&output, candidate, size, rng_seed)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        "evolve" => {
            let search_args = &args[1..];
            let population = required_path(search_args, "--population")?;
            let results: SearchResults =
                serde_json::from_slice(&fs::read(required_path(search_args, "--results")?)?)?;
            let output = required_path(search_args, "--output")?;
            let size = usize_option(search_args, "--size", 16)?;
            let elites = usize_option(search_args, "--elites", (size / 4).max(1))?;
            let rng_seed = u64_option(search_args, "--rng-seed", 1)?;
            let manifest = evolve_population(
                &population,
                &results,
                &output,
                EvolutionConfig {
                    population_size: size,
                    elite_count: elites,
                    rng_seed,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        "rank" => {
            let search_args = &args[1..];
            let manifest: PopulationManifest =
                serde_json::from_slice(&fs::read(required_path(search_args, "--population")?)?)?;
            let results: SearchResults =
                serde_json::from_slice(&fs::read(required_path(search_args, "--results")?)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&rank_population(&manifest, &results)?)?
            );
            Ok(())
        }
        "collect" => {
            let search_args = &args[1..];
            let manifest: PopulationManifest =
                serde_json::from_slice(&fs::read(required_path(search_args, "--population")?)?)?;
            let inputs = repeated_option(search_args, "--input");
            if inputs.is_empty() {
                return Err("search collect requires at least one --input FILE".into());
            }
            let artifacts = inputs
                .iter()
                .map(|path| serde_json::from_slice(&fs::read(path)?).map_err(Into::into))
                .collect::<Result<Vec<EvaluationArtifact>, Box<dyn Error>>>()?;
            let results = collect_results(&manifest, artifacts)?;
            let output = required_path(search_args, "--output")?;
            fs::write(&output, serde_json::to_vec_pretty(&results)?)?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(())
        }
        "inspect" if args.len() == 2 => {
            let candidate: Candidate = serde_json::from_slice(&fs::read(&args[1])?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "candidate_id": candidate.id()?,
                    "segment": candidate.segment,
                    "target": candidate.segment.target(),
                    "target_depth": candidate.segment.target_depth(),
                    "action_count": candidate.actions.len(),
                    "frame_count": candidate.frame_count(),
                    "ancestry": candidate.ancestry,
                }))?
            );
            Ok(())
        }
        "mock-evaluate" => {
            let search_args = &args[1..];
            let population_path = required_path(search_args, "--population")?;
            let output = required_path(search_args, "--output")?;
            let attempts = u32::try_from(usize_option(search_args, "--attempts", 3)?)?;
            if attempts == 0 {
                return Err("--attempts must be greater than zero".into());
            }
            let manifest: PopulationManifest = serde_json::from_slice(&fs::read(population_path)?)?;
            let candidates = manifest
                .members
                .iter()
                .map(|member| {
                    (
                        member.candidate_id.clone(),
                        CandidateResult {
                            goal_reached: Some(true),
                            milestone_depth: manifest.segment.target_depth(),
                            attempts,
                            successes: attempts,
                            first_hit_ticks: vec![member.frame_count; attempts as usize],
                            risk_events: None,
                            boundary_compatibility: huntctl::search::BoundaryCompatibility::Unknown,
                        },
                    )
                })
                .collect();
            let results = SearchResults {
                schema: RESULTS_SCHEMA.into(),
                segment: manifest.segment,
                boot: manifest.boot,
                candidates,
            };
            fs::write(&output, serde_json::to_vec_pretty(&results)?)?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(())
        }
        _ => usage_error(),
    }
}
