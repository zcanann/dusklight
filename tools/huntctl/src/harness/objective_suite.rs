//! Versioned, content-addressed conformance-objective suite manifests.

use super::observation_contract::ObjectiveObservationRequirements;
use crate::artifact::Digest;
use crate::controller_program::ControllerProgram;
use crate::milestone_dsl;
use crate::observation_view::ObservationSpec;
use crate::scenario_fixture::ScenarioFixture;
use crate::tape::{InputTape, TapeBoot};
use crate::{tape_dsl, tape_program::TapeProgram};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const OBJECTIVE_SUITE_SCHEMA_V2: &str = "dusklight-objective-suite/v2";
const MAX_CASES: usize = 64;
const MAX_TEXT_BYTES: usize = 2_048;
const MAX_LOGICAL_TICKS: u64 = 10_000_000;
const MAX_HOST_TIMEOUT_SECONDS: u32 = 86_400;
const MAX_REPETITIONS: u16 = 100;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveSuite {
    pub schema: String,
    pub content_sha256: Digest,
    pub id: String,
    pub description: String,
    pub cases: Vec<ObjectiveSuiteCase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveSuiteCase {
    pub id: String,
    pub description: String,
    pub role: ObjectiveCaseRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_for: Option<String>,
    pub boot: ObjectiveBoot,
    pub scenario: ArtifactReference,
    pub objective: ObjectiveProgramReference,
    pub observation_view: ObservationViewReference,
    pub action_schema: SchemaIdentity,
    pub observation_requirements: ObjectiveObservationRequirements,
    pub seed: ObjectiveSeed,
    pub logical_tick_budget: u64,
    pub host_timeout_seconds: u32,
    pub repetitions: u16,
    pub expected_terminal: ExpectedTerminalClass,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveCaseRole {
    Positive,
    NegativeControl,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectiveBoot {
    Process,
    Stage {
        stage: String,
        room: i8,
        point: i16,
        layer: i8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_slot: Option<u8>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub path: String,
    pub sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveProgramReference {
    pub source: ArtifactReference,
    pub program_sha256: Digest,
    pub goal: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationViewReference {
    pub source: ArtifactReference,
    pub schema_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaIdentity {
    pub id: String,
    pub sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectiveSeed {
    Neutral,
    Tape { artifact: ArtifactReference },
    TapeSource { artifact: ArtifactReference },
    Controller { artifact: ArtifactReference },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedTerminalClass {
    Reached,
    ObjectiveMiss,
    Unsupported,
    Impossible,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveSuiteValidationReport {
    pub schema: &'static str,
    pub suite_id: String,
    pub suite_sha256: Digest,
    pub case_count: u64,
    pub positive_count: u64,
    pub negative_control_count: u64,
    pub case_ids: Vec<String>,
}

impl ObjectiveSuite {
    pub fn validate(&self) -> Result<(), ObjectiveSuiteError> {
        if self.schema != OBJECTIVE_SUITE_SCHEMA_V2 {
            return Err(suite_error("unsupported objective-suite schema"));
        }
        validate_id("suite id", &self.id)?;
        validate_text("suite description", &self.description)?;
        if self.cases.is_empty()
            || self.cases.len() > MAX_CASES
            || !self.cases.windows(2).all(|pair| pair[0].id < pair[1].id)
        {
            return Err(suite_error(
                "objective-suite cases must be nonempty, bounded, unique, and id-sorted",
            ));
        }
        let case_roles = self
            .cases
            .iter()
            .map(|case| (case.id.as_str(), case.role))
            .collect::<BTreeMap<_, _>>();
        for case in &self.cases {
            case.validate(&case_roles)?;
        }
        if self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(suite_error("objective-suite content identity is invalid"));
        }
        Ok(())
    }

    pub fn validate_files(
        &self,
        repository_root: &Path,
    ) -> Result<ObjectiveSuiteValidationReport, ObjectiveSuiteError> {
        self.validate()?;
        let canonical_root = repository_root.canonicalize().map_err(|error| {
            suite_error(format!(
                "cannot resolve repository root {}: {error}",
                repository_root.display()
            ))
        })?;
        for case in &self.cases {
            validate_case_files(case, &canonical_root)?;
        }
        Ok(self.report())
    }

    pub fn refresh_content_sha256(&mut self) -> Result<(), ObjectiveSuiteError> {
        self.content_sha256 = self.compute_content_sha256()?;
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, ObjectiveSuiteError> {
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| suite_error(format!("cannot encode objective suite: {error}")))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn report(&self) -> ObjectiveSuiteValidationReport {
        ObjectiveSuiteValidationReport {
            schema: "dusklight-objective-suite-validation/v1",
            suite_id: self.id.clone(),
            suite_sha256: self.content_sha256,
            case_count: self.cases.len() as u64,
            positive_count: self
                .cases
                .iter()
                .filter(|case| case.role == ObjectiveCaseRole::Positive)
                .count() as u64,
            negative_control_count: self
                .cases
                .iter()
                .filter(|case| case.role == ObjectiveCaseRole::NegativeControl)
                .count() as u64,
            case_ids: self.cases.iter().map(|case| case.id.clone()).collect(),
        }
    }

    fn compute_content_sha256(&self) -> Result<Digest, ObjectiveSuiteError> {
        let encoded = serde_json::to_vec(&(&self.schema, &self.id, &self.description, &self.cases))
            .map_err(|error| suite_error(format!("cannot encode objective suite: {error}")))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.objective-suite/v2\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        Ok(Digest(hasher.finalize().into()))
    }
}

impl ObjectiveSuiteCase {
    fn validate(
        &self,
        case_roles: &BTreeMap<&str, ObjectiveCaseRole>,
    ) -> Result<(), ObjectiveSuiteError> {
        validate_id("case id", &self.id)?;
        validate_text("case description", &self.description)?;
        match (self.role, self.control_for.as_deref()) {
            (ObjectiveCaseRole::Positive, None) => {}
            (ObjectiveCaseRole::NegativeControl, Some(positive))
                if positive != self.id
                    && case_roles.get(positive) == Some(&ObjectiveCaseRole::Positive) => {}
            (ObjectiveCaseRole::Positive, Some(_)) => {
                return Err(suite_error(format!(
                    "positive case {} cannot declare control_for",
                    self.id
                )));
            }
            (ObjectiveCaseRole::NegativeControl, _) => {
                return Err(suite_error(format!(
                    "negative case {} must reference an existing positive case",
                    self.id
                )));
            }
        }
        self.boot.validate()?;
        self.scenario.validate("scenario")?;
        self.objective.validate()?;
        self.observation_view.validate()?;
        self.action_schema.validate("action schema")?;
        self.observation_requirements
            .validate()
            .map_err(|error| suite_error(format!("case {}: {error}", self.id)))?;
        self.seed.validate()?;
        if !(1..=MAX_LOGICAL_TICKS).contains(&self.logical_tick_budget)
            || !(1..=MAX_HOST_TIMEOUT_SECONDS).contains(&self.host_timeout_seconds)
            || !(2..=MAX_REPETITIONS).contains(&self.repetitions)
        {
            return Err(suite_error(format!(
                "case {} budgets or repetition count are outside the supported bounds",
                self.id
            )));
        }
        if self.role == ObjectiveCaseRole::Positive
            && self.expected_terminal == ExpectedTerminalClass::ObjectiveMiss
        {
            return Err(suite_error(format!(
                "positive case {} cannot expect an objective miss",
                self.id
            )));
        }
        if self.role == ObjectiveCaseRole::NegativeControl
            && self.expected_terminal != ExpectedTerminalClass::ObjectiveMiss
        {
            return Err(suite_error(format!(
                "negative case {} must expect an objective miss",
                self.id
            )));
        }
        Ok(())
    }

    pub(crate) fn validate_bound_files(&self, root: &Path) -> Result<(), ObjectiveSuiteError> {
        validate_case_files(self, root)
    }

    pub(crate) fn validate_bound_structure(&self) -> Result<(), ObjectiveSuiteError> {
        let roles = BTreeMap::from([(self.id.as_str(), ObjectiveCaseRole::Positive)]);
        self.validate(&roles)
    }
}

impl ObjectiveBoot {
    fn validate(&self) -> Result<(), ObjectiveSuiteError> {
        let boot = self.to_tape_boot();
        InputTape {
            boot,
            ..InputTape::default()
        }
        .validate()
        .map_err(|error| suite_error(format!("invalid objective-suite boot: {error}")))
    }

    fn to_tape_boot(&self) -> TapeBoot {
        match self {
            Self::Process => TapeBoot::Process,
            Self::Stage {
                stage,
                room,
                point,
                layer,
                save_slot,
            } => TapeBoot::Stage {
                stage: stage.clone(),
                room: *room,
                point: *point,
                layer: *layer,
                save_slot: *save_slot,
                fixture: None,
            },
        }
    }

    fn matches_tape(&self, tape: &TapeBoot) -> bool {
        match (self, tape) {
            (Self::Process, TapeBoot::Process) => true,
            (
                Self::Stage {
                    stage,
                    room,
                    point,
                    layer,
                    save_slot,
                },
                TapeBoot::Stage {
                    stage: actual_stage,
                    room: actual_room,
                    point: actual_point,
                    layer: actual_layer,
                    save_slot: actual_save_slot,
                    ..
                },
            ) => {
                stage == actual_stage
                    && room == actual_room
                    && point == actual_point
                    && layer == actual_layer
                    && save_slot == actual_save_slot
            }
            _ => false,
        }
    }
}

impl ArtifactReference {
    fn validate(&self, label: &str) -> Result<(), ObjectiveSuiteError> {
        validate_relative_path(label, &self.path)?;
        if self.sha256 == Digest::ZERO {
            return Err(suite_error(format!("{label} digest must be nonzero")));
        }
        Ok(())
    }
}

impl ObjectiveProgramReference {
    fn validate(&self) -> Result<(), ObjectiveSuiteError> {
        self.source.validate("objective source")?;
        validate_id("objective goal", &self.goal)?;
        if self.program_sha256 == Digest::ZERO {
            return Err(suite_error("objective program digest must be nonzero"));
        }
        Ok(())
    }
}

impl ObservationViewReference {
    fn validate(&self) -> Result<(), ObjectiveSuiteError> {
        self.source.validate("observation view source")?;
        if self.schema_sha256 == Digest::ZERO {
            return Err(suite_error("observation view digest must be nonzero"));
        }
        Ok(())
    }
}

impl SchemaIdentity {
    fn validate(&self, label: &str) -> Result<(), ObjectiveSuiteError> {
        validate_id(label, &self.id)?;
        if self.sha256 == Digest::ZERO {
            return Err(suite_error(format!("{label} digest must be nonzero")));
        }
        Ok(())
    }
}

impl ObjectiveSeed {
    fn validate(&self) -> Result<(), ObjectiveSuiteError> {
        match self {
            Self::Neutral => Ok(()),
            Self::Tape { artifact } => artifact.validate("seed tape"),
            Self::TapeSource { artifact } => artifact.validate("seed tape source"),
            Self::Controller { artifact } => artifact.validate("seed controller"),
        }
    }
}

fn validate_case_files(case: &ObjectiveSuiteCase, root: &Path) -> Result<(), ObjectiveSuiteError> {
    let scenario_bytes = read_artifact(root, &case.scenario, "scenario")?;
    let scenario: ScenarioFixture = serde_json::from_slice(&scenario_bytes)
        .map_err(|error| suite_error(format!("case {} scenario is invalid: {error}", case.id)))?;
    scenario
        .validate()
        .map_err(|error| suite_error(format!("case {} scenario is invalid: {error}", case.id)))?;

    let objective_bytes = read_artifact(root, &case.objective.source, "objective source")?;
    let objective_source = std::str::from_utf8(&objective_bytes).map_err(|error| {
        suite_error(format!(
            "case {} objective source is not UTF-8: {error}",
            case.id
        ))
    })?;
    let objective_program = milestone_dsl::parse(objective_source)
        .map_err(|error| suite_error(format!("case {} objective is invalid: {error}", case.id)))?;
    let compiled = milestone_dsl::compile(&objective_program)
        .map_err(|error| suite_error(format!("case {} objective is invalid: {error}", case.id)))?;
    if Digest(compiled.program_sha256) != case.objective.program_sha256
        || !compiled
            .definitions
            .iter()
            .any(|definition| definition.name == case.objective.goal)
    {
        return Err(suite_error(format!(
            "case {} objective identity or goal is stale",
            case.id
        )));
    }
    let required_facts =
        milestone_dsl::required_query_facts(&objective_program, &case.objective.goal).map_err(
            |error| suite_error(format!("case {} objective is invalid: {error}", case.id)),
        )?;
    if required_facts != case.observation_requirements.facts {
        return Err(suite_error(format!(
            "case {} observation facts do not exactly match objective dependencies",
            case.id
        )));
    }

    let observation_bytes = read_artifact(
        root,
        &case.observation_view.source,
        "observation view source",
    )?;
    let observation: ObservationSpec =
        serde_json::from_slice(&observation_bytes).map_err(|error| {
            suite_error(format!(
                "case {} observation view is invalid: {error}",
                case.id
            ))
        })?;
    observation.validate().map_err(|error| {
        suite_error(format!(
            "case {} observation view is invalid: {error}",
            case.id
        ))
    })?;
    if observation.digest().map_err(|error| {
        suite_error(format!(
            "case {} observation view is invalid: {error}",
            case.id
        ))
    })? != case.observation_view.schema_sha256
    {
        return Err(suite_error(format!(
            "case {} observation view identity is stale",
            case.id
        )));
    }
    if observation.objective.id != case.objective.goal {
        return Err(suite_error(format!(
            "case {} observation view names a different objective",
            case.id
        )));
    }

    match &case.seed {
        ObjectiveSeed::Neutral => {}
        ObjectiveSeed::Tape { artifact } => {
            let bytes = read_artifact(root, artifact, "seed tape")?;
            let decoded = InputTape::decode(&bytes).map_err(|error| {
                suite_error(format!("case {} seed tape is invalid: {error}", case.id))
            })?;
            validate_seed_tape(case, &decoded.tape, &scenario)?;
        }
        ObjectiveSeed::TapeSource { artifact } => {
            let bytes = read_artifact(root, artifact, "seed tape source")?;
            let source = std::str::from_utf8(&bytes).map_err(|error| {
                suite_error(format!(
                    "case {} seed tape source is not UTF-8: {error}",
                    case.id
                ))
            })?;
            let program: TapeProgram = tape_dsl::parse(source).map_err(|error| {
                suite_error(format!(
                    "case {} seed tape source is invalid: {error}",
                    case.id
                ))
            })?;
            let compiled = program.compile().map_err(|error| {
                suite_error(format!(
                    "case {} seed tape source is invalid: {error}",
                    case.id
                ))
            })?;
            validate_seed_tape(case, &compiled.tape, &scenario)?;
        }
        ObjectiveSeed::Controller { artifact } => {
            let bytes = read_artifact(root, artifact, "seed controller")?;
            ControllerProgram::decode(&bytes).map_err(|error| {
                suite_error(format!(
                    "case {} seed controller is invalid: {error}",
                    case.id
                ))
            })?;
        }
    }
    Ok(())
}

fn validate_seed_tape(
    case: &ObjectiveSuiteCase,
    tape: &InputTape,
    scenario: &ScenarioFixture,
) -> Result<(), ObjectiveSuiteError> {
    if !case.boot.matches_tape(&tape.boot) {
        return Err(suite_error(format!(
            "case {} seed tape boot does not match the declared boot",
            case.id
        )));
    }
    if let TapeBoot::Stage {
        fixture: Some(embedded),
        ..
    } = &tape.boot
        && embedded != scenario
    {
        return Err(suite_error(format!(
            "case {} seed tape embeds a different scenario fixture",
            case.id
        )));
    }
    Ok(())
}

fn read_artifact(
    root: &Path,
    reference: &ArtifactReference,
    label: &str,
) -> Result<Vec<u8>, ObjectiveSuiteError> {
    let path = root.join(&reference.path);
    let canonical = path.canonicalize().map_err(|error| {
        suite_error(format!(
            "cannot resolve {label} {}: {error}",
            path.display()
        ))
    })?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(suite_error(format!(
            "{label} escapes the repository or is not a file: {}",
            reference.path
        )));
    }
    let bytes = fs::read(&canonical).map_err(|error| {
        suite_error(format!(
            "cannot read {label} {}: {error}",
            canonical.display()
        ))
    })?;
    let actual = Digest(Sha256::digest(&bytes).into());
    if actual != reference.sha256 {
        return Err(suite_error(format!(
            "{label} digest is stale for {}",
            reference.path
        )));
    }
    Ok(bytes)
}

fn validate_relative_path(label: &str, value: &str) -> Result<(), ObjectiveSuiteError> {
    let path = PathBuf::from(value);
    if value.is_empty()
        || value.contains('\\')
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::CurDir
                    | Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        })
    {
        return Err(suite_error(format!(
            "{label} path must be a canonical repository-relative path"
        )));
    }
    Ok(())
}

fn validate_id(label: &str, value: &str) -> Result<(), ObjectiveSuiteError> {
    if value.is_empty()
        || value.len() > 192
        || value != value.trim()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b'/')
        })
    {
        return Err(suite_error(format!(
            "{label} is not a canonical identifier"
        )));
    }
    Ok(())
}

fn validate_text(label: &str, value: &str) -> Result<(), ObjectiveSuiteError> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value != value.trim() {
        return Err(suite_error(format!("{label} is empty or too large")));
    }
    Ok(())
}

#[derive(Debug)]
pub struct ObjectiveSuiteError(String);

impl fmt::Display for ObjectiveSuiteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ObjectiveSuiteError {}

fn suite_error(message: impl Into<String>) -> ObjectiveSuiteError {
    ObjectiveSuiteError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation_view::movement_state_v2_spec;
    use crate::scenario_fixture::SCENARIO_FIXTURE_SCHEMA;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;

    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);

    fn sha256(bytes: &[u8]) -> Digest {
        Digest(Sha256::digest(bytes).into())
    }

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "huntctl-objective-suite-{}-{}-{}",
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::process::id(),
            NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed),
        ))
    }

    fn write_inputs(
        root: &Path,
    ) -> (
        ArtifactReference,
        ObjectiveProgramReference,
        ObservationViewReference,
    ) {
        fs::create_dir_all(root.join("harness")).unwrap();
        let scenario = ScenarioFixture {
            schema: SCENARIO_FIXTURE_SCHEMA.into(),
            name: "stage-ready".into(),
            form: None,
            health: None,
            rng: Vec::new(),
            video_mode: None,
            inventory: Vec::new(),
            equipment: Vec::new(),
            flags: Vec::new(),
            settings: Vec::new(),
        };
        let scenario_bytes = serde_json::to_vec_pretty(&scenario).unwrap();
        fs::write(root.join("harness/scenario.json"), &scenario_bytes).unwrap();

        let objective_bytes = b"milestones 1.0\n\nmilestone stage_ready {\n  phase post_sim\n  when stage.name == \"F_SP103\" && player.exists\n}\n";
        fs::write(root.join("harness/objective.milestones"), objective_bytes).unwrap();
        let compiled = milestone_dsl::compile(
            &milestone_dsl::parse(std::str::from_utf8(objective_bytes).unwrap()).unwrap(),
        )
        .unwrap();

        let mut observation = movement_state_v2_spec();
        observation.objective.id = "stage_ready".into();
        let observation_bytes = serde_json::to_vec_pretty(&observation).unwrap();
        fs::write(root.join("harness/observation.json"), &observation_bytes).unwrap();
        (
            ArtifactReference {
                path: "harness/scenario.json".into(),
                sha256: sha256(&scenario_bytes),
            },
            ObjectiveProgramReference {
                source: ArtifactReference {
                    path: "harness/objective.milestones".into(),
                    sha256: sha256(objective_bytes),
                },
                program_sha256: Digest(compiled.program_sha256),
                goal: "stage_ready".into(),
            },
            ObservationViewReference {
                source: ArtifactReference {
                    path: "harness/observation.json".into(),
                    sha256: sha256(&observation_bytes),
                },
                schema_sha256: observation.digest().unwrap(),
            },
        )
    }

    fn suite(root: &Path) -> ObjectiveSuite {
        let (scenario, objective, observation_view) = write_inputs(root);
        let positive = ObjectiveSuiteCase {
            id: "stage-ready".into(),
            description: "Prove a stage fixture reaches its declared ready state.".into(),
            role: ObjectiveCaseRole::Positive,
            control_for: None,
            boot: ObjectiveBoot::Stage {
                stage: "F_SP103".into(),
                room: 0,
                point: 0,
                layer: 0,
                save_slot: None,
            },
            scenario: scenario.clone(),
            objective: objective.clone(),
            observation_view: observation_view.clone(),
            action_schema: SchemaIdentity {
                id: "neutral/v1".into(),
                sha256: Digest([9; 32]),
            },
            observation_requirements: ObjectiveObservationRequirements {
                schema: crate::harness::observation_contract::OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1
                    .into(),
                families: vec![
                    crate::harness::observation_contract::ObservationFamilyRequirement {
                        id: "player_motion".into(),
                        minimum_version: 1,
                    },
                    crate::harness::observation_contract::ObservationFamilyRequirement {
                        id: "stage".into(),
                        minimum_version: 1,
                    },
                ],
                facts: vec!["player.exists".into(), "stage.name".into()],
            },
            seed: ObjectiveSeed::Neutral,
            logical_tick_budget: 300,
            host_timeout_seconds: 30,
            repetitions: 2,
            expected_terminal: ExpectedTerminalClass::Reached,
        };
        let mut negative = positive.clone();
        negative.id = "stage-ready-wrong-stage".into();
        negative.description = "The wrong stage must not satisfy stage-ready.".into();
        negative.role = ObjectiveCaseRole::NegativeControl;
        negative.control_for = Some(positive.id.clone());
        negative.expected_terminal = ExpectedTerminalClass::ObjectiveMiss;
        let mut suite = ObjectiveSuite {
            schema: OBJECTIVE_SUITE_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            id: "core-conformance/v1".into(),
            description: "Cheap end-to-end harness conformance cases.".into(),
            cases: vec![positive, negative],
        };
        suite.cases.sort_by(|left, right| left.id.cmp(&right.id));
        suite.refresh_content_sha256().unwrap();
        suite
    }

    #[test]
    fn validates_bound_files_and_reports_control_counts() {
        let root = temp_root();
        let suite = suite(&root);
        let report = suite.validate_files(&root).unwrap();
        assert_eq!(report.positive_count, 1);
        assert_eq!(report.negative_control_count, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_stale_files_and_detached_controls() {
        let root = temp_root();
        let mut suite = suite(&root);
        fs::write(root.join("harness/scenario.json"), b"{}").unwrap();
        assert!(suite.validate_files(&root).is_err());

        suite.cases[0].control_for = Some("missing".into());
        suite.refresh_content_sha256().unwrap();
        assert!(suite.validate().is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_path_escape_zero_identity_and_single_repetition() {
        let root = temp_root();
        let mut suite = suite(&root);
        let positive = suite
            .cases
            .iter_mut()
            .find(|case| case.role == ObjectiveCaseRole::Positive)
            .unwrap();
        positive.scenario.path = "../scenario.json".into();
        positive.action_schema.sha256 = Digest::ZERO;
        positive.repetitions = 1;
        suite.refresh_content_sha256().unwrap();
        assert!(suite.validate().is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_observation_facts_not_used_by_the_objective() {
        let root = temp_root();
        let mut suite = suite(&root);
        suite.cases[0]
            .observation_requirements
            .facts
            .insert(1, "player.position.x".into());
        suite.refresh_content_sha256().unwrap();
        assert!(suite.validate().is_ok());
        assert!(suite.validate_files(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
