//! Managed persistent native suffix-worker sessions.

use crate::native_suffix_result::{
    NativeSuffixBatchResult, NativeTerminalBinding, ValidatedNativeSuffixBatch,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::native_fidelity::FIXED_AUTOMATION_CVARS;
use dusklight_automation_contracts::tape::{InputTape, TapeBoot};
use dusklight_search::suffix_batch::{NATIVE_SUFFIX_BATCH_SCHEMA, NativeSuffixBatch};
use dusklight_worker_protocol::client::{BatchComplete, HelloResponse, WorkerClient};
use dusklight_worker_protocol::transport::ProcessTransport;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct NativeSuffixWorkerLaunch {
    pub executable: PathBuf,
    pub game_data: PathBuf,
    pub input_tape: PathBuf,
    pub milestone_program: PathBuf,
    pub working_directory: PathBuf,
    pub state_root: PathBuf,
    pub world_context_sha256: Digest,
    pub terminal: NativeTerminalBinding,
    pub initial_batch: PathBuf,
    pub initial_result: PathBuf,
    pub initial_winner_tape: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSuffixWorkerIdentity {
    pub executable_sha256: Digest,
    pub game_data_sha256: Digest,
    pub input_tape_sha256: Digest,
    pub milestone_program_sha256: Digest,
    pub world_context_sha256: Digest,
    pub source_frame: u64,
    pub source_boundary_fingerprint: String,
    pub checkpoint_validation_kind: String,
    pub checkpoint_validation_ticks: u64,
    pub maximum_ticks: u64,
    pub terminal: NativeTerminalBinding,
}

pub struct NativeSuffixWorkerSession {
    client: WorkerClient<ProcessTransport>,
    hello: HelloResponse,
    identity: NativeSuffixWorkerIdentity,
    terminal: NativeTerminalBinding,
}

struct PreparedLaunch {
    executable: PathBuf,
    working_directory: PathBuf,
    args: Vec<String>,
    batch: NativeSuffixBatch,
    result: PathBuf,
    identity: NativeSuffixWorkerIdentity,
    terminal: NativeTerminalBinding,
}

impl NativeSuffixWorkerSession {
    pub fn launch(
        config: &NativeSuffixWorkerLaunch,
    ) -> Result<(Self, ValidatedNativeSuffixBatch), NativeSuffixWorkerError> {
        let prepared = prepare_launch(config)?;
        let transport = ProcessTransport::spawn_in(
            &prepared.executable,
            &prepared.args,
            Some(&prepared.working_directory),
        )
        .map_err(worker_error)?;
        let mut client = WorkerClient::new(transport);
        let hello = client.handshake().map_err(worker_error)?.clone();
        if !hello.capabilities.persistent_control || !hello.capabilities.batch_run {
            return Err(worker_message(
                "native child does not advertise persistent suffix-batch capability",
            ));
        }
        let complete = client.await_initial_batch().map_err(worker_error)?;
        let validated = validate_completed_batch(
            &complete,
            &prepared.result,
            &prepared.batch,
            &prepared.terminal,
        )?;
        let session = Self {
            client,
            hello,
            identity: prepared.identity,
            terminal: prepared.terminal,
        };
        Ok((session, validated))
    }

    pub fn hello(&self) -> &HelloResponse {
        &self.hello
    }

    pub fn identity(&self) -> &NativeSuffixWorkerIdentity {
        &self.identity
    }

    pub fn run_batch(
        &mut self,
        batch_path: &Path,
        result_path: &Path,
        winner_tape_path: Option<&Path>,
    ) -> Result<ValidatedNativeSuffixBatch, NativeSuffixWorkerError> {
        let batch_path = canonical_file(batch_path, "suffix batch")?;
        let batch: NativeSuffixBatch =
            serde_json::from_slice(&fs::read(&batch_path).map_err(worker_error)?)
                .map_err(worker_error)?;
        validate_batch_identity(&batch, &self.identity)?;
        let result_path = prepare_new_result_output(result_path, "suffix result")?;
        let winner_tape_path = winner_tape_path
            .map(|path| prepare_new_output(path, "suffix winner tape"))
            .transpose()?;
        let complete = self
            .client
            .run_batch(
                path_text(&batch_path, "suffix batch")?,
                path_text(&result_path, "suffix result")?,
                winner_tape_path
                    .as_deref()
                    .map(|path| path_text(path, "suffix winner tape"))
                    .transpose()?,
            )
            .map_err(worker_error)?;
        validate_completed_batch(&complete, &result_path, &batch, &self.terminal)
    }

    pub fn shutdown(mut self) -> Result<(), NativeSuffixWorkerError> {
        self.client.shutdown().map_err(worker_error)
    }
}

fn prepare_launch(
    config: &NativeSuffixWorkerLaunch,
) -> Result<PreparedLaunch, NativeSuffixWorkerError> {
    if config.world_context_sha256 == Digest::ZERO
        || config.terminal.program_sha256 == Digest::ZERO
        || config.terminal.definition_sha256 == Digest::ZERO
        || config.terminal.goal.is_empty()
    {
        return Err(worker_message(
            "native suffix launch identities are incomplete",
        ));
    }
    let executable = canonical_file(&config.executable, "executable")?;
    let game_data = canonical_file(&config.game_data, "game data")?;
    let input_tape = canonical_file(&config.input_tape, "input tape")?;
    let milestone_program = canonical_file(&config.milestone_program, "milestone program")?;
    let working_directory = canonical_directory(&config.working_directory, "working directory")?;
    let batch_path = canonical_file(&config.initial_batch, "initial suffix batch")?;
    let batch_bytes = fs::read(&batch_path).map_err(worker_error)?;
    let batch: NativeSuffixBatch = serde_json::from_slice(&batch_bytes).map_err(worker_error)?;
    validate_batch_shape(&batch)?;

    let tape_bytes = fs::read(&input_tape).map_err(worker_error)?;
    let tape = InputTape::decode(&tape_bytes).map_err(worker_error)?.tape;
    if tape.boot != TapeBoot::Process
        || batch
            .source_frame
            .checked_add(batch.maximum_ticks)
            .is_none_or(|end| end > tape.frames.len())
        || batch
            .source_frame
            .checked_add(batch.checkpoint_validation.ticks)
            .is_none_or(|end| end > tape.frames.len())
    {
        return Err(worker_message(
            "native suffix source and horizon do not fit the absolute process-boot tape",
        ));
    }

    let program_bytes = fs::read(&milestone_program).map_err(worker_error)?;
    let decoded =
        dusklight_objectives::milestone_dsl::decode(&program_bytes).map_err(worker_error)?;
    let definition_index = decoded
        .program
        .definitions
        .iter()
        .position(|definition| definition.name == config.terminal.goal)
        .ok_or_else(|| worker_message("milestone program does not define the terminal goal"))?;
    if Digest(decoded.program_sha256) != config.terminal.program_sha256
        || Digest(decoded.definitions[definition_index].sha256) != config.terminal.definition_sha256
    {
        return Err(worker_message(
            "milestone program identities differ from the terminal binding",
        ));
    }

    let result = prepare_new_result_output(&config.initial_result, "initial suffix result")?;
    let winner_tape = config
        .initial_winner_tape
        .as_deref()
        .map(|path| prepare_new_output(path, "initial suffix winner tape"))
        .transpose()?;
    prepare_state_root(&config.state_root)?;
    let state_root = config.state_root.canonicalize().map_err(worker_error)?;
    let renderer_cache = state_root
        .parent()
        .unwrap_or(&state_root)
        .join("renderer-cache");
    fs::create_dir_all(&renderer_cache).map_err(worker_error)?;

    let game_data_sha256 = sha256_file(&game_data)?;
    let identity = NativeSuffixWorkerIdentity {
        executable_sha256: sha256_file(&executable)?,
        game_data_sha256,
        input_tape_sha256: sha256(&tape_bytes),
        milestone_program_sha256: sha256(&program_bytes),
        world_context_sha256: config.world_context_sha256,
        source_frame: batch.source_frame as u64,
        source_boundary_fingerprint: batch.source_boundary_fingerprint.clone(),
        checkpoint_validation_kind: batch.checkpoint_validation.kind.clone(),
        checkpoint_validation_ticks: batch.checkpoint_validation.ticks as u64,
        maximum_ticks: batch.maximum_ticks as u64,
        terminal: config.terminal.clone(),
    };
    let mut args = vec![
        "--automation-engine-worker".into(),
        "--headless".into(),
        "--dvd".into(),
        path_text(&game_data, "game data")?.into(),
        "--input-tape".into(),
        path_text(&input_tape, "input tape")?.into(),
        "--input-tape-end".into(),
        "release".into(),
        "--automation-data-root".into(),
        path_text(&state_root, "state root")?.into(),
        "--renderer-cache-root".into(),
        path_text(&renderer_cache, "renderer cache")?.into(),
        "--suffix-batch".into(),
        path_text(&batch_path, "initial suffix batch")?.into(),
        "--suffix-batch-result".into(),
        path_text(&result, "initial suffix result")?.into(),
        "--automation-game-data-sha256".into(),
        game_data_sha256.to_string(),
        "--automation-world-context-sha256".into(),
        config.world_context_sha256.to_string(),
        "--milestone-program".into(),
        path_text(&milestone_program, "milestone program")?.into(),
        "--milestones".into(),
        config.terminal.goal.clone(),
        "--milestone-goal".into(),
        config.terminal.goal.clone(),
        "--milestone-result".into(),
        path_text(&state_root.join("milestones.json"), "milestone result")?.into(),
    ];
    if let Some(winner_tape) = &winner_tape {
        args.push("--suffix-batch-winner-tape".into());
        args.push(path_text(winner_tape, "initial suffix winner tape")?.into());
    }
    for cvar in FIXED_AUTOMATION_CVARS {
        args.push("--cvar".into());
        args.push((*cvar).into());
    }
    Ok(PreparedLaunch {
        executable,
        working_directory,
        args,
        batch,
        result,
        identity,
        terminal: config.terminal.clone(),
    })
}

fn validate_completed_batch(
    complete: &BatchComplete,
    expected_result: &Path,
    batch: &NativeSuffixBatch,
    terminal: &NativeTerminalBinding,
) -> Result<ValidatedNativeSuffixBatch, NativeSuffixWorkerError> {
    let result_path = canonical_file(expected_result, "native suffix result")?;
    if Path::new(&complete.result) != result_path {
        return Err(worker_message(
            "engine worker returned a different suffix result path",
        ));
    }
    let result: NativeSuffixBatchResult =
        serde_json::from_slice(&fs::read(&result_path).map_err(worker_error)?)
            .map_err(worker_error)?;
    let validated = result
        .validate_against(batch, terminal)
        .map_err(worker_error)?;
    let episode_path = canonical_file(Path::new(&complete.episode_shard), "native episode shard")?;
    if Path::new(&complete.episode_shard) != episode_path
        || Path::new(&validated.episode_shard_path) != episode_path
    {
        return Err(worker_message(
            "engine worker response, suffix result, and episode shard paths differ",
        ));
    }
    Ok(validated)
}

fn validate_batch_identity(
    batch: &NativeSuffixBatch,
    identity: &NativeSuffixWorkerIdentity,
) -> Result<(), NativeSuffixWorkerError> {
    validate_batch_shape(batch)?;
    if batch.source_frame as u64 != identity.source_frame
        || batch.source_boundary_fingerprint != identity.source_boundary_fingerprint
        || batch.checkpoint_validation.kind != identity.checkpoint_validation_kind
        || batch.checkpoint_validation.ticks as u64 != identity.checkpoint_validation_ticks
        || batch.maximum_ticks as u64 != identity.maximum_ticks
    {
        return Err(worker_message(
            "next suffix batch differs from the authenticated session source",
        ));
    }
    Ok(())
}

fn validate_batch_shape(batch: &NativeSuffixBatch) -> Result<(), NativeSuffixWorkerError> {
    if batch.schema != NATIVE_SUFFIX_BATCH_SCHEMA
        || batch.candidates.is_empty()
        || batch.source_boundary_fingerprint.len() != 32
        || !batch
            .source_boundary_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || batch.maximum_ticks == 0
        || batch.checkpoint_validation.kind != "recorded_replay_window"
        || batch.checkpoint_validation.ticks == 0
    {
        return Err(worker_message("native suffix batch shape is invalid"));
    }
    Ok(())
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, NativeSuffixWorkerError> {
    let canonical = path.canonicalize().map_err(worker_error)?;
    if !canonical.is_file() {
        return Err(worker_message(format!("{label} is not a regular file")));
    }
    Ok(canonical)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, NativeSuffixWorkerError> {
    let canonical = path.canonicalize().map_err(worker_error)?;
    if !canonical.is_dir() {
        return Err(worker_message(format!("{label} is not a directory")));
    }
    Ok(canonical)
}

fn prepare_new_output(path: &Path, label: &str) -> Result<PathBuf, NativeSuffixWorkerError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(worker_error)?.join(path)
    };
    if absolute.exists() {
        return Err(worker_message(format!("{label} already exists")));
    }
    let parent = absolute
        .parent()
        .ok_or_else(|| worker_message(format!("{label} has no parent")))?;
    fs::create_dir_all(parent).map_err(worker_error)?;
    let parent = parent.canonicalize().map_err(worker_error)?;
    let name = absolute
        .file_name()
        .ok_or_else(|| worker_message(format!("{label} has no filename")))?;
    Ok(parent.join(name))
}

fn prepare_new_result_output(path: &Path, label: &str) -> Result<PathBuf, NativeSuffixWorkerError> {
    let output = prepare_new_output(path, label)?;
    let mut episode_name = output.as_os_str().to_os_string();
    episode_name.push(".episodes.dseps");
    if Path::new(&episode_name).exists() {
        return Err(worker_message(format!(
            "{label} episode shard already exists"
        )));
    }
    Ok(output)
}

fn prepare_state_root(path: &Path) -> Result<(), NativeSuffixWorkerError> {
    if path.exists() {
        if !path.is_dir() || fs::read_dir(path).map_err(worker_error)?.next().is_some() {
            return Err(worker_message(
                "native suffix state root must be new or empty",
            ));
        }
    } else {
        fs::create_dir_all(path).map_err(worker_error)?;
    }
    Ok(())
}

fn path_text<'a>(path: &'a Path, label: &str) -> Result<&'a str, NativeSuffixWorkerError> {
    path.to_str()
        .ok_or_else(|| worker_message(format!("{label} path is not UTF-8")))
}

