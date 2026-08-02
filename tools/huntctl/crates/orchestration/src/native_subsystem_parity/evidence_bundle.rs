use super::*;
use std::collections::BTreeSet;

pub const NATIVE_SUBSYSTEM_PARITY_EVIDENCE_BUNDLE_SCHEMA_V1: &str =
    "dusklight-native-subsystem-parity-evidence-bundle/v1";
pub const NATIVE_SUBSYSTEM_PARITY_EVIDENCE_MANIFEST: &str = "manifest.json";
const MAXIMUM_JSON_BYTES: u64 = 64 * 1024 * 1024;
const MAXIMUM_EPISODE_SHARD_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSubsystemParityBundleArtifact {
    pub path: String,
    pub sha256: Digest,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSubsystemParityConditionEvidence {
    pub condition: String,
    pub request: NativeSubsystemParityBundleArtifact,
    pub result: NativeSubsystemParityBundleArtifact,
    pub episode_shard: NativeSubsystemParityBundleArtifact,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSubsystemParityEvidenceBundle {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request: NativeSubsystemParityBundleArtifact,
    pub execution_binding: NativeSubsystemParityBundleArtifact,
    pub report: NativeSubsystemParityBundleArtifact,
    pub conditions: Vec<NativeSubsystemParityConditionEvidence>,
}

impl NativeSubsystemParityEvidenceBundle {
    pub fn seal(
        optimization_request_path: &Path,
        execution_binding_path: &Path,
        report_path: &Path,
        run_root: &Path,
        bundle_root: &Path,
    ) -> Result<Self, Box<dyn Error>> {
        if path_exists(bundle_root)? || !physical_directory(run_root)? {
            return Err("native subsystem parity bundle destination or run root is invalid".into());
        }
        let optimization_bytes = read_bounded_regular_file(
            optimization_request_path,
            MAXIMUM_JSON_BYTES,
            "optimization request",
        )?;
        let optimization: OptimizationRequest = serde_json::from_slice(&optimization_bytes)?;
        optimization.validate()?;
        let execution_bytes = read_bounded_regular_file(
            execution_binding_path,
            MAXIMUM_JSON_BYTES,
            "execution binding",
        )?;
        let execution: NativeResidualExecutionBinding = serde_json::from_slice(&execution_bytes)?;
        execution.validate_seal(&optimization)?;
        let report_bytes =
            read_bounded_regular_file(report_path, MAXIMUM_JSON_BYTES, "parity report")?;
        let report: NativeSubsystemParityReport = serde_json::from_slice(&report_bytes)?;
        report.validate()?;
        if report.optimization_request_sha256 != optimization.content_sha256
            || report.execution_sha256 != execution.content_sha256
        {
            return Err("native subsystem parity bundle authorities are detached".into());
        }

        if let Some(parent) = bundle_root
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(bundle_root)?;
        let optimization_request = copy_artifact(
            bundle_root,
            "optimization-request.json",
            &optimization_bytes,
        )?;
        let execution_binding =
            copy_artifact(bundle_root, "execution-binding.json", &execution_bytes)?;
        let report_artifact = copy_artifact(bundle_root, "report.json", &report_bytes)?;
        let mut conditions = Vec::with_capacity(report.conditions.len());
        for measurement in &report.conditions {
            let proof_disabled = measurement.condition == "state_hash_proof_disabled";
            let source_root = run_root.join(if proof_disabled {
                "production_all_disabled"
            } else {
                measurement.condition.as_str()
            });
            let request_source = source_root.join(if proof_disabled {
                "proof-disabled.request.json"
            } else {
                "request.json"
            });
            let result_source = source_root.join(if proof_disabled {
                "proof-disabled.result.json"
            } else {
                "result.json"
            });
            let request_bytes =
                read_bounded_regular_file(&request_source, MAXIMUM_JSON_BYTES, "batch request")?;
            let result_bytes =
                read_bounded_regular_file(&result_source, MAXIMUM_JSON_BYTES, "batch result")?;
            let raw: NativeSuffixBatchResult = serde_json::from_slice(&result_bytes)?;
            let shard_source =
                resolve_source_file(run_root, &source_root, Path::new(&raw.episode_shard.path))?;
            let shard_bytes = read_bounded_regular_file(
                &shard_source,
                MAXIMUM_EPISODE_SHARD_BYTES,
                "episode shard",
            )?;
            let relative_root = format!("conditions/{}", measurement.condition);
            conditions.push(NativeSubsystemParityConditionEvidence {
                condition: measurement.condition.clone(),
                request: copy_artifact(
                    bundle_root,
                    &format!("{relative_root}/request.json"),
                    &request_bytes,
                )?,
                result: copy_artifact(
                    bundle_root,
                    &format!("{relative_root}/result.json"),
                    &result_bytes,
                )?,
                episode_shard: copy_artifact(
                    bundle_root,
                    &format!("{relative_root}/episode.dseps"),
                    &shard_bytes,
                )?,
            });
        }
        let mut bundle = Self {
            schema: NATIVE_SUBSYSTEM_PARITY_EVIDENCE_BUNDLE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request,
            execution_binding,
            report: report_artifact,
            conditions,
        };
        bundle.content_sha256 = bundle.compute_content_sha256()?;
        let mut manifest = serde_json::to_vec_pretty(&bundle)?;
        manifest.push(b'\n');
        write_new(
            &bundle_root.join(NATIVE_SUBSYSTEM_PARITY_EVIDENCE_MANIFEST),
            &manifest,
        )?;
        bundle.validate(bundle_root)?;
        Ok(bundle)
    }

    pub fn read_and_validate(bundle_root: &Path) -> Result<Self, Box<dyn Error>> {
        if !physical_directory(bundle_root)? {
            return Err("native subsystem parity bundle root is not a physical directory".into());
        }
        let manifest = read_bounded_regular_file(
            &bundle_root.join(NATIVE_SUBSYSTEM_PARITY_EVIDENCE_MANIFEST),
            MAXIMUM_JSON_BYTES,
            "parity bundle manifest",
        )?;
        let bundle: Self = serde_json::from_slice(&manifest)?;
        bundle.validate(bundle_root)?;
        Ok(bundle)
    }

    pub fn validate(&self, bundle_root: &Path) -> Result<(), Box<dyn Error>> {
        if self.schema != NATIVE_SUBSYSTEM_PARITY_EVIDENCE_BUNDLE_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
            || self.conditions.is_empty()
        {
            return Err("native subsystem parity bundle identity is invalid".into());
        }
        let mut expected_paths =
            BTreeSet::from([NATIVE_SUBSYSTEM_PARITY_EVIDENCE_MANIFEST.to_owned()]);
        let optimization_bytes = validate_artifact(
            bundle_root,
            &self.optimization_request,
            MAXIMUM_JSON_BYTES,
            &mut expected_paths,
        )?;
        let execution_bytes = validate_artifact(
            bundle_root,
            &self.execution_binding,
            MAXIMUM_JSON_BYTES,
            &mut expected_paths,
        )?;
        let report_bytes = validate_artifact(
            bundle_root,
            &self.report,
            MAXIMUM_JSON_BYTES,
            &mut expected_paths,
        )?;
        let optimization: OptimizationRequest = serde_json::from_slice(&optimization_bytes)?;
        optimization.validate()?;
        let execution: NativeResidualExecutionBinding = serde_json::from_slice(&execution_bytes)?;
        execution.validate_seal(&optimization)?;
        let report: NativeSubsystemParityReport = serde_json::from_slice(&report_bytes)?;
        report.validate()?;
        if report.optimization_request_sha256 != optimization.content_sha256
            || report.execution_sha256 != execution.content_sha256
            || self.conditions.len() != report.conditions.len()
        {
            return Err("native subsystem parity bundle authorities are detached".into());
        }
        let terminal = NativeTerminalBinding {
            goal: optimization.terminal_predicate.goal.clone(),
            program_sha256: optimization.terminal_predicate.program_sha256,
            definition_sha256: optimization.terminal_predicate.definition_sha256,
        };
        for (condition, measurement) in self.conditions.iter().zip(&report.conditions) {
            if condition.condition != measurement.condition {
                return Err("native subsystem parity bundle condition order is detached".into());
            }
            let request_bytes = validate_artifact(
                bundle_root,
                &condition.request,
                MAXIMUM_JSON_BYTES,
                &mut expected_paths,
            )?;
            let result_bytes = validate_artifact(
                bundle_root,
                &condition.result,
                MAXIMUM_JSON_BYTES,
                &mut expected_paths,
            )?;
            validate_artifact(
                bundle_root,
                &condition.episode_shard,
                MAXIMUM_EPISODE_SHARD_BYTES,
                &mut expected_paths,
            )?;
            let request: NativeSuffixBatch = serde_json::from_slice(&request_bytes)?;
            let raw: NativeSuffixBatchResult = serde_json::from_slice(&result_bytes)?;
            raw.validate_against(&request, &terminal)?;
            let shard = NativeEpisodeShard::read(&bundle_root.join(&condition.episode_shard.path))?;
            validate_condition_source(
                measurement,
                &raw,
                &shard,
                report.schema == NATIVE_SUBSYSTEM_PARITY_SCHEMA_V2,
            )?;
        }
        let actual_paths = bundle_files(bundle_root)?;
        if actual_paths != expected_paths {
            return Err("native subsystem parity bundle contains detached files".into());
        }
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, Box<dyn Error>> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        digest_json(&unsigned)
    }
}

fn validate_condition_source(
    measurement: &NativeSubsystemConditionMeasurement,
    raw: &NativeSuffixBatchResult,
    shard: &NativeEpisodeShard,
    legacy_v2: bool,
) -> Result<(), Box<dyn Error>> {
    let phases = raw
        .timing
        .phases
        .as_object()
        .ok_or("native subsystem timing phases are not an object")?;
    let cpu_renderer_submission_micros = phases
        .get("cpu_renderer_submission")
        .and_then(|value| value.get("micros"))
        .and_then(Value::as_u64)
        .ok_or("native subsystem CPU renderer timing is missing")?;
    let gpu_work = phases
        .get("gpu_work")
        .ok_or("native subsystem GPU work is missing")?;
    let state_validation = phases
        .get("state_validation")
        .ok_or("native subsystem state validation is missing")?;
    let evidence = if legacy_v2 {
        evidence_projection_with_action_surface(
            &measurement.evidence.applicable_action_surface_context,
            (
                measurement.evidence.applicable_action_surface_sha256,
                measurement.evidence.applicable_action_surface_boundaries,
                measurement.evidence.applicable_action_descriptors,
            ),
            raw,
            shard,
        )?
    } else {
        evidence_projection_with_context(
            &measurement.evidence.applicable_action_surface_context,
            raw,
            shard,
        )?
    };
    let configuration_verified = if legacy_v2 {
        legacy_v2::validate_configuration_projection(
            measurement.comparators,
            measurement.verify_state_hashes,
            raw.timing.candidate_ticks,
            &raw.timing.headless_audit,
            cpu_renderer_submission_micros,
            gpu_work,
            state_validation,
        )
    } else {
        validate_configuration_projection(
            measurement.comparators,
            measurement.verify_state_hashes,
            raw.timing.candidate_ticks,
            &raw.timing.headless_audit,
            cpu_renderer_submission_micros,
            gpu_work,
            state_validation,
        )
    };
    if raw.verify_state_hashes != measurement.verify_state_hashes
        || raw.timing.batch_wall_micros != measurement.batch_wall_micros
        || phase_micros(raw, "simulation")? != measurement.simulation_micros
        || cpu_renderer_submission_micros != measurement.cpu_renderer_submission_micros
        || raw.timing.headless_audit != measurement.headless_audit
        || gpu_work != &measurement.gpu_work
        || state_validation != &measurement.state_validation
        || evidence != measurement.evidence
        || configuration_verified != measurement.configuration_verified
    {
        return Err(format!(
            "native subsystem parity condition {} is detached from bundled native evidence",
            measurement.condition
        )
        .into());
    }
    Ok(())
}

fn copy_artifact(
    bundle_root: &Path,
    relative: &str,
    bytes: &[u8],
) -> Result<NativeSubsystemParityBundleArtifact, Box<dyn Error>> {
    let destination = safe_bundle_path(bundle_root, relative)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    write_new(&destination, bytes)?;
    Ok(NativeSubsystemParityBundleArtifact {
        path: relative.into(),
        sha256: Digest(Sha256::digest(bytes).into()),
        bytes: u64::try_from(bytes.len())?,
    })
}

fn validate_artifact(
    bundle_root: &Path,
    artifact: &NativeSubsystemParityBundleArtifact,
    maximum_bytes: u64,
    expected_paths: &mut BTreeSet<String>,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if artifact.sha256 == Digest::ZERO
        || artifact.bytes == 0
        || artifact.bytes > maximum_bytes
        || !expected_paths.insert(artifact.path.clone())
    {
        return Err("native subsystem parity bundle artifact identity is invalid".into());
    }
    let bytes = read_bounded_regular_file(
        &safe_bundle_path(bundle_root, &artifact.path)?,
        maximum_bytes,
        "parity bundle artifact",
    )?;
    if u64::try_from(bytes.len())? != artifact.bytes
        || Digest(Sha256::digest(&bytes).into()) != artifact.sha256
    {
        return Err("native subsystem parity bundle artifact content is detached".into());
    }
    Ok(bytes)
}

fn safe_bundle_path(bundle_root: &Path, relative: &str) -> Result<PathBuf, Box<dyn Error>> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("native subsystem parity bundle path is invalid".into());
    }
    Ok(bundle_root.join(relative))
}

