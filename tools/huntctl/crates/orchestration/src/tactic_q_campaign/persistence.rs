use super::*;

impl TacticQCampaign {
    pub fn checkpoint(&self) -> Result<TacticQCampaignCheckpoint, TacticQCampaignError> {
        let mut checkpoint = TacticQCampaignCheckpoint {
            schema: TACTIC_Q_CHECKPOINT_SCHEMA_V4.into(),
            content_sha256: Digest::ZERO,
            execution_authority_sha256: self.execution_authority_sha256,
            feature_schema_sha256: self.feature_schema_sha256,
            objective_sha256: self.objective_sha256,
            root_checkpoint_sha256: self.root_checkpoint_sha256,
            episode_group: self.episode_group,
            decision_index: self.decision_index,
            current: self.current.clone(),
            route_tape: self.route_tape.clone(),
            replay: self.replay.clone(),
            replay_routes: self.replay_routes.clone(),
            episode_groups: self.episode_groups.clone(),
            training_replay: self.training_replay.clone(),
            training_replay_routes: self.training_replay_routes.clone(),
            training_episode_groups: self.training_episode_groups.clone(),
            model_revision: self.model_revision,
            model_config: self.model_config.clone(),
            exploration: self.exploration,
        };
        checkpoint.content_sha256 = checkpoint_digest(&checkpoint)?;
        validate_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    /// Seal the fitted critic as an independently reloadable greedy policy.
    ///
    /// The executable action schema is derived from the checkpoint's complete
    /// action mask. This is intentionally not supplied by the caller: campaigns
    /// may extend the default catalog with goal-conditioned tactics, and
    /// freezing against a separately reconstructed default catalog would stamp
    /// the policy with the wrong executable universe.
    pub fn freeze_greedy_policy(&self) -> Result<TacticFrozenPolicy, TacticQCampaignError> {
        let checkpoint = self.checkpoint()?;
        let first = self
            .replay
            .first()
            .ok_or(TacticQCampaignError::InvalidState(
                "freezing a tactic policy requires replay",
            ))?;
        let training_batch = OptionValueBatch::new(
            self.feature_schema_sha256,
            self.objective_sha256,
            first.value_sample.state.len(),
            self.training_replay
                .iter()
                .map(|transition| transition.value_sample.clone())
                .collect(),
            self.training_episode_groups.clone(),
        )?;
        let action_universe_sha256 = Digest(
            Sha256::digest(
                serde_json::to_vec(
                    &self
                        .current
                        .action_mask
                        .iter()
                        .map(|entry| &entry.descriptor)
                        .collect::<Vec<_>>(),
                )
                .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?,
            )
            .into(),
        );
        TacticFrozenPolicy::freeze(
            self.execution_authority_sha256,
            checkpoint.content_sha256,
            self.root_checkpoint_sha256,
            first.before_state_sha256,
            self.feature_schema_sha256,
            action_universe_sha256,
            self.objective_sha256,
            training_batch,
            checkpoint.model_config,
        )
        .map_err(TacticQCampaignError::FrozenPolicy)
    }

    /// Writes one immutable, content-addressed checkpoint. A completed file is
    /// always resumable; a crash can leave only an unreferenced `.partial`
    /// file, never a half-written checkpoint at the final path.
    pub fn write_checkpoint(&self, directory: &Path) -> Result<PathBuf, TacticQCampaignError> {
        let checkpoint = self.checkpoint()?;
        tactic_q_checkpoint_store::write_checkpoint_with_local_store(&checkpoint, directory)
    }

    pub fn write_checkpoint_with_store(
        &self,
        directory: &Path,
        content_root: &Path,
    ) -> Result<PathBuf, TacticQCampaignError> {
        let checkpoint = self.checkpoint()?;
        tactic_q_checkpoint_store::write_checkpoint(&checkpoint, directory, content_root)
    }

    pub fn read_checkpoint(path: &Path) -> Result<Self, TacticQCampaignError> {
        Self::resume(Self::read_checkpoint_payload(path)?)
    }

    /// Reads and validates the durable checkpoint payload without rebuilding
    /// the fitted model. Orchestration uses this to authenticate run-specific
    /// identities before calling `resume`.
    pub fn read_checkpoint_payload(
        path: &Path,
    ) -> Result<TacticQCampaignCheckpoint, TacticQCampaignError> {
        tactic_q_checkpoint_store::read_checkpoint(path)
    }

    /// Measures only deterministic checkpoint-root serialization. Native
    /// simulation, object-store writes, filesystem sync, and report projection
    /// are deliberately outside this codec boundary.
    pub fn benchmark_checkpoint_serialization(
        legacy_json_path: &Path,
        current_checkpoint_path: &Path,
        iterations: u64,
    ) -> Result<TacticQCheckpointSerializationBenchmark, TacticQCampaignError> {
        tactic_q_checkpoint_store::benchmark_checkpoint_serialization(
            legacy_json_path,
            current_checkpoint_path,
            iterations,
        )
    }

    pub fn resume(checkpoint: TacticQCampaignCheckpoint) -> Result<Self, TacticQCampaignError> {
        Self::resume_with_model(checkpoint, true)
    }

    fn resume_with_model(
        mut checkpoint: TacticQCampaignCheckpoint,
        fit_model: bool,
    ) -> Result<Self, TacticQCampaignError> {
        validate_checkpoint(&checkpoint)?;
        if checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V2 {
            checkpoint.training_replay = checkpoint.replay.clone();
            checkpoint.training_replay_routes = checkpoint.replay_routes.clone();
            checkpoint.training_episode_groups = checkpoint.episode_groups.clone();
        }
        let model = if fit_model {
            replay_model(
                checkpoint.feature_schema_sha256,
                checkpoint.objective_sha256,
                &checkpoint.training_replay,
                &checkpoint.training_episode_groups,
                &checkpoint.model_config,
            )?
            .map(Arc::new)
        } else {
            None
        };
        let mut visited_states = BTreeSet::from([tactic_state_descriptor(
            &checkpoint.current.snapshot,
            checkpoint.current.snapshot.terminal.reached == Some(true),
        )]);
        for transition in &checkpoint.training_replay {
            visited_states.insert(tactic_state_descriptor(
                &transition.before,
                transition.before.terminal.reached == Some(true),
            ));
            visited_states.insert(tactic_state_descriptor(
                &transition.after,
                transition.value_sample.terminal,
            ));
        }
        let hindsight = HindsightOptionReplay::new(checkpoint.feature_schema_sha256)
            .map_err(TacticQCampaignError::Hindsight)?;
        let training_identities = checkpoint
            .training_replay
            .iter()
            .map(OptionTransitionSample::replay_identity_sha256)
            .collect::<Result<BTreeSet<_>, _>>()?;
        let frontier_archive = build_frontier_archive(
            checkpoint.root_checkpoint_sha256,
            &checkpoint.training_replay,
            &checkpoint.training_replay_routes,
            &checkpoint.training_episode_groups,
        )?;
        Ok(Self {
            schema: TACTIC_Q_CAMPAIGN_SCHEMA_V1.into(),
            execution_authority_sha256: checkpoint.execution_authority_sha256,
            feature_schema_sha256: checkpoint.feature_schema_sha256,
            objective_sha256: checkpoint.objective_sha256,
            root_checkpoint_sha256: checkpoint.root_checkpoint_sha256,
            episode_group: checkpoint.episode_group,
            decision_index: checkpoint.decision_index,
            current: checkpoint.current,
            route_tape: checkpoint.route_tape,
            replay: checkpoint.replay,
            replay_routes: checkpoint.replay_routes,
            episode_groups: checkpoint.episode_groups,
            training_replay: checkpoint.training_replay,
            training_replay_routes: checkpoint.training_replay_routes,
            training_episode_groups: checkpoint.training_episode_groups,
            training_identities,
            frontier_archive,
            model_config: checkpoint.model_config,
            exploration: checkpoint.exploration,
            model,
            model_revision: checkpoint.model_revision,
            campaign_learner_authority_managed: false,
            value_treatment: TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
            generalized_model: RefCell::new(None),
            continuous_model: RefCell::new(None),
            visited_states,
            hindsight,
        })
    }

    /// Resume authenticated lane state without rebuilding a lane-local model.
    /// Native route orchestration immediately installs the campaign authority's
    /// declared immutable snapshot.
    pub fn resume_without_model(
        checkpoint: TacticQCampaignCheckpoint,
    ) -> Result<Self, TacticQCampaignError> {
        Self::resume_with_model(checkpoint, false)
    }

    pub fn final_result(&self) -> Result<TacticQFinalResult, TacticQCampaignError> {
        if self.current.snapshot.terminal.configured != Some(true)
            || self.current.snapshot.terminal.reached != Some(true)
            || self.replay.last().map(|row| row.after_state_sha256)
                != Some(self.current.snapshot_sha256)
        {
            return Err(TacticQCampaignError::InvalidState(
                "final result requires a native-authorized terminal replay boundary",
            ));
        }
        self.build_final_result(
            self.route_tape.clone(),
            self.replay.clone(),
            self.replay_routes.clone(),
            self.current.snapshot.clone(),
        )
    }

    /// Seal an authenticated terminal sibling evaluated at the current
    /// frontier without changing the policy-selected campaign trajectory.
    ///
    /// Native proposal batches are both learning evidence and a real bounded
    /// candidate search. A terminal sibling is therefore eligible route
    /// evidence even though it must not retroactively replace the learner's
    /// selected action.
    pub fn final_result_from_evaluated_terminal(
        &self,
        evaluated: &EvaluatedRewardedTacticOutcome,
    ) -> Result<TacticQFinalResult, TacticQCampaignError> {
        let outcome = &evaluated.outcome;
        evaluated.transition.validate()?;
        let source_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let terminal_state_sha256 = outcome
            .next_facts
            .content_sha256()
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        if !outcome.terminal
            || outcome.next_facts.terminal.configured != Some(true)
            || outcome.next_facts.terminal.reached != Some(true)
            || outcome.selected.decision_index != self.decision_index
            || outcome.selected.learner_snapshot_sha256 != self.current.snapshot_sha256
            || outcome.source_checkpoint_sha256 != self.root_checkpoint_sha256
            || !extends(&self.route_tape, &outcome.route_tape)
            || evaluated.transition.before_state_sha256 != self.current.snapshot_sha256
            || evaluated.transition.after_state_sha256 != terminal_state_sha256
            || evaluated.transition.source_checkpoint_sha256 != source_checkpoint_sha256
            || evaluated.transition.next_checkpoint_sha256
                != route_checkpoint(self.root_checkpoint_sha256, &outcome.route_tape)?
            || !evaluated.transition.value_sample.terminal
            || evaluated.transition.value_sample.action != outcome.selected.descriptor
            || evaluated.transition.execution != outcome.execution
            || evaluated.transition.value_sample.reward.to_bits()
                != evaluated.reward.training_reward.to_bits()
        {
            return Err(TacticQCampaignError::InvalidState(
                "evaluated terminal tactic is detached from the current campaign frontier",
            ));
        }
        let mut replay = self.replay.clone();
        replay.push(evaluated.transition.clone());
        let mut replay_routes = self.replay_routes.clone();
        replay_routes.push(outcome.route_tape.clone());
        self.build_final_result(
            outcome.route_tape.clone(),
            replay,
            replay_routes,
            outcome.next_facts.clone(),
        )
    }

    fn build_final_result(
        &self,
        route_tape: InputTape,
        replay: Vec<OptionTransitionSample>,
        replay_routes: Vec<InputTape>,
        terminal: FactSnapshot,
    ) -> Result<TacticQFinalResult, TacticQCampaignError> {
        let route_bytes = route_tape
            .encode()
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
        let replay_bytes = serde_cbor::to_vec(&(&replay, &replay_routes))
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        let terminal_state_sha256 = terminal
            .content_sha256()
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let mut result = TacticQFinalResult {
            schema: TACTIC_Q_FINAL_RESULT_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            execution_authority_sha256: self.execution_authority_sha256,
            objective_sha256: self.objective_sha256,
            root_checkpoint_sha256: self.root_checkpoint_sha256,
            route_tape_sha256: sha256(&route_bytes),
            replay_sha256: sha256(&replay_bytes),
            terminal_state_sha256,
            route_tape,
            replay,
            replay_routes,
            terminal,
        };
        result.content_sha256 = final_result_digest(&result)?;
        validate_final_result(&result)?;
        Ok(result)
    }

    /// Admit a native-evaluated false-to-true hindsight row only when it
    /// relabels an exact primary replay transition from this campaign. The row
    /// is refit under its own compiled objective, never the primary critic.
    pub fn admit_hindsight(
        &mut self,
        row: RelabeledHindsightOption,
    ) -> Result<&OptionValueModel, TacticQCampaignError> {
        let Some((index, _)) = self.replay.iter().enumerate().find(|(_, original)| {
            if original.value_sample.reward.to_bits() != row.original_reward.to_bits()
                || original.value_sample.terminal
            {
                return false;
            }
            let mut expected = original.value_sample.clone();
            expected.reward = row.transition.reward;
            expected.terminal = true;
            expected == row.transition
        }) else {
            return Err(TacticQCampaignError::InvalidState(
                "hindsight row does not relabel campaign replay",
            ));
        };
        self.hindsight
            .admit_and_refit(row, self.episode_groups[index], &self.model_config)
            .map_err(TacticQCampaignError::Hindsight)
    }
}
