//! Multi-generation anchored route policy over authenticated native evaluation.

use crate::search_drivers::SearchRunConfig;
use dusklight_evaluation::harness_authority::validate_anchored_harness_request;
use dusklight_automation_contracts::artifact::Digest as ArtifactDigest;
use dusklight_automation_contracts::candidate_envelope::{
    CandidateEnvelope, CandidateEnvelopeSet, NamedDigest, ProposerIdentity, ProposerKind,
};
use dusklight_automation_contracts::tape::InputTape;
use dusklight_evaluation::*;
use dusklight_evidence::transition_corpus::TransitionCorpus;
use dusklight_harness_contracts::run_contract::HarnessTerminalReason;
use dusklight_learning::evaluation_isolation::{
    EvaluationAttemptInput, EvaluationGenerationSeal, EvaluationOutcomeCollection,
    EvaluationOutcomeInput,
};
use dusklight_learning::offline_rl::movement_action_schema_digest_v2;
use dusklight_learning::online_lineage::{OnlineDatasetGeneration, OnlineModelLineage};
use dusklight_proposals::behavior_archive::{
    BehaviorArchive, BehaviorContext, describe_behavior_with_context,
};
use dusklight_proposals::q_search::{
    QEpisode, QProposalConfig, QProposalReadinessEvidence, propose_q_candidates_with_lineage,
};
use dusklight_search::search::{
    Candidate, EvolutionConfig, LexicographicScore, PopulationManifest, SegmentProfile,
    evolve_population_with_retained_and_proposals, rank_population, write_seed_population,
};
use dusklight_semantic_novelty::catalog::{
    SemanticNoveltyCatalog, SemanticNoveltyCatalogConfig,
};
use dusklight_semantic_novelty::proposal_signal::{
    SemanticNoveltyProposalSignal, SemanticNoveltyProposalSignalConfig,
};
use dusklight_semantic_novelty::SemanticNoveltyDescriptor;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const ANCHORED_RUN_SCHEMA: &str = "dusklight-anchored-search-run/v2";

