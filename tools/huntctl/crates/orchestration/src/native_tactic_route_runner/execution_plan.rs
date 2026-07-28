use super::*;

pub const NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V1: &str =
    "dusklight-native-tactic-execution-plan/v1";
pub const NATIVE_TACTIC_EXECUTION_PLAN_FILE: &str = "execution-plan.dtp";
const PLAN_MAGIC: &[u8; 8] = b"DSKTPN01";
const PLAN_VERSION: u16 = 1;
const PLAN_HEADER_BYTES: usize = 8 + 2 + 8 + 32;
const MAXIMUM_PLAN_BYTES: usize = 4 * 1024 * 1024;
const EPISODE_GROUP_STRIDE: u64 = 1_000_000;

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
    pub execution_strategy: NativeGenericExecutionStrategy,
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
    pub execution_strategy: NativeGenericExecutionStrategy,
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
            || request.budgets.decisions_per_lane >= EPISODE_GROUP_STRIDE
            || request.epsilon_per_million > 1_000_000
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
                let support = request.proposal_policy == TacticProposalPolicy::Learned
                    && generation_lane_index == 0;
                let acquisition = if request.proposal_policy == TacticProposalPolicy::Learned
                    && request.seeds.len() == 1
                {
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
                    epsilon_per_million: if support {
                        0
                    } else {
                        request.epsilon_per_million
                    },
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
        let single_lane = request.seeds.len() == 1;
        let plan = Self {
            schema: NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V1.into(),
            seeds: request.seeds,
            proposal_policy: request.proposal_policy,
            execution_strategy: request.execution_strategy,
            proposal_width_per_decision: request.proposal_width_per_decision,
            branch_every_decisions: request.branch_every_decisions,
            refit_every_decisions: request.refit_every_decisions,
            root_refresh_cadence: request.root_refresh_cadence,
            demonstration_chunk_ticks: request.demonstration_chunk_ticks,
            replay_sharing: request.replay_sharing,
            checkpoint: NativeTacticCheckpointPlan {
                ownership: NativeTacticCheckpointOwnership::WorkerLocal,
                fallback: NativeTacticCheckpointFallback::AuthenticatedRootReplay,
                cross_decision_direct_restore: single_lane,
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
        if self.schema != NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V1
            || self.seeds.len() != self.lanes.len()
            || self.generations.is_empty()
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
            execution_strategy: NativeGenericExecutionStrategy::NativeController,
            lanes_per_generation: 4,
            proposal_width_per_decision: 4,
            branch_every_decisions: 8,
            refit_every_decisions: 4,
            root_refresh_cadence: 4,
            epsilon_per_million: 350_000,
            demonstration_chunk_ticks: Some(8),
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
                (NativeTacticLaneRole::TerminalSupport, 0, 0, 0),
                (NativeTacticLaneRole::RankedExploration, 1, 350_000, 1,),
                (NativeTacticLaneRole::RankedExploration, 2, 350_000, 2,),
                (NativeTacticLaneRole::RankedExploration, 3, 350_000, 3,),
            ]
        );
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
    fn every_sealed_behavior_field_changes_plan_identity() {
        let plan = NativeTacticExecutionPlan::build(request()).unwrap();
        let identity = plan.identity().unwrap();
        let mut variants = Vec::new();

        let mut changed = plan.clone();
        changed.proposal_policy = TacticProposalPolicy::RandomValid;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.execution_strategy = NativeGenericExecutionStrategy::ProgressiveAudit;
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
        changed.checkpoint.cross_decision_direct_restore = true;
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
