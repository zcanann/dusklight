//! Build sealed, equal-budget negative controls for reachability experiments.

use super::*;

pub(super) fn prepare_materialized_rows(
    datasets: &[NativeGoalTrajectoryDataset],
    shards: &[NativeEpisodeShard],
    config: NativeGoalReachabilityConfig,
) -> Result<Vec<MaterializedRow>, NativeGoalReachabilityError> {
    config.validate()?;
    let observation_schemas = datasets
        .iter()
        .map(|dataset| dataset.observation_schema.as_str())
        .collect::<BTreeSet<_>>();
    let action_schemas = datasets
        .iter()
        .map(|dataset| dataset.action_schema.as_str())
        .collect::<BTreeSet<_>>();
    let demonstration_modes = datasets
        .iter()
        .map(|dataset| dataset.config.demonstration_mode)
        .collect::<BTreeSet<_>>();
    if observation_schemas.len() != 1 || action_schemas.len() != 1 || demonstration_modes.len() != 1
    {
        return Err(NativeGoalReachabilityError::new(
            "goal reachability datasets mix observation, action, or demonstration-mode contracts",
        ));
    }
    materialize(datasets, shards)
}

pub(super) fn apply_negative_control(
    rows: &mut [MaterializedRow],
    control: Option<NativeGoalReachabilityNegativeControl>,
    seed: u64,
) -> Result<ControlImpact, NativeGoalReachabilityError> {
    let Some(control) = control else {
        return Ok(ControlImpact {
            representation: NegativeControlRepresentation::FullyRepresented,
            representation_note: "unmodified_goal_conditioned_native_observation",
            changed_input_rows: 0,
            changed_input_cells: 0,
            changed_training_target_rows: 0,
        });
    };
    let mut changed_input_rows = 0;
    let mut changed_input_cells = 0;
    let mut changed_training_target_rows = 0;
    let (representation, representation_note) = match control {
        NativeGoalReachabilityNegativeControl::ShuffledOutcomes => {
            #[derive(Clone, Copy, PartialEq)]
            struct Outcome {
                success: bool,
                ticks_to_goal: Option<u32>,
                terminal_reward: f64,
                bootstrap_discount: f64,
                realized_return: f64,
                discounted_tick_cost: f64,
            }
            let training = rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| (row.split == AuxiliarySplit::Training).then_some(index))
                .collect::<Vec<_>>();
            let original = training
                .iter()
                .map(|index| {
                    let row = &rows[*index];
                    Outcome {
                        success: row.success,
                        ticks_to_goal: row.ticks_to_goal,
                        terminal_reward: row.terminal_reward,
                        bootstrap_discount: row.bootstrap_discount,
                        realized_return: row.realized_return,
                        discounted_tick_cost: row.discounted_tick_cost,
                    }
                })
                .collect::<Vec<_>>();
            let mut shuffled = original.clone();
            Rng::new(seed ^ 0x5348_5546_464c_4544).shuffle(&mut shuffled);
            for ((index, original), shuffled) in training.into_iter().zip(original).zip(shuffled) {
                let row = &mut rows[index];
                row.success = shuffled.success;
                row.ticks_to_goal = shuffled.ticks_to_goal;
                row.terminal_reward = shuffled.terminal_reward;
                row.bootstrap_discount = shuffled.bootstrap_discount;
                row.realized_return = shuffled.realized_return;
                row.discounted_tick_cost = shuffled.discounted_tick_cost;
                changed_training_target_rows += usize::from(original != shuffled);
            }
            if changed_training_target_rows == 0 {
                return Err(NativeGoalReachabilityError::new(
                    "shuffled-outcome control did not change any training target",
                ));
            }
            (
                NegativeControlRepresentation::FullyRepresented,
                "deterministic_rowwise_permutation_of_training_outcomes_only",
            )
        }
        NativeGoalReachabilityNegativeControl::ActionOnlyInput => {
            // The only action-valued part of the v1 observation vector is the
            // previous raw PAD at [31, 57). Remove state and goal conditioning.
            for row in rows.iter_mut() {
                let mut row_changes = 0;
                for (index, value) in row.features.iter_mut().enumerate() {
                    if !(31..57).contains(&index) && value.to_bits() != 0.0_f64.to_bits() {
                        *value = 0.0;
                        row_changes += 1;
                    }
                }
                changed_input_rows += usize::from(row_changes > 0);
                changed_input_cells += row_changes;
            }
            (
                NegativeControlRepresentation::ProxyOnly,
                "previous_pad_only;current_consumed_action_is_not_an_input",
            )
        }
        NativeGoalReachabilityNegativeControl::RemovedCollisionGeometry => {
            // v1 carries contact bits, ground/roof heights, and the last XZ
            // correction, but no collision mesh or surface geometry.
            for row in rows.iter_mut() {
                let mut row_changes = 0;
                for index in (11..20).chain(27..30) {
                    if row.features[index].to_bits() != 0.0_f64.to_bits() {
                        row.features[index] = 0.0;
                        row_changes += 1;
                    }
                }
                changed_input_rows += usize::from(row_changes > 0);
                changed_input_cells += row_changes;
            }
            (
                NegativeControlRepresentation::ProxyOnly,
                "removed_contact_height_and_correction_proxies;collision_geometry_is_absent",
            )
        }
        NativeGoalReachabilityNegativeControl::RemovedActors => (
            NegativeControlRepresentation::NotRepresented,
            "actor_set_and_non_player_actor_features_are_absent",
        ),
        NativeGoalReachabilityNegativeControl::RemovedHistory => {
            for row in rows.iter_mut() {
                let mut row_changes = 0;
                for value in &mut row.features[31..57] {
                    if value.to_bits() != 0.0_f64.to_bits() {
                        *value = 0.0;
                        row_changes += 1;
                    }
                }
                changed_input_rows += usize::from(row_changes > 0);
                changed_input_cells += row_changes;
            }
            (
                NegativeControlRepresentation::ProxyOnly,
                "removed_previous_pad;multi_step_observation_history_is_absent",
            )
        }
        NativeGoalReachabilityNegativeControl::RemovedCheckpointTapeIdentity => (
            NegativeControlRepresentation::NotRepresented,
            "checkpoint_and_tape_identifiers_are_excluded_from_model_input",
        ),
    };
    if representation == NegativeControlRepresentation::NotRepresented
        && (changed_input_rows != 0 || changed_input_cells != 0)
    {
        return Err(NativeGoalReachabilityError::new(
            "unrepresented negative control unexpectedly changed model input",
        ));
    }
    Ok(ControlImpact {
        representation,
        representation_note,
        changed_input_rows,
        changed_input_cells,
        changed_training_target_rows,
    })
}
