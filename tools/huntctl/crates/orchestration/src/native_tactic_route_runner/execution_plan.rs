use super::*;

pub const NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V1: &str =
    "dusklight-native-tactic-execution-plan/v1";
pub const NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V2: &str =
    "dusklight-native-tactic-execution-plan/v2";
pub const NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V3: &str =
    "dusklight-native-tactic-execution-plan/v3";
pub const NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V4: &str =
    "dusklight-native-tactic-execution-plan/v4";
pub const NATIVE_TACTIC_EXECUTION_PLAN_FILE: &str = "execution-plan.dtp";
const PLAN_MAGIC: &[u8; 8] = b"DSKTPN01";
const PLAN_VERSION: u16 = 4;
const PLAN_HEADER_BYTES: usize = 8 + 2 + 8 + 32;
const MAXIMUM_PLAN_BYTES: usize = 4 * 1024 * 1024;
pub(super) const EPISODE_GROUP_STRIDE: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticLaneRole {
    TerminalSupport,
    RankedExploration,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NativeTacticAcquisitionPlan {
    FixedRank {
        rank: u64,
    },
    CyclicSupportAndRanks {
        cycle_width: u32,
        ranked_lanes_per_cycle: u32,
    },
}

impl NativeTacticAcquisitionPlan {
    pub fn rank(self, decision_index: u64) -> u64 {
        match self {
            Self::FixedRank { rank } => rank,
            Self::CyclicSupportAndRanks {
                cycle_width,
                ranked_lanes_per_cycle,
            } => {
                let cycle_width = u64::from(cycle_width);
                let lane = decision_index % cycle_width;
                if lane == 0 {
                    0
                } else {
                    (decision_index / cycle_width)
                        .saturating_mul(u64::from(ranked_lanes_per_cycle))
                        .saturating_add(lane)
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticInterventionPlan {
    None,
    DemonstrationFrontierOnce,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticLanePlan {
    pub lane_index: usize,
    pub generation_index: usize,
    pub generation_lane_index: usize,
    pub seed: u64,
    pub role: NativeTacticLaneRole,
    pub acquisition: NativeTacticAcquisitionPlan,
    pub epsilon_per_million: u32,
    pub intervention: NativeTacticInterventionPlan,
    pub root_refresh_phase: u32,
    pub episode_group_base: u64,
}

impl NativeTacticLanePlan {
    pub fn episode_group(&self, episode: u64) -> Result<u64, NativeTacticRouteRunError> {
        self.episode_group_base
            .checked_add(episode)
            .ok_or_else(|| route_message("episode group overflowed"))
    }

    pub fn root_refresh_due(&self, episode: u64, cadence: u32) -> bool {
        episode.saturating_add(u64::from(self.root_refresh_phase)) % u64::from(cadence) == 0
    }

    pub fn owns_episode_group(&self, episode_group: u64) -> bool {
        episode_group >= self.episode_group_base
            && episode_group < self.episode_group_base.saturating_add(EPISODE_GROUP_STRIDE)
    }

    pub fn counterfactual_episode_group(
        &self,
        decision_index: u64,
        proposal_index: usize,
        decisions_per_lane: u64,
        proposal_width_per_decision: usize,
    ) -> Result<u64, NativeTacticRouteRunError> {
        if decision_index >= decisions_per_lane
            || proposal_index == 0
            || proposal_index >= proposal_width_per_decision
        {
            return Err(route_message(
                "counterfactual tactic episode identity is outside its execution plan",
            ));
        }
        let sibling_width = u64::try_from(proposal_width_per_decision - 1).map_err(route_error)?;
        let sibling_index = u64::try_from(proposal_index - 1).map_err(route_error)?;
        let offset = decisions_per_lane
            .checked_add(
                decision_index
                    .checked_mul(sibling_width)
                    .ok_or_else(|| route_message("counterfactual episode group overflowed"))?,
            )
            .and_then(|value| value.checked_add(sibling_index))
            .ok_or_else(|| route_message("counterfactual episode group overflowed"))?;
        if offset >= EPISODE_GROUP_STRIDE {
            return Err(route_message(
                "counterfactual tactic episode identity exceeds its lane",
            ));
        }
        self.episode_group_base
            .checked_add(offset)
            .ok_or_else(|| route_message("counterfactual episode group overflowed"))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticGenerationPlan {
    pub generation_index: usize,
    pub lane_indices: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NativeTacticReplaySharingPlan {
    GenerationBarrier,
    BoundedStaleness { maximum_stale_replay_revisions: u64 },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticCheckpointOwnership {
    WorkerLocal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticCheckpointFallback {
    AuthenticatedRootReplay,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCheckpointPlan {
    pub ownership: NativeTacticCheckpointOwnership,
    pub fallback: NativeTacticCheckpointFallback,
    pub cross_decision_direct_restore: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum NativeTacticResourceLimit {
    Bounded(u64),
    Unbounded,
}

impl NativeTacticResourceLimit {
    pub(super) fn is_valid(self) -> bool {
        !matches!(self, Self::Bounded(0))
    }

    pub(super) fn reached(self, consumed: u64) -> bool {
        matches!(self, Self::Bounded(limit) if consumed >= limit)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticPlanBudgets {
    pub decisions_per_lane: u64,
    pub native_ticks: NativeTacticResourceLimit,
    pub memory_bytes: NativeTacticResourceLimit,
    pub wall_micros: NativeTacticResourceLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeTacticExecutionPlanRequest {
    pub seeds: Vec<u64>,
    pub proposal_policy: TacticProposalPolicy,
    pub value_treatment: TacticValueTreatment,
    pub execution_strategy: NativeGenericExecutionStrategy,
    pub promoted_tactic_registry_sha256: Option<Digest>,
    pub lanes_per_generation: usize,
    pub proposal_width_per_decision: usize,
    pub branch_every_decisions: u64,
    pub refit_every_decisions: u64,
    pub root_refresh_cadence: u32,
    pub epsilon_per_million: u32,
    pub demonstration_chunk_ticks: Option<u32>,
    pub replay_sharing: NativeTacticReplaySharingPlan,
    pub budgets: NativeTacticPlanBudgets,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticExecutionPlan {
    pub schema: String,
    pub seeds: Vec<u64>,
    pub proposal_policy: TacticProposalPolicy,
    pub value_treatment: TacticValueTreatment,
    pub execution_strategy: NativeGenericExecutionStrategy,
    pub promoted_tactic_registry_sha256: Option<Digest>,
    pub proposal_width_per_decision: usize,
    pub branch_every_decisions: u64,
    pub refit_every_decisions: u64,
    pub root_refresh_cadence: u32,
    pub demonstration_chunk_ticks: Option<u32>,
    pub replay_sharing: NativeTacticReplaySharingPlan,
    pub checkpoint: NativeTacticCheckpointPlan,
    pub budgets: NativeTacticPlanBudgets,
    pub generations: Vec<NativeTacticGenerationPlan>,
    pub lanes: Vec<NativeTacticLanePlan>,
}

impl NativeTacticExecutionPlan {
    pub fn build(
        request: NativeTacticExecutionPlanRequest,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if request.seeds.is_empty()
            || request.seeds.len() > MAX_ROUTE_SEEDS
            || request.seeds.windows(2).any(|pair| pair[0] >= pair[1])
            || request.lanes_per_generation == 0
            || request.lanes_per_generation > request.seeds.len()
            || request.proposal_width_per_decision == 0
            || request.proposal_width_per_decision > MAX_TACTIC_PROPOSALS_PER_DECISION
            || request.branch_every_decisions == 0
            || request.refit_every_decisions == 0
            || request.root_refresh_cadence == 0
            || request.branch_every_decisions > request.budgets.decisions_per_lane
            || request.refit_every_decisions > request.budgets.decisions_per_lane
            || request.budgets.decisions_per_lane == 0
            || !request.budgets.native_ticks.is_valid()
            || !request.budgets.memory_bytes.is_valid()
            || !request.budgets.wall_micros.is_valid()
            || request
                .budgets
                .decisions_per_lane
                .checked_mul(request.proposal_width_per_decision as u64)
                .is_none_or(|groups| groups > EPISODE_GROUP_STRIDE)
            || request.epsilon_per_million > 1_000_000
            || request.promoted_tactic_registry_sha256 == Some(Digest::ZERO)
            || request.demonstration_chunk_ticks == Some(0)
            || (matches!(
                request.replay_sharing,
                NativeTacticReplaySharingPlan::BoundedStaleness { .. }
            ) && request.proposal_policy != TacticProposalPolicy::Learned)
        {
            return Err(route_message("native tactic execution plan is invalid"));
        }
        let intervention = if request.demonstration_chunk_ticks.is_some() {
            NativeTacticInterventionPlan::DemonstrationFrontierOnce
        } else {
            NativeTacticInterventionPlan::None
        };
        let mut lanes = Vec::with_capacity(request.seeds.len());
        let mut generations = Vec::new();
        for (generation_index, seeds) in request
            .seeds
            .chunks(request.lanes_per_generation)
            .enumerate()
        {
            let mut lane_indices = Vec::with_capacity(seeds.len());
            for (generation_lane_index, seed) in seeds.iter().copied().enumerate() {
                let lane_index = lanes.len();
                // Graph-node acquisition is an experimental control, not part
                // of the action-ranking treatment. Learned, structured
                // scheduler-only, and random-valid cells must traverse the
                // same sealed lane schedule so policy is the only search
                // intervention.
                let support = generation_lane_index == 0;
                let acquisition = if request.seeds.len() == 1 {
                    NativeTacticAcquisitionPlan::CyclicSupportAndRanks {
                        cycle_width: request.lanes_per_generation.max(4) as u32,
                        ranked_lanes_per_cycle: request.lanes_per_generation.max(4) as u32 - 1,
                    }
                } else {
                    let rank = if support {
                        0
                    } else {
                        generation_index
                            .saturating_mul(request.lanes_per_generation.saturating_sub(1))
                            .saturating_add(generation_lane_index) as u64
                    };
                    NativeTacticAcquisitionPlan::FixedRank { rank }
                };
                lanes.push(NativeTacticLanePlan {
                    lane_index,
                    generation_index,
                    generation_lane_index,
                    seed,
                    role: if support {
                        NativeTacticLaneRole::TerminalSupport
                    } else {
                        NativeTacticLaneRole::RankedExploration
                    },
                    acquisition,
                    // Discovery is exploratory in every lane. The decision
                    // policy suppresses epsilon for the rank-zero support
                    // acquisition only after authenticated terminal evidence
                    // exists.
                    epsilon_per_million: request.epsilon_per_million,
                    intervention,
                    root_refresh_phase: generation_lane_index as u32 % request.root_refresh_cadence,
                    episode_group_base: (lane_index as u64)
                        .checked_mul(EPISODE_GROUP_STRIDE)
                        .ok_or_else(|| route_message("episode group overflowed"))?,
                });
                lane_indices.push(lane_index);
            }
            generations.push(NativeTacticGenerationPlan {
                generation_index,
                lane_indices,
            });
        }
        let plan = Self {
            schema: NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V4.into(),
            seeds: request.seeds,
            proposal_policy: request.proposal_policy,
            value_treatment: request.value_treatment,
            execution_strategy: request.execution_strategy,
            promoted_tactic_registry_sha256: request.promoted_tactic_registry_sha256,
            proposal_width_per_decision: request.proposal_width_per_decision,
            branch_every_decisions: request.branch_every_decisions,
            refit_every_decisions: request.refit_every_decisions,
            root_refresh_cadence: request.root_refresh_cadence,
            demonstration_chunk_ticks: request.demonstration_chunk_ticks,
            replay_sharing: request.replay_sharing,
            checkpoint: NativeTacticCheckpointPlan {
                ownership: NativeTacticCheckpointOwnership::WorkerLocal,
                fallback: NativeTacticCheckpointFallback::AuthenticatedRootReplay,
                cross_decision_direct_restore: true,
            },
            budgets: request.budgets,
            generations,
            lanes,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn identity(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let bytes = serde_cbor::to_vec(self).map_err(route_error)?;
        Ok(Digest(Sha256::digest(bytes).into()))
    }

    pub fn write(&self, path: &Path) -> Result<Digest, NativeTacticRouteRunError> {
        self.validate()?;
        let payload = serde_cbor::to_vec(self).map_err(route_error)?;
        let payload_len = u64::try_from(payload.len()).map_err(route_error)?;
        let identity = self.identity()?;
        let mut envelope = Vec::with_capacity(PLAN_HEADER_BYTES + payload.len());
        envelope.extend_from_slice(PLAN_MAGIC);
        envelope.extend_from_slice(&PLAN_VERSION.to_le_bytes());
        envelope.extend_from_slice(&payload_len.to_le_bytes());
        envelope.extend_from_slice(&identity.0);
        envelope.extend_from_slice(&payload);
        write_new(path, &envelope)?;
        Ok(identity)
    }

    pub fn read(path: &Path) -> Result<Self, NativeTacticRouteRunError> {
        let bytes = fs::read(path).map_err(route_error)?;
        if bytes.len() < PLAN_HEADER_BYTES || bytes.len() > MAXIMUM_PLAN_BYTES {
            return Err(route_message(
                "native tactic execution plan envelope is invalid",
            ));
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().map_err(route_error)?);
        let payload_len =
            u64::from_le_bytes(bytes[10..18].try_into().map_err(route_error)?) as usize;
        let expected = Digest(bytes[18..50].try_into().map_err(route_error)?);
        if &bytes[..8] != PLAN_MAGIC
            || version != PLAN_VERSION
            || payload_len != bytes.len() - PLAN_HEADER_BYTES
        {
            return Err(route_message(
                "native tactic execution plan envelope is invalid",
            ));
        }
        let plan: Self =
            serde_cbor::from_slice(&bytes[PLAN_HEADER_BYTES..]).map_err(route_error)?;
        plan.validate()?;
        if plan.identity()? != expected {
            return Err(route_message(
                "native tactic execution plan identity is invalid",
            ));
        }
        Ok(plan)
    }

    pub(super) fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V4
            || self.promoted_tactic_registry_sha256 == Some(Digest::ZERO)
            || self.seeds.len() != self.lanes.len()
            || self.generations.is_empty()
            || !self.budgets.native_ticks.is_valid()
            || !self.budgets.memory_bytes.is_valid()
            || !self.budgets.wall_micros.is_valid()
            || self
                .budgets
                .decisions_per_lane
                .checked_mul(self.proposal_width_per_decision as u64)
                .is_none_or(|groups| groups > EPISODE_GROUP_STRIDE)
            || self
                .lanes
                .iter()
                .enumerate()
                .any(|(index, lane)| lane.lane_index != index || lane.seed != self.seeds[index])
            || self
                .generations
                .iter()
                .enumerate()
                .any(|(index, generation)| {
                    generation.generation_index != index
                        || generation.lane_indices.is_empty()
                        || generation.lane_indices.iter().any(|lane_index| {
                            self.lanes.get(*lane_index).is_none_or(|lane| {
                                lane.generation_index != generation.generation_index
                            })
                        })
                })
        {
            return Err(route_message("native tactic execution plan is detached"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> NativeTacticExecutionPlanRequest {
        NativeTacticExecutionPlanRequest {
            seeds: vec![11, 22, 33, 44],
            proposal_policy: TacticProposalPolicy::Learned,
            value_treatment: TacticValueTreatment::ContinuousFittedQForestV1,
            execution_strategy: NativeGenericExecutionStrategy::NativeController,
            promoted_tactic_registry_sha256: None,
            lanes_per_generation: 4,
            proposal_width_per_decision: 4,
            branch_every_decisions: 8,
            refit_every_decisions: 4,
            root_refresh_cadence: 4,
            epsilon_per_million: 350_000,
            demonstration_chunk_ticks: Some(4),
            replay_sharing: NativeTacticReplaySharingPlan::GenerationBarrier,
            budgets: NativeTacticPlanBudgets {
                decisions_per_lane: 256,
                native_ticks: NativeTacticResourceLimit::Bounded(100_000),
                memory_bytes: NativeTacticResourceLimit::Unbounded,
                wall_micros: NativeTacticResourceLimit::Unbounded,
            },
        }
    }

    #[test]
    fn equal_requests_seal_equal_jobs_without_lane_arithmetic_at_runtime() {
        let first = NativeTacticExecutionPlan::build(request()).unwrap();
        let second = NativeTacticExecutionPlan::build(request()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.identity().unwrap(), second.identity().unwrap());
        assert_eq!(first.generations.len(), 1);
        assert_eq!(
            first
                .lanes
                .iter()
                .map(|lane| (
                    lane.role,
                    lane.acquisition.rank(999),
                    lane.epsilon_per_million,
                    lane.root_refresh_phase,
                ))
                .collect::<Vec<_>>(),
            vec![
                (NativeTacticLaneRole::TerminalSupport, 0, 350_000, 0),
                (NativeTacticLaneRole::RankedExploration, 1, 350_000, 1,),
                (NativeTacticLaneRole::RankedExploration, 2, 350_000, 2,),
                (NativeTacticLaneRole::RankedExploration, 3, 350_000, 3,),
            ]
        );
    }

    #[test]
    fn ranking_treatments_share_one_graph_acquisition_schedule() {
        let plans = [
            TacticProposalPolicy::Learned,
            TacticProposalPolicy::StructuredNonLearning,
            TacticProposalPolicy::RandomValid,
        ]
        .map(|proposal_policy| {
            let mut request = request();
            request.proposal_policy = proposal_policy;
            NativeTacticExecutionPlan::build(request).unwrap()
        });
        for plan in &plans[1..] {
            assert_eq!(plan.generations, plans[0].generations);
            assert_eq!(plan.lanes, plans[0].lanes);
            assert_eq!(plan.checkpoint, plans[0].checkpoint);
            assert_eq!(plan.budgets, plans[0].budgets);
        }

        let single_seed = [
            TacticProposalPolicy::Learned,
            TacticProposalPolicy::StructuredNonLearning,
            TacticProposalPolicy::RandomValid,
        ]
        .map(|proposal_policy| {
            let mut request = request();
            request.seeds = vec![11];
            request.lanes_per_generation = 1;
            request.proposal_policy = proposal_policy;
            NativeTacticExecutionPlan::build(request).unwrap()
        });
        assert!(single_seed.iter().all(|plan| {
            plan.lanes[0].acquisition
                == NativeTacticAcquisitionPlan::CyclicSupportAndRanks {
                    cycle_width: 4,
                    ranked_lanes_per_cycle: 3,
                }
        }));
    }

    #[test]
    fn bounded_staleness_is_an_explicit_validated_execution_mode() {
        let mut asynchronous = request();
        asynchronous.replay_sharing = NativeTacticReplaySharingPlan::BoundedStaleness {
            maximum_stale_replay_revisions: 4,
        };
        let plan = NativeTacticExecutionPlan::build(asynchronous).unwrap();
        assert_eq!(
            plan.replay_sharing,
            NativeTacticReplaySharingPlan::BoundedStaleness {
                maximum_stale_replay_revisions: 4,
            }
        );

        let mut invalid = request();
        invalid.proposal_policy = TacticProposalPolicy::RandomValid;
        invalid.replay_sharing = NativeTacticReplaySharingPlan::BoundedStaleness {
            maximum_stale_replay_revisions: 0,
        };
        assert!(NativeTacticExecutionPlan::build(invalid).is_err());

        let mut freshest = request();
        freshest.replay_sharing = NativeTacticReplaySharingPlan::BoundedStaleness {
            maximum_stale_replay_revisions: 0,
        };
        assert!(NativeTacticExecutionPlan::build(freshest).is_ok());
    }

    #[test]
    fn multi_seed_plans_route_cross_decision_restores_to_checkpoint_owners() {
        let plan = NativeTacticExecutionPlan::build(request()).unwrap();
        assert_eq!(plan.lanes.len(), 4);
        assert!(plan.checkpoint.cross_decision_direct_restore);
        assert_eq!(
            plan.checkpoint.ownership,
            NativeTacticCheckpointOwnership::WorkerLocal
        );
        assert_eq!(
            plan.checkpoint.fallback,
            NativeTacticCheckpointFallback::AuthenticatedRootReplay
        );
    }

    #[test]
    fn counterfactual_siblings_receive_distinct_censored_episode_lineages() {
        let plan = NativeTacticExecutionPlan::build(request()).unwrap();
        let lane = &plan.lanes[0];
        let retained = lane.episode_group(3).unwrap();
        let first = lane
            .counterfactual_episode_group(
                7,
                1,
                plan.budgets.decisions_per_lane,
                plan.proposal_width_per_decision,
            )
            .unwrap();
        let second = lane
            .counterfactual_episode_group(
                7,
                2,
                plan.budgets.decisions_per_lane,
                plan.proposal_width_per_decision,
            )
            .unwrap();
        assert_ne!(retained, first);
        assert_ne!(first, second);
        assert!(lane.owns_episode_group(first));
        assert!(lane.owns_episode_group(second));

        let mut oversized = request();
        oversized.budgets.decisions_per_lane = EPISODE_GROUP_STRIDE / 4 + 1;
        assert!(NativeTacticExecutionPlan::build(oversized).is_err());
    }

    #[test]
    fn zero_is_not_a_bounded_resource_budget() {
        for mutate in [
            |budgets: &mut NativeTacticPlanBudgets| {
                budgets.native_ticks = NativeTacticResourceLimit::Bounded(0);
            },
            |budgets: &mut NativeTacticPlanBudgets| {
                budgets.memory_bytes = NativeTacticResourceLimit::Bounded(0);
            },
            |budgets: &mut NativeTacticPlanBudgets| {
                budgets.wall_micros = NativeTacticResourceLimit::Bounded(0);
            },
        ] {
            let mut invalid = request();
            mutate(&mut invalid.budgets);
            assert!(NativeTacticExecutionPlan::build(invalid).is_err());
        }
        assert!(!NativeTacticResourceLimit::Unbounded.reached(u64::MAX));
        assert!(!NativeTacticResourceLimit::Bounded(10).reached(9));
        assert!(NativeTacticResourceLimit::Bounded(10).reached(10));
    }

    #[test]
    fn every_sealed_behavior_field_changes_plan_identity() {
        let plan = NativeTacticExecutionPlan::build(request()).unwrap();
        let identity = plan.identity().unwrap();
        let mut variants = Vec::new();

        let mut changed = plan.clone();
        changed.proposal_policy = TacticProposalPolicy::RandomValid;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.value_treatment = TacticValueTreatment::LocalGeneralizedFittedQKnnV1;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.execution_strategy = NativeGenericExecutionStrategy::ProgressiveAudit;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.promoted_tactic_registry_sha256 = Some(Digest([9; 32]));
        variants.push(changed);
        let mut changed = plan.clone();
        changed.proposal_width_per_decision += 1;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.branch_every_decisions += 1;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.refit_every_decisions += 1;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.root_refresh_cadence += 1;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.demonstration_chunk_ticks = None;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.replay_sharing = NativeTacticReplaySharingPlan::BoundedStaleness {
            maximum_stale_replay_revisions: 1,
        };
        variants.push(changed);
        let mut changed = plan.clone();
        changed.checkpoint.cross_decision_direct_restore = false;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.budgets.decisions_per_lane += 1;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.budgets.native_ticks = NativeTacticResourceLimit::Bounded(99_999);
        variants.push(changed);
        let mut changed = plan.clone();
        changed.budgets.memory_bytes = NativeTacticResourceLimit::Bounded(1);
        variants.push(changed);
        let mut changed = plan.clone();
        changed.budgets.wall_micros = NativeTacticResourceLimit::Bounded(1);
        variants.push(changed);
        let mut changed = plan.clone();
        changed.lanes[1].epsilon_per_million -= 1;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.lanes[1].acquisition = NativeTacticAcquisitionPlan::FixedRank { rank: 9 };
        variants.push(changed);
        let mut changed = plan.clone();
        changed.generations[0].lane_indices.swap(1, 2);
        variants.push(changed);

        assert!(
            variants
                .iter()
                .all(|variant| variant.identity().unwrap() != identity)
        );
    }

    #[test]
    fn binary_plan_round_trips_and_rejects_tampering() {
        let plan = NativeTacticExecutionPlan::build(request()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "dusklight-native-tactic-plan-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let path = root.join(NATIVE_TACTIC_EXECUTION_PLAN_FILE);
        let identity = plan.write(&path).unwrap();
        assert_eq!(NativeTacticExecutionPlan::read(&path).unwrap(), plan);
        assert_eq!(identity, plan.identity().unwrap());
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(NativeTacticExecutionPlan::read(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
