use dusklight_evaluation::*;
use dusklight_search::bayesian_search::{
    BayesianConfig, BayesianObservation, BayesianOptimizer, BayesianProposal,
};
use dusklight_search::continuous_search::{
    ContinuousOptimizer, ContinuousOptimizerConfig, ContinuousSample, ContinuousTemplate,
};
use dusklight_search::search::{
    Ancestry, Candidate, EvolutionConfig, InterventionRange, LexicographicScore, SearchResults,
    evolve_population, rank_population, tape_intervention, write_explicit_population,
    write_seed_population,
};
use dusklight_learning::planning_priors::option_catalog_sha256;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

fn directory_is_nonempty(path: &Path) -> Result<bool, EvaluateError> {
    Ok(path.is_dir() && fs::read_dir(path)?.next().transpose()?.is_some())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), EvaluateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

pub fn run_search(config: &SearchRunConfig) -> Result<SearchRunSummary, EvaluateError> {
    if config.generations == 0
        || config.population_size == 0
        || config.elite_count == 0
        || config.elite_count > config.population_size
    {
        return Err(EvaluateError::InvalidConfig(
            "generations, population size, and elites must be valid and nonzero".into(),
        ));
    }
    if !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "game, DVD, working directory, and a new/empty output root are required".into(),
        ));
    }
    fs::create_dir_all(&config.output_root)?;
    let seed_candidate = config
        .seed_candidate
        .clone()
        .unwrap_or_else(|| Candidate::baseline(config.segment));
    if seed_candidate.segment != config.segment {
        return Err(EvaluateError::InvalidConfig(
            "seed candidate segment does not match the search segment".into(),
        ));
    }
    seed_candidate.validate()?;
    let mut population_root = config.output_root.join("g000");
    let mut manifest = write_seed_population(
        &population_root,
        seed_candidate,
        config.population_size,
        config.rng_seed,
    )?;
    let mut final_results = None;
    for generation in 0..config.generations {
        let manifest_path = population_root.join("manifest.json");
        let results_path = population_root.join("results.json");
        evaluate_population(&EvaluateConfig {
            population_path: manifest_path.clone(),
            game: config.game.clone(),
            dvd: config.dvd.clone(),
            output_root: population_root.join("evaluations"),
            results_path: results_path.clone(),
            working_directory: config.working_directory.clone(),
            game_args_prefix: config.game_args_prefix.clone(),
            workers: config.workers,
            repetitions: config.repetitions,
            timeout: config.timeout,
            harness: config.harness.clone(),
        })?;
        let results: SearchResults = serde_json::from_slice(&fs::read(&results_path)?)?;
        let leaderboard = rank_population(&manifest, &results)?;
        write_json(&population_root.join("leaderboard.json"), &leaderboard)?;
        final_results = Some(results);
        if generation + 1 < config.generations {
            let next_root = config.output_root.join(format!("g{:03}", generation + 1));
            manifest = evolve_population(
                &manifest_path,
                final_results.as_ref().unwrap(),
                &next_root,
                EvolutionConfig {
                    population_size: config.population_size,
                    elite_count: config.elite_count,
                    rng_seed: config.rng_seed + u64::from(generation) + 1,
                },
            )?;
            population_root = next_root;
        }
    }
    let results = final_results.expect("nonzero generations");
    let leaderboard = rank_population(&manifest, &results)?;
    let champion = leaderboard.first().ok_or(EvaluateError::EmptyLeaderboard)?;
    let member = manifest
        .members
        .iter()
        .find(|member| member.candidate_id == champion.candidate_id)
        .ok_or(EvaluateError::EmptyLeaderboard)?;
    let source = population_root.join(&member.tape_file);
    let champion_tape = config.output_root.join("champion.tape");
    fs::copy(source, &champion_tape)?;
    let champion_candidate = config.output_root.join("champion.candidate.json");
    fs::copy(
        population_root.join(&member.candidate_file),
        &champion_candidate,
    )?;
    let summary = SearchRunSummary {
        schema: SEARCH_RUN_SCHEMA,
        segment: config.segment,
        generations: config.generations,
        population_size: config.population_size,
        repetitions: config.repetitions,
        rng_seed: config.rng_seed,
        champion_id: champion.candidate_id.clone(),
        champion_candidate,
        champion_tape,
        score: champion.score,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("run.summary.json"), &summary)?;
    Ok(summary)
}

