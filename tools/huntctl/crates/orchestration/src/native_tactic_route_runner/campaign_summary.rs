use super::scratch_discovery::route_report_sha256;
use super::*;

pub const NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V1: &str =
    "dusklight-native-tactic-campaign-summary/v1";
pub const NATIVE_TACTIC_CAMPAIGN_SUMMARY_FILE: &str = "campaign-summary.json";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticCausalLink {
    ObservedState,
    LegalActionSurface,
    NativeExploration,
    ExperiencePublication,
    LearnerUpdate,
    PolicyDeployment,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignIdentities {
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub objective_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub replay_snapshot_sha256: Digest,
    pub learner_snapshot_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignTreatmentSummary {
    pub proposal_policy: TacticProposalPolicy,
    pub value_treatment: TacticValueTreatment,
    pub execution_strategy: NativeGenericExecutionStrategy,
    pub seeds: Vec<u64>,
    pub workers: usize,
    pub proposal_width_per_decision: usize,
    pub decisions_per_seed: u64,
    pub branch_every_decisions: u64,
    pub refit_every_decisions: u64,
    pub root_refresh_cadence: u32,
    pub epsilon_per_million: Vec<u32>,
    pub demonstration_chunk_ticks: Option<u32>,
    pub demonstration_transitions: u64,
    pub resource_budgets: NativeTacticPlanBudgets,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignOutcomeSummary {
    pub terminal_seeds: u64,
    pub seed_count: u64,
    pub best_authenticated_tick: Option<u64>,
    pub median_time_to_first_terminal_micros: Option<u64>,
    pub worst_time_to_first_terminal_micros: Option<u64>,
    pub total_decisions: u64,
    pub total_proposals: u64,
    pub total_native_ticks: u64,
    pub useful_decisions: u64,
    pub unique_useful_graph_expansions: u64,
    pub stop_reasons: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignCausalSummary {
    pub learning_expected: bool,
    pub decisions_with_observed_state: u64,
    pub decisions_with_complete_action_surface: u64,
    pub decisions_with_native_proposals: u64,
    pub realized_native_proposals: u64,
    pub newly_published_training_rows: u64,
    pub final_training_replay_rows: u64,
    pub learner_updates: u64,
    pub model_snapshots_published: u64,
    pub model_snapshots_consumed: u64,
    pub distinct_model_snapshots_consumed: u64,
    pub post_update_policy_decisions: u64,
    pub selected_action_changes_at_model_change: u64,
    pub causal_chain_ready_for_matched_evaluation: bool,
    pub first_incomplete_link: Option<NativeTacticCausalLink>,
    /// A single treatment can prove that an updated policy was deployed, but
    /// only the matched learned/control comparison can attribute an outcome
    /// change to learning.
    pub outcome_effect_requires_matched_control: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignEfficiencySummary {
    pub useful_expansions_per_second_millionths: u64,
    pub native_ticks_per_second_millionths: u64,
    pub learner_updates_per_second_millionths: u64,
    pub native_worker_utilization_per_million: u64,
    pub maximum_model_replay_lag_revisions: u64,
    pub maximum_observed_stale_revisions: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignTimingSummary {
    pub wall_micros: u64,
    pub process_launch_micros: u64,
    pub tactic_selection_micros: u64,
    pub checkpoint_branching_micros: u64,
    pub native_wait_micros: u64,
    pub native_simulation_occupancy_micros: u64,
    pub ipc_and_result_transport_micros: u64,
    pub native_observation_capture_micros: u64,
    pub native_corpus_encoding_micros: u64,
    pub rust_state_extraction_micros: u64,
    pub model_update_micros: u64,
    pub evidence_projection_micros: u64,
    pub persistence_micros: u64,
    pub orchestration_micros: u64,
    pub reporting_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignWorkSummary {
    pub lease_accounting_complete: bool,
    pub proposal_dispatches: u64,
    pub completed_leases: u64,
    pub retryable_leases: u64,
    pub cancelled_leases: u64,
    pub failed_leases: u64,
    pub unresolved_leases: u64,
    pub discarded_proposals: u64,
    pub replay_duplicate_admissions: u64,
    pub duplicate_training_transitions: u64,
    pub censored_training_transitions: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignResourceSummary {
    pub memory_budget_bytes: Option<u64>,
    pub checkpoint_cache_capacity_per_worker_bytes: u64,
    pub peak_worker_resident_bytes: u64,
    pub peak_checkpoint_bytes: u64,
    pub peak_host_snapshot_bytes: u64,
    pub peak_live_endpoint_entries: u64,
    pub peak_live_endpoint_host_snapshot_bytes: u64,
    pub memory_bound_satisfied: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignSummary {
    pub schema: String,
    pub content_sha256: Digest,
    pub route_report_sha256: Digest,
    pub identities: NativeTacticCampaignIdentities,
    pub treatment: NativeTacticCampaignTreatmentSummary,
    pub outcome: NativeTacticCampaignOutcomeSummary,
    pub causal_chain: NativeTacticCampaignCausalSummary,
    pub efficiency: NativeTacticCampaignEfficiencySummary,
    pub timing: NativeTacticCampaignTimingSummary,
    pub work: NativeTacticCampaignWorkSummary,
    pub resources: NativeTacticCampaignResourceSummary,
}

impl NativeTacticCampaignSummary {
    pub fn build(
        route: &NativeTacticRouteReport,
        plan: &NativeTacticExecutionPlan,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if plan.identity()? != route.execution_plan_sha256
            || plan.seeds != route.exploration_seeds
            || plan.proposal_policy != route.proposal_policy
            || plan.value_treatment != route.value_treatment
            || plan.execution_strategy != route.execution_strategy
            || plan.budgets != route.resource_budgets
        {
            return Err(route_message(
                "campaign summary execution plan is detached from its route report",
            ));
        }

        let mut epsilon_per_million = plan
            .lanes
            .iter()
            .map(|lane| lane.epsilon_per_million)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        epsilon_per_million.sort_unstable();

        let total_proposals = route
            .seeds
            .iter()
            .flat_map(|seed| &seed.trace)
            .map(|decision| decision.proposal_batch.len() as u64)
            .sum();
        let mut stop_reasons = BTreeMap::new();
        for reason in route
            .seeds
            .iter()
            .flat_map(|seed| seed.stop_reasons.iter().copied())
        {
            let name = match reason {
                NativeTacticSeedStopReason::DecisionBudgetReached => "decision_budget_reached",
                NativeTacticSeedStopReason::SimulatedTickBudgetReached => {
                    "simulated_tick_budget_reached"
                }
                NativeTacticSeedStopReason::NativeTickBudgetReached => "native_tick_budget_reached",
                NativeTacticSeedStopReason::WallBudgetReached => "wall_budget_reached",
            };
            *stop_reasons.entry(name.into()).or_default() += 1;
        }

        let causal_chain = causal_summary(route);
        let work = work_summary(route, total_proposals);
        let worker_capacity_micros = route
            .timing
            .tactic_execution_micros
            .saturating_mul(route.workers as u64);
        let native_worker_utilization_per_million = ratio_per_million(
            route.timing.native_simulation_micros,
            worker_capacity_micros,
        )
        .min(1_000_000);
        let memory_budget_bytes = match route.resource_budgets.memory_bytes {
            NativeTacticResourceLimit::Bounded(value) => Some(value),
            NativeTacticResourceLimit::Unbounded => None,
        };
        let peak_worker_resident_bytes = route.native_restore_accounting.peak_resident_bytes;

        let mut summary = Self {
            schema: NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            route_report_sha256: route_report_sha256(route)?,
            identities: NativeTacticCampaignIdentities {
                optimization_request_sha256: route.optimization_request_sha256,
                execution_binding_sha256: route.execution_binding_sha256,
                execution_plan_sha256: route.execution_plan_sha256,
                objective_sha256: route.objective_sha256,
                feature_schema_sha256: route.feature_schema_sha256,
                action_schema_sha256: route.action_schema_sha256,
                replay_snapshot_sha256: route.replay_snapshot_sha256,
                learner_snapshot_sha256: route.learner_authority.latest_model_snapshot_sha256,
            },
            treatment: NativeTacticCampaignTreatmentSummary {
                proposal_policy: route.proposal_policy,
                value_treatment: route.value_treatment,
                execution_strategy: route.execution_strategy,
                seeds: route.exploration_seeds.clone(),
                workers: route.workers,
                proposal_width_per_decision: plan.proposal_width_per_decision,
                decisions_per_seed: route.decisions_per_seed,
                branch_every_decisions: plan.branch_every_decisions,
                refit_every_decisions: route.refit_every_decisions,
                root_refresh_cadence: plan.root_refresh_cadence,
                epsilon_per_million,
                demonstration_chunk_ticks: plan.demonstration_chunk_ticks,
                demonstration_transitions: route.demonstration_transitions,
                resource_budgets: route.resource_budgets,
            },
            outcome: NativeTacticCampaignOutcomeSummary {
                terminal_seeds: route.terminal_seeds,
                seed_count: route.seeds.len() as u64,
                best_authenticated_tick: route.best_authenticated_tick,
                median_time_to_first_terminal_micros: route.median_time_to_first_terminal_micros,
                worst_time_to_first_terminal_micros: route.worst_time_to_first_terminal_micros,
                total_decisions: route.total_decisions,
                total_proposals,
                total_native_ticks: route.total_native_ticks,
                useful_decisions: route.useful_decisions,
                unique_useful_graph_expansions: route.unique_useful_graph_expansions,
                stop_reasons,
            },
            causal_chain,
            efficiency: NativeTacticCampaignEfficiencySummary {
                useful_expansions_per_second_millionths: route
                    .timing
                    .unique_useful_graph_expansions_per_second_millionths,
                native_ticks_per_second_millionths: route.timing.native_ticks_per_second_millionths,
                learner_updates_per_second_millionths: route.learner_updates_per_second_millionths,
                native_worker_utilization_per_million,
                maximum_model_replay_lag_revisions: route
                    .replay_sharing
                    .maximum_model_replay_lag_revisions,
                maximum_observed_stale_revisions: route
                    .replay_sharing
                    .maximum_observed_stale_revisions,
            },
            timing: NativeTacticCampaignTimingSummary {
                wall_micros: route.timing.wall_micros,
                process_launch_micros: route.timing.process_launch_micros,
                tactic_selection_micros: route.timing.tactic_selection_micros,
                checkpoint_branching_micros: route.timing.checkpoint_branching_micros,
                native_wait_micros: route.timing.tactic_execution_micros,
                native_simulation_occupancy_micros: route.timing.native_simulation_micros,
                ipc_and_result_transport_micros: route.timing.ipc_and_result_transport_micros,
                native_observation_capture_micros: route.timing.native_observation_capture_micros,
                native_corpus_encoding_micros: route.timing.native_corpus_encoding_micros,
                rust_state_extraction_micros: route.timing.rust_state_extraction_micros,
                model_update_micros: route.timing.model_update_micros,
                evidence_projection_micros: route.timing.evidence_projection_micros,
                persistence_micros: route.timing.persistence_micros,
                orchestration_micros: route.timing.orchestration_micros,
                reporting_micros: route.timing.reporting_micros,
            },
            work,
            resources: NativeTacticCampaignResourceSummary {
                memory_budget_bytes,
                checkpoint_cache_capacity_per_worker_bytes: route
                    .checkpoint_cache_capacity_per_worker_bytes,
                peak_worker_resident_bytes,
                peak_checkpoint_bytes: route
                    .native_restore_accounting
                    .peak_resident_checkpoint_bytes,
                peak_host_snapshot_bytes: route
                    .native_restore_accounting
                    .peak_resident_host_snapshot_bytes,
                peak_live_endpoint_entries: route
                    .native_restore_accounting
                    .peak_live_endpoint_entries,
                peak_live_endpoint_host_snapshot_bytes: route
                    .native_restore_accounting
                    .peak_live_endpoint_host_snapshot_bytes,
                memory_bound_satisfied: memory_budget_bytes
                    .is_none_or(|bound| peak_worker_resident_bytes <= bound),
            },
        };
        summary.content_sha256 = summary.compute_content_sha256()?;
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.route_report_sha256 == Digest::ZERO
            || self.identities.optimization_request_sha256 == Digest::ZERO
            || self.identities.execution_binding_sha256 == Digest::ZERO
            || self.identities.execution_plan_sha256 == Digest::ZERO
            || self.identities.objective_sha256 == Digest::ZERO
            || self.identities.feature_schema_sha256 == Digest::ZERO
            || self.identities.action_schema_sha256 == Digest::ZERO
            || self.identities.replay_snapshot_sha256 == Digest::ZERO
            || self.identities.learner_snapshot_sha256 == Digest::ZERO
            || self.treatment.seeds.is_empty()
            || self.treatment.workers == 0
            || self.treatment.proposal_width_per_decision == 0
            || self.treatment.decisions_per_seed == 0
            || self.outcome.seed_count != self.treatment.seeds.len() as u64
            || self.outcome.total_decisions < self.outcome.useful_decisions
            || self.efficiency.native_worker_utilization_per_million > 1_000_000
            || self.work.proposal_dispatches
                != self
                    .work
                    .completed_leases
                    .saturating_add(self.work.retryable_leases)
                    .saturating_add(self.work.cancelled_leases)
                    .saturating_add(self.work.failed_leases)
                    .saturating_add(self.work.unresolved_leases)
            || self.causal_chain.causal_chain_ready_for_matched_evaluation
                == self.causal_chain.first_incomplete_link.is_some()
            || self.compute_content_sha256()? != self.content_sha256
        {
            return Err(route_message("native tactic campaign summary is invalid"));
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        route: &NativeTacticRouteReport,
        plan: &NativeTacticExecutionPlan,
    ) -> Result<(), NativeTacticRouteRunError> {
        self.validate()?;
        if self != &Self::build(route, plan)? {
            return Err(route_message(
                "native tactic campaign summary is detached from its report or plan",
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

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut identity = self.clone();
        identity.content_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&identity).map_err(route_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight-native-tactic-campaign-summary/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn causal_summary(route: &NativeTacticRouteReport) -> NativeTacticCampaignCausalSummary {
    let traces = route
        .seeds
        .iter()
        .flat_map(|seed| seed.trace.iter())
        .collect::<Vec<_>>();
    let decisions_with_observed_state = traces
        .iter()
        .filter(|decision| {
            decision.before.snapshot_sha256 != Digest::ZERO
                && decision.after.snapshot_sha256 != Digest::ZERO
        })
        .count() as u64;
    let decisions_with_complete_action_surface = traces
        .iter()
        .filter(|decision| {
            !decision.applicable_tactics.is_empty()
                && decision
                    .applicable_tactics
                    .iter()
                    .all(|action| action.applicable && action.descriptor.is_some())
                && decision
                    .applicable_tactics
                    .iter()
                    .filter(|action| action.selected)
                    .count()
                    == 1
                && decision.applicable_tactics.iter().any(|action| {
                    action.selected && action.option_id == decision.selected_option_id
                })
        })
        .count() as u64;
    let decisions_with_native_proposals = traces
        .iter()
        .filter(|decision| !decision.proposal_batch.is_empty())
        .count() as u64;
    let realized_native_proposals = traces
        .iter()
        .map(|decision| decision.proposal_batch.len() as u64)
        .sum();
    let newly_published_training_rows = traces
        .iter()
        .map(|decision| decision.newly_admitted_training_rows)
        .sum();
    let consumed = traces
        .iter()
        .map(|decision| decision.learner_snapshot_sha256)
        .filter(|sha256| *sha256 != Digest::ZERO)
        .collect::<BTreeSet<_>>();
    let mut post_update_policy_decisions = 0_u64;
    let mut selected_action_changes_at_model_change = 0_u64;
    for seed in &route.seeds {
        let Some(first) = seed.trace.first() else {
            continue;
        };
        for pair in seed.trace.windows(2) {
            if pair[1].learner_snapshot_sha256 != pair[0].learner_snapshot_sha256 {
                post_update_policy_decisions = post_update_policy_decisions.saturating_add(1);
                if pair[1].selected_option_id != pair[0].selected_option_id {
                    selected_action_changes_at_model_change =
                        selected_action_changes_at_model_change.saturating_add(1);
                }
            } else if pair[1].learner_snapshot_sha256 != first.learner_snapshot_sha256 {
                post_update_policy_decisions = post_update_policy_decisions.saturating_add(1);
            }
        }
    }
    let learning_expected = route.proposal_policy.deploys_policy_updates();
    let decision_count = traces.len() as u64;
    let first_incomplete_link = if decision_count == 0
        || decisions_with_observed_state != decision_count
    {
        Some(NativeTacticCausalLink::ObservedState)
    } else if decisions_with_complete_action_surface != decision_count {
        Some(NativeTacticCausalLink::LegalActionSurface)
    } else if decisions_with_native_proposals != decision_count || realized_native_proposals == 0 {
        Some(NativeTacticCausalLink::NativeExploration)
    } else if learning_expected && newly_published_training_rows == 0 {
        Some(NativeTacticCausalLink::ExperiencePublication)
    } else if learning_expected && route.learner_updates == 0 {
        Some(NativeTacticCausalLink::LearnerUpdate)
    } else if learning_expected
        && (route.learner_authority.declared_model_snapshots_consumed == 0
            || consumed.len() < 2
            || post_update_policy_decisions == 0)
    {
        Some(NativeTacticCausalLink::PolicyDeployment)
    } else {
        None
    };

    NativeTacticCampaignCausalSummary {
        learning_expected,
        decisions_with_observed_state,
        decisions_with_complete_action_surface,
        decisions_with_native_proposals,
        realized_native_proposals,
        newly_published_training_rows,
        final_training_replay_rows: route.learner_authority.latest_training_replay_rows,
        learner_updates: route.learner_updates,
        model_snapshots_published: route.learner_authority.model_snapshots_published,
        model_snapshots_consumed: route.learner_authority.declared_model_snapshots_consumed,
        distinct_model_snapshots_consumed: consumed.len() as u64,
        post_update_policy_decisions,
        selected_action_changes_at_model_change,
        causal_chain_ready_for_matched_evaluation: first_incomplete_link.is_none(),
        first_incomplete_link,
        outcome_effect_requires_matched_control: learning_expected,
    }
}

fn work_summary(
    route: &NativeTacticRouteReport,
    total_proposals: u64,
) -> NativeTacticCampaignWorkSummary {
    let lease_accounting_complete = route.seeds.iter().all(|seed| seed.graph_metrics.is_some());
    let (
        proposal_dispatches,
        completed_leases,
        retryable_leases,
        cancelled_leases,
        failed_leases,
        unresolved_leases,
    ) = if lease_accounting_complete {
        route
            .seeds
            .iter()
            .filter_map(|seed| seed.graph_metrics.as_ref())
            .fold(
                (0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64),
                |totals, metrics| {
                    let leases = &metrics.lease_accounting;
                    (
                        totals.0.saturating_add(leases.proposal_dispatches),
                        totals.1.saturating_add(leases.completed_leases),
                        totals.2.saturating_add(leases.retryable_leases),
                        totals.3.saturating_add(leases.cancelled_leases),
                        totals.4.saturating_add(leases.failed_leases),
                        totals.5.saturating_add(leases.unresolved_leases),
                    )
                },
            )
    } else {
        (total_proposals, total_proposals, 0, 0, 0, 0)
    };
    NativeTacticCampaignWorkSummary {
        lease_accounting_complete,
        proposal_dispatches,
        completed_leases,
        retryable_leases,
        cancelled_leases,
        failed_leases,
        unresolved_leases,
        discarded_proposals: route
            .seeds
            .iter()
            .flat_map(|seed| &seed.trace)
            .flat_map(|decision| &decision.proposal_batch)
            .filter(|proposal| !proposal.retained)
            .count() as u64,
        replay_duplicate_admissions: route.replay_admission.duplicates,
        duplicate_training_transitions: route.duplicate_training_transitions,
        censored_training_transitions: route.censored_training_transitions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retained_report_and_plan() -> (Vec<u8>, NativeTacticRouteReport, NativeTacticExecutionPlan) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let evidence = root.join(
            "benchmarks/native-tactic-fault-recovery/win32-x86_64/\
             after-recovery-point-commit-portable-boundary-v1/blobs/sha256",
        );
        let compressed = fs::read(
            evidence.join("c9/80f212588942ea67ed0b48bb019913298233642ea884a4fc56a4fb1de7e3e9"),
        )
        .unwrap();
        let raw = zstd::stream::decode_all(compressed.as_slice()).unwrap();
        let route = serde_json::from_slice(&raw).unwrap();
        let plan = NativeTacticExecutionPlan::read(
            &evidence.join("86/ad166401846662dfbc6604acbeae522e09420e3369e8f7d087cde63cd8f742"),
        )
        .unwrap();
        (raw, route, plan)
    }

    #[test]
    fn compact_summary_projects_the_complete_learning_chain_and_authorities() {
        let (raw, route, plan) = retained_report_and_plan();
        let summary = NativeTacticCampaignSummary::build(&route, &plan).unwrap();
        summary.validate_against(&route, &plan).unwrap();
        let encoded = summary.to_pretty_json().unwrap();

        assert!(
            summary
                .causal_chain
                .causal_chain_ready_for_matched_evaluation
        );
        assert_eq!(summary.causal_chain.first_incomplete_link, None);
        assert_eq!(summary.causal_chain.decisions_with_observed_state, 2);
        assert_eq!(
            summary.causal_chain.decisions_with_complete_action_surface,
            2
        );
        assert_eq!(summary.causal_chain.realized_native_proposals, 4);
        assert_eq!(summary.causal_chain.newly_published_training_rows, 4);
        assert_eq!(summary.causal_chain.learner_updates, 2);
        assert_eq!(summary.causal_chain.post_update_policy_decisions, 1);
        assert_eq!(summary.work.proposal_dispatches, 4);
        assert_eq!(summary.work.unresolved_leases, 0);
        assert!(summary.resources.memory_bound_satisfied);
        assert!(encoded.len() < 16 * 1024);
        assert!(raw.len() > encoded.len() * 40);
    }

    #[test]
    fn summary_rejects_tampering_and_a_detached_plan() {
        let (_, route, plan) = retained_report_and_plan();
        let original = NativeTacticCampaignSummary::build(&route, &plan).unwrap();

        let mut tampered = original.clone();
        tampered.work.unresolved_leases = 1;
        assert!(tampered.validate().is_err());

        tampered = original;
        tampered.identities.objective_sha256.0[0] ^= 1;
        assert!(tampered.validate().is_err());

        let mut detached = plan;
        detached.proposal_width_per_decision += 1;
        assert!(NativeTacticCampaignSummary::build(&route, &detached).is_err());
    }
}
