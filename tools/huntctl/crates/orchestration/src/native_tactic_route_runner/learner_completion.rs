use super::*;

pub(super) const NATIVE_TACTIC_LEARNER_COMPLETION_FILE: &str = "campaign-learner-complete.dtlc";
const NATIVE_TACTIC_LEARNER_COMPLETION_SCHEMA_V1: &str =
    "dusklight-native-tactic-learner-completion/v1";
const LEARNER_COMPLETION_MAGIC: &[u8; 8] = b"DSKTLC01";
const LEARNER_COMPLETION_VERSION: u16 = 1;
const LEARNER_COMPLETION_HEADER_BYTES: usize = 8 + 2 + 2 + 4 + 32;
const MAXIMUM_LEARNER_COMPLETION_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

/// Compact authority for reporting a completed campaign's replay and learner
/// state. It is published only after the full journals, content objects, and
/// latest learner snapshot have passed their ordinary validation path.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeTacticLearnerCompletion {
    schema: String,
    content_sha256: Digest,
    replay_identity_sha256: Digest,
    replay_journal_sha256: Digest,
    learner_head_journal_sha256: Digest,
    replay_revision: u64,
    replay_snapshot_sha256: Digest,
    replay_rows: u64,
    useful_training_transitions: u64,
    censored_training_transitions: u64,
    replay_admission: TacticReplayAdmissionMetrics,
    learner_updates: u64,
    model_snapshots_published: u64,
    latest_snapshot_sha256: Digest,
    latest_manifest: TacticQLearnerSnapshot,
}

impl NativeTacticLearnerCompletion {
    fn build(
        output_root: &Path,
        identity: &TacticReplayControlPlaneIdentity,
        snapshot: &CampaignLearnerFinalizationSnapshot,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let (replay_path, learner_head_path) = authority_paths(output_root);
        let mut completion = Self {
            schema: NATIVE_TACTIC_LEARNER_COMPLETION_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            replay_identity_sha256: identity.content_sha256().map_err(route_error)?,
            replay_journal_sha256: hash_physical_file(&replay_path)?,
            learner_head_journal_sha256: hash_physical_file(&learner_head_path)?,
            replay_revision: snapshot.replay_snapshot.revision,
            replay_snapshot_sha256: snapshot.replay_snapshot.sha256,
            replay_rows: snapshot.replay_rows,
            useful_training_transitions: snapshot.useful_training_transitions,
            censored_training_transitions: snapshot.censored_training_transitions,
            replay_admission: snapshot.replay_admission,
            learner_updates: snapshot.learner_updates,
            model_snapshots_published: snapshot.model_snapshots_published,
            latest_snapshot_sha256: snapshot.latest_snapshot_sha256,
            latest_manifest: snapshot.latest_manifest.clone(),
        };
        completion.content_sha256 = completion.compute_content_sha256()?;
        completion.validate(output_root, identity)?;
        Ok(completion)
    }

    pub(super) fn read_and_validate(
        path: &Path,
        output_root: &Path,
        identity: &TacticReplayControlPlaneIdentity,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let bytes = fs::read(path).map_err(route_error)?;
        let completion = decode_learner_completion(&bytes)?;
        completion.validate(output_root, identity)?;
        Ok(completion)
    }

    fn validate(
        &self,
        output_root: &Path,
        identity: &TacticReplayControlPlaneIdentity,
    ) -> Result<(), NativeTacticRouteRunError> {
        let (replay_path, learner_head_path) = authority_paths(output_root);
        self.latest_manifest.validate().map_err(route_error)?;
        if self.schema != NATIVE_TACTIC_LEARNER_COMPLETION_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.compute_content_sha256()? != self.content_sha256
            || self.replay_identity_sha256 != identity.content_sha256().map_err(route_error)?
            || self.replay_journal_sha256 != hash_physical_file(&replay_path)?
            || self.learner_head_journal_sha256 != hash_physical_file(&learner_head_path)?
            || self.replay_snapshot_sha256 == Digest::ZERO
            || self.replay_rows != self.replay_revision
            || self.useful_training_transitions > self.replay_rows
            || self.censored_training_transitions > self.replay_rows
            || self.latest_snapshot_sha256 == Digest::ZERO
            || self.latest_manifest.content_sha256()? != self.latest_snapshot_sha256
            || self.latest_manifest.execution_authority_sha256
                != identity.execution_authority_sha256
            || self.latest_manifest.feature_schema_sha256 != identity.feature_schema_sha256
            || self.latest_manifest.objective_sha256 != identity.objective_sha256
            || self.latest_manifest.root_checkpoint_sha256 != identity.root_checkpoint_sha256
            || self.latest_manifest.training_replay_rows != self.replay_revision
            || self.latest_manifest.model_revision != self.learner_updates
            || self.model_snapshots_published == 0
        {
            return Err(route_message(
                "native tactic learner completion projection is detached",
            ));
        }
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(digest_bytes(
            &serde_cbor::to_vec(&unsigned).map_err(route_error)?,
        ))
    }

    pub(super) fn replay_rows(&self) -> u64 {
        self.replay_rows
    }

    pub(super) fn finalization_snapshot(&self) -> CampaignLearnerFinalizationSnapshot {
        CampaignLearnerFinalizationSnapshot {
            replay_snapshot: TacticReplaySnapshotVersion {
                revision: self.replay_revision,
                sha256: self.replay_snapshot_sha256,
            },
            replay_rows: self.replay_rows,
            useful_training_transitions: self.useful_training_transitions,
            censored_training_transitions: self.censored_training_transitions,
            replay_admission: self.replay_admission,
            learner_metrics: CampaignLearnerUpdateMetrics::default(),
            learner_updates: self.learner_updates,
            model_snapshots_published: self.model_snapshots_published,
            latest_snapshot_sha256: self.latest_snapshot_sha256,
            latest_manifest: self.latest_manifest.clone(),
        }
    }
}

