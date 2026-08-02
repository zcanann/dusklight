//! Creation and recovery of the mutable campaign state for one seed lane.

use super::*;

pub(super) struct SeedCampaignInitialization {
    pub campaign: TacticQCampaign,
    pub trace: Vec<NativeTacticDecisionTrace>,
    pub selection_counts: BTreeMap<String, u64>,
    pub native_ticks: u64,
    pub episode: u64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_or_create_seed_campaign(
    config: &NativeTacticRouteRunConfig<'_>,
    lane: &NativeTacticLanePlan,
    seed_root: &Path,
    encoder: &GoalConditionedTacticFeatureEncoder,
    root_checkpoint_sha256: Digest,
    seed_index: usize,
    seed: u64,
    inherited_learner_snapshot: &Arc<TacticQImmutableLearnerSnapshot>,
    initial_facts: &FactSnapshot,
    registry: &FactRegistry,
    maximum_tactic_ticks: u32,
    action_schema_sha256: Digest,
    promoted_tactics: &[ImportedPromotedTactic],
    route_prefix: &InputTape,
    execution_plan_sha256: Digest,
    resuming_seed: bool,
    seed_root_preexisted: bool,
) -> Result<SeedCampaignInitialization, NativeTacticRouteRunError> {
    let (
        campaign,
        trace,
        selection_counts,
        native_ticks,
        episode,
        _initial_imported_training_replay_rows,
    ) = if resuming_seed {
        let resumed = resume_seed(
            config,
            &seed_root,
            encoder.schema_sha256,
            root_checkpoint_sha256,
            seed_index,
            seed,
        )?;
        let mut resumed_campaign = resumed.0;
        let imported = if config
            .execution_plan
            .proposal_policy
            .uses_learned_selector()
        {
            resumed_campaign
                .consume_learner_snapshot(&inherited_learner_snapshot)
                .map_err(route_error)?;
            usize::try_from(inherited_learner_snapshot.manifest.training_replay_rows)
                .map_err(route_error)?
        } else {
            0
        };
        (
            resumed_campaign,
            resumed.1,
            resumed.2,
            resumed.3,
            resumed.4,
            imported,
        )
    } else {
        if seed_root_preexisted && !config.resume {
            return Err(route_message(
                "unexpected pre-existing tactic seed evidence",
            ));
        }
        fs::create_dir_all(&seed_root).map_err(route_error)?;
        let initial_proposals = parameterized_catalog_for_state_with_promoted(
            seed,
            0,
            initial_facts,
            encoder,
            maximum_tactic_ticks,
            None,
            action_schema_sha256,
            promoted_tactics,
        )?;
        let current = LearnerState::build(
            initial_facts.clone(),
            registry,
            &initial_proposals.catalog,
            &initial_proposals.blueprints,
            |_| true,
        )
        .map_err(route_error)?;
        let mut campaign = TacticQCampaign::new(
            encoder.schema_sha256,
            config.optimization.terminal_predicate.definition_sha256,
            root_checkpoint_sha256,
            lane.episode_group(0)?,
            current,
            route_prefix.clone(),
            route_option_value_config(execution_plan_sha256),
            TacticExplorationConfig {
                seed,
                epsilon_per_million: lane.epsilon_per_million,
            },
        )
        .map_err(route_error)?;
        campaign
            .bind_execution_authority(execution_plan_sha256)
            .map_err(route_error)?;
        let imported = if config
            .execution_plan
            .proposal_policy
            .uses_learned_selector()
        {
            campaign
                .consume_learner_snapshot(&inherited_learner_snapshot)
                .map_err(route_error)?;
            usize::try_from(inherited_learner_snapshot.manifest.training_replay_rows)
                .map_err(route_error)?
        } else {
            0
        };
        (campaign, Vec::new(), BTreeMap::new(), 0, 0, imported)
    };
    Ok(SeedCampaignInitialization {
        campaign,
        trace,
        selection_counts,
        native_ticks,
        episode,
    })
}
