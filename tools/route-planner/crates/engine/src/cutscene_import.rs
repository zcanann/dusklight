//! Conservative joins from retail event-wrapper records into planner topology.
//!
//! This layer proves only the outer wrapper encoded by DZS/DZR REVT, LBNK,
//! SCLS, and `event_list.dat`. It intentionally does not turn an undecoded STB
//! or an untraced load failure into executable cutscene effects.

use crate::artifact::Digest;
use crate::orig_extraction::{
    ExtractedEventCut, ExtractedEventData, ExtractedEventDataValue, ExtractedEventList,
    ExtractedSceneTransition, ExtractedStageData,
};
use crate::{PlannerContractError, canonical_json, validate_label};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const CUTSCENE_WRAPPER_TOPOLOGY_SCHEMA: &str =
    "dusklight.route-planner.cutscene-wrapper-topology/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneWrapperSourceIdentity {
    pub stage_archive_sha256: Digest,
    pub stage_resource_sha256: Digest,
    pub event_list_resource_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutsceneWrapperCoverageStatus {
    Extracted,
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneWrapperCoverage {
    pub outer_event_wrapper: CutsceneWrapperCoverageStatus,
    pub jstudio_phase_program: CutsceneWrapperCoverageStatus,
    pub resource_failure_control_flow: CutsceneWrapperCoverageStatus,
    pub return_place_writers: CutsceneWrapperCoverageStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneWrapperTopology {
    pub schema: String,
    pub source: CutsceneWrapperSourceIdentity,
    pub event_name: String,
    pub layer: u8,
    pub map_event_record_index: u32,
    pub event_type: u8,
    pub map_tool_id: u8,
    pub director_map_tool_id: i32,
    pub event_list_index: u32,
    pub demo_archive_name: String,
    pub package_stb_file: String,
    pub staff_paths: Vec<CutsceneWrapperStaffPath>,
    pub normal_exit: Option<CutsceneWrapperExit>,
    pub skip_exit: Option<CutsceneWrapperExit>,
    pub coverage: CutsceneWrapperCoverage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneWrapperStaffPath {
    pub staff_index: u32,
    pub name: String,
    pub staff_type: i32,
    pub cuts: Vec<CutsceneWrapperCut>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneWrapperCut {
    pub cut_index: u32,
    pub name: String,
    pub parameters: Vec<ExtractedEventData>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutsceneWrapperExitKind {
    Normal,
    Skip,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneWrapperExit {
    pub kind: CutsceneWrapperExitKind,
    pub transition: ExtractedSceneTransition,
}

impl CutsceneWrapperTopology {
    pub fn build(
        source: CutsceneWrapperSourceIdentity,
        stage: &ExtractedStageData,
        event_list: &ExtractedEventList,
        event_name: &str,
        layer: u8,
    ) -> Result<Self, PlannerContractError> {
        source.validate()?;
        validate_label("cutscene_wrapper.event_name", event_name)?;

        let map_event = exactly_one(
            stage
                .map_events
                .iter()
                .filter(|event| event.event_name.as_deref() == Some(event_name)),
            "cutscene_wrapper.map_event",
        )?;
        let event = exactly_one(
            event_list
                .events
                .iter()
                .filter(|event| event.name == event_name),
            "cutscene_wrapper.event_list_event",
        )?;
        let demo_bank = exactly_one(
            stage
                .demo_archive_banks
                .iter()
                .filter(|bank| bank.layer == layer),
            "cutscene_wrapper.demo_archive_bank",
        )?;
        let demo_archive_name = demo_bank.archive_name.clone().ok_or_else(|| {
            PlannerContractError::new(
                "cutscene_wrapper.demo_archive_bank",
                "does not select a demo archive",
            )
        })?;

        let mut staff_paths = Vec::with_capacity(event.staff_indices.len());
        for staff_index in &event.staff_indices {
            let staff = event_list
                .staff
                .get(*staff_index as usize)
                .filter(|staff| staff.index == *staff_index)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "cutscene_wrapper.staff",
                        "event references a missing or noncanonical staff record",
                    )
                })?;
            let cuts = collect_cut_path(event_list, staff.start_cut_index)?;
            staff_paths.push(CutsceneWrapperStaffPath {
                staff_index: *staff_index,
                name: staff.name.clone(),
                staff_type: staff.staff_type,
                cuts,
            });
        }

        let (package_stb_file, director_map_tool_id) = wrapper_parameters(&staff_paths)?;
        if director_map_tool_id != i32::from(map_event.map_tool_id) {
            return Err(PlannerContractError::new(
                "cutscene_wrapper.director_map_tool_id",
                "does not match the REVT map-tool ID",
            ));
        }

        let topology = Self {
            schema: CUTSCENE_WRAPPER_TOPOLOGY_SCHEMA.into(),
            source,
            event_name: event_name.into(),
            layer,
            map_event_record_index: map_event.record_index,
            event_type: map_event.event_type,
            map_tool_id: map_event.map_tool_id,
            director_map_tool_id,
            event_list_index: event.index,
            demo_archive_name,
            package_stb_file: package_stb_file.into(),
            staff_paths,
            normal_exit: resolve_exit(
                stage,
                map_event.normal_exit_id,
                CutsceneWrapperExitKind::Normal,
            )?,
            skip_exit: resolve_exit(stage, map_event.skip_exit_id, CutsceneWrapperExitKind::Skip)?,
            coverage: CutsceneWrapperCoverage {
                outer_event_wrapper: CutsceneWrapperCoverageStatus::Extracted,
                jstudio_phase_program: CutsceneWrapperCoverageStatus::Unresolved,
                resource_failure_control_flow: CutsceneWrapperCoverageStatus::Unresolved,
                return_place_writers: CutsceneWrapperCoverageStatus::Unresolved,
            },
        };
        topology.validate()?;
        Ok(topology)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != CUTSCENE_WRAPPER_TOPOLOGY_SCHEMA {
            return Err(PlannerContractError::new(
                "cutscene_wrapper.schema",
                "is unsupported",
            ));
        }
        self.source.validate()?;
        validate_label("cutscene_wrapper.event_name", &self.event_name)?;
        validate_label(
            "cutscene_wrapper.demo_archive_name",
            &self.demo_archive_name,
        )?;
        validate_label("cutscene_wrapper.package_stb_file", &self.package_stb_file)?;
        if self.staff_paths.is_empty() {
            return Err(PlannerContractError::new(
                "cutscene_wrapper.staff_paths",
                "must not be empty",
            ));
        }
        let mut prior_staff_index = None;
        for staff in &self.staff_paths {
            validate_label("cutscene_wrapper.staff.name", &staff.name)?;
            if prior_staff_index.is_some_and(|prior| prior >= staff.staff_index)
                || staff.cuts.is_empty()
            {
                return Err(PlannerContractError::new(
                    "cutscene_wrapper.staff_paths",
                    "must be ordered by unique staff index and contain cuts",
                ));
            }
            prior_staff_index = Some(staff.staff_index);
            for cut in &staff.cuts {
                validate_label("cutscene_wrapper.cut.name", &cut.name)?;
            }
        }
        let (package_stb_file, director_map_tool_id) = wrapper_parameters(&self.staff_paths)?;
        if package_stb_file != self.package_stb_file
            || director_map_tool_id != self.director_map_tool_id
            || director_map_tool_id != i32::from(self.map_tool_id)
        {
            return Err(PlannerContractError::new(
                "cutscene_wrapper.parameters",
                "do not match the retained package and REVT map-tool coordinates",
            ));
        }
        if self.coverage.outer_event_wrapper != CutsceneWrapperCoverageStatus::Extracted
            || self.coverage.jstudio_phase_program != CutsceneWrapperCoverageStatus::Unresolved
            || self.coverage.resource_failure_control_flow
                != CutsceneWrapperCoverageStatus::Unresolved
            || self.coverage.return_place_writers != CutsceneWrapperCoverageStatus::Unresolved
        {
            return Err(PlannerContractError::new(
                "cutscene_wrapper.coverage",
                "must not overstate the v1 extractor's evidence boundary",
            ));
        }
        validate_exit(self.normal_exit.as_ref(), CutsceneWrapperExitKind::Normal)?;
        validate_exit(self.skip_exit.as_ref(), CutsceneWrapperExitKind::Skip)?;
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let topology: Self = serde_json::from_slice(bytes)?;
        topology.validate()?;
        if topology.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "cutscene_wrapper",
                "is not canonical JSON",
            ));
        }
        Ok(topology)
    }
}

