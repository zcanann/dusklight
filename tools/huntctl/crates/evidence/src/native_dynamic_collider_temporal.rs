//! Temporal coverage for the complete dynamic-collider set in native episodes.
//!
//! This is an observation audit, not a contact objective. It reports which
//! typed collider facts changed and whether ownership can be joined to the
//! complete actor set without retaining raw values or proposing actions.

use crate::native_episode_shard::{
    NativeDynamicColliderObservation, NativeDynamicColliderShape, NativeEpisodeShard,
    NativeLearningObservation,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DynamicColliderOwnerTemporalCoverage {
    pub owner_status: String,
    pub owner_profile_name: Option<i16>,
    pub shapes: Vec<String>,
    pub boundary_samples: u64,
    pub persistent_transition_pairs: u64,
    pub in_context_appearances: u64,
    pub in_context_disappearances: u64,
    pub context_change_appearances: u64,
    pub context_change_disappearances: u64,
    pub changed_fields: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DynamicColliderTemporalCoverage {
    pub boundary_count: u64,
    pub compared_transition_count: u64,
    pub collider_boundary_samples: u64,
    pub persistent_transition_pairs: u64,
    pub in_context_appearances: u64,
    pub in_context_disappearances: u64,
    pub context_change_appearances: u64,
    pub context_change_disappearances: u64,
    pub duplicate_identity_boundaries: u64,
    pub unresolved_owner_samples: u64,
    pub owner_groups: Vec<DynamicColliderOwnerTemporalCoverage>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum OwnerGroupKey {
    Joined(i16),
    NoOwner,
    Unresolved,
}

#[derive(Default)]
struct OwnerGroupAccumulator {
    shapes: BTreeSet<String>,
    boundary_samples: u64,
    persistent_transition_pairs: u64,
    in_context_appearances: u64,
    in_context_disappearances: u64,
    context_change_appearances: u64,
    context_change_disappearances: u64,
    changed_fields: BTreeMap<String, u64>,
}

#[derive(Default)]
struct Accumulator {
    boundary_count: u64,
    compared_transition_count: u64,
    collider_boundary_samples: u64,
    persistent_transition_pairs: u64,
    in_context_appearances: u64,
    in_context_disappearances: u64,
    context_change_appearances: u64,
    context_change_disappearances: u64,
    duplicate_identity_boundaries: u64,
    unresolved_owner_samples: u64,
    owner_groups: BTreeMap<OwnerGroupKey, OwnerGroupAccumulator>,
}

impl Accumulator {
    fn finish(self) -> DynamicColliderTemporalCoverage {
        DynamicColliderTemporalCoverage {
            boundary_count: self.boundary_count,
            compared_transition_count: self.compared_transition_count,
            collider_boundary_samples: self.collider_boundary_samples,
            persistent_transition_pairs: self.persistent_transition_pairs,
            in_context_appearances: self.in_context_appearances,
            in_context_disappearances: self.in_context_disappearances,
            context_change_appearances: self.context_change_appearances,
            context_change_disappearances: self.context_change_disappearances,
            duplicate_identity_boundaries: self.duplicate_identity_boundaries,
            unresolved_owner_samples: self.unresolved_owner_samples,
            owner_groups: self
                .owner_groups
                .into_iter()
                .map(|(key, group)| {
                    let (owner_status, owner_profile_name) = match key {
                        OwnerGroupKey::Joined(profile) => ("joined", Some(profile)),
                        OwnerGroupKey::NoOwner => ("no_owner", None),
                        OwnerGroupKey::Unresolved => ("unresolved", None),
                    };
                    DynamicColliderOwnerTemporalCoverage {
                        owner_status: owner_status.into(),
                        owner_profile_name,
                        shapes: group.shapes.into_iter().collect(),
                        boundary_samples: group.boundary_samples,
                        persistent_transition_pairs: group.persistent_transition_pairs,
                        in_context_appearances: group.in_context_appearances,
                        in_context_disappearances: group.in_context_disappearances,
                        context_change_appearances: group.context_change_appearances,
                        context_change_disappearances: group.context_change_disappearances,
                        changed_fields: group.changed_fields,
                    }
                })
                .collect(),
        }
    }
}

fn shape_name(shape: NativeDynamicColliderShape) -> &'static str {
    match shape {
        NativeDynamicColliderShape::Unknown => "unknown",
        NativeDynamicColliderShape::Sphere => "sphere",
        NativeDynamicColliderShape::Cylinder => "cylinder",
    }
}

fn owner_group(
    observation: &NativeLearningObservation,
    collider: &NativeDynamicColliderObservation,
) -> OwnerGroupKey {
    let Some(generation) = collider.owner_runtime_generation else {
        return OwnerGroupKey::NoOwner;
    };
    observation
        .actors
        .iter()
        .find(|actor| actor.runtime_generation == u64::from(generation))
        .map_or(OwnerGroupKey::Unresolved, |actor| {
            OwnerGroupKey::Joined(actor.profile_name)
        })
}

fn float_changed(left: f32, right: f32) -> bool {
    left.to_bits() != right.to_bits()
}

fn float_array_changed<const N: usize>(left: [f32; N], right: [f32; N]) -> bool {
    left.iter()
        .zip(right)
        .any(|(left, right)| float_changed(*left, right))
}

fn changed(group: &mut OwnerGroupAccumulator, field: &'static str, value: bool) {
    if value {
        *group.changed_fields.entry(field.into()).or_default() += 1;
    }
}

fn record_changes(
    group: &mut OwnerGroupAccumulator,
    before: &NativeDynamicColliderObservation,
    after: &NativeDynamicColliderObservation,
) {
    changed(
        group,
        "owner_runtime_generation",
        before.owner_runtime_generation != after.owner_runtime_generation,
    );
    changed(
        group,
        "attack_hit_owner_runtime_generation",
        before.attack_hit_owner_runtime_generation != after.attack_hit_owner_runtime_generation,
    );
    changed(
        group,
        "target_hit_owner_runtime_generation",
        before.target_hit_owner_runtime_generation != after.target_hit_owner_runtime_generation,
    );
    changed(
        group,
        "correction_hit_owner_runtime_generation",
        before.correction_hit_owner_runtime_generation
            != after.correction_hit_owner_runtime_generation,
    );
    changed(
        group,
        "status_present",
        before.status_present != after.status_present,
    );
    changed(
        group,
        "shape_present",
        before.shape_present != after.shape_present,
    );
    changed(group, "attack_set", before.attack_set != after.attack_set);
    changed(group, "target_set", before.target_set != after.target_set);
    changed(
        group,
        "correction_set",
        before.correction_set != after.correction_set,
    );
    changed(group, "attack_hit", before.attack_hit != after.attack_hit);
    changed(group, "target_hit", before.target_hit != after.target_hit);
    changed(
        group,
        "correction_hit",
        before.correction_hit != after.correction_hit,
    );
    changed(group, "shape", before.shape != after.shape);
    changed(
        group,
        "attack_type",
        before.attack_type != after.attack_type,
    );
    changed(
        group,
        "target_type",
        before.target_type != after.target_type,
    );
    changed(
        group,
        "attack_source_parameters",
        before.attack_source_parameters != after.attack_source_parameters,
    );
    changed(
        group,
        "attack_result_parameters",
        before.attack_result_parameters != after.attack_result_parameters,
    );
    changed(
        group,
        "target_source_parameters",
        before.target_source_parameters != after.target_source_parameters,
    );
    changed(
        group,
        "target_result_parameters",
        before.target_result_parameters != after.target_result_parameters,
    );
    changed(
        group,
        "correction_source_parameters",
        before.correction_source_parameters != after.correction_source_parameters,
    );
    changed(
        group,
        "correction_result_parameters",
        before.correction_result_parameters != after.correction_result_parameters,
    );
    changed(
        group,
        "attack_power",
        before.attack_power != after.attack_power,
    );
    changed(group, "weight", before.weight != after.weight);
    changed(group, "damage", before.damage != after.damage);
    changed(
        group,
        "center",
        float_array_changed(before.center, after.center),
    );
    changed(group, "radius", float_changed(before.radius, after.radius));
    changed(group, "height", float_changed(before.height, after.height));
    changed(
        group,
        "aabb_min",
        float_array_changed(before.aabb_min, after.aabb_min),
    );
    changed(
        group,
        "aabb_max",
        float_array_changed(before.aabb_max, after.aabb_max),
    );
    changed(
        group,
        "correction",
        float_array_changed(before.correction, after.correction),
    );
}

fn collider_map(
    observation: &NativeLearningObservation,
) -> (
    BTreeMap<(u16, Option<u32>), &NativeDynamicColliderObservation>,
    bool,
) {
    let mut colliders = BTreeMap::new();
    let mut duplicate = false;
    for collider in &observation.dynamic_colliders {
        duplicate |= colliders
            .insert(
                (
                    collider.registration_index,
                    collider.owner_runtime_generation,
                ),
                collider,
            )
            .is_some();
    }
    (colliders, duplicate)
}

pub fn inspect_dynamic_collider_temporal_coverage(
    shards: &[NativeEpisodeShard],
) -> DynamicColliderTemporalCoverage {
    let mut accumulator = Accumulator::default();
    for shard in shards {
        for episode in &shard.episodes {
            let mut boundaries = Vec::with_capacity(episode.steps.len() + 1);
            boundaries.push(&episode.steps[0].pre_input);
            boundaries.extend(episode.steps.iter().map(|step| &step.post_simulation));
            accumulator.boundary_count += boundaries.len() as u64;
            for observation in &boundaries {
                accumulator.collider_boundary_samples += observation.dynamic_colliders.len() as u64;
                accumulator.duplicate_identity_boundaries += u64::from(collider_map(observation).1);
                for collider in &observation.dynamic_colliders {
                    let key = owner_group(observation, collider);
                    accumulator.unresolved_owner_samples +=
                        u64::from(key == OwnerGroupKey::Unresolved);
                    let group = accumulator.owner_groups.entry(key).or_default();
                    group.boundary_samples += 1;
                    group.shapes.insert(shape_name(collider.shape).into());
                }
            }
            for pair in boundaries.windows(2) {
                let before = pair[0];
                let after = pair[1];
                accumulator.compared_transition_count += 1;
                let same_context = before.stage == after.stage
                    && before.room == after.room
                    && before.layer == after.layer;
                let before_by_id = collider_map(before).0;
                let after_by_id = collider_map(after).0;
                for (identity, collider) in &after_by_id {
                    let key = owner_group(after, collider);
                    let group = accumulator.owner_groups.entry(key).or_default();
                    if let Some(previous) = before_by_id.get(identity) {
                        accumulator.persistent_transition_pairs += 1;
                        group.persistent_transition_pairs += 1;
                        record_changes(group, previous, collider);
                    } else if same_context {
                        accumulator.in_context_appearances += 1;
                        group.in_context_appearances += 1;
                    } else {
                        accumulator.context_change_appearances += 1;
                        group.context_change_appearances += 1;
                    }
                }
                for (identity, collider) in &before_by_id {
                    if !after_by_id.contains_key(identity) {
                        let key = owner_group(before, collider);
                        let group = accumulator.owner_groups.entry(key).or_default();
                        if same_context {
                            accumulator.in_context_disappearances += 1;
                            group.in_context_disappearances += 1;
                        } else {
                            accumulator.context_change_disappearances += 1;
                            group.context_change_disappearances += 1;
                        }
                    }
                }
            }
        }
    }
    accumulator.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_lifecycle_and_typed_changes_without_raw_values() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v14.dseps");
        let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        let step = &mut shard.episodes[0].steps[0];
        let persistent = step.pre_input.dynamic_colliders[0].clone();
        step.post_simulation.dynamic_colliders = vec![persistent.clone()];
        step.post_simulation.dynamic_colliders[0].attack_hit =
            !step.post_simulation.dynamic_colliders[0].attack_hit;
        step.post_simulation.dynamic_colliders[0].correction[0] += 1.0;

        let mut disappeared = persistent.clone();
        disappeared.registration_index = 60_000;
        step.pre_input.dynamic_colliders.push(disappeared);
        step.pre_input
            .dynamic_colliders
            .sort_by_key(|collider| collider.registration_index);

        let mut appeared = persistent;
        appeared.registration_index = 60_001;
        step.post_simulation.dynamic_colliders.push(appeared);
        step.post_simulation
            .dynamic_colliders
            .sort_by_key(|collider| collider.registration_index);

        let report = inspect_dynamic_collider_temporal_coverage(&[shard]);
        assert_eq!(report.boundary_count, 2);
        assert_eq!(report.compared_transition_count, 1);
        assert_eq!(report.persistent_transition_pairs, 1);
        assert_eq!(report.in_context_appearances, 1);
        assert_eq!(report.in_context_disappearances, 1);
        assert_eq!(report.context_change_appearances, 0);
        assert_eq!(report.context_change_disappearances, 0);
        assert_eq!(report.duplicate_identity_boundaries, 0);
        let changed = report
            .owner_groups
            .iter()
            .find(|group| group.persistent_transition_pairs == 1)
            .unwrap();
        assert_eq!(changed.changed_fields["attack_hit"], 1);
        assert_eq!(changed.changed_fields["correction"], 1);
        assert!(!changed.changed_fields.contains_key("damage"));
    }

    #[test]
    fn reports_unresolved_owner_joins_and_duplicate_identities() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v14.dseps");
        let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        let observation = &mut shard.episodes[0].steps[0].pre_input;
        let mut collider = observation.dynamic_colliders[0].clone();
        collider.owner_runtime_generation = Some(u32::MAX);
        observation.dynamic_colliders = vec![collider.clone(), collider];

        let report = inspect_dynamic_collider_temporal_coverage(&[shard]);
        assert!(report.unresolved_owner_samples >= 2);
        assert!(report.duplicate_identity_boundaries >= 1);
        assert!(
            report
                .owner_groups
                .iter()
                .any(|group| group.owner_status == "unresolved")
        );
    }
}
