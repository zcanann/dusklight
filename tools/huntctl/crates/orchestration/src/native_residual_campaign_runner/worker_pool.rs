//! Own persistent native workers and their process-local checkpoint sessions.

use super::*;

impl<'a> WorkerPool<'a> {
    pub(super) fn new(
        root: &'a Path,
        campaign: &'a Path,
        optimization: &'a OptimizationRequest,
        execution: &'a NativeResidualExecutionBinding,
    ) -> Result<Self, NativeResidualCampaignRunnerError> {
        Self::new_for_terminal(
            root,
            campaign,
            optimization,
            execution,
            NativeTerminalBinding {
                goal: optimization.terminal_predicate.goal.clone(),
                program_sha256: optimization.terminal_predicate.program_sha256,
                definition_sha256: optimization.terminal_predicate.definition_sha256,
            },
            root.join(&execution.milestone_program.path),
            "promotion",
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_for_terminal(
        root: &'a Path,
        campaign: &'a Path,
        optimization: &'a OptimizationRequest,
        execution: &'a NativeResidualExecutionBinding,
        terminal: NativeTerminalBinding,
        milestone_program: PathBuf,
        namespace: &str,
    ) -> Result<Self, NativeResidualCampaignRunnerError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(native_error)?
            .as_nanos();
        let session_root = campaign
            .join("native-sessions")
            .join(namespace)
            .join(format!("run-{}-{nonce}", std::process::id()));
        let lanes = optimization
            .execution
            .deterministic_seeds
            .iter()
            .enumerate()
            .map(|(index, seed)| WorkerLane {
                index,
                seed: *seed,
                session: None,
                state_root: session_root.join(format!("worker-{index:03}")),
            })
            .collect();
        Ok(Self {
            root,
            optimization,
            execution,
            card_fixture_root: execution
                .card_fixture_root(root, optimization)
                .map_err(native_error)?,
            terminal,
            milestone_program,
            session_root,
            lanes,
        })
    }

    pub(super) fn run_jobs(
        &mut self,
        jobs: Vec<BatchJob>,
    ) -> Result<Vec<BatchOutput>, NativeResidualCampaignRunnerError> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut by_lane = jobs
            .into_iter()
            .map(|job| (job.lane, job))
            .collect::<BTreeMap<_, _>>();
        let root = self.root;
        let optimization = self.optimization;
        let execution = self.execution;
        let terminal = &self.terminal;
        let milestone_program = &self.milestone_program;
        let card_fixture_root = &self.card_fixture_root;
        let outputs = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for lane in &mut self.lanes {
                let Some(job) = by_lane.remove(&lane.index) else {
                    continue;
                };
                handles.push(scope.spawn(move || {
                    run_lane_job(
                        root,
                        optimization,
                        execution,
                        terminal,
                        milestone_program,
                        card_fixture_root,
                        lane,
                        job,
                    )
                }));
            }
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| native_message("native residual worker thread panicked"))?
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        if !by_lane.is_empty() {
            return Err(native_message(
                "native batch targeted an unknown worker lane",
            ));
        }
        let mut hello: Option<&dusklight_worker_protocol::client::HelloResponse> = None;
        for lane in &self.lanes {
            let Some(session) = &lane.session else {
                continue;
            };
            if let Some(expected) = hello {
                let differences = expected.identity_differences(session.hello());
                if !differences.is_empty() {
                    return Err(native_message(format!(
                        "native residual worker pool build identity differs: {}",
                        differences
                            .iter()
                            .map(|difference| difference.message())
                            .collect::<Vec<_>>()
                            .join("; ")
                    )));
                }
            } else {
                hello = Some(session.hello());
            }
        }
        Ok(outputs)
    }

    pub(super) fn shutdown(&mut self) -> Result<(), NativeResidualCampaignRunnerError> {
        let mut failures = Vec::new();
        for lane in &mut self.lanes {
            if let Some(session) = lane.session.take()
                && let Err(error) = session.shutdown()
            {
                failures.push(format!("worker {}: {error}", lane.index));
            }
        }
        match fs::remove_dir_all(&self.session_root) {
            Ok(()) => {
                if let Some(parent) = self.session_root.parent() {
                    let _ = fs::remove_dir(parent);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!(
                "ephemeral session {}: {error}",
                self.session_root.display()
            )),
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(native_message(format!(
                "native residual worker shutdown failed: {}",
                failures.join("; ")
            )))
        }
    }
}

