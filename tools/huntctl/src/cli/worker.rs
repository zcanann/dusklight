//! Persistent control-worker bootstrap, health, and capability adapters.

use crate::{option, repeated_option, required_path, usage_error};
use huntctl::client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION, WorkerClient};
use huntctl::pool::{MixedBuildPolicy, WorkerLaunch, WorkerPool};
use huntctl::transport::ProcessTransport;
use serde_json::{Value, json};
use std::error::Error;
use std::path::PathBuf;

pub(crate) fn command_pool(args: &[String]) -> Result<(), Box<dyn Error>> {
    if args.first().map(String::as_str) != Some("health") {
        return usage_error();
    }
    let pool_args = &args[1..];
    let (program, worker_args) = worker_spec(pool_args)?;
    let worker_count: usize = option(pool_args, "--workers")
        .unwrap_or_else(|| "1".into())
        .parse()?;
    let check_count: usize = option(pool_args, "--checks")
        .unwrap_or_else(|| worker_count.to_string())
        .parse()?;
    if worker_count == 0 {
        return Err("--workers must be greater than zero".into());
    }
    let policy = if pool_args.iter().any(|arg| arg == "--allow-mixed-builds") {
        MixedBuildPolicy::AllowMixed
    } else {
        MixedBuildPolicy::RequireIdentical
    };
    let launches = (0..worker_count)
        .map(|index| WorkerLaunch {
            label: format!("worker-{index}"),
            program: program.clone(),
            args: worker_args.clone(),
        })
        .collect();
    let start = WorkerPool::spawn(launches, policy);
    let startup_failures: Vec<Value> = start
        .failures
        .iter()
        .map(|failure| {
            json!({
                "index": failure.index, "label": failure.label,
                "kind": format!("{:?}", failure.kind), "message": failure.message
            })
        })
        .collect();
    let mut pool = start.pool;
    let active_workers = pool.worker_count();
    let health = pool.health_jobs(check_count);
    let jobs: Vec<Value> = health
        .jobs
        .iter()
        .map(|job| {
            json!({
                "job_id": job.job_id, "worker_index": job.worker_index,
                "worker_label": job.worker_label, "ok": job.is_ok(),
                "latency_micros": job.latency_micros, "error": job.error
            })
        })
        .collect();
    let shutdown = pool.shutdown();
    let shutdown_results: Vec<Value> = shutdown
        .iter()
        .map(|result| {
            json!({
                "worker_index": result.worker_index, "worker_label": result.worker_label,
                "ok": result.error.is_none(), "error": result.error
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "policy": format!("{policy:?}"), "requested_workers": worker_count,
            "active_workers": active_workers, "startup_failures": startup_failures,
            "health_jobs": jobs, "shutdown": shutdown_results
        }))?
    );
    if !start.failures.is_empty()
        || !health.all_ok()
        || shutdown.iter().any(|result| result.error.is_some())
    {
        return Err("worker-pool health check reported failures".into());
    }
    Ok(())
}

pub(crate) fn command_hello(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (program, worker_args) = worker_spec(args)?;
    let mut client = WorkerClient::new(ProcessTransport::spawn(program, &worker_args)?);
    let hello = client.handshake()?.clone();
    println!(
        "protocol={CONTROL_PROTOCOL_NAME}/{} version={} revision={} dirty_digest={} aurora={} compiler={} target={} config={} features={} fidelity={} platform={}/{} pointer_bits={}",
        CONTROL_PROTOCOL_VERSION,
        hello.build.version,
        hello.build.revision,
        hello.build.dirty_digest,
        hello.build.aurora_revision,
        hello.build.compiler,
        hello.build.compiler_target,
        hello.build.build_type,
        hello.build.feature_digest,
        hello.build.fidelity_profile,
        hello.build.platform,
        hello.build.architecture,
        hello.build.pointer_bits
    );
    println!(
        "persistent={} engine_session={} headless={} input_tape={} batch_run={} commands={}",
        hello.capabilities.persistent_control,
        hello.capabilities.engine_session,
        hello.capabilities.headless,
        hello.capabilities.input_tape,
        hello.capabilities.batch_run,
        hello.capabilities.commands.join(",")
    );
    client.shutdown()?;
    Ok(())
}

pub(crate) fn command_ping(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (program, worker_args) = worker_spec(args)?;
    let mut client = WorkerClient::new(ProcessTransport::spawn(program, &worker_args)?);
    client.handshake()?;
    client.ping()?;
    println!("pong");
    client.shutdown()?;
    Ok(())
}

pub(crate) fn command_not_ready(command: &str, args: &[String]) -> Result<(), Box<dyn Error>> {
    let (program, worker_args) = worker_spec(args)?;
    let mut client = WorkerClient::new(ProcessTransport::spawn(program, &worker_args)?);
    let capabilities = client.handshake()?.capabilities.clone();
    client.shutdown()?;
    Err(format!("{command} is unavailable (engine_session={}, input_tape={}, batch_run={}); protocol v{CONTROL_PROTOCOL_VERSION} currently exposes bootstrap control only",
        capabilities.engine_session, capabilities.input_tape, capabilities.batch_run).into())
}

fn worker_spec(args: &[String]) -> Result<(PathBuf, Vec<String>), Box<dyn Error>> {
    Ok((
        required_path(args, "--worker")?,
        repeated_option(args, "--worker-arg"),
    ))
}
