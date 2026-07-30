use super::*;

const RECOVERY_ROOT_DIRECTORY: &str = "recovery-checkpoints";
const RECOVERY_MANIFEST_FILE: &str = "recovery.dtrc";
const RECOVERY_SCHEMA_V1: &str = "dusklight-native-tactic-recovery/v1";
const RECOVERY_MAGIC: &[u8; 8] = b"DSKTRC01";
const RECOVERY_VERSION: u16 = 1;
const RECOVERY_HEADER_BYTES: usize = 8 + 2 + 2 + 4 + 32;
const MAX_RECOVERY_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_RECOVERY_FILE_BYTES: usize = RECOVERY_HEADER_BYTES + MAX_RECOVERY_MANIFEST_BYTES;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticRecoveryPoint {
    schema: String,
    content_sha256: Digest,
    decision_index: u64,
    checkpoint_file: String,
    checkpoint_sha256: Digest,
    performance: NativeTacticSeedPerformance,
}

pub(super) struct LoadedTacticRecoveryPoint {
    pub(super) checkpoint_path: PathBuf,
    pub(super) performance: NativeTacticSeedPerformance,
}

pub(super) fn has_tactic_recovery_point(
    seed_root: &Path,
) -> Result<bool, NativeTacticRouteRunError> {
    let recovery_root = seed_root.join(RECOVERY_ROOT_DIRECTORY);
    let metadata = match fs::symlink_metadata(&recovery_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(route_error(error)),
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "native tactic recovery root is not a physical directory",
        ));
    }
    for entry in fs::read_dir(recovery_root).map_err(route_error)? {
        let entry = entry.map_err(route_error)?;
        if entry.file_type().map_err(route_error)?.is_dir()
            && entry
                .file_name()
                .to_str()
                .and_then(parse_recovery_directory_name)
                .is_some()
            && physical_recovery_manifest_exists(&entry.path())?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn persist_tactic_recovery_point(
    seed_root: &Path,
    campaign: &TacticQCampaign,
    content_store: &TacticQContentStore,
    performance: NativeTacticSeedPerformance,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    if performance.decisions != campaign.decision_index {
        return Err(route_message(
            "native tactic recovery performance is detached from the campaign",
        ));
    }
    let recovery_root = seed_root.join(RECOVERY_ROOT_DIRECTORY);
    fs::create_dir_all(&recovery_root).map_err(route_error)?;
    let final_directory = recovery_root.join(recovery_directory_name(campaign.decision_index));
    if final_directory.exists() {
        return Err(route_message("native tactic recovery point already exists"));
    }
    let partial_directory = recovery_root.join(format!(
        ".{}-{}.partial",
        recovery_directory_name(campaign.decision_index),
        std::process::id()
    ));
    if partial_directory.exists() {
        remove_recovery_directory(&recovery_root, &partial_directory)?;
    }
    fs::create_dir(&partial_directory).map_err(route_error)?;
    let (checkpoint_path, checkpoint) = campaign
        .write_checkpoint_with_content_store(&partial_directory, content_store)
        .map_err(route_error)?;
    let checkpoint_file = checkpoint_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| route_message("native tactic recovery checkpoint name is invalid"))?
        .to_owned();
    let mut recovery = NativeTacticRecoveryPoint {
        schema: RECOVERY_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        decision_index: campaign.decision_index,
        checkpoint_file,
        checkpoint_sha256: checkpoint.content_sha256,
        performance,
    };
    recovery.content_sha256 = recovery_digest(&recovery)?;
    let encoded = encode_recovery(&recovery)?;
    write_new(&partial_directory.join(RECOVERY_MANIFEST_FILE), &encoded)?;
    fs::rename(&partial_directory, &final_directory).map_err(route_error)?;
    sync_directory(&recovery_root)?;
    Ok(final_directory)
}