impl Drop for WorkerPool<'_> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl<'a> NativeResidualExactReplayPool<'a> {
    pub(crate) fn new(
        root: &'a Path,
        campaign: &'a Path,
        config: &'a NativeResidualCampaignRunConfig<'a>,
    ) -> Result<Self, NativeResidualCampaignRunnerError> {
        ensure_not_cancelled(config)?;
        config
            .execution
            .validate_files(root, config.optimization)
            .map_err(native_error)?;
        Ok(Self {
            root,
            campaign,
            config,
            profile: segment_profile(root, config.optimization)?,
            pool: WorkerPool::new(root, campaign, config.optimization, config.execution)?,
            round: 0,
        })
    }

    pub(crate) fn replay(
        &mut self,
        candidates: &[NativeResidualExactReplayCandidate],
    ) -> Result<BTreeMap<String, Vec<NativeResidualAttempt>>, NativeResidualCampaignRunnerError>
    {
        self.replay_with_repetitions(candidates, self.config.optimization.execution.repetitions)
    }

    pub(crate) fn replay_with_repetitions(
        &mut self,
        candidates: &[NativeResidualExactReplayCandidate],
        repetitions: u16,
    ) -> Result<BTreeMap<String, Vec<NativeResidualAttempt>>, NativeResidualCampaignRunnerError>
    {
        self.replay_with_process_mode(candidates, repetitions, false)
    }

    pub(crate) fn replay_with_cold_repetitions(
        &mut self,
        candidates: &[NativeResidualExactReplayCandidate],
        repetitions: u16,
    ) -> Result<BTreeMap<String, Vec<NativeResidualAttempt>>, NativeResidualCampaignRunnerError>
    {
        self.replay_with_process_mode(candidates, repetitions, true)
    }

    fn replay_with_process_mode(
        &mut self,
        candidates: &[NativeResidualExactReplayCandidate],
        repetitions: u16,
        cold_process_per_repetition: bool,
    ) -> Result<BTreeMap<String, Vec<NativeResidualAttempt>>, NativeResidualCampaignRunnerError>
    {
        if candidates.is_empty() {
            return Err(native_message(
                "native exact replay requires at least one candidate",
            ));
        }
        if repetitions == 0 {
            return Err(native_message(
                "native exact replay requires at least one repetition",
            ));
        }
        let mut ids = BTreeSet::new();
        if candidates.iter().any(|candidate| {
            candidate.id.is_empty()
                || candidate.id.len() > 128
                || !candidate
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
                || !ids.insert(candidate.id.as_str())
        }) {
            return Err(native_message(
                "native exact replay candidate IDs are invalid or duplicated",
            ));
        }
        let lane_count = self.pool.lanes.len();
        if lane_count == 0 {
            return Err(native_message("native exact replay has no worker lanes"));
        }
        let mut attempts = candidates
            .iter()
            .map(|candidate| (candidate.id.clone(), Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for repetition in 1..=repetitions {
            ensure_not_cancelled(self.config)?;
            let mut groups = vec![Vec::new(); lane_count];
            for (index, candidate) in candidates.iter().enumerate() {
                groups[index % lane_count].push(candidate);
            }
            let mut jobs = Vec::new();
            let mut adopted = Vec::new();
            for (lane, group) in groups.iter().enumerate() {
                if group.is_empty() {
                    continue;
                }
                let batch = exact_replay_batch(
                    self.config.optimization,
                    self.config.execution,
                    self.profile,
                    group,
                    repetition,
                )?;
                let batch_root = self
                    .campaign
                    .join("minimization")
                    .join("native-batches")
                    .join(format!("round-{:06}", self.round))
                    .join(format!("repetition-{repetition:03}"))
                    .join(format!("worker-{lane:03}"))
                    .join(format!("batch-{}", batch_group_id(&batch)));
                fs::create_dir_all(&batch_root).map_err(native_error)?;
                let request_path = batch_root.join("request.json");
                write_exact_or_new(&request_path, &pretty_json(&batch).map_err(native_error)?)
                    .map_err(native_error)?;
                let (result_path, validated) =
                    select_result_path(&batch_root, &batch, &self.pool.terminal)?;
                if let Some(validated) = validated {
                    adopted.push(BatchOutput {
                        lane,
                        request_path,
                        result_path,
                        validated,
                    });
                } else {
                    jobs.push(BatchJob {
                        lane,
                        request_path,
                        result_path,
                        batch,
                    });
                }
            }
            let mut outputs = self.pool.run_jobs(jobs)?;
            outputs.extend(adopted);
            outputs.sort_by_key(|output| output.lane);
            for output in outputs {
                let request =
                    artifact_reference(self.root, &output.request_path).map_err(native_error)?;
                let result =
                    artifact_reference(self.root, &output.result_path).map_err(native_error)?;
                let episode =
                    artifact_reference(self.root, Path::new(&output.validated.episode_shard_path))
                        .map_err(native_error)?;
                for actual in &output.validated.candidates {
                    let candidate_id = actual
                        .id
                        .strip_suffix(&format!("-r{repetition:03}"))
                        .ok_or_else(|| {
                            native_message("native exact replay wire candidate ID is malformed")
                        })?;
                    attempts
                        .get_mut(candidate_id)
                        .ok_or_else(|| {
                            native_message("native exact replay returned an unknown candidate")
                        })?
                        .push(native_attempt(
                            repetition,
                            self.pool.lanes[output.lane].seed,
                            actual,
                            request.clone(),
                            result.clone(),
                            episode.clone(),
                            &output.validated,
                        ));
                }
            }
            if cold_process_per_repetition && repetition < repetitions {
                self.pool.shutdown()?;
                self.pool = WorkerPool::new(
                    self.root,
                    self.campaign,
                    self.config.optimization,
                    self.config.execution,
                )?;
            }
        }
        self.round = self
            .round
            .checked_add(1)
            .ok_or_else(|| native_message("native exact replay round overflowed"))?;
        if attempts
            .values()
            .any(|rows| rows.len() != usize::from(repetitions))
        {
            return Err(native_message(
                "native exact replay did not return every sealed repetition",
            ));
        }
        Ok(attempts)
    }
}

pub(super) fn alternate_worker_pools<'a>(
    root: &'a Path,
    campaign: &'a Path,
    optimization: &'a OptimizationRequest,
    execution: &'a NativeResidualExecutionBinding,
) -> Result<Vec<WorkerPool<'a>>, NativeResidualCampaignRunnerError> {
    optimization
        .alternate_terminal_predicates_after_request_validation(root)
        .map_err(native_error)?
        .into_iter()
        .enumerate()
        .map(|(index, binding)| {
            let source =
                fs::read_to_string(root.join(&binding.source.path)).map_err(native_error)?;
            let program =
                dusklight_objectives::milestone_dsl::parse(&source).map_err(native_error)?;
            let program =
                dusklight_objectives::milestone_dsl::compile(&program).map_err(native_error)?;
            if Digest(program.program_sha256) != binding.program_sha256 {
                return Err(native_message(format!(
                    "alternate terminal {} compiled identity changed after request validation",
                    binding.goal
                )));
            }
            let program_path = campaign
                .join("alternate-terminals")
                .join(format!("{index:03}-{}", binding.goal))
                .join(format!("program-{}.dmsp", binding.program_sha256));
            write_exact_or_new(&program_path, &program.bytes).map_err(native_error)?;
            WorkerPool::new_for_terminal(
                root,
                campaign,
                optimization,
                execution,
                NativeTerminalBinding {
                    goal: binding.goal,
                    program_sha256: binding.program_sha256,
                    definition_sha256: binding.definition_sha256,
                },
                program_path,
                &format!("alternate-{index:03}"),
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_lane_job(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    terminal: &NativeTerminalBinding,
    milestone_program: &Path,
    card_fixture_root: &Path,
    lane: &mut WorkerLane,
    job: BatchJob,
) -> Result<BatchOutput, NativeResidualCampaignRunnerError> {
    write_exact_or_new(
        &job.request_path,
        &pretty_json(&job.batch).map_err(native_error)?,
    )
    .map_err(native_error)?;
    let validated = if let Some(session) = &mut lane.session {
        session
            .run_batch(&job.request_path, &job.result_path, None)
            .map_err(native_error)?
    } else {
        let launch = NativeSuffixWorkerLaunch {
            executable: root.join(&execution.executable.path),
            game_data: root.join(&execution.game_data.path),
            input_tape: root.join(&execution.process_boot_tape.path),
            milestone_program: milestone_program.to_path_buf(),
            card_fixture: card_fixture_root.to_path_buf(),
            card_fixture_sha256: execution.card_fixture_manifest.sha256,
            working_directory: root.to_path_buf(),
            state_root: lane.state_root.clone(),
            world_context_sha256: execution.world_context.sha256,
            terminal: terminal.clone(),
            initial_batch: job.request_path.clone(),
            initial_result: job.result_path.clone(),
            initial_winner_tape: None,
        };
        let (session, validated) = NativeSuffixWorkerSession::launch_with_prevalidated_files(
            &launch,
            NativeSuffixPrevalidatedFileIdentities {
                executable_sha256: execution.executable.sha256,
                game_data_sha256: execution.game_data.sha256,
            },
        )
        .map_err(native_error)?;
        let identity = session.identity();
        if identity.source_frame != optimization.route.source_boundary_index
            || identity.source_boundary_fingerprint
                != optimization.route.native_source_boundary_fingerprint
            || identity.maximum_ticks != optimization.budgets.exploration_horizon_ticks
            || identity.checkpoint_validation_ticks != execution.checkpoint_validation_ticks
            || identity.world_context_sha256 != execution.world_context.sha256
            || identity.card_fixture_sha256 != execution.card_fixture_manifest.sha256
            || identity.terminal != *terminal
        {
            return Err(native_message(
                "native residual worker identity differs from its sealed execution",
            ));
        }
        lane.session = Some(session);
        validated
    };
    Ok(BatchOutput {
        lane: lane.index,
        request_path: job.request_path,
        result_path: job.result_path,
        validated,
    })
}
