use super::scratch_discovery::route_report_sha256;
use super::*;
use dusklight_learning::generalized_tactic_value::{
    GeneralizedTacticContext, generalized_tactic_action_factors,
};

pub const NATIVE_TACTIC_OBSERVATION_AUDIT_SCHEMA_V1: &str =
    "dusklight-native-tactic-observation-audit/v1";

const BUTTON_L: u16 = 0x0040;
const BUTTON_A: u16 = 0x0100;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticSignalCoverage {
    pub signal: String,
    pub feature_names: Vec<String>,
    pub observed_snapshots: u64,
    pub native_supported_snapshots: u64,
    pub feature_available_snapshots: u64,
    pub availability_mismatches: u64,
    pub distinct_supported_measurements: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticActionSurfaceCoverage {
    pub decisions: u64,
    pub complete_decisions: u64,
    pub source_snapshots_resolved: u64,
    pub applicable_descriptors: u64,
    pub unique_descriptors: u64,
    pub untyped_descriptors: u64,
    pub selected_descriptors: u64,
    pub roll_available_decisions: u64,
    pub roll_selected_decisions: u64,
    pub a_button_available_decisions: u64,
    pub a_button_selected_decisions: u64,
    pub camera_modifier_available_decisions: u64,
    pub camera_modifier_selected_decisions: u64,
    pub option_type_availability: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticPolicySignalContract {
    pub goal_coordinate_is_objective_derived: bool,
    pub atomic_goal_target_only: bool,
    pub authored_route_coordinates_used: bool,
    pub authored_route_ids: Vec<String>,
    pub route_sequence_count: u64,
    pub benchmark_specific_feature_names: Vec<String>,
    pub passed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticObservationAudit {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub route_report_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub corpus_sha256: Vec<Digest>,
    pub unique_native_transitions: u64,
    pub unique_native_snapshots: u64,
    pub compared_feature_vectors: u64,
    pub exact_feature_vectors: u64,
    pub feature_vector_mismatches: u64,
    pub signals: Vec<NativeTacticSignalCoverage>,
    pub action_surface: NativeTacticActionSurfaceCoverage,
    pub policy_signal_contract: NativeTacticPolicySignalContract,
    pub passed: bool,
}

impl NativeTacticObservationAudit {
    pub fn build(
        request: &OptimizationRequest,
        route: &NativeTacticRouteReport,
        corpora: &[(Digest, TacticQTrainingCorpus)],
    ) -> Result<Self, NativeTacticRouteRunError> {
        request.validate().map_err(route_error)?;
        if route.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V34
            || route.optimization_request_sha256 != request.content_sha256
            || route.execution_plan_sha256 == Digest::ZERO
            || route.feature_schema_sha256 == Digest::ZERO
            || route.action_schema_sha256 == Digest::ZERO
            || corpora.is_empty()
        {
            return Err(route_message(
                "observation audit requires a current native route report and training corpora",
            ));
        }
        let encoder = GoalConditionedTacticFeatureEncoder::new(route.goal_target.coordinate)
            .map_err(route_error)?;
        if encoder.schema_sha256 != route.feature_schema_sha256 {
            return Err(route_message(
                "route report feature schema differs from the current tactic encoder",
            ));
        }

        let mut corpus_sha256 = BTreeSet::new();
        let mut root_checkpoint_sha256 = BTreeSet::new();
        let mut transitions = BTreeMap::new();
        for (identity, corpus) in corpora {
            validate_training_corpus(corpus).map_err(route_error)?;
            if *identity == Digest::ZERO
                || corpus.execution_authority_sha256 != route.execution_plan_sha256
                || corpus.feature_schema_sha256 != route.feature_schema_sha256
                || corpus.objective_sha256 != route.objective_sha256
            {
                return Err(route_message(
                    "observation audit corpus is detached from the native route report",
                ));
            }
            corpus_sha256.insert(*identity);
            root_checkpoint_sha256.insert(corpus.root_checkpoint_sha256);
            for transition in &corpus.transitions {
                let identity = transition.replay_identity_sha256().map_err(route_error)?;
                match transitions.entry(identity) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(transition.clone());
                    }
                    std::collections::btree_map::Entry::Occupied(entry)
                        if entry.get() != transition =>
                    {
                        return Err(route_message(
                            "equal replay identities contain different native transitions",
                        ));
                    }
                    std::collections::btree_map::Entry::Occupied(_) => {}
                }
            }
        }
        if transitions.is_empty() {
            return Err(route_message(
                "observation audit has no authenticated native transitions",
            ));
        }
        if root_checkpoint_sha256.len() != 1 || root_checkpoint_sha256.contains(&Digest::ZERO) {
            return Err(route_message(
                "observation audit corpora do not share one native root checkpoint",
            ));
        }
        let root_checkpoint_sha256 = *root_checkpoint_sha256
            .first()
            .expect("one checked root checkpoint");

        let mut snapshots = BTreeMap::<Digest, (FactSnapshot, Vec<f32>)>::new();
        let mut compared_feature_vectors = 0_u64;
        let mut exact_feature_vectors = 0_u64;
        for transition in transitions.values() {
            for (snapshot, retained) in [
                (&transition.before, &transition.value_sample.state),
                (&transition.after, &transition.value_sample.next_state),
            ] {
                let encoded = encoder.encode(snapshot).map_err(route_error)?;
                compared_feature_vectors = compared_feature_vectors.saturating_add(1);
                exact_feature_vectors = exact_feature_vectors
                    .saturating_add(u64::from(float_vectors_equal(&encoded, retained)));
                let snapshot_sha256 = snapshot.content_sha256().map_err(route_error)?;
                match snapshots.entry(snapshot_sha256) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert((snapshot.clone(), encoded));
                    }
                    std::collections::btree_map::Entry::Occupied(entry)
                        if entry.get().0 != *snapshot =>
                    {
                        return Err(route_message(
                            "equal snapshot identities contain different native facts",
                        ));
                    }
                    std::collections::btree_map::Entry::Occupied(_) => {}
                }
            }
        }
        let feature_indices = encoder
            .feature_names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        let signal_inputs = snapshots.values().collect::<Vec<_>>();
        let signals = vec![
            signal_coverage(
                "player_velocity",
                &[
                    "velocity_available",
                    "velocity_x",
                    "velocity_y",
                    "velocity_z",
                ],
                &signal_inputs,
                &feature_indices,
                |facts| facts.player.velocity_f32_bits.is_some(),
            )?,
            signal_coverage(
                "past_trajectory",
                &[
                    "trajectory_available",
                    "trajectory_planar_path_length",
                    "trajectory_planar_displacement",
                    "trajectory_straightness",
                    "trajectory_mean_planar_speed",
                    "trajectory_stalled_command_fraction",
                    "trajectory_speed_retention",
                ],
                &signal_inputs,
                &feature_indices,
                has_same_world_history,
            )?,
            signal_coverage(
                "camera_yaw",
                &["camera_yaw_available", "camera_yaw_sin", "camera_yaw_cos"],
                &signal_inputs,
                &feature_indices,
                |facts| facts.player.camera_yaw_radians_f32_bits.is_some(),
            )?,
            signal_coverage(
                "prompted_action_state",
                &[
                    "player_action_available",
                    "player_do_prompt_available",
                    "player_do_status",
                    "player_front_roll_prompt_available",
                ],
                &signal_inputs,
                &feature_indices,
                |facts| {
                    facts.player.action_state.is_some() || !facts.player.action_lanes.is_empty()
                },
            )?,
            signal_coverage(
                "recent_option_kinematics",
                &[
                    "recent_option_trajectory_available",
                    "recent_option_wall_contact_fraction",
                    "recent_option_momentum_loss_per_tick",
                    "recent_option_collision_correction_per_tick",
                ],
                &signal_inputs,
                &feature_indices,
                |facts| {
                    facts
                        .recent_option
                        .as_ref()
                        .and_then(|option| option.trajectory.as_ref())
                        .is_some()
                },
            )?,
            signal_coverage(
                "contact_correlated_slowdown",
                &[
                    "recent_option_contact_slowdown_available",
                    "recent_option_contact_commanded_fraction",
                    "recent_option_contact_momentum_loss_per_command_tick",
                ],
                &signal_inputs,
                &feature_indices,
                |facts| {
                    facts
                        .recent_option
                        .as_ref()
                        .and_then(|option| option.trajectory.as_ref())
                        .is_some_and(|trajectory| {
                            trajectory.wall_contact_commanded_motion_ticks.is_some()
                                && trajectory
                                    .wall_contact_commanded_momentum_loss_f32_bits
                                    .is_some()
                        })
                },
            )?,
        ];
        let action_surface = action_surface_coverage(route, &snapshots)?;
        let benchmark_specific_feature_names = encoder
            .feature_names
            .iter()
            .filter(|name| benchmark_specific_feature_name(name))
            .cloned()
            .collect::<Vec<_>>();
        let atomic_goal_target_only = route.goal_target.route_sequences.is_empty()
            && route.goal_target.authored_route_ids.is_empty()
            && route.goal_target.tactic_targets.len() == 1
            && vec3_bits_equal(
                route.goal_target.tactic_targets[0],
                route.goal_target.coordinate,
            );
        let policy_signal_contract = NativeTacticPolicySignalContract {
            goal_coordinate_is_objective_derived: route.goal_target.supporting_load_triggers > 0
                && route.goal_target.source_inventory_sha256 != Digest::ZERO,
            atomic_goal_target_only,
            authored_route_coordinates_used: route.goal_target.authored_route_coordinates_used,
            authored_route_ids: route.goal_target.authored_route_ids.clone(),
            route_sequence_count: route.goal_target.route_sequences.len() as u64,
            benchmark_specific_feature_names,
            passed: false,
        };
        let mut report = Self {
            schema: NATIVE_TACTIC_OBSERVATION_AUDIT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: request.content_sha256,
            route_report_sha256: route_report_sha256(route)?,
            execution_plan_sha256: route.execution_plan_sha256,
            feature_schema_sha256: route.feature_schema_sha256,
            action_schema_sha256: route.action_schema_sha256,
            root_checkpoint_sha256,
            corpus_sha256: corpus_sha256.into_iter().collect(),
            unique_native_transitions: transitions.len() as u64,
            unique_native_snapshots: snapshots.len() as u64,
            compared_feature_vectors,
            exact_feature_vectors,
            feature_vector_mismatches: compared_feature_vectors
                .saturating_sub(exact_feature_vectors),
            signals,
            action_surface,
            policy_signal_contract,
            passed: false,
        };
        report.policy_signal_contract.passed = report
            .policy_signal_contract
            .goal_coordinate_is_objective_derived
            && report.policy_signal_contract.atomic_goal_target_only
            && !report
                .policy_signal_contract
                .authored_route_coordinates_used
            && report
                .policy_signal_contract
                .benchmark_specific_feature_names
                .is_empty();
        report.passed = report.feature_vector_mismatches == 0
            && report.signals.iter().all(|signal| {
                signal.native_supported_snapshots > 0 && signal.availability_mismatches == 0
            })
            && report.action_surface.decisions > 0
            && report.action_surface.complete_decisions == report.action_surface.decisions
            && report.action_surface.source_snapshots_resolved == report.action_surface.decisions
            && report.action_surface.untyped_descriptors == 0
            && report.action_surface.selected_descriptors == report.action_surface.decisions
            && report.action_surface.roll_available_decisions > 0
            && report.action_surface.a_button_available_decisions > 0
            && report.action_surface.camera_modifier_available_decisions > 0
            && report.policy_signal_contract.passed;
        report.content_sha256 = report.digest()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_OBSERVATION_AUDIT_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.digest()?
            || self.optimization_request_sha256 == Digest::ZERO
            || self.route_report_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.action_schema_sha256 == Digest::ZERO
            || self.root_checkpoint_sha256 == Digest::ZERO
            || self.corpus_sha256.is_empty()
            || self.corpus_sha256.contains(&Digest::ZERO)
            || !strictly_increasing(&self.corpus_sha256)
            || self.unique_native_transitions == 0
            || self.unique_native_snapshots == 0
            || self.compared_feature_vectors == 0
            || self.exact_feature_vectors > self.compared_feature_vectors
            || self.feature_vector_mismatches
                != self
                    .compared_feature_vectors
                    .saturating_sub(self.exact_feature_vectors)
            || self.signals.len() != 6
            || self
                .signals
                .iter()
                .map(|signal| signal.signal.as_str())
                .ne([
                    "player_velocity",
                    "past_trajectory",
                    "camera_yaw",
                    "prompted_action_state",
                    "recent_option_kinematics",
                    "contact_correlated_slowdown",
                ])
            || self.signals.iter().any(|signal| {
                signal.observed_snapshots != self.unique_native_snapshots
                    || signal.feature_names.is_empty()
                    || signal.feature_available_snapshots > signal.observed_snapshots
                    || signal.native_supported_snapshots > signal.observed_snapshots
                    || signal.availability_mismatches > signal.observed_snapshots
                    || signal.distinct_supported_measurements > signal.native_supported_snapshots
            })
            || self.action_surface.complete_decisions > self.action_surface.decisions
            || self.action_surface.source_snapshots_resolved > self.action_surface.decisions
            || self.action_surface.untyped_descriptors > self.action_surface.applicable_descriptors
            || self.action_surface.unique_descriptors
                > self
                    .action_surface
                    .applicable_descriptors
                    .saturating_sub(self.action_surface.untyped_descriptors)
            || self.action_surface.selected_descriptors > self.action_surface.applicable_descriptors
            || self.action_surface.roll_available_decisions > self.action_surface.decisions
            || self.action_surface.roll_selected_decisions
                > self.action_surface.roll_available_decisions
            || self.action_surface.a_button_available_decisions > self.action_surface.decisions
            || self.action_surface.a_button_selected_decisions
                > self.action_surface.a_button_available_decisions
            || self.action_surface.camera_modifier_available_decisions
                > self.action_surface.decisions
            || self.action_surface.camera_modifier_selected_decisions
                > self.action_surface.camera_modifier_available_decisions
            || self.policy_signal_contract.passed
                != (self
                    .policy_signal_contract
                    .goal_coordinate_is_objective_derived
                    && self.policy_signal_contract.atomic_goal_target_only
                    && !self.policy_signal_contract.authored_route_coordinates_used
                    && self
                        .policy_signal_contract
                        .benchmark_specific_feature_names
                        .is_empty())
            || self.passed
                != (self.feature_vector_mismatches == 0
                    && self.signals.iter().all(|signal| {
                        signal.native_supported_snapshots > 0 && signal.availability_mismatches == 0
                    })
                    && self.action_surface.decisions > 0
                    && self.action_surface.complete_decisions == self.action_surface.decisions
                    && self.action_surface.source_snapshots_resolved
                        == self.action_surface.decisions
                    && self.action_surface.untyped_descriptors == 0
                    && self.action_surface.selected_descriptors == self.action_surface.decisions
                    && self.action_surface.roll_available_decisions > 0
                    && self.action_surface.a_button_available_decisions > 0
                    && self.action_surface.camera_modifier_available_decisions > 0
                    && self.policy_signal_contract.passed)
        {
            return Err(route_message(
                "native tactic observation audit is invalid or detached",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeTacticRouteRunError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(route_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn digest(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_json::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

fn signal_coverage<F>(
    signal: &str,
    feature_names: &[&str],
    snapshots: &[&(FactSnapshot, Vec<f32>)],
    feature_indices: &BTreeMap<&str, usize>,
    supported: F,
) -> Result<NativeTacticSignalCoverage, NativeTacticRouteRunError>
where
    F: Fn(&FactSnapshot) -> bool,
{
    let indices = feature_names
        .iter()
        .map(|name| {
            feature_indices
                .get(name)
                .copied()
                .ok_or_else(|| route_message(format!("required tactic feature {name:?} is absent")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let availability_index = indices[0];
    let mut native_supported_snapshots = 0_u64;
    let mut feature_available_snapshots = 0_u64;
    let mut availability_mismatches = 0_u64;
    let mut measurements = BTreeSet::<Vec<u32>>::new();
    for (facts, encoded) in snapshots {
        let expected = supported(facts);
        let available = encoded[availability_index] >= 0.5;
        native_supported_snapshots = native_supported_snapshots.saturating_add(u64::from(expected));
        feature_available_snapshots =
            feature_available_snapshots.saturating_add(u64::from(available));
        availability_mismatches =
            availability_mismatches.saturating_add(u64::from(expected != available));
        if expected {
            measurements.insert(
                indices
                    .iter()
                    .skip(1)
                    .map(|index| encoded[*index].to_bits())
                    .collect(),
            );
        }
    }
    Ok(NativeTacticSignalCoverage {
        signal: signal.into(),
        feature_names: feature_names.iter().map(|name| (*name).into()).collect(),
        observed_snapshots: snapshots.len() as u64,
        native_supported_snapshots,
        feature_available_snapshots,
        availability_mismatches,
        distinct_supported_measurements: measurements.len() as u64,
    })
}

fn action_surface_coverage(
    route: &NativeTacticRouteReport,
    snapshots: &BTreeMap<Digest, (FactSnapshot, Vec<f32>)>,
) -> Result<NativeTacticActionSurfaceCoverage, NativeTacticRouteRunError> {
    let mut coverage = NativeTacticActionSurfaceCoverage {
        decisions: 0,
        complete_decisions: 0,
        source_snapshots_resolved: 0,
        applicable_descriptors: 0,
        unique_descriptors: 0,
        untyped_descriptors: 0,
        selected_descriptors: 0,
        roll_available_decisions: 0,
        roll_selected_decisions: 0,
        a_button_available_decisions: 0,
        a_button_selected_decisions: 0,
        camera_modifier_available_decisions: 0,
        camera_modifier_selected_decisions: 0,
        option_type_availability: BTreeMap::new(),
    };
    let mut unique_descriptors = BTreeSet::new();
    for decision in route.seeds.iter().flat_map(|seed| &seed.trace) {
        coverage.decisions = coverage.decisions.saturating_add(1);
        let unique_ids = decision
            .applicable_tactics
            .iter()
            .map(|tactic| tactic.option_id.as_str())
            .collect::<BTreeSet<_>>();
        let selected_count = decision
            .applicable_tactics
            .iter()
            .filter(|tactic| tactic.selected)
            .count();
        let complete = !decision.applicable_tactics.is_empty()
            && unique_ids.len() == decision.applicable_tactics.len()
            && selected_count == 1
            && decision
                .applicable_tactics
                .iter()
                .any(|tactic| tactic.selected && tactic.option_id == decision.selected_option_id);
        coverage.complete_decisions = coverage
            .complete_decisions
            .saturating_add(u64::from(complete));
        let source = snapshots.get(&decision.frontier_identity);
        coverage.source_snapshots_resolved = coverage
            .source_snapshots_resolved
            .saturating_add(u64::from(source.is_some()));
        let context = source
            .map(|(facts, _)| GeneralizedTacticContext::from_facts(facts))
            .transpose()
            .map_err(route_error)?;
        let mut roll_available = false;
        let mut roll_selected = false;
        let mut a_available = false;
        let mut a_selected = false;
        let mut l_available = false;
        let mut l_selected = false;
        for tactic in &decision.applicable_tactics {
            coverage.applicable_descriptors = coverage.applicable_descriptors.saturating_add(1);
            coverage.selected_descriptors = coverage
                .selected_descriptors
                .saturating_add(u64::from(tactic.selected));
            let Some(descriptor) = &tactic.descriptor else {
                coverage.untyped_descriptors = coverage.untyped_descriptors.saturating_add(1);
                continue;
            };
            unique_descriptors.insert(descriptor.content_sha256().map_err(route_error)?);
            *coverage
                .option_type_availability
                .entry(option_type_name(&descriptor.option_type))
                .or_default() += 1;
            let Some(context) = &context else {
                continue;
            };
            let factors =
                generalized_tactic_action_factors(context, descriptor).map_err(route_error)?;
            let is_roll = factors.rolling;
            let has_a = factors.button_mask & BUTTON_A != 0;
            let has_l = factors.button_mask & BUTTON_L != 0;
            roll_available |= is_roll;
            roll_selected |= is_roll && tactic.selected;
            a_available |= has_a;
            a_selected |= has_a && tactic.selected;
            l_available |= has_l;
            l_selected |= has_l && tactic.selected;
        }
        coverage.roll_available_decisions = coverage
            .roll_available_decisions
            .saturating_add(u64::from(roll_available));
        coverage.roll_selected_decisions = coverage
            .roll_selected_decisions
            .saturating_add(u64::from(roll_selected));
        coverage.a_button_available_decisions = coverage
            .a_button_available_decisions
            .saturating_add(u64::from(a_available));
        coverage.a_button_selected_decisions = coverage
            .a_button_selected_decisions
            .saturating_add(u64::from(a_selected));
        coverage.camera_modifier_available_decisions = coverage
            .camera_modifier_available_decisions
            .saturating_add(u64::from(l_available));
        coverage.camera_modifier_selected_decisions = coverage
            .camera_modifier_selected_decisions
            .saturating_add(u64::from(l_selected));
    }
    coverage.unique_descriptors = unique_descriptors.len() as u64;
    Ok(coverage)
}

fn has_same_world_history(facts: &FactSnapshot) -> bool {
    !facts.recent_history.is_empty()
        && facts
            .recent_history
            .iter()
            .all(|row| row.stage == facts.world.stage && row.room == facts.world.room)
}

fn float_vectors_equal(left: &[f32], right: &[f32]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.to_bits() == right.to_bits())
}

fn vec3_bits_equal(left: [f32; 3], right: [f32; 3]) -> bool {
    left.map(f32::to_bits) == right.map(f32::to_bits)
}

fn benchmark_specific_feature_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    [
        "ordon",
        "spring",
        "load_zone",
        "wall_penalty",
        "roll_reward",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn option_type_name(option_type: &OptionType) -> String {
    match option_type {
        OptionType::Custom(name) => format!("custom/{name}"),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

fn strictly_increasing(values: &[Digest]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_report() -> NativeTacticObservationAudit {
        let signals = [
            "player_velocity",
            "past_trajectory",
            "camera_yaw",
            "prompted_action_state",
            "recent_option_kinematics",
            "contact_correlated_slowdown",
        ]
        .into_iter()
        .map(|signal| NativeTacticSignalCoverage {
            signal: signal.into(),
            feature_names: vec![format!("{signal}_available")],
            observed_snapshots: 1,
            native_supported_snapshots: 1,
            feature_available_snapshots: 1,
            availability_mismatches: 0,
            distinct_supported_measurements: 1,
        })
        .collect();
        let mut report = NativeTacticObservationAudit {
            schema: NATIVE_TACTIC_OBSERVATION_AUDIT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: Digest([1; 32]),
            route_report_sha256: Digest([2; 32]),
            execution_plan_sha256: Digest([3; 32]),
            feature_schema_sha256: Digest([4; 32]),
            action_schema_sha256: Digest([5; 32]),
            root_checkpoint_sha256: Digest([6; 32]),
            corpus_sha256: vec![Digest([7; 32])],
            unique_native_transitions: 1,
            unique_native_snapshots: 1,
            compared_feature_vectors: 2,
            exact_feature_vectors: 2,
            feature_vector_mismatches: 0,
            signals,
            action_surface: NativeTacticActionSurfaceCoverage {
                decisions: 1,
                complete_decisions: 1,
                source_snapshots_resolved: 1,
                applicable_descriptors: 3,
                unique_descriptors: 3,
                untyped_descriptors: 0,
                selected_descriptors: 1,
                roll_available_decisions: 1,
                roll_selected_decisions: 0,
                a_button_available_decisions: 1,
                a_button_selected_decisions: 0,
                camera_modifier_available_decisions: 1,
                camera_modifier_selected_decisions: 0,
                option_type_availability: BTreeMap::from([("move".into(), 3)]),
            },
            policy_signal_contract: NativeTacticPolicySignalContract {
                goal_coordinate_is_objective_derived: true,
                atomic_goal_target_only: true,
                authored_route_coordinates_used: false,
                authored_route_ids: Vec::new(),
                route_sequence_count: 0,
                benchmark_specific_feature_names: Vec::new(),
                passed: true,
            },
            passed: true,
        };
        report.content_sha256 = report.digest().unwrap();
        report
    }

    #[test]
    fn feature_schema_contains_every_required_generic_signal_without_benchmark_names() {
        let encoder = GoalConditionedTacticFeatureEncoder::new([1.0, 2.0, 3.0]).unwrap();
        for required in [
            "velocity_available",
            "trajectory_available",
            "camera_yaw_available",
            "player_action_available",
            "recent_option_trajectory_available",
            "recent_option_contact_slowdown_available",
        ] {
            assert!(encoder.feature_names.iter().any(|name| name == required));
        }
        assert!(
            encoder
                .feature_names
                .iter()
                .all(|name| !benchmark_specific_feature_name(name))
        );
    }

    #[test]
    fn exact_float_vector_comparison_rejects_equal_numeric_negative_zero() {
        assert!(float_vectors_equal(&[1.0, 0.0], &[1.0, 0.0]));
        assert!(!float_vectors_equal(&[1.0, -0.0], &[1.0, 0.0]));
    }

    #[test]
    fn audit_validation_rejects_resealed_semantic_drift() {
        let report = valid_report();
        report.validate().unwrap();

        let mut drifted = report;
        drifted.action_surface.camera_modifier_available_decisions = 0;
        drifted.content_sha256 = drifted.digest().unwrap();
        assert!(drifted.validate().is_err());
    }
}
