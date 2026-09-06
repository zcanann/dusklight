use super::*;
use dusklight_automation_contracts::tape::{InputFrame, RawPadState};
use dusklight_control::option_execution::{OptionCondition, OptionEndReason, TapeRange};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::fact_snapshot::{FactPhase, FactTerminalReason};
use dusklight_learning::reward_shaping::TACTIC_REWARD_SPEC_SCHEMA_V2;
use dusklight_learning::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
use dusklight_learning::tactic_features::GoalConditionedTacticFeatureEncoder;

const ROOT_CHECKPOINT: Digest = Digest([0x91; 32]);
const EXECUTION_AUTHORITY: Digest = Digest([0x92; 32]);
const OBJECTIVE: Digest = Digest([0x93; 32]);
const ACTION_SCHEMA: Digest = Digest([0x94; 32]);
const LEARNER_MODEL: Digest = Digest([0x95; 32]);

fn base_facts() -> FactSnapshot {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    FactSnapshot::from_native_learning(&shard.episodes[0].steps[0].pre_input, &[], None, Vec::new())
        .unwrap()
}

fn input_frame(stick_x: i8) -> InputFrame {
    let mut frame = InputFrame {
        owned_ports: 1,
        ..InputFrame::default()
    };
    frame.pads[0] = RawPadState {
        stick_x,
        connected: true,
        ..RawPadState::default()
    };
    frame
}

fn action_catalog(namespace: &str, magnitude: i8) -> TacticAssetCatalog {
    TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            format!("{namespace}/toward"),
            TacticAssetSource::RecordedTape(InputTape {
                frames: vec![input_frame(magnitude)],
                ..InputTape::default()
            }),
        )
        .unwrap(),
        TacticCatalogEntry::new(
            format!("{namespace}/away"),
            TacticAssetSource::RecordedTape(InputTape {
                frames: vec![input_frame(-magnitude)],
                ..InputTape::default()
            }),
        )
        .unwrap(),
    ])
    .unwrap()
}

fn facts_at(base: &FactSnapshot, group: u8, marker: u8, x: f32) -> FactSnapshot {
    let mut facts = base.clone();
    facts.phase = FactPhase::PreInput;
    facts.state_identity = [0; 16];
    facts.state_identity[0] = group;
    facts.state_identity[1] = marker;
    facts.world.stage = "GENERIC_LEARNING_FIXTURE".into();
    facts.world.room = 0;
    facts.world.point = Some(0);
    facts.player.position_f32_bits = [x.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()];
    facts.player.procedure = Some(3);
    facts.player.velocity_f32_bits = Some([0.0_f32.to_bits(); 3]);
    facts.player.forward_speed_f32_bits = Some(0.0_f32.to_bits());
    facts.recent_history.clear();
    facts.recent_option = None;
    facts.terminal.configured = Some(true);
    facts.terminal.reached = Some(false);
    facts.terminal.reason = FactTerminalReason::None;
    facts.terminal.first_hit_tick = None;
    facts.validate().unwrap();
    facts
}

