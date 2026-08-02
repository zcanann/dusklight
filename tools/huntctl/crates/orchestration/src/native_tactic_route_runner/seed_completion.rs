use super::*;

pub(super) const NATIVE_TACTIC_SEED_COMPLETION_FILE: &str = "seed-complete.dtsc";
const NATIVE_TACTIC_SEED_COMPLETION_SCHEMA_V1: &str = "dusklight-native-tactic-seed-completion/v1";
const SEED_COMPLETION_MAGIC: &[u8; 8] = b"DSKTSC01";
const SEED_COMPLETION_VERSION: u16 = 1;
const SEED_COMPLETION_HEADER_BYTES: usize = 8 + 2 + 2 + 4 + 32;
const MAXIMUM_SEED_COMPLETION_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct NativeTacticSeedCompletionProjection {
    pub(super) final_checkpoint_content_sha256: Digest,
    pub(super) feature_schema_sha256: Digest,
    pub(super) objective_sha256: Digest,
    pub(super) root_checkpoint_sha256: Digest,
    pub(super) root_facts: FactSnapshot,
    pub(super) useful_graph_expansion_identities: Vec<Digest>,
}

/// Compact durable authority projected while the complete seed graph is still
/// validated in memory. It makes all-seed-complete preflight proportional to
/// the reporting facts it consumes rather than to every historical graph
/// journal record. Legacy seeds without this seal retain the full checkpoint
/// validation path.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeTacticSeedCompletion {
    schema: String,
    content_sha256: Digest,
    execution_plan_sha256: Digest,
    seed: u64,
    seed_result_sha256: Digest,
    final_checkpoint_path: String,
    final_checkpoint_file_sha256: Digest,
    final_checkpoint_content_sha256: Digest,
    lease_journal_sha256: Digest,
    feature_schema_sha256: Digest,
    objective_sha256: Digest,
    root_checkpoint_sha256: Digest,
    root_facts: FactSnapshot,
    state_graph_sha256: Digest,
    useful_graph_expansion_identities: Vec<Digest>,
}

impl NativeTacticSeedCompletion {
    fn build(
        seed_root: &Path,
        result: &NativeTacticSeedResult,
        result_bytes: &[u8],
        projection: &NativeTacticSeedCompletionProjection,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let useful = CampaignUsefulGraphExpansionSet::from_identities(
            projection.useful_graph_expansion_identities.clone(),
        )?;
        let final_checkpoint_path = Path::new(&result.final_checkpoint);
        let lease_journal_path = seed_root.join(NATIVE_TACTIC_LEASE_JOURNAL_FILE);
        let mut completion = Self {
            schema: NATIVE_TACTIC_SEED_COMPLETION_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            execution_plan_sha256: result.execution_plan_sha256,
            seed: result.seed,
            seed_result_sha256: digest_bytes(result_bytes),
            final_checkpoint_path: result.final_checkpoint.clone(),
            final_checkpoint_file_sha256: hash_physical_file(final_checkpoint_path)?,
            final_checkpoint_content_sha256: projection.final_checkpoint_content_sha256,
            lease_journal_sha256: hash_physical_file(&lease_journal_path)?,
            feature_schema_sha256: projection.feature_schema_sha256,
            objective_sha256: projection.objective_sha256,
            root_checkpoint_sha256: projection.root_checkpoint_sha256,
            root_facts: projection.root_facts.clone(),
            state_graph_sha256: result.state_graph_sha256,
            useful_graph_expansion_identities: projection.useful_graph_expansion_identities.clone(),
        };
        completion.content_sha256 = completion.compute_content_sha256()?;
        completion.validate_projection(result, result_bytes, seed_root, useful)?;
        Ok(completion)
    }

    pub(super) fn read_and_validate(
        path: &Path,
        seed_root: &Path,
        result: &NativeTacticSeedResult,
        result_bytes: &[u8],
    ) -> Result<Self, NativeTacticRouteRunError> {
        let bytes = fs::read(path).map_err(route_error)?;
        let completion = decode_seed_completion(&bytes)?;
        let useful = CampaignUsefulGraphExpansionSet::from_identities(
            completion.useful_graph_expansion_identities.clone(),
        )?;
        completion.validate_projection(result, result_bytes, seed_root, useful)?;
        Ok(completion)
    }

