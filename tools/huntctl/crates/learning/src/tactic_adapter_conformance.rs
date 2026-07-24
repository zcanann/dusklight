use crate::artifact::Digest;
use crate::fact_snapshot::FactSnapshot;
use crate::native_generic_tactic::{
    GenericTactic, NATIVE_GENERIC_TACTIC_SCHEMA_V1, NativeGenericTacticCandidate,
    NativeGenericTacticPlan, NativeTacticObservation, select_and_execute_generic,
};
use crate::option_values::{OptionValueConfig, OptionValueModel, OptionValueSample};
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
