//! Temporal availability and change coverage for global native observations.
//!
//! The report contains counts only. It does not retain field values, define an
//! objective, or turn an observed transition into a preferred action.

use crate::native_episode_shard::{NativeEpisodeShard, NativeLearningObservation};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GlobalTemporalFieldCoverage {
    pub compared_pairs: u64,
    pub changed_pairs: u64,
    pub missing_pairs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StageGlobalTemporalCoverage {
    pub stage: String,
    pub transition_count: u64,
    pub context_change_count: u64,
    pub fields: BTreeMap<String, GlobalTemporalFieldCoverage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GlobalTemporalCoverage {
    pub boundary_count: u64,
    pub transition_count: u64,
    pub context_change_count: u64,
    pub fields: BTreeMap<String, GlobalTemporalFieldCoverage>,
    pub stages: Vec<StageGlobalTemporalCoverage>,
}

#[derive(Default)]
struct FieldAccumulator {
    compared_pairs: u64,
    changed_pairs: u64,
}

#[derive(Default)]
struct Accumulator {
    transition_count: u64,
    context_change_count: u64,
    fields: BTreeMap<String, FieldAccumulator>,
}

impl Accumulator {
    fn record(&mut self, field: &'static str, comparable: bool, changed: bool) {
        if comparable {
            let value = self.fields.entry(field.into()).or_default();
            value.compared_pairs += 1;
            value.changed_pairs += u64::from(changed);
        } else {
            self.fields.entry(field.into()).or_default();
        }
    }

    fn finish(self) -> BTreeMap<String, GlobalTemporalFieldCoverage> {
        self.fields
            .into_iter()
            .map(|(field, coverage)| {
                (
                    field,
                    GlobalTemporalFieldCoverage {
                        compared_pairs: coverage.compared_pairs,
                        changed_pairs: coverage.changed_pairs,
                        missing_pairs: self
                            .transition_count
                            .saturating_sub(coverage.compared_pairs),
                    },
                )
            })
            .collect()
    }
}

fn float_changed(left: f32, right: f32) -> bool {
    left.to_bits() != right.to_bits()
}

fn float_array_changed<const N: usize>(left: [f32; N], right: [f32; N]) -> bool {
    left.iter()
        .zip(right)
        .any(|(left, right)| float_changed(*left, right))
}

fn record_transition(
    accumulator: &mut Accumulator,
    before: &NativeLearningObservation,
    after: &NativeLearningObservation,
) {
    accumulator.transition_count += 1;
    let context_changed =
        before.stage != after.stage || before.room != after.room || before.layer != after.layer;
    accumulator.context_change_count += u64::from(context_changed);
    macro_rules! record {
        ($name:literal, $changed:expr) => {
            accumulator.record($name, true, $changed)
        };
    }
    macro_rules! record_optional {
        ($name:literal, $before:expr, $after:expr) => {{
            let left = $before;
            let right = $after;
            accumulator.record(
                $name,
                left.is_some() && right.is_some(),
                left.zip(right).is_some_and(|(left, right)| left != right),
            );
        }};
    }

    record!("stage", before.stage != after.stage);
    record!("room", before.room != after.room);
    record!("layer", before.layer != after.layer);
    record!("point", before.point != after.point);
    record!("next_stage", before.next_stage != after.next_stage);
    record!("next_room", before.next_room != after.next_room);
    record!("next_layer", before.next_layer != after.next_layer);
    record!("next_point", before.next_point != after.next_point);
    record!(
        "player_present",
        before.player_present != after.player_present
    );
    record!(
        "player_is_link",
        before.player_is_link != after.player_is_link
    );
    record!(
        "player_procedure",
        before.player_procedure != after.player_procedure
    );
    record!(
        "player_position",
        float_array_changed(before.player_position, after.player_position)
    );
    record!(
        "player_velocity",
        float_array_changed(before.player_velocity, after.player_velocity)
    );
    record!(
        "player_forward_speed",
        float_changed(before.player_forward_speed, after.player_forward_speed)
    );
    record!(
        "player_current_angle",
        before.player_current_angle != after.player_current_angle
    );
    record!(
        "player_shape_angle",
        before.player_shape_angle != after.player_shape_angle
    );
    record!(
        "player_mode_flags",
        before.player_mode_flags != after.player_mode_flags
    );
    record!(
        "player_damage_timers",
        before.player_damage_wait_timer != after.player_damage_wait_timer
            || before.player_ice_damage_wait_timer != after.player_ice_damage_wait_timer
            || before.player_sword_change_wait_timer != after.player_sword_change_wait_timer
    );
    record!(
        "player_do_status",
        before.player_do_status != after.player_do_status
    );
    record!(
        "player_contacts",
        before.player_contacts != after.player_contacts
    );
    record_optional!(
        "player_ground_height",
        before.player_ground_height.as_ref(),
        after.player_ground_height.as_ref()
    );
    record_optional!(
        "player_roof_height",
        before.player_roof_height.as_ref(),
        after.player_roof_height.as_ref()
    );
    record!("event_running", before.event_running != after.event_running);
    record!("event_id", before.event_id != after.event_id);
    record!("event_mode", before.event_mode != after.event_mode);
    record!("event_status", before.event_status != after.event_status);
    record!(
        "event_map_tool_id",
        before.event_map_tool_id != after.event_map_tool_id
    );
    record!(
        "event_name_hash",
        before.event_name_hash != after.event_name_hash
    );
    record!("menu_flags", before.menu_flags != after.menu_flags);
    record!(
        "menu_procedures",
        before.menu_procedures != after.menu_procedures
    );
    record!(
        "player_form_present",
        before.player_form_present != after.player_form_present
    );
    record!(
        "player_is_wolf",
        before.player_is_wolf != after.player_is_wolf
    );
    record!("rng_version", before.rng_version != after.rng_version);
    record!("rng_streams", before.rng_streams != after.rng_streams);
    record!("camera.status", before.camera_status != after.camera_status);
    record_optional!(
        "camera.value",
        before.camera.as_ref(),
        after.camera.as_ref()
    );
    record!(
        "player_action.status",
        before.player_action_status != after.player_action_status
    );
    record_optional!(
        "player_action.value",
        before.player_action.as_ref(),
        after.player_action.as_ref()
    );
    record!(
        "player_background_collision.status",
        before.player_background_collision_status != after.player_background_collision_status
    );
    record_optional!(
        "player_background_collision.value",
        before.player_background_collision.as_ref(),
        after.player_background_collision.as_ref()
    );
    record!(
        "player_collision_surfaces.status",
        before.player_collision_surfaces_status != after.player_collision_surfaces_status
    );
    record_optional!(
        "player_collision_surfaces.value",
        before.player_collision_surfaces.as_ref(),
        after.player_collision_surfaces.as_ref()
    );
    record!(
        "scene_exit.status",
        before.scene_exit_status != after.scene_exit_status
    );
    record_optional!(
        "scene_exit.value",
        before.scene_exit.as_ref(),
        after.scene_exit.as_ref()
    );
    record!(
        "dynamic_colliders.status",
        before.dynamic_colliders_status != after.dynamic_colliders_status
    );
    record!(
        "player_resources.status",
        before.player_resources_status != after.player_resources_status
    );
    record_optional!(
        "player_resources.value",
        before.player_resources.as_ref(),
        after.player_resources.as_ref()
    );
    record!(
        "player_relationships.status",
        before.player_relationships_status != after.player_relationships_status
    );
    record_optional!(
        "player_relationships.value",
        before.player_relationships.as_ref(),
        after.player_relationships.as_ref()
    );
    record!(
        "player_collision_solver.status",
        before.player_collision_solver_status != after.player_collision_solver_status
    );
    record_optional!(
        "player_collision_solver.value",
        before.player_collision_solver.as_ref(),
        after.player_collision_solver.as_ref()
    );
    record_optional!(
        "event_flags",
        before.event_flags.as_ref(),
        after.event_flags.as_ref()
    );
    record_optional!(
        "temporary_flags",
        before.temporary_flags.as_ref(),
        after.temporary_flags.as_ref()
    );
    record_optional!(
        "temporary_event_bytes",
        before.temporary_event_bytes.as_ref(),
        after.temporary_event_bytes.as_ref()
    );
    record_optional!(
        "dungeon_flags",
        before.dungeon_flags.as_ref(),
        after.dungeon_flags.as_ref()
    );
    record_optional!(
        "switch_flags",
        before.switch_flags.as_ref(),
        after.switch_flags.as_ref()
    );
    record!(
        "switch_flag_room",
        before.switch_flag_room != after.switch_flag_room
    );
    record!(
        "runtime_file.status",
        before.runtime_file_status != after.runtime_file_status
    );
    record_optional!(
        "runtime_file.value",
        before.runtime_file.as_ref(),
        after.runtime_file.as_ref()
    );
    record!(
        "return_place.status",
        before.return_place_status != after.return_place_status
    );
    record_optional!(
        "return_place.value",
        before.return_place.as_ref(),
        after.return_place.as_ref()
    );
    record!(
        "restart.status",
        before.restart_status != after.restart_status
    );
    record_optional!(
        "restart.value",
        before.restart.as_ref(),
        after.restart.as_ref()
    );
    record!(
        "event_handoff.status",
        before.event_handoff_status != after.event_handoff_status
    );
    record_optional!(
        "event_handoff.value",
        before.event_handoff.as_ref(),
        after.event_handoff.as_ref()
    );
    record!(
        "message_session.status",
        before.message_session_status != after.message_session_status
    );
    record_optional!(
        "message_session.value",
        before.message_session.as_ref(),
        after.message_session.as_ref()
    );
}

pub fn inspect_global_temporal_coverage(shards: &[NativeEpisodeShard]) -> GlobalTemporalCoverage {
    let mut boundary_count = 0_u64;
    let mut aggregate = Accumulator::default();
    let mut stages = BTreeMap::<String, Accumulator>::new();
    for shard in shards {
        for episode in &shard.episodes {
            boundary_count += episode.steps.len() as u64 + 1;
            let mut before = &episode.steps[0].pre_input;
            for step in &episode.steps {
                let after = &step.post_simulation;
                record_transition(&mut aggregate, before, after);
                record_transition(
                    stages.entry(before.stage.clone()).or_default(),
                    before,
                    after,
                );
                before = after;
            }
        }
    }
    let transition_count = aggregate.transition_count;
    let context_change_count = aggregate.context_change_count;
    GlobalTemporalCoverage {
        boundary_count,
        transition_count,
        context_change_count,
        fields: aggregate.finish(),
        stages: stages
            .into_iter()
            .map(|(stage, accumulator)| StageGlobalTemporalCoverage {
                stage,
                transition_count: accumulator.transition_count,
                context_change_count: accumulator.context_change_count,
                fields: accumulator.finish(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_changed_constant_and_missing_transition_fields() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v14.dseps");
        let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        let step = &mut shard.episodes[0].steps[0];
        step.post_simulation.event_running = !step.pre_input.event_running;
        step.post_simulation.player_position[0] += 1.0;
        step.post_simulation.camera = None;

        let report = inspect_global_temporal_coverage(&[shard]);
        assert_eq!(report.boundary_count, 2);
        assert_eq!(report.transition_count, 1);
        assert_eq!(report.fields["event_running"].changed_pairs, 1);
        assert_eq!(report.fields["player_position"].changed_pairs, 1);
        assert_eq!(report.fields["event_id"].changed_pairs, 0);
        assert_eq!(report.fields["camera.value"].compared_pairs, 0);
        assert_eq!(report.fields["camera.value"].missing_pairs, 1);
        assert_eq!(report.stages.len(), 1);
    }

    #[test]
    fn attributes_context_changes_to_the_source_stage() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v14.dseps");
        let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        let source = shard.episodes[0].steps[0].pre_input.stage.clone();
        shard.episodes[0].steps[0].post_simulation.stage = "NEXT".into();

        let report = inspect_global_temporal_coverage(&[shard]);
        assert_eq!(report.context_change_count, 1);
        assert_eq!(report.stages[0].stage, source);
        assert_eq!(report.stages[0].context_change_count, 1);
        assert_eq!(report.stages[0].fields["stage"].changed_pairs, 1);
    }
}
