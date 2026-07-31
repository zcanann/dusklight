use super::scratch_discovery::route_report_sha256;
use super::*;

pub const NATIVE_TACTIC_ROUTE_DIAGNOSIS_SCHEMA_V1: &str =
    "dusklight-native-tactic-route-diagnosis/v1";

const BUTTON_L: u16 = 0x0040;
const BUTTON_A: u16 = 0x0100;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRouteProfile {
    pub native_ticks: u64,
    pub option_count: u64,
    pub mean_option_ticks: f64,
    pub maximum_option_ticks: u32,
    pub repeated_consecutive_options: u64,
    pub option_type_counts: BTreeMap<String, u64>,
    pub trajectory_option_count: u64,
    pub trajectory_observed_ticks: u64,
    pub planar_path_length: f64,
    pub route_planar_displacement: Option<f64>,
    pub route_excess_path_length: Option<f64>,
    pub option_local_planar_displacement: f64,
    pub option_local_excess_path_length: f64,
    pub mean_planar_speed: Option<f64>,
    pub maximum_planar_velocity: Option<f64>,
    pub terminal_planar_velocity: Option<f64>,
    pub commanded_motion_ticks: u64,
    pub commanded_stall_ticks: u64,
    pub wall_contact_ticks: u64,
    pub commanded_momentum_loss: f64,
    pub contact_slowdown_option_count: u64,
    pub contact_commanded_motion_ticks: u64,
    pub contact_commanded_momentum_loss: f64,
    pub collision_correction_ticks: u64,
    pub collision_correction_total: f64,
    pub controller_neutral_ticks: u64,
    pub camera_modifier_ticks: u64,
    pub a_button_ticks: u64,
    pub typed_roll_options: u64,
    pub typed_roll_ticks: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRouteActionSurfaceProfile {
    pub route_option_count: u64,
    pub exact_expansion_matches: u64,
    pub selected_action_available: u64,
    pub selected_action_supported: u64,
    pub boundaries_with_roll_available: u64,
    pub boundaries_with_camera_modifier_available: u64,
    pub typed_descriptor_timeline_complete: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchRouteDiagnosis {
    pub seed: u64,
    pub authenticated_first_hit_tick: u64,
    pub terminal_result_sha256: Digest,
    pub profile: NativeTacticRouteProfile,
    pub action_surface: NativeTacticRouteActionSurfaceProfile,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRouteDiagnosisReport {
    pub schema: String,
    pub content_sha256: Digest,
    pub scratch_route_report_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub demonstration_corpus_sha256: Digest,
    pub demonstration_first_hit_tick: u64,
    pub demonstration_profile: NativeTacticRouteProfile,
    /// Demonstration capture predates policy ranking and deliberately has no
    /// reconstructed action-availability claims.
    pub demonstration_action_surface_available: bool,
    pub scratch_routes: Vec<NativeTacticScratchRouteDiagnosis>,
}

impl NativeTacticRouteDiagnosisReport {
    pub fn build(
        scratch: &NativeTacticRouteReport,
        demonstration: &NativeTacticDemonstrationReport,
        demonstration_corpus: &TacticQTrainingCorpus,
        terminal_results: Vec<(u64, TacticQFinalResult)>,
    ) -> Result<Self, NativeTacticRouteRunError> {
        validate_training_corpus(demonstration_corpus).map_err(route_error)?;
        if scratch.demonstration_transitions != 0
            || scratch.demonstration.is_some()
            || demonstration.execution_binding_sha256 != scratch.execution_binding_sha256
            || demonstration.objective_sha256 != scratch.objective_sha256
            || demonstration.feature_schema_sha256 != scratch.feature_schema_sha256
            || demonstration.root_checkpoint_sha256 != demonstration_corpus.root_checkpoint_sha256
            || demonstration.objective_sha256 != demonstration_corpus.objective_sha256
            || demonstration.feature_schema_sha256 != demonstration_corpus.feature_schema_sha256
            || demonstration.transition_count != demonstration_corpus.transitions.len() as u64
            || demonstration.native_ticks != demonstration.first_hit_tick.saturating_add(1)
        {
            return Err(route_message(
                "route diagnosis demonstration differs from scratch authority",
            ));
        }
        let demonstration_profile = route_profile(&demonstration_corpus.transitions)?;
        if demonstration_profile.native_ticks != demonstration.native_ticks {
            return Err(route_message(
                "route diagnosis demonstration ticks differ from its corpus",
            ));
        }

        let terminal_seed_count = scratch
            .seeds
            .iter()
            .filter(|seed| seed.terminal_discovered)
            .count();
        if terminal_seed_count != terminal_results.len() {
            return Err(route_message(
                "route diagnosis requires one result per terminal seed",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut scratch_routes = Vec::with_capacity(terminal_results.len());
        for (seed_value, result) in terminal_results {
            let seed = scratch
                .seeds
                .iter()
                .find(|seed| seed.seed == seed_value && seed.terminal_discovered)
                .ok_or_else(|| route_message("route diagnosis result has no terminal seed"))?;
            if !seen.insert(seed_value)
                || result.execution_authority_sha256 != scratch.execution_plan_sha256
                || result.objective_sha256 != scratch.objective_sha256
                || result.root_checkpoint_sha256 != demonstration.root_checkpoint_sha256
                || seed.best_authenticated_tick.is_none()
                || seed.best_terminal_state_sha256 != Some(result.terminal_state_sha256)
            {
                return Err(route_message(
                    "route diagnosis terminal result is detached from its seed",
                ));
            }
            let profile = route_profile(&result.replay)?;
            let authenticated_first_hit_tick = seed
                .best_authenticated_tick
                .ok_or_else(|| route_message("terminal seed lacks an authenticated tick"))?;
            if profile.native_ticks != authenticated_first_hit_tick.saturating_add(1) {
                return Err(route_message(
                    "route diagnosis terminal ticks differ from authenticated first hit",
                ));
            }
            scratch_routes.push(NativeTacticScratchRouteDiagnosis {
                seed: seed_value,
                authenticated_first_hit_tick,
                terminal_result_sha256: result.content_sha256,
                action_surface: route_action_surface(&result.replay, &seed.trace)?,
                profile,
            });
        }
        scratch_routes.sort_by_key(|route| route.seed);
        let mut report = Self {
            schema: NATIVE_TACTIC_ROUTE_DIAGNOSIS_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            scratch_route_report_sha256: route_report_sha256(scratch)?,
            optimization_request_sha256: scratch.optimization_request_sha256,
            execution_binding_sha256: scratch.execution_binding_sha256,
            objective_sha256: scratch.objective_sha256,
            root_checkpoint_sha256: demonstration.root_checkpoint_sha256,
            feature_schema_sha256: scratch.feature_schema_sha256,
            action_schema_sha256: scratch.action_schema_sha256,
            demonstration_corpus_sha256: demonstration.corpus_sha256,
            demonstration_first_hit_tick: demonstration.first_hit_tick,
            demonstration_profile,
            demonstration_action_surface_available: false,
            scratch_routes,
        };
        report.content_sha256 = report.compute_content_sha256()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_ROUTE_DIAGNOSIS_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.scratch_route_report_sha256 == Digest::ZERO
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.root_checkpoint_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.action_schema_sha256 == Digest::ZERO
            || self.demonstration_corpus_sha256 == Digest::ZERO
            || self.demonstration_action_surface_available
            || self.demonstration_profile.native_ticks
                != self.demonstration_first_hit_tick.saturating_add(1)
            || self.scratch_routes.is_empty()
            || !self
                .scratch_routes
                .windows(2)
                .all(|pair| pair[0].seed < pair[1].seed)
            || self.scratch_routes.iter().any(|route| {
                route.terminal_result_sha256 == Digest::ZERO
                    || route.profile.native_ticks
                        != route.authenticated_first_hit_tick.saturating_add(1)
                    || !profile_is_valid(&route.profile)
                    || route.action_surface.route_option_count != route.profile.option_count
                    || !action_surface_is_valid(&route.action_surface)
            })
            || !profile_is_valid(&self.demonstration_profile)
            || self.compute_content_sha256()? != self.content_sha256
        {
            return Err(route_message("native tactic route diagnosis is invalid"));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeTacticRouteRunError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(route_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_json::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

fn route_profile(
    transitions: &[OptionTransitionSample],
) -> Result<NativeTacticRouteProfile, NativeTacticRouteRunError> {
    if transitions.is_empty() {
        return Err(route_message("route profile has no transitions"));
    }
    let mut profile = NativeTacticRouteProfile {
        native_ticks: 0,
        option_count: transitions.len() as u64,
        mean_option_ticks: 0.0,
        maximum_option_ticks: 0,
        repeated_consecutive_options: 0,
        option_type_counts: BTreeMap::new(),
        trajectory_option_count: 0,
        trajectory_observed_ticks: 0,
        planar_path_length: 0.0,
        route_planar_displacement: None,
        route_excess_path_length: None,
        option_local_planar_displacement: 0.0,
        option_local_excess_path_length: 0.0,
        mean_planar_speed: None,
        maximum_planar_velocity: None,
        terminal_planar_velocity: None,
        commanded_motion_ticks: 0,
        commanded_stall_ticks: 0,
        wall_contact_ticks: 0,
        commanded_momentum_loss: 0.0,
        contact_slowdown_option_count: 0,
        contact_commanded_motion_ticks: 0,
        contact_commanded_momentum_loss: 0.0,
        collision_correction_ticks: 0,
        collision_correction_total: 0.0,
        controller_neutral_ticks: 0,
        camera_modifier_ticks: 0,
        a_button_ticks: 0,
        typed_roll_options: 0,
        typed_roll_ticks: 0,
    };
    let mut previous = None::<&OptionActionDescriptor>;
    for transition in transitions {
        transition.validate().map_err(route_error)?;
        let ticks = transition.execution.duration.realized_ticks;
        profile.native_ticks = profile.native_ticks.saturating_add(u64::from(ticks));
        profile.maximum_option_ticks = profile.maximum_option_ticks.max(ticks);
        *profile
            .option_type_counts
            .entry(option_type_key(
                &transition.value_sample.action.option_type,
            )?)
            .or_default() += 1;
        if previous == Some(&transition.value_sample.action) {
            profile.repeated_consecutive_options =
                profile.repeated_consecutive_options.saturating_add(1);
        }
        previous = Some(&transition.value_sample.action);
        if transition.value_sample.action.option_type == OptionType::Roll {
            profile.typed_roll_options = profile.typed_roll_options.saturating_add(1);
            profile.typed_roll_ticks = profile.typed_roll_ticks.saturating_add(u64::from(ticks));
        }
        for frame in &transition.execution.emitted_raw_actions {
            let pad = frame.pads[0];
            profile.controller_neutral_ticks = profile
                .controller_neutral_ticks
                .saturating_add(u64::from(pad == RawPadState::default()));
            profile.camera_modifier_ticks = profile
                .camera_modifier_ticks
                .saturating_add(u64::from(pad.buttons & BUTTON_L != 0));
            profile.a_button_ticks = profile
                .a_button_ticks
                .saturating_add(u64::from(pad.buttons & BUTTON_A != 0));
        }
        let trajectory = transition
            .after
            .recent_option
            .as_ref()
            .filter(|recent| recent.option_id == transition.value_sample.action.option_id)
            .and_then(|recent| recent.trajectory);
        if let Some(trajectory) = trajectory {
            profile.trajectory_option_count = profile.trajectory_option_count.saturating_add(1);
            profile.trajectory_observed_ticks = profile
                .trajectory_observed_ticks
                .saturating_add(u64::from(trajectory.observed_ticks));
            let path = f64::from(f32::from_bits(trajectory.planar_path_length_f32_bits));
            let displacement = f64::from(f32::from_bits(trajectory.planar_displacement_f32_bits));
            profile.planar_path_length += path;
            profile.option_local_planar_displacement += displacement;
            profile.option_local_excess_path_length += (path - displacement).max(0.0);
            profile.maximum_planar_velocity = Some(
                profile
                    .maximum_planar_velocity
                    .unwrap_or(0.0)
                    .max(f64::from(f32::from_bits(
                        trajectory.maximum_planar_velocity_f32_bits,
                    ))),
            );
            profile.terminal_planar_velocity = Some(f64::from(f32::from_bits(
                trajectory.final_planar_velocity_f32_bits,
            )));
            profile.commanded_motion_ticks = profile
                .commanded_motion_ticks
                .saturating_add(u64::from(trajectory.commanded_motion_ticks));
            profile.commanded_stall_ticks = profile
                .commanded_stall_ticks
                .saturating_add(u64::from(trajectory.commanded_stall_ticks));
            profile.wall_contact_ticks = profile
                .wall_contact_ticks
                .saturating_add(u64::from(trajectory.wall_contact_ticks));
            profile.commanded_momentum_loss +=
                f64::from(f32::from_bits(trajectory.commanded_momentum_loss_f32_bits));
            if let (Some(contact_ticks), Some(contact_loss)) = (
                trajectory.wall_contact_commanded_motion_ticks,
                trajectory.wall_contact_commanded_momentum_loss_f32_bits,
            ) {
                profile.contact_slowdown_option_count =
                    profile.contact_slowdown_option_count.saturating_add(1);
                profile.contact_commanded_motion_ticks = profile
                    .contact_commanded_motion_ticks
                    .saturating_add(u64::from(contact_ticks));
                profile.contact_commanded_momentum_loss += f64::from(f32::from_bits(contact_loss));
            }
            profile.collision_correction_ticks = profile
                .collision_correction_ticks
                .saturating_add(u64::from(trajectory.collision_correction_ticks));
            profile.collision_correction_total += f64::from(f32::from_bits(
                trajectory.collision_correction_total_f32_bits,
            ));
        }
    }
    profile.mean_option_ticks = profile.native_ticks as f64 / profile.option_count as f64;
    profile.mean_planar_speed = (profile.trajectory_observed_ticks != 0)
        .then_some(profile.planar_path_length / profile.trajectory_observed_ticks as f64);
    let first = &transitions[0].before;
    let last = &transitions[transitions.len() - 1].after;
    if first.world.stage == last.world.stage && first.world.room == last.world.room {
        let start = first.player.position_f32_bits.map(f32::from_bits);
        let end = last.player.position_f32_bits.map(f32::from_bits);
        let displacement = f64::from(
            (end[0] - start[0])
                .mul_add(end[0] - start[0], (end[2] - start[2]).powi(2))
                .sqrt(),
        );
        profile.route_planar_displacement = Some(displacement);
        profile.route_excess_path_length =
            Some((profile.planar_path_length - displacement).max(0.0));
    }
    if profile_is_valid(&profile) {
        Ok(profile)
    } else {
        Err(route_message("native tactic route profile is invalid"))
    }
}

fn route_action_surface(
    transitions: &[OptionTransitionSample],
    trace: &[NativeTacticDecisionTrace],
) -> Result<NativeTacticRouteActionSurfaceProfile, NativeTacticRouteRunError> {
    let mut profile = NativeTacticRouteActionSurfaceProfile {
        route_option_count: transitions.len() as u64,
        typed_descriptor_timeline_complete: true,
        ..Default::default()
    };
    for transition in transitions {
        let matching = trace.iter().find(|decision| {
            decision.before.snapshot_sha256 == transition.before_state_sha256
                && decision.proposal_batch.iter().any(|proposal| {
                    proposal.option_id == transition.value_sample.action.option_id
                        && proposal.after_snapshot_sha256 == transition.after_state_sha256
                })
        });
        let Some(decision) = matching else {
            profile.typed_descriptor_timeline_complete = false;
            continue;
        };
        profile.exact_expansion_matches = profile.exact_expansion_matches.saturating_add(1);
        profile.typed_descriptor_timeline_complete &= decision
            .applicable_tactics
            .iter()
            .all(|value| value.descriptor.is_some());
        let selected = decision.applicable_tactics.iter().find(|value| {
            value.applicable
                && value
                    .descriptor
                    .as_ref()
                    .is_some_and(|descriptor| descriptor == &transition.value_sample.action)
        });
        if let Some(selected) = selected {
            profile.selected_action_available = profile.selected_action_available.saturating_add(1);
            profile.selected_action_supported = profile
                .selected_action_supported
                .saturating_add(u64::from(selected.mean_q.is_some()));
        }
        profile.boundaries_with_roll_available = profile
            .boundaries_with_roll_available
            .saturating_add(u64::from(decision.applicable_tactics.iter().any(|value| {
                value.applicable
                    && value
                        .descriptor
                        .as_ref()
                        .is_some_and(|descriptor| descriptor.option_type == OptionType::Roll)
            })));
        profile.boundaries_with_camera_modifier_available = profile
            .boundaries_with_camera_modifier_available
            .saturating_add(u64::from(decision.applicable_tactics.iter().any(|value| {
                value.applicable
                    && value
                        .descriptor
                        .as_ref()
                        .is_some_and(descriptor_uses_camera_modifier)
            })));
    }
    profile.typed_descriptor_timeline_complete &= profile.exact_expansion_matches
        == profile.route_option_count
        && profile.selected_action_available == profile.route_option_count;
    Ok(profile)
}

fn descriptor_uses_camera_modifier(descriptor: &OptionActionDescriptor) -> bool {
    descriptor
        .parameters
        .get("command_button_mask")
        .is_some_and(
            |parameter| matches!(parameter, OptionParameter::Unsigned(mask) if mask & u64::from(BUTTON_L) != 0),
        )
}

fn option_type_key(option_type: &OptionType) -> Result<String, NativeTacticRouteRunError> {
    let value = serde_json::to_value(option_type).map_err(route_error)?;
    Ok(value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string()))
}

fn profile_is_valid(profile: &NativeTacticRouteProfile) -> bool {
    let finite_nonnegative = [
        profile.mean_option_ticks,
        profile.planar_path_length,
        profile.option_local_planar_displacement,
        profile.option_local_excess_path_length,
        profile.commanded_momentum_loss,
        profile.contact_commanded_momentum_loss,
        profile.collision_correction_total,
    ]
    .into_iter()
    .all(|value| value.is_finite() && value >= 0.0);
    profile.native_ticks > 0
        && profile.option_count > 0
        && profile.maximum_option_ticks > 0
        && profile.repeated_consecutive_options < profile.option_count
        && profile.trajectory_option_count <= profile.option_count
        && profile.controller_neutral_ticks <= profile.native_ticks
        && profile.camera_modifier_ticks <= profile.native_ticks
        && profile.a_button_ticks <= profile.native_ticks
        && profile.typed_roll_options <= profile.option_count
        && profile.typed_roll_ticks <= profile.native_ticks
        && profile.contact_slowdown_option_count <= profile.trajectory_option_count
        && profile.contact_commanded_motion_ticks <= profile.commanded_motion_ticks
        && profile.contact_commanded_momentum_loss <= profile.commanded_momentum_loss + 1.0e-6
        && profile
            .mean_planar_speed
            .is_none_or(|value| value.is_finite() && value >= 0.0)
        && profile
            .route_planar_displacement
            .is_none_or(|value| value.is_finite() && value >= 0.0)
        && profile
            .route_excess_path_length
            .is_none_or(|value| value.is_finite() && value >= 0.0)
        && (profile.route_planar_displacement.is_some()
            == profile.route_excess_path_length.is_some())
        && profile
            .maximum_planar_velocity
            .is_none_or(|value| value.is_finite() && value >= 0.0)
        && profile
            .terminal_planar_velocity
            .is_none_or(|value| value.is_finite() && value >= 0.0)
        && finite_nonnegative
}

fn action_surface_is_valid(profile: &NativeTacticRouteActionSurfaceProfile) -> bool {
    profile.route_option_count > 0
        && profile.exact_expansion_matches <= profile.route_option_count
        && profile.selected_action_available <= profile.exact_expansion_matches
        && profile.selected_action_supported <= profile.selected_action_available
        && profile.boundaries_with_roll_available <= profile.exact_expansion_matches
        && profile.boundaries_with_camera_modifier_available <= profile.exact_expansion_matches
        && (!profile.typed_descriptor_timeline_complete
            || (profile.exact_expansion_matches == profile.route_option_count
                && profile.selected_action_available == profile.route_option_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_action_keys_and_camera_modifier_are_not_id_parsing() {
        let descriptor = OptionActionDescriptor {
            option_id: "opaque".into(),
            option_type: OptionType::Move,
            parameters: BTreeMap::from([(
                "command_button_mask".into(),
                OptionParameter::Unsigned(u64::from(BUTTON_L | BUTTON_A)),
            )]),
        };
        assert!(descriptor_uses_camera_modifier(&descriptor));
        assert_eq!(option_type_key(&OptionType::Roll).unwrap(), "roll");
    }
}
