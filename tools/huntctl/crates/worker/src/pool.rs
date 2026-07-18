use crate::client::{HelloResponse, WorkerBuildIdentity, WorkerCapabilities, WorkerClient};
use crate::transport::ProcessTransport;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MixedBuildPolicy {
    /// All accepted workers must report byte-for-byte equal build identity.
    #[default]
    RequireIdentical,
    /// Keep workers with different builds and expose each build in metadata.
    AllowMixed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerLaunch {
    pub label: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerMetadata {
    pub index: usize,
    pub label: String,
    pub process_id: u32,
    pub build: WorkerBuildIdentity,
    pub capabilities: WorkerCapabilities,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupFailureKind {
    Spawn,
    Protocol,
    IncompatibleBuild,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupFailure {
    pub index: usize,
    pub label: String,
    pub kind: StartupFailureKind,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthJobResult {
    pub job_id: u64,
    pub worker_index: usize,
    pub worker_label: String,
    pub latency_micros: u128,
    pub error: Option<String>,
}

impl HealthJobResult {
    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolHealthReport {
    pub jobs: Vec<HealthJobResult>,
}

impl PoolHealthReport {
    pub fn all_ok(&self) -> bool {
        self.jobs.iter().all(HealthJobResult::is_ok)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShutdownResult {
    pub worker_index: usize,
    pub worker_label: String,
    pub error: Option<String>,
}

pub struct PoolStartReport {
    pub pool: WorkerPool,
    pub failures: Vec<StartupFailure>,
}

enum WorkerCommand {
    Ping {
        job_id: u64,
        reply: Sender<HealthJobResult>,
    },
    Shutdown {
        reply: Sender<ShutdownResult>,
    },
}

struct WorkerHandle {
    metadata: WorkerMetadata,
    commands: Sender<WorkerCommand>,
    thread: Option<JoinHandle<()>>,
}

pub struct WorkerPool {
    workers: Vec<WorkerHandle>,
    policy: MixedBuildPolicy,
}

impl WorkerPool {
    /// Starts every launch independently. A failed worker is reported and does
    /// not prevent compatible workers from entering the pool.
    pub fn spawn(launches: Vec<WorkerLaunch>, policy: MixedBuildPolicy) -> PoolStartReport {
        let mut pool = Self {
            workers: Vec::new(),
            policy,
        };
        let mut failures = Vec::new();
        let mut reference_identity: Option<HelloResponse> = None;

        for (index, launch) in launches.into_iter().enumerate() {
            let transport = match ProcessTransport::spawn(&launch.program, &launch.args) {
                Ok(transport) => transport,
                Err(error) => {
                    failures.push(StartupFailure {
                        index,
                        label: launch.label,
                        kind: StartupFailureKind::Spawn,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let process_id = transport.child_id();
            let mut client = WorkerClient::new(transport);
            let hello = match client.handshake() {
                Ok(hello) => hello.clone(),
                Err(error) => {
                    failures.push(StartupFailure {
                        index,
                        label: launch.label,
                        kind: StartupFailureKind::Protocol,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let differences = reference_identity
                .as_ref()
                .map(|expected| expected.identity_differences(&hello))
                .unwrap_or_default();
            if policy == MixedBuildPolicy::RequireIdentical && !differences.is_empty() {
                let _ = client.shutdown();
                failures.push(StartupFailure {
                    index,
                    label: launch.label,
                    kind: StartupFailureKind::IncompatibleBuild,
                    message: format!(
                        "worker identity mismatch:\n- {}",
                        differences
                            .iter()
                            .map(|difference| difference.message())
                            .collect::<Vec<_>>()
                            .join("\n- ")
                    ),
                });
                continue;
            }
            reference_identity.get_or_insert_with(|| hello.clone());
            pool.workers
                .push(start_worker(index, launch.label, process_id, hello, client));
        }
        PoolStartReport { pool, failures }
    }

    pub fn policy(&self) -> MixedBuildPolicy {
        self.policy
    }
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }
    pub fn workers(&self) -> impl ExactSizeIterator<Item = &WorkerMetadata> {
        self.workers.iter().map(|worker| &worker.metadata)
    }

    /// Distributes coarse health jobs round-robin. Each worker executes its
    /// queue serially, while different workers execute in parallel.
    pub fn health_jobs(&self, job_count: usize) -> PoolHealthReport {
        if job_count == 0 {
            return PoolHealthReport::default();
        }
        if self.workers.is_empty() {
            return PoolHealthReport {
                jobs: (0..job_count)
                    .map(|job_id| HealthJobResult {
                        job_id: job_id as u64,
                        worker_index: usize::MAX,
                        worker_label: String::new(),
                        latency_micros: 0,
                        error: Some("worker pool is empty".into()),
                    })
                    .collect(),
            };
        }

        let (reply_tx, reply_rx) = mpsc::channel();
        let mut immediate = Vec::new();
        let mut submitted = 0;
        for job_id in 0..job_count {
            let worker = &self.workers[job_id % self.workers.len()];
            let command = WorkerCommand::Ping {
                job_id: job_id as u64,
                reply: reply_tx.clone(),
            };
            if worker.commands.send(command).is_ok() {
                submitted += 1;
            } else {
                immediate.push(HealthJobResult {
                    job_id: job_id as u64,
                    worker_index: worker.metadata.index,
                    worker_label: worker.metadata.label.clone(),
                    latency_micros: 0,
                    error: Some("worker command channel is closed".into()),
                });
            }
        }
        drop(reply_tx);
        immediate.extend(reply_rx.iter().take(submitted));
        let mut reported = vec![false; job_count];
        for result in &immediate {
            if let Some(slot) = reported.get_mut(result.job_id as usize) {
                *slot = true;
            }
        }
        for (job_id, was_reported) in reported.into_iter().enumerate() {
            if !was_reported {
                let worker = &self.workers[job_id % self.workers.len()];
                immediate.push(HealthJobResult {
                    job_id: job_id as u64,
                    worker_index: worker.metadata.index,
                    worker_label: worker.metadata.label.clone(),
                    latency_micros: 0,
                    error: Some("worker exited without reporting the job".into()),
                });
            }
        }
        immediate.sort_by_key(|result| result.job_id);
        PoolHealthReport { jobs: immediate }
    }

    pub fn shutdown(&mut self) -> Vec<ShutdownResult> {
        let workers = std::mem::take(&mut self.workers);
        let mut results = Vec::with_capacity(workers.len());
        for mut worker in workers {
            let (reply_tx, reply_rx) = mpsc::channel();
            let fallback = ShutdownResult {
                worker_index: worker.metadata.index,
                worker_label: worker.metadata.label.clone(),
                error: Some("worker command channel is closed".into()),
            };
            let result = if worker
                .commands
                .send(WorkerCommand::Shutdown { reply: reply_tx })
                .is_ok()
            {
                reply_rx.recv().unwrap_or(fallback)
            } else {
                fallback
            };
            if let Some(thread) = worker.thread.take()
                && thread.join().is_err()
                && result.error.is_none()
            {
                results.push(ShutdownResult {
                    error: Some("worker thread panicked".into()),
                    ..result
                });
                continue;
            }
            results.push(result);
        }
        results
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn start_worker(
    index: usize,
    label: String,
    process_id: u32,
    hello: HelloResponse,
    mut client: WorkerClient<ProcessTransport>,
) -> WorkerHandle {
    let metadata = WorkerMetadata {
        index,
        label: label.clone(),
        process_id,
        build: hello.build,
        capabilities: hello.capabilities,
    };
    let thread_label = label.clone();
    let (commands, receiver) = mpsc::channel();
    let thread = thread::Builder::new()
        .name(format!("hunt-worker-{index}"))
        .spawn(move || {
            while let Ok(command) = receiver.recv() {
                match command {
                    WorkerCommand::Ping { job_id, reply } => {
                        let started = Instant::now();
                        let error = client.ping().err().map(|error| error.to_string());
                        let _ = reply.send(HealthJobResult {
                            job_id,
                            worker_index: index,
                            worker_label: thread_label.clone(),
                            latency_micros: started.elapsed().as_micros(),
                            error,
                        });
                    }
                    WorkerCommand::Shutdown { reply } => {
                        let error = client.shutdown().err().map(|error| error.to_string());
                        let _ = reply.send(ShutdownResult {
                            worker_index: index,
                            worker_label: thread_label.clone(),
                            error,
                        });
                        break;
                    }
                }
            }
        })
        .expect("failed to spawn worker control thread");
    WorkerHandle {
        metadata,
        commands,
        thread: Some(thread),
    }
}
