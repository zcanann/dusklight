//! Portable, inspectable behavior descriptors for novelty search.
//!
//! The descriptor deliberately keeps semantic facts rather than only a digest.
//! Search code may index the per-axis identities, while discovery artifacts can
//! retain this value to explain exactly why an episode occupied a new cell.

use crate::trace::{
    DecodedTrace, TraceCollisionBackingFormat, TraceCollisionSurfaceKind, TraceRecord,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const SEMANTIC_NOVELTY_SCHEMA: &str = "dusklight-semantic-novelty/v1";
pub const MAX_SEMANTIC_SEQUENCE_STATES: usize = 4_096;
const POSITION_EXTREMA_BIN_WORLD_UNITS: f32 = 16.0;
const VELOCITY_EXTREMA_BIN_WORLD_UNITS: f32 = 1.0 / 16.0;
const ACTOR_RELATION_BIN_WORLD_UNITS: f32 = 128.0;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SemanticState {
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
    pub player_procedure: Option<u16>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct StateTransitionFact {
    pub from: SemanticState,
    pub to: SemanticState,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EventFact {
    pub event_id: i16,
    pub mode: u8,
    pub status: u8,
    pub map_tool_id: u8,
    pub name_hash: Option<u32>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ContactFact {
    pub kind: &'static str,
    pub wall_slot: u8,
    pub backing: Option<&'static str>,
    pub bg_index: Option<u16>,
    pub poly_index: Option<u16>,
    pub material_row: Option<u16>,
    pub group_row: Option<u16>,
    pub raw_exit_id: Option<u8>,
    pub source_room: Option<i8>,
    pub destination: Option<(String, i8, i8, i16)>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ContactState {
    pub background_flags: Option<u32>,
    pub surfaces: Vec<ContactFact>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ActorRelationshipFact {
    pub actor_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    pub health: i16,
    pub status: u32,
    pub player_relative_position_bin: [i32; 3],
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ActorRelationshipState {
    pub actors: Vec<ActorRelationshipFact>,
    pub observation_truncated: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FlagState {
    pub record_flags: u32,
    pub player_mode_flags: Option<u32>,
    pub event_status: u8,
    pub event_mode: u8,
    pub goal_configured: Option<bool>,
    pub goal_reached: Option<bool>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SemanticStateCombination {
    pub state: SemanticState,
    pub event: EventFact,
    pub contact: Option<ContactState>,
    pub actor_relationships: Option<ActorRelationshipState>,
    pub flags: FlagState,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct KinematicExtrema {
    pub position_bin_world_units: u32,
    pub velocity_bin_world_units_numerator: u32,
    pub velocity_bin_world_units_denominator: u32,
    pub min_position_bin: [i32; 3],
    pub max_position_bin: [i32; 3],
    pub min_velocity_bin: [i32; 3],
    pub max_velocity_bin: [i32; 3],
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct BoundaryFingerprintFact {
    pub name: String,
    pub schema: String,
    pub algorithm: String,
    pub canonical_encoding: String,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticNoveltyDescriptor {
    pub schema: &'static str,
    pub procedure_sequence: Vec<Option<u16>>,
    pub event_sequence: Vec<EventFact>,
    pub state_transitions: Vec<StateTransitionFact>,
    pub contact_sequence: Vec<ContactState>,
    pub actor_relationship_sequence: Vec<ActorRelationshipState>,
    pub flag_sequence: Vec<FlagState>,
    pub state_combinations: Vec<SemanticStateCombination>,
    pub kinematic_extrema: Option<KinematicExtrema>,
    pub boundary_fingerprints: Vec<BoundaryFingerprintFact>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticNoveltyAxisIdentities {
    pub procedure_sequence: Option<String>,
    pub event_sequence: Option<String>,
    pub state_transitions: Option<String>,
    pub contacts: Option<String>,
    pub actor_relationships: Option<String>,
    pub flags: Option<String>,
    pub state_combinations: Option<String>,
    pub kinematic_extrema: Option<String>,
    pub boundary_fingerprints: Option<String>,
}

#[derive(Debug)]
pub struct SemanticNoveltyError(String);

impl fmt::Display for SemanticNoveltyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SemanticNoveltyError {}

impl SemanticNoveltyDescriptor {
    pub fn from_trace(
        trace: &DecodedTrace,
        boundary_fingerprints: Vec<BoundaryFingerprintFact>,
    ) -> Result<Self, SemanticNoveltyError> {
        validate_boundaries(&boundary_fingerprints)?;
        let mut descriptor = Self {
            schema: SEMANTIC_NOVELTY_SCHEMA,
            procedure_sequence: Vec::new(),
            event_sequence: Vec::new(),
            state_transitions: Vec::new(),
            contact_sequence: Vec::new(),
            actor_relationship_sequence: Vec::new(),
            flag_sequence: Vec::new(),
            state_combinations: Vec::new(),
            kinematic_extrema: kinematic_extrema(&trace.records)?,
            boundary_fingerprints,
        };
        let mut previous_state = None;
        for record in &trace.records {
            let state = semantic_state(record);
            let event = event_fact(record);
            let contact = contact_state(record);
            let relationships = actor_relationship_state(record)?;
            let flags = flag_state(record);
            push_changed(&mut descriptor.procedure_sequence, record.player_proc_id)?;
            push_changed(&mut descriptor.event_sequence, event.clone())?;
            if let Some(contact) = contact.as_ref() {
                push_changed(&mut descriptor.contact_sequence, contact.clone())?;
            }
            if let Some(relationships) = relationships.as_ref() {
                push_changed(
                    &mut descriptor.actor_relationship_sequence,
                    relationships.clone(),
                )?;
            }
            push_changed(&mut descriptor.flag_sequence, flags.clone())?;
            push_changed(
                &mut descriptor.state_combinations,
                SemanticStateCombination {
                    state: state.clone(),
                    event,
                    contact,
                    actor_relationships: relationships,
                    flags,
                },
            )?;

            if let Some(from) = previous_state.replace(state.clone()) {
                if from != state {
                    push_bounded(
                        &mut descriptor.state_transitions,
                        StateTransitionFact { from, to: state },
                    )?;
                }
            }
        }
        Ok(descriptor)
    }

    pub fn identity(&self) -> String {
        axis_identity(b"descriptor/v1", self)
    }

    pub fn axis_identities(&self) -> SemanticNoveltyAxisIdentities {
        SemanticNoveltyAxisIdentities {
            procedure_sequence: nonempty_identity(b"procedures/v1", &self.procedure_sequence),
            event_sequence: nonempty_identity(b"events/v1", &self.event_sequence),
            state_transitions: nonempty_identity(b"transitions/v1", &self.state_transitions),
            contacts: nonempty_identity(b"contacts/v1", &self.contact_sequence),
            actor_relationships: nonempty_identity(
                b"actor-relationships/v1",
                &self.actor_relationship_sequence,
            ),
            flags: nonempty_identity(b"flags/v1", &self.flag_sequence),
            state_combinations: nonempty_identity(
                b"state-combinations/v1",
                &self.state_combinations,
            ),
            kinematic_extrema: self
                .kinematic_extrema
                .as_ref()
                .map(|value| axis_identity(b"kinematic-extrema/v1", value)),
            boundary_fingerprints: nonempty_identity(
                b"boundary-fingerprints/v1",
                &self.boundary_fingerprints,
            ),
        }
    }
}

fn semantic_state(record: &TraceRecord) -> SemanticState {
    SemanticState {
        stage: record.stage_name.clone(),
        room: record.room,
        layer: record.layer,
        point: record.point,
        player_procedure: record.player_proc_id,
    }
}

fn event_fact(record: &TraceRecord) -> EventFact {
    EventFact {
        event_id: record.event_id,
        mode: record.event_mode,
        status: record.event_status,
        map_tool_id: record.event_map_tool_id,
        name_hash: record
            .event_name_hash_present
            .then_some(record.event_name_hash),
    }
}

fn contact_state(record: &TraceRecord) -> Option<ContactState> {
    if record.player_background_collision.is_none() && record.player_collision_surfaces.is_none() {
        return None;
    }
    let mut surfaces = record
        .player_collision_surfaces
        .iter()
        .flat_map(|set| &set.surfaces)
        .filter(|surface| surface.bg_index.is_some() || surface.poly_index.is_some())
        .map(|surface| ContactFact {
            kind: match surface.kind {
                TraceCollisionSurfaceKind::Ground => "ground",
                TraceCollisionSurfaceKind::Roof => "roof",
                TraceCollisionSurfaceKind::Water => "water",
                TraceCollisionSurfaceKind::Wall => "wall",
            },
            wall_slot: surface.wall_slot,
            backing: surface.backing_format.map(|format| match format {
                TraceCollisionBackingFormat::Dzb => "dzb",
                TraceCollisionBackingFormat::Kcl => "kcl",
            }),
            bg_index: surface.bg_index,
            poly_index: surface.poly_index,
            material_row: surface.material_row,
            group_row: surface.group_row,
            raw_exit_id: surface.raw_exit_id,
            source_room: surface.source_room,
            destination: surface.destination.as_ref().map(|destination| {
                (
                    destination.stage_name.clone(),
                    destination.room,
                    destination.layer,
                    destination.point,
                )
            }),
        })
        .collect::<Vec<_>>();
    surfaces.sort();
    Some(ContactState {
        background_flags: record
            .player_background_collision
            .as_ref()
            .map(|collision| collision.flags),
        surfaces,
    })
}

fn actor_relationship_state(
    record: &TraceRecord,
) -> Result<Option<ActorRelationshipState>, SemanticNoveltyError> {
    let Some(selected) = record.selected_actors.as_ref() else {
        return Ok(None);
    };
    let mut actors = selected
        .actors
        .iter()
        .filter(|actor| Some(actor.session_process_id) != record.player_session_process_id)
        .map(|actor| {
            let mut relative = [0; 3];
            for axis in 0..3 {
                relative[axis] = quantize(
                    actor.position[axis] - record.position[axis],
                    ACTOR_RELATION_BIN_WORLD_UNITS,
                    "actor relative position",
                )?;
            }
            Ok(ActorRelationshipFact {
                actor_name: actor.actor_name,
                set_id: actor.set_id,
                home_room: actor.home_room,
                current_room: actor.current_room,
                health: actor.health,
                status: actor.status,
                player_relative_position_bin: relative,
            })
        })
        .collect::<Result<Vec<_>, SemanticNoveltyError>>()?;
    actors.sort();
    Ok(Some(ActorRelationshipState {
        actors,
        observation_truncated: selected.truncated,
    }))
}

fn flag_state(record: &TraceRecord) -> FlagState {
    FlagState {
        record_flags: record.flags,
        player_mode_flags: record
            .player_action
            .as_ref()
            .map(|action| action.mode_flags),
        event_status: record.event_status,
        event_mode: record.event_mode,
        goal_configured: record.goal_progress.as_ref().map(|goal| goal.configured),
        goal_reached: record.goal_progress.as_ref().map(|goal| goal.reached),
    }
}

fn kinematic_extrema(
    records: &[TraceRecord],
) -> Result<Option<KinematicExtrema>, SemanticNoveltyError> {
    let mut minimum_position = [i32::MAX; 3];
    let mut maximum_position = [i32::MIN; 3];
    let mut minimum_velocity = [i32::MAX; 3];
    let mut maximum_velocity = [i32::MIN; 3];
    let mut observed = false;
    for record in records
        .iter()
        .filter(|record| record.player_session_process_id.is_some())
    {
        observed = true;
        for axis in 0..3 {
            let position = quantize(
                record.position[axis],
                POSITION_EXTREMA_BIN_WORLD_UNITS,
                "player position",
            )?;
            let velocity = quantize(
                record.velocity[axis],
                VELOCITY_EXTREMA_BIN_WORLD_UNITS,
                "player velocity",
            )?;
            minimum_position[axis] = minimum_position[axis].min(position);
            maximum_position[axis] = maximum_position[axis].max(position);
            minimum_velocity[axis] = minimum_velocity[axis].min(velocity);
            maximum_velocity[axis] = maximum_velocity[axis].max(velocity);
        }
    }
    Ok(observed.then_some(KinematicExtrema {
        position_bin_world_units: POSITION_EXTREMA_BIN_WORLD_UNITS as u32,
        velocity_bin_world_units_numerator: 1,
        velocity_bin_world_units_denominator: 16,
        min_position_bin: minimum_position,
        max_position_bin: maximum_position,
        min_velocity_bin: minimum_velocity,
        max_velocity_bin: maximum_velocity,
    }))
}

fn quantize(value: f32, width: f32, label: &str) -> Result<i32, SemanticNoveltyError> {
    if !value.is_finite() {
        return Err(SemanticNoveltyError(format!(
            "semantic novelty {label} is not finite"
        )));
    }
    let value = (value / width).round();
    if value < i32::MIN as f32 || value > i32::MAX as f32 {
        return Err(SemanticNoveltyError(format!(
            "semantic novelty {label} exceeds the portable bin range"
        )));
    }
    Ok(value as i32)
}

fn push_changed<T: Eq>(values: &mut Vec<T>, value: T) -> Result<(), SemanticNoveltyError> {
    if values.last() != Some(&value) {
        push_bounded(values, value)?;
    }
    Ok(())
}

fn push_bounded<T>(values: &mut Vec<T>, value: T) -> Result<(), SemanticNoveltyError> {
    if values.len() >= MAX_SEMANTIC_SEQUENCE_STATES {
        return Err(SemanticNoveltyError(format!(
            "semantic novelty sequence exceeds {MAX_SEMANTIC_SEQUENCE_STATES} states"
        )));
    }
    values.push(value);
    Ok(())
}

fn validate_boundaries(boundaries: &[BoundaryFingerprintFact]) -> Result<(), SemanticNoveltyError> {
    if boundaries.len() > MAX_SEMANTIC_SEQUENCE_STATES {
        return Err(SemanticNoveltyError(
            "semantic novelty has too many boundary fingerprints".into(),
        ));
    }
    if !boundaries
        .windows(2)
        .all(|pair| pair[0].name < pair[1].name)
    {
        return Err(SemanticNoveltyError(
            "semantic novelty boundary fingerprints must have unique sorted names".into(),
        ));
    }
    if boundaries.iter().any(|boundary| {
        boundary.name.is_empty()
            || boundary.digest.is_empty()
            || boundary.schema.is_empty()
            || boundary.algorithm.is_empty()
            || boundary.canonical_encoding.is_empty()
    }) {
        return Err(SemanticNoveltyError(
            "semantic novelty boundary fingerprints must be fully identified".into(),
        ));
    }
    Ok(())
}

fn nonempty_identity<T: Serialize>(domain: &[u8], values: &[T]) -> Option<String> {
    (!values.is_empty()).then(|| axis_identity(domain, values))
}

fn axis_identity<T: Serialize + ?Sized>(domain: &[u8], value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("semantic descriptor is serializable");
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-semantic-novelty-axis/v1\0");
    hasher.update((domain.len() as u64).to_le_bytes());
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::TapeBoot;
    use crate::trace::{TraceRecord, TraceSelectedActor, TraceSelectedActors};
    use std::collections::BTreeMap;

    fn trace(records: Vec<TraceRecord>) -> DecodedTrace {
        DecodedTrace {
            version: 5,
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: 0,
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records,
        }
    }

    fn record(procedure: u16, event: i16, x: f32) -> TraceRecord {
        TraceRecord {
            stage_name: "F_SP104".into(),
            room: 1,
            player_session_process_id: Some(1),
            player_proc_id: Some(procedure),
            event_id: event,
            position: [x, 20.0, 30.0],
            velocity: [2.0, -1.0, 0.5],
            selected_actors: Some(TraceSelectedActors {
                observed_count: 1,
                truncated: false,
                actors: vec![TraceSelectedActor {
                    session_process_id: 99,
                    actor_name: 12,
                    set_id: 3,
                    home_room: 1,
                    current_room: 1,
                    health: 4,
                    status: 8,
                    position: [x + 200.0, 20.0, 30.0],
                    current_angle: [0; 3],
                    shape_angle: [0; 3],
                }],
            }),
            ..TraceRecord::default()
        }
    }

    #[test]
    fn descriptor_keeps_raw_semantic_reasons_and_deduplicates_runs() {
        let descriptor = SemanticNoveltyDescriptor::from_trace(
            &trace(vec![
                record(3, -1, 0.0),
                record(3, -1, 8.0),
                record(7, 42, 64.0),
            ]),
            vec![BoundaryFingerprintFact {
                name: "goal".into(),
                schema: "boundary/v1".into(),
                algorithm: "sha256".into(),
                canonical_encoding: "native".into(),
                digest: "ab".repeat(32),
            }],
        )
        .unwrap();
        assert_eq!(descriptor.procedure_sequence, vec![Some(3), Some(7)]);
        assert_eq!(descriptor.event_sequence.len(), 2);
        assert_eq!(descriptor.state_transitions.len(), 1);
        assert_eq!(descriptor.actor_relationship_sequence.len(), 1);
        assert_eq!(
            descriptor.actor_relationship_sequence[0].actors[0].player_relative_position_bin,
            [2, 0, 0]
        );
        assert_eq!(
            descriptor
                .kinematic_extrema
                .as_ref()
                .unwrap()
                .max_position_bin[0],
            4
        );
        assert_eq!(descriptor.identity().len(), 64);
        assert!(descriptor.axis_identities().boundary_fingerprints.is_some());
    }

    #[test]
    fn portable_actor_relationship_ignores_session_process_ids() {
        let left =
            SemanticNoveltyDescriptor::from_trace(&trace(vec![record(3, 1, 0.0)]), vec![]).unwrap();
        let mut right_record = record(3, 1, 0.0);
        right_record.selected_actors.as_mut().unwrap().actors[0].session_process_id = 1234;
        let right =
            SemanticNoveltyDescriptor::from_trace(&trace(vec![right_record]), vec![]).unwrap();
        assert_eq!(left, right);
        assert_eq!(left.identity(), right.identity());
    }

    #[test]
    fn changed_semantic_fact_changes_only_its_axis_identity() {
        let left =
            SemanticNoveltyDescriptor::from_trace(&trace(vec![record(3, 1, 0.0)]), vec![]).unwrap();
        let right =
            SemanticNoveltyDescriptor::from_trace(&trace(vec![record(3, 2, 0.0)]), vec![]).unwrap();
        let left_axes = left.axis_identities();
        let right_axes = right.axis_identities();
        assert_ne!(left_axes.event_sequence, right_axes.event_sequence);
        assert_eq!(left_axes.procedure_sequence, right_axes.procedure_sequence);
        assert_eq!(
            left_axes.actor_relationships,
            right_axes.actor_relationships
        );
        assert_ne!(left.identity(), right.identity());
    }

    #[test]
    fn boundary_fingerprints_require_unique_canonical_order() {
        let boundary = |name: &str| BoundaryFingerprintFact {
            name: name.into(),
            schema: "boundary/v1".into(),
            algorithm: "sha256".into(),
            canonical_encoding: "native".into(),
            digest: "ab".repeat(32),
        };
        assert!(
            SemanticNoveltyDescriptor::from_trace(
                &trace(vec![record(3, 1, 0.0)]),
                vec![boundary("z"), boundary("a")],
            )
            .is_err()
        );
    }
}
