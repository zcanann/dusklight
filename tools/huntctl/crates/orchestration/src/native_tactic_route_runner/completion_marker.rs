use super::*;

pub const NATIVE_TACTIC_CAMPAIGN_COMPLETION_FILE: &str = "campaign-complete.dtcm";
pub const NATIVE_TACTIC_CAMPAIGN_COMPLETION_SCHEMA_V1: &str =
    "dusklight-native-tactic-campaign-completion/v1";

const COMPLETION_MAGIC: &[u8; 8] = b"DSKTCM01";
const COMPLETION_VERSION: u16 = 1;
const COMPLETION_HEADER_BYTES: usize = 8 + 2 + 2 + 4 + 32;
const MAXIMUM_COMPLETION_PAYLOAD_BYTES: usize = 16 * 1024;

/// Durable authority that all campaign work preceding publication completed.
///
/// `campaign_wall_micros` ends immediately before this create-new marker is
/// published. The marker write itself is deliberately outside the interval:
/// its durable presence attests that the measured prerequisite interval and
/// both hashed final artifacts completed successfully.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignCompletion {
    pub schema: String,
    pub content_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub report_sha256: Digest,
    pub summary_sha256: Digest,
    pub route_cutoff_wall_micros: u64,
    pub report_build_micros: u64,
    pub fleet_shutdown_micros: u64,
    pub final_artifact_persistence_micros: u64,
    pub campaign_completion_coordination_micros: u64,
    pub campaign_wall_micros: u64,
}

impl NativeTacticCampaignCompletion {
    pub(super) fn build(
        execution_plan_sha256: Digest,
        report_bytes: &[u8],
        summary_bytes: &[u8],
        route_cutoff_wall_micros: u64,
        report_build_micros: u64,
        fleet_shutdown_micros: u64,
        final_artifact_persistence_micros: u64,
        observed_campaign_wall_micros: u64,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let accounted_wall_micros = [
            report_build_micros,
            fleet_shutdown_micros,
            final_artifact_persistence_micros,
        ]
        .into_iter()
        .try_fold(route_cutoff_wall_micros, |total, phase| {
            total
                .checked_add(phase)
                .ok_or_else(|| route_message("native tactic completion wall timing overflowed"))
        })?;
        let campaign_wall_micros = observed_campaign_wall_micros.max(accounted_wall_micros);
        let campaign_completion_coordination_micros = campaign_wall_micros
            .checked_sub(accounted_wall_micros)
            .ok_or_else(|| route_message("native tactic completion timing is detached"))?;
        let mut completion = Self {
            schema: NATIVE_TACTIC_CAMPAIGN_COMPLETION_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            execution_plan_sha256,
            report_sha256: digest_bytes(report_bytes),
            summary_sha256: digest_bytes(summary_bytes),
            route_cutoff_wall_micros,
            report_build_micros,
            fleet_shutdown_micros,
            final_artifact_persistence_micros,
            campaign_completion_coordination_micros,
            campaign_wall_micros,
        };
        completion.content_sha256 = completion.compute_content_sha256()?;
        completion.validate()?;
        Ok(completion)
    }

    pub fn read(path: &Path) -> Result<Self, NativeTacticRouteRunError> {
        let bytes = fs::read(path).map_err(route_error)?;
        decode_completion(&bytes)
    }