/// Beam search over a finite discrete option catalog. Every score comes from
/// the ordinary native evaluator. Branch-and-bound prunes only descendants of
/// a prefix whose terminal goal was already proved: appending inputs after its
/// first hit cannot improve that hit and can only make the tape larger.
pub fn run_beam_search(config: &BeamSearchConfig) -> Result<BeamSearchSummary, EvaluateError> {
    if config.seed_candidate.segment != config.segment
        || config.options.is_empty()
        || config.options.len() > 128
        || config.beam_width == 0
        || config.beam_width > 256
        || config.maximum_depth == 0
        || config.maximum_depth > 32
        || config.candidate_budget == 0
        || config.candidate_budget > 100_000
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "beam search requires a matching seed, 1..=128 options, bounded positive beam/depth/budget, native inputs, at least two repetitions, and a new output root"
                .into(),
        ));
    }
    config.seed_candidate.validate()?;
    let option_catalog_sha256 = option_catalog_sha256(&config.options)
        .map_err(|error| EvaluateError::InvalidConfig(error.to_string()))?;
    if let Some(priors) = &config.q_priors {
        priors
            .validate_for_catalog(option_catalog_sha256, config.options.len())
            .map_err(|error| EvaluateError::InvalidConfig(error.to_string()))?;
    }
    fs::create_dir_all(&config.output_root)?;
    let mut seen = HashSet::new();
    seen.insert(config.seed_candidate.id()?);
    let mut batch = vec![config.seed_candidate.clone()];
    let mut evaluated = 0_usize;
    let mut duplicate_proposals = 0_usize;
    let mut beam_pruned_prefixes = 0_usize;
    let mut terminal_bound_pruned_children = 0_usize;
    let mut q_prior_ranked_children = 0_usize;
    let mut depths_evaluated = 0_u32;
    let mut champion: Option<(LexicographicScore, String, Candidate)> = None;

    for depth in 0..=config.maximum_depth {
        if batch.is_empty() || evaluated >= config.candidate_budget {
            break;
        }
        let remaining = config.candidate_budget - evaluated;
        batch.truncate(remaining);
        let depth_root = config.output_root.join(format!("d{depth:03}"));
        let manifest =
            write_explicit_population(&depth_root, config.segment, depth, batch.clone())?;
        let manifest_path = depth_root.join("manifest.json");
        let results_path = depth_root.join("results.json");
        evaluate_population(&EvaluateConfig {
            population_path: manifest_path,
            game: config.game.clone(),
            dvd: config.dvd.clone(),
            output_root: depth_root.join("evaluations"),
            results_path: results_path.clone(),
            working_directory: config.working_directory.clone(),
            game_args_prefix: config.game_args_prefix.clone(),
            workers: config.workers,
            repetitions: config.repetitions,
            timeout: config.timeout,
            harness: None,
        })?;
        let results: SearchResults = serde_json::from_slice(&fs::read(&results_path)?)?;
        let leaderboard = rank_population(&manifest, &results)?;
        write_json(&depth_root.join("leaderboard.json"), &leaderboard)?;
        evaluated += batch.len();
        depths_evaluated += 1;
        let candidates = batch
            .drain(..)
            .map(|candidate| Ok((candidate.id()?, candidate)))
            .collect::<Result<BTreeMap<_, _>, EvaluateError>>()?;
        for row in &leaderboard {
            let candidate = candidates
                .get(&row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            if champion.as_ref().is_none_or(|(score, prior_id, _)| {
                row.score > *score || (row.score == *score && row.candidate_id < *prior_id)
            }) {
                champion = Some((row.score, row.candidate_id.clone(), candidate.clone()));
            }
        }
        if depth == config.maximum_depth || evaluated >= config.candidate_budget {
            break;
        }

        let mut frontier = Vec::new();
        for row in &leaderboard {
            let result = results
                .candidates
                .get(&row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            if result.goal_reached == Some(true) {
                terminal_bound_pruned_children =
                    terminal_bound_pruned_children.saturating_add(config.options.len());
                continue;
            }
            if frontier.len() < config.beam_width {
                frontier.push(
                    candidates
                        .get(&row.candidate_id)
                        .ok_or(EvaluateError::EmptyLeaderboard)?
                        .clone(),
                );
            } else {
                beam_pruned_prefixes += 1;
            }
        }
        let mut next = Vec::new();
        'parents: for parent in frontier {
            let parent_id = parent.id()?;
            let parent_frames = parent.frame_count();
            let option_indices = config.q_priors.as_ref().map_or_else(
                || (0..config.options.len()).collect::<Vec<_>>(),
                |priors| priors.ranked_option_indices(&parent_id, config.options.len()),
            );
            for option_index in option_indices {
                if evaluated + next.len() >= config.candidate_budget {
                    break 'parents;
                }
                let option = &config.options[option_index];
                if config
                    .q_priors
                    .as_ref()
                    .is_some_and(|priors| priors.has_prior(&parent_id, option_index))
                {
                    q_prior_ranked_children += 1;
                }
                let mut child = parent.clone();
                child.actions.push(option.clone());
                let child_frames = child.frame_count();
                child.ancestry = Ancestry {
                    generation: depth + 1,
                    parent_id: Some(parent_id.clone()),
                    mutation: Some(format!("beam discrete option {option_index}")),
                    intervention: Some(InterventionRange {
                        start_frame: parent_frames,
                        end_frame_exclusive: child_frames,
                        parent_end_frame_exclusive: parent_frames,
                    }),
                };
                if child.validate().is_err() {
                    continue;
                }
                let id = child.id()?;
                if seen.insert(id) {
                    next.push(child);
                } else {
                    duplicate_proposals += 1;
                }
            }
        }
        batch = next;
    }

    let (champion_score, champion_id, champion) =
        champion.ok_or(EvaluateError::EmptyLeaderboard)?;
    let champion_candidate = config.output_root.join("champion.candidate.json");
    let champion_tape = config.output_root.join("champion.tape");
    fs::write(&champion_candidate, serde_json::to_vec_pretty(&champion)?)?;
    fs::write(&champion_tape, champion.compile()?.encode()?)?;
    let summary = BeamSearchSummary {
        schema: "dusklight-beam-search/v2",
        segment: config.segment,
        beam_width: config.beam_width,
        maximum_depth: config.maximum_depth,
        candidate_budget: config.candidate_budget,
        evaluated_candidates: evaluated,
        simulator_episodes: evaluated.saturating_mul(config.repetitions as usize),
        duplicate_proposals,
        beam_pruned_prefixes,
        terminal_bound_pruned_children,
        q_prior_table_sha256: config.q_priors.as_ref().map(|priors| priors.table_sha256),
        q_prior_model_sha256: config.q_priors.as_ref().map(|priors| priors.model_sha256),
        q_prior_ranked_children,
        q_prior_role: "supported_child_ordering_only",
        native_rollout_ranking_authority: true,
        policy_owns_route: false,
        depths_evaluated,
        champion_id,
        champion_score,
        champion_candidate,
        champion_tape,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("beam.summary.json"), &summary)?;
    Ok(summary)
}

