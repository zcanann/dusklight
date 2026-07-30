use super::worker_pool::{
    NativeTacticProposalJob, NativeTacticProposalPool, launch_tactic_route_worker,
    run_tactic_proposal_worker,
};
use super::*;

const MAX_CONCURRENT_TACTIC_WORKER_LAUNCHES: usize = 2;

pub(super) struct NativeTacticWorkerFleet {
    senders: Vec<mpsc::Sender<NativeTacticProposalJob>>,
    worker_handles: Vec<std::thread::JoinHandle<Result<(), NativeTacticRouteRunError>>>,
    optimization_request_sha256: Digest,
    execution_binding_sha256: Digest,
    execution_plan_sha256: Digest,
    initial_facts: FactSnapshot,
    root_checkpoint_sha256: Digest,
    checkpoint_cache_capacity_bytes: usize,
    launch_micros: u64,
}

impl NativeTacticWorkerFleet {
    pub(super) fn launch(
        config: &NativeTacticRouteRunConfig<'_>,
        repository_root: &Path,
        fleet_root: &Path,
        initial_batch: &NativeSuffixBatch,
        terminal: &NativeTerminalBinding,
        card_fixture: &Path,
        worker_count: usize,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if worker_count == 0 || worker_count > MAX_ROUTE_WORKERS {
            return Err(route_message("native tactic worker fleet size is invalid"));
        }
        let checkpoint_cache_capacity_bytes = tactic_checkpoint_cache_capacity_per_worker(
            config.execution_plan.budgets.memory_bytes,
            worker_count,
        )?;
        let execution_plan_sha256 = config.execution_plan.identity()?;
        if initial_batch
            .checkpoint_cache
            .as_ref()
            .is_none_or(|cache| cache.capacity_bytes != checkpoint_cache_capacity_bytes)
        {
            return Err(route_message(
                "native tactic worker fleet cache capacity is detached from its execution plan",
            ));
        }
        fs::create_dir_all(fleet_root).map_err(route_error)?;
        let attempt_roots = (0..worker_count)
            .map(|_| reserve_attempt_root(fleet_root))
            .collect::<Result<Vec<_>, _>>()?;
        let process_launch_started = Instant::now();
        let mut launched = Vec::with_capacity(worker_count);
        for batch in worker_launch_batches(worker_count)? {
            let mut batch_launched = std::thread::scope(|scope| {
                let handles = batch
                    .clone()
                    .map(|worker_index| {
                        let attempt_root = &attempt_roots[worker_index];
                        scope.spawn(move || {
                            launch_tactic_route_worker(
                                config,
                                repository_root,
                                attempt_root,
                                initial_batch,
                                terminal,
                                card_fixture,
                            )
                            .map(|worker| (worker_index, worker))
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().map_err(|_| {
                            route_message("native tactic route worker launch panicked")
                        })?
                    })
                    .collect::<Result<Vec<_>, _>>()
            })?;
            launched.append(&mut batch_launched);
        }
        let launch_micros = elapsed_micros(process_launch_started.elapsed());
        launched.sort_by_key(|(worker_index, _)| *worker_index);

        let mut sessions = Vec::with_capacity(worker_count);
        let mut worker_initial_facts = Vec::with_capacity(worker_count);
        let mut worker_root_checkpoints = Vec::with_capacity(worker_count);
        let mut worker_checkpoint_bytes = Vec::with_capacity(worker_count);
        for (_, (worker, facts, root_checkpoint_sha256, checkpoint_bytes)) in launched {
            sessions.push(worker);
            worker_initial_facts.push(facts);
            worker_root_checkpoints.push(root_checkpoint_sha256);
            worker_checkpoint_bytes.push(checkpoint_bytes);
        }
        validate_fleet_checkpoint_capacity(
            checkpoint_cache_capacity_bytes,
            &worker_checkpoint_bytes,
        )?;
        let initial_facts = worker_initial_facts
            .first()
            .cloned()
            .ok_or_else(|| route_message("native tactic worker fleet is empty"))?;
        let root_checkpoint_sha256 = worker_root_checkpoints[0];
        if initial_facts.tape_frame != config.optimization.route.source_boundary_index
            || initial_facts.terminal.reached != Some(false)
            || worker_initial_facts
                .iter()
                .any(|facts| facts != &initial_facts)
            || worker_root_checkpoints
                .iter()
                .any(|checkpoint| *checkpoint != root_checkpoint_sha256)
        {
            return Err(route_message(
                "native worker fleet does not share the requested source boundary",
            ));
        }

        let mut senders = Vec::with_capacity(worker_count);
        let worker_handles = sessions
            .into_iter()
            .enumerate()
            .map(|(worker_slot, worker)| {
                let (sender, receiver) = mpsc::channel();
                senders.push(sender);
                std::thread::spawn(move || {
                    run_tactic_proposal_worker(worker_slot, worker, receiver)
                })
            })
            .collect();
        Ok(Self {
            senders,
            worker_handles,
            optimization_request_sha256: config.optimization.content_sha256,
            execution_binding_sha256: config.execution.content_sha256,
            execution_plan_sha256,
            initial_facts,
            root_checkpoint_sha256,
            checkpoint_cache_capacity_bytes,
            launch_micros,
        })
    }

    pub(super) fn validate_for(
        &self,
        config: &NativeTacticRouteRunConfig<'_>,
    ) -> Result<(), NativeTacticRouteRunError> {
        if config.optimization.content_sha256 != self.optimization_request_sha256
            || config.execution.content_sha256 != self.execution_binding_sha256
            || config.execution_plan.identity()? != self.execution_plan_sha256
            || config.workers == 0
            || config.workers > self.senders.len()
            || self.initial_facts.tape_frame != config.optimization.route.source_boundary_index
            || self.initial_facts.terminal.reached != Some(false)
        {
            return Err(route_message(
                "native tactic route is incompatible with its persistent worker fleet",
            ));
        }
        Ok(())
    }

    pub(super) fn initial_facts(&self) -> &FactSnapshot {
        &self.initial_facts
    }

    pub(super) fn root_checkpoint_sha256(&self) -> Digest {
        self.root_checkpoint_sha256
    }

    pub(super) fn launch_micros(&self) -> u64 {
        self.launch_micros
    }

    pub(super) fn pool(
        &self,
        config: &NativeTacticRouteRunConfig<'_>,
        execution_plan_sha256: Digest,
        root_source_frame: usize,
    ) -> Result<NativeTacticProposalPool, NativeTacticRouteRunError> {
        self.validate_for(config)?;
        if execution_plan_sha256 != self.execution_plan_sha256 {
            return Err(route_message(
                "native tactic worker fleet execution plan identity is detached",
            ));
        }
        Ok(proposal_pool_view(
            &self.senders,
            config.workers,
            config
                .execution_plan
                .checkpoint
                .cross_decision_direct_restore,
            root_source_frame,
            config.execution_plan.execution_strategy,
            execution_plan_sha256,
            self.checkpoint_cache_capacity_bytes,
        )?
        .with_lane_owner_partition(config.execution_plan))
    }

    pub(super) fn shutdown(mut self) -> Result<(), NativeTacticRouteRunError> {
        self.shutdown_inner()
    }

    fn shutdown_inner(&mut self) -> Result<(), NativeTacticRouteRunError> {
        self.senders.clear();
        let handles = std::mem::take(&mut self.worker_handles);
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| route_message("native tactic route worker thread panicked"))?
            })
            .collect::<Result<Vec<_>, _>>()
            .map(drop)
    }
}

