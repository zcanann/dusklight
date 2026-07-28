//! Decode shard headers, metadata, compressed blocks, and content identity.

use super::*;

impl NativeEpisodeShard {
    pub fn read(path: impl AsRef<Path>) -> Result<Self, NativeEpisodeShardError> {
        let bytes =
            fs::read(path).map_err(|error| NativeEpisodeShardError::new(error.to_string()))?;
        Self::decode(&bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, NativeEpisodeShardError> {
        if bytes.len() < HEADER_SIZE || &bytes[..8] != MAGIC {
            return Err(NativeEpisodeShardError::new(
                "invalid native episode shard magic",
            ));
        }
        let mut header = Reader::new(bytes);
        header.bytes(8)?;
        let shard_version = header.u16()?;
        if !matches!(shard_version, VERSION_V1 | VERSION_V2 | VERSION_V3)
            || usize::from(header.u16()?) != HEADER_SIZE
        {
            return Err(NativeEpisodeShardError::new(
                "unsupported native episode shard version",
            ));
        }
        let flags = header.u32()?;
        let episode_count = usize::try_from(header.u32()?)
            .map_err(|_| NativeEpisodeShardError::new("episode count overflow"))?;
        if flags != COMPLETE || !(1..=MAX_EPISODES).contains(&episode_count) {
            return Err(NativeEpisodeShardError::new(
                "incomplete or oversized native episode shard",
            ));
        }
        let observation_version = header.u16()?;
        if !matches!(
            observation_version,
            OBSERVATION_VERSION_V2
                | OBSERVATION_VERSION_V3
                | OBSERVATION_VERSION_V4
                | OBSERVATION_VERSION_V5
                | OBSERVATION_VERSION_V6
                | OBSERVATION_VERSION_V7
                | OBSERVATION_VERSION_V8
                | OBSERVATION_VERSION_V9
                | OBSERVATION_VERSION_V10
                | OBSERVATION_VERSION_V11
                | OBSERVATION_VERSION_V12
                | OBSERVATION_VERSION_V13
                | OBSERVATION_VERSION_V14
                | OBSERVATION_VERSION_V15
                | OBSERVATION_VERSION_V16
                | OBSERVATION_VERSION_V17
                | OBSERVATION_VERSION_V18
                | OBSERVATION_VERSION_V19
                | OBSERVATION_VERSION_V20
                | OBSERVATION_VERSION_V21
                | OBSERVATION_VERSION_V22
                | OBSERVATION_VERSION_V23
                | OBSERVATION_VERSION_V24
                | OBSERVATION_VERSION_V25
                | OBSERVATION_VERSION_V26
                | OBSERVATION_VERSION_V27
                | OBSERVATION_VERSION_V28
                | OBSERVATION_VERSION_V29
        ) || header.u16()? != ACTION_VERSION
        {
            return Err(NativeEpisodeShardError::new(
                "unsupported observation or action schema",
            ));
        }
        let source_frame = header.u64()?;
        let maximum_ticks = header.u32()?;
        if maximum_ticks == 0 || maximum_ticks as usize > MAX_TICKS || header.u32()? != 0 {
            return Err(NativeEpisodeShardError::new("invalid shard tick bound"));
        }
        let metadata_offset = header.usize_u64()?;
        let metadata_length = header.usize_u64()?;
        let payload_offset = header.usize_u64()?;
        let payload_length = header.usize_u64()?;
        let expected_uncompressed = header.u64()?;
        let expected_compressed = header.u64()?;
        if metadata_offset != HEADER_SIZE
            || payload_offset
                != metadata_offset
                    .checked_add(metadata_length)
                    .ok_or_else(|| NativeEpisodeShardError::new("metadata range overflow"))?
            || payload_offset.checked_add(payload_length) != Some(bytes.len())
            || header
                .bytes(HEADER_SIZE - 88)?
                .iter()
                .any(|byte| *byte != 0)
            || expected_uncompressed as usize > MAX_EXPANDED_BYTES
        {
            return Err(NativeEpisodeShardError::new(
                "noncanonical native episode shard layout",
            ));
        }
        let metadata = decode_metadata(
            &bytes[metadata_offset..payload_offset],
            shard_version,
            observation_version,
        )?;
        let mut payload = Reader::new(&bytes[payload_offset..]);
        let mut episodes = Vec::with_capacity(episode_count);
        let mut uncompressed_total = 0_u64;
        let mut compressed_total = 0_u64;
        for _ in 0..episode_count {
            let (episode, expanded_size, compressed_size) = decode_episode(
                &mut payload,
                maximum_ticks,
                source_frame,
                observation_version,
            )?;
            uncompressed_total = uncompressed_total
                .checked_add(expanded_size)
                .ok_or_else(|| NativeEpisodeShardError::new("uncompressed byte total overflow"))?;
            compressed_total = compressed_total
                .checked_add(compressed_size)
                .ok_or_else(|| NativeEpisodeShardError::new("compressed byte total overflow"))?;
            episodes.push(episode);
        }
        if !payload.done()
            || uncompressed_total != expected_uncompressed
            || compressed_total != expected_compressed
        {
            return Err(NativeEpisodeShardError::new(
                "native episode shard byte totals disagree",
            ));
        }
        Ok(Self {
            content_sha256: Digest(Sha256::digest(bytes).into()),
            source_frame,
            maximum_ticks,
            metadata,
            episodes,
            uncompressed_bytes: uncompressed_total,
            compressed_bytes: compressed_total,
        })
    }

    /// Fail closed unless this shard was produced for the supplied exact
    /// authored predicate program and definition.
    pub fn verify_authored_objective(
        &self,
        program_sha256: &str,
        definition_sha256: &str,
    ) -> Result<(), NativeEpisodeShardError> {
        let expected = authored_milestone_objective_identity(program_sha256, definition_sha256)?;
        if self.metadata.objective_identity != expected {
            return Err(NativeEpisodeShardError::new(format!(
                "native episode objective identity {} does not match authored milestone {}",
                self.metadata.objective_identity, expected
            )));
        }
        Ok(())
    }
}

pub(super) fn decode_metadata(
    bytes: &[u8],
    shard_version: u16,
    observation_version: u16,
) -> Result<NativeEpisodeShardMetadata, NativeEpisodeShardError> {
    let mut reader = Reader::new(bytes);
    let expected_field_count = match shard_version {
        VERSION_V1 => 12,
        VERSION_V2 => 15,
        VERSION_V3 => 21,
        _ => unreachable!("shard version was validated by the header decoder"),
    };
    if usize::from(reader.u16()?) != expected_field_count {
        return Err(NativeEpisodeShardError::new(
            "unsupported shard metadata field count",
        ));
    }
    let mut fields = Vec::with_capacity(expected_field_count);
    for _ in 0..expected_field_count {
        fields.push(reader.string16()?);
    }
    let expected_observation_schema = match observation_version {
        OBSERVATION_VERSION_V2 => LEARNING_OBSERVATION_SCHEMA_V2,
        OBSERVATION_VERSION_V3 => LEARNING_OBSERVATION_SCHEMA_V3,
        OBSERVATION_VERSION_V4 => LEARNING_OBSERVATION_SCHEMA_V4,
        OBSERVATION_VERSION_V5 => LEARNING_OBSERVATION_SCHEMA_V5,
        OBSERVATION_VERSION_V6 => LEARNING_OBSERVATION_SCHEMA_V6,
        OBSERVATION_VERSION_V7 => LEARNING_OBSERVATION_SCHEMA_V7,
        OBSERVATION_VERSION_V8 => LEARNING_OBSERVATION_SCHEMA_V8,
        OBSERVATION_VERSION_V9 => LEARNING_OBSERVATION_SCHEMA_V9,
        OBSERVATION_VERSION_V10 => LEARNING_OBSERVATION_SCHEMA_V10,
        OBSERVATION_VERSION_V11 => LEARNING_OBSERVATION_SCHEMA_V11,
        OBSERVATION_VERSION_V12 => LEARNING_OBSERVATION_SCHEMA_V12,
        OBSERVATION_VERSION_V13 => LEARNING_OBSERVATION_SCHEMA_V13,
        OBSERVATION_VERSION_V14 => LEARNING_OBSERVATION_SCHEMA_V14,
        OBSERVATION_VERSION_V15 => LEARNING_OBSERVATION_SCHEMA_V15,
        OBSERVATION_VERSION_V16 => LEARNING_OBSERVATION_SCHEMA_V16,
        OBSERVATION_VERSION_V17 => LEARNING_OBSERVATION_SCHEMA_V17,
        OBSERVATION_VERSION_V18 => LEARNING_OBSERVATION_SCHEMA_V18,
        OBSERVATION_VERSION_V19 => LEARNING_OBSERVATION_SCHEMA_V19,
        OBSERVATION_VERSION_V20 => LEARNING_OBSERVATION_SCHEMA_V20,
        OBSERVATION_VERSION_V21 => LEARNING_OBSERVATION_SCHEMA_V21,
        OBSERVATION_VERSION_V22 => LEARNING_OBSERVATION_SCHEMA_V22,
        OBSERVATION_VERSION_V23 => LEARNING_OBSERVATION_SCHEMA_V23,
        OBSERVATION_VERSION_V24 => LEARNING_OBSERVATION_SCHEMA_V24,
        OBSERVATION_VERSION_V25 => LEARNING_OBSERVATION_SCHEMA_V25,
        OBSERVATION_VERSION_V26 => LEARNING_OBSERVATION_SCHEMA_V26,
        OBSERVATION_VERSION_V27 => LEARNING_OBSERVATION_SCHEMA_V27,
        OBSERVATION_VERSION_V28 => LEARNING_OBSERVATION_SCHEMA_V28,
        OBSERVATION_VERSION_V29 => LEARNING_OBSERVATION_SCHEMA_V29,
        _ => {
            return Err(NativeEpisodeShardError::new(
                "unsupported observation schema version",
            ));
        }
    };
    let expected_shard_schema = match shard_version {
        VERSION_V1 => NATIVE_EPISODE_SHARD_SCHEMA_V1,
        VERSION_V2 => NATIVE_EPISODE_SHARD_SCHEMA_V2,
        VERSION_V3 => NATIVE_EPISODE_SHARD_SCHEMA_V3,
        _ => unreachable!("shard version was validated by the header decoder"),
    };
    if !reader.done()
        || fields[0] != expected_shard_schema
        || fields[1] != expected_observation_schema
        || fields[2] != RAW_PAD_ACTION_SCHEMA_V2
        || fields[3].len() != 32
        || fields[4].len() != 32
        || fields[5].is_empty()
        || fields[6].len() != 32
        || fields[7].is_empty()
        || fields[9].is_empty()
        || fields[10].is_empty()
    {
        return Err(NativeEpisodeShardError::new(
            "invalid shard identity metadata",
        ));
    }
    let game_data_sha256 = if shard_version != VERSION_V1 {
        Some(parse_canonical_digest(&fields[11], "game-data SHA-256")?)
    } else {
        None
    };
    let card_fixture_index = if shard_version == VERSION_V1 { 11 } else { 12 };
    let card_fixture_identity =
        (!fields[card_fixture_index].is_empty()).then(|| fields[card_fixture_index].clone());
    if shard_version != VERSION_V1
        && (!fields[card_fixture_index].starts_with("card-fixture:")
            || !fields[13].starts_with("actor-profile-catalog:"))
    {
        return Err(NativeEpisodeShardError::new(
            "invalid static dependency identity metadata",
        ));
    }
    let actor_profile_catalog_identity = (shard_version != VERSION_V1).then(|| fields[13].clone());
    let world_context_sha256 = if shard_version != VERSION_V1 {
        Some(parse_canonical_digest(
            &fields[14],
            "world-context SHA-256",
        )?)
    } else {
        None
    };
    let policy_model = if shard_version == VERSION_V3 {
        if fields[15].is_empty()
            || fields[16].len() != 32
            || !fields[16]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(NativeEpisodeShardError::new(
                "invalid frozen policy model identity metadata",
            ));
        }
        let feature_width = fields[20]
            .parse::<u32>()
            .map_err(|_| NativeEpisodeShardError::new("invalid policy feature width metadata"))?;
        if feature_width == 0 || feature_width.to_string() != fields[20] {
            return Err(NativeEpisodeShardError::new(
                "noncanonical policy feature width metadata",
            ));
        }
        Some(NativeEpisodePolicyModelIdentity {
            schema: fields[15].clone(),
            model_xxh3_128: fields[16].clone(),
            feature_schema_sha256: parse_canonical_digest(
                &fields[17],
                "policy feature-schema SHA-256",
            )?,
            action_schema_sha256: parse_canonical_digest(
                &fields[18],
                "policy action-schema SHA-256",
            )?,
            objective_sha256: parse_canonical_digest(&fields[19], "policy objective SHA-256")?,
            feature_width,
        })
    } else {
        None
    };
    Ok(NativeEpisodeShardMetadata {
        shard_schema: fields[0].clone(),
        observation_schema: fields[1].clone(),
        action_schema: fields[2].clone(),
        source_boundary_fingerprint: fields[3].clone(),
        checkpoint_identity: fields[4].clone(),
        objective: fields[5].clone(),
        objective_identity: fields[6].clone(),
        build_revision: fields[7].clone(),
        aurora_revision: fields[8].clone(),
        feature_digest: fields[9].clone(),
        fidelity_profile: fields[10].clone(),
        game_data_sha256,
        card_fixture_identity,
        actor_profile_catalog_identity,
        world_context_sha256,
        policy_model,
    })
}

pub(super) fn parse_canonical_digest(
    value: &str,
    label: &str,
) -> Result<Digest, NativeEpisodeShardError> {
    let digest: Digest = value
        .parse()
        .map_err(|_| NativeEpisodeShardError::new(format!("invalid {label} in shard metadata")))?;
    if digest == Digest::ZERO || digest.to_string() != value {
        return Err(NativeEpisodeShardError::new(format!(
            "noncanonical {label} in shard metadata"
        )));
    }
    Ok(digest)
}
