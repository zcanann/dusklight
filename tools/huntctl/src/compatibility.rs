use crate::artifact::ArtifactIdentity;

/// The operation being protected by an identity comparison. Different
/// operations intentionally admit different kinds of variation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilityMode {
    Replay,
    TraceMerge,
    ModelTraining,
    CheckpointRestore,
    CrossBuildComparison,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityDifference {
    pub field: &'static str,
    pub expected: String,
    pub actual: String,
}

impl CompatibilityDifference {
    pub fn message(&self) -> String {
        format!(
            "{}: expected {}, received {}",
            self.field, self.expected, self.actual
        )
    }
}

/// Returns every field which makes `actual` incompatible with `expected` for
/// the selected operation. Artifact payload digests are deliberately excluded:
/// two artifacts can contain different tapes or traces from the same run
/// environment.
pub fn compatibility_differences(
    mode: CompatibilityMode,
    expected: &ArtifactIdentity,
    actual: &ArtifactIdentity,
) -> Vec<CompatibilityDifference> {
    let mut differences = Vec::new();
    macro_rules! compare {
        ($field:literal, $expected:expr, $actual:expr) => {
            if $expected != $actual {
                differences.push(CompatibilityDifference {
                    field: $field,
                    expected: format!("{:?}", $expected),
                    actual: format!("{:?}", $actual),
                });
            }
        };
    }

    compare!(
        "schema_version",
        expected.schema_version,
        actual.schema_version
    );

    let exact_build = matches!(
        mode,
        CompatibilityMode::Replay
            | CompatibilityMode::TraceMerge
            | CompatibilityMode::CheckpointRestore
    );
    if exact_build {
        compare!(
            "build.dusklight_commit",
            expected.build.dusklight_commit,
            actual.build.dusklight_commit
        );
        compare!(
            "build.aurora_commit",
            expected.build.aurora_commit,
            actual.build.aurora_commit
        );
        compare!(
            "build.compiler",
            expected.build.compiler,
            actual.build.compiler
        );
        compare!("build.target", expected.build.target, actual.build.target);
        compare!(
            "build.profile",
            expected.build.profile,
            actual.build.profile
        );
        compare!(
            "build.dirty_digest",
            expected.build.dirty_digest,
            actual.build.dirty_digest
        );
    }

    if exact_build || mode == CompatibilityMode::ModelTraining {
        compare!(
            "build.feature_digest",
            expected.build.feature_digest,
            actual.build.feature_digest
        );
    }

    compare!(
        "build.game_digest",
        expected.build.game_digest,
        actual.build.game_digest
    );
    compare!(
        "build.fidelity_profile",
        expected.build.fidelity_profile,
        actual.build.fidelity_profile
    );
    compare!(
        "protocol_name",
        expected.protocol_name,
        actual.protocol_name
    );
    compare!(
        "protocol_version",
        expected.protocol_version,
        actual.protocol_version
    );
    compare!(
        "protocol_capabilities_digest",
        expected.protocol_capabilities_digest,
        actual.protocol_capabilities_digest
    );
    compare!(
        "region_digest",
        expected.region_digest,
        actual.region_digest
    );
    compare!(
        "language_assets_digest",
        expected.language_assets_digest,
        actual.language_assets_digest
    );
    compare!(
        "settings_digest",
        expected.settings_digest,
        actual.settings_digest
    );

    let exact_scenario = matches!(
        mode,
        CompatibilityMode::Replay
            | CompatibilityMode::CheckpointRestore
            | CompatibilityMode::CrossBuildComparison
    );
    if exact_scenario {
        compare!("scenario_id", expected.scenario_id, actual.scenario_id);
        compare!(
            "scenario_digest",
            expected.scenario_digest,
            actual.scenario_digest
        );
    }

    let predicate_relevant = matches!(
        mode,
        CompatibilityMode::Replay | CompatibilityMode::CrossBuildComparison
    );
    if predicate_relevant {
        compare!(
            "predicate_program_digest",
            expected.predicate_program_digest,
            actual.predicate_program_digest
        );
    }

    let schemas_relevant = !matches!(mode, CompatibilityMode::CheckpointRestore);
    if schemas_relevant {
        compare!(
            "action_schema_digest",
            expected.action_schema_digest,
            actual.action_schema_digest
        );
        compare!(
            "observation_schema_digest",
            expected.observation_schema_digest,
            actual.observation_schema_digest
        );
    }

    differences
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{ARTIFACT_SCHEMA_VERSION, BuildIdentity, Digest};

    fn identity() -> ArtifactIdentity {
        ArtifactIdentity {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            content_digest: Digest([1; 32]),
            build: BuildIdentity {
                dusklight_commit: "dusk-a".into(),
                aurora_commit: "aurora-a".into(),
                compiler: "clang-a".into(),
                target: "arm64-macos".into(),
                profile: "debug".into(),
                feature_digest: Digest([2; 32]),
                game_digest: Digest([3; 32]),
                dirty_digest: Some(Digest([4; 32])),
                fidelity_profile: "retail".into(),
            },
            protocol_name: "dusklight-worker".into(),
            protocol_version: 2,
            protocol_capabilities_digest: Digest([5; 32]),
            scenario_id: "stage-f-sp103".into(),
            region_digest: Digest([6; 32]),
            language_assets_digest: Digest([7; 32]),
            scenario_digest: Digest([8; 32]),
            predicate_program_digest: Digest([9; 32]),
            action_schema_digest: Digest([10; 32]),
            observation_schema_digest: Digest([11; 32]),
            settings_digest: Digest([12; 32]),
        }
    }

    fn fields(
        mode: CompatibilityMode,
        expected: &ArtifactIdentity,
        actual: &ArtifactIdentity,
    ) -> Vec<&'static str> {
        compatibility_differences(mode, expected, actual)
            .into_iter()
            .map(|difference| difference.field)
            .collect()
    }

    #[test]
    fn replay_rejects_build_scenario_and_schema_changes() {
        let expected = identity();
        let mut actual = expected.clone();
        actual.build.compiler = "clang-b".into();
        actual.scenario_digest = Digest([20; 32]);
        actual.observation_schema_digest = Digest([21; 32]);
        assert_eq!(
            fields(CompatibilityMode::Replay, &expected, &actual),
            [
                "build.compiler",
                "scenario_digest",
                "observation_schema_digest"
            ]
        );
    }

    #[test]
    fn training_allows_build_scenario_and_predicate_diversity() {
        let expected = identity();
        let mut actual = expected.clone();
        actual.build.compiler = "clang-b".into();
        actual.scenario_digest = Digest([20; 32]);
        actual.predicate_program_digest = Digest([21; 32]);
        assert!(
            compatibility_differences(CompatibilityMode::ModelTraining, &expected, &actual)
                .is_empty()
        );

        actual.observation_schema_digest = Digest([22; 32]);
        assert_eq!(
            fields(CompatibilityMode::ModelTraining, &expected, &actual),
            ["observation_schema_digest"]
        );
    }

    #[test]
    fn cross_build_comparison_requires_identical_run_inputs() {
        let expected = identity();
        let mut actual = expected.clone();
        actual.build.dusklight_commit = "dusk-b".into();
        actual.build.compiler = "clang-b".into();
        assert!(
            compatibility_differences(CompatibilityMode::CrossBuildComparison, &expected, &actual)
                .is_empty()
        );

        actual.settings_digest = Digest([23; 32]);
        assert_eq!(
            fields(CompatibilityMode::CrossBuildComparison, &expected, &actual),
            ["settings_digest"]
        );
    }

    #[test]
    fn payload_digest_is_not_an_environment_mismatch() {
        let expected = identity();
        let mut actual = expected.clone();
        actual.content_digest = Digest([99; 32]);
        for mode in [
            CompatibilityMode::Replay,
            CompatibilityMode::TraceMerge,
            CompatibilityMode::ModelTraining,
            CompatibilityMode::CheckpointRestore,
            CompatibilityMode::CrossBuildComparison,
        ] {
            assert!(compatibility_differences(mode, &expected, &actual).is_empty());
        }
    }
}
