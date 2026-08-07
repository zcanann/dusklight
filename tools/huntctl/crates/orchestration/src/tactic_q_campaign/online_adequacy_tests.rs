use super::*;
use dusklight_automation_contracts::tape::{InputFrame, RawPadState};
use dusklight_control::option_execution::{OptionCondition, OptionEndReason, TapeRange};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::fact_snapshot::{FactPhase, FactTerminalReason};
use dusklight_learning::reward_shaping::{TACTIC_REWARD_SPEC_SCHEMA_V2, TacticRewardSpec};
use dusklight_learning::tactic_asset::{TacticAssetSource, TacticCatalogEntry};

const ROOT_CHECKPOINT: Digest = Digest([0x71; 32]);
const EXECUTION_AUTHORITY: Digest = Digest([0x72; 32]);
const FEATURE_SCHEMA: Digest = Digest([0x73; 32]);
const OBJECTIVE: Digest = Digest([0x74; 32]);
const LEARNER_SNAPSHOT: Digest = Digest([0x75; 32]);
const ACTION_SCHEMA: Digest = Digest([0x76; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdequacyState {
    Start,
    VariantStart,
    Detour(u8),
    Optimal(u8),
    Shortcut(u8),
    Goal,
    VariantDetour(u8),
    VariantOptimal(u8),
    VariantShortcut(u8),
    VariantGoal,
}

impl AdequacyState {
    fn code(self) -> u8 {
        match self {
            Self::Start => 0,
            Self::VariantStart => 64,
            Self::Detour(step) => 1 + step,
            Self::Optimal(step) => 32 + step,
            Self::Shortcut(step) => 48 + step,
            Self::Goal => 63,
            Self::VariantDetour(step) => 65 + step,
            Self::VariantOptimal(step) => 96 + step,
            Self::VariantShortcut(step) => 112 + step,
            Self::VariantGoal => 127,
        }
    }

    fn position(self) -> [f32; 3] {
        match self {
            Self::Start => [0.0, 0.0, 0.0],
            Self::VariantStart => [10.0, 0.0, 0.0],
            // The detour initially moves toward the goal, then winds around a
            // longer corridor. Position is observation, not reward.
            Self::Detour(step) => {
                const POSITIONS: [[f32; 3]; 11] = [
                    [1.0, 0.0, 0.0],
                    [2.0, 0.0, 0.0],
                    [2.0, 0.0, -1.0],
                    [2.0, 0.0, -2.0],
                    [3.0, 0.0, -2.0],
                    [4.0, 0.0, -2.0],
                    [4.0, 0.0, -1.0],
                    [4.0, 0.0, 0.0],
                    [4.0, 0.0, 1.0],
                    [3.0, 0.0, 1.0],
                    [3.0, 0.0, 0.5],
                ];
                POSITIONS[step as usize]
            }
            // The optimal route must move away from the goal for three ticks
            // before it can turn the corner.
            Self::Optimal(step) => {
                const POSITIONS: [[f32; 3]; 8] = [
                    [0.0, 0.0, 1.0],
                    [0.0, 0.0, 2.0],
                    [0.0, 0.0, 3.0],
                    [1.0, 0.0, 3.0],
                    [2.0, 0.0, 3.0],
                    [3.0, 0.0, 3.0],
                    [3.0, 0.0, 2.0],
                    [3.0, 0.0, 1.0],
                ];
                POSITIONS[step as usize]
            }
            Self::Shortcut(step) => {
                const POSITIONS: [[f32; 3]; 5] = [
                    [2.0, 0.0, -1.0],
                    [2.5, 0.0, -1.0],
                    [3.0, 0.0, -1.0],
                    [3.5, 0.0, -1.0],
                    [3.5, 0.0, 0.0],
                ];
                POSITIONS[step as usize]
            }
            Self::Goal => [3.0, 0.0, 0.0],
            Self::VariantDetour(step) => translated(Self::Detour(step).position()),
            Self::VariantOptimal(step) => translated(Self::Optimal(step).position()),
            Self::VariantShortcut(step) => translated(Self::Shortcut(step).position()),
            Self::VariantGoal => translated(Self::Goal.position()),
        }
    }

    fn applicable(self, action: &str) -> bool {
        match self {
            Self::Start | Self::VariantStart => matches!(action, "east" | "north"),
            Self::Detour(3) | Self::VariantDetour(3) => matches!(action, "east" | "north"),
            Self::Detour(step) => action == detour_action(step),
            Self::Optimal(step) => action == optimal_action(step),
            Self::Shortcut(step) => action == shortcut_action(step),
            Self::VariantDetour(step) => action == detour_action(step),
            Self::VariantOptimal(step) => action == optimal_action(step),
            Self::VariantShortcut(step) => action == shortcut_action(step),
            Self::Goal | Self::VariantGoal => false,
        }
    }

    fn execute(self, action: &str) -> Option<Self> {
        if !self.applicable(action) {
            return None;
        }
        match self {
            Self::Start if action == "east" => Some(Self::Detour(0)),
            Self::Start if action == "north" => Some(Self::Optimal(0)),
            Self::VariantStart if action == "east" => Some(Self::VariantDetour(0)),
            Self::VariantStart if action == "north" => Some(Self::VariantOptimal(0)),
            Self::Detour(3) if action == "north" => Some(Self::Shortcut(0)),
            Self::Detour(10) => Some(Self::Goal),
            Self::Detour(step) => Some(Self::Detour(step + 1)),
            Self::Optimal(7) => Some(Self::Goal),
            Self::Optimal(step) => Some(Self::Optimal(step + 1)),
            Self::Shortcut(4) => Some(Self::Goal),
            Self::Shortcut(step) => Some(Self::Shortcut(step + 1)),
            Self::VariantDetour(3) if action == "north" => Some(Self::VariantShortcut(0)),
            Self::VariantDetour(10) => Some(Self::VariantGoal),
            Self::VariantDetour(step) => Some(Self::VariantDetour(step + 1)),
            Self::VariantOptimal(7) => Some(Self::VariantGoal),
            Self::VariantOptimal(step) => Some(Self::VariantOptimal(step + 1)),
            Self::VariantShortcut(4) => Some(Self::VariantGoal),
            Self::VariantShortcut(step) => Some(Self::VariantShortcut(step + 1)),
            _ => None,
        }
    }

    fn variant(self) -> bool {
        matches!(
            self,
            Self::VariantStart
                | Self::VariantDetour(_)
                | Self::VariantOptimal(_)
                | Self::VariantShortcut(_)
                | Self::VariantGoal
        )
    }
}

fn translated(mut position: [f32; 3]) -> [f32; 3] {
    position[0] += 10.0;
    position
}

fn detour_action(step: u8) -> &'static str {
    [
        "east", "south", "south", "east", "east", "north", "north", "north", "west", "south",
        "south",
    ][step as usize]
}

fn optimal_action(step: u8) -> &'static str {
    [
        "north", "north", "east", "east", "east", "south", "south", "south",
    ][step as usize]
}

