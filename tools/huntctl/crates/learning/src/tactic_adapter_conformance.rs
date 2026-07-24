use crate::artifact::Digest;
use crate::fact_snapshot::{FactSnapshot, FactTerminalReason};
use crate::fqi::FqiConfig;
use crate::native_generic_tactic::{
    GenericTactic, NATIVE_GENERIC_TACTIC_SCHEMA_V1, NativeGenericTacticCandidate,
    NativeGenericTacticPlan, NativeTacticObservation, select_and_execute_generic,
};
use crate::option_transition::OptionTransitionSample;
use crate::option_values::{
    OptionValueBatch, OptionValueConfig, OptionValueModel, OptionValueSample,
};
use crate::tactic_asset::{
    PreparedTacticExecution, TacticAssetCatalog, TacticAssetSource, TacticCatalogEntry,
};
use crate::tactic_blueprint::{
    CompiledStaticSegment, TacticBlueprint, TacticBlueprintNode, compile_controller_layer,
};
use dusklight_control::controller_compilation::compile_static_controller;
use dusklight_control::controller_program::ControllerProgram;
use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
use dusklight_control::option_execution::TapeRange;
use dusklight_control::tape::InputTape;
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;

fn invoke(option_id: &str) -> TacticBlueprintNode {
    TacticBlueprintNode::Invoke {
        option_id: option_id.into(),
    }
}

#[test]
fn multi_tactic_adapters_preserve_queries_composition_boundaries_and_pad() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let mut native = shard.episodes[0].steps[0].pre_input.clone();
    native.camera_yaw_radians = Some(0.0);
    let tactic_observation = NativeTacticObservation::from_native(&native).unwrap();
    let position = native.player_position;
    let native_plan = NativeGenericTacticPlan {
        schema: NATIVE_GENERIC_TACTIC_SCHEMA_V1.into(),
        tactic: GenericTactic::SeekCoordinate {
            coordinate_f32_bits: [position[0] + 10.0, position[1], position[2]].map(f32::to_bits),
            tolerance_f32_bits: 0.25_f32.to_bits(),
            magnitude: 80,
        },
        minimum_ticks: 1,
        maximum_ticks: 1,
    };
    native_plan.validate().unwrap();

    let move_program = ControllerProgram::parse(
        "duskcontrol 1\nframes 2\nbezier replace from 0 for 2 p0 0 80 p1 0 80 p2 0 80 p3 0 80\n",
    )
    .unwrap();
    let button_program =
        ControllerProgram::parse("duskcontrol 1\nframes 2\nbuttons from 0 for 2 B\n").unwrap();
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "buttons",
            TacticAssetSource::ReactiveController(button_program),
        )
        .unwrap(),
        TacticCatalogEntry::new("move", TacticAssetSource::ReactiveController(move_program))
            .unwrap(),
        TacticCatalogEntry::new(
            "native.seek",
            TacticAssetSource::NativeGenericTactic(native_plan.clone()),
        )
        .unwrap(),
        TacticCatalogEntry::new(
            "shield",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();

    // Static composition must remain byte-for-byte equal to the existing
    // per-tactic realizers and DUSKCTRL layer compiler.
    let layer = TacticBlueprintNode::Layer {
        layers: vec![invoke("move"), invoke("buttons")],
    };
    let blueprint = TacticBlueprint::new(
        "conformance.trace",
        TacticBlueprintNode::Sequence {
            steps: vec![invoke("shield"), layer.clone()],
        },
    )
    .unwrap();
    let compiled = blueprint.compile_static(&catalog).unwrap();
    let PreparedTacticExecution::Static(shield) = catalog.prepare_execution("shield").unwrap()
    else {
        panic!("shield must retain its exact static executor")
    };
    let direct_layer =
        compile_static_controller(&compile_controller_layer(&layer, &catalog).unwrap()).unwrap();
    let mut direct_frames = shield.tape.frames.clone();
    direct_frames.extend(direct_layer.frames.clone());
    assert_eq!(compiled.tape.frames, direct_frames);
    assert_eq!(compiled.segments.len(), 2);
    let CompiledStaticSegment::Invoke { execution, .. } = &compiled.segments[0] else {
        panic!("first segment must remain the shield option")
    };
    assert_eq!(
        execution.realized_tape_range,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: 1,
        }
    );
    assert_eq!(execution.emitted_raw_actions, shield.tape.frames);
    let CompiledStaticSegment::Layer {
        tape_range,
        emitted_raw_actions,
        ..
    } = &compiled.segments[1]
    else {
        panic!("second segment must remain the controller layer")
    };
    assert_eq!(
        *tape_range,
        TapeRange {
            start_frame: 1,
            end_frame_exclusive: 3,
        }
    );
    assert_eq!(emitted_raw_actions, &direct_layer.frames);
    compiled.validate().unwrap();

    // Preparing the generic tactic through the catalog must retain the exact
    // existing candidate, query log, option boundary, and emitted PAD.
    let direct_candidate =
        NativeGenericTacticCandidate::new("native.seek".into(), native_plan).unwrap();
    let PreparedTacticExecution::NativeGeneric(adapted_candidate) =
        catalog.prepare_execution("native.seek").unwrap()
    else {
        panic!("native tactic must retain its observation-loop executor")
    };
    assert_eq!(adapted_candidate, direct_candidate);
    let sample = OptionValueSample {
        action: direct_candidate.descriptor().clone(),
        state: vec![0.0],
        duration_ticks: 1,
        reward: 1.0,
        next_state: vec![1.0],
        terminal: true,
        before_state_sha256: Digest([10; 32]),
        after_state_sha256: Digest([11; 32]),
        source_checkpoint_sha256: Digest([12; 32]),
        next_checkpoint_sha256: Digest([13; 32]),
        realized_tape_range: TapeRange {
            start_frame: 0,
            end_frame_exclusive: 1,
        },
        realized_tape_sha256: Digest([9; 32]),
    };
    let model = OptionValueModel::fit(1, &[sample], &[0], &OptionValueConfig::default()).unwrap();
    let observations = [tactic_observation.clone()];
    let direct = select_and_execute_generic(
        &model,
        &[0.0],
        &[direct_candidate],
        &InputTape::default(),
        &observations,
    )
    .unwrap();
    let adapted = select_and_execute_generic(
        &model,
        &[0.0],
        &[adapted_candidate],
        &InputTape::default(),
        &observations,
    )
    .unwrap();
    assert_eq!(adapted.queries, direct.queries);
    assert!(
        adapted.queries[0]
            .queried_fields
            .iter()
            .any(|field| field == "player_position")
    );
    assert_eq!(adapted.tape, direct.tape);
    assert_eq!(adapted.execution, direct.execution);
    assert_eq!(
        adapted.execution.realized_tape_range,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: 1,
        }
    );
    assert_eq!(adapted.execution.emitted_raw_actions, adapted.tape.frames);
    assert!(adapted.every_read_only_query_recorded);
    assert!(adapted.every_pad_frame_recorded);

    // The learner-facing projection consumes the same observation and does not
    // alter any core fact queried by the tactic.
    let full = FactSnapshot::from_native_learning(&native, &[], None, Vec::new()).unwrap();
    let compact = FactSnapshot::from_native_tactic(&tactic_observation, Vec::new()).unwrap();
    assert_eq!(compact.state_identity, full.state_identity);
    assert_eq!(compact.world.stage, full.world.stage);
    assert_eq!(compact.world.room, full.world.room);
    assert_eq!(
        compact.player.position_f32_bits,
        full.player.position_f32_bits
    );
    assert_eq!(compact.player.procedure, full.player.procedure);
}