impl CutsceneWrapperSourceIdentity {
    fn validate(&self) -> Result<(), PlannerContractError> {
        if self.stage_archive_sha256 == Digest::ZERO
            || self.stage_resource_sha256 == Digest::ZERO
            || self.event_list_resource_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "cutscene_wrapper.source",
                "must contain nonzero exact resource identities",
            ));
        }
        Ok(())
    }
}

fn collect_cut_path(
    event_list: &ExtractedEventList,
    start_cut_index: u32,
) -> Result<Vec<CutsceneWrapperCut>, PlannerContractError> {
    let mut current = Some(start_cut_index);
    let mut visited = BTreeSet::new();
    let mut cuts = Vec::new();
    while let Some(index) = current {
        if !visited.insert(index) {
            return Err(PlannerContractError::new(
                "cutscene_wrapper.cut_path",
                "contains a cycle",
            ));
        }
        let cut = event_list
            .cuts
            .get(index as usize)
            .filter(|cut| cut.index == index)
            .ok_or_else(|| {
                PlannerContractError::new(
                    "cutscene_wrapper.cut_path",
                    "references a missing or noncanonical cut",
                )
            })?;
        cuts.push(CutsceneWrapperCut {
            cut_index: index,
            name: cut.name.clone(),
            parameters: collect_data_path(event_list, cut)?,
        });
        current = cut.next_cut_index;
    }
    Ok(cuts)
}