pub(super) fn load_tactic_recovery_point(
    seed_root: &Path,
    decision_index: u64,
) -> Result<LoadedTacticRecoveryPoint, NativeTacticRouteRunError> {
    let recovery_root = seed_root.join(RECOVERY_ROOT_DIRECTORY);
    let directory = recovery_root.join(recovery_directory_name(decision_index));
    let metadata = fs::symlink_metadata(&directory).map_err(|error| {
        route_message(format!(
            "native tactic recovery point is unavailable at {}: {error}",
            directory.display()
        ))
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "native tactic recovery point is not a physical directory",
        ));
    }
    let recovery = decode_recovery(&read_recovery_manifest(&directory)?)?;
    if recovery.decision_index != decision_index
        || recovery.performance.decisions != decision_index
        || !valid_checkpoint_file_name(&recovery.checkpoint_file)
    {
        return Err(route_message("native tactic recovery manifest is detached"));
    }
    let checkpoint_path = directory.join(&recovery.checkpoint_file);
    let checkpoint =
        TacticQCampaign::read_checkpoint_payload(&checkpoint_path).map_err(route_error)?;
    if checkpoint.decision_index != decision_index
        || checkpoint.content_sha256 != recovery.checkpoint_sha256
    {
        return Err(route_message(
            "native tactic recovery checkpoint is detached from its manifest",
        ));
    }
    Ok(LoadedTacticRecoveryPoint {
        checkpoint_path,
        performance: recovery.performance,
    })
}

pub(super) fn prune_tactic_recovery_points(
    seed_root: &Path,
    retain_decision_index: u64,
) -> Result<(), NativeTacticRouteRunError> {
    let recovery_root = seed_root.join(RECOVERY_ROOT_DIRECTORY);
    let metadata = fs::symlink_metadata(&recovery_root).map_err(route_error)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "native tactic recovery root is not a physical directory",
        ));
    }
    let retained = recovery_root.join(recovery_directory_name(retain_decision_index));
    if !retained.is_dir() {
        return Err(route_message(
            "native tactic retained recovery point is absent",
        ));
    }
    for entry in fs::read_dir(&recovery_root).map_err(route_error)? {
        let entry = entry.map_err(route_error)?;
        let path = entry.path();
        if path == retained {
            continue;
        }
        let file_type = entry.file_type().map_err(route_error)?;
        if !file_type.is_dir() || file_type.is_symlink() {
            return Err(route_message(
                "native tactic recovery root contains an unexpected entry",
            ));
        }
        remove_recovery_directory(&recovery_root, &path)?;
    }
    sync_directory(&recovery_root)
}

pub(super) fn prune_tactic_native_attempts(
    seed_root: &Path,
    completed_decisions: u64,
) -> Result<(), NativeTacticRouteRunError> {
    let native_root = seed_root.join("native");
    let metadata = match fs::symlink_metadata(&native_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(route_error(error)),
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "native tactic attempt root is not a physical directory",
        ));
    }
    for entry in fs::read_dir(&native_root).map_err(route_error)? {
        let entry = entry.map_err(route_error)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(route_error)?;
        let decision_index = entry
            .file_name()
            .to_str()
            .and_then(parse_recovery_directory_name)
            .ok_or_else(|| route_message("native tactic attempt root contains an invalid entry"))?;
        if !file_type.is_dir() || file_type.is_symlink() {
            return Err(route_message(
                "native tactic attempt root contains a non-physical decision",
            ));
        }
        if decision_index >= completed_decisions {
            remove_recovery_directory(&native_root, &path)?;
        }
    }
    sync_directory(&native_root)
}

fn remove_recovery_directory(
    recovery_root: &Path,
    directory: &Path,
) -> Result<(), NativeTacticRouteRunError> {
    let recovery_root = recovery_root.canonicalize().map_err(route_error)?;
    let directory = directory.canonicalize().map_err(route_error)?;
    if directory.parent() != Some(recovery_root.as_path()) || directory == recovery_root {
        return Err(route_message(
            "native tactic recovery deletion escaped its root",
        ));
    }
    fs::remove_dir_all(directory).map_err(route_error)
}

fn recovery_directory_name(decision_index: u64) -> String {
    format!("decision-{decision_index:06}")
}

fn parse_recovery_directory_name(name: &str) -> Option<u64> {
    let decision = name.strip_prefix("decision-")?.parse::<u64>().ok()?;
    (recovery_directory_name(decision) == name).then_some(decision)
}

fn valid_checkpoint_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && Path::new(name).file_name().and_then(|value| value.to_str()) == Some(name)
        && Path::new(name)
            .extension()
            .is_some_and(|extension| extension == TACTIC_Q_CHECKPOINT_EXTENSION)
}