    fn validate_projection(
        &self,
        result: &NativeTacticSeedResult,
        result_bytes: &[u8],
        seed_root: &Path,
        useful: CampaignUsefulGraphExpansionSet,
    ) -> Result<(), NativeTacticRouteRunError> {
        self.root_facts.validate().map_err(route_error)?;
        let graph_metrics = result
            .graph_metrics
            .as_ref()
            .ok_or_else(|| route_message("sealed tactic seed has no graph metrics"))?;
        if self.schema != NATIVE_TACTIC_SEED_COMPLETION_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.compute_content_sha256()? != self.content_sha256
            || self.execution_plan_sha256 != result.execution_plan_sha256
            || self.seed != result.seed
            || self.seed_result_sha256 != digest_bytes(result_bytes)
            || self.final_checkpoint_path != result.final_checkpoint
            || self.final_checkpoint_file_sha256
                != hash_physical_file(Path::new(&result.final_checkpoint))?
            || self.final_checkpoint_content_sha256 == Digest::ZERO
            || self.lease_journal_sha256
                != hash_physical_file(&seed_root.join(NATIVE_TACTIC_LEASE_JOURNAL_FILE))?
            || self.feature_schema_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.root_checkpoint_sha256 == Digest::ZERO
            || self.state_graph_sha256 != result.state_graph_sha256
            || self.state_graph_sha256 != graph_metrics.graph.graph_sha256
            || useful.count()? != result.unique_useful_graph_expansions
            || result.useful_graph_expansion_set_sha256 == Digest::ZERO
        {
            return Err(route_message(
                "native tactic seed completion projection is detached",
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

    pub(super) fn root_facts(&self) -> &FactSnapshot {
        &self.root_facts
    }

    pub(super) fn feature_schema_sha256(&self) -> Digest {
        self.feature_schema_sha256
    }

    pub(super) fn objective_sha256(&self) -> Digest {
        self.objective_sha256
    }

    pub(super) fn root_checkpoint_sha256(&self) -> Digest {
        self.root_checkpoint_sha256
    }

    pub(super) fn useful_graph_expansions(
        &self,
    ) -> Result<CampaignUsefulGraphExpansionSet, NativeTacticRouteRunError> {
        CampaignUsefulGraphExpansionSet::from_identities(
            self.useful_graph_expansion_identities.clone(),
        )
    }
}

pub(super) fn publish_seed_completion(
    seed_root: &Path,
    result: &NativeTacticSeedResult,
    result_bytes: &[u8],
    projection: &NativeTacticSeedCompletionProjection,
) -> Result<(), NativeTacticRouteRunError> {
    let completion =
        NativeTacticSeedCompletion::build(seed_root, result, result_bytes, projection)?;
    publish_new_atomic(
        &seed_root.join(NATIVE_TACTIC_SEED_COMPLETION_FILE),
        &encode_seed_completion(&completion)?,
    )
}

fn encode_seed_completion(
    completion: &NativeTacticSeedCompletion,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let payload = serde_cbor::to_vec(completion).map_err(route_error)?;
    if payload.len() > MAXIMUM_SEED_COMPLETION_PAYLOAD_BYTES {
        return Err(route_message(
            "native tactic seed completion projection exceeds its bound",
        ));
    }
    let mut bytes = Vec::with_capacity(SEED_COMPLETION_HEADER_BYTES + payload.len());
    bytes.extend_from_slice(SEED_COMPLETION_MAGIC);
    bytes.extend_from_slice(&SEED_COMPLETION_VERSION.to_le_bytes());
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

fn decode_seed_completion(
    bytes: &[u8],
) -> Result<NativeTacticSeedCompletion, NativeTacticRouteRunError> {
    if bytes.len() < SEED_COMPLETION_HEADER_BYTES
        || &bytes[..8] != SEED_COMPLETION_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"))
            != SEED_COMPLETION_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message(
            "native tactic seed completion header is invalid",
        ));
    }
    let payload_len = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice")) as usize;
    if payload_len > MAXIMUM_SEED_COMPLETION_PAYLOAD_BYTES
        || bytes.len()
            != SEED_COMPLETION_HEADER_BYTES
                .checked_add(payload_len)
                .unwrap_or(usize::MAX)
    {
        return Err(route_message(
            "native tactic seed completion length is invalid",
        ));
    }
    let expected: [u8; 32] = bytes[16..48].try_into().expect("fixed slice");
    let payload = &bytes[SEED_COMPLETION_HEADER_BYTES..];
    if expected != <[u8; 32]>::from(Sha256::digest(payload)) {
        return Err(route_message(
            "native tactic seed completion payload digest is invalid",
        ));
    }
    let completion: NativeTacticSeedCompletion =
        serde_cbor::from_slice(payload).map_err(route_error)?;
    Ok(completion)
}

fn hash_physical_file(path: &Path) -> Result<Digest, NativeTacticRouteRunError> {
    let metadata = fs::symlink_metadata(path).map_err(route_error)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "native tactic seed completion artifact is not a physical file",
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

    fn completion() -> NativeTacticSeedCompletion {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let native = &shard.episodes[0].steps[0];
        let root_facts =
            FactSnapshot::from_native_learning(&native.pre_input, &[], None, Vec::new()).unwrap();
        let mut completion = NativeTacticSeedCompletion {
            schema: NATIVE_TACTIC_SEED_COMPLETION_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            execution_plan_sha256: Digest([1; 32]),
            seed: 7,
            seed_result_sha256: Digest([2; 32]),
            final_checkpoint_path: "checkpoint.dtqz".into(),
            final_checkpoint_file_sha256: Digest([3; 32]),
            final_checkpoint_content_sha256: Digest([4; 32]),
            lease_journal_sha256: Digest([5; 32]),
            feature_schema_sha256: Digest([6; 32]),
            objective_sha256: Digest([7; 32]),
            root_checkpoint_sha256: Digest([8; 32]),
            root_facts,
            state_graph_sha256: Digest([9; 32]),
            useful_graph_expansion_identities: vec![Digest([10; 32]), Digest([11; 32])],
        };
        completion.content_sha256 = completion.compute_content_sha256().unwrap();
        completion
    }

    #[test]
    fn binary_seed_completion_round_trips_and_rejects_corruption() {
        let completion = completion();
        let encoded = encode_seed_completion(&completion).unwrap();
        assert_eq!(decode_seed_completion(&encoded).unwrap(), completion);

        let mut corrupt = encoded;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode_seed_completion(&corrupt).is_err());
    }

    #[test]
    fn seed_completion_identity_covers_projection_fields() {
        let completion = completion();
        let mut detached = completion.clone();
        detached.state_graph_sha256 = Digest([12; 32]);
        assert_ne!(
            detached.compute_content_sha256().unwrap(),
            completion.content_sha256
        );
    }
}
