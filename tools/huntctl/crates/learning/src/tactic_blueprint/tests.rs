use super::*;
use crate::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
use dusklight_control::controller_program::ControllerProgram;
use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
use dusklight_control::roll_option::{RollOptionPlan, RollSpacing};

fn condition() -> OptionCondition {
    OptionCondition::TargetReached {
        target: "door".into(),
    }
}

fn invoke(option_id: &str) -> TacticBlueprintNode {
    TacticBlueprintNode::Invoke {
        option_id: option_id.into(),
    }
}

#[test]
fn all_blueprint_node_kinds_round_trip_with_stable_identity() {
    let blueprint = TacticBlueprint::new(
        "route.opening",
        TacticBlueprintNode::Sequence {
            steps: vec![
                TacticBlueprintNode::Layer {
                    layers: vec![invoke("move"), invoke("camera")],
                },
                TacticBlueprintNode::Conditional {
                    condition: condition(),
                    when_true: Box::new(invoke("interact")),
                    when_false: Box::new(TacticBlueprintNode::Fallback {
                        attempts: vec![invoke("roll"), invoke("wait")],
                    }),
                },
                TacticBlueprintNode::Until {
                    condition: condition(),
                    max_iterations: 3,
                    body: Box::new(invoke("move")),
                },
            ],
        },
    )
    .unwrap();

    let bytes = blueprint.canonical_bytes().unwrap();
    let decoded: TacticBlueprint = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, blueprint);
    assert_eq!(
        decoded.content_sha256().unwrap(),
        blueprint.content_sha256().unwrap()
    );
    assert_eq!(
        blueprint.referenced_option_ids(),
        ["camera", "interact", "move", "roll", "wait"]
            .into_iter()
            .collect()
    );
}

#[test]
fn blueprint_rejects_unknown_catalog_references() {
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "wait",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();
    let blueprint = TacticBlueprint::new(
        "unknown.reference",
        TacticBlueprintNode::Sequence {
            steps: vec![invoke("wait"), invoke("missing")],
        },
    )
    .unwrap();

    assert_eq!(
        blueprint.validate_against_catalog(&catalog).unwrap_err(),
        TacticBlueprintError::UnknownOption("missing".into())
    );
}

#[test]
fn blueprint_rejects_unbounded_or_meaningless_control_flow() {
    assert_eq!(
        TacticBlueprint::new(
            "unbounded",
            TacticBlueprintNode::Until {
                condition: condition(),
                max_iterations: 0,
                body: Box::new(invoke("wait")),
            },
        )
        .unwrap_err(),
        TacticBlueprintError::InvalidIterationBound
    );
    assert!(matches!(
        TacticBlueprint::new(
            "bad.guard",
            TacticBlueprintNode::Conditional {
                condition: OptionCondition::DurationElapsed,
                when_true: Box::new(invoke("wait")),
                when_false: Box::new(invoke("wait")),
            },
        )
        .unwrap_err(),
        TacticBlueprintError::InvalidCondition(_)
    ));
}