/// Runs seeded CEM or full-covariance CMA-ES while keeping native repeated
/// rollout evidence as the only ranking signal.
pub fn run_continuous_search(
    config: &ContinuousSearchRunConfig,
) -> Result<ContinuousSearchRunSummary, EvaluateError> {
    if config.generations == 0
        || config.generations > 1_000
        || config.candidate_budget == 0
        || config.candidate_budget > 100_000
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "continuous search requires bounded generations/budget, native inputs, at least two repetitions, and a new output root"
                .into(),
        ));
    }
    let template = ContinuousTemplate::new(config.seed_candidate.clone(), config.axes.clone())?;
    let mut optimizer = ContinuousOptimizer::new(
        template.clone(),
        ContinuousOptimizerConfig {
            method: config.method,
            population_size: config.population_size,
            elite_count: config.elite_count,
            initial_sigma: config.initial_sigma,
            seed: config.rng_seed,
        },
    )?;
    fs::create_dir_all(&config.output_root)?;
    let seed_id = config.seed_candidate.id()?;
    let seed_tape = config.seed_candidate.compile()?;
    let mut seen = HashSet::new();
    let mut evaluated = 0_usize;
    let mut duplicate_proposals = 0_usize;
    let mut invalid_proposals = 0_usize;
    let mut generations_completed = 0_u32;
    let mut champion: Option<(LexicographicScore, String, Candidate, Vec<f64>)> = None;

    for generation in 0..config.generations {
        if evaluated >= config.candidate_budget {
            break;
        }
        let samples = optimizer.ask()?;
        let mut sample_by_candidate = BTreeMap::<String, ContinuousSample>::new();
        let mut candidates = Vec::new();
        for sample in samples {
            if evaluated + candidates.len() >= config.candidate_budget {
                break;
            }
            let Ok(mut candidate) = template.candidate(&sample.values) else {
                invalid_proposals += 1;
                continue;
            };
            let tape = candidate.compile()?;
            let Some(intervention) = tape_intervention(&seed_tape, &tape) else {
                duplicate_proposals += 1;
                continue;
            };
            candidate.ancestry = Ancestry {
                generation,
                parent_id: Some(seed_id.clone()),
                mutation: Some(format!("{:?} bounded continuous sample", config.method)),
                intervention: Some(intervention),
            };
            let id = candidate.id()?;
            if !seen.insert(id.clone()) {
                duplicate_proposals += 1;
                continue;
            }
            sample_by_candidate.insert(id, sample);
            candidates.push(candidate);
        }
        if candidates.len() < config.elite_count {
            if generations_completed == 0 {
                return Err(EvaluateError::InvalidConfig(format!(
                    "continuous bounds produced only {} unique valid candidates; at least {} are required",
                    candidates.len(),
                    config.elite_count
                )));
            }
            break;
        }
        let generation_root = config.output_root.join(format!("g{generation:03}"));
        let manifest = write_explicit_population(
            &generation_root,
            config.seed_candidate.segment,
            generation,
            candidates,
        )?;
        let results_path = generation_root.join("results.json");
        evaluate_population(&EvaluateConfig {
            population_path: generation_root.join("manifest.json"),
            game: config.game.clone(),
            dvd: config.dvd.clone(),
            output_root: generation_root.join("evaluations"),
            results_path: results_path.clone(),
            working_directory: config.working_directory.clone(),
            game_args_prefix: config.game_args_prefix.clone(),
            workers: config.workers,
            repetitions: config.repetitions,
            timeout: config.timeout,
            harness: None,
        })?;
        let results: SearchResults = serde_json::from_slice(&fs::read(results_path)?)?;
        let leaderboard = rank_population(&manifest, &results)?;
        write_json(&generation_root.join("leaderboard.json"), &leaderboard)?;
        let ranked_samples = leaderboard
            .iter()
            .map(|row| {
                sample_by_candidate
                    .get(&row.candidate_id)
                    .cloned()
                    .ok_or(EvaluateError::EmptyLeaderboard)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for row in &leaderboard {
            let sample = sample_by_candidate
                .get(&row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            let member = manifest
                .members
                .iter()
                .find(|member| member.candidate_id == row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(generation_root.join(&member.candidate_file))?)?;
            if champion.as_ref().is_none_or(|(score, id, _, _)| {
                row.score > *score || (row.score == *score && row.candidate_id < *id)
            }) {
                champion = Some((
                    row.score,
                    row.candidate_id.clone(),
                    candidate,
                    sample.values.clone(),
                ));
            }
        }
        optimizer.tell(&ranked_samples)?;
        write_json(
            &generation_root.join("optimizer.json"),
            &serde_json::json!({
                "schema": "dusklight-continuous-generation/v1",
                "method": config.method,
                "axes": config.axes,
                "ranked_samples": ranked_samples,
                "next_state": optimizer.snapshot(),
            }),
        )?;
        evaluated += manifest.members.len();
        generations_completed += 1;
    }

    let (champion_score, champion_id, champion, champion_values) =
        champion.ok_or(EvaluateError::EmptyLeaderboard)?;
    let champion_candidate = config.output_root.join("champion.candidate.json");
    let champion_tape = config.output_root.join("champion.tape");
    fs::write(&champion_candidate, serde_json::to_vec_pretty(&champion)?)?;
    fs::write(&champion_tape, champion.compile()?.encode()?)?;
    let summary = ContinuousSearchRunSummary {
        schema: "dusklight-continuous-search/v1",
        method: config.method,
        segment: config.seed_candidate.segment,
        generations_requested: config.generations,
        generations_completed,
        population_size: config.population_size,
        elite_count: config.elite_count,
        candidate_budget: config.candidate_budget,
        evaluated_candidates: evaluated,
        simulator_episodes: evaluated.saturating_mul(config.repetitions as usize),
        duplicate_proposals,
        invalid_proposals,
        rng_seed: config.rng_seed,
        final_optimizer: optimizer.snapshot(),
        champion_id,
        champion_score,
        champion_values,
        champion_candidate,
        champion_tape,
        output_root: config.output_root.clone(),
    };
    write_json(
        &config.output_root.join("continuous.summary.json"),
        &summary,
    )?;
    Ok(summary)
}

/// Bounded Gaussian-process expected-improvement search. The surrogate models
/// empirical native rank utility only; final ordering and proof remain native.
pub fn run_bayesian_search(
    config: &BayesianSearchRunConfig,
) -> Result<BayesianSearchRunSummary, EvaluateError> {
    if config.generations == 0
        || config.generations > 1_000
        || config.batch_size == 0
        || config.batch_size > 512
        || config.candidate_budget == 0
        || config.candidate_budget > 100_000
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "Bayesian search requires bounded batches/generations/budget, native inputs, at least two repetitions, and a new output root"
                .into(),
        ));
    }
    let template = ContinuousTemplate::new(config.seed_candidate.clone(), config.axes.clone())?;
    let mut optimizer = BayesianOptimizer::new(BayesianConfig {
        dimensions: template.dimensions(),
        initial_samples: config.initial_samples,
        acquisition_pool: config.acquisition_pool,
        length_scale: config.length_scale,
        observation_noise: config.observation_noise,
        exploration: config.exploration,
        seed: config.rng_seed,
    })?;
    fs::create_dir_all(&config.output_root)?;
    let seed_id = config.seed_candidate.id()?;
    let seed_tape = config.seed_candidate.compile()?;
    let mut seen = HashSet::new();
    let mut evaluated = 0_usize;
    let mut duplicate_proposals = 0_usize;
    let mut invalid_proposals = 0_usize;
    let mut generations_completed = 0_u32;
    let mut champion: Option<(LexicographicScore, String, Candidate, Vec<f64>)> = None;

    for generation in 0..config.generations {
        if evaluated >= config.candidate_budget {
            break;
        }
        let request = config
            .batch_size
            .min(config.candidate_budget.saturating_sub(evaluated));
        let proposals = optimizer.ask(request)?;
        let mut proposal_by_candidate = BTreeMap::<String, (BayesianProposal, Vec<f64>)>::new();
        let mut candidates = Vec::new();
        for proposal in proposals {
            let values = template.values_from_normalized(&proposal.normalized)?;
            let Ok(mut candidate) = template.candidate(&values) else {
                invalid_proposals += 1;
                continue;
            };
            let tape = candidate.compile()?;
            let Some(intervention) = tape_intervention(&seed_tape, &tape) else {
                duplicate_proposals += 1;
                continue;
            };
            candidate.ancestry = Ancestry {
                generation,
                parent_id: Some(seed_id.clone()),
                mutation: Some("Gaussian-process expected-improvement proposal".into()),
                intervention: Some(intervention),
            };
            let id = candidate.id()?;
            if !seen.insert(id.clone()) {
                duplicate_proposals += 1;
                continue;
            }
            proposal_by_candidate.insert(id, (proposal, values));
            candidates.push(candidate);
        }
        if candidates.is_empty() {
            if generations_completed == 0 {
                return Err(EvaluateError::InvalidConfig(
                    "Bayesian bounds produced no unique valid candidates".into(),
                ));
            }
            break;
        }
        let generation_root = config.output_root.join(format!("g{generation:03}"));
        let manifest = write_explicit_population(
            &generation_root,
            config.seed_candidate.segment,
            generation,
            candidates,
        )?;
        let results_path = generation_root.join("results.json");
        evaluate_population(&EvaluateConfig {
            population_path: generation_root.join("manifest.json"),
            game: config.game.clone(),
            dvd: config.dvd.clone(),
            output_root: generation_root.join("evaluations"),
            results_path: results_path.clone(),
            working_directory: config.working_directory.clone(),
            game_args_prefix: config.game_args_prefix.clone(),
            workers: config.workers,
            repetitions: config.repetitions,
            timeout: config.timeout,
            harness: None,
        })?;
        let results: SearchResults = serde_json::from_slice(&fs::read(results_path)?)?;
        let leaderboard = rank_population(&manifest, &results)?;
        write_json(&generation_root.join("leaderboard.json"), &leaderboard)?;
        let denominator = leaderboard.len().saturating_sub(1).max(1) as f64;
        let mut observations = Vec::with_capacity(leaderboard.len());
        for (rank, row) in leaderboard.iter().enumerate() {
            let (proposal, values) = proposal_by_candidate
                .get(&row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            let utility = if leaderboard.len() == 1 {
                1.0
            } else {
                (leaderboard.len() - rank - 1) as f64 / denominator
            };
            observations.push(BayesianObservation {
                normalized: proposal.normalized.clone(),
                rank_utility: utility,
            });
            let member = manifest
                .members
                .iter()
                .find(|member| member.candidate_id == row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(generation_root.join(&member.candidate_file))?)?;
            if champion.as_ref().is_none_or(|(score, id, _, _)| {
                row.score > *score || (row.score == *score && row.candidate_id < *id)
            }) {
                champion = Some((
                    row.score,
                    row.candidate_id.clone(),
                    candidate,
                    values.clone(),
                ));
            }
        }
        optimizer.tell(observations.clone())?;
        write_json(
            &generation_root.join("optimizer.json"),
            &serde_json::json!({
                "schema": "dusklight-bayesian-generation/v1",
                "axes": config.axes,
                "proposals": proposal_by_candidate,
                "rank_observations": observations,
                "next_state": optimizer.snapshot(),
            }),
        )?;
        evaluated += manifest.members.len();
        generations_completed += 1;
    }

    let (champion_score, champion_id, champion, champion_values) =
        champion.ok_or(EvaluateError::EmptyLeaderboard)?;
    let champion_candidate = config.output_root.join("champion.candidate.json");
    let champion_tape = config.output_root.join("champion.tape");
    fs::write(&champion_candidate, serde_json::to_vec_pretty(&champion)?)?;
    fs::write(&champion_tape, champion.compile()?.encode()?)?;
    let summary = BayesianSearchRunSummary {
        schema: "dusklight-bayesian-search/v1",
        segment: config.seed_candidate.segment,
        generations_requested: config.generations,
        generations_completed,
        batch_size: config.batch_size,
        candidate_budget: config.candidate_budget,
        evaluated_candidates: evaluated,
        simulator_episodes: evaluated.saturating_mul(config.repetitions as usize),
        duplicate_proposals,
        invalid_proposals,
        rng_seed: config.rng_seed,
        final_optimizer: optimizer.snapshot(),
        champion_id,
        champion_score,
        champion_values,
        champion_candidate,
        champion_tape,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("bayesian.summary.json"), &summary)?;
    Ok(summary)
}