impl Drop for NativeTacticWorkerFleet {
    fn drop(&mut self) {
        let _ = self.shutdown_inner();
    }
}

fn worker_launch_batches(
    worker_count: usize,
) -> Result<Vec<std::ops::Range<usize>>, NativeTacticRouteRunError> {
    if worker_count == 0 || worker_count > MAX_ROUTE_WORKERS {
        return Err(route_message("native tactic worker fleet size is invalid"));
    }
    Ok((0..worker_count)
        .step_by(MAX_CONCURRENT_TACTIC_WORKER_LAUNCHES)
        .map(|start| {
            start..worker_count.min(start.saturating_add(MAX_CONCURRENT_TACTIC_WORKER_LAUNCHES))
        })
        .collect())
}

fn validate_fleet_checkpoint_capacity(
    checkpoint_cache_capacity_bytes: usize,
    worker_checkpoint_bytes: &[u64],
) -> Result<u64, NativeTacticRouteRunError> {
    let checkpoint_bytes = worker_checkpoint_bytes
        .first()
        .copied()
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| route_message("native tactic worker fleet reported no root checkpoint"))?;
    if worker_checkpoint_bytes
        .iter()
        .any(|bytes| *bytes != checkpoint_bytes)
    {
        return Err(route_message(
            "native tactic worker fleet root checkpoint sizes differ",
        ));
    }
    let capacity_bytes = u64::try_from(checkpoint_cache_capacity_bytes).map_err(route_error)?;
    if capacity_bytes < checkpoint_bytes {
        return Err(route_message(format!(
            "native tactic worker fleet checkpoint cache is too small: \
             {capacity_bytes} bytes per worker cannot retain a {checkpoint_bytes}-byte checkpoint"
        )));
    }
    Ok(checkpoint_bytes)
}

fn proposal_pool_view(
    senders: &[mpsc::Sender<NativeTacticProposalJob>],
    active_workers: usize,
    direct_restore_enabled: bool,
    root_source_frame: usize,
    execution_strategy: NativeGenericExecutionStrategy,
    execution_plan_sha256: Digest,
    checkpoint_cache_capacity_bytes: usize,
) -> Result<NativeTacticProposalPool, NativeTacticRouteRunError> {
    if active_workers == 0 || active_workers > senders.len() {
        return Err(route_message(
            "native tactic worker fleet view size is invalid",
        ));
    }
    Ok(NativeTacticProposalPool {
        senders: Arc::new(senders.iter().take(active_workers).cloned().collect()),
        next_worker: Arc::new(AtomicUsize::new(0)),
        direct_restore_enabled,
        root_source_frame,
        execution_strategy,
        execution_plan_sha256,
        checkpoint_cache_capacity_bytes,
        dedicated_owner_slots: 0,
        preferred_owner_slot: None,
    })
}

#[cfg(test)]
#[path = "worker_fleet_tests.rs"]
mod tests;
