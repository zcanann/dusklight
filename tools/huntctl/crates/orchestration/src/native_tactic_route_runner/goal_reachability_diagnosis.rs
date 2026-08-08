use super::*;

pub const NATIVE_TACTIC_GOAL_REACHABILITY_DIAGNOSIS_SCHEMA_V1: &str =
    "dusklight-native-tactic-goal-reachability-diagnosis/v1";
const MAXIMUM_DIAGNOSTIC_REVISIONS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticGoalReachabilityRevisionDiagnosis {
    pub replay_revision: u64,
    pub replay_snapshot_sha256: Digest,
    pub diagnosis: GoalReachabilityCalibrationDiagnosis,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticGoalReachabilityDiagnosis {
    pub schema: String,
    pub route_report_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub objective_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub revisions: Vec<NativeTacticGoalReachabilityRevisionDiagnosis>,
}

impl NativeTacticGoalReachabilityDiagnosis {
    /// Reconstructs the exact held-out sibling experiment at selected durable
    /// replay revisions. No native execution or new learner evidence occurs.
    pub fn build(
        repository_root: &Path,
        route: &NativeTacticRouteReport,
        revisions: &[u64],
    ) -> Result<Self, NativeTacticRouteRunError> {
        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        if revisions.is_empty()
            || revisions.len() > MAXIMUM_DIAGNOSTIC_REVISIONS
            || revisions.windows(2).any(|pair| pair[0] >= pair[1])
            || revisions
                .last()
                .is_some_and(|revision| *revision > route.replay_revision)
        {
            return Err(route_message(
                "goal reachability diagnosis revisions are invalid",
            ));
        }
        let seed = route
            .seeds
            .first()
            .ok_or_else(|| route_message("goal reachability diagnosis has no seed authority"))?;
        let checkpoint_path = confined_existing_file(
            &repository_root,
            Path::new(&seed.final_checkpoint),
            "goal reachability checkpoint",
        )?;
        let checkpoint =
            TacticQCampaign::read_checkpoint_payload(&checkpoint_path).map_err(route_error)?;
        if checkpoint.execution_authority_sha256 != route.execution_plan_sha256
            || checkpoint.feature_schema_sha256 != route.feature_schema_sha256
            || checkpoint.objective_sha256 != route.objective_sha256
        {
            return Err(route_message(
                "goal reachability checkpoint differs from route authority",
            ));
        }

        let replay_path = confined_existing_file(
            &repository_root,
            Path::new(&route.replay_control_plane_path),
            "goal reachability replay journal",
        )?;
        let content_root = replay_path
            .parent()
            .ok_or_else(|| route_message("goal reachability replay has no campaign root"))?
            .join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY);
        let identity = TacticReplayControlPlaneIdentity::new(
            route.execution_plan_sha256,
            route.feature_schema_sha256,
            route.objective_sha256,
            checkpoint.root_checkpoint_sha256,
        )
        .map_err(route_error)?;
        let replay = TacticReplayControlPlane::open(&replay_path, &content_root, &identity)
            .map_err(route_error)?;
        if replay.replay_snapshot().revision != route.replay_revision
            || replay.replay_snapshot().sha256 != route.replay_snapshot_sha256
        {
            return Err(route_message(
                "goal reachability replay differs from completed route",
            ));
        }

        let encoder = GoalConditionedTacticFeatureEncoder::new([0.0; 3])
            .map_err(|error| route_message(error.to_string()))?;
        if encoder.schema_sha256 != route.feature_schema_sha256 {
            return Err(route_message(
                "goal reachability feature schema differs from completed route",
            ));
        }
        let mut revision_diagnoses = Vec::with_capacity(revisions.len());
        for revision in revisions {
            let snapshot = replay.snapshot_through(*revision).map_err(route_error)?;
            let diagnosis = calibrate_goal_reachability_with_diagnosis(
                &snapshot.corpus.transitions,
                encoder.goal_distance_feature(),
            )
            .map_err(route_error)?;
            revision_diagnoses.push(NativeTacticGoalReachabilityRevisionDiagnosis {
                replay_revision: *revision,
                replay_snapshot_sha256: snapshot.version.sha256,
                diagnosis,
            });
        }
        Ok(Self {
            schema: NATIVE_TACTIC_GOAL_REACHABILITY_DIAGNOSIS_SCHEMA_V1.into(),
            route_report_sha256: super::scratch_discovery::route_report_sha256(route)?,
            execution_plan_sha256: route.execution_plan_sha256,
            objective_sha256: route.objective_sha256,
            feature_schema_sha256: route.feature_schema_sha256,
            revisions: revision_diagnoses,
        })
    }
}

fn confined_existing_file(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    let path = path.canonicalize().map_err(route_error)?;
    if !path.starts_with(repository_root) || !path.is_file() {
        return Err(route_message(format!(
            "{label} is outside the repository or absent"
        )));
    }
    Ok(path)
}
