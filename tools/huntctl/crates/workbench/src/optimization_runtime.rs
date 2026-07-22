//! Explicit, workbench-owned launch state for native residual optimization.

use super::*;
use dusklight_orchestration::native_residual_campaign::{
    NativeResidualExecutionBinding, materialize_native_residual_process_tape,
};
use dusklight_orchestration::native_residual_campaign_runner::{
    NativeResidualCampaignRunConfig, run_native_residual_campaign,
};
use dusklight_orchestration::optimization_request::OptimizationRequest;
use std::fs::OpenOptions;

const OPTIMIZATION_START_SCHEMA: &str = "dusklight.route-workbench.optimization-start.v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserOptimizationStartRequest {
    pub campaign: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct OptimizationStartResponse {
    pub schema: &'static str,
    pub campaign: String,
    pub request_sha256: String,
    pub status: &'static str,
}

#[derive(Clone, Debug)]
pub(super) struct OptimizationRuntimeStatus {
    pub status: &'static str,
    pub error: Option<String>,
}

fn optimization_runs() -> &'static Mutex<BTreeMap<String, OptimizationRuntimeStatus>> {
    static RUNS: OnceLock<Mutex<BTreeMap<String, OptimizationRuntimeStatus>>> = OnceLock::new();
    RUNS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn optimization_runtime_status(
    request_sha256: &str,
) -> Option<OptimizationRuntimeStatus> {
    optimization_runs()
        .lock()
        .ok()?
        .get(request_sha256)
        .cloned()
}

pub(super) fn start_optimization_campaign(
    config: &WorkbenchConfig,
    browser_request: &BrowserOptimizationStartRequest,
) -> Result<OptimizationStartResponse, WorkbenchError> {
    let world_context = config.world_context.as_ref().ok_or_else(|| {
        WorkbenchError::new(
            "optimization requires a sealed world context; restart the workbench with --world-context WORLD.json",
        )
    })?;
    let root = config
        .repository_root
        .canonicalize()
        .map_err(optimization_runtime_error)?;
    let timeline = load_authoritative_timeline(&config.timeline_path)?;
    let artifact_root = configured_artifact_root(config)?;
    let mut graph = graph_with_drafts(&timeline, &artifact_root, &config.state_root)?;
    append_optimization_campaigns(&mut graph, &root, &config.timeline_path)?;
    let campaign = graph
        .campaigns
        .iter()
        .find(|campaign| campaign.id == browser_request.campaign)
        .ok_or_else(|| WorkbenchError::new("unknown optimization campaign"))?;
    if campaign.status == "invalid" {
        return Err(WorkbenchError::new(
            campaign
                .error
                .as_deref()
                .unwrap_or("optimization campaign is invalid"),
        ));
    }
    if campaign.status == "completed" {
        return Err(WorkbenchError::new(
            "optimization campaign is already complete",
        ));
    }

    let request_path = root.join(&campaign.request);
    let optimization: OptimizationRequest =
        serde_json::from_slice(&fs::read(&request_path).map_err(optimization_runtime_error)?)
            .map_err(optimization_runtime_error)?;
    optimization
        .validate_files(&root)
        .map_err(optimization_runtime_error)?;
    if optimization.id != browser_request.campaign {
        return Err(WorkbenchError::new(
            "optimization request identity changed while launching",
        ));
    }
    let request_sha256 = optimization.content_sha256.to_string();
    {
        let mut runs = optimization_runs()
            .lock()
            .map_err(|_| WorkbenchError::new("optimization runtime registry is unavailable"))?;
        if runs
            .get(&request_sha256)
            .is_some_and(|run| run.status == "running")
        {
            return Err(WorkbenchError::new(
                "optimization campaign is already running",
            ));
        }
        runs.insert(
            request_sha256.clone(),
            OptimizationRuntimeStatus {
                status: "preparing",
                error: None,
            },
        );
    }

    let execution = match prepare_optimization_execution(
        &root,
        &optimization,
        &config.game,
        &config.dvd,
        world_context,
    ) {
        Ok(execution) => execution,
        Err(error) => {
            set_optimization_runtime_status(
                &request_sha256,
                OptimizationRuntimeStatus {
                    status: "failed",
                    error: Some(error.to_string()),
                },
            );
            return Err(error);
        }
    };
    set_optimization_runtime_status(
        &request_sha256,
        OptimizationRuntimeStatus {
            status: "running",
            error: None,
        },
    );

    let thread_root = root;
    let thread_request_sha256 = request_sha256.clone();
    let spawn = thread::Builder::new()
        .name(format!("optimization-{}", optimization.id))
        .spawn(move || {
            let result = run_native_residual_campaign(&NativeResidualCampaignRunConfig {
                repository_root: &thread_root,
                optimization: &optimization,
                execution: &execution,
            });
            let status = match result {
                Ok(_) => OptimizationRuntimeStatus {
                    status: "completed",
                    error: None,
                },
                Err(error) => OptimizationRuntimeStatus {
                    status: "failed",
                    error: Some(error.to_string()),
                },
            };
            set_optimization_runtime_status(&thread_request_sha256, status);
        });
    if let Err(error) = spawn {
        let message = format!("cannot start optimization thread: {error}");
        set_optimization_runtime_status(
            &request_sha256,
            OptimizationRuntimeStatus {
                status: "failed",
                error: Some(message.clone()),
            },
        );
        return Err(WorkbenchError::new(message));
    }

    Ok(OptimizationStartResponse {
        schema: OPTIMIZATION_START_SCHEMA,
        campaign: browser_request.campaign.clone(),
        request_sha256,
        status: "running",
    })
}