#[derive(Clone, Debug)]
pub struct AnchoredSearchRunConfig {
    pub search: SearchRunConfig,
    pub objective: AnchoredObjectiveConfig,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnchoredSearchRunSummary {
    pub schema: &'static str,
    pub segment: SegmentProfile,
    pub objective: AnchoredObjectiveIdentity,
    pub generations: u32,
    pub population_size: usize,
    pub repetitions: u32,
    pub rng_seed: u64,
    pub champion_id: String,
    pub champion_candidate: PathBuf,
    pub champion_suffix_tape: PathBuf,
    pub champion_tape: PathBuf,
    pub score: LexicographicScore,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
struct CandidateSemanticNoveltyAssessment {
    candidate_id: String,
    assessment: dusklight_semantic_novelty::catalog::SemanticNoveltyAssessment,
    proposal_signal: SemanticNoveltyProposalSignal,
}

#[derive(Clone, Debug, Serialize)]
struct SemanticNoveltyGenerationReport {
    schema: &'static str,
    generation: u32,
    baseline_observed_episodes: u64,
    candidates: Vec<CandidateSemanticNoveltyAssessment>,
}

fn is_anchored_profile(profile: SegmentProfile) -> bool {
    matches!(
        profile,
        SegmentProfile::Fsp103ToFsp104 | SegmentProfile::LinkControlToTunnelCrawlStart
    )
}

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

fn validate_anchored_execution(
    objective: &AnchoredObjectiveConfig,
    search: &SearchRunConfig,
) -> Result<(), EvaluateError> {
    if !search.game_args_prefix.is_empty() {
        return Err(EvaluateError::InvalidConfig(
            "anchored evaluation rejects game_args_prefix so CVars, timing, stage, and proof inputs cannot diverge from its execution contract".into(),
        ));
    }
    if fs::canonicalize(&objective.game)? != fs::canonicalize(&search.game)?
        || fs::canonicalize(&objective.dvd)? != fs::canonicalize(&search.dvd)?
    {
        return Err(EvaluateError::InvalidConfig(
            "anchored objective game/DVD paths do not match the launched execution paths".into(),
        ));
    }
    Ok(())
}

pub fn run_anchored_search(
    config: &AnchoredSearchRunConfig,
) -> Result<AnchoredSearchRunSummary, EvaluateError> {
    let search = &config.search;
    if !is_anchored_profile(search.segment) {
        return Err(EvaluateError::InvalidConfig(format!(
            "anchored search requires a movement segment, got {}",
            search.segment.as_str()
        )));
    }
    if config.objective.segment != search.segment {
        return Err(EvaluateError::InvalidConfig(
            "anchored search segment does not match its objective".into(),
        ));
    }
    if search.generations == 0
        || search.population_size == 0
        || search.elite_count == 0
        || search.elite_count > search.population_size
        || !search.game.is_file()
        || !search.dvd.is_file()
        || !search.working_directory.is_dir()
        || directory_is_nonempty(&search.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "valid execution paths, population limits, and a new/empty output root are required"
                .into(),
        ));
    }
    let seed = search.seed_candidate.clone().ok_or_else(|| {
        EvaluateError::InvalidConfig(
            "anchored search requires a losslessly imported observed suffix candidate; it has no synthetic baseline"
                .into(),
        )
    })?;
    if seed.segment != search.segment {
        return Err(EvaluateError::InvalidConfig(
            "anchored seed candidate has the wrong segment profile".into(),
        ));
    }
    seed.validate()?;
    let prepared = prepare_anchored_evaluator(&config.objective)?;
    validate_anchored_execution(&config.objective, search)?;
    validate_anchored_harness_request(
        search.harness.as_ref(),
        prepared.identity(),
        "anchored route search",
    )?;
    fs::create_dir_all(&search.output_root)?;
    let mut population_root = search.output_root.join("g000");
    let mut manifest = write_seed_population(
        &population_root,
        seed,
        search.population_size,
        search.rng_seed,
    )?;
    write_initial_proposal_envelopes(
        &manifest,
        &population_root,
        NamedDigest::new(
            prepared.identity().goal_milestone.clone(),
            prepared.identity().digest.parse().map_err(|error| {
                EvaluateError::InvalidResult(format!(
                    "invalid anchored objective digest: {error}"
                ))
            })?,
        ),
        search.population_size,
    )?;
    let mut final_results = None;
    let mut training_corpora = BTreeMap::<String, TransitionCorpus>::new();
    let mut previous_dataset_generation: Option<OnlineDatasetGeneration> = None;
    let mut previous_model_lineage: Option<OnlineModelLineage> = None;
    let mut initial_learned_trial_consumed = false;
    let mut behavior_archive = BehaviorArchive::default();
    let mut semantic_novelty_catalog = SemanticNoveltyCatalog::default();
    for generation in 0..search.generations {
        let manifest_path = population_root.join("manifest.json");
        let results_path = population_root.join("results.json");
        let (report, results) = evaluate_prepared_anchored_population(
            &AnchoredEvaluateConfig {
                evaluation: EvaluateConfig {
                    population_path: manifest_path.clone(),
                    game: search.game.clone(),
                    dvd: search.dvd.clone(),
                    output_root: population_root.join("evaluations"),
                    results_path: results_path.clone(),
                    working_directory: search.working_directory.clone(),
                    game_args_prefix: search.game_args_prefix.clone(),
                    workers: search.workers,
                    repetitions: search.repetitions,
                    timeout: search.timeout,
                    harness: search.harness.clone(),
                },
                objective: config.objective.clone(),
            },
            &prepared,
        )?;
        let leaderboard = rank_population(&manifest, &results.results)?;
        write_json(&population_root.join("leaderboard.json"), &leaderboard)?;
        let mut generation_corpora = BTreeMap::new();
        let mut generation_contexts = BTreeMap::new();
        let mut generation_semantics =
            BTreeMap::<String, (u32, SemanticNoveltyDescriptor, BehaviorContext)>::new();
        let mut generation_outcomes = BTreeMap::new();
        let mut evaluation_attempts = Vec::with_capacity(report.attempts.len());
        let mut evaluation_outcomes = Vec::with_capacity(report.attempts.len());
        let mut quarantined_corpora = BTreeMap::<String, TransitionCorpus>::new();
        let mut quarantined_digests = BTreeSet::<ArtifactDigest>::new();
        for attempt in &report.attempts {
            if let Some(descriptor) = attempt_semantic_novelty_descriptor(attempt)? {
                let replace = generation_semantics
                    .get(&attempt.candidate_id)
                    .is_none_or(|(selected_attempt, _, _)| attempt.attempt < *selected_attempt);
                if replace {
                    let context = attempt_behavior_context(attempt, &descriptor);
                    generation_semantics.insert(
                        attempt.candidate_id.clone(),
                        (attempt.attempt, descriptor, context),
                    );
                }
            }
            let transition_corpus_sha256 = if let Some(path) = attempt.transition_corpus.as_ref() {
                let corpus = TransitionCorpus::read_zstd_file(path)
                    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                let digest = corpus
                    .content_digest()
                    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                quarantined_digests.insert(digest);
                quarantined_corpora
                    .entry(digest.to_string())
                    .or_insert_with(|| corpus.clone());
                generation_corpora
                    .entry(attempt.candidate_id.clone())
                    .or_insert(corpus);
                generation_outcomes
                    .entry(attempt.candidate_id.clone())
                    .or_insert(attempt.outcome.class);
                Some(digest)
            } else {
                None
            };
            evaluation_attempts.push(EvaluationAttemptInput {
                candidate_id: attempt.candidate_id.clone(),
                attempt: attempt.attempt,
                worker_id: attempt.worker_id.clone(),
                transition_corpus_sha256,
            });
            evaluation_outcomes.push(EvaluationOutcomeInput {
                candidate_id: attempt.candidate_id.clone(),
                attempt: attempt.attempt,
                outcome: attempt.outcome.class,
                milestone_depth: attempt.milestone_depth.saturating_sub(1),
                goal_reached: attempt.goal_reached,
                transition_corpus_sha256,
            });
        }
        generation_contexts.extend(
            generation_semantics
                .iter()
                .map(|(candidate_id, (_, _, context))| (candidate_id.clone(), context.clone())),
        );
        let baseline_observed_episodes = semantic_novelty_catalog.observed_episodes();
        let candidates = generation_semantics
            .iter()
            .map(|(candidate_id, (_, descriptor, _))| {
                let assessment = semantic_novelty_catalog
                    .assess(descriptor, SemanticNoveltyCatalogConfig::default())
                    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                let proposal_signal = SemanticNoveltyProposalSignal::from_assessment(
                    assessment.clone(),
                    SemanticNoveltyProposalSignalConfig::default(),
                )
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                Ok::<_, EvaluateError>(CandidateSemanticNoveltyAssessment {
                    candidate_id: candidate_id.clone(),
                    assessment,
                    proposal_signal,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let semantic_proposal_scores = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.candidate_id.clone(),
                    candidate.proposal_signal.proposal_ordering_score(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        write_json(
            &population_root.join("semantic-novelty.json"),
            &SemanticNoveltyGenerationReport {
                schema: "dusklight-semantic-novelty-generation/v1",
                generation,
                baseline_observed_episodes,
                candidates,
            },
        )?;
        for (_, descriptor, _) in generation_semantics.values() {
            semantic_novelty_catalog
                .record(descriptor)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        }
        write_json(
            &population_root.join("semantic-novelty-catalog.json"),
            &semantic_novelty_catalog.snapshot(),
        )?;
        let evaluation_seal = EvaluationGenerationSeal::build(
            generation,
            report.repetitions,
            report.planned_attempts,
            report.completed_attempts,
            report.infrastructure_faults,
            &evaluation_attempts,
        )
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        write_json(
            &population_root.join("evaluation-generation-seal.json"),
            &evaluation_seal,
        )?;
        let outcome_collection =
            EvaluationOutcomeCollection::build(&evaluation_seal, &evaluation_outcomes)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        write_json(
            &population_root.join("evaluation-outcomes.json"),
            &outcome_collection,
        )?;
        if generation + 1 < search.generations {
            evaluation_seal
                .admit_training_generation(generation + 1, &quarantined_digests)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            for (digest, corpus) in quarantined_corpora {
                training_corpora.entry(digest).or_insert(corpus);
            }
        }
        let member_by_id: BTreeMap<_, _> = manifest
            .members
            .iter()
            .map(|member| (member.candidate_id.as_str(), member))
            .collect();
        let mut evaluated_episodes = BTreeMap::new();
        for row in &leaderboard {
            let Some(corpus) = generation_corpora.get(&row.candidate_id) else {
                continue;
            };
            let member = member_by_id[row.candidate_id.as_str()];
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(population_root.join(&member.candidate_file))?)?;
            let episode = QEpisode {
                candidate,
                corpus: corpus.clone(),
                outcome: generation_outcomes[&row.candidate_id],
                objective: NamedDigest::new(
                    prepared.identity().goal_milestone.clone(),
                    prepared.identity().digest.parse().map_err(|error| {
                        EvaluateError::InvalidResult(format!(
                            "invalid anchored objective digest: {error}"
                        ))
                    })?,
                ),
            };
            let context = generation_contexts
                .get(&row.candidate_id)
                .cloned()
                .unwrap_or_default();
            behavior_archive
                .consider_with_context(episode.clone(), row.score, generation, &context)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            let descriptor = describe_behavior_with_context(corpus, &context)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            evaluated_episodes.insert(row.candidate_id.clone(), (episode, descriptor));
        }
        final_results = Some(results);
        if generation + 1 < search.generations {
            let next_root = search.output_root.join(format!("g{:03}", generation + 1));
            let mut q_episodes = Vec::new();
            let mut elite_ids = HashSet::new();
            let mut elite_descriptors = Vec::new();
            for row in leaderboard.iter().take(search.elite_count) {
                elite_ids.insert(row.candidate_id.clone());
                let Some((episode, descriptor)) = evaluated_episodes.get(&row.candidate_id) else {
                    continue;
                };
                elite_descriptors.push(descriptor.clone());
                q_episodes.push(episode.clone());
            }
            let non_elite_budget = search.population_size - search.elite_count;
            let archive_budget = if non_elite_budget >= 3 {
                (non_elite_budget / 4).max(1)
            } else {
                0
            };
            let archived = behavior_archive
                .select_diverse(&elite_ids, &elite_descriptors, archive_budget)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            let archive_summary = behavior_archive
                .summary(&archived)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            write_json(
                &population_root.join("behavior-archive.json"),
                &archive_summary,
            )?;
            let archived_candidates = archived
                .iter()
                .map(|entry| entry.episode.candidate.clone())
                .collect::<Vec<_>>();
            let corpora = training_corpora.values().cloned().collect::<Vec<_>>();
            let dataset_generation = if corpora.is_empty() {
                None
            } else {
                let dataset = OnlineDatasetGeneration::build(
                    previous_dataset_generation.as_ref(),
                    &evaluation_seal,
                    &corpora,
                )
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                write_json(
                    &population_root.join("online-dataset-generation.json"),
                    &dataset,
                )?;
                Some(dataset)
            };
            let mut q_ids = elite_ids;
            for entry in &archived {
                let id = entry.episode.candidate.id()?;
                if q_ids.insert(id) {
                    q_episodes.push(entry.episode.clone());
                }
            }
            q_episodes.sort_by(|left, right| {
                let left_id = left
                    .candidate
                    .id()
                    .expect("proposal source candidates were validated before archiving");
                let right_id = right
                    .candidate
                    .id()
                    .expect("proposal source candidates were validated before archiving");
                semantic_proposal_scores
                    .get(&right_id)
                    .copied()
                    .unwrap_or_default()
                    .cmp(
                        &semantic_proposal_scores
                            .get(&left_id)
                            .copied()
                            .unwrap_or_default(),
                    )
                    .then_with(|| left_id.cmp(&right_id))
            });
            let q_budget = (non_elite_budget - archived_candidates.len()).div_ceil(2);
            let initial_bounded_trial = !initial_learned_trial_consumed;
            let readiness = QProposalReadinessEvidence {
                required_facts_supported: attempts_support_required_native_facts(&report.attempts),
                determinism_proved: report.repetitions >= 2
                    && report.attempts.iter().all(|attempt| {
                        attempt.harness_terminal != Some(HarnessTerminalReason::Nondeterministic)
                    }),
                held_out_performance_adequate: !initial_bounded_trial
                    && learned_proposals_pass_holdout(&manifest, &leaderboard),
                initial_bounded_trial,
            };
            let q_result = match dataset_generation.as_ref() {
                Some(dataset_generation)
                    if outcome_collection.required_mix_complete
                        && q_budget > 0
                        && !q_episodes.is_empty() =>
                {
                    propose_q_candidates_with_lineage(
                        &corpora,
                        &q_episodes,
                        QProposalConfig {
                            generation: generation + 1,
                            max_proposals: q_budget,
                            iterations: 12,
                            trees_per_action: 15,
                            seed: search.rng_seed + u64::from(generation) + 1,
                            readiness,
                        },
                        dataset_generation,
                        previous_model_lineage.as_ref(),
                    )
                    .map_err(|error| error.to_string())
                }
                _ if !outcome_collection.required_mix_complete => Err(
                    "sealed evaluation generation has no complete success/near-miss/ordinary-failure mix"
                        .to_string(),
                ),
                _ => Err(
                    "no non-elite slots, aligned elite episodes, or sealed training generation is available"
                        .to_string(),
                ),
            };
            let q_candidates = match q_result {
                Ok(batch) => {
                    if batch.summary.proposal_gate.initial_bounded_trial
                        && batch.summary.proposal_gate.learned_policy_enabled
                        && batch
                            .summary
                            .collection_schedule
                            .iter()
                            .any(|lane| matches!(*lane, "guided_exploit" | "ensemble_disagreement"))
                    {
                        initial_learned_trial_consumed = true;
                    }
                    if let Some(lineage) = batch.summary.model_lineage.as_ref() {
                        write_json(&population_root.join("online-model-lineage.json"), lineage)?;
                        previous_model_lineage = Some(lineage.clone());
                    }
                    let candidate_ids = batch
                        .candidates
                        .iter()
                        .map(Candidate::id)
                        .collect::<Result<Vec<_>, _>>()?;
                    write_json(
                        &population_root.join("q-proposals.json"),
                        &serde_json::json!({
                            "status": "ready",
                            "summary": batch.summary,
                            "candidate_ids": candidate_ids,
                            "envelopes": batch.envelopes,
                        }),
                    )?;
                    batch.candidates
                }
                Err(error) => {
                    write_json(
                        &population_root.join("q-proposals.json"),
                        &serde_json::json!({
                            "status": "unavailable",
                            "error": error,
                            "training_corpora": training_corpora.len(),
                            "aligned_elite_episodes": q_episodes.len(),
                        }),
                    )?;
                    Vec::new()
                }
            };
            if let Some(dataset_generation) = dataset_generation {
                previous_dataset_generation = Some(dataset_generation);
            }
            manifest = evolve_population_with_retained_and_proposals(
                &manifest_path,
                &final_results.as_ref().unwrap().results,
                &next_root,
                EvolutionConfig {
                    population_size: search.population_size,
                    elite_count: search.elite_count,
                    rng_seed: search.rng_seed + u64::from(generation) + 1,
                },
                &archived_candidates,
                &q_candidates,
            )?;
            population_root = next_root;
        }
    }
    let results = final_results.expect("nonzero generations");
    if &results.objective != prepared.identity() {
        return Err(EvaluateError::InvalidResult(
            "final anchored results changed objective identity".into(),
        ));
    }
    let leaderboard = rank_population(&manifest, &results.results)?;
    let champion = leaderboard.first().ok_or(EvaluateError::EmptyLeaderboard)?;
    let member = manifest
        .members
        .iter()
        .find(|member| member.candidate_id == champion.candidate_id)
        .ok_or(EvaluateError::EmptyLeaderboard)?;
    let source = population_root.join(&member.tape_file);
    let suffix = InputTape::decode(&fs::read(&source)?)?.tape;
    let full = prepared.realize_suffix(suffix)?;
    let champion_suffix_tape = search.output_root.join("champion.suffix.tape");
    fs::copy(source, &champion_suffix_tape)?;
    let champion_tape = search.output_root.join("champion.tape");
    fs::write(&champion_tape, full.encode()?)?;
    let champion_candidate = search.output_root.join("champion.candidate.json");
    fs::copy(
        population_root.join(&member.candidate_file),
        &champion_candidate,
    )?;
    let summary = AnchoredSearchRunSummary {
        schema: ANCHORED_RUN_SCHEMA,
        segment: search.segment,
        objective: results.objective,
        generations: search.generations,
        population_size: search.population_size,
        repetitions: search.repetitions,
        rng_seed: search.rng_seed,
        champion_id: champion.candidate_id.clone(),
        champion_candidate,
        champion_suffix_tape,
        champion_tape,
        score: champion.score,
        output_root: search.output_root.clone(),
    };
    write_json(&search.output_root.join("run.summary.json"), &summary)?;
    Ok(summary)
}

fn write_initial_proposal_envelopes(
    manifest: &PopulationManifest,
    population_root: &Path,
    objective: NamedDigest,
    population_size: usize,
) -> Result<(), EvaluateError> {
    let configuration = serde_json::to_vec(&(
        "dusklight-anchored-seed-population/v1",
        manifest.segment,
        &manifest.boot,
        manifest.rng_seed,
        population_size,
    ))?;
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.anchored-seed-population/v1\0");
    hasher.update((configuration.len() as u64).to_le_bytes());
    hasher.update(configuration);
    let configuration_sha256 = ArtifactDigest(hasher.finalize().into());
    let mut envelopes = Vec::with_capacity(manifest.members.len());
    for member in &manifest.members {
        let candidate_sha256 = member.candidate_id.parse().map_err(|error| {
            EvaluateError::InvalidManifest(format!("invalid candidate digest: {error}"))
        })?;
        let parent_candidate_sha256 = member
            .ancestry
            .parent_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|error| {
                EvaluateError::InvalidManifest(format!("invalid parent candidate digest: {error}"))
            })?;
        let (kind, id) = if parent_candidate_sha256.is_some() {
            (ProposerKind::StructuredSearch, "search.seed-mutation")
        } else {
            (ProposerKind::Scripted, "scripted.observed-seed")
        };
        envelopes.push(
            CandidateEnvelope::build(
                candidate_sha256,
                parent_candidate_sha256,
                member.ancestry.generation,
                objective.clone(),
                NamedDigest::new("movement-action/v2", movement_action_schema_digest_v2()),
                manifest.rng_seed,
                ProposerIdentity {
                    kind,
                    id: id.into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    configuration_sha256,
                },
            )
            .map_err(|error| EvaluateError::InvalidManifest(error.to_string()))?,
        );
    }
    let set = CandidateEnvelopeSet::build(envelopes)
        .map_err(|error| EvaluateError::InvalidManifest(error.to_string()))?;
    write_json(&population_root.join("proposal-envelopes.json"), &set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn initial_population_seals_scripted_and_structured_proposal_envelopes() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("huntctl-proposal-envelopes-{unique}"));
        let manifest = write_seed_population(
            &root,
            Candidate::baseline(SegmentProfile::Fsp103ToFsp104),
            4,
            71,
        )
        .unwrap();
        let objective = NamedDigest::new("entered-f-sp104", ArtifactDigest([0xa7; 32]));
        write_initial_proposal_envelopes(&manifest, &root, objective.clone(), 4).unwrap();
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("proposal-envelopes.json")).unwrap())
                .unwrap();
        let envelopes: Vec<CandidateEnvelope> =
            serde_json::from_value(document["envelopes"].clone()).unwrap();
        assert_eq!(envelopes.len(), manifest.members.len());
        assert_eq!(
            envelopes
                .iter()
                .filter(|envelope| envelope.proposer.kind == ProposerKind::Scripted)
                .count(),
            1
        );
        assert_eq!(
            envelopes
                .iter()
                .filter(|envelope| envelope.proposer.kind == ProposerKind::StructuredSearch)
                .count(),
            3
        );
        assert!(envelopes.iter().all(|envelope| {
            envelope.validate().is_ok()
                && envelope.objective == objective
                && envelope.action_schema.sha256 == movement_action_schema_digest_v2()
                && envelope.seed == 71
        }));
        fs::remove_dir_all(root).unwrap();
    }
}