#[test]
fn composed_delayed_reward_fixture_learns_the_required_tactic_chain() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let native = &shard.episodes[0].steps[0].pre_input;
    let mut start = FactSnapshot::from_native_learning(native, &[], None, Vec::new()).unwrap();
    start.boundary_index = 0;
    start.simulation_tick = 0;
    start.tape_frame = 0;
    start.state_identity = [1; 16];
    start.player.position_f32_bits[0] = 0.0_f32.to_bits();
    start.recent_history.clear();
    start.recent_option = None;
    start.terminal.reason = FactTerminalReason::None;
    start.terminal.configured = Some(true);
    start.terminal.reached = Some(false);

    let mut primed = start.clone();
    primed.boundary_index = 1;
    primed.simulation_tick = 1;
    primed.tape_frame = 1;
    primed.state_identity = [2; 16];
    primed.player.position_f32_bits[0] = 1.0_f32.to_bits();

    let mut goal = primed.clone();
    goal.boundary_index = 2;
    goal.simulation_tick = 3;
    goal.tape_frame = 3;
    goal.state_identity = [3; 16];
    goal.player.position_f32_bits[0] = 2.0_f32.to_bits();
    goal.terminal.reason = FactTerminalReason::GoalReached;
    goal.terminal.reached = Some(true);

    let mut dead_end = start.clone();
    dead_end.boundary_index = 1;
    dead_end.simulation_tick = 1;
    dead_end.tape_frame = 1;
    dead_end.state_identity = [4; 16];
    dead_end.player.position_f32_bits[0] = (-1.0_f32).to_bits();
    dead_end.terminal.reason = FactTerminalReason::GoalReached;
    dead_end.terminal.reached = Some(true);

    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "decoy",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Target { frames: 1 })),
        )
        .unwrap(),
        TacticCatalogEntry::new(
            "finish",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Interact {
                press_frames: 1,
                recovery_frames: 1,
            })),
        )
        .unwrap(),
        TacticCatalogEntry::new(
            "prime",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();
    let solution = TacticBlueprint::new(
        "delayed.solution",
        TacticBlueprintNode::Sequence {
            steps: vec![invoke("prime"), invoke("finish")],
        },
    )
    .unwrap();
    let compiled = solution.compile_static(&catalog).unwrap();
    assert_eq!(compiled.segments.len(), 2);
    assert_eq!(compiled.tape.frames.len(), 3);
    let CompiledStaticSegment::Invoke {
        execution: prime_execution,
        ..
    } = &compiled.segments[0]
    else {
        panic!("solution prefix must be an exact invoked tactic")
    };
    let CompiledStaticSegment::Invoke {
        execution: finish_execution,
        ..
    } = &compiled.segments[1]
    else {
        panic!("solution suffix must be an exact invoked tactic")
    };

    let encode = |facts: &FactSnapshot| {
        Ok::<_, &'static str>(vec![f32::from_bits(facts.player.position_f32_bits[0])])
    };
    let feature_schema = Digest([31; 32]);
    let prime = OptionTransitionSample::capture(
        feature_schema,
        Digest([10; 32]),
        Digest([11; 32]),
        start.clone(),
        primed.clone(),
        prime_execution.clone(),
        &compiled.tape,
        0.0,
        false,
        encode,
    )
    .unwrap();
    let finish = OptionTransitionSample::capture(
        feature_schema,
        Digest([11; 32]),
        Digest([12; 32]),
        primed,
        goal,
        finish_execution.clone(),
        &compiled.tape,
        10.0,
        true,
        encode,
    )
    .unwrap();
    let PreparedTacticExecution::Static(decoy_realization) =
        catalog.prepare_execution("decoy").unwrap()
    else {
        panic!("decoy must retain its exact static executor")
    };
    let decoy = OptionTransitionSample::capture(
        feature_schema,
        Digest([10; 32]),
        Digest([13; 32]),
        start,
        dead_end,
        decoy_realization.execution,
        &decoy_realization.tape,
        -2.0,
        true,
        encode,
    )
    .unwrap();

    // The first action has no immediate reward. It can outrank the terminal
    // decoy only if fitted Q follows the same episode into the rewarded suffix.
    assert_eq!(prime.value_sample.reward, 0.0);
    assert!(!prime.value_sample.terminal);
    let batch = OptionValueBatch::new(
        feature_schema,
        Digest([32; 32]),
        1,
        vec![
            prime.value_sample.clone(),
            finish.value_sample.clone(),
            decoy.value_sample.clone(),
        ],
        vec![1, 1, 2],
    )
    .unwrap();
    let model = OptionValueModel::fit_batch(
        &batch,
        &OptionValueConfig {
            fitted_q: FqiConfig {
                iterations: 20,
                trees_per_action: 9,
                max_tree_depth: 3,
                bootstrap: false,
                seed: 73,
                ..FqiConfig::default()
            },
        },
    )
    .unwrap();
    let start_ranking = model
        .rank_available_options(
            &[0.0],
            &[
                prime.value_sample.action.clone(),
                decoy.value_sample.action.clone(),
            ],
        )
        .unwrap();
    assert_eq!(start_ranking.ranked[0].descriptor.option_id, "prime");
    assert!(start_ranking.ranked[0].mean_q > 0.0);
    assert!(
        start_ranking.ranked[0].mean_q
            > start_ranking
                .ranked
                .iter()
                .find(|ranked| ranked.descriptor.option_id == "decoy")
                .unwrap()
                .mean_q
    );
    let primed_ranking = model
        .rank_available_options(&[1.0], std::slice::from_ref(&finish.value_sample.action))
        .unwrap();
    assert_eq!(primed_ranking.ranked[0].descriptor.option_id, "finish");
    assert_eq!(
        [
            &prime.execution.emitted_raw_actions[..],
            &finish.execution.emitted_raw_actions[..]
        ]
        .concat(),
        compiled.tape.frames
    );
}