#[test]
fn sequence_and_layer_compile_to_one_exact_ordered_tape() {
    let entry = |id, source| TacticCatalogEntry::new(id, source).unwrap();
    let catalog = TacticAssetCatalog::new(vec![
            entry(
                "buttons",
                TacticAssetSource::ReactiveController(
                    ControllerProgram::parse(
                        "duskcontrol 1\nframes 2\nbuttons from 0 for 2 B\n",
                    )
                    .unwrap(),
                ),
            ),
            entry(
                "move",
                TacticAssetSource::ReactiveController(
                    ControllerProgram::parse(
                        "duskcontrol 1\nframes 2\nbezier replace from 0 for 2 p0 0 80 p1 0 80 p2 0 80 p3 0 80\n",
                    )
                    .unwrap(),
                ),
            ),
            entry(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            ),
        ])
        .unwrap();
    let blueprint = TacticBlueprint::new(
        "static.sequence-layer",
        TacticBlueprintNode::Sequence {
            steps: vec![
                invoke("shield"),
                TacticBlueprintNode::Layer {
                    layers: vec![invoke("move"), invoke("buttons")],
                },
            ],
        },
    )
    .unwrap();

    let compiled = blueprint.compile_static(&catalog).unwrap();
    assert_eq!(compiled.tape.frames.len(), 3);
    assert_eq!(compiled.tape.frames[0].pads[0].buttons, 0x0020);
    for frame in &compiled.tape.frames[1..] {
        assert_eq!(frame.pads[0].stick_y, 80);
        assert_eq!(frame.pads[0].buttons, 0x0200);
    }
    assert_eq!(compiled.segments.len(), 2);
    let CompiledStaticSegment::Invoke { execution, .. } = &compiled.segments[0] else {
        panic!("sequence prefix must remain an option execution")
    };
    execution.validate_against_tape(&compiled.tape).unwrap();
    assert_eq!(
        execution.realized_tape_range,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: 1,
        }
    );
    let CompiledStaticSegment::Layer {
        option_ids,
        tape_range,
        ..
    } = &compiled.segments[1]
    else {
        panic!("concurrent children must remain a layer segment")
    };
    assert_eq!(option_ids, &["move", "buttons"]);
    assert_eq!(
        *tape_range,
        TapeRange {
            start_frame: 1,
            end_frame_exclusive: 3,
        }
    );
    compiled.validate().unwrap();
}

#[test]
fn layer_rejects_ambiguous_writers_and_non_controller_children() {
    let program = |magnitude| {
        ControllerProgram::parse(&format!(
                "duskcontrol 1\nframes 1\nturn replace from 0 for 1 direction right magnitude {magnitude}\n"
            ))
            .unwrap()
    };
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new("first", TacticAssetSource::ReactiveController(program(20)))
            .unwrap(),
        TacticCatalogEntry::new("second", TacticAssetSource::ReactiveController(program(70)))
            .unwrap(),
        TacticCatalogEntry::new(
            "shield",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();
    let layer = TacticBlueprintNode::Layer {
        layers: vec![invoke("first"), invoke("second")],
    };
    assert!(matches!(
        compile_controller_layer(&layer, &catalog),
        Err(TacticBlueprintError::StaticExecution(message))
            if message.contains("replace stick layers")
    ));

    let invalid = TacticBlueprintNode::Layer {
        layers: vec![invoke("first"), invoke("shield")],
    };
    assert!(compile_controller_layer(&invalid, &catalog).is_err());
}

#[test]
fn sequence_does_not_insert_hidden_frames_to_rephase_a_roll() {
    let mut roll = RollOptionPlan::new(0, 100, 0);
    roll.spacing = RollSpacing {
        period_ticks: 2,
        phase_tick: 0,
    };
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new("roll", TacticAssetSource::Roll(roll)).unwrap(),
        TacticCatalogEntry::new(
            "shield",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();
    let blueprint = TacticBlueprint::new(
        "roll.phase",
        TacticBlueprintNode::Sequence {
            steps: vec![invoke("shield"), invoke("roll")],
        },
    )
    .unwrap();
    assert!(matches!(
        blueprint.compile_static(&catalog),
        Err(TacticBlueprintError::StaticExecution(_))
    ));
}

#[test]
fn execution_validation_rejects_conditions_absent_from_the_fact_registry() {
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "wait",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();
    let blueprint = TacticBlueprint::new(
        "fact.availability",
        TacticBlueprintNode::Conditional {
            condition: condition(),
            when_true: Box::new(invoke("wait")),
            when_false: Box::new(invoke("wait")),
        },
    )
    .unwrap();
    assert_eq!(
        blueprint
            .validate_for_execution(&catalog, |_| false)
            .unwrap_err(),
        TacticBlueprintError::UnavailableCondition
    );
    blueprint
        .validate_for_execution(&catalog, |available| available == &condition())
        .unwrap();
}

#[test]
fn applicability_enumeration_is_finite_concrete_and_schema_bound() {
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "shield.short",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
        TacticCatalogEntry::new(
            "shield.long",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 3 })),
        )
        .unwrap(),
        TacticCatalogEntry::new(
            "interact",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Interact {
                press_frames: 1,
                recovery_frames: 1,
            })),
        )
        .unwrap(),
    ])
    .unwrap();
    let blueprint = TacticBlueprint::new(
        "shield.then.interact",
        TacticBlueprintNode::Sequence {
            steps: vec![invoke("shield.short"), invoke("interact")],
        },
    )
    .unwrap();

    let choices =
        ApplicableTacticChoices::enumerate(&catalog, &[blueprint], |_| true, |_| Some(true))
            .unwrap();
    assert_eq!(
        choices
            .candidates
            .iter()
            .map(|choice| choice.choice_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "blueprint/shield.then.interact",
            "interact",
            "shield.long",
            "shield.short",
        ]
    );
    assert_eq!(choices.applicable_mask, vec![true; 4]);
    assert_ne!(choices.choice_schema_sha256, Digest::ZERO);
    assert_ne!(choices.applicability_sha256, Digest::ZERO);
    assert_eq!(
        choices.candidates[0].duration,
        TacticDurationBounds {
            minimum_ticks: 2,
            maximum_ticks: 3,
        }
    );
    let short = choices
        .candidates
        .iter()
        .find(|choice| choice.choice_id == "shield.short")
        .unwrap();
    let long = choices
        .candidates
        .iter()
        .find(|choice| choice.choice_id == "shield.long")
        .unwrap();
    assert_ne!(short.descriptor.parameters, long.descriptor.parameters);
}

