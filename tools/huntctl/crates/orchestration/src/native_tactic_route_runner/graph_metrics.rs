use super::*;

const CAMPAIGN_EXECUTABLE_EXPANSION_SET_SCHEMA_V1: &[u8] =
    b"dusklight-campaign-executable-expansion-set/v1";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct CampaignUsefulGraphExpansionSet {
    identities: BTreeSet<Digest>,
}

impl CampaignUsefulGraphExpansionSet {
    pub(super) fn include_graph(&mut self, graph: &crate::state_graph::StateGraph) {
        self.identities
            .extend(graph.completed_executable_expansion_identities());
    }

    #[cfg(test)]
    fn include_identities(&mut self, identities: impl IntoIterator<Item = Digest>) {
        self.identities.extend(identities);
    }

    pub(super) fn count(&self) -> Result<u64, NativeTacticRouteRunError> {
        u64::try_from(self.identities.len()).map_err(route_error)
    }

    pub(super) fn content_sha256(&self) -> Digest {
        let mut hasher = Sha256::new();
        hasher.update(CAMPAIGN_EXECUTABLE_EXPANSION_SET_SCHEMA_V1);
        hasher.update((self.identities.len() as u64).to_le_bytes());
        for identity in &self.identities {
            hasher.update(identity.0);
        }
        Digest(hasher.finalize().into())
    }
}

pub(super) fn campaign_useful_graph_expansion_set(
    repository_root: &Path,
    seeds: &[NativeTacticSeedResult],
) -> Result<CampaignUsefulGraphExpansionSet, NativeTacticRouteRunError> {
    let repository_root = repository_root.canonicalize().map_err(route_error)?;
    let mut campaign = CampaignUsefulGraphExpansionSet::default();
    for seed in seeds {
        let declared = Path::new(&seed.final_checkpoint);
        let candidate = if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            repository_root.join(declared)
        };
        let checkpoint_path = candidate.canonicalize().map_err(route_error)?;
        if !checkpoint_path.starts_with(&repository_root) || !checkpoint_path.is_file() {
            return Err(route_message(
                "native tactic final checkpoint is outside the repository or absent",
            ));
        }
        let checkpoint =
            TacticQCampaign::read_checkpoint_payload(&checkpoint_path).map_err(route_error)?;
        let graph = &checkpoint.state_graph;
        validate_seed_useful_graph_accounting(seed, graph)?;
        campaign.include_graph(graph);
    }
    Ok(campaign)
}

pub(super) fn validate_seed_useful_graph_accounting(
    seed: &NativeTacticSeedResult,
    graph: &crate::state_graph::StateGraph,
) -> Result<(), NativeTacticRouteRunError> {
    let graph_sha256 = graph.content_sha256().map_err(route_error)?;
    let useful_count =
        u64::try_from(graph.completed_executable_expansion_count()).map_err(route_error)?;
    let useful_set_sha256 = graph.completed_executable_expansion_set_sha256();
    if graph_sha256 != seed.state_graph_sha256
        || useful_count != seed.unique_useful_graph_expansions
        || useful_set_sha256 == Digest::ZERO
        || useful_set_sha256 != seed.useful_graph_expansion_set_sha256
    {
        return Err(route_message(
            "native tactic seed useful graph accounting is detached from its checkpoint",
        ));
    }
    Ok(())
}

impl NativeTacticRouteReport {
    /// Recompute campaign-wide derived graph accounting from immutable final
    /// checkpoints. This repairs projections without changing seed evidence.
    pub fn reproject_useful_graph_accounting(
        &mut self,
        repository_root: &Path,
    ) -> Result<(), NativeTacticRouteRunError> {
        let campaign = campaign_useful_graph_expansion_set(repository_root, &self.seeds)?;
        self.unique_useful_graph_expansions = campaign.count()?;
        refresh_route_throughput(
            &mut self.timing,
            &self.seeds,
            self.unique_useful_graph_expansions,
        );
        Ok(())
    }
}

pub(super) fn tactic_graph_metrics(
    graph: &crate::state_graph::StateGraph,
    graph_sha256: Digest,
    trace: &[NativeTacticDecisionTrace],
    lease_accounting: NativeTacticLeaseAccounting,
) -> Result<NativeTacticGraphMetrics, NativeTacticRouteRunError> {
    let graph_report =
        GraphSearchReport::from_validated_graph(graph, graph_sha256).map_err(route_error)?;
    let completed_trace_dispatches = trace.iter().try_fold(0_u64, |total, decision| {
        total
            .checked_add(u64::try_from(decision.proposal_batch.len()).map_err(route_error)?)
            .ok_or_else(|| route_message("completed tactic lease count overflowed"))
    })?;
    lease_accounting.validate()?;
    if lease_accounting.completed_leases != completed_trace_dispatches
        || lease_accounting.unresolved_leases != 0
    {
        return Err(route_message(
            "native tactic lease accounting is detached from durable completed decisions",
        ));
    }
    let duplicate_transpositions = graph_report
        .observed_segments
        .saturating_add(1)
        .saturating_sub(graph_report.nodes);
    let terminal_paths =
        u64::try_from(graph.nodes().filter(|node| node.terminal).count()).map_err(route_error)?;
    if terminal_paths == 0 && graph_report.best_terminal.is_some()
        || terminal_paths > 0 && graph_report.best_terminal.is_none()
    {
        return Err(route_message(
            "native tactic graph metrics are detached from terminal paths",
        ));
    }
    Ok(NativeTacticGraphMetrics {
        graph: graph_report,
        lease_accounting,
        duplicate_transpositions,
        terminal_paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: u8) -> Digest {
        Digest([byte; 32])
    }

    #[test]
    fn campaign_set_counts_shared_expansions_once() {
        let mut campaign = CampaignUsefulGraphExpansionSet::default();
        campaign.include_identities([digest(1), digest(2), digest(3)]);
        campaign.include_identities([digest(2), digest(3), digest(4)]);

        assert_eq!(campaign.count().unwrap(), 4);
        assert_ne!(campaign.content_sha256(), Digest::ZERO);
    }

    #[test]
    fn campaign_set_keeps_disjoint_graph_work_additive() {
        let mut campaign = CampaignUsefulGraphExpansionSet::default();
        campaign.include_identities([digest(1), digest(2)]);
        campaign.include_identities([digest(3), digest(4)]);

        assert_eq!(campaign.count().unwrap(), 4);
    }

    #[test]
    fn campaign_set_identity_is_order_independent() {
        let mut first = CampaignUsefulGraphExpansionSet::default();
        first.include_identities([digest(1), digest(2), digest(3)]);
        let mut second = CampaignUsefulGraphExpansionSet::default();
        second.include_identities([digest(3), digest(1), digest(2)]);

        assert_eq!(first.content_sha256(), second.content_sha256());
    }
}
