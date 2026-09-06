use super::*;
use dusklight_learning::native_generic_tactic::NativeGenericTacticPlan;

#[test]
fn camera_relative_heading_emits_the_same_pad_in_both_execution_strategies() {
    // A camera-relative command must not reverse or rotate when Link turns.
    // Check actual outputs, not merely the opcode chosen by the compiler.
    for heading in [0.0_f32, 0.375, std::f32::consts::FRAC_PI_2, -2.0] {
        let plan = NativeGenericTacticPlan::new(
            GenericTactic::MaintainRelativeHeading {
                heading_radians_f32_bits: heading.to_bits(),
                magnitude: 100,
            },
            3,
        );
        let program = native_generic_controller_program(
            &plan,
            TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: 3,
            },
        )
        .unwrap()
        .unwrap();
        // Include the wire round trip used by the native controller path.
        let program = ControllerProgram::decode(&program.encode().unwrap()).unwrap();
        let mut controller = ControllerProgramStepper::new(program).unwrap();
        let mut generic = NativeGenericTacticStepper::new(plan).unwrap();
        for (tick, (camera, player_yaw)) in [(0.0_f32, 0_i16), (1.25, 8192), (-2.0, -16384)]
            .into_iter()
            .enumerate()
        {
            let observation = NativeTacticObservation {
                boundary_index: tick as u64,
                simulation_tick: tick as u64,
                tape_frame: tick as u64,
                state_identity: [tick as u8 + 1; 16],
                stage: "test".into(),
                room: 0,
                player_position_f32_bits: [0.0_f32.to_bits(); 3],
                player_yaw,
                player_procedure: 0,
                player_mode_flags: 0,
                player_contacts: 0,
                camera_yaw_radians_f32_bits: Some(camera.to_bits()),
                action_lanes: Vec::new(),
                actor_set_complete: true,
                actors: Vec::new(),
            };
            let native = controller
                .step(&ControllerRuntimeObservation {
                    boundary_index: observation.boundary_index,
                    simulation_tick: observation.simulation_tick,
                    tape_frame: observation.tape_frame,
                    state_identity: observation.state_identity,
                    player_present: true,
                    player_position: [0.0; 3],
                    player_yaw_radians: Some(
                        f32::from(player_yaw) * std::f32::consts::PI / 32768.0,
                    ),
                    player_velocity_xz: None,
                    camera_yaw_radians: Some(camera),
                    stage: observation.stage.clone(),
                    actors_complete: true,
                    actors: Vec::new(),
                })
                .unwrap()
                .frame
                .unwrap()
                .pads[0];
            let audited = generic.step(observation).unwrap().frame.pads[0];
            let expected = (
                (-heading.sin() * 100.0).round() as i8,
                (heading.cos() * 100.0).round() as i8,
            );
            assert_eq!(
                (native.stick_x, native.stick_y),
                expected,
                "native heading {heading}, tick {tick}"
            );
            assert_eq!(
                (audited.stick_x, audited.stick_y),
                expected,
                "audit heading {heading}, tick {tick}"
            );
            assert_eq!(native.buttons, 0);
            assert_eq!(audited.buttons, 0);
        }
    }
}