fn collect_data_path(
    event_list: &ExtractedEventList,
    cut: &ExtractedEventCut,
) -> Result<Vec<ExtractedEventData>, PlannerContractError> {
    let mut current = cut.data_index;
    let mut visited = BTreeSet::new();
    let mut data = Vec::new();
    while let Some(index) = current {
        if !visited.insert(index) {
            return Err(PlannerContractError::new(
                "cutscene_wrapper.data_path",
                "contains a cycle",
            ));
        }
        let parameter = event_list
            .data
            .get(index as usize)
            .filter(|parameter| parameter.index == index)
            .ok_or_else(|| {
                PlannerContractError::new(
                    "cutscene_wrapper.data_path",
                    "references a missing or noncanonical data record",
                )
            })?;
        data.push(parameter.clone());
        current = parameter.next_data_index;
    }
    Ok(data)
}

fn string_parameter<'a>(
    cut: &'a CutsceneWrapperCut,
    name: &str,
) -> Result<&'a str, PlannerContractError> {
    let parameter = exactly_one(
        cut.parameters
            .iter()
            .filter(|parameter| parameter.name == name),
        "cutscene_wrapper.string_parameter",
    )?;
    match &parameter.value {
        ExtractedEventDataValue::StringBytes {
            ascii: Some(value), ..
        } => Ok(value),
        _ => Err(PlannerContractError::new(
            "cutscene_wrapper.string_parameter",
            "is not a decoded ASCII string",
        )),
    }
}

fn wrapper_parameters(
    staff_paths: &[CutsceneWrapperStaffPath],
) -> Result<(&str, i32), PlannerContractError> {
    let mut package_stb_files = Vec::new();
    let mut director_map_tool_ids = Vec::new();
    for staff in staff_paths {
        for cut in &staff.cuts {
            if staff.name == "PACKAGE" && cut.name == "PLAY" {
                package_stb_files.push(string_parameter(cut, "FileName")?);
            }
            if staff.name == "DIRECTOR" && cut.name == "MAPTOOL" {
                director_map_tool_ids.push(integer_parameter(cut, "ID")?);
            }
        }
    }
    Ok((
        exactly_one(
            package_stb_files.into_iter(),
            "cutscene_wrapper.package_stb_file",
        )?,
        exactly_one(
            director_map_tool_ids.into_iter(),
            "cutscene_wrapper.director_map_tool_id",
        )?,
    ))
}

fn integer_parameter(cut: &CutsceneWrapperCut, name: &str) -> Result<i32, PlannerContractError> {
    let parameter = exactly_one(
        cut.parameters
            .iter()
            .filter(|parameter| parameter.name == name),
        "cutscene_wrapper.integer_parameter",
    )?;
    match parameter.value {
        ExtractedEventDataValue::Integers { ref values } if values.len() == 1 => Ok(values[0]),
        _ => Err(PlannerContractError::new(
            "cutscene_wrapper.integer_parameter",
            "is not exactly one integer",
        )),
    }
}