fn sha256_file(path: &Path) -> Result<Digest, NativeSuffixWorkerError> {
    let mut file = File::open(path).map_err(worker_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(worker_error)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Debug)]
pub enum NativeSuffixWorkerError {
    Message(String),
}

fn worker_message(message: impl Into<String>) -> NativeSuffixWorkerError {
    NativeSuffixWorkerError::Message(message.into())
}

fn worker_error(error: impl fmt::Display) -> NativeSuffixWorkerError {
    NativeSuffixWorkerError::Message(error.to_string())
}

impl fmt::Display for NativeSuffixWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl Error for NativeSuffixWorkerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::InputFrame;
    use dusklight_search::search::MacroAction;
    use dusklight_search::suffix_batch::{NativeCheckpointValidation, NativeSuffixCandidate};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "dusklight-native-suffix-worker-{}-{}",
                std::process::id(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ));
            if path.exists() {
                fs::remove_dir_all(&path).unwrap();
            }
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> (TestRoot, NativeSuffixWorkerLaunch) {
        let root = TestRoot::new();
        let executable = root.0.join("Dusklight");
        let game_data = root.0.join("game.iso");
        let input_tape = root.0.join("full.tape");
        let milestone_program = root.0.join("goal.dmsp");
        let working_directory = root.0.join("cwd");
        let initial_batch = root.0.join("batch.json");
        fs::write(&executable, b"executable").unwrap();
        fs::write(&game_data, b"game-data").unwrap();
        fs::create_dir(&working_directory).unwrap();

        let tape = InputTape {
            frames: vec![InputFrame::default(); 3],
            ..InputTape::default()
        };
        fs::write(&input_tape, tape.encode().unwrap()).unwrap();

        let source =
            "milestones 1.7\nmilestone goal {\n  phase post_sim\n  when stage.room == 1\n}\n";
        let program = dusklight_objectives::milestone_dsl::parse(source).unwrap();
        let compiled = dusklight_objectives::milestone_dsl::compile(&program).unwrap();
        fs::write(&milestone_program, &compiled.bytes).unwrap();

        let batch = NativeSuffixBatch {
            schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
            source_frame: 1,
            source_boundary_fingerprint: "1".repeat(32),
            checkpoint_validation: NativeCheckpointValidation {
                kind: "recorded_replay_window".into(),
                ticks: 2,
            },
            maximum_ticks: 2,
            verify_state_hashes: false,
            candidates: vec![NativeSuffixCandidate {
                id: "candidate-0".into(),
                actions: vec![MacroAction::Neutral { frames: 2 }],
            }],
        };
        fs::write(&initial_batch, serde_json::to_vec_pretty(&batch).unwrap()).unwrap();

        let launch = NativeSuffixWorkerLaunch {
            executable,
            game_data,
            input_tape,
            milestone_program,
            working_directory,
            state_root: root.0.join("state"),
            world_context_sha256: Digest([4; 32]),
            terminal: NativeTerminalBinding {
                goal: "goal".into(),
                program_sha256: Digest(compiled.program_sha256),
                definition_sha256: Digest(compiled.definitions[0].sha256),
            },
            initial_batch,
            initial_result: root.0.join("result.json"),
            initial_winner_tape: Some(root.0.join("winner.tape")),
        };
        (root, launch)
    }

    #[test]
    fn launch_preflight_binds_every_persistent_source_identity() {
        let (_root, launch) = fixture();
        let prepared = prepare_launch(&launch).unwrap();

        assert_eq!(prepared.identity.source_frame, 1);
        assert_eq!(prepared.identity.maximum_ticks, 2);
        assert_eq!(prepared.identity.terminal, launch.terminal);
        assert_ne!(prepared.identity.executable_sha256, Digest::ZERO);
        assert_ne!(prepared.identity.game_data_sha256, Digest::ZERO);
        assert_ne!(prepared.identity.input_tape_sha256, Digest::ZERO);
        assert_ne!(prepared.identity.milestone_program_sha256, Digest::ZERO);
        for required in [
            "--automation-engine-worker",
            "--headless",
            "--suffix-batch",
            "--automation-game-data-sha256",
            "--automation-world-context-sha256",
            "--milestone-program",
            "--milestone-goal",
        ] {
            assert!(prepared.args.iter().any(|argument| argument == required));
        }
        for cvar in FIXED_AUTOMATION_CVARS {
            assert!(prepared.args.iter().any(|argument| argument == cvar));
        }
    }

    #[test]
    fn launch_preflight_rejects_terminal_and_horizon_drift() {
        let (_root, mut launch) = fixture();
        launch.terminal.definition_sha256 = Digest([9; 32]);
        assert!(prepare_launch(&launch).is_err());

        let (_root, mut launch) = fixture();
        let mut batch: NativeSuffixBatch =
            serde_json::from_slice(&fs::read(&launch.initial_batch).unwrap()).unwrap();
        batch.maximum_ticks = 3;
        fs::write(
            &launch.initial_batch,
            serde_json::to_vec_pretty(&batch).unwrap(),
        )
        .unwrap();
        assert!(prepare_launch(&launch).is_err());

        launch.world_context_sha256 = Digest::ZERO;
        assert!(prepare_launch(&launch).is_err());
    }
}