fn shortcut_action(step: u8) -> &'static str {
    ["north", "east", "east", "south", "west"][step as usize]
}

fn input_frame(action: &str) -> InputFrame {
    let (stick_x, stick_y) = match action {
        "north" => (0, 127),
        "east" => (127, 0),
        "south" => (0, -127),
        "west" => (-127, 0),
        "variant" => (0, 0),
        _ => unreachable!(),
    };
    let mut frame = InputFrame {
        owned_ports: 1,
        ..InputFrame::default()
    };
    frame.pads[0] = RawPadState {
        stick_x,
        stick_y,
        connected: true,
        ..RawPadState::default()
    };
    frame
}

fn action_from_input(frame: &InputFrame) -> &'static str {
    match (frame.pads[0].stick_x, frame.pads[0].stick_y) {
        (0, 127) => "north",
        (127, 0) => "east",
        (0, -127) => "south",
        (-127, 0) => "west",
        _ => panic!("adequacy tactic emitted an unknown input"),
    }
}

fn catalog() -> TacticAssetCatalog {
    TacticAssetCatalog::new(
        ["east", "north", "south", "west"]
            .into_iter()
            .map(|action| {
                TacticCatalogEntry::new(
                    action,
                    TacticAssetSource::RecordedTape(InputTape {
                        frames: vec![input_frame(action)],
                        ..InputTape::default()
                    }),
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap()
}

fn two_north_candidate(
    catalog: &TacticAssetCatalog,
    base_start: &FactSnapshot,
    variant_start: &FactSnapshot,
) -> DiscoveredMacroCandidate {
    let component =
        TacticMacroComponent::from_catalog_entry(catalog.entry("north").unwrap()).unwrap();
    let source = |snapshot: &FactSnapshot, transition: u8| MacroSourceProvenance {
        seed: 7,
        frontier_state_sha256: snapshot.content_sha256().unwrap(),
        transition_sha256s: vec![
            Digest([transition; 32]),
            Digest([transition.wrapping_add(1); 32]),
        ],
        entry: MacroEntryObservation {
            stage: snapshot.world.stage.clone(),
            room: snapshot.world.room,
            player_procedure: snapshot.player.procedure,
            player_contacts: snapshot.player.contacts,
            goal_distance_f32_bits: 3.0_f32.to_bits(),
        },
    };
    replay_macro_candidate(
        InputTape {
            frames: vec![input_frame("north"), input_frame("north")],
            ..InputTape::default()
        },
        vec![component.clone(), component],
        vec![source(base_start, 0x81), source(variant_start, 0x82)],
    )
    .unwrap()
}

fn base_facts() -> (FactSnapshot, Digest) {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let facts = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        Vec::new(),
    )
    .unwrap();
    (facts, shard.content_sha256)
}

fn facts_at(base: &FactSnapshot, state: AdequacyState, tape_frame: u64) -> FactSnapshot {
    let mut facts = base.clone();
    facts.phase = FactPhase::PreInput;
    facts.boundary_index = tape_frame;
    facts.simulation_tick = tape_frame;
    facts.tape_frame = tape_frame;
    facts.state_identity = [state.code(); 16];
    facts.world.stage = "ADEQ".into();
    facts.world.room = 0;
    facts.world.point = Some(if state.variant() { 1 } else { 0 });
    facts.player.position_f32_bits = state.position().map(f32::to_bits);
    facts.player.procedure = Some(u16::from(state.code()));
    facts.player.velocity_f32_bits = Some([0.0_f32.to_bits(); 3]);
    facts.player.forward_speed_f32_bits = Some(0.0_f32.to_bits());
    facts.recent_history.clear();
    facts.recent_option = None;
    let terminal = matches!(state, AdequacyState::Goal | AdequacyState::VariantGoal);
    facts.terminal.configured = Some(true);
    facts.terminal.reached = Some(terminal);
    facts.terminal.reason = if terminal {
        FactTerminalReason::GoalReached
    } else {
        FactTerminalReason::None
    };
    facts.terminal.first_hit_tick = terminal.then_some(tape_frame);
    facts.validate().unwrap();
    facts
}

fn state_from_facts(facts: &FactSnapshot) -> AdequacyState {
    match facts.state_identity[0] {
        0 => AdequacyState::Start,
        64 => AdequacyState::VariantStart,
        1..=11 => AdequacyState::Detour(facts.state_identity[0] - 1),
        32..=39 => AdequacyState::Optimal(facts.state_identity[0] - 32),
        48..=52 => AdequacyState::Shortcut(facts.state_identity[0] - 48),
        63 => AdequacyState::Goal,
        65..=75 => AdequacyState::VariantDetour(facts.state_identity[0] - 65),
        96..=103 => AdequacyState::VariantOptimal(facts.state_identity[0] - 96),
        112..=116 => AdequacyState::VariantShortcut(facts.state_identity[0] - 112),
        127 => AdequacyState::VariantGoal,
        _ => panic!("unknown adequacy state"),
    }
}

fn encode(facts: &FactSnapshot) -> Result<Vec<f32>, &'static str> {
    let [x, _, y] = facts.player.position_f32_bits.map(f32::from_bits);
    let origin_x = if facts.world.point == Some(1) {
        10.0
    } else {
        0.0
    };
    Ok(vec![x - origin_x, y])
}

fn campaign_with_cold_primary(
    base: &FactSnapshot,
    catalog: &TacticAssetCatalog,
    root_state: AdequacyState,
    primary_option_id: &str,
) -> TacticQCampaign {
    for seed in 0..10_000 {
        let root = facts_at(base, root_state, base.tape_frame);
        let current = LearnerState::build(
            root,
            &FactRegistry::canonical(),
            catalog,
            &[],
            |description| {
                root_state.applicable(&description.option.option_id)
                    || description.option.option_id == primary_option_id
            },
        )
        .unwrap();
        let mut campaign = TacticQCampaign::new(
            FEATURE_SCHEMA,
            OBJECTIVE,
            ROOT_CHECKPOINT,
            0,
            current,
            InputTape {
                frames: vec![InputFrame::default(); base.tape_frame as usize],
                ..InputTape::default()
            },
            OptionValueConfig::default(),
            TacticExplorationConfig {
                seed,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        campaign
            .bind_execution_authority(EXECUTION_AUTHORITY)
            .unwrap();
        let mut online = TacticQOnlineLearningController::default();
        if decide_production_batch(&mut online, &campaign, catalog, 2).proposals[0]
            .descriptor
            .option_id
            == primary_option_id
        {
            return campaign;
        }
    }
    panic!("no deterministic cold-start seed selected the requested primary")
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

fn decide_production_batch(
    online: &mut TacticQOnlineLearningController,
    campaign: &TacticQCampaign,
    catalog: &TacticAssetCatalog,
    maximum_proposals: usize,
) -> TacticQProposalBatch {
    let applicable = campaign
        .current
        .action_mask
        .iter()
        .filter(|choice| choice.applicable)
        .map(|choice| choice.descriptor.option_id.as_str())
        .collect::<BTreeSet<_>>();
    let state_catalog = TacticAssetCatalog::new(
        catalog
            .entries()
            .iter()
            .filter(|entry| applicable.contains(entry.option_id()))
            .cloned()
            .collect(),
    )
    .unwrap();
    online
        .select_action_batch(
            campaign,
            &state_catalog,
            &[],
            &encode,
            TacticQOnlineActionSelectionRequest {
                family_schema_sha256: ACTION_SCHEMA,
                maximum_proposals,
                acquisition_partition: 0,
                policy: TacticProposalPolicy::Learned,
                goal_distance_feature: None,
                force_exploration: false,
            },
        )
        .unwrap()
}

fn execute_selected(
    campaign: &TacticQCampaign,
    catalog: &TacticAssetCatalog,
    selected: &SelectedTactic,
    episode_shard_sha256: Digest,
) -> NativeTacticWorkerOutcome {
    let frames = match catalog
        .entry(&selected.descriptor.option_id)
        .expect("selected adequacy tactic must exist in its catalog")
        .source()
    {
        TacticAssetSource::RecordedTape(tape) => tape.frames.clone(),
        TacticAssetSource::GuardedRecordedTape(guarded) => guarded.tape.frames.clone(),
        _ => panic!("adequacy tactic must be an exact recorded input sequence"),
    };
    let target = frames.iter().fold(
        state_from_facts(&campaign.current.snapshot),
        |state, frame| {
            state
                .execute(action_from_input(frame))
                .expect("the production action mask must expose only executable fixture actions")
        },
    );
    let mut route_tape = campaign.route_tape.clone();
    route_tape.frames.extend(frames.iter().cloned());
    let start_frame = campaign.current.snapshot.tape_frame;
    let realized_ticks = u32::try_from(frames.len()).unwrap();
    let execution = OptionExecution::capture(
        selected.descriptor.option_id.clone(),
        selected.descriptor.option_type.clone(),
        selected.descriptor.parameters.clone(),
        realized_ticks,
        realized_ticks,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &route_tape,
        TapeRange {
            start_frame,
            end_frame_exclusive: start_frame + u64::from(realized_ticks),
        },
    )
    .unwrap();
    let next_facts = facts_at(
        &campaign.current.snapshot,
        target,
        start_frame + u64::from(realized_ticks),
    );
    NativeTacticWorkerOutcome {
        schema: crate::native_tactic_worker::NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V2.into(),
        source_checkpoint_sha256: ROOT_CHECKPOINT,
        checkpoint_identity: format!("online-adequacy-{}", start_frame + 1),
        episode_shard_sha256,
        selected: selected.clone(),
        execution,
        native_queries: Vec::new(),
        route_tape,
        next_facts,
        state_extraction_micros: 1,
        intermediate_boundaries: Vec::new(),
        terminal: matches!(target, AdequacyState::Goal | AdequacyState::VariantGoal),
        retained_native_checkpoint: None,
        retained_native_boundary_fingerprint: None,
    }
}

fn run_one_step(
    online: &mut TacticQOnlineLearningController,
    campaign: &mut TacticQCampaign,
    catalog: &TacticAssetCatalog,
    episode_shard_sha256: Digest,
) -> String {
    let registry = FactRegistry::canonical();
    let batch = decide_production_batch(online, campaign, catalog, 4);
    let leased = match online
        .prepare_decision(
            campaign,
            batch,
            TacticQOnlineDecisionRequest {
                suffix_ticks: 0,
                horizon: 32,
                maximum_proposals: 1,
                learner_model_sha256: LEARNER_SNAPSHOT,
                lease_mode: TacticQOnlineLeaseMode::Exploration,
            },
        )
        .unwrap()
    {
        TacticQOnlineDecisionPlan::Execute(leased) => leased,
        TacticQOnlineDecisionPlan::RestoreCheckpoint { .. } => {
            panic!("one-tick adequacy actions must fit the rollout horizon")
        }
    };
    assert_eq!(
        online.pending_decision_index(),
        Some(campaign.decision_index)
    );
    let selected = leased.batch.proposals[0].clone();
    let outcome = execute_selected(campaign, catalog, &selected, episode_shard_sha256);
    let evaluated = campaign
        .evaluate_rewarded_outcome(outcome.clone(), &encode, &reward_spec())
        .unwrap();
    let target = state_from_facts(&outcome.next_facts);
    let episode_group = campaign.episode_group;
    let admission = online
        .admit(
            campaign,
            &leased.batch,
            &[evaluated],
            &[episode_group],
            &leased.leases,
            catalog,
            &[],
            &registry,
            &encode,
            |description| target.applicable(&description.option.option_id),
            &reward_spec(),
            TacticQOnlinePolicyUpdate::Adaptive { refit_model: true },
        )
        .unwrap();
    assert_eq!(admission.newly_admitted_training_rows, 1);
    assert_eq!(online.pending_decision_index(), None);
    selected.descriptor.option_id
}

fn run_to_terminal(
    online: &mut TacticQOnlineLearningController,
    campaign: &mut TacticQCampaign,
    catalog: &TacticAssetCatalog,
    episode_shard_sha256: Digest,
) -> Vec<String> {
    let mut actions = Vec::new();
    while campaign.current.snapshot.terminal.reached != Some(true) {
        actions.push(run_one_step(
            online,
            campaign,
            catalog,
            episode_shard_sha256,
        ));
        if campaign.current.snapshot.terminal.reached != Some(true) {
            assert_eq!(
                plan_online_continuation(TacticQOnlineContinuationRequest {
                    force_branch: false,
                    terminal_restart: false,
                    native_terminal_supported: campaign.native_terminal_supported(),
                    active_acquisition_rank: 1,
                    // Broad exploration must finish its selected continuation;
                    // terminal-focused acquisition is tested by the explicit
                    // retained-frontier restore below.
                    next_acquisition_rank: 1,
                    current_rollout_ticks: 0,
                    post_terminal_discovery_tick_budget: None,
                    demonstration_coverage_pending: false,
                    terminal_refinement_in_progress: false,
                    terminal_refinement_completed: false,
                    root_refresh_due: false,
                    goal_relabeling_enabled: false,
                    terminal_frontier_action_value_enabled: false,
                })
                .unwrap(),
                None,
                "a nonterminal production rollout must continue until terminal or horizon"
            );
        }
        assert!(
            actions.len() <= 32,
            "production rollout failed to terminate"
        );
    }
    actions
}

fn restore_next_scheduled_frontier(
    online: &mut TacticQOnlineLearningController,
    campaign: &mut TacticQCampaign,
    catalog: &TacticAssetCatalog,
    episode_group: u64,
    acquisition_rank: u64,
) -> AdequacyState {
    let selected = online
        .continue_rollout(
            campaign,
            TacticQOnlineRolloutRequest {
                force_branch: false,
                active_acquisition_rank: acquisition_rank,
                next_acquisition_rank: acquisition_rank,
                current_rollout_ticks: 0,
                post_terminal_discovery_tick_budget: None,
                demonstration_coverage_pending: false,
                terminal_refinement_in_progress: false,
                terminal_refinement_completed: false,
                root_refresh_due: false,
                goal_relabeling_enabled: false,
                terminal_frontier_action_value_enabled: false,
                seed: 0,
                round: episode_group,
                episode_group,
                maximum_route_frames: usize::MAX,
                goal_distance_feature: 0,
            },
            &FactRegistry::canonical(),
            &[],
            &[],
            &encode,
            &|_, facts: &FactSnapshot| {
                let state = state_from_facts(facts);
                let applicable_actions = catalog
                    .option_descriptors()
                    .filter(|descriptor| state.applicable(&descriptor.option_id))
                    .cloned()
                    .collect();
                Ok(TacticQOnlineActionSurface {
                    catalog: TacticAssetCatalog::new(
                        catalog
                            .entries()
                            .iter()
                            .filter(|entry| state.applicable(entry.option_id()))
                            .cloned()
                            .collect(),
                    )
                    .unwrap(),
                    blueprints: Vec::new(),
                    applicable_actions,
                })
            },
        )
        .unwrap()
        .expect("a terminal rollout must schedule another checkpoint")
        .branch;
    let state = state_from_facts(&selected.state);
    state
}

#[derive(Debug, Eq, PartialEq)]
struct AdequacyRollout {
    start: AdequacyState,
    actions: Vec<String>,
    best_terminal_ticks: u64,
}

fn learn_until_terminal_ticks(
    online: &mut TacticQOnlineLearningController,
    campaign: &mut TacticQCampaign,
    catalog: &TacticAssetCatalog,
    episode_shard_sha256: Digest,
    target_ticks: u64,
) -> Vec<AdequacyRollout> {
    let mut episode = 0_u64;
    let mut rollouts = Vec::new();
    loop {
        let start = state_from_facts(&campaign.current.snapshot);
        let actions = run_to_terminal(online, campaign, catalog, episode_shard_sha256);
        let best_terminal_ticks = campaign
            .best_graph_terminal_path()
            .unwrap()
            .expect("a completed fixture rollout must authenticate a terminal")
            .root_to_terminal_ticks;
        rollouts.push(AdequacyRollout {
            start,
            actions,
            best_terminal_ticks,
        });
        if best_terminal_ticks <= target_ticks {
            return rollouts;
        }
        episode += 1;
        restore_next_scheduled_frontier(online, campaign, catalog, episode, episode);
        assert!(episode <= 8, "online fixture failed to converge");
    }
}

#[test]
fn ordinary_exploration_preserves_the_policy_selected_action() {
    let (base, _) = base_facts();
    let catalog = catalog();
    let mut campaign = campaign_with_cold_primary(&base, &catalog, AdequacyState::Start, "east");
    let mut online = TacticQOnlineLearningController::default();
    let batch = decide_production_batch(&mut online, &campaign, &catalog, 1);
    let selected = batch.proposals[0].clone();
    assert_eq!(
        batch
            .ranking
            .choices
            .iter()
            .filter(|choice| choice.applicable)
            .count(),
        2,
        "the regression requires an unselected applicable alternative"
    );

    let leased = match online
        .prepare_decision(
            &mut campaign,
            batch,
            TacticQOnlineDecisionRequest {
                suffix_ticks: 0,
                horizon: 32,
                maximum_proposals: 1,
                learner_model_sha256: LEARNER_SNAPSHOT,
                lease_mode: TacticQOnlineLeaseMode::Exploration,
            },
        )
        .unwrap()
    {
        TacticQOnlineDecisionPlan::Execute(leased) => leased,
        TacticQOnlineDecisionPlan::RestoreCheckpoint { .. } => {
            panic!("the selected one-tick action must fit the rollout horizon")
        }
    };

    assert_eq!(leased.batch.proposals, vec![selected.clone()]);
    assert_eq!(leased.leases.len(), 1);
    assert_eq!(leased.leases[0].descriptor, selected.descriptor);
    let scheduler = leased
        .scheduler_decision
        .expect("ordinary exploration must still use an authenticated graph lease");
    assert_eq!(scheduler.ranked_source_queue.len(), 1);
    assert_eq!(scheduler.evaluated_expansion_sha256.len(), 1);
}

#[test]
fn production_campaign_learns_the_shorter_around_corner_route_online() {
    let (base, episode_shard_sha256) = base_facts();
    let catalog = catalog();
    let mut campaign = campaign_with_cold_primary(&base, &catalog, AdequacyState::Start, "east");
    let mut online = TacticQOnlineLearningController::default();

    let rollouts = learn_until_terminal_ticks(
        &mut online,
        &mut campaign,
        &catalog,
        episode_shard_sha256,
        9,
    );
    assert_eq!(rollouts.len(), 3);
    assert_eq!(rollouts[0].start, AdequacyState::Start);
    assert_eq!(rollouts[0].actions.len(), 12);
    assert_eq!(rollouts[0].actions[0], "east");
    assert_eq!(rollouts[0].best_terminal_ticks, 12);
    assert_eq!(rollouts[1].start, AdequacyState::Detour(3));
    assert_eq!(rollouts[1].actions.len(), 6);
    assert_eq!(rollouts[1].actions[0], "north");
    assert_eq!(rollouts[1].best_terminal_ticks, 10);
    assert_eq!(rollouts[2].start, AdequacyState::Start);
    assert_eq!(rollouts[2].actions.len(), 9);
    assert_eq!(rollouts[2].actions[0], "north");
    assert_eq!(rollouts[2].best_terminal_ticks, 9);

    assert_eq!(
        restore_next_scheduled_frontier(&mut online, &mut campaign, &catalog, 3, 3),
        AdequacyState::Start,
        "the production scheduler must revisit the learned root boundary"
    );
    let learned = decide_production_batch(&mut online, &campaign, &catalog, 2);
    assert_eq!(learned.proposals[0].descriptor.option_id, "north");
    assert_eq!(
        learned.proposals[0].reason,
        dusklight_learning::tactic_exploration::TacticSelectionReason::ExactTerminalReturn
    );
    assert!(campaign.model_revision() > 0);

    let mut variant_campaign =
        campaign_with_cold_primary(&base, &catalog, AdequacyState::VariantStart, "east");
    let variant_rollouts = learn_until_terminal_ticks(
        &mut online,
        &mut variant_campaign,
        &catalog,
        episode_shard_sha256,
        9,
    );
    assert!((2..=3).contains(&variant_rollouts.len()));
    assert_eq!(variant_rollouts[0].start, AdequacyState::VariantStart);
    assert_eq!(variant_rollouts[0].actions[0], "east");
    assert!((10..=12).contains(&variant_rollouts[0].best_terminal_ticks));
    let variant_final = variant_rollouts.last().unwrap();
    assert_eq!(variant_final.start, AdequacyState::VariantStart);
    assert_eq!(variant_final.actions.len(), 9);
    assert_eq!(variant_final.actions[0], "north");
    assert_eq!(variant_final.best_terminal_ticks, 9);
    let next_variant_episode = variant_rollouts.len() as u64;
    assert_eq!(
        restore_next_scheduled_frontier(
            &mut online,
            &mut variant_campaign,
            &catalog,
            next_variant_episode,
            next_variant_episode,
        ),
        AdequacyState::VariantStart,
        "the production scheduler must revisit the translated learned root boundary"
    );
    let generalized = decide_production_batch(&mut online, &variant_campaign, &catalog, 2);
    assert_eq!(generalized.proposals[0].descriptor.option_id, "north");
    assert_eq!(
        generalized.proposals[0].reason,
        dusklight_learning::tactic_exploration::TacticSelectionReason::ExactTerminalReturn
    );
    assert!(variant_campaign.model_revision() > 0);

    let learning_rows = campaign.graph_learning_batch().unwrap().rows;
    let variant_learning_rows = variant_campaign.graph_learning_batch().unwrap().rows;
    let base_state_ids = learning_rows
        .iter()
        .map(|row| row.source_state.state_identity)
        .collect::<BTreeSet<_>>();
    let variant_state_ids = variant_learning_rows
        .iter()
        .map(|row| row.source_state.state_identity)
        .collect::<BTreeSet<_>>();
    assert!(base_state_ids.is_disjoint(&variant_state_ids));
    let exact_return = |state: AdequacyState, action: &str| {
        learning_rows
            .iter()
            .find(|row| {
                row.source_state.state_identity[0] == state.code() && row.action.option_id == action
            })
            .and_then(|row| row.exact_conditional_ticks_to_terminal)
    };
    let variant_exact_return = |state: AdequacyState, action: &str| {
        variant_learning_rows
            .iter()
            .find(|row| {
                row.source_state.state_identity[0] == state.code() && row.action.option_id == action
            })
            .and_then(|row| row.exact_conditional_ticks_to_terminal)
    };
    assert_eq!(exact_return(AdequacyState::Start, "north"), Some(9));
    assert_eq!(exact_return(AdequacyState::Detour(3), "north"), Some(6));
    assert_eq!(
        variant_exact_return(AdequacyState::VariantStart, "north"),
        Some(9)
    );
    for step in 0..=7 {
        assert_eq!(
            exact_return(AdequacyState::Optimal(step), optimal_action(step)),
            Some(8 - u64::from(step))
        );
        assert_eq!(
            variant_exact_return(AdequacyState::VariantOptimal(step), optimal_action(step)),
            Some(8 - u64::from(step))
        );
    }

    let root_rows = learning_rows
        .into_iter()
        .filter(|row| row.source_state.state_identity[0] == AdequacyState::Start.code())
        .map(|row| {
            (
                row.action.option_id,
                row.exact_conditional_ticks_to_terminal,
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(root_rows.get("east"), Some(&Some(10)));
    assert_eq!(root_rows.get("north"), Some(&Some(9)));
}

#[test]
fn promoted_composition_reduces_online_policy_decisions() {
    let (base, episode_shard_sha256) = base_facts();
    let primitive_catalog = catalog();
    let base_start = facts_at(&base, AdequacyState::Start, base.tape_frame);
    let variant_start = facts_at(&base, AdequacyState::VariantStart, base.tape_frame);
    let candidate = two_north_candidate(&primitive_catalog, &base_start, &variant_start);

    let mut registry = TacticMacroPromotionRegistry::default();
    let mut lifecycle = TacticQOnlineLearningController::default();
    let proposed = lifecycle
        .update_tactics(
            &mut registry,
            [candidate.clone()],
            Vec::<MacroComparisonEvidence>::new(),
        )
        .unwrap();
    assert_eq!(proposed.proposed_count, 1);
    let comparisons = [
        base_start.content_sha256().unwrap(),
        variant_start.content_sha256().unwrap(),
    ]
    .into_iter()
    .map(|state| {
        MacroComparisonEvidence::new(
            candidate.candidate_sha256,
            7,
            state,
            false,
            2.0,
            2,
            false,
            2.0,
            2,
        )
        .unwrap()
    })
    .collect::<Vec<_>>();
    let promoted = lifecycle
        .update_tactics(
            &mut registry,
            Vec::<DiscoveredMacroCandidate>::new(),
            comparisons,
        )
        .unwrap();
    assert_eq!(promoted.promoted_count, 1);
    assert_eq!(promoted.newly_promoted, vec![candidate.candidate_sha256]);

    let mut entries = primitive_catalog.entries().to_vec();
    entries.push(candidate.catalog_entry().unwrap());
    let promoted_catalog = TacticAssetCatalog::new(entries).unwrap();
    for primitive in ["north", "east", "south", "west"] {
        assert!(promoted_catalog.entry(primitive).is_some());
    }

    let mut macro_campaign = campaign_with_cold_primary(
        &base,
        &promoted_catalog,
        AdequacyState::Start,
        &candidate.option_id,
    );
    let mut macro_controller = TacticQOnlineLearningController::default();
    assert_eq!(
        run_one_step(
            &mut macro_controller,
            &mut macro_campaign,
            &promoted_catalog,
            episode_shard_sha256,
        ),
        candidate.option_id
    );
    assert_eq!(
        state_from_facts(&macro_campaign.current.snapshot),
        AdequacyState::Optimal(1)
    );
    assert_eq!(macro_campaign.decision_index, 1);
    assert_eq!(macro_campaign.training_replay_len(), 1);

    let mut primitive_campaign =
        campaign_with_cold_primary(&base, &primitive_catalog, AdequacyState::Start, "north");
    let mut primitive_controller = TacticQOnlineLearningController::default();
    assert_eq!(
        run_one_step(
            &mut primitive_controller,
            &mut primitive_campaign,
            &primitive_catalog,
            episode_shard_sha256,
        ),
        "north"
    );
    assert_eq!(
        run_one_step(
            &mut primitive_controller,
            &mut primitive_campaign,
            &primitive_catalog,
            episode_shard_sha256,
        ),
        "north"
    );
    assert_eq!(
        primitive_campaign.current.snapshot.state_identity,
        macro_campaign.current.snapshot.state_identity
    );
    assert_eq!(primitive_campaign.route_tape, macro_campaign.route_tape);
    assert_eq!(primitive_campaign.decision_index, 2);
    assert_eq!(primitive_campaign.training_replay_len(), 2);

    let remaining = run_to_terminal(
        &mut macro_controller,
        &mut macro_campaign,
        &promoted_catalog,
        episode_shard_sha256,
    );
    assert!(!remaining.is_empty());
    let primitive_remaining = run_to_terminal(
        &mut primitive_controller,
        &mut primitive_campaign,
        &primitive_catalog,
        episode_shard_sha256,
    );
    assert!(!primitive_remaining.is_empty());
    assert!(
        macro_campaign
            .import_training_corpora_without_refit(&[primitive_campaign.training_corpus()])
            .unwrap()
            > 0
    );
    let root = macro_campaign
        .sample_root_and_frontier(0, 0, &[], usize::MAX)
        .unwrap()[0]
        .clone();
    macro_campaign
        .restore_branch(
            &root,
            1,
            &FactRegistry::canonical(),
            &promoted_catalog,
            &[],
            |description| {
                AdequacyState::Start.applicable(&description.option.option_id)
                    || description.option.option_id == candidate.option_id
            },
        )
        .unwrap();
    let learned =
        decide_production_batch(&mut macro_controller, &macro_campaign, &promoted_catalog, 2);
    assert_eq!(
        learned.proposals[0].descriptor.option_id,
        candidate.option_id
    );
    assert_eq!(
        learned.proposals[0].reason,
        TacticSelectionReason::ExactTerminalReturn
    );
    assert!(
        learned
            .ranking
            .values
            .ranked
            .iter()
            .any(|ranked| { ranked.descriptor.option_id == candidate.option_id })
    );
    assert!(
        !learned
            .ranking
            .values
            .unsupported
            .iter()
            .any(|descriptor| descriptor.option_id == candidate.option_id)
    );
    let exact_root_returns = macro_campaign
        .graph_learning_batch()
        .unwrap()
        .rows
        .into_iter()
        .filter(|row| row.source.state_sha256 == root.logical_frontier.state_sha256)
        .filter_map(|row| {
            row.exact_conditional_ticks_to_terminal
                .map(|ticks| (row.action.option_id, ticks))
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(exact_root_returns.get("north"), Some(&9));
    assert_eq!(
        exact_root_returns.get(&candidate.option_id),
        Some(&9),
        "equal native cost must expose policy-decision compression as a tie break"
    );
    assert!(macro_campaign.model_revision() > 0);

    macro_campaign.exploration.epsilon_per_million = 1_000_000;
    let exploratory = macro_campaign
        .decide_batch(&promoted_catalog, &[], &encode, 2)
        .unwrap();
    assert_eq!(
        exploratory.proposals[0].reason,
        TacticSelectionReason::Epsilon,
        "exact objective support must not disable explicit exploration"
    );
    assert!(
        exploratory
            .proposals
            .iter()
            .any(|proposal| proposal.descriptor.option_id == candidate.option_id)
    );
}