fn physical_recovery_manifest_exists(directory: &Path) -> Result<bool, NativeTacticRouteRunError> {
    let path = directory.join(RECOVERY_MANIFEST_FILE);
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(route_message(
                    "native tactic recovery manifest must not be a symlink",
                ));
            }
            Ok(metadata.file_type().is_file())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(route_error(error)),
    }
}

fn read_recovery_manifest(directory: &Path) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let path = directory.join(RECOVERY_MANIFEST_FILE);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        route_message(format!(
            "native tactic recovery manifest is unavailable: {error}"
        ))
    })?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_RECOVERY_FILE_BYTES as u64
    {
        return Err(route_message(
            "native tactic recovery manifest is not a bounded physical file",
        ));
    }
    fs::read(path).map_err(route_error)
}

fn recovery_digest(
    recovery: &NativeTacticRecoveryPoint,
) -> Result<Digest, NativeTacticRouteRunError> {
    let mut unsigned = recovery.clone();
    unsigned.content_sha256 = Digest::ZERO;
    Ok(Digest(
        Sha256::digest(serde_cbor::to_vec(&unsigned).map_err(route_error)?).into(),
    ))
}

fn encode_recovery(
    recovery: &NativeTacticRecoveryPoint,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    if recovery.schema != RECOVERY_SCHEMA_V1
        || recovery.content_sha256 == Digest::ZERO
        || recovery.content_sha256 != recovery_digest(recovery)?
        || recovery.performance.schema != TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2
        || recovery.performance.decisions != recovery.decision_index
        || recovery.checkpoint_sha256 == Digest::ZERO
        || !valid_checkpoint_file_name(&recovery.checkpoint_file)
    {
        return Err(route_message("native tactic recovery point is invalid"));
    }
    let payload = serde_cbor::to_vec(recovery).map_err(route_error)?;
    if payload.len() > MAX_RECOVERY_MANIFEST_BYTES {
        return Err(route_message(
            "native tactic recovery manifest exceeds its bound",
        ));
    }
    let mut bytes = Vec::with_capacity(RECOVERY_HEADER_BYTES + payload.len());
    bytes.extend_from_slice(RECOVERY_MAGIC);
    bytes.extend_from_slice(&RECOVERY_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(
        &u32::try_from(payload.len())
            .map_err(route_error)?
            .to_le_bytes(),
    );
    bytes.extend_from_slice(&<[u8; 32]>::from(Sha256::digest(&payload)));
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode_recovery(bytes: &[u8]) -> Result<NativeTacticRecoveryPoint, NativeTacticRouteRunError> {
    if bytes.len() < RECOVERY_HEADER_BYTES
        || &bytes[..8] != RECOVERY_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice")) != RECOVERY_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message(
            "native tactic recovery manifest header is invalid",
        ));
    }
    let payload_len = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice")) as usize;
    if payload_len > MAX_RECOVERY_MANIFEST_BYTES
        || bytes.len() != RECOVERY_HEADER_BYTES.saturating_add(payload_len)
    {
        return Err(route_message(
            "native tactic recovery manifest length is invalid",
        ));
    }
    let expected: [u8; 32] = bytes[16..48].try_into().expect("fixed slice");
    let payload = &bytes[RECOVERY_HEADER_BYTES..];
    let actual: [u8; 32] = Sha256::digest(payload).into();
    if expected != actual {
        return Err(route_message(
            "native tactic recovery manifest digest is invalid",
        ));
    }
    let mut deserializer = serde_cbor::Deserializer::from_slice(payload);
    let recovery =
        NativeTacticRecoveryPoint::deserialize(&mut deserializer).map_err(route_error)?;
    deserializer.end().map_err(route_error)?;
    encode_recovery(&recovery)?;
    Ok(recovery)
}

