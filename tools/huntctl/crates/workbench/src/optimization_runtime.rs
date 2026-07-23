//! Explicit, workbench-owned launch state for native residual optimization.

use super::*;
use dusklight_orchestration::native_residual_campaign::{
    NativeResidualExecutionBinding, materialize_native_residual_process_tape,
    resolve_card_fixture_manifest,
};
use dusklight_orchestration::native_residual_campaign_runner::{
    NativeResidualCampaignRunConfig, run_native_residual_campaign,
};
use dusklight_orchestration::optimization_request::OptimizationRequest;
use std::fs::OpenOptions;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const OPTIMIZATION_START_SCHEMA: &str = "dusklight.route-workbench.optimization-start.v1";
const OPTIMIZATION_LIFECYCLE_SCHEMA: &str = "dusklight.route-workbench.optimization-lifecycle.v1";

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserOptimizationLifecycleRequest {
    pub campaign: String,
    pub request_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct OptimizationLifecycleResponse {
    pub schema: &'static str,
    pub campaign: String,
    pub request_sha256: String,
    pub status: &'static str,
    pub artifacts_removed: bool,
}

#[derive(Clone, Debug)]
pub(super) struct OptimizationRuntimeStatus {
    pub status: &'static str,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
struct OptimizationRuntimeEntry {
    status: OptimizationRuntimeStatus,
    cancellation: Arc<AtomicBool>,
}

fn optimization_runs() -> &'static Mutex<BTreeMap<String, OptimizationRuntimeEntry>> {
    static RUNS: OnceLock<Mutex<BTreeMap<String, OptimizationRuntimeEntry>>> = OnceLock::new();
    RUNS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn optimization_lifecycle_edits() -> &'static Mutex<()> {
    static LIFECYCLE: OnceLock<Mutex<()>> = OnceLock::new();
    LIFECYCLE.get_or_init(|| Mutex::new(()))
}

pub(super) fn optimization_runtime_status(
    request_sha256: &str,
) -> Option<OptimizationRuntimeStatus> {
    optimization_runs()
        .lock()
        .ok()?
        .get(request_sha256)
        .map(|entry| entry.status.clone())
}

pub(super) fn start_optimization_campaign(
    config: &WorkbenchConfig,
    browser_request: &BrowserOptimizationStartRequest,
) -> Result<OptimizationStartResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization lifecycle lock is unavailable"))?;
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
    append_optimization_campaigns(&mut graph, &root, &config.timeline_path, Some(config))?;
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
    if let Some(blocker) = &campaign.blocker {
        return Err(WorkbenchError::new(blocker));
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
    if goal_learning_campaign_active(&request_sha256) {
        return Err(WorkbenchError::new(
            "goal learning must stop before residual optimization can resume",
        ));
    }
    let cancellation = Arc::new(AtomicBool::new(false));
    {
        let mut runs = optimization_runs()
            .lock()
            .map_err(|_| WorkbenchError::new("optimization runtime registry is unavailable"))?;
        if runs
            .get(&request_sha256)
            .is_some_and(|run| matches!(run.status.status, "preparing" | "running" | "cancelling"))
        {
            return Err(WorkbenchError::new(
                "optimization campaign is already running",
            ));
        }
        runs.insert(
            request_sha256.clone(),
            OptimizationRuntimeEntry {
                status: OptimizationRuntimeStatus {
                    status: "preparing",
                    error: None,
                },
                cancellation: Arc::clone(&cancellation),
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
    if cancellation.load(Ordering::Acquire) {
        set_optimization_runtime_status(
            &request_sha256,
            OptimizationRuntimeStatus {
                status: "cancelled",
                error: None,
            },
        );
        return Ok(OptimizationStartResponse {
            schema: OPTIMIZATION_START_SCHEMA,
            campaign: browser_request.campaign.clone(),
            request_sha256,
            status: "cancelled",
        });
    }
    set_optimization_runtime_status(
        &request_sha256,
        OptimizationRuntimeStatus {
            status: "running",
            error: None,
        },
    );

    let thread_root = root;
    let thread_request_sha256 = request_sha256.clone();
    let thread_cancellation = Arc::clone(&cancellation);
    let spawn = thread::Builder::new()
        .name(format!("optimization-{}", optimization.id))
        .spawn(move || {
            let result = run_native_residual_campaign(&NativeResidualCampaignRunConfig {
                repository_root: &thread_root,
                optimization: &optimization,
                execution: &execution,
                cancellation: Some(&thread_cancellation),
            });
            let status = match result {
                Ok(_) => OptimizationRuntimeStatus {
                    status: "completed",
                    error: None,
                },
                Err(error) if error.is_cancelled() => OptimizationRuntimeStatus {
                    status: "cancelled",
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

pub(super) fn cancel_optimization_campaign(
    config: &WorkbenchConfig,
    browser: &BrowserOptimizationLifecycleRequest,
) -> Result<OptimizationLifecycleResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization lifecycle lock is unavailable"))?;
    let (_, optimization) = checked_optimization_request(config, browser)?;
    let request_sha256 = optimization.content_sha256.to_string();
    let status = request_optimization_cancellation(&request_sha256)?;
    Ok(OptimizationLifecycleResponse {
        schema: OPTIMIZATION_LIFECYCLE_SCHEMA,
        campaign: optimization.id,
        request_sha256,
        status,
        artifacts_removed: false,
    })
}

fn request_optimization_cancellation(request_sha256: &str) -> Result<&'static str, WorkbenchError> {
    let mut runs = optimization_runs()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization runtime registry is unavailable"))?;
    let entry = runs
        .get_mut(request_sha256)
        .ok_or_else(|| WorkbenchError::new("optimization campaign is not running"))?;
    match entry.status.status {
        "preparing" | "running" => {
            entry.cancellation.store(true, Ordering::Release);
            entry.status = OptimizationRuntimeStatus {
                status: "cancelling",
                error: None,
            };
            Ok("cancelling")
        }
        "cancelling" => Ok("cancelling"),
        "cancelled" => Ok("cancelled"),
        _ => Err(WorkbenchError::new("optimization campaign is not running")),
    }
}

pub(super) fn cleanup_optimization_campaign(
    config: &WorkbenchConfig,
    browser: &BrowserOptimizationLifecycleRequest,
) -> Result<OptimizationLifecycleResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization lifecycle lock is unavailable"))?;
    let (root, optimization) = checked_optimization_request(config, browser)?;
    let request_sha256 = optimization.content_sha256.to_string();
    if optimization_runs()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization runtime registry is unavailable"))?
        .get(&request_sha256)
        .is_some_and(|entry| matches!(entry.status.status, "preparing" | "running" | "cancelling"))
    {
        return Err(WorkbenchError::new(
            "optimization campaign must finish cancellation before cleanup",
        ));
    }
    if optimization_request_promotion_active(&request_sha256) {
        return Err(WorkbenchError::new(
            "optimization candidate promotion must finish before cleanup",
        ));
    }
    if goal_learning_campaign_active(&request_sha256) {
        return Err(WorkbenchError::new(
            "goal learning must stop before campaign cleanup",
        ));
    }
    let artifacts_removed = remove_optimization_campaign_artifacts(&root, &optimization)?;
    if let Ok(mut runs) = optimization_runs().lock() {
        runs.remove(&request_sha256);
    }
    forget_goal_learning_campaign(&request_sha256);
    forget_optimization_promotions(&request_sha256);
    Ok(OptimizationLifecycleResponse {
        schema: OPTIMIZATION_LIFECYCLE_SCHEMA,
        campaign: optimization.id,
        request_sha256,
        status: "ready",
        artifacts_removed,
    })
}

pub(super) fn checked_optimization_request(
    config: &WorkbenchConfig,
    browser: &BrowserOptimizationLifecycleRequest,
) -> Result<(PathBuf, OptimizationRequest), WorkbenchError> {
    let root = config
        .repository_root
        .canonicalize()
        .map_err(optimization_runtime_error)?;
    let timeline = load_authoritative_timeline(&config.timeline_path)?;
    let artifact_root = configured_artifact_root(config)?;
    let mut graph = graph_with_drafts(&timeline, &artifact_root, &config.state_root)?;
    append_optimization_campaigns(&mut graph, &root, &config.timeline_path, Some(config))?;
    let campaign = graph
        .campaigns
        .iter()
        .find(|campaign| campaign.id == browser.campaign)
        .ok_or_else(|| WorkbenchError::new("unknown optimization campaign"))?;
    if campaign.request_sha256 != browser.request_sha256 {
        return Err(WorkbenchError::new(
            "optimization campaign changed; refresh before changing its lifecycle",
        ));
    }
    let path = root.join(&campaign.request);
    let optimization: OptimizationRequest =
        serde_json::from_slice(&fs::read(path).map_err(optimization_runtime_error)?)
            .map_err(optimization_runtime_error)?;
    optimization
        .validate_files(&root)
        .map_err(optimization_runtime_error)?;
    if optimization.id != browser.campaign
        || optimization.content_sha256.to_string() != browser.request_sha256
    {
        return Err(WorkbenchError::new(
            "optimization request identity changed during lifecycle action",
        ));
    }
    Ok((root, optimization))
}

pub(super) fn optimization_campaign_artifacts_present(
    root: &Path,
    optimization: &OptimizationRequest,
) -> bool {
    optimization_campaign_root(root, optimization)
        .ok()
        .is_some_and(|path| fs::symlink_metadata(path).is_ok())
}

fn remove_optimization_campaign_artifacts(
    root: &Path,
    optimization: &OptimizationRequest,
) -> Result<bool, WorkbenchError> {
    let path = optimization_campaign_root(root, optimization)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(optimization_runtime_error(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WorkbenchError::new(
            "optimization campaign artifact root is not a physical directory",
        ));
    }
    let canonical = path.canonicalize().map_err(optimization_runtime_error)?;
    if canonical != path || !canonical.starts_with(root) {
        return Err(WorkbenchError::new(
            "optimization campaign artifact root contains a symbolic-link escape",
        ));
    }
    fs::remove_dir_all(&path).map_err(optimization_runtime_error)?;
    if fs::symlink_metadata(&path).is_ok() {
        return Err(WorkbenchError::new(
            "optimization campaign artifacts remain after cleanup",
        ));
    }
    Ok(true)
}

pub(super) fn optimization_campaign_root(
    root: &Path,
    optimization: &OptimizationRequest,
) -> Result<PathBuf, WorkbenchError> {
    let state = Path::new(&optimization.resume.state_path);
    let journal = Path::new(&optimization.resume.journal_path);
    let state_parent = state
        .parent()
        .ok_or_else(|| WorkbenchError::new("optimization state has no campaign directory"))?;
    if journal.parent() != Some(state_parent) {
        return Err(WorkbenchError::new(
            "optimization state and journal do not share one cleanup-owned directory",
        ));
    }
    let components = state_parent.components().collect::<Vec<_>>();
    if components.len() < 3
        || components
            .iter()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || !state_parent.starts_with("build/campaigns")
    {
        return Err(WorkbenchError::new(
            "optimization cleanup root must be a specific directory beneath build/campaigns/",
        ));
    }
    Ok(root.join(state_parent))
}

pub(super) fn prepare_optimization_execution(
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
    let card_fixture_manifest =
        resolve_card_fixture_manifest(root, optimization).map_err(optimization_runtime_error)?;
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

pub(super) fn write_exact_or_new(path: &Path, bytes: &[u8]) -> Result<(), WorkbenchError> {
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
    if let Ok(mut runs) = optimization_runs().lock()
        && let Some(entry) = runs.get_mut(request_sha256)
    {
        entry.status = status;
    }
}

fn optimization_runtime_error(error: impl fmt::Display) -> WorkbenchError {
    WorkbenchError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "dusklight-optimization-runtime-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn cleanup_request() -> OptimizationRequest {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let mut request: OptimizationRequest = serde_json::from_slice(
            &fs::read(repository.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        request.resume.state_path = "build/campaigns/lifecycle-test/state.json".into();
        request.resume.journal_path = "build/campaigns/lifecycle-test/journal.jsonl".into();
        request
    }

    #[test]
    fn cancellation_token_transitions_only_live_runtime_entries() {
        let key = format!("runtime-cancel-test-{}", std::process::id());
        let cancellation = Arc::new(AtomicBool::new(false));
        optimization_runs().lock().unwrap().insert(
            key.clone(),
            OptimizationRuntimeEntry {
                status: OptimizationRuntimeStatus {
                    status: "running",
                    error: None,
                },
                cancellation: Arc::clone(&cancellation),
            },
        );

        assert_eq!(
            request_optimization_cancellation(&key).unwrap(),
            "cancelling"
        );
        assert!(cancellation.load(Ordering::Acquire));
        assert_eq!(
            optimization_runtime_status(&key).unwrap().status,
            "cancelling"
        );
        assert_eq!(
            request_optimization_cancellation(&key).unwrap(),
            "cancelling"
        );

        optimization_runs().lock().unwrap().remove(&key);
        assert!(request_optimization_cancellation(&key).is_err());
    }

    #[test]
    fn cleanup_removes_only_the_specific_campaign_artifact_root() {
        let root = test_root("cleanup");
        let request = cleanup_request();
        let campaign = root.join("build/campaigns/lifecycle-test");
        let sibling = root.join("build/campaigns/keep-me");
        fs::create_dir_all(campaign.join("native-sessions/run-stale")).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        fs::write(campaign.join("state.json"), b"state").unwrap();
        fs::write(sibling.join("evidence.json"), b"keep").unwrap();

        assert!(remove_optimization_campaign_artifacts(&root, &request).unwrap());
        assert!(!campaign.exists());
        assert_eq!(fs::read(sibling.join("evidence.json")).unwrap(), b"keep");
        assert!(!remove_optimization_campaign_artifacts(&root, &request).unwrap());

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_rejects_a_symlinked_campaign_root_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let root = test_root("cleanup-symlink");
        let outside = test_root("cleanup-symlink-outside");
        let request = cleanup_request();
        fs::create_dir_all(root.join("build/campaigns")).unwrap();
        fs::write(outside.join("evidence.json"), b"keep").unwrap();
        symlink(&outside, root.join("build/campaigns/lifecycle-test")).unwrap();

        assert!(remove_optimization_campaign_artifacts(&root, &request).is_err());
        assert_eq!(fs::read(outside.join("evidence.json")).unwrap(), b"keep");

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