pub(super) fn publish_learner_completion(
    output_root: &Path,
    identity: &TacticReplayControlPlaneIdentity,
    snapshot: &CampaignLearnerFinalizationSnapshot,
) -> Result<(), NativeTacticRouteRunError> {
    let path = output_root.join(NATIVE_TACTIC_LEARNER_COMPLETION_FILE);
    let expected = NativeTacticLearnerCompletion::build(output_root, identity, snapshot)?;
    if path.is_file() {
        let existing =
            NativeTacticLearnerCompletion::read_and_validate(&path, output_root, identity)?;
        if existing != expected {
            return Err(route_message(
                "native tactic learner completion changed after publication",
            ));
        }
        return Ok(());
    }
    publish_new_atomic(&path, &encode_learner_completion(&expected)?)
}

fn authority_paths(output_root: &Path) -> (PathBuf, PathBuf) {
    let replay = output_root.join(NATIVE_TACTIC_REPLAY_CONTROL_PLANE_FILE);
    let mut learner_head = replay.as_os_str().to_os_string();
    learner_head.push(".learner-head");
    (replay, PathBuf::from(learner_head))
}

fn encode_learner_completion(
    completion: &NativeTacticLearnerCompletion,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let payload = serde_cbor::to_vec(completion).map_err(route_error)?;
    if payload.len() > MAXIMUM_LEARNER_COMPLETION_PAYLOAD_BYTES {
        return Err(route_message(
            "native tactic learner completion projection exceeds its bound",
        ));
    }
    let mut bytes = Vec::with_capacity(LEARNER_COMPLETION_HEADER_BYTES + payload.len());
    bytes.extend_from_slice(LEARNER_COMPLETION_MAGIC);
    bytes.extend_from_slice(&LEARNER_COMPLETION_VERSION.to_le_bytes());
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

fn decode_learner_completion(
    bytes: &[u8],
) -> Result<NativeTacticLearnerCompletion, NativeTacticRouteRunError> {
    if bytes.len() < LEARNER_COMPLETION_HEADER_BYTES
        || &bytes[..8] != LEARNER_COMPLETION_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"))
            != LEARNER_COMPLETION_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message(
            "native tactic learner completion header is invalid",
        ));
    }
    let payload_len = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice")) as usize;
    if payload_len > MAXIMUM_LEARNER_COMPLETION_PAYLOAD_BYTES
        || bytes.len()
            != LEARNER_COMPLETION_HEADER_BYTES
                .checked_add(payload_len)
                .unwrap_or(usize::MAX)
    {
        return Err(route_message(
            "native tactic learner completion length is invalid",
        ));
    }
    let expected: [u8; 32] = bytes[16..48].try_into().expect("fixed slice");
    let payload = &bytes[LEARNER_COMPLETION_HEADER_BYTES..];
    if expected != <[u8; 32]>::from(Sha256::digest(payload)) {
        return Err(route_message(
            "native tactic learner completion payload digest is invalid",
        ));
    }
    serde_cbor::from_slice(payload).map_err(route_error)
}

fn hash_physical_file(path: &Path) -> Result<Digest, NativeTacticRouteRunError> {
    let metadata = fs::symlink_metadata(path).map_err(route_error)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "native tactic learner completion artifact is not a physical file",
        ));
    }
    let mut file = fs::File::open(path).map_err(route_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(route_error)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Digest(hasher.finalize().into()))
}

fn digest_bytes(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion() -> NativeTacticLearnerCompletion {
        let latest_manifest = TacticQLearnerSnapshot {
            schema: TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V4.into(),
            kind: TacticQLearnerSnapshotKind::Learned,
            value_treatment: TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
            execution_authority_sha256: Digest([1; 32]),
            feature_schema_sha256: Digest([2; 32]),
            objective_sha256: Digest([3; 32]),
            root_checkpoint_sha256: Digest([4; 32]),
            training_replay_rows: 8,
            training_replay_sha256: Digest([5; 32]),
            model_revision: 2,
            model_config: route_option_value_config(Digest([1; 32])),
            model_sha256: Some(Digest([6; 32])),
            goal_reachability_calibration: None,
        };
        let mut completion = NativeTacticLearnerCompletion {
            schema: NATIVE_TACTIC_LEARNER_COMPLETION_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            replay_identity_sha256: Digest([7; 32]),
            replay_journal_sha256: Digest([8; 32]),
            learner_head_journal_sha256: Digest([9; 32]),
            replay_revision: 8,
            replay_snapshot_sha256: Digest([10; 32]),
            replay_rows: 8,
            useful_training_transitions: 5,
            censored_training_transitions: 2,
            replay_admission: TacticReplayAdmissionMetrics::default(),
            learner_updates: 2,
            model_snapshots_published: 2,
            latest_snapshot_sha256: latest_manifest.content_sha256().unwrap(),
            latest_manifest,
        };
        completion.content_sha256 = completion.compute_content_sha256().unwrap();
        completion
    }

    #[test]
    fn binary_learner_completion_round_trips_and_rejects_corruption() {
        let completion = completion();
        let encoded = encode_learner_completion(&completion).unwrap();
        assert_eq!(decode_learner_completion(&encoded).unwrap(), completion);

        let mut corrupt = encoded;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode_learner_completion(&corrupt).is_err());
    }

    #[test]
    fn learner_completion_identity_covers_replay_projection() {
        let completion = completion();
        let mut detached = completion.clone();
        detached.replay_snapshot_sha256 = Digest([11; 32]);
        assert_ne!(
            detached.compute_content_sha256().unwrap(),
            completion.content_sha256
        );
    }
}