fn resolve_exit(
    stage: &ExtractedStageData,
    exit_id: Option<u8>,
    kind: CutsceneWrapperExitKind,
) -> Result<Option<CutsceneWrapperExit>, PlannerContractError> {
    let Some(exit_id) = exit_id else {
        return Ok(None);
    };
    let transition = exactly_one(
        stage
            .scene_transitions
            .iter()
            .filter(|transition| transition.exit_id == u32::from(exit_id)),
        "cutscene_wrapper.exit",
    )?;
    Ok(Some(CutsceneWrapperExit {
        kind,
        transition: transition.clone(),
    }))
}

fn validate_exit(
    exit: Option<&CutsceneWrapperExit>,
    expected_kind: CutsceneWrapperExitKind,
) -> Result<(), PlannerContractError> {
    if exit.is_some_and(|exit| exit.kind != expected_kind) {
        return Err(PlannerContractError::new(
            "cutscene_wrapper.exit.kind",
            "does not match its topology field",
        ));
    }
    Ok(())
}

fn exactly_one<T>(
    mut values: impl Iterator<Item = T>,
    field: &'static str,
) -> Result<T, PlannerContractError> {
    let value = values
        .next()
        .ok_or_else(|| PlannerContractError::new(field, "has no matching source record"))?;
    if values.next().is_some() {
        return Err(PlannerContractError::new(
            field,
            "has more than one matching source record",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orig_extraction::{
        ExtractedDemoArchiveBank, ExtractedEvent, ExtractedEventStaff, ExtractedMapEvent,
    };

    fn source() -> CutsceneWrapperSourceIdentity {
        CutsceneWrapperSourceIdentity {
            stage_archive_sha256: Digest([1; 32]),
            stage_resource_sha256: Digest([2; 32]),
            event_list_resource_sha256: Digest([3; 32]),
        }
    }

    fn stage() -> ExtractedStageData {
        ExtractedStageData {
            chunks: Vec::new(),
            stage_information: None,
            room_transforms: Vec::new(),
            file_lists: Vec::new(),
            room_read_table: Vec::new(),
            cameras: Vec::new(),
            camera_arrows: Vec::new(),
            paths: Vec::new(),
            path_points: Vec::new(),
            scene_transitions: vec![
                ExtractedSceneTransition {
                    exit_id: 1,
                    destination_stage: "F_SP116".into(),
                    destination_spawn: 20,
                    destination_room: 0,
                    scene_layer: Some(8),
                    time_hour: None,
                    wipe: 24,
                    wipe_time: 22,
                    raw_hex: "normal".into(),
                },
                ExtractedSceneTransition {
                    exit_id: 2,
                    destination_stage: "R_SP107".into(),
                    destination_spawn: 1,
                    destination_room: 3,
                    scene_layer: None,
                    time_hour: None,
                    wipe: 63,
                    wipe_time: 0,
                    raw_hex: "skip".into(),
                },
            ],
            map_events: vec![ExtractedMapEvent {
                record_index: 0,
                event_type: 2,
                map_tool_id: 4,
                priority: 100,
                normal_exit_id: Some(1),
                skip_exit_id: Some(2),
                event_name: Some("demo07_02".into()),
                switch_no: None,
                raw_hex: "revt".into(),
            }],
            demo_archive_banks: vec![ExtractedDemoArchiveBank {
                layer: 8,
                bank: Some(7),
                bank2: Some(2),
                archive_name: Some("Demo07_02".into()),
                raw_hex: "0702ff".into(),
            }],
            actor_placements: Vec::new(),
            treasure_placements: Vec::new(),
            player_spawns: Vec::new(),
        }
    }

    fn data(index: u32, name: &str, value: ExtractedEventDataValue) -> ExtractedEventData {
        ExtractedEventData {
            index,
            name: name.into(),
            data_type: match value {
                ExtractedEventDataValue::Integers { .. } => 3,
                ExtractedEventDataValue::StringBytes { .. } => 4,
                _ => unreachable!(),
            },
            value_index: 0,
            value_count: 1,
            next_data_index: None,
            value,
            raw_hex: format!("data{index}"),
        }
    }

    fn event_list() -> ExtractedEventList {
        ExtractedEventList {
            resource_size: 1,
            events: vec![ExtractedEvent {
                index: 0,
                name: "demo07_02".into(),
                priority: 100,
                staff_indices: vec![0, 1],
                finish_flags: [-1; 3],
                raw_hex: "event".into(),
            }],
            staff: vec![
                ExtractedEventStaff {
                    index: 0,
                    name: "PACKAGE".into(),
                    tag_id: 0,
                    flag_id: 3,
                    staff_type: 11,
                    start_cut_index: 0,
                    raw_hex: "package".into(),
                },
                ExtractedEventStaff {
                    index: 1,
                    name: "DIRECTOR".into(),
                    tag_id: 0,
                    flag_id: 10,
                    staff_type: 6,
                    start_cut_index: 2,
                    raw_hex: "director".into(),
                },
            ],
            cuts: vec![
                ExtractedEventCut {
                    index: 0,
                    name: "PLAY".into(),
                    tag_id: 0,
                    start_flags: [-1; 3],
                    flag_id: 3,
                    data_index: Some(0),
                    next_cut_index: Some(1),
                    raw_hex: "play".into(),
                },
                ExtractedEventCut {
                    index: 1,
                    name: "WAIT".into(),
                    tag_id: 1,
                    start_flags: [-1; 3],
                    flag_id: 5,
                    data_index: None,
                    next_cut_index: None,
                    raw_hex: "wait".into(),
                },
                ExtractedEventCut {
                    index: 2,
                    name: "MAPTOOL".into(),
                    tag_id: 2,
                    start_flags: [-1; 3],
                    flag_id: 10,
                    data_index: Some(1),
                    next_cut_index: None,
                    raw_hex: "maptool".into(),
                },
            ],
            data: vec![
                data(
                    0,
                    "FileName",
                    ExtractedEventDataValue::StringBytes {
                        raw_hex: "demo".into(),
                        ascii: Some("demo07_02.stb".into()),
                    },
                ),
                data(
                    1,
                    "ID",
                    ExtractedEventDataValue::Integers { values: vec![4] },
                ),
            ],
            float_data_bits: Vec::new(),
            integer_data: vec![4],
            string_data_hex: "demo".into(),
        }
    }

    #[test]
    fn joins_wrapper_resources_without_claiming_stb_or_failure_semantics() {
        let topology =
            CutsceneWrapperTopology::build(source(), &stage(), &event_list(), "demo07_02", 8)
                .unwrap();
        assert_eq!(topology.demo_archive_name, "Demo07_02");
        assert_eq!(topology.package_stb_file, "demo07_02.stb");
        assert_eq!(
            topology
                .normal_exit
                .as_ref()
                .unwrap()
                .transition
                .destination_stage,
            "F_SP116"
        );
        assert_eq!(
            topology
                .skip_exit
                .as_ref()
                .unwrap()
                .transition
                .destination_stage,
            "R_SP107"
        );
        assert_eq!(
            topology.coverage.resource_failure_control_flow,
            CutsceneWrapperCoverageStatus::Unresolved
        );
        assert_eq!(
            CutsceneWrapperTopology::decode_canonical(&topology.canonical_bytes().unwrap())
                .unwrap(),
            topology
        );
    }

    #[test]
    fn rejects_map_tool_mismatch_and_ambiguous_exits() {
        let mut mismatched = event_list();
        mismatched.data[1].value = ExtractedEventDataValue::Integers { values: vec![9] };
        assert_eq!(
            CutsceneWrapperTopology::build(source(), &stage(), &mismatched, "demo07_02", 8,)
                .unwrap_err()
                .field(),
            "cutscene_wrapper.director_map_tool_id"
        );

        let mut ambiguous = stage();
        ambiguous
            .scene_transitions
            .push(ambiguous.scene_transitions[0].clone());
        assert_eq!(
            CutsceneWrapperTopology::build(source(), &ambiguous, &event_list(), "demo07_02", 8,)
                .unwrap_err()
                .field(),
            "cutscene_wrapper.exit"
        );
    }
}
