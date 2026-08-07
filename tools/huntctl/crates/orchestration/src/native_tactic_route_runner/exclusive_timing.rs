use super::*;

#[derive(Clone, Copy)]
pub(super) enum SeedOrchestrationPhase {
    LearnerRefresh,
    ActionCatalogConstruction,
    GraphSchedulingAndLeasing,
    TacticSelection,
    ResultValidationAndFactExtraction,
    CampaignAdmission,
    DecisionBookkeeping,
    TimingBoundary,
    SeedSetup,
    SeedFinalization,
}

#[derive(Clone, Copy)]
pub(super) struct ExclusiveTopTimingSnapshot {
    tactic_execution_micros: u64,
    model_update_micros: u64,
    evidence_projection_micros: u64,
    persistence_micros: u64,
}

impl ExclusiveTopTimingSnapshot {
    pub(super) fn capture(timing: &NativeTacticRouteTiming) -> Self {
        Self {
            tactic_execution_micros: timing.tactic_execution_micros,
            model_update_micros: timing.model_update_micros,
            evidence_projection_micros: timing.evidence_projection_micros,
            persistence_micros: timing.persistence_micros,
        }
    }

    pub(super) fn checked_delta_total(
        self,
        timing: &NativeTacticRouteTiming,
    ) -> Result<u64, NativeTacticRouteRunError> {
        [
            timing
                .tactic_execution_micros
                .checked_sub(self.tactic_execution_micros),
            timing
                .model_update_micros
                .checked_sub(self.model_update_micros),
            timing
                .evidence_projection_micros
                .checked_sub(self.evidence_projection_micros),
            timing
                .persistence_micros
                .checked_sub(self.persistence_micros),
        ]
        .into_iter()
        .try_fold(0_u64, |total, delta| total.checked_add(delta?))
        .ok_or_else(|| route_message("native tactic exclusive phase timing is detached"))
    }
}

pub(super) fn record_orchestration_detail(
    timing: &mut NativeTacticRouteTiming,
    phase: SeedOrchestrationPhase,
    micros: u64,
) -> Result<(), NativeTacticRouteRunError> {
    let Some(breakdown) = timing.orchestration_breakdown.as_mut() else {
        return Ok(());
    };
    let field = match phase {
        SeedOrchestrationPhase::LearnerRefresh => &mut breakdown.learner_refresh_micros,
        SeedOrchestrationPhase::ActionCatalogConstruction => {
            &mut breakdown.action_catalog_construction_micros
        }
        SeedOrchestrationPhase::GraphSchedulingAndLeasing => {
            &mut breakdown.graph_scheduling_and_leasing_micros
        }
        SeedOrchestrationPhase::TacticSelection => &mut breakdown.tactic_selection_micros,
        SeedOrchestrationPhase::ResultValidationAndFactExtraction => {
            &mut breakdown.result_validation_and_fact_extraction_micros
        }
        SeedOrchestrationPhase::CampaignAdmission => &mut breakdown.campaign_admission_micros,
        SeedOrchestrationPhase::DecisionBookkeeping => &mut breakdown.decision_bookkeeping_micros,
        SeedOrchestrationPhase::TimingBoundary => &mut breakdown.timing_boundary_micros,
        SeedOrchestrationPhase::SeedSetup => &mut breakdown.seed_setup_micros,
        SeedOrchestrationPhase::SeedFinalization => &mut breakdown.seed_finalization_micros,
    };
    *field = field
        .checked_add(micros)
        .ok_or_else(|| route_message("native tactic orchestration timing overflowed"))?;
    Ok(())
}

pub(super) fn record_orchestration_total(
    timing: &mut NativeTacticRouteTiming,
    micros: u64,
) -> Result<(), NativeTacticRouteRunError> {
    timing.orchestration_micros = timing
        .orchestration_micros
        .checked_add(micros)
        .ok_or_else(|| route_message("native tactic orchestration total overflowed"))?;
    Ok(())
}

pub(super) fn orchestration_detail_total(
    timing: &NativeTacticRouteTiming,
) -> Result<Option<u64>, NativeTacticRouteRunError> {
    timing
        .orchestration_breakdown
        .map(|breakdown| {
            breakdown
                .checked_total_micros()
                .ok_or_else(|| route_message("native tactic orchestration detail overflowed"))
        })
        .transpose()
}