fn sync_directory(path: &Path) -> Result<(), NativeTacticRouteRunError> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(route_error)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    use dusklight_learning::tactic_asset::{TacticAssetSource, TacticCatalogEntry};

    fn performance(decisions: u64) -> NativeTacticSeedPerformance {
        NativeTacticSeedPerformance {
            schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2.into(),
            decisions,
            useful_decisions: decisions,
            native_restore_accounting: NativeTacticRestoreAccounting::default(),
            timing: NativeTacticRouteTiming::default(),
        }
    }

    fn recovery() -> NativeTacticRecoveryPoint {
        let mut recovery = NativeTacticRecoveryPoint {
            schema: RECOVERY_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            decision_index: 7,
            checkpoint_file: format!(
                "tactic-q-{}.{}",
                Digest([2; 32]),
                TACTIC_Q_CHECKPOINT_EXTENSION
            ),
            checkpoint_sha256: Digest([2; 32]),
            performance: performance(7),
        };
        recovery.content_sha256 = recovery_digest(&recovery).unwrap();
        recovery
    }

    fn campaign() -> TacticQCampaign {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let facts = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let current = LearnerState::build(
            facts.clone(),
            &FactRegistry::canonical(),
            &catalog,
            &[],
            |_| true,
        )
        .unwrap();
        let mut campaign = TacticQCampaign::new(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            0,
            current,
            InputTape {
                frames: vec![InputFrame::default(); facts.tape_frame as usize],
                ..InputTape::default()
            },
            OptionValueConfig::default(),
            TacticExplorationConfig {
                seed: 17,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        campaign.bind_execution_authority(Digest([4; 32])).unwrap();
        campaign
    }

    #[test]
    fn binary_recovery_manifest_round_trips_and_rejects_tampering() {
        let recovery = recovery();
        let bytes = encode_recovery(&recovery).unwrap();
        assert_eq!(decode_recovery(&bytes).unwrap(), recovery);

        let mut tampered = bytes;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(decode_recovery(&tampered).is_err());
    }

    #[test]
    fn recovery_manifest_rejects_path_escape_and_resealed_performance_drift() {
        let mut recovery = recovery();
        recovery.checkpoint_file = "../outside.dtqz".into();
        recovery.content_sha256 = recovery_digest(&recovery).unwrap();
        assert!(encode_recovery(&recovery).is_err());

        let mut recovery = self::recovery();
        recovery.performance.decisions += 1;
        recovery.content_sha256 = recovery_digest(&recovery).unwrap();
        assert!(encode_recovery(&recovery).is_err());
    }

    #[test]
    fn recovery_point_round_trips_a_real_campaign_and_prunes_partial_work() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-native-tactic-recovery-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let campaign = campaign();
        let content_root = root.join("objects");
        let content_store = TacticQContentStore::initialize(&content_root).unwrap();
        persist_tactic_recovery_point(&root, &campaign, &content_store, performance(0)).unwrap();
        assert!(has_tactic_recovery_point(&root).unwrap());
        let loaded = load_tactic_recovery_point(&root, 0).unwrap();
        let checkpoint = TacticQCampaign::read_checkpoint_payload(&loaded.checkpoint_path).unwrap();
        assert_eq!(checkpoint.decision_index, 0);
        assert_eq!(loaded.performance, performance(0));

        let partial = root
            .join(RECOVERY_ROOT_DIRECTORY)
            .join(".decision-000001-crash.partial");
        fs::create_dir(&partial).unwrap();
        fs::write(partial.join("orphan"), b"orphan").unwrap();
        prune_tactic_recovery_points(&root, 0).unwrap();
        assert!(!partial.exists());
        assert!(load_tactic_recovery_point(&root, 0).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resume_prunes_only_native_attempts_without_a_committed_decision() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-native-tactic-attempt-prune-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let native = root.join("native");
        let committed = native.join("decision-000000");
        let partial = native.join("decision-000001");
        fs::create_dir_all(&committed).unwrap();
        fs::create_dir_all(&partial).unwrap();
        fs::write(committed.join("result"), b"committed").unwrap();
        fs::write(partial.join("result"), b"partial").unwrap();

        prune_tactic_native_attempts(&root, 1).unwrap();

        assert!(committed.join("result").is_file());
        assert!(!partial.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_manifest_reader_rejects_oversized_files_before_decode() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-native-tactic-recovery-bound-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let directory = root
            .join(RECOVERY_ROOT_DIRECTORY)
            .join(recovery_directory_name(0));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(RECOVERY_MANIFEST_FILE),
            vec![0; MAX_RECOVERY_FILE_BYTES + 1],
        )
        .unwrap();
        assert!(read_recovery_manifest(&directory).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