#[test]
fn applicability_excludes_blueprints_with_unavailable_inputs() {
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "wait",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();
    let conditional = TacticBlueprint::new(
        "conditional",
        TacticBlueprintNode::Conditional {
            condition: condition(),
            when_true: Box::new(invoke("wait")),
            when_false: Box::new(invoke("wait")),
        },
    )
    .unwrap();

    let choices =
        ApplicableTacticChoices::enumerate(&catalog, &[conditional], |_| true, |_| None).unwrap();
    assert_eq!(choices.candidates.len(), 2);
    assert_eq!(
        choices
            .candidates
            .iter()
            .map(|choice| choice.choice_id.as_str())
            .collect::<Vec<_>>(),
        vec!["blueprint/conditional", "wait"]
    );
    assert_eq!(choices.applicable_mask, vec![false, true]);
}

#[test]
fn composite_applicability_uses_only_the_current_start_path() {
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "available",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
        TacticCatalogEntry::new(
            "blocked",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Interact {
                press_frames: 1,
                recovery_frames: 1,
            })),
        )
        .unwrap(),
    ])
    .unwrap();
    let sequence = TacticBlueprint::new(
        "sequence",
        TacticBlueprintNode::Sequence {
            steps: vec![invoke("available"), invoke("blocked")],
        },
    )
    .unwrap();
    let conditional = TacticBlueprint::new(
        "conditional",
        TacticBlueprintNode::Conditional {
            condition: condition(),
            when_true: Box::new(invoke("available")),
            when_false: Box::new(invoke("blocked")),
        },
    )
    .unwrap();
    let until = TacticBlueprint::new(
        "until",
        TacticBlueprintNode::Until {
            condition: condition(),
            max_iterations: 2,
            body: Box::new(invoke("available")),
        },
    )
    .unwrap();

    let choices = ApplicableTacticChoices::enumerate(
        &catalog,
        &[sequence, conditional, until],
        |description| description.option.option_id == "available",
        |_| Some(true),
    )
    .unwrap();
    assert_eq!(
        choices
            .candidates
            .iter()
            .map(|choice| choice.choice_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "available",
            "blocked",
            "blueprint/conditional",
            "blueprint/sequence",
            "blueprint/until",
        ]
    );
    assert_eq!(
        choices.applicable_mask,
        vec![true, false, true, true, false]
    );
}

#[test]
fn applicability_can_report_an_empty_bounded_choice_set() {
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "blocked",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();

    let choices = ApplicableTacticChoices::enumerate(&catalog, &[], |_| false, |_| None).unwrap();
    assert_eq!(choices.candidates.len(), 1);
    assert_eq!(choices.applicable_mask, vec![false]);
    assert_ne!(choices.choice_schema_sha256, Digest::ZERO);
    assert_ne!(choices.applicability_sha256, Digest::ZERO);
}
