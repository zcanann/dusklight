use super::*;

pub(super) const NATIVE_TACTIC_MACRO_DISCOVERY_FILE: &str = "tactic-macro-discovery.dtmd";
const MACRO_DISCOVERY_SCHEMA_V1: &str = "dusklight-tactic-macro-discovery-artifact/v1";
const MACRO_DISCOVERY_MAGIC: &[u8; 8] = b"DSKMD001";
const MACRO_DISCOVERY_VERSION: u16 = 1;
const HEADER_BYTES: usize = 8 + 2 + 2 + 4 + 32;
const MAXIMUM_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredMacroDiscoveryReport {
    schema: String,
    content_sha256: Digest,
    execution_plan_sha256: Digest,
    objective_sha256: Digest,
    feature_schema_sha256: Digest,
    root_checkpoint_sha256: Digest,
    report: NativeTacticMacroDiscoveryReport,
}

impl StoredMacroDiscoveryReport {
    fn build(
        execution_plan_sha256: Digest,
        objective_sha256: Digest,
        feature_schema_sha256: Digest,
        root_checkpoint_sha256: Digest,
        report: NativeTacticMacroDiscoveryReport,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let mut stored = Self {
            schema: MACRO_DISCOVERY_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            execution_plan_sha256,
            objective_sha256,
            feature_schema_sha256,
            root_checkpoint_sha256,
            report,
        };
        stored.content_sha256 = stored.expected_content_sha256()?;
        stored.validate()?;
        Ok(stored)
    }

    fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != MACRO_DISCOVERY_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.root_checkpoint_sha256 == Digest::ZERO
            || self.report.registry_sha256 == Digest::ZERO
            || self
                .report
                .active_promoted_option_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .report
                .active_promoted_option_ids
                .iter()
                .any(|option_id| !option_id.starts_with("promoted/"))
            || (!self.report.active_promoted_option_ids.is_empty()
                && self.report.active_refresh_count == 0)
            || (self.report.active_selected_decisions > 0
                && self.report.active_promoted_option_ids.is_empty())
            || self.report.active_policy_evidence_admitted_rows
                > self.report.active_policy_evidence_rows
            || self.expected_content_sha256()? != self.content_sha256
        {
            return Err(route_message(
                "tactic macro discovery artifact identity is invalid",
            ));
        }
        let registry_path = Path::new(&self.report.registry_path);
        let registry = read_tactic_macro_registry(registry_path).map_err(route_error)?;
        if registry.content_sha256 != self.report.registry_sha256 {
            return Err(route_message(
                "tactic macro discovery artifact registry is detached",
            ));
        }
        Ok(())
    }

    fn expected_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(&serde_cbor::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

pub(super) fn write_macro_discovery_report(
    output_root: &Path,
    execution_plan_sha256: Digest,
    objective_sha256: Digest,
    feature_schema_sha256: Digest,
    root_checkpoint_sha256: Digest,
    report: NativeTacticMacroDiscoveryReport,
) -> Result<NativeTacticMacroDiscoveryReport, NativeTacticRouteRunError> {
    let stored = StoredMacroDiscoveryReport::build(
        execution_plan_sha256,
        objective_sha256,
        feature_schema_sha256,
        root_checkpoint_sha256,
        report,
    )?;
    let path = output_root.join(NATIVE_TACTIC_MACRO_DISCOVERY_FILE);
    publish_new_atomic(&path, &encode(&stored)?)?;
    read_macro_discovery_report(
        output_root,
        execution_plan_sha256,
        objective_sha256,
        feature_schema_sha256,
        root_checkpoint_sha256,
    )
}

pub(super) fn read_macro_discovery_report(
    output_root: &Path,
    execution_plan_sha256: Digest,
    objective_sha256: Digest,
    feature_schema_sha256: Digest,
    root_checkpoint_sha256: Digest,
) -> Result<NativeTacticMacroDiscoveryReport, NativeTacticRouteRunError> {
    let stored = decode(
        &fs::read(output_root.join(NATIVE_TACTIC_MACRO_DISCOVERY_FILE)).map_err(route_error)?,
    )?;
    if stored.execution_plan_sha256 != execution_plan_sha256
        || stored.objective_sha256 != objective_sha256
        || stored.feature_schema_sha256 != feature_schema_sha256
        || stored.root_checkpoint_sha256 != root_checkpoint_sha256
    {
        return Err(route_message(
            "tactic macro discovery artifact belongs to another campaign",
        ));
    }
    Ok(stored.report)
}

fn encode(stored: &StoredMacroDiscoveryReport) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    stored.validate()?;
    let payload = serde_cbor::to_vec(stored).map_err(route_error)?;
    if payload.len() > MAXIMUM_PAYLOAD_BYTES {
        return Err(route_message(
            "tactic macro discovery artifact exceeds its size bound",
        ));
    }
    let mut bytes = Vec::with_capacity(HEADER_BYTES + payload.len());
    bytes.extend_from_slice(MACRO_DISCOVERY_MAGIC);
    bytes.extend_from_slice(&MACRO_DISCOVERY_VERSION.to_le_bytes());
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

fn decode(bytes: &[u8]) -> Result<StoredMacroDiscoveryReport, NativeTacticRouteRunError> {
    if bytes.len() < HEADER_BYTES
        || &bytes[..8] != MACRO_DISCOVERY_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"))
            != MACRO_DISCOVERY_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message(
            "tactic macro discovery artifact header is invalid",
        ));
    }
    let payload_len = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice")) as usize;
    if payload_len > MAXIMUM_PAYLOAD_BYTES
        || bytes.len() != HEADER_BYTES.checked_add(payload_len).unwrap_or(usize::MAX)
    {
        return Err(route_message(
            "tactic macro discovery artifact length is invalid",
        ));
    }
    let payload = &bytes[HEADER_BYTES..];
    if bytes[16..48] != <[u8; 32]>::from(Sha256::digest(payload)) {
        return Err(route_message(
            "tactic macro discovery artifact checksum is invalid",
        ));
    }
    let stored: StoredMacroDiscoveryReport =
        serde_cbor::from_slice(payload).map_err(route_error)?;
    stored.validate()?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(path: &Path, registry_sha256: Digest) -> NativeTacticMacroDiscoveryReport {
        NativeTacticMacroDiscoveryReport {
            active_refresh_count: 1,
            active_promoted_option_ids: vec!["promoted/001122".into()],
            active_selected_decisions: 2,
            active_policy_evidence_rows: 2,
            active_policy_evidence_admitted_rows: 2,
            observation_count: 3,
            unreconstructable_component_count: 1,
            high_value_observation_count: 1,
            mined_observation_count: 2,
            candidate_count: 1,
            entry_condition_count: 1,
            held_out_compatible_candidate_count: 0,
            source_state_exclusion_count: 0,
            entry_incompatible_frontier_count: 0,
            proposed_count: 1,
            promoted_count: 0,
            demoted_count: 0,
            validation_state_count: 0,
            comparison_count: 0,
            reused_primitive_baseline_count: 0,
            executed_component_baseline_count: 0,
            validation_native_ticks: 0,
            validation_wall_micros: 7,
            validation_native_simulation_micros: 0,
            validation_ipc_and_result_transport_micros: 0,
            validation_native_observation_capture_micros: 0,
            validation_native_corpus_encoding_micros: 0,
            validation_rust_state_extraction_micros: 0,
            validation_preparation_micros: 0,
            validation_restore_accounting: NativeTacticRestoreAccounting::default(),
            reuse: None,
            registry_path: path_text(path),
            registry_sha256,
        }
    }

    #[test]
    fn binary_macro_discovery_report_rejects_payload_tampering() {
        let directory = std::env::temp_dir().join(format!(
            "dusklight-macro-discovery-report-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let registry_path =
            directory.join(format!("tactic-macros.{TACTIC_MACRO_REGISTRY_EXTENSION}"));
        let registry = TacticMacroPromotionRegistry::default();
        let registry_sha256 = write_tactic_macro_registry(&registry_path, &registry).unwrap();
        let report = report(&registry_path, registry_sha256);
        let stored = StoredMacroDiscoveryReport::build(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            Digest([4; 32]),
            report.clone(),
        )
        .unwrap();
        let mut detached_active = report.clone();
        detached_active.active_promoted_option_ids.clear();
        assert!(
            StoredMacroDiscoveryReport::build(
                Digest([1; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                Digest([4; 32]),
                detached_active,
            )
            .is_err()
        );
        let encoded = encode(&stored).unwrap();
        assert_eq!(decode(&encoded).unwrap(), stored);
        let mut tampered = encoded;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(decode(&tampered).is_err());

        assert_eq!(
            write_macro_discovery_report(
                &directory,
                Digest([1; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                Digest([4; 32]),
                report.clone(),
            )
            .unwrap(),
            report
        );
        assert!(
            read_macro_discovery_report(
                &directory,
                Digest([9; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                Digest([4; 32]),
            )
            .is_err()
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