fn resolve_source_file(
    run_root: &Path,
    condition_root: &Path,
    declared: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    let candidates = if declared.is_absolute() {
        vec![declared.to_path_buf()]
    } else {
        vec![
            declared.to_path_buf(),
            run_root.join(declared),
            condition_root.join(declared),
        ]
    };
    let mut matches = BTreeSet::new();
    for candidate in candidates {
        if path_exists(&candidate)? {
            let canonical = fs::canonicalize(candidate)?;
            let metadata = fs::symlink_metadata(&canonical)?;
            if metadata.file_type().is_file() && !metadata.file_type().is_symlink() {
                matches.insert(canonical);
            }
        }
    }
    if matches.len() != 1 {
        return Err("native subsystem parity episode shard path is absent or ambiguous".into());
    }
    Ok(matches.pop_first().expect("one checked source file"))
}

fn bundle_files(bundle_root: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    fn visit(
        bundle_root: &Path,
        directory: &Path,
        files: &mut BTreeSet<String>,
    ) -> Result<(), Box<dyn Error>> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err("native subsystem parity bundle contains a symlink".into());
            }
            if file_type.is_dir() {
                visit(bundle_root, &entry.path(), files)?;
            } else if file_type.is_file() {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(bundle_root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            } else {
                return Err("native subsystem parity bundle contains a special file".into());
            }
        }
        Ok(())
    }
    let mut files = BTreeSet::new();
    visit(bundle_root, bundle_root, &mut files)?;
    Ok(files)
}

fn path_exists(path: &Path) -> Result<bool, Box<dyn Error>> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn physical_directory(path: &Path) -> Result<bool, Box<dyn Error>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_dir() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn read_bounded_regular_file(
    path: &Path,
    maximum_bytes: u64,
    description: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > maximum_bytes
    {
        return Err(format!("{description} is not a bounded physical file").into());
    }
    let bytes = fs::read(path)?;
    if u64::try_from(bytes.len())? != metadata.len() {
        return Err(format!("{description} changed while it was read").into());
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_paths_are_confined_to_the_bundle_root() {
        let root = Path::new("bundle");
        assert_eq!(
            safe_bundle_path(root, "conditions/control/result.json").unwrap(),
            root.join("conditions/control/result.json")
        );
        assert!(safe_bundle_path(root, "../detached.json").is_err());
        assert!(safe_bundle_path(root, "conditions/../detached.json").is_err());
        assert!(safe_bundle_path(root, "").is_err());
    }
}