fn prepare_optimization_execution(
    root: &Path,
    optimization: &OptimizationRequest,
    game: &Path,
    dvd: &Path,
    world_context: &Path,
) -> Result<NativeResidualExecutionBinding, WorkbenchError> {
    let state_path = root.join(&optimization.resume.state_path);
    let execution_root = state_path
        .parent()
        .ok_or_else(|| WorkbenchError::new("optimization state path has no campaign root"))?
        .join("execution");
    let tape_path = execution_root.join("process-route.tape");
    let program_path = execution_root.join("terminal.dmsp");
    let binding_path = execution_root.join("execution.json");
    if binding_path.exists() {
        let binding: NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(&binding_path).map_err(optimization_runtime_error)?)
                .map_err(optimization_runtime_error)?;
        binding
            .validate_files(root, optimization)
            .map_err(optimization_runtime_error)?;
        return Ok(binding);
    }

    let tape = materialize_native_residual_process_tape(root, optimization)
        .map_err(optimization_runtime_error)?;
    write_exact_or_new(
        &tape_path,
        &tape.encode().map_err(optimization_runtime_error)?,
    )?;
    let source_path = root.join(&optimization.terminal_predicate.source.path);
    let source = fs::read_to_string(source_path).map_err(optimization_runtime_error)?;
    let compiled = milestone_dsl::compile_source(&source).map_err(optimization_runtime_error)?;
    write_exact_or_new(&program_path, &compiled.bytes)?;
    let card_fixture_manifest = root
        .join(&optimization.route.timeline.path)
        .with_extension("")
        .join("benchmarks/process_boot.fixture.json");
    let binding = NativeResidualExecutionBinding::seal(
        root,
        optimization,
        game,
        dvd,
        &tape_path,
        &program_path,
        world_context,
        &card_fixture_manifest,
        8,
        false,
    )
    .map_err(optimization_runtime_error)?;
    write_exact_or_new(
        &binding_path,
        &binding
            .to_pretty_json()
            .map_err(optimization_runtime_error)?,
    )?;
    Ok(binding)
}

fn write_exact_or_new(path: &Path, bytes: &[u8]) -> Result<(), WorkbenchError> {
    if path.exists() {
        if fs::read(path).map_err(optimization_runtime_error)? != bytes {
            return Err(WorkbenchError::new(format!(
                "existing optimization artifact differs: {}",
                path.display()
            )));
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(optimization_runtime_error)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(optimization_runtime_error)?;
    file.write_all(bytes).map_err(optimization_runtime_error)?;
    file.sync_all().map_err(optimization_runtime_error)?;
    Ok(())
}

fn set_optimization_runtime_status(request_sha256: &str, status: OptimizationRuntimeStatus) {
    if let Ok(mut runs) = optimization_runs().lock() {
        runs.insert(request_sha256.into(), status);
    }
}

fn optimization_runtime_error(error: impl fmt::Display) -> WorkbenchError {
    WorkbenchError::new(error.to_string())
}