fn campaign(
    base: &FactSnapshot,
    group: u8,
    x: f32,
    catalog: &TacticAssetCatalog,
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> TacticQCampaign {
    let root = facts_at(base, group, 0, x);
    let current = LearnerState::build(
        root.clone(),
        &FactRegistry::canonical(),
        catalog,
        &[],
        |_| true,
    )
    .unwrap();
    let mut campaign = TacticQCampaign::new(
        encoder.schema_sha256,
        OBJECTIVE,
        ROOT_CHECKPOINT,
        u64::from(group) * 2,
        current,
        InputTape {
            frames: vec![InputFrame::default(); root.tape_frame as usize],
            ..InputTape::default()
        },
        OptionValueConfig::default(),
        TacticExplorationConfig {
            seed: u64::from(group),
            epsilon_per_million: 0,
        },
    )
    .unwrap();
    campaign
        .bind_execution_authority(EXECUTION_AUTHORITY)
        .unwrap();
    campaign
}

fn one_entry_catalog(catalog: &TacticAssetCatalog, option_id: &str) -> TacticAssetCatalog {
    TacticAssetCatalog::new(vec![catalog.entry(option_id).unwrap().clone()]).unwrap()
}

fn rewarded_outcome(
    campaign: &TacticQCampaign,
    selected: &SelectedTactic,
    catalog: &TacticAssetCatalog,
    group: u8,
    marker: u8,
) -> NativeTacticWorkerOutcome {
    let frame = match catalog
        .entry(&selected.descriptor.option_id)
        .unwrap()
        .source()
    {
        TacticAssetSource::RecordedTape(tape) => tape.frames[0].clone(),
        _ => panic!("fixture actions must be recorded one-frame inputs"),
    };
    let mut route_tape = campaign.route_tape.clone();
    route_tape.frames.push(frame.clone());
    let start_frame = campaign.current.snapshot.tape_frame;
    let execution = OptionExecution::capture(
        selected.descriptor.option_id.clone(),
        selected.descriptor.option_type.clone(),
        selected.descriptor.parameters.clone(),
        1,
        1,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &route_tape,
        TapeRange {
            start_frame,
            end_frame_exclusive: start_frame + 1,
        },
    )
    .unwrap();
    let before_x = f32::from_bits(campaign.current.snapshot.player.position_f32_bits[0]);
    let delta = match frame.pads[0].stick_x {
        value if value > 0 => 1.0,
        value if value < 0 => -1.0,
        _ => unreachable!("fixture action cannot be neutral"),
    };
    let mut next_facts = facts_at(&campaign.current.snapshot, group, marker, before_x + delta);
    next_facts.boundary_index = campaign.current.snapshot.boundary_index + 1;
    next_facts.simulation_tick = campaign.current.snapshot.simulation_tick + 1;
    next_facts.tape_frame = start_frame + 1;
    next_facts.validate().unwrap();
    NativeTacticWorkerOutcome {
        schema: crate::native_tactic_worker::NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V2.into(),
        source_checkpoint_sha256: ROOT_CHECKPOINT,
        checkpoint_identity: format!("parameter-transfer-{group}-{marker}"),
        episode_shard_sha256: Digest([group; 32]),
        selected: selected.clone(),
        execution,
        native_queries: Vec::new(),
        route_tape,
        next_facts,
        state_extraction_micros: 1,
        intermediate_boundaries: Vec::new(),
        terminal: false,
        retained_native_checkpoint: None,
        retained_native_boundary_fingerprint: None,
    }
}

fn reward_spec() -> TacticRewardSpec {
    TacticRewardSpec {
        schema: TACTIC_REWARD_SPEC_SCHEMA_V2.into(),
        terminal_reward: 100.0,
        tick_cost: 1.0,
        novelty_reward: 0.0,
        per_tick_discount: 1.0,
        potential: None,
        motion_cost: None,
    }
}

fn execute_and_admit(
    online: &mut TacticQOnlineLearningController,
    campaign: &mut TacticQCampaign,
    full_catalog: &TacticAssetCatalog,
    option_id: &str,
    group: u8,
    marker: u8,
    encoder: &GoalConditionedTacticFeatureEncoder,
) {
    let selected_catalog = one_entry_catalog(full_catalog, option_id);
    let batch = online
        .select_action_batch(
            campaign,
            &selected_catalog,
            &[],
            &|facts| encoder.encode(facts),
            TacticQOnlineActionSelectionRequest {
                family_schema_sha256: ACTION_SCHEMA,
                maximum_proposals: 1,
                acquisition_partition: 0,
                policy: TacticProposalPolicy::Learned,
                goal_distance_feature: Some(encoder.goal_distance_feature()),
                force_exploration: false,
                lease_mode: TacticQOnlineLeaseMode::Exploration,
            },
        )
        .unwrap();
    let leased = match online
        .prepare_decision(
            campaign,
            batch,
            TacticQOnlineDecisionRequest {
                suffix_ticks: 0,
                horizon: 32,
                maximum_proposals: 1,
                learner_model_sha256: LEARNER_MODEL,
                lease_mode: TacticQOnlineLeaseMode::Exploration,
            },
        )
        .unwrap()
    {
        TacticQOnlineDecisionPlan::Execute(leased) => leased,
        other => panic!("fresh sibling was not executable: {other:?}"),
    };
    let outcome = rewarded_outcome(
        campaign,
        &leased.batch.proposals[0],
        full_catalog,
        group,
        marker,
    );
    let evaluated = campaign
        .evaluate_rewarded_outcome(outcome, &|facts| encoder.encode(facts), &reward_spec())
        .unwrap();
    let episode_group = campaign.episode_group;
    online
        .admit(
            campaign,
            &leased.batch,
            &[evaluated],
            &[episode_group],
            &leased.leases,
            full_catalog,
            &[],
            &FactRegistry::canonical(),
            &|facts| encoder.encode(facts),
            |_| true,
            &reward_spec(),
            TacticQOnlinePolicyUpdate::Adaptive { refit_model: false },
        )
        .unwrap();
}

fn current_branch(campaign: &TacticQCampaign) -> TacticCampaignBranch {
    let graph = campaign.state_graph.as_ref().unwrap();
    let current = ExactStateId {
        route_checkpoint_sha256: route_checkpoint(
            campaign.root_checkpoint_sha256,
            &campaign.route_tape,
        )
        .unwrap(),
        state_sha256: campaign.current.snapshot_sha256,
    };
    if graph.root() == current {
        graph_root_branch(graph).unwrap()
    } else {
        campaign.exact_frontier_branch(current).unwrap()
    }
}

fn collect_sibling_feedback(
    base: &FactSnapshot,
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> TacticQTrainingCorpus {
    let catalog = action_catalog("train", 127);
    let mut campaign = campaign(base, 0, 0.0, &catalog, encoder);
    let mut online = TacticQOnlineLearningController::default();
    let mut episode_group = 0_u64;

    for group in 0..16_u8 {
        let source = current_branch(&campaign);
        execute_and_admit(
            &mut online,
            &mut campaign,
            &catalog,
            "train/away",
            group,
            2,
            encoder,
        );
        episode_group += 1;
        campaign
            .restore_branch(
                &source,
                episode_group,
                &FactRegistry::canonical(),
                &catalog,
                &[],
                |_| true,
            )
            .unwrap();
        execute_and_admit(
            &mut online,
            &mut campaign,
            &catalog,
            "train/toward",
            group + 1,
            0,
            encoder,
        );
        episode_group += 1;
    }

    let corpus = campaign.training_corpus();
    assert_eq!(corpus.transitions.len(), 32);
    let sibling_groups = corpus.transitions.iter().fold(
        BTreeMap::<Digest, BTreeSet<&str>>::new(),
        |mut groups, row| {
            groups
                .entry(row.before_state_sha256)
                .or_default()
                .insert(row.value_sample.action.option_id.as_str());
            groups
        },
    );
    assert_eq!(sibling_groups.len(), 16);
    assert!(sibling_groups.values().all(|actions| actions.len() == 2));
    corpus
}

fn learned_batch(
    online: &mut TacticQOnlineLearningController,
    campaign: &TacticQCampaign,
    catalog: &TacticAssetCatalog,
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> TacticQProposalBatch {
    online
        .select_action_batch(
            campaign,
            catalog,
            &[],
            &|facts| encoder.encode(facts),
            TacticQOnlineActionSelectionRequest {
                family_schema_sha256: ACTION_SCHEMA,
                maximum_proposals: 1,
                acquisition_partition: 0,
                policy: TacticProposalPolicy::Learned,
                goal_distance_feature: Some(encoder.goal_distance_feature()),
                force_exploration: false,
                lease_mode: TacticQOnlineLeaseMode::Exploration,
            },
        )
        .unwrap()
}

#[test]
fn hindsight_snapshot_drives_executable_choices_without_a_motion_gate() {
    let base = base_facts();
    let encoder = GoalConditionedTacticFeatureEncoder::new([1.0, 0.0, 0.0]).unwrap();
    let training = collect_sibling_feedback(&base, &encoder);
    assert!(
        training
            .transitions
            .iter()
            .all(|row| !row.value_sample.terminal)
    );
    let replay_revision = training.transitions.len() as u64;
    let snapshot = TacticQImmutableLearnerSnapshot::fit(
        training,
        replay_revision,
        1,
        OptionValueConfig::default(),
        encoder.goal_distance_feature(),
        TacticValueTreatment::HindsightReturnKnnV1,
    )
    .unwrap();
    assert!(snapshot.manifest.goal_reachability_calibration.is_none());
    assert!(snapshot.manifest.terminal_action_calibration.is_none());
    let query_catalog = action_catalog("held-out", 120);
    let mut campaign = campaign(&base, 0, 0.0, &query_catalog, &encoder);
    campaign
        .consume_learner_snapshot_with_exploration_filter(&snapshot, |_| false)
        .unwrap();
    let mut online = TacticQOnlineLearningController::default();
    let batch = learned_batch(&mut online, &campaign, &query_catalog, &encoder);
    assert_eq!(batch.proposals[0].descriptor.option_id, "held-out/toward");
    assert_eq!(
        batch.proposals[0].reason,
        TacticSelectionReason::GeneralizedValue
    );
    assert!(batch.goal_reachability_calibration.is_none());
    assert!(!campaign.native_terminal_supported());
    let leased = match online
        .prepare_decision(
            &mut campaign,
            batch,
            TacticQOnlineDecisionRequest {
                suffix_ticks: 0,
                horizon: 32,
                maximum_proposals: 1,
                learner_model_sha256: snapshot.sha256,
                lease_mode: TacticQOnlineLeaseMode::Exploration,
            },
        )
        .unwrap()
    {
        TacticQOnlineDecisionPlan::Execute(leased) => leased,
        other => panic!("hindsight-selected action was not executable: {other:?}"),
    };
    let outcome = rewarded_outcome(&campaign, &leased.batch.proposals[0], &query_catalog, 0, 1);
    assert!(outcome.route_tape.frames.last().unwrap().pads[0].stick_x > 0);
    assert!(!outcome.terminal);
    // Exploration still belongs to epsilon, including after publishing a model.
    campaign.exploration.epsilon_per_million = 1_000_000;
    let explored = campaign
        .decide_parameterized_batch_with_policy(
            &query_catalog,
            &[],
            ACTION_SCHEMA,
            &|facts| encoder.encode(facts),
            1,
            0,
            TacticProposalPolicy::Learned,
            Some(encoder.goal_distance_feature()),
            false,
        )
        .unwrap();
    assert_eq!(explored.proposals[0].reason, TacticSelectionReason::Epsilon);
}

#[test]
fn retained_sibling_feedback_controls_and_executes_an_unseen_compatible_action() {
    let base = base_facts();
    let encoder = GoalConditionedTacticFeatureEncoder::new([100.0, 0.0, 0.0]).unwrap();
    let training = collect_sibling_feedback(&base, &encoder);
    let replay_revision = training.transitions.len() as u64;
    let snapshot = TacticQImmutableLearnerSnapshot::fit(
        training,
        replay_revision,
        1,
        OptionValueConfig::default(),
        encoder.goal_distance_feature(),
        TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4,
    )
    .unwrap();
    let calibration = snapshot
        .manifest
        .goal_reachability_calibration
        .as_ref()
        .unwrap();
    assert_eq!(calibration.source_transitions, replay_revision as usize);
    assert!(calibration.deployment_ready, "{calibration:#?}");

    let query_catalog = action_catalog("held-out", 120);
    assert!(
        snapshot
            .training_corpus()
            .transitions
            .iter()
            .all(|transition| {
                !transition
                    .value_sample
                    .action
                    .option_id
                    .starts_with("held-out/")
            })
    );
    let mut campaign = campaign(&base, 0, 0.0, &query_catalog, &encoder);
    let (cold, mut online) = (0..10_000_u64)
        .find_map(|seed| {
            campaign.exploration.seed = seed;
            let mut online = TacticQOnlineLearningController::default();
            let batch = learned_batch(&mut online, &campaign, &query_catalog, &encoder);
            (batch.proposals[0].descriptor.option_id == "held-out/away").then_some((batch, online))
        })
        .expect("a deterministic cold-start seed must select the inferior sibling");
    assert_eq!(cold.proposals[0].descriptor.option_id, "held-out/away");
    assert_ne!(
        cold.proposals[0].reason,
        TacticSelectionReason::GoalReachability
    );
    assert_eq!(
        campaign
            .consume_learner_snapshot_with_exploration_filter(&snapshot, |_| false)
            .unwrap(),
        replay_revision as usize
    );
    let root = graph_root_branch(campaign.state_graph.as_ref().unwrap()).unwrap();
    let batch = learned_batch(&mut online, &campaign, &query_catalog, &encoder);
    assert_eq!(batch.proposals[0].descriptor.option_id, "held-out/toward");
    assert_eq!(
        batch.proposals[0].reason,
        TacticSelectionReason::GoalReachability
    );
    let leased = match online
        .prepare_decision(
            &mut campaign,
            batch,
            TacticQOnlineDecisionRequest {
                suffix_ticks: 0,
                horizon: 32,
                maximum_proposals: 1,
                learner_model_sha256: snapshot.sha256,
                lease_mode: TacticQOnlineLeaseMode::Exploration,
            },
        )
        .unwrap()
    {
        TacticQOnlineDecisionPlan::Execute(leased) => leased,
        other => panic!("held-out learned action was not executable: {other:?}"),
    };
    let outcome = rewarded_outcome(&campaign, &leased.batch.proposals[0], &query_catalog, 0, 1);
    let target = outcome.next_facts.clone();
    let evaluated = campaign
        .evaluate_rewarded_outcome(outcome, &|facts| encoder.encode(facts), &reward_spec())
        .unwrap();
    let episode_group = campaign.episode_group;
    online
        .admit(
            &mut campaign,
            &leased.batch,
            &[evaluated],
            &[episode_group],
            &leased.leases,
            &query_catalog,
            &[],
            &FactRegistry::canonical(),
            &|facts| encoder.encode(facts),
            |_| true,
            &reward_spec(),
            TacticQOnlinePolicyUpdate::Frozen,
        )
        .unwrap();
    campaign
        .restore_branch(
            &root,
            episode_group + 1,
            &FactRegistry::canonical(),
            &query_catalog,
            &[],
            |_| true,
        )
        .unwrap();
    let repeat = learned_batch(&mut online, &campaign, &query_catalog, &encoder);
    match online
        .prepare_decision(
            &mut campaign,
            repeat,
            TacticQOnlineDecisionRequest {
                suffix_ticks: 0,
                horizon: 32,
                maximum_proposals: 1,
                learner_model_sha256: snapshot.sha256,
                lease_mode: TacticQOnlineLeaseMode::Exploration,
            },
        )
        .unwrap()
    {
        TacticQOnlineDecisionPlan::FollowCompletedExpansion(traversal) => {
            assert_eq!(
                traversal.batch.proposals[0].descriptor.option_id,
                "held-out/toward"
            );
            assert_eq!(traversal.target.state, target);
        }
        other => panic!("learned graph edge was not reusable: {other:?}"),
    }
}
