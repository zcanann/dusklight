use super::candidate_retention::route_frames_first_hit_tick;
use super::scratch_discovery::route_report_sha256;
use super::*;
use crate::state_graph::{ActionExpansionStatus, ExpansionEvidenceAuthority, StateGraph};
use crate::tactic_q_campaign::TacticQCampaign;
use dusklight_learning::tactic_exploration::TacticSelectionReason;
use std::collections::{BTreeMap, BTreeSet};

pub const NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V2: &str =
    "dusklight-native-tactic-scratch-campaign-audit/v2";
pub const NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V3: &str =
    "dusklight-native-tactic-scratch-campaign-audit/v3";
pub const NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V4: &str =
    "dusklight-native-tactic-scratch-campaign-audit/v4";
pub const NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V5: &str =
    "dusklight-native-tactic-scratch-campaign-audit/v5";
pub const NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V6: &str =
    "dusklight-native-tactic-scratch-campaign-audit/v6";
pub const NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V7: &str =
    "dusklight-native-tactic-scratch-campaign-audit/v7";

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn is_zero_digest(value: &Digest) -> bool {
    *value == Digest::ZERO
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticScratchStopReason {
    DecisionBudgetExhausted,
    SimulatedTickBudgetExhausted,
    NativeTickBudgetExhausted,
    WallBudgetExhausted,
    LegacyUnreportedBudget,
}

impl From<NativeTacticSeedStopReason> for NativeTacticScratchStopReason {
    fn from(value: NativeTacticSeedStopReason) -> Self {
        match value {
            NativeTacticSeedStopReason::DecisionBudgetReached => Self::DecisionBudgetExhausted,
            NativeTacticSeedStopReason::SimulatedTickBudgetReached => {
                Self::SimulatedTickBudgetExhausted
            }
            NativeTacticSeedStopReason::NativeTickBudgetReached => Self::NativeTickBudgetExhausted,
            NativeTacticSeedStopReason::WallBudgetReached => Self::WallBudgetExhausted,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchTerminalImprovementAudit {
    pub decision_index: u64,
    pub cumulative_wall_micros: u64,
    pub cumulative_proposal_expansions: u64,
    pub cumulative_useful_graph_expansions: u64,
    pub authenticated_tick: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchDecisionAudit {
    pub decision_index: u64,
    pub cumulative_wall_micros: u64,
    pub learner_snapshot_sha256: Digest,
    pub replay_rows_at_decision: u64,
    pub replay_generation: u64,
    pub acquisition_rank: u64,
    pub frontier_identity: Digest,
    pub source_route_ticks: u64,
    pub checkpoint_owner_worker_slot: Option<usize>,
    pub proposal_worker_slots: Vec<usize>,
    pub restore_source: Option<NativeTacticRestoreSource>,
    pub selected_option_id: String,
    pub selection_reason: TacticSelectionReason,
    /// Exact applicable action surface and fitted value support visible to the
    /// policy before native execution. Legacy reports omitted this evidence.
    #[serde(default)]
    pub applicable_tactics: Vec<NativeTacticValueTrace>,
    /// Exact graph/model-bound action queue that produced this selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_decision: Option<TacticSchedulerDecisionTrace>,
    pub branch_acquisition: Option<TacticFrontierAcquisition>,
    pub proposal_count: u64,
    pub terminal_proposal_count: u64,
    pub retained_proposal_count: u64,
    pub completed_executable_graph_expansions: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_authenticated_tick_after_decision: Option<u64>,
    pub terminal: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchSeedAudit {
    pub seed: u64,
    pub stop_reasons: Vec<NativeTacticScratchStopReason>,
    /// Campaign incumbent already present in the imported graph before this
    /// seed made a native terminal proposal. This is a baseline, not a
    /// discovery or improvement attributable to the seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_best_authenticated_tick: Option<u64>,
    pub terminal_discovered: bool,
    pub best_authenticated_tick: Option<u64>,
    pub first_terminal_decision_index: Option<u64>,
    pub time_to_first_terminal_micros: Option<u64>,
    pub proposal_expansions_to_first_terminal: Option<u64>,
    pub useful_graph_expansions_to_first_terminal: Option<u64>,
    /// False for legacy reports written before terminal proposals carried
    /// their exact root-derived route length and cumulative graph work.
    pub terminal_improvement_timing_complete: bool,
    pub terminal_improvements: Vec<NativeTacticScratchTerminalImprovementAudit>,
    pub best_terminal_decision_index: Option<u64>,
    pub time_to_best_terminal_micros: Option<u64>,
    pub proposal_expansions_to_best_terminal: Option<u64>,
    pub useful_graph_expansions_to_best_terminal: Option<u64>,
    pub total_proposal_expansions: u64,
    pub native_ticks: u64,
    pub unique_useful_graph_expansions: u64,
    pub graph_expansion_timeline_complete: bool,
    /// True only when every decision retained a non-empty, duplicate-free
    /// action surface containing exactly one selected action.
    #[serde(default)]
    pub action_surface_timeline_complete: bool,
    /// True only when every decision retains a valid scheduler queue bound to
    /// the same learner revision reported for the decision.
    #[serde(default)]
    pub scheduler_timeline_complete: bool,
    /// Number of decisions on which each action was applicable.
    #[serde(default)]
    pub action_availability_counts: BTreeMap<String, u64>,
    /// Applicable decisions for which the learner had no fitted estimate.
    #[serde(default)]
    pub unsupported_action_availability_counts: BTreeMap<String, u64>,
    pub proposal_dispatches: u64,
    pub completed_graph_leases: u64,
    pub retryable_graph_leases: u64,
    pub cancelled_graph_leases: u64,
    pub failed_graph_leases: u64,
    pub unresolved_graph_leases: u64,
    pub terminal_path_ticks: Vec<u64>,
    pub selection_counts: BTreeMap<String, u64>,
    pub proposal_selection_counts: BTreeMap<String, u64>,
    pub learner_snapshots_consumed: Vec<Digest>,
    pub native_restore_accounting: NativeTacticRestoreAccounting,
    pub timing: NativeTacticRouteTiming,
    pub decisions: Vec<NativeTacticScratchDecisionAudit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignResourceAudit {
    pub completed_decisions: u64,
    pub declared_memory_bound_bytes: Option<u64>,
    pub configured_checkpoint_cache_capacity_per_worker_bytes: u64,
    pub configured_checkpoint_pool_capacity_bytes: u64,
    pub observed_peak_worker_resident_bytes: u64,
    pub observed_checkpoint_pool_resident_upper_bound_bytes: u64,
    pub memory_bound_satisfied: bool,
    pub maximum_allowed_stale_replay_revisions: u64,
    pub maximum_model_replay_lag_revisions: u64,
    pub maximum_lane_refresh_gap_revisions: u64,
    pub learner_staleness_bound_satisfied: bool,
    pub direct_restore_fallback_replays: u64,
    pub prefix_materializations: u64,
    pub fallback_rate_per_million_decisions: u64,
    pub fallback_bound_satisfied: bool,
    pub checkpoint_owner_available_decisions: u64,
    pub checkpoint_owner_local_decisions: u64,
    pub misrouted_owner_local_decisions: u64,
    pub checkpoint_owner_counts_by_worker: Vec<u64>,
    pub checkpoint_owner_assignment_skew: u64,
    pub passed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchCampaignAudit {
    pub schema: String,
    pub content_sha256: Digest,
    pub route_report_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub objective_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub workers: usize,
    pub proposal_policy: TacticProposalPolicy,
    pub value_treatment: TacticValueTreatment,
    pub resources: NativeTacticCampaignResourceAudit,
    /// Exact union of completed executable expansion identities across every
    /// final seed graph. Legacy audits only retained per-seed snapshot counts.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub unique_useful_graph_expansions: u64,
    #[serde(default, skip_serializing_if = "is_zero_digest")]
    pub useful_graph_expansion_set_sha256: Digest,
    pub seeds: Vec<NativeTacticScratchSeedAudit>,
}

impl NativeTacticScratchCampaignAudit {
    pub fn build(
        repository_root: &Path,
        route: &NativeTacticRouteReport,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let mut seeds = Vec::with_capacity(route.seeds.len());
        let mut campaign_useful_expansions = CampaignUsefulGraphExpansionSet::default();
        for seed in &route.seeds {
            let checkpoint_path =
                confined_checkpoint(&repository_root, Path::new(&seed.final_checkpoint))?;
            let checkpoint =
                TacticQCampaign::read_checkpoint_payload(&checkpoint_path).map_err(route_error)?;
            validate_seed_useful_graph_accounting(seed, &checkpoint.state_graph)?;
            campaign_useful_expansions.include_graph(&checkpoint.state_graph);
            seeds.push(seed_audit(route, seed, &checkpoint.state_graph)?);
        }
        seeds.sort_by_key(|seed| seed.seed);
        let unique_useful_graph_expansions = campaign_useful_expansions.count()?;
        let useful_graph_expansion_set_sha256 = campaign_useful_expansions.content_sha256();
        if route.unique_useful_graph_expansions != unique_useful_graph_expansions
            || route
                .timing
                .unique_useful_graph_expansions_per_second_millionths
                != per_second_millionths(unique_useful_graph_expansions, route.timing.wall_micros)
        {
            return Err(route_message(
                "scratch campaign route has invalid campaign-wide useful graph accounting",
            ));
        }
        let plan = NativeTacticExecutionPlan::read(Path::new(&route.execution_plan_path))?;
        if plan.identity()? != route.execution_plan_sha256
            || plan.budgets != route.resource_budgets
            || plan.seeds != route.exploration_seeds
        {
            return Err(route_message(
                "scratch campaign resource audit is detached from its execution plan",
            ));
        }
        let resources = resource_audit(route, &plan)?;
        let mut audit = Self {
            schema: NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V7.into(),
            content_sha256: Digest::ZERO,
            route_report_sha256: route_report_sha256(route)?,
            execution_plan_sha256: route.execution_plan_sha256,
            objective_sha256: route.objective_sha256,
            execution_binding_sha256: route.execution_binding_sha256,
            workers: route.workers,
            proposal_policy: route.proposal_policy,
            value_treatment: route.value_treatment,
            resources,
            unique_useful_graph_expansions,
            useful_graph_expansion_set_sha256,
            seeds,
        };
        audit.content_sha256 = audit.compute_content_sha256()?;
        audit.validate()?;
        Ok(audit)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        self.validate_without_content_identity()?;
        if self.compute_content_sha256()? != self.content_sha256 {
            return Err(route_message(
                "scratch campaign audit content identity is invalid",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_historical_json(
        &self,
        source: &[u8],
    ) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V3 {
            return Err(route_message(
                "historical scratch campaign audit schema is unsupported",
            ));
        }
        self.validate_without_content_identity()?;
        if historical_json_content_sha256(source, self.content_sha256)? != self.content_sha256 {
            return Err(route_message(
                "historical scratch campaign audit content identity is invalid",
            ));
        }
        Ok(())
    }

    fn validate_without_content_identity(&self) -> Result<(), NativeTacticRouteRunError> {
        let seed_is_valid: fn(&NativeTacticScratchSeedAudit) -> bool = match self.schema.as_str() {
            NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V2 => seed_is_valid_v2,
            NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V3 => seed_is_valid_v3_legacy,
            NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V4 => seed_is_valid_v3,
            NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V5 => seed_is_valid_v5,
            NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V6 => seed_is_valid_v6,
            NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V7 => seed_is_valid_v7,
            _ => {
                return Err(route_message(
                    "scratch campaign audit schema is unsupported",
                ));
            }
        };
        let campaign_accounting_valid = if matches!(
            self.schema.as_str(),
            NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V4
                | NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V5
                | NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V6
                | NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V7
        ) {
            self.useful_graph_expansion_set_sha256 != Digest::ZERO
                && self.unique_useful_graph_expansions
                    <= self
                        .seeds
                        .iter()
                        .map(|seed| seed.unique_useful_graph_expansions)
                        .sum()
        } else {
            self.unique_useful_graph_expansions == 0
                && self.useful_graph_expansion_set_sha256 == Digest::ZERO
        };
        if self.content_sha256 == Digest::ZERO
            || self.route_report_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
        {
            return Err(route_message(
                "scratch campaign audit has an absent authority identity",
            ));
        }
        if self.workers == 0 || self.seeds.is_empty() {
            return Err(route_message(
                "scratch campaign audit has no worker or seed evidence",
            ));
        }
        if !self
            .seeds
            .windows(2)
            .all(|pair| pair[0].seed < pair[1].seed)
        {
            return Err(route_message(
                "scratch campaign audit seeds are not strictly ordered",
            ));
        }
        if let Some(seed) = self.seeds.iter().find(|seed| !seed_is_valid(seed)) {
            return Err(route_message(format!(
                "scratch campaign audit seed {} is invalid",
                seed.seed
            )));
        }
        if !campaign_accounting_valid {
            return Err(route_message(
                "scratch campaign audit useful-expansion accounting is invalid",
            ));
        }
        if !resource_audit_is_valid(&self.resources, self.workers, &self.seeds) {
            return Err(route_message(
                "scratch campaign audit resource accounting is invalid",
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

    pub fn validate_resource_binding(
        &self,
        route: &NativeTacticRouteReport,
        plan: &NativeTacticExecutionPlan,
    ) -> Result<(), NativeTacticRouteRunError> {
        self.validate()?;
        self.validate_resource_fields(route, plan)
    }

    pub(super) fn validate_historical_resource_binding(
        &self,
        audit_source: &[u8],
        route_source: &[u8],
        route: &NativeTacticRouteReport,
        plan: &NativeTacticExecutionPlan,
    ) -> Result<(), NativeTacticRouteRunError> {
        self.validate_historical_json(audit_source)?;
        if self.route_report_sha256 != source_compatible_route_report_sha256(route, route_source)? {
            return Err(route_message(
                "historical scratch campaign audit is detached from its route identity",
            ));
        }
        self.validate_plan_and_resource_fields(route, plan)
    }

    fn validate_resource_fields(
        &self,
        route: &NativeTacticRouteReport,
        plan: &NativeTacticExecutionPlan,
    ) -> Result<(), NativeTacticRouteRunError> {
        if self.route_report_sha256 != route_report_sha256(route)? {
            return Err(route_message(
                "scratch campaign audit is detached from its route identity",
            ));
        }
        self.validate_plan_and_resource_fields(route, plan)
    }

    fn validate_plan_and_resource_fields(
        &self,
        route: &NativeTacticRouteReport,
        plan: &NativeTacticExecutionPlan,
    ) -> Result<(), NativeTacticRouteRunError> {
        if self.execution_plan_sha256 != plan.identity()? {
            return Err(route_message(
                "scratch campaign audit is detached from its execution-plan identity",
            ));
        }
        let expected_resources = resource_audit(route, plan)?;
        if self.resources != expected_resources {
            return Err(route_message(
                "scratch campaign resource fields differ from the route",
            ));
        }
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_json::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

fn historical_json_content_sha256(
    source: &[u8],
    content_sha256: Digest,
) -> Result<Digest, NativeTacticRouteRunError> {
    let source = std::str::from_utf8(source).map_err(route_error)?;
    let encoded = content_sha256.to_string();
    if source.match_indices(&encoded).count() != 1 {
        return Err(route_message(
            "historical scratch campaign audit content identity is ambiguous",
        ));
    }
    let unsigned = source.replacen(&encoded, &Digest::ZERO.to_string(), 1);
    historical_json_sha256(unsigned.as_bytes())
}

pub(super) fn historical_json_sha256(source: &[u8]) -> Result<Digest, NativeTacticRouteRunError> {
    let mut compact = Vec::with_capacity(source.len());
    let mut in_string = false;
    let mut escaped = false;
    for &byte in source {
        if in_string {
            compact.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
            compact.push(byte);
        } else if !byte.is_ascii_whitespace() {
            compact.push(byte);
        }
    }
    if in_string || escaped {
        return Err(route_message(
            "historical scratch campaign audit JSON is incomplete",
        ));
    }
    Ok(Digest(Sha256::digest(compact).into()))
}

pub(super) fn source_compatible_route_report_sha256(
    route: &NativeTacticRouteReport,
    source: &[u8],
) -> Result<Digest, NativeTacticRouteRunError> {
    if route.schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V37 {
        historical_json_sha256(source)
    } else {
        route_report_sha256(route)
    }
}

pub(super) fn supports_retained_evidence_route_report_schema(schema: &str) -> bool {
    schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V37 || supports_current_route_report_schema(schema)
}

mod resource;
use resource::resource_audit_is_valid;

pub(super) fn resource_audit(
    route: &NativeTacticRouteReport,
    plan: &NativeTacticExecutionPlan,
) -> Result<NativeTacticCampaignResourceAudit, NativeTacticRouteRunError> {
    resource::resource_audit(route, plan)
}

fn seed_audit(
    route: &NativeTacticRouteReport,
    seed: &NativeTacticSeedResult,
    graph: &StateGraph,
) -> Result<NativeTacticScratchSeedAudit, NativeTacticRouteRunError> {
    let mut proposal_expansions = 0_u64;
    let mut proposal_expansions_to_first_terminal = None;
    let mut useful_graph_expansions_to_first_terminal = None;
    let mut proposal_selection_counts = BTreeMap::<String, u64>::new();
    let mut learner_snapshots = BTreeSet::new();
    let mut action_availability_counts = BTreeMap::<String, u64>::new();
    let mut unsupported_action_availability_counts = BTreeMap::<String, u64>::new();
    let action_surface_timeline_complete = true;
    let scheduler_timeline_complete = true;
    let mut terminal_improvements = Vec::new();
    let mut terminal_improvement_timing_complete = true;
    let source_frame = graph
        .node(graph.root())
        .map(|root| root.restoration.route.tape_frames)
        .ok_or_else(|| route_message("scratch campaign audit graph root is absent"))?;
    // A later generation can begin with a terminal path imported through the
    // shared campaign graph. With no terminal proposal in the first decision,
    // a reported incumbent can only predate that decision. Preserve it as the
    // seed baseline instead of fabricating a local discovery at decision zero.
    let inherited_best_authenticated_tick = seed.trace.first().and_then(|trace| {
        (!trace
            .proposal_batch
            .iter()
            .any(|proposal| proposal.terminal))
        .then_some(trace.best_authenticated_tick_after_decision)
        .flatten()
    });
    let mut best_observed_terminal_tick = inherited_best_authenticated_tick;
    let mut decisions = Vec::with_capacity(seed.trace.len());
    for trace in &seed.trace {
        if trace.source_route_ticks != trace.before.tape_frame {
            return Err(route_message(
                "scratch campaign decision source route differs from its native boundary",
            ));
        }
        let unique_action_ids = trace
            .applicable_tactics
            .iter()
            .map(|tactic| tactic.option_id.as_str())
            .collect::<BTreeSet<_>>();
        let action_surface_valid = !trace.applicable_tactics.is_empty()
            && unique_action_ids.len() == trace.applicable_tactics.len()
            && trace
                .applicable_tactics
                .iter()
                .filter(|tactic| tactic.selected)
                .count()
                == 1
            && trace.applicable_tactics.iter().any(|tactic| {
                tactic.applicable && tactic.selected && tactic.option_id == trace.selected_option_id
            });
        if !action_surface_valid {
            return Err(route_message(format!(
                "scratch decision {} has incomplete action-surface provenance",
                trace.decision_index
            )));
        }
        let Some(scheduler) = trace.scheduler_decision.as_ref() else {
            return Err(route_message(format!(
                "scratch decision {} has no scheduler provenance",
                trace.decision_index
            )));
        };
        if scheduler.learner_model_sha256 != trace.learner_snapshot_sha256 {
            return Err(route_message(format!(
                "scratch scheduler decision {} is detached from its learner snapshot",
                trace.decision_index
            )));
        }
        scheduler.validate().map_err(route_error)?;
        if scheduler.evaluated_expansion_sha256.len() != trace.proposal_batch.len() {
            return Err(route_message(format!(
                "scratch scheduler decision {} has a detached proposal count",
                trace.decision_index
            )));
        }
        for (proposal_index, proposal) in trace.proposal_batch.iter().enumerate() {
            if let Some(issue) =
                proposal_graph_expansion_issue(graph, scheduler, proposal_index, proposal)
            {
                return Err(route_message(format!(
                    "scratch scheduler decision {} proposal {} is detached: {issue}",
                    trace.decision_index, proposal_index
                )));
            }
        }
        for tactic in &trace.applicable_tactics {
            if !tactic.applicable {
                continue;
            }
            let availability = action_availability_counts
                .entry(tactic.option_id.clone())
                .or_default();
            *availability = availability
                .checked_add(1)
                .ok_or_else(|| route_message("scratch action availability count overflows"))?;
            if tactic.mean_q.is_none() {
                let unsupported = unsupported_action_availability_counts
                    .entry(tactic.option_id.clone())
                    .or_default();
                *unsupported = unsupported.checked_add(1).ok_or_else(|| {
                    route_message("scratch unsupported action availability count overflows")
                })?;
            }
        }
        let proposal_count = u64::try_from(trace.proposal_batch.len()).map_err(route_error)?;
        proposal_expansions = proposal_expansions
            .checked_add(proposal_count)
            .ok_or_else(|| route_message("scratch proposal expansion count overflows"))?;
        if Some(trace.decision_index) == seed.first_terminal_decision_index {
            proposal_expansions_to_first_terminal = Some(proposal_expansions);
            useful_graph_expansions_to_first_terminal =
                (trace.completed_executable_graph_expansions != 0)
                    .then_some(trace.completed_executable_graph_expansions);
        }
        if trace.learner_snapshot_sha256 != Digest::ZERO {
            learner_snapshots.insert(trace.learner_snapshot_sha256);
        }
        for (proposal_index, proposal) in trace.proposal_batch.iter().enumerate() {
            let selected = proposal_selection_counts
                .entry(selection_reason_key(proposal.selection_reason)?)
                .or_default();
            *selected = selected
                .checked_add(1)
                .ok_or_else(|| route_message("scratch proposal selection count overflows"))?;
            if proposal.terminal && trace.best_authenticated_tick_after_decision.is_none() {
                let graph_authenticated =
                    trace.scheduler_decision.as_ref().is_some_and(|scheduler| {
                        proposal_matches_graph_expansion(graph, scheduler, proposal_index, proposal)
                    });
                if !graph_authenticated {
                    terminal_improvement_timing_complete = false;
                    continue;
                }
                let Some(authenticated_tick) =
                    route_frames_first_hit_tick(proposal.root_route_ticks, source_frame)
                else {
                    terminal_improvement_timing_complete = false;
                    continue;
                };
                if trace.completed_executable_graph_expansions == 0 {
                    terminal_improvement_timing_complete = false;
                    continue;
                }
                if best_observed_terminal_tick
                    .is_none_or(|incumbent| authenticated_tick < incumbent)
                {
                    best_observed_terminal_tick = Some(authenticated_tick);
                    terminal_improvements.push(NativeTacticScratchTerminalImprovementAudit {
                        decision_index: trace.decision_index,
                        cumulative_wall_micros: trace.cumulative_wall_micros,
                        cumulative_proposal_expansions: proposal_expansions,
                        cumulative_useful_graph_expansions: trace
                            .completed_executable_graph_expansions,
                        authenticated_tick,
                    });
                }
            }
        }
        if let Some(authenticated_tick) = trace.best_authenticated_tick_after_decision {
            if trace.completed_executable_graph_expansions == 0 {
                terminal_improvement_timing_complete = false;
            } else if best_observed_terminal_tick
                .is_none_or(|incumbent| authenticated_tick < incumbent)
            {
                best_observed_terminal_tick = Some(authenticated_tick);
                terminal_improvements.push(NativeTacticScratchTerminalImprovementAudit {
                    decision_index: trace.decision_index,
                    cumulative_wall_micros: trace.cumulative_wall_micros,
                    cumulative_proposal_expansions: proposal_expansions,
                    cumulative_useful_graph_expansions: trace.completed_executable_graph_expansions,
                    authenticated_tick,
                });
            }
        }
        decisions.push(NativeTacticScratchDecisionAudit {
            decision_index: trace.decision_index,
            cumulative_wall_micros: trace.cumulative_wall_micros,
            learner_snapshot_sha256: trace.learner_snapshot_sha256,
            replay_rows_at_decision: trace.replay_rows_at_decision,
            replay_generation: trace.replay_generation,
            acquisition_rank: trace.acquisition_rank,
            frontier_identity: trace.frontier_identity,
            source_route_ticks: trace.source_route_ticks,
            checkpoint_owner_worker_slot: trace.checkpoint_owner_worker_slot,
            proposal_worker_slots: trace.proposal_worker_slots.clone(),
            restore_source: trace.restore_source,
            selected_option_id: trace.selected_option_id.clone(),
            selection_reason: trace.selection_reason,
            applicable_tactics: trace.applicable_tactics.clone(),
            scheduler_decision: trace.scheduler_decision.clone(),
            branch_acquisition: trace.branch_acquisition.clone(),
            proposal_count,
            terminal_proposal_count: trace
                .proposal_batch
                .iter()
                .filter(|proposal| proposal.terminal)
                .count() as u64,
            retained_proposal_count: trace
                .proposal_batch
                .iter()
                .filter(|proposal| proposal.retained)
                .count() as u64,
            completed_executable_graph_expansions: trace.completed_executable_graph_expansions,
            best_authenticated_tick_after_decision: trace.best_authenticated_tick_after_decision,
            terminal: trace.terminal,
        });
    }
    let mut terminal_path_ticks = graph
        .nodes()
        .filter(|node| node.terminal && node.restoration.executable)
        .map(|node| {
            node.root_ticks.checked_sub(1).ok_or_else(|| {
                route_message("scratch campaign terminal node precedes its first native tick")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    terminal_path_ticks.sort_unstable();
    let graph_metrics = seed
        .graph_metrics
        .as_ref()
        .ok_or_else(|| route_message("scratch campaign audit seed lacks graph metrics"))?;
    if terminal_path_ticks.len() as u64 != graph_metrics.terminal_paths
        || terminal_path_ticks.first().copied() != seed.best_authenticated_tick
        || seed.execution_plan_sha256 != route.execution_plan_sha256
    {
        return Err(route_message(
            "scratch campaign audit terminal paths differ from seed report",
        ));
    }
    let graph_expansion_timeline_complete = decisions
        .iter()
        .all(|decision| decision.completed_executable_graph_expansions != 0)
        && decisions.windows(2).all(|pair| {
            pair[0].completed_executable_graph_expansions
                <= pair[1].completed_executable_graph_expansions
        })
        && decisions
            .last()
            .map(|decision| decision.completed_executable_graph_expansions)
            == Some(seed.unique_useful_graph_expansions);
    if terminal_improvement_timing_complete
        && best_observed_terminal_tick != seed.best_authenticated_tick
    {
        return Err(route_message(
            "scratch campaign terminal improvement trace differs from final graph",
        ));
    }
    let best_terminal = terminal_improvements.last();
    Ok(NativeTacticScratchSeedAudit {
        seed: seed.seed,
        stop_reasons: stop_reasons(route, seed),
        inherited_best_authenticated_tick,
        terminal_discovered: seed.terminal_discovered,
        best_authenticated_tick: seed.best_authenticated_tick,
        first_terminal_decision_index: seed.first_terminal_decision_index,
        time_to_first_terminal_micros: seed.time_to_first_terminal_micros,
        proposal_expansions_to_first_terminal,
        useful_graph_expansions_to_first_terminal,
        terminal_improvement_timing_complete,
        terminal_improvements: terminal_improvements.clone(),
        best_terminal_decision_index: best_terminal.map(|row| row.decision_index),
        time_to_best_terminal_micros: best_terminal.map(|row| row.cumulative_wall_micros),
        proposal_expansions_to_best_terminal: best_terminal
            .map(|row| row.cumulative_proposal_expansions),
        useful_graph_expansions_to_best_terminal: best_terminal
            .map(|row| row.cumulative_useful_graph_expansions),
        total_proposal_expansions: proposal_expansions,
        native_ticks: seed.native_ticks,
        unique_useful_graph_expansions: seed.unique_useful_graph_expansions,
        graph_expansion_timeline_complete,
        action_surface_timeline_complete,
        scheduler_timeline_complete,
        action_availability_counts,
        unsupported_action_availability_counts,
        proposal_dispatches: graph_metrics.lease_accounting.proposal_dispatches,
        completed_graph_leases: graph_metrics.lease_accounting.completed_leases,
        retryable_graph_leases: graph_metrics.lease_accounting.retryable_leases,
        cancelled_graph_leases: graph_metrics.lease_accounting.cancelled_leases,
        failed_graph_leases: graph_metrics.lease_accounting.failed_leases,
        unresolved_graph_leases: graph_metrics.lease_accounting.unresolved_leases,
        terminal_path_ticks,
        selection_counts: seed.selection_counts.clone(),
        proposal_selection_counts,
        learner_snapshots_consumed: learner_snapshots.into_iter().collect(),
        native_restore_accounting: seed.native_restore_accounting.clone(),
        timing: seed.timing.clone(),
        decisions,
    })
}

fn stop_reasons(
    route: &NativeTacticRouteReport,
    seed: &NativeTacticSeedResult,
) -> Vec<NativeTacticScratchStopReason> {
    if !seed.stop_reasons.is_empty() {
        return seed.stop_reasons.iter().copied().map(Into::into).collect();
    }
    let mut reasons = Vec::new();
    if seed.decisions >= route.decisions_per_seed {
        reasons.push(NativeTacticScratchStopReason::DecisionBudgetExhausted);
    }
    if route
        .resource_budgets
        .native_ticks
        .reached(seed.native_ticks)
    {
        reasons.push(NativeTacticScratchStopReason::NativeTickBudgetExhausted);
    }
    if seed.wall_budget_reached {
        reasons.push(NativeTacticScratchStopReason::WallBudgetExhausted);
    }
    if reasons.is_empty() {
        reasons.push(NativeTacticScratchStopReason::LegacyUnreportedBudget);
    }
    reasons
}

fn seed_is_valid_v5(seed: &NativeTacticScratchSeedAudit) -> bool {
    if !seed_is_valid_v3(seed) || !seed.terminal_improvement_timing_complete {
        return false;
    }
    graph_authoritative_terminal_timeline_is_valid(seed, false)
}

fn seed_is_valid_v6(seed: &NativeTacticScratchSeedAudit) -> bool {
    seed.action_surface_timeline_complete
        && seed.scheduler_timeline_complete
        && seed_is_valid_v5(seed)
}

fn seed_is_valid_v7(seed: &NativeTacticScratchSeedAudit) -> bool {
    seed.action_surface_timeline_complete
        && seed.scheduler_timeline_complete
        && seed.terminal_improvement_timing_complete
        && seed_is_valid_v3_with_selected_applicability(seed, true, true)
        && graph_authoritative_terminal_timeline_is_valid(seed, true)
}

fn graph_authoritative_terminal_timeline_is_valid(
    seed: &NativeTacticScratchSeedAudit,
    allow_inherited: bool,
) -> bool {
    let mut prior_best = allow_inherited
        .then_some(seed.inherited_best_authenticated_tick)
        .flatten();
    for decision in &seed.decisions {
        match decision.best_authenticated_tick_after_decision {
            None if prior_best.is_some() => return false,
            None => {}
            Some(best) => {
                if prior_best.is_none()
                    && (Some(decision.decision_index) != seed.first_terminal_decision_index
                        || decision.terminal_proposal_count == 0)
                {
                    return false;
                }
                if prior_best.is_some_and(|prior| best > prior) {
                    return false;
                }
                if prior_best.is_some_and(|prior| best < prior)
                    && decision.terminal_proposal_count == 0
                {
                    return false;
                }
                prior_best = Some(best);
            }
        }
    }
    prior_best == seed.best_authenticated_tick
}

fn seed_is_valid_v3(seed: &NativeTacticScratchSeedAudit) -> bool {
    seed_is_valid_v3_with_selected_applicability(seed, true, false)
}

fn seed_is_valid_v3_legacy(seed: &NativeTacticScratchSeedAudit) -> bool {
    seed_is_valid_v3_with_selected_applicability(seed, false, false)
}

fn seed_is_valid_v3_with_selected_applicability(
    seed: &NativeTacticScratchSeedAudit,
    require_selected_applicable: bool,
    allow_inherited: bool,
) -> bool {
    let Some(total_proposals) = seed.decisions.iter().try_fold(0_u64, |total, decision| {
        total.checked_add(decision.proposal_count)
    }) else {
        return false;
    };
    let first_terminal_valid = first_terminal_evidence_is_valid(seed, allow_inherited);
    seed.decisions.windows(2).all(|pair| {
        pair[0].decision_index < pair[1].decision_index
            && pair[0].cumulative_wall_micros <= pair[1].cumulative_wall_micros
    }) && !seed.stop_reasons.is_empty()
        && total_proposals == seed.total_proposal_expansions
        && seed.completed_graph_leases == total_proposals
        && seed.proposal_dispatches
            == seed
                .completed_graph_leases
                .saturating_add(seed.retryable_graph_leases)
                .saturating_add(seed.cancelled_graph_leases)
                .saturating_add(seed.failed_graph_leases)
                .saturating_add(seed.unresolved_graph_leases)
        && seed.unresolved_graph_leases == 0
        && (!seed.action_surface_timeline_complete
            || seed.decisions.iter().all(|decision| {
                let unique = decision
                    .applicable_tactics
                    .iter()
                    .map(|tactic| tactic.option_id.as_str())
                    .collect::<BTreeSet<_>>();
                !decision.applicable_tactics.is_empty()
                    && unique.len() == decision.applicable_tactics.len()
                    && decision
                        .applicable_tactics
                        .iter()
                        .filter(|tactic| tactic.selected)
                        .count()
                        == 1
                    && decision.applicable_tactics.iter().any(|tactic| {
                        (!require_selected_applicable || tactic.applicable)
                            && tactic.selected
                            && tactic.option_id == decision.selected_option_id
                    })
            }))
        && (!seed.scheduler_timeline_complete
            || seed.decisions.iter().all(|decision| {
                decision
                    .scheduler_decision
                    .as_ref()
                    .is_some_and(|scheduler| {
                        scheduler.learner_model_sha256 == decision.learner_snapshot_sha256
                            && scheduler.validate().is_ok()
                            && usize::try_from(decision.proposal_count).ok()
                                == Some(scheduler.evaluated_expansion_sha256.len())
                    })
            }))
        && first_terminal_valid
        && seed.terminal_discovered == !seed.terminal_path_ticks.is_empty()
        && seed
            .terminal_path_ticks
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
        && seed.best_authenticated_tick == seed.terminal_path_ticks.first().copied()
        && terminal_improvement_timeline_is_valid(seed, allow_inherited)
}

fn seed_is_valid_v2(seed: &NativeTacticScratchSeedAudit) -> bool {
    let Some(total_proposals) = seed.decisions.iter().try_fold(0_u64, |total, decision| {
        total.checked_add(decision.proposal_count)
    }) else {
        return false;
    };
    let first_terminal_valid = match (
        seed.terminal_discovered,
        seed.first_terminal_decision_index,
        seed.time_to_first_terminal_micros,
        seed.proposal_expansions_to_first_terminal,
        seed.best_authenticated_tick,
    ) {
        (true, Some(decision), Some(_), Some(expansions), Some(_)) => {
            expansions > 0
                && seed
                    .decisions
                    .iter()
                    .any(|row| row.decision_index == decision && row.terminal_proposal_count > 0)
        }
        (false, None, None, None, None) => true,
        _ => false,
    };
    seed.decisions
        .windows(2)
        .all(|pair| pair[0].decision_index < pair[1].decision_index)
        && !seed.stop_reasons.is_empty()
        && total_proposals == seed.total_proposal_expansions
        && seed.completed_graph_leases == total_proposals
        && seed.proposal_dispatches
            == seed
                .completed_graph_leases
                .saturating_add(seed.retryable_graph_leases)
                .saturating_add(seed.cancelled_graph_leases)
                .saturating_add(seed.failed_graph_leases)
                .saturating_add(seed.unresolved_graph_leases)
        && seed.unresolved_graph_leases == 0
        && (!seed.action_surface_timeline_complete
            || seed.decisions.iter().all(|decision| {
                !decision.applicable_tactics.is_empty()
                    && decision
                        .applicable_tactics
                        .iter()
                        .filter(|tactic| tactic.selected)
                        .count()
                        == 1
                    && decision.applicable_tactics.iter().any(|tactic| {
                        tactic.selected && tactic.option_id == decision.selected_option_id
                    })
            }))
        && (!seed.scheduler_timeline_complete
            || seed.decisions.iter().all(|decision| {
                decision
                    .scheduler_decision
                    .as_ref()
                    .is_some_and(|scheduler| {
                        scheduler.learner_model_sha256 == decision.learner_snapshot_sha256
                            && scheduler.validate().is_ok()
                    })
            }))
        && first_terminal_valid
        && seed.terminal_discovered == !seed.terminal_path_ticks.is_empty()
        && seed.best_authenticated_tick == seed.terminal_path_ticks.first().copied()
        && (!seed.terminal_improvement_timing_complete
            || (seed.best_terminal_decision_index.is_some() == seed.terminal_discovered
                && seed.time_to_best_terminal_micros.is_some() == seed.terminal_discovered
                && seed.proposal_expansions_to_best_terminal.is_some() == seed.terminal_discovered
                && seed.useful_graph_expansions_to_best_terminal.is_some()
                    == seed.terminal_discovered
                && seed
                    .terminal_improvements
                    .last()
                    .map(|row| row.authenticated_tick)
                    == seed.best_authenticated_tick))
}

fn first_terminal_evidence_is_valid(
    seed: &NativeTacticScratchSeedAudit,
    allow_inherited: bool,
) -> bool {
    match (
        seed.terminal_discovered,
        seed.first_terminal_decision_index,
        seed.time_to_first_terminal_micros,
        seed.proposal_expansions_to_first_terminal,
        seed.useful_graph_expansions_to_first_terminal,
        seed.best_authenticated_tick,
    ) {
        (
            true,
            Some(decision_index),
            Some(wall_micros),
            Some(proposal_expansions),
            Some(useful_expansions),
            Some(_),
        ) => {
            let Some((position, decision)) = seed
                .decisions
                .iter()
                .enumerate()
                .find(|(_, row)| row.decision_index == decision_index)
            else {
                return false;
            };
            let expected_proposals = seed.decisions[..=position]
                .iter()
                .try_fold(0_u64, |total, row| total.checked_add(row.proposal_count));
            wall_micros == decision.cumulative_wall_micros
                && expected_proposals == Some(proposal_expansions)
                && proposal_expansions > 0
                && useful_expansions == decision.completed_executable_graph_expansions
                && useful_expansions > 0
                && decision.terminal_proposal_count > 0
        }
        (false, None, None, None, None, None) => true,
        (true, None, None, None, None, Some(best))
            if allow_inherited && seed.inherited_best_authenticated_tick == Some(best) =>
        {
            true
        }
        _ => false,
    }
}

fn terminal_improvement_timeline_is_valid(
    seed: &NativeTacticScratchSeedAudit,
    allow_inherited: bool,
) -> bool {
    if !seed.terminal_discovered {
        return (!allow_inherited || seed.inherited_best_authenticated_tick.is_none())
            && seed.terminal_improvements.is_empty()
            && seed.best_terminal_decision_index.is_none()
            && seed.time_to_best_terminal_micros.is_none()
            && seed.proposal_expansions_to_best_terminal.is_none()
            && seed.useful_graph_expansions_to_best_terminal.is_none();
    }
    if !seed.terminal_improvement_timing_complete {
        return true;
    }
    if seed.terminal_improvements.is_empty() {
        return allow_inherited
            && seed.inherited_best_authenticated_tick == seed.best_authenticated_tick
            && seed.best_terminal_decision_index.is_none()
            && seed.time_to_best_terminal_micros.is_none()
            && seed.proposal_expansions_to_best_terminal.is_none()
            && seed.useful_graph_expansions_to_best_terminal.is_none();
    }
    let Some(first) = seed.terminal_improvements.first() else {
        return false;
    };
    let Some(last) = seed.terminal_improvements.last() else {
        return false;
    };
    let first_improvement_matches_first_discovery = allow_inherited
        && seed.inherited_best_authenticated_tick.is_some()
        || (Some(first.decision_index) == seed.first_terminal_decision_index
            && Some(first.cumulative_wall_micros) == seed.time_to_first_terminal_micros
            && Some(first.cumulative_proposal_expansions)
                == seed.proposal_expansions_to_first_terminal
            && Some(first.cumulative_useful_graph_expansions)
                == seed.useful_graph_expansions_to_first_terminal);
    if !first_improvement_matches_first_discovery
        || Some(last.decision_index) != seed.best_terminal_decision_index
        || Some(last.cumulative_wall_micros) != seed.time_to_best_terminal_micros
        || Some(last.cumulative_proposal_expansions) != seed.proposal_expansions_to_best_terminal
        || Some(last.cumulative_useful_graph_expansions)
            != seed.useful_graph_expansions_to_best_terminal
        || Some(last.authenticated_tick) != seed.best_authenticated_tick
        || (allow_inherited
            && seed
                .inherited_best_authenticated_tick
                .is_some_and(|baseline| first.authenticated_tick >= baseline))
        || seed.terminal_improvements.windows(2).any(|pair| {
            pair[0].authenticated_tick <= pair[1].authenticated_tick
                || pair[0].decision_index > pair[1].decision_index
                || pair[0].cumulative_wall_micros > pair[1].cumulative_wall_micros
                || pair[0].cumulative_proposal_expansions > pair[1].cumulative_proposal_expansions
                || pair[0].cumulative_useful_graph_expansions
                    > pair[1].cumulative_useful_graph_expansions
        })
    {
        return false;
    }
    seed.terminal_improvements.iter().all(|improvement| {
        let Some((position, decision)) = seed
            .decisions
            .iter()
            .enumerate()
            .find(|(_, row)| row.decision_index == improvement.decision_index)
        else {
            return false;
        };
        let expected_proposals = seed.decisions[..=position]
            .iter()
            .try_fold(0_u64, |total, row| total.checked_add(row.proposal_count));
        improvement.cumulative_wall_micros == decision.cumulative_wall_micros
            && expected_proposals == Some(improvement.cumulative_proposal_expansions)
            && improvement.cumulative_useful_graph_expansions
                == decision.completed_executable_graph_expansions
            && improvement.cumulative_proposal_expansions > 0
            && improvement.cumulative_useful_graph_expansions > 0
            && decision.terminal_proposal_count > 0
            && (decision.best_authenticated_tick_after_decision
                == Some(improvement.authenticated_tick)
                || (decision.best_authenticated_tick_after_decision.is_none()
                    && decision.terminal_proposal_count > 0))
    })
}

fn proposal_matches_graph_expansion(
    graph: &StateGraph,
    scheduler: &crate::tactic_q_campaign::TacticSchedulerDecisionTrace,
    proposal_index: usize,
    proposal: &NativeTacticProposalTrace,
) -> bool {
    proposal_graph_expansion_issue(graph, scheduler, proposal_index, proposal).is_none()
}

fn proposal_graph_expansion_issue(
    graph: &StateGraph,
    scheduler: &crate::tactic_q_campaign::TacticSchedulerDecisionTrace,
    proposal_index: usize,
    proposal: &NativeTacticProposalTrace,
) -> Option<&'static str> {
    let Some(expansion_sha256) = scheduler
        .evaluated_expansion_sha256
        .get(proposal_index)
        .copied()
    else {
        return Some("scheduler expansion is absent");
    };
    let Some(expansion) = graph.expansion(expansion_sha256) else {
        return Some("final graph expansion is absent");
    };
    let (
        ActionExpansionStatus::Completed {
            authority: ExpansionEvidenceAuthority::Executable,
            evidence,
            ..
        },
        Some(target),
        Some(execution),
    ) = (
        &expansion.status,
        expansion.target,
        expansion.execution.as_ref(),
    )
    else {
        return Some("final graph expansion lacks executable completion");
    };
    let Some(target_node) = graph.node(target) else {
        return Some("final graph target is absent");
    };
    if expansion.action.option_id != proposal.option_id {
        return Some("action identity differs");
    }
    if execution.option_id != expansion.action.option_id
        || execution.option_type != expansion.action.option_type
        || execution.parameters != expansion.action.parameters
    {
        return Some("execution differs from the scheduled action");
    }
    if execution.duration.realized_ticks != proposal.realized_ticks {
        return Some("realized duration differs");
    }
    // A later exact-state route relaxation may shorten the source or target
    // node's canonical restoration route. The expansion execution and its
    // authenticated native transition are immutable historical evidence, so
    // bind the proposal to those rather than to mutable restorations.
    if execution
        .realized_tape_range
        .start_frame
        .checked_add(u64::from(proposal.realized_ticks))
        != Some(execution.realized_tape_range.end_frame_exclusive)
        || execution.realized_tape_range.end_frame_exclusive != proposal.root_route_ticks
    {
        return Some("historical realized tape range differs");
    }
    if target_node.terminal != proposal.terminal
        || target_node.id.state_sha256 != proposal.after_snapshot_sha256
    {
        return Some("final graph target differs");
    }
    if !evidence.values().any(|row| {
        row.authority == ExpansionEvidenceAuthority::Executable
            && row.transition.before_state_sha256 == expansion.source.state_sha256
            && row.transition.after_state_sha256 == proposal.after_snapshot_sha256
            && row.transition.execution == *execution
            && row.transition.value_sample.action == expansion.action
            && row.transition.value_sample.realized_tape_sha256 == proposal.emitted_tape_sha256
            && row.transition.value_sample.reward.to_bits() == proposal.reward.to_bits()
            && row.transition.value_sample.terminal == proposal.terminal
    }) {
        return Some("authenticated native evidence differs");
    }
    None
}

fn selection_reason_key(
    reason: TacticSelectionReason,
) -> Result<String, NativeTacticRouteRunError> {
    serde_json::to_value(reason)
        .map_err(route_error)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| route_message("tactic selection reason is not a string"))
}

fn confined_checkpoint(
    repository_root: &Path,
    declared: &Path,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    let candidate = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        repository_root.join(declared)
    };
    let resolved = candidate.canonicalize().map_err(route_error)?;
    if !resolved.starts_with(repository_root) || !resolved.is_file() {
        return Err(route_message(
            "scratch campaign checkpoint is outside the repository or absent",
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_json_identity_preserves_tokens_and_ignores_formatting() {
        let unsigned = concat!(
            r#"{"schema":"legacy","content_sha256":""#,
            "0000000000000000000000000000000000000000000000000000000000000000",
            r#"","label":"space stays here","escaped":"quote: \""}"#,
        );
        let identity = Digest(Sha256::digest(unsigned.as_bytes()).into());
        let source = format!(
            "{{\n  \"schema\": \"legacy\",\n  \"content_sha256\": \"{identity}\",\n  \"label\": \"space stays here\",\n  \"escaped\": \"quote: \\\"\"\n}}\n"
        );
        assert_eq!(
            historical_json_content_sha256(source.as_bytes(), identity).unwrap(),
            identity
        );
    }

    fn terminal_decision(
        decision_index: u64,
        cumulative_wall_micros: u64,
        completed_executable_graph_expansions: u64,
    ) -> NativeTacticScratchDecisionAudit {
        NativeTacticScratchDecisionAudit {
            decision_index,
            cumulative_wall_micros,
            learner_snapshot_sha256: Digest::ZERO,
            replay_rows_at_decision: 0,
            replay_generation: 0,
            acquisition_rank: 0,
            frontier_identity: Digest::ZERO,
            source_route_ticks: 0,
            checkpoint_owner_worker_slot: None,
            proposal_worker_slots: Vec::new(),
            restore_source: None,
            selected_option_id: "walk".to_owned(),
            selection_reason: TacticSelectionReason::GoalReachability,
            applicable_tactics: Vec::new(),
            scheduler_decision: None,
            branch_acquisition: None,
            proposal_count: 1,
            terminal_proposal_count: 1,
            retained_proposal_count: 1,
            completed_executable_graph_expansions,
            best_authenticated_tick_after_decision: None,
            terminal: true,
        }
    }

    fn valid_terminal_seed_audit() -> NativeTacticScratchSeedAudit {
        NativeTacticScratchSeedAudit {
            seed: 7,
            stop_reasons: vec![NativeTacticScratchStopReason::DecisionBudgetExhausted],
            inherited_best_authenticated_tick: None,
            terminal_discovered: true,
            best_authenticated_tick: Some(9),
            first_terminal_decision_index: Some(0),
            time_to_first_terminal_micros: Some(10),
            proposal_expansions_to_first_terminal: Some(1),
            useful_graph_expansions_to_first_terminal: Some(1),
            terminal_improvement_timing_complete: true,
            terminal_improvements: vec![NativeTacticScratchTerminalImprovementAudit {
                decision_index: 0,
                cumulative_wall_micros: 10,
                cumulative_proposal_expansions: 1,
                cumulative_useful_graph_expansions: 1,
                authenticated_tick: 9,
            }],
            best_terminal_decision_index: Some(0),
            time_to_best_terminal_micros: Some(10),
            proposal_expansions_to_best_terminal: Some(1),
            useful_graph_expansions_to_best_terminal: Some(1),
            total_proposal_expansions: 1,
            native_ticks: 9,
            unique_useful_graph_expansions: 1,
            graph_expansion_timeline_complete: true,
            action_surface_timeline_complete: false,
            scheduler_timeline_complete: false,
            action_availability_counts: BTreeMap::new(),
            unsupported_action_availability_counts: BTreeMap::new(),
            proposal_dispatches: 1,
            completed_graph_leases: 1,
            retryable_graph_leases: 0,
            cancelled_graph_leases: 0,
            failed_graph_leases: 0,
            unresolved_graph_leases: 0,
            terminal_path_ticks: vec![9],
            selection_counts: BTreeMap::new(),
            proposal_selection_counts: BTreeMap::new(),
            learner_snapshots_consumed: vec![Digest::ZERO],
            native_restore_accounting: NativeTacticRestoreAccounting::default(),
            timing: NativeTacticRouteTiming::default(),
            decisions: vec![terminal_decision(0, 10, 1)],
        }
    }

    #[test]
    fn selection_reason_keys_use_the_stable_wire_names() {
        assert_eq!(
            selection_reason_key(TacticSelectionReason::GoalReachability).unwrap(),
            "goal_reachability"
        );
        assert_eq!(
            selection_reason_key(TacticSelectionReason::RandomBaseline).unwrap(),
            "random_baseline"
        );
    }

    #[test]
    fn exact_stop_reasons_have_stable_audit_names() {
        assert_eq!(
            NativeTacticScratchStopReason::from(
                NativeTacticSeedStopReason::SimulatedTickBudgetReached
            ),
            NativeTacticScratchStopReason::SimulatedTickBudgetExhausted
        );
        assert_eq!(
            NativeTacticScratchStopReason::from(NativeTacticSeedStopReason::WallBudgetReached),
            NativeTacticScratchStopReason::WallBudgetExhausted
        );
    }

    #[test]
    fn resource_audit_recomputes_every_bound_and_rejects_stale_conclusions() {
        let mut resources = NativeTacticCampaignResourceAudit {
            completed_decisions: 0,
            declared_memory_bound_bytes: Some(1_000),
            configured_checkpoint_cache_capacity_per_worker_bytes: 400,
            configured_checkpoint_pool_capacity_bytes: 800,
            observed_peak_worker_resident_bytes: 300,
            observed_checkpoint_pool_resident_upper_bound_bytes: 600,
            memory_bound_satisfied: true,
            maximum_allowed_stale_replay_revisions: 2,
            maximum_model_replay_lag_revisions: 1,
            maximum_lane_refresh_gap_revisions: 7,
            learner_staleness_bound_satisfied: true,
            direct_restore_fallback_replays: 0,
            prefix_materializations: 0,
            fallback_rate_per_million_decisions: 0,
            fallback_bound_satisfied: true,
            checkpoint_owner_available_decisions: 0,
            checkpoint_owner_local_decisions: 0,
            misrouted_owner_local_decisions: 0,
            checkpoint_owner_counts_by_worker: vec![0, 0],
            checkpoint_owner_assignment_skew: 0,
            passed: true,
        };
        assert!(resource_audit_is_valid(&resources, 2, &[]));

        resources.maximum_model_replay_lag_revisions = 3;
        assert!(!resource_audit_is_valid(&resources, 2, &[]));
        resources.maximum_model_replay_lag_revisions = 1;
        resources.observed_checkpoint_pool_resident_upper_bound_bytes = 599;
        assert!(!resource_audit_is_valid(&resources, 2, &[]));
    }

    #[test]
    fn terminal_timeline_recomputes_first_and_best_evidence_from_decisions() {
        let seed = valid_terminal_seed_audit();
        assert!(first_terminal_evidence_is_valid(&seed, false));
        assert!(terminal_improvement_timeline_is_valid(&seed, false));
        assert!(seed_is_valid_v3(&seed));

        let mut detached_wall = seed.clone();
        detached_wall.time_to_first_terminal_micros = Some(11);
        assert!(!first_terminal_evidence_is_valid(&detached_wall, false));

        let mut detached_work = seed.clone();
        detached_work.proposal_expansions_to_first_terminal = Some(2);
        assert!(!first_terminal_evidence_is_valid(&detached_work, false));

        let mut detached_useful_work = seed;
        detached_useful_work.useful_graph_expansions_to_first_terminal = Some(2);
        assert!(!first_terminal_evidence_is_valid(
            &detached_useful_work,
            false
        ));
    }

    #[test]
    fn v5_requires_graph_authoritative_best_tick_after_each_terminal_decision() {
        let mut seed = valid_terminal_seed_audit();
        assert!(!graph_authoritative_terminal_timeline_is_valid(
            &seed, false
        ));
        seed.decisions[0].best_authenticated_tick_after_decision = Some(9);
        assert!(graph_authoritative_terminal_timeline_is_valid(&seed, false));
        assert!(seed_is_valid_v5(&seed));
        assert!(!seed_is_valid_v6(&seed));

        seed.decisions[0].best_authenticated_tick_after_decision = Some(10);
        assert!(!graph_authoritative_terminal_timeline_is_valid(
            &seed, false
        ));
    }

    #[test]
    fn v6_fails_closed_without_complete_action_and_scheduler_provenance() {
        let mut seed = valid_terminal_seed_audit();
        seed.decisions[0].best_authenticated_tick_after_decision = Some(9);
        assert!(seed_is_valid_v3(&seed));
        assert!(!seed_is_valid_v6(&seed));

        seed.action_surface_timeline_complete = true;
        assert!(!seed_is_valid_v6(&seed));
        seed.action_surface_timeline_complete = false;
        seed.scheduler_timeline_complete = true;
        assert!(!seed_is_valid_v6(&seed));
    }

    #[test]
    fn inherited_campaign_incumbent_is_a_baseline_not_a_seed_discovery() {
        let mut seed = valid_terminal_seed_audit();
        seed.inherited_best_authenticated_tick = Some(9);
        seed.first_terminal_decision_index = None;
        seed.time_to_first_terminal_micros = None;
        seed.proposal_expansions_to_first_terminal = None;
        seed.useful_graph_expansions_to_first_terminal = None;
        seed.terminal_improvements.clear();
        seed.best_terminal_decision_index = None;
        seed.time_to_best_terminal_micros = None;
        seed.proposal_expansions_to_best_terminal = None;
        seed.useful_graph_expansions_to_best_terminal = None;
        seed.decisions[0].terminal_proposal_count = 0;
        seed.decisions[0].terminal = false;
        seed.decisions[0].best_authenticated_tick_after_decision = Some(9);

        assert!(first_terminal_evidence_is_valid(&seed, true));
        assert!(terminal_improvement_timeline_is_valid(&seed, true));
        assert!(graph_authoritative_terminal_timeline_is_valid(&seed, true));
        assert!(!first_terminal_evidence_is_valid(&seed, false));
    }

    #[test]
    fn inherited_campaign_incumbent_allows_a_strict_local_improvement() {
        let mut seed = valid_terminal_seed_audit();
        seed.inherited_best_authenticated_tick = Some(10);
        seed.decisions[0].best_authenticated_tick_after_decision = Some(9);

        assert!(first_terminal_evidence_is_valid(&seed, true));
        assert!(terminal_improvement_timeline_is_valid(&seed, true));
        assert!(graph_authoritative_terminal_timeline_is_valid(&seed, true));

        seed.decisions[0].terminal_proposal_count = 0;
        assert!(!graph_authoritative_terminal_timeline_is_valid(&seed, true));
    }

    #[test]
    fn terminal_improvements_must_strictly_improve_and_match_decision_totals() {
        let mut seed = valid_terminal_seed_audit();
        seed.best_authenticated_tick = Some(8);
        seed.best_terminal_decision_index = Some(1);
        seed.time_to_best_terminal_micros = Some(20);
        seed.proposal_expansions_to_best_terminal = Some(2);
        seed.useful_graph_expansions_to_best_terminal = Some(2);
        seed.total_proposal_expansions = 2;
        seed.proposal_dispatches = 2;
        seed.completed_graph_leases = 2;
        seed.terminal_path_ticks = vec![8, 9];
        seed.decisions.push(terminal_decision(1, 20, 2));
        seed.terminal_improvements
            .push(NativeTacticScratchTerminalImprovementAudit {
                decision_index: 1,
                cumulative_wall_micros: 20,
                cumulative_proposal_expansions: 2,
                cumulative_useful_graph_expansions: 2,
                authenticated_tick: 8,
            });
        assert!(terminal_improvement_timeline_is_valid(&seed, false));
        assert!(seed_is_valid_v3(&seed));

        let mut non_improving = seed.clone();
        non_improving.terminal_improvements[1].authenticated_tick = 9;
        non_improving.best_authenticated_tick = Some(9);
        assert!(!terminal_improvement_timeline_is_valid(
            &non_improving,
            false
        ));

        let mut detached_work = seed;
        detached_work.terminal_improvements[1].cumulative_proposal_expansions = 1;
        assert!(!terminal_improvement_timeline_is_valid(
            &detached_work,
            false
        ));
    }

    #[test]
    fn absent_terminal_rejects_improvement_claims_even_when_timing_is_incomplete() {
        let mut seed = valid_terminal_seed_audit();
        seed.terminal_discovered = false;
        seed.best_authenticated_tick = None;
        seed.first_terminal_decision_index = None;
        seed.time_to_first_terminal_micros = None;
        seed.proposal_expansions_to_first_terminal = None;
        seed.useful_graph_expansions_to_first_terminal = None;
        seed.best_terminal_decision_index = None;
        seed.time_to_best_terminal_micros = None;
        seed.proposal_expansions_to_best_terminal = None;
        seed.useful_graph_expansions_to_best_terminal = None;
        seed.terminal_improvement_timing_complete = false;
        assert!(!terminal_improvement_timeline_is_valid(&seed, false));
    }

    #[test]
    fn legacy_v2_validation_does_not_fabricate_v3_timeline_completeness() {
        let mut seed = valid_terminal_seed_audit();
        seed.useful_graph_expansions_to_first_terminal = None;
        seed.terminal_improvement_timing_complete = false;
        assert!(seed_is_valid_v2(&seed));
        assert!(!seed_is_valid_v3(&seed));
    }
}