/// A recovery point is committed before the decision journal and the timing
/// attribution for that decision's durable tail. Once the matching journal
/// record exists, the decision is authoritative, but those post-checkpoint
/// micros cannot be reconstructed by phase. Charge the exact recovered gap to
/// the explicit timing boundary before adding work from this invocation.
pub(super) fn reconcile_recovered_seed_timing(
    timing: &mut NativeTacticRouteTiming,
) -> Result<(), NativeTacticRouteRunError> {
    if timing.orchestration_breakdown.is_none() {
        return Ok(());
    }
    if let Some(breakdown) = timing.persistence_breakdown.as_mut() {
        let attributed = breakdown
            .checked_total_micros()
            .ok_or_else(|| route_message("recovered tactic persistence timing overflowed"))?;
        if let Some(boundary) = timing.persistence_micros.checked_sub(attributed) {
            breakdown.unattributed_micros = breakdown
                .unattributed_micros
                .checked_add(boundary)
                .ok_or_else(|| route_message("recovered tactic persistence timing overflowed"))?;
        } else {
            let recovered_tail = attributed
                .checked_sub(timing.persistence_micros)
                .ok_or_else(|| route_message("recovered tactic persistence timing underflowed"))?;
            timing.persistence_micros = attributed;
            timing.evidence_projection_and_persistence_micros = timing
                .evidence_projection_and_persistence_micros
                .checked_add(recovered_tail)
                .ok_or_else(|| route_message("recovered tactic evidence timing overflowed"))?;
        }
    }
    if let Some(breakdown) = timing.orchestration_breakdown.as_mut() {
        let attributed = breakdown
            .checked_total_micros()
            .ok_or_else(|| route_message("recovered tactic orchestration timing overflowed"))?;
        if let Some(boundary) = timing.orchestration_micros.checked_sub(attributed) {
            breakdown.timing_boundary_micros = breakdown
                .timing_boundary_micros
                .checked_add(boundary)
                .ok_or_else(|| route_message("recovered tactic orchestration timing overflowed"))?;
        } else {
            timing.orchestration_micros = attributed;
        }
    }
    if !timing.persistence_attribution_is_valid() || !timing.orchestration_attribution_is_valid() {
        return Err(route_message(
            "recovered native tactic phase breakdown is detached",
        ));
    }
    let attributed = [
        timing.tactic_execution_micros,
        timing.model_update_micros,
        timing.evidence_projection_micros,
        timing.persistence_micros,
        timing.orchestration_micros,
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add)
    .ok_or_else(|| route_message("recovered native tactic phase timing overflowed"))?;
    let boundary = timing.wall_micros.checked_sub(attributed).ok_or_else(|| {
        route_message("recovered native tactic phases exceed their committed wall")
    })?;
    record_orchestration_detail(timing, SeedOrchestrationPhase::TimingBoundary, boundary)?;
    record_orchestration_total(timing, boundary)?;
    if !timing.seed_wall_attribution_is_exact() {
        return Err(route_message(
            "recovered native tactic phases do not reconcile to their committed wall",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct CampaignExclusiveTimingInput {
    pub process_launch_micros: u64,
    pub demonstration_execution_micros: u64,
    pub macro_validation_execution_micros: u64,
    pub active_macro_validation_execution_micros: u64,
    pub learner_update_micros: u64,
    pub learner_reconstruction_micros: u64,
    pub campaign_setup_model_update_micros: u64,
    pub generation_model_update_micros: u64,
    pub seed_invocation_model_update_micros: u64,
    pub campaign_setup_wall_micros: u64,
    pub generation_coordination_wall_micros: u64,
    pub campaign_finalization_wall_micros: u64,
}

#[derive(Clone, Copy, Default)]
pub(super) struct CampaignPhaseWallTiming {
    pub campaign_setup_micros: u64,
    pub campaign_setup_model_update_micros: u64,
    pub generation_coordination_micros: u64,
    pub active_macro_validation_micros: u64,
    pub generation_model_update_micros: u64,
    pub campaign_finalization_started_micros: u64,
    pub seed_invocation_critical_lane_wall_micros: u64,
    pub seed_invocation_model_update_micros: u64,
}

pub(super) fn cumulative_route_wall_micros(
    observed_invocation_wall_micros: u64,
    durable_seed_critical_lane_wall_micros: u64,
    seed_invocation_critical_lane_wall_micros: u64,
) -> Result<u64, NativeTacticRouteRunError> {
    observed_invocation_wall_micros
        .checked_sub(seed_invocation_critical_lane_wall_micros)
        .and_then(|campaign_only| durable_seed_critical_lane_wall_micros.checked_add(campaign_only))
        .ok_or_else(|| route_message("native tactic cumulative route wall timing is detached"))
}

fn attribute_campaign_model_timing(
    timing: &mut NativeTacticRouteTiming,
    learner_update_micros: u64,
    learner_reconstruction_micros: u64,
    campaign_setup_model_update_micros: u64,
    generation_model_update_micros: u64,
    seed_invocation_model_update_micros: u64,
) -> Result<u64, NativeTacticRouteRunError> {
    let attributed_update_micros = campaign_setup_model_update_micros
        .checked_add(generation_model_update_micros)
        .and_then(|total| total.checked_add(seed_invocation_model_update_micros))
        .ok_or_else(|| route_message("native tactic learner update timing overflowed"))?;
    if attributed_update_micros != learner_update_micros {
        return Err(route_message(
            "native tactic learner update timing differs from its measured phases",
        ));
    }
    let campaign_model_update_micros = campaign_setup_model_update_micros
        .checked_add(generation_model_update_micros)
        .and_then(|total| total.checked_add(learner_reconstruction_micros))
        .ok_or_else(|| route_message("native tactic learner timing overflowed"))?;
    timing.model_update_micros = timing
        .model_update_micros
        .checked_add(campaign_model_update_micros)
        .ok_or_else(|| route_message("native tactic route model timing overflowed"))?;
    Ok(campaign_model_update_micros)
}

pub(super) fn attribute_campaign_timing(
    timing: &mut NativeTacticRouteTiming,
    execution_plan: &NativeTacticExecutionPlan,
    seeds: &[NativeTacticSeedResult],
    input: CampaignExclusiveTimingInput,
) -> Result<(), NativeTacticRouteRunError> {
    let campaign_model_update_micros = attribute_campaign_model_timing(
        timing,
        input.learner_update_micros,
        input.learner_reconstruction_micros,
        input.campaign_setup_model_update_micros,
        input.generation_model_update_micros,
        input.seed_invocation_model_update_micros,
    )?;

    let campaign_setup_known_micros = input
        .process_launch_micros
        .checked_add(input.demonstration_execution_micros)
        .and_then(|total| total.checked_add(input.campaign_setup_model_update_micros))
        .and_then(|total| total.checked_add(input.learner_reconstruction_micros))
        .ok_or_else(|| route_message("native tactic campaign setup timing overflowed"))?;
    let campaign_setup_orchestration_micros = input
        .campaign_setup_wall_micros
        .checked_sub(campaign_setup_known_micros)
        .ok_or_else(|| route_message("native tactic campaign setup phases exceed setup wall"))?;
    let generation_orchestration_micros = input
        .generation_coordination_wall_micros
        .checked_sub(input.generation_model_update_micros)
        .and_then(|total| total.checked_sub(input.active_macro_validation_execution_micros))
        .ok_or_else(|| route_message("native tactic generation work exceeds coordination wall"))?;
    let final_macro_validation_execution_micros = input
        .macro_validation_execution_micros
        .checked_sub(input.active_macro_validation_execution_micros)
        .ok_or_else(|| route_message("active macro validation exceeds total validation"))?;
    let campaign_finalization_orchestration_micros = input
        .campaign_finalization_wall_micros
        .checked_sub(final_macro_validation_execution_micros)
        .ok_or_else(|| {
            route_message("native tactic macro validation exceeds campaign finalization wall")
        })?;
    let campaign_orchestration_micros = [
        campaign_setup_orchestration_micros,
        generation_orchestration_micros,
        campaign_finalization_orchestration_micros,
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add)
    .ok_or_else(|| route_message("native tactic campaign orchestration timing overflowed"))?;
    timing.orchestration_micros = timing
        .orchestration_micros
        .checked_add(campaign_orchestration_micros)
        .ok_or_else(|| route_message("native tactic route orchestration timing overflowed"))?;
    if let Some(breakdown) = timing.orchestration_breakdown.as_mut() {
        breakdown.campaign_setup_micros = breakdown
            .campaign_setup_micros
            .checked_add(campaign_setup_orchestration_micros)
            .ok_or_else(|| route_message("native tactic campaign setup timing overflowed"))?;
        breakdown.generation_replay_and_coordination_micros = breakdown
            .generation_replay_and_coordination_micros
            .checked_add(generation_orchestration_micros)
            .ok_or_else(|| route_message("native tactic generation timing overflowed"))?;
        breakdown.campaign_finalization_micros = breakdown
            .campaign_finalization_micros
            .checked_add(campaign_finalization_orchestration_micros)
            .ok_or_else(|| route_message("native tactic finalization timing overflowed"))?;
    }

    let campaign_critical_lane_wall_micros =
        accumulated_coordinator_wall_micros(execution_plan, seeds);
    let critical_accounted_micros = [
        input.process_launch_micros,
        input.demonstration_execution_micros,
        input.macro_validation_execution_micros,
        campaign_model_update_micros,
        campaign_orchestration_micros,
        campaign_critical_lane_wall_micros,
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add)
    .ok_or_else(|| route_message("native tactic campaign critical timing overflowed"))?;
    let campaign_timing_boundary_micros = timing
        .wall_micros
        .checked_sub(critical_accounted_micros)
        .ok_or_else(|| route_message("native tactic campaign phases exceed route wall"))?;
    timing.orchestration_micros = timing
        .orchestration_micros
        .checked_add(campaign_timing_boundary_micros)
        .ok_or_else(|| route_message("native tactic route timing boundary overflowed"))?;
    if let Some(breakdown) = timing.orchestration_breakdown.as_mut() {
        breakdown.timing_boundary_micros = breakdown
            .timing_boundary_micros
            .checked_add(campaign_timing_boundary_micros)
            .ok_or_else(|| route_message("native tactic timing boundary overflowed"))?;
    }
    if !timing.orchestration_attribution_is_valid() {
        return Err(route_message(
            "native tactic route orchestration detail does not reconcile",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_breakdown_rejects_a_one_microsecond_detachment() {
        let mut timing = NativeTacticRouteTiming {
            wall_micros: 100,
            tactic_execution_micros: 40,
            model_update_micros: 10,
            evidence_projection_micros: 5,
            persistence_micros: 20,
            orchestration_micros: 25,
            orchestration_breakdown: Some(NativeTacticOrchestrationTiming {
                decision_bookkeeping_micros: 25,
                ..NativeTacticOrchestrationTiming::default()
            }),
            ..NativeTacticRouteTiming::default()
        };
        assert!(timing.seed_wall_attribution_is_exact());
        timing.orchestration_micros += 1;
        assert!(!timing.seed_wall_attribution_is_exact());
        timing.orchestration_micros -= 1;
        timing
            .orchestration_breakdown
            .as_mut()
            .unwrap()
            .graph_scheduling_breakdown
            .registration_micros = 1;
        assert!(!timing.seed_wall_attribution_is_exact());
    }

    #[test]
    fn recovered_decision_tail_is_charged_to_the_explicit_timing_boundary() {
        let mut timing = NativeTacticRouteTiming {
            wall_micros: 100,
            tactic_execution_micros: 40,
            model_update_micros: 10,
            evidence_projection_micros: 5,
            persistence_micros: 10,
            persistence_breakdown: Some(NativeTacticPersistenceTiming {
                unattributed_micros: 10,
                ..NativeTacticPersistenceTiming::default()
            }),
            orchestration_micros: 15,
            orchestration_breakdown: Some(NativeTacticOrchestrationTiming {
                decision_bookkeeping_micros: 15,
                ..NativeTacticOrchestrationTiming::default()
            }),
            ..NativeTacticRouteTiming::default()
        };

        reconcile_recovered_seed_timing(&mut timing).unwrap();

        assert_eq!(timing.orchestration_micros, 35);
        assert_eq!(
            timing
                .orchestration_breakdown
                .unwrap()
                .timing_boundary_micros,
            20
        );
        assert!(timing.seed_wall_attribution_is_exact());
        reconcile_recovered_seed_timing(&mut timing).unwrap();
        assert_eq!(timing.orchestration_micros, 35);
    }

    #[test]
    fn recovered_subphase_gaps_are_explicit_before_wall_reconciliation() {
        let mut timing = NativeTacticRouteTiming {
            wall_micros: 100,
            tactic_execution_micros: 40,
            model_update_micros: 10,
            evidence_projection_micros: 5,
            persistence_micros: 12,
            persistence_breakdown: Some(NativeTacticPersistenceTiming {
                unattributed_micros: 10,
                ..NativeTacticPersistenceTiming::default()
            }),
            orchestration_micros: 18,
            orchestration_breakdown: Some(NativeTacticOrchestrationTiming {
                decision_bookkeeping_micros: 15,
                ..NativeTacticOrchestrationTiming::default()
            }),
            ..NativeTacticRouteTiming::default()
        };

        reconcile_recovered_seed_timing(&mut timing).unwrap();

        assert_eq!(
            timing.persistence_breakdown.unwrap().unattributed_micros,
            12
        );
        assert_eq!(timing.orchestration_micros, 33);
        assert_eq!(
            timing
                .orchestration_breakdown
                .unwrap()
                .timing_boundary_micros,
            18
        );
        assert!(timing.seed_wall_attribution_is_exact());
    }

    #[test]
    fn recovered_subphase_tails_promote_parent_totals_before_wall_reconciliation() {
        let mut timing = NativeTacticRouteTiming {
            wall_micros: 100,
            tactic_execution_micros: 40,
            model_update_micros: 10,
            evidence_projection_and_persistence_micros: 15,
            evidence_projection_micros: 5,
            persistence_micros: 10,
            persistence_breakdown: Some(NativeTacticPersistenceTiming {
                unattributed_micros: 12,
                ..NativeTacticPersistenceTiming::default()
            }),
            orchestration_micros: 10,
            orchestration_breakdown: Some(NativeTacticOrchestrationTiming {
                decision_bookkeeping_micros: 15,
                ..NativeTacticOrchestrationTiming::default()
            }),
            ..NativeTacticRouteTiming::default()
        };

        reconcile_recovered_seed_timing(&mut timing).unwrap();

        assert_eq!(timing.persistence_micros, 12);
        assert_eq!(timing.evidence_projection_and_persistence_micros, 17);
        assert_eq!(timing.orchestration_micros, 33);
        assert_eq!(
            timing
                .orchestration_breakdown
                .unwrap()
                .timing_boundary_micros,
            18
        );
        assert!(timing.seed_wall_attribution_is_exact());
    }

    #[test]
    fn resumed_campaign_keeps_durable_seed_model_time_and_adds_reconstruction() {
        let mut timing = NativeTacticRouteTiming {
            model_update_micros: 279,
            ..NativeTacticRouteTiming::default()
        };

        let campaign_model = attribute_campaign_model_timing(&mut timing, 0, 3, 0, 0, 0).unwrap();

        assert_eq!(campaign_model, 3);
        assert_eq!(timing.model_update_micros, 282);
    }

    #[test]
    fn current_seed_model_time_is_not_counted_twice() {
        let mut timing = NativeTacticRouteTiming {
            model_update_micros: 40,
            ..NativeTacticRouteTiming::default()
        };

        let campaign_model = attribute_campaign_model_timing(&mut timing, 55, 3, 7, 8, 40).unwrap();

        assert_eq!(campaign_model, 18);
        assert_eq!(timing.model_update_micros, 58);
    }

    #[test]
    fn learner_update_time_must_reconcile_to_measured_phases() {
        let mut timing = NativeTacticRouteTiming::default();

        assert!(attribute_campaign_model_timing(&mut timing, 55, 3, 7, 7, 40).is_err());
        assert_eq!(timing.model_update_micros, 0);
    }

    #[test]
    fn cumulative_route_wall_replaces_only_current_seed_lane_with_durable_lane() {
        assert_eq!(cumulative_route_wall_micros(100, 80, 80).unwrap(), 100);
        assert_eq!(cumulative_route_wall_micros(25, 180, 5).unwrap(), 200);
        assert!(cumulative_route_wall_micros(4, 180, 5).is_err());
    }
}