    pub fn validate_files(
        &self,
        report_path: &Path,
        summary_path: &Path,
    ) -> Result<(), NativeTacticRouteRunError> {
        self.validate()?;
        let report = fs::read(report_path).map_err(route_error)?;
        let summary = fs::read(summary_path).map_err(route_error)?;
        self.validate_artifact_hashes(&report, &summary)?;
        let route = read_native_tactic_route_report(report_path)?;
        let summary: NativeTacticCampaignSummary =
            serde_json::from_slice(&summary).map_err(route_error)?;
        summary.validate()?;
        let plan = NativeTacticExecutionPlan::read(Path::new(&route.execution_plan_path))?;
        let expected_summary = NativeTacticCampaignSummary::build(&route, &plan)?;
        if self.execution_plan_sha256 != route.execution_plan_sha256
            || self.execution_plan_sha256 != summary.identities.execution_plan_sha256
            || plan.identity()? != self.execution_plan_sha256
            || summary != expected_summary
        {
            return Err(route_message(
                "native tactic completion report and summary are not one derived artifact chain",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_artifact_hashes(
        &self,
        report: &[u8],
        summary: &[u8],
    ) -> Result<(), NativeTacticRouteRunError> {
        if digest_bytes(&report) != self.report_sha256
            || digest_bytes(&summary) != self.summary_sha256
        {
            return Err(route_message(
                "native tactic completion artifacts differ from the durable marker",
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        let expected_wall = [
            self.report_build_micros,
            self.fleet_shutdown_micros,
            self.final_artifact_persistence_micros,
            self.campaign_completion_coordination_micros,
        ]
        .into_iter()
        .try_fold(self.route_cutoff_wall_micros, u64::checked_add);
        if self.schema != NATIVE_TACTIC_CAMPAIGN_COMPLETION_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || self.report_sha256 == Digest::ZERO
            || self.summary_sha256 == Digest::ZERO
            || expected_wall != Some(self.campaign_wall_micros)
            || self.compute_content_sha256()? != self.content_sha256
        {
            return Err(route_message(
                "native tactic campaign completion marker is invalid",
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
}

pub(super) fn publish_completion(
    path: &Path,
    completion: &NativeTacticCampaignCompletion,
) -> Result<(), NativeTacticRouteRunError> {
    completion.validate()?;
    publish_new_atomic(path, &encode_completion(completion)?)
}

fn encode_completion(
    completion: &NativeTacticCampaignCompletion,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let payload = serde_cbor::to_vec(completion).map_err(route_error)?;
    if payload.len() > MAXIMUM_COMPLETION_PAYLOAD_BYTES {
        return Err(route_message(
            "native tactic completion marker exceeds its bound",
        ));
    }
    let mut bytes = Vec::with_capacity(COMPLETION_HEADER_BYTES + payload.len());
    bytes.extend_from_slice(COMPLETION_MAGIC);
    bytes.extend_from_slice(&COMPLETION_VERSION.to_le_bytes());
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

fn decode_completion(
    bytes: &[u8],
) -> Result<NativeTacticCampaignCompletion, NativeTacticRouteRunError> {
    if bytes.len() < COMPLETION_HEADER_BYTES
        || &bytes[..8] != COMPLETION_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice")) != COMPLETION_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message(
            "native tactic completion marker header is invalid",
        ));
    }
    let payload_len = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice")) as usize;
    if payload_len > MAXIMUM_COMPLETION_PAYLOAD_BYTES
        || bytes.len()
            != COMPLETION_HEADER_BYTES
                .checked_add(payload_len)
                .unwrap_or(usize::MAX)
    {
        return Err(route_message(
            "native tactic completion marker length is invalid",
        ));
    }
    let expected: [u8; 32] = bytes[16..48].try_into().expect("fixed slice");
    let payload = &bytes[COMPLETION_HEADER_BYTES..];
    if expected != <[u8; 32]>::from(Sha256::digest(payload)) {
        return Err(route_message(
            "native tactic completion marker payload digest is invalid",
        ));
    }
    let completion: NativeTacticCampaignCompletion =
        serde_cbor::from_slice(payload).map_err(route_error)?;
    completion.validate()?;
    Ok(completion)
}

fn digest_bytes(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_completion_round_trips_and_rejects_timing_or_payload_drift() {
        let completion = NativeTacticCampaignCompletion::build(
            Digest([1; 32]),
            b"report",
            b"summary",
            100,
            20,
            30,
            40,
            190,
        )
        .unwrap();
        assert_eq!(completion.campaign_wall_micros, 190);
        let encoded = encode_completion(&completion).unwrap();
        assert_eq!(decode_completion(&encoded).unwrap(), completion);

        let mut detached = completion;
        detached.campaign_wall_micros += 1;
        assert!(detached.validate().is_err());

        let mut corrupt = encoded;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode_completion(&corrupt).is_err());
    }

    #[test]
    fn completion_hashes_bind_both_artifacts_before_semantic_validation() {
        let completion = NativeTacticCampaignCompletion::build(
            Digest([1; 32]),
            b"report",
            b"summary",
            100,
            20,
            30,
            40,
            190,
        )
        .unwrap();
        completion
            .validate_artifact_hashes(b"report", b"summary")
            .unwrap();
        assert!(
            completion
                .validate_artifact_hashes(b"changed", b"summary")
                .is_err()
        );
        assert!(
            completion
                .validate_artifact_hashes(b"report", b"changed")
                .is_err()
        );
    }
}
