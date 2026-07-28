use super::*;
use crate::optimization_request::{OptimizationCriticCorpus, OptimizationCriticRanking};
use crate::residual_critic_ranking::PreparedResidualCriticRanker;
use dusklight_automation_contracts::observation_view::movement_state_v2_spec;
use dusklight_evidence::transition_corpus::{
    MacroAction, StateReference, StateReferenceKind, Transition, TransitionCorpus,
};
use dusklight_learning::offline_rl::MovementActionSchema;
use dusklight_search::residual_action::{
    AnalogChannel, AnalogResidual, ResidualCandidate, TemporalBasis, compile_residual_candidate,
};
use dusklight_search::residual_optimizer::{
    ResidualGenome, ResidualProposal, ResidualProposalBatch,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap()
}

#[test]
fn crash_before_optimizer_checkpoint_reproposes_without_repeating_candidates() {
    let root = repository();
    let checked = root.join(
        "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
    );
    let mut optimization: OptimizationRequest =
        serde_json::from_slice(&fs::read(checked).unwrap()).unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let relative = format!(
        "build/campaigns/residual-crash-window-{}-{nonce}",
        std::process::id()
    );
    optimization.resume.state_path = format!("{relative}/state.json");
    optimization.resume.journal_path = format!("{relative}/journal.jsonl");
    optimization.content_sha256 = Digest::ZERO;
    optimization.refresh_content_sha256().unwrap();
    optimization.validate_files(&root).unwrap();
    let incumbent = optimization.incumbent.as_ref().unwrap();
    let parent_bytes = fs::read(root.join(&incumbent.tape.path)).unwrap();
    let parent = InputTape::decode(&parent_bytes).unwrap().tape;
    let campaign = root.join(&relative);
    let resume = initialize_optimization_resume(&optimization, &root).unwrap();
    let mut first = new_optimizer(&optimization, &parent_bytes).unwrap();
    let ResidualCampaignOptimizer::Cem(first) = &mut first else {
        panic!("checked campaign is not CEM");
    };
    let batch = first.ask(&parent, &parent_bytes).unwrap();
    let prepared = prepare_batch(&optimization, &parent, &parent_bytes, 0, batch).unwrap();
    let sealed = seal_candidate_batch(&root, &campaign, &optimization, &resume, &prepared).unwrap();
    assert_eq!(sealed.candidates.len(), 64);
    assert_eq!(sealed.record_count, 64);

    let mut recovered = new_optimizer(&optimization, &parent_bytes).unwrap();
    let ResidualCampaignOptimizer::Cem(recovered) = &mut recovered else {
        panic!("checked campaign is not CEM");
    };
    let reproposed = recovered.ask(&parent, &parent_bytes).unwrap();
    let reproposed = prepare_batch(&optimization, &parent, &parent_bytes, 0, reproposed).unwrap();
    let adopted =
        seal_candidate_batch(&root, &campaign, &optimization, &sealed, &reproposed).unwrap();
    assert_eq!(adopted.record_count, sealed.record_count);
    assert_eq!(adopted.candidates, sealed.candidates);
    fs::remove_dir_all(campaign).unwrap();
}

#[test]
fn campaign_candidates_preserve_request_bound_critic_ordering() {
    let root = repository();
    let checked = root.join(
        "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
    );
    let mut optimization: OptimizationRequest =
        serde_json::from_slice(&fs::read(checked).unwrap()).unwrap();
    let incumbent = optimization.incumbent.as_ref().unwrap();
    let parent_bytes = fs::read(root.join(&incumbent.tape.path)).unwrap();
    let parent = InputTape::decode(&parent_bytes).unwrap().tape;
    let spec = movement_state_v2_spec();
    let action_schema = MovementActionSchema::V2;
    let reference = |byte| StateReference {
        kind: StateReferenceKind::Boundary,
        digest: Digest([byte; 32]),
    };
    let parent_rows = (0..126)
        .map(|frame| {
            let pad = parent.frames[frame].pads[0];
            let action = action_schema.action_id(pad).unwrap_or(0);
            Transition {
                source: reference((frame % 250 + 1) as u8),
                state: vec![0.0; spec.features.len()],
                action: MacroAction {
                    action_id: action,
                    macro_kind: action_schema.macro_kind(),
                    parameters: vec![
                        i16::from(pad.stick_x),
                        i16::from(pad.stick_y),
                        pad.buttons as i16,
                    ],
                },
                duration_ticks: 1,
                reward: -1.0,
                next: reference((frame % 250 + 2) as u8),
                next_state: vec![0.0; spec.features.len()],
                terminal: frame == 125,
            }
        })
        .collect::<Vec<_>>();
    let parent_corpus = TransitionCorpus::new(
        spec.digest().unwrap(),
        action_schema.digest(),
        spec.features.len() as u32,
        parent_rows,
    )
    .unwrap();
    let mut training_rows = Vec::new();
    for action in 0..action_schema.action_count() {
        training_rows.push(Transition {
            source: reference((action % 250 + 1) as u8),
            state: vec![0.0; spec.features.len()],
            action: MacroAction {
                action_id: action,
                macro_kind: action_schema.macro_kind(),
                parameters: vec![0, 0, 0],
            },
            duration_ticks: 1,
            reward: action as f32,
            next: reference((action % 250 + 2) as u8),
            next_state: vec![0.0; spec.features.len()],
            terminal: true,
        });
    }
    let training = TransitionCorpus::new(
        spec.digest().unwrap(),
        action_schema.digest(),
        spec.features.len() as u32,
        training_rows,
    )
    .unwrap();
    let artifact = ArtifactReference {
        path: "build/critic-fixture.dtcz".into(),
        sha256: Digest([61; 32]),
    };
    let source = OptimizationCriticCorpus {
        corpus: artifact.clone(),
        evidence: ArtifactReference {
            path: "build/critic-fixture.evidence.json".into(),
            sha256: Digest([62; 32]),
        },
    };
    let binding = OptimizationCriticRanking {
        training_corpora: vec![source.clone()],
        parent_corpus: source,
        parent_corpus_start_frame: 0,
        iterations: 1,
        trees_per_action: 1,
        seed: 7,
        uncertainty_penalty_millionths: 250_000,
    };
    optimization.proposal.critic_ranking = Some(binding.clone());
    optimization.refresh_content_sha256().unwrap();
    let ranker =
        PreparedResidualCriticRanker::from_corpora(&[training], parent_corpus, &binding).unwrap();
    let candidate = ResidualCandidate::seal(
        &parent_bytes,
        vec![AnalogResidual {
            port: 0,
            channel: AnalogChannel::MainY,
            basis: TemporalBasis::ExactFrame {
                frame: 8,
                delta: 127,
            },
        }],
        vec![],
    )
    .unwrap();
    let compiled = compile_residual_candidate(&parent, &parent_bytes, &candidate).unwrap();
    let prepared = prepare_batch_with_ranker(
        &optimization,
        &parent,
        &parent_bytes,
        0,
        ResidualProposalBatch {
            proposals: vec![ResidualProposal {
                generation: 0,
                sample_index: 0,
                genome: ResidualGenome { genes: vec![] },
                candidate,
                compiled,
            }],
            rejected_invalid: 0,
            rejected_duplicate_tape: 0,
        },
        Some(&ranker),
    )
    .unwrap();
    let evidence = prepared[0].envelope.critic_ranking.as_ref().unwrap();
    assert_eq!(evidence.rank, 0);
    assert!(evidence.exact_simulation_authority);
    assert!(!evidence.promotion_authority);
    prepared[0].envelope.validate().unwrap();
}
