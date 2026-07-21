//! Loss-aware projection from authenticated native observations into planner snapshots.

use crate::artifact::Digest;
use crate::identity::RuntimeConfiguration;
use crate::native_observation::{
    NativeAttentionCandidateObservation, NativeAttentionCandidatesObservation, NativeChannelStatus,
    NativeEventActorReferenceObservation, NativeEventHandoffObservation,
    NativeEventQueueObservation, NativeLearningObservation, NativeMessageSessionObservation,
    NativePlayerResourcesObservation,
};
use crate::snapshot::{
    SNAPSHOT_CHAIN_SCHEMA, STATE_SNAPSHOT_SCHEMA, SnapshotChain, SnapshotChainEntry, StateDiff,
    StateSnapshot,
};
use crate::state::{
    ActorLifecycle, BackingAttachment, BoundaryKind, CaptureStatus, ComponentBinding,
    ComponentKind, ComponentPayload, ComponentProvenance, EXECUTION_ENVIRONMENT_SCHEMA,
    ExecutionEnvironment, LiveWorldObject, PhysicalSlotId, PhysicalSlotObservation, PlayerForm,
    PlayerMount, PlayerState, ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle,
    RuntimeFileOrigin, SceneLocation, SemanticLifetime, SerializationOwner, StateComponent,
    StateValue,
};
use crate::{PlannerContractError, validate_stable_id};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct NativeSnapshotContext {
    pub snapshot_id: String,
    pub sequence: u64,
    pub runtime_configuration: RuntimeConfiguration,
    pub runtime_file_id: String,
    pub session_id: String,
    pub evidence_id: String,
    pub evidence_sha256: Digest,
}

#[derive(Clone, Debug)]
pub struct NativeStateEvidence {
    pub snapshots: Vec<StateSnapshot>,
    pub diffs: Vec<StateDiff>,
    pub chain: SnapshotChain,
}

impl NativeStateEvidence {
    pub fn begin(
        observation: &NativeLearningObservation,
        context: NativeSnapshotContext,
    ) -> Result<Self, PlannerContractError> {
        let snapshot = snapshot_native_observation(observation, context)?;
        let digest = snapshot.digest()?;
        Ok(Self {
            snapshots: vec![snapshot],
            diffs: Vec::new(),
            chain: SnapshotChain {
                schema: SNAPSHOT_CHAIN_SCHEMA.into(),
                entries: vec![SnapshotChainEntry {
                    snapshot_sha256: digest,
                    previous_snapshot_sha256: None,
                    incoming_boundary: None,
                }],
            },
        })
    }

    pub fn append(
        &mut self,
        observation: &NativeLearningObservation,
        context: NativeSnapshotContext,
        boundary: BoundaryKind,
    ) -> Result<(), PlannerContractError> {
        let before = self.snapshots.last().ok_or_else(|| {
            PlannerContractError::new("native_state_evidence", "has no initial snapshot")
        })?;
        let after = snapshot_native_observation(observation, context)?;
        let diff = StateDiff::between(before, &after, boundary.clone())?;
        diff.validate()?;
        let previous_snapshot_sha256 = before.digest()?;
        let snapshot_sha256 = after.digest()?;
        self.snapshots.push(after);
        self.diffs.push(diff);
        self.chain.entries.push(SnapshotChainEntry {
            snapshot_sha256,
            previous_snapshot_sha256: Some(previous_snapshot_sha256),
            incoming_boundary: Some(boundary),
        });
        self.chain.validate()
    }
}

/// Projects one complete native boundary observation without inventing unavailable state.
///
/// A planner `StateSnapshot` requires a location and player pose, so observations without a
/// present Link actor are rejected rather than being filled with default values. Unknown runtime
/// origin, backing, control, slot contents, and unsupported component payloads remain explicit.
pub fn snapshot_native_observation(
    observation: &NativeLearningObservation,
    context: NativeSnapshotContext,
) -> Result<StateSnapshot, PlannerContractError> {
    validate_context(&context)?;
    if observation.stage.is_empty() {
        return Err(PlannerContractError::new(
            "native_observation.stage",
            "is unavailable",
        ));
    }
    if !observation.player_present || !observation.player_is_link {
        return Err(PlannerContractError::new(
            "native_observation.player",
            "must contain a present Link actor",
        ));
    }
    if observation.runtime_file_status != NativeChannelStatus::Present {
        return Err(PlannerContractError::new(
            "native_observation.runtime_file",
            "must be present to identify the active runtime",
        ));
    }
    let runtime = observation.runtime_file.as_ref().ok_or_else(|| {
        PlannerContractError::new(
            "native_observation.runtime_file",
            "status is present but payload is absent",
        )
    })?;
    let backing = match (
        runtime.backing_attachment_status,
        runtime.attached_physical_slot,
    ) {
        (NativeChannelStatus::Present, Some(slot)) => BackingAttachment::CardBacked {
            slot: PhysicalSlotId(slot),
        },
        _ => BackingAttachment::Unknown,
    };
    let physical_slot_observations = runtime
        .physical_slots
        .iter()
        .map(|slot| PhysicalSlotObservation {
            slot: PhysicalSlotId(slot.number),
            content_status: capture_status(slot.content_status),
            attached_to_active_runtime: slot.attached_to_runtime,
        })
        .collect();

    let provenance = ComponentProvenance {
        source_kind: ProvenanceSourceKind::TraceObservation,
        source_id: context.evidence_id.clone(),
        source_sha256: Some(context.evidence_sha256),
        transition_id: None,
    };
    let runtime_binding = ComponentBinding::RuntimeFile {
        runtime_file_id: context.runtime_file_id.clone(),
    };
    let runtime_owner = SerializationOwner::RuntimeFile {
        runtime_file_id: context.runtime_file_id.clone(),
    };
    let mut components = Vec::new();
    components.push(structured_component(
        "runtime-file.header",
        ComponentKind::Session,
        fields([
            (
                "no_file_raw",
                StateValue::Unsigned(runtime.no_file_raw.into()),
            ),
            (
                "data_num_raw",
                StateValue::Unsigned(runtime.data_num_raw.into()),
            ),
            (
                "backing_attachment_status",
                StateValue::Text(status_text(runtime.backing_attachment_status).into()),
            ),
        ]),
        runtime_binding.clone(),
        SemanticLifetime::RuntimeFile,
        runtime_owner.clone(),
        &provenance,
    ));
    push_raw_component(
        &mut components,
        "flags.event",
        ComponentKind::Custom {
            id: "observed-event-flag-labels".into(),
        },
        observation.event_flags.as_deref(),
        None,
        runtime_binding.clone(),
        SemanticLifetime::RuntimeFile,
        runtime_owner.clone(),
        &provenance,
    );
    push_raw_component(
        &mut components,
        "flags.temporary",
        ComponentKind::Custom {
            id: "observed-temporary-flag-labels".into(),
        },
        observation.temporary_flags.as_deref(),
        None,
        runtime_binding.clone(),
        SemanticLifetime::StageLoad,
        runtime_owner.clone(),
        &provenance,
    );
    push_raw_component(
        &mut components,
        "flags.temporary-event-registers",
        ComponentKind::TemporaryFlags,
        observation.temporary_event_bytes.as_deref(),
        Some(256),
        runtime_binding.clone(),
        SemanticLifetime::StageLoad,
        runtime_owner.clone(),
        &provenance,
    );
    push_raw_component(
        &mut components,
        "flags.dungeon",
        ComponentKind::DungeonMemory,
        observation.dungeon_flags.as_deref(),
        None,
        ComponentBinding::Stage {
            stage: observation.stage.clone(),
        },
        SemanticLifetime::StageLoad,
        SerializationOwner::StageBank {
            runtime_file_id: context.runtime_file_id.clone(),
            stage: observation.stage.clone(),
        },
        &provenance,
    );
    push_raw_component(
        &mut components,
        "flags.switch",
        ComponentKind::ZoneMemory,
        observation.switch_flags.as_deref(),
        None,
        ComponentBinding::Room {
            stage: observation.stage.clone(),
            room: observation.switch_flag_room,
        },
        SemanticLifetime::RoomLoad,
        runtime_owner.clone(),
        &provenance,
    );
    push_statused_structured(
        &mut components,
        "return-place",
        ComponentKind::PersistentSave,
        observation.return_place_status,
        observation.return_place.as_ref().map(|value| {
            fields([
                ("stage", StateValue::Text(value.stage.clone())),
                ("room", StateValue::Signed(value.room.into())),
                (
                    "player_status",
                    StateValue::Unsigned(value.player_status.into()),
                ),
            ])
        }),
        runtime_binding.clone(),
        SemanticLifetime::RuntimeFile,
        runtime_owner.clone(),
        &provenance,
    );
    push_statused_structured(
        &mut components,
        "restart",
        ComponentKind::Restart,
        observation.restart_status,
        observation.restart.as_ref().map(|value| {
            fields([
                ("room", StateValue::Signed(value.room.into())),
                ("start_point", StateValue::Signed(value.start_point.into())),
                ("angle_y", StateValue::Signed(value.angle_y.into())),
                ("position", StateValue::Bytes(f32_bytes(&value.position))),
                ("room_param", StateValue::Unsigned(value.room_param.into())),
                (
                    "last_speed_f32_bits",
                    StateValue::Unsigned(value.last_speed.to_bits().into()),
                ),
                ("last_mode", StateValue::Unsigned(value.last_mode.into())),
                (
                    "last_angle_y",
                    StateValue::Signed(value.last_angle_y.into()),
                ),
            ])
        }),
        runtime_binding.clone(),
        SemanticLifetime::RuntimeFile,
        runtime_owner.clone(),
        &provenance,
    );
    push_statused_structured(
        &mut components,
        "event-handoff",
        ComponentKind::PendingOperation,
        observation.event_handoff_status,
        observation.event_handoff.as_ref().map(event_handoff_fields),
        ComponentBinding::Session {
            session_id: context.session_id.clone(),
        },
        SemanticLifetime::Action,
        SerializationOwner::None,
        &provenance,
    );
    push_statused_structured(
        &mut components,
        "event-recent-item",
        ComponentKind::Session,
        observation.event_handoff_status,
        observation.event_handoff.as_ref().map(|value| {
            fields([(
                "get_item_no",
                StateValue::Unsigned(value.get_item_no.into()),
            )])
        }),
        ComponentBinding::Session {
            session_id: context.session_id.clone(),
        },
        SemanticLifetime::Session,
        SerializationOwner::None,
        &provenance,
    );
    push_statused_structured(
        &mut components,
        "message-session",
        ComponentKind::MessageFlow,
        observation.message_session_status,
        observation
            .message_session
            .as_ref()
            .map(message_session_fields),
        ComponentBinding::Session {
            session_id: context.session_id.clone(),
        },
        SemanticLifetime::Action,
        SerializationOwner::None,
        &provenance,
    );
    push_statused_structured(
        &mut components,
        "event-queue",
        ComponentKind::PendingOperation,
        observation.event_queue_status,
        observation.event_queue.as_ref().map(event_queue_fields),
        ComponentBinding::Session {
            session_id: context.session_id.clone(),
        },
        SemanticLifetime::Action,
        SerializationOwner::None,
        &provenance,
    );
    push_statused_structured(
        &mut components,
        "attention-candidates",
        ComponentKind::PendingOperation,
        observation.attention_candidates_status,
        observation
            .attention_candidates
            .as_ref()
            .map(attention_candidate_fields),
        ComponentBinding::Session {
            session_id: context.session_id.clone(),
        },
        SemanticLifetime::Action,
        SerializationOwner::None,
        &provenance,
    );
    push_statused_structured(
        &mut components,
        "inventory-and-resources",
        ComponentKind::Inventory,
        observation.player_resources_status,
        observation
            .player_resources
            .as_ref()
            .map(player_resource_fields),
        runtime_binding,
        SemanticLifetime::RuntimeFile,
        runtime_owner,
        &provenance,
    );
    let mut live_world_objects = Vec::new();
    for actor in &observation.actors {
        let Some(writer) = &actor.return_place_writer else {
            continue;
        };
        let instance_id = format!("actor.runtime.{:016x}", actor.runtime_generation);
        let writer_fields = fields([
            ("target_stage", StateValue::Text(observation.stage.clone())),
            ("save_room", StateValue::Signed(writer.save_room.into())),
            ("save_point", StateValue::Unsigned(writer.save_point.into())),
            ("switch_room", StateValue::Signed(writer.switch_room.into())),
            (
                "required_event_set",
                StateValue::Unsigned(writer.required_event_set.into()),
            ),
            (
                "required_event_unset",
                StateValue::Unsigned(writer.required_event_unset.into()),
            ),
            (
                "required_switch_set",
                StateValue::Unsigned(writer.required_switch_set.into()),
            ),
            (
                "required_switch_unset",
                StateValue::Unsigned(writer.required_switch_unset.into()),
            ),
            ("no_telop_clear", StateValue::Boolean(writer.no_telop_clear)),
            (
                "event_set_satisfied",
                StateValue::Boolean(writer.event_set_satisfied),
            ),
            (
                "event_unset_satisfied",
                StateValue::Boolean(writer.event_unset_satisfied),
            ),
            (
                "switch_set_satisfied",
                StateValue::Boolean(writer.switch_set_satisfied),
            ),
            (
                "switch_unset_satisfied",
                StateValue::Boolean(writer.switch_unset_satisfied),
            ),
            ("eligible", StateValue::Boolean(writer.eligible)),
        ]);
        components.push(structured_component(
            &format!("{instance_id}.return-place-writer"),
            ComponentKind::ActorInstance,
            writer_fields.clone(),
            ComponentBinding::Actor {
                instance_id: instance_id.clone(),
            },
            SemanticLifetime::RoomLoad,
            SerializationOwner::None,
            &provenance,
        ));
        live_world_objects.push(LiveWorldObject {
            instance_id,
            static_object_id: None,
            actor_type: "kytag14.return-place-writer".into(),
            lifecycle: ActorLifecycle::Loaded,
            fields: writer_fields,
        });
    }
    components.sort_by(|left, right| left.id.cmp(&right.id));

    let player = PlayerState {
        form: if observation.player_form_present {
            if observation.player_is_wolf {
                PlayerForm::Wolf
            } else {
                PlayerForm::Human
            }
        } else {
            PlayerForm::Unknown
        },
        mount: observed_mount(observation),
        position: canonical_vec3(observation.player_position),
        rotation: observation.player_current_angle,
        has_control: None,
        action: observation.player_action.as_ref().map_or_else(
            || "unknown".into(),
            |action| format!("procedure.{:04x}", action.procedure_id),
        ),
    };
    let snapshot = StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: context.snapshot_id,
        sequence: context.sequence,
        environment: ExecutionEnvironment {
            schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
            runtime_configuration: context.runtime_configuration,
            active_runtime_file: RuntimeFile {
                id: context.runtime_file_id,
                origin: RuntimeFileOrigin::Unknown,
                backing,
                allowed_serialization_targets: vec![
                    PhysicalSlotId(1),
                    PhysicalSlotId(2),
                    PhysicalSlotId(3),
                ],
                lifecycle: RuntimeFileLifecycle::Active,
            },
            inactive_runtime_files: Vec::new(),
            physical_slots: Vec::new(),
            physical_slot_observations,
            location: SceneLocation {
                stage: observation.stage.clone(),
                room: observation.room,
                layer: observation.layer,
                spawn: observation.point,
            },
            player,
            components,
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: Vec::new(),
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects,
        },
        semantic_observations: Vec::new(),
    };
    snapshot.validate()?;
    Ok(snapshot)
}

fn validate_context(context: &NativeSnapshotContext) -> Result<(), PlannerContractError> {
    validate_stable_id("snapshot_id", &context.snapshot_id)?;
    validate_stable_id("runtime_file_id", &context.runtime_file_id)?;
    validate_stable_id("session_id", &context.session_id)?;
    validate_stable_id("evidence_id", &context.evidence_id)?;
    if context.evidence_sha256 == Digest::ZERO {
        return Err(PlannerContractError::new(
            "evidence_sha256",
            "must be nonzero",
        ));
    }
    context.runtime_configuration.validate()
}

fn capture_status(status: NativeChannelStatus) -> CaptureStatus {
    match status {
        NativeChannelStatus::NotSampled => CaptureStatus::NotSampled,
        NativeChannelStatus::Present => CaptureStatus::Present,
        NativeChannelStatus::Absent => CaptureStatus::Absent,
        NativeChannelStatus::Unavailable => CaptureStatus::Unavailable,
    }
}

fn status_text(status: NativeChannelStatus) -> &'static str {
    match status {
        NativeChannelStatus::NotSampled => "not_sampled",
        NativeChannelStatus::Present => "present",
        NativeChannelStatus::Absent => "absent",
        NativeChannelStatus::Unavailable => "unavailable",
    }
}

fn fields<const N: usize>(
    entries: [(&'static str, StateValue); N],
) -> BTreeMap<String, StateValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect()
}

fn structured_component(
    id: &str,
    component_kind: ComponentKind,
    fields: BTreeMap<String, StateValue>,
    binding: ComponentBinding,
    lifetime: SemanticLifetime,
    serialization_owner: SerializationOwner,
    provenance: &ComponentProvenance,
) -> StateComponent {
    StateComponent {
        id: id.into(),
        component_kind,
        payload: ComponentPayload::Structured { fields },
        binding,
        lifetime,
        serialization_owner,
        provenance: vec![provenance.clone()],
    }
}

#[allow(clippy::too_many_arguments)]
fn push_raw_component(
    components: &mut Vec<StateComponent>,
    id: &str,
    component_kind: ComponentKind,
    bytes: Option<&[u8]>,
    expected_bytes: Option<u32>,
    binding: ComponentBinding,
    lifetime: SemanticLifetime,
    serialization_owner: SerializationOwner,
    provenance: &ComponentProvenance,
) {
    let payload = bytes.map_or(ComponentPayload::Unknown { expected_bytes }, |bytes| {
        ComponentPayload::Raw {
            bytes: bytes.to_vec(),
            known_mask: vec![0xff; bytes.len()],
        }
    });
    components.push(StateComponent {
        id: id.into(),
        component_kind,
        payload,
        binding,
        lifetime,
        serialization_owner,
        provenance: vec![provenance.clone()],
    });
}

#[allow(clippy::too_many_arguments)]
fn push_statused_structured(
    components: &mut Vec<StateComponent>,
    id: &str,
    component_kind: ComponentKind,
    status: NativeChannelStatus,
    value: Option<BTreeMap<String, StateValue>>,
    binding: ComponentBinding,
    lifetime: SemanticLifetime,
    serialization_owner: SerializationOwner,
    provenance: &ComponentProvenance,
) {
    let payload = match (status, value) {
        (NativeChannelStatus::Present, Some(mut fields)) => {
            fields.insert("capture_status".into(), StateValue::Text("present".into()));
            ComponentPayload::Structured { fields }
        }
        _ => ComponentPayload::Structured {
            fields: fields([(
                "capture_status",
                StateValue::Text(status_text(status).into()),
            )]),
        },
    };
    components.push(StateComponent {
        id: id.into(),
        component_kind,
        payload,
        binding,
        lifetime,
        serialization_owner,
        provenance: vec![provenance.clone()],
    });
}

fn event_handoff_fields(value: &NativeEventHandoffObservation) -> BTreeMap<String, StateValue> {
    let mut output = fields([
        (
            "pre_item_no",
            StateValue::Unsigned(value.pre_item_no.into()),
        ),
        (
            "event_flags",
            StateValue::Unsigned(value.event_flags.into()),
        ),
        (
            "secondary_flags",
            StateValue::Unsigned(value.secondary_flags.into()),
        ),
        ("hind_flags", StateValue::Unsigned(value.hind_flags.into())),
        (
            "talk_xy_type",
            StateValue::Unsigned(value.talk_xy_type.into()),
        ),
        ("compulsory", StateValue::Unsigned(value.compulsory.into())),
        ("room_info_set", StateValue::Boolean(value.room_info_set)),
        ("skip_timer", StateValue::Signed(value.skip_timer.into())),
        (
            "skip_parameter",
            StateValue::Signed(value.skip_parameter.into()),
        ),
        (
            "event_name_status",
            StateValue::Text(status_text(value.event_name_status).into()),
        ),
        (
            "message_flow_status",
            StateValue::Text(status_text(value.message_flow_status).into()),
        ),
        (
            "message_cut_status",
            StateValue::Text(status_text(value.message_cut_status).into()),
        ),
        (
            "pending_cleanup_status",
            StateValue::Text(status_text(value.pending_cleanup_status).into()),
        ),
        (
            "player_control_status",
            StateValue::Text(status_text(value.player_control_status).into()),
        ),
        (
            "no_telop_status",
            StateValue::Text(status_text(value.no_telop_status).into()),
        ),
    ]);
    if let Some(event_name) = &value.event_name {
        output.insert("event_name".into(), StateValue::Text(event_name.clone()));
    }
    if let Some(flow) = value.message_flow {
        output.insert("flow_id".into(), StateValue::Unsigned(flow.flow_id.into()));
        output.insert(
            "node_index".into(),
            StateValue::Unsigned(flow.node_index.into()),
        );
        if value.message_cut_status == NativeChannelStatus::Present {
            output.insert(
                "cut_name_hash".into(),
                StateValue::Unsigned(flow.cut_name_hash.into()),
            );
        }
    }
    if let Some(flags) = value.pending_cleanup_flags {
        output.insert(
            "pending_cleanup_flags".into(),
            StateValue::Unsigned(flags.into()),
        );
    }
    if let Some(control) = value.player_control {
        output.insert(
            "player_mode_flags".into(),
            StateValue::Unsigned(control.mode_flags.into()),
        );
        output.insert(
            "player_do_status".into(),
            StateValue::Unsigned(control.do_status.into()),
        );
    }
    if let Some(no_telop) = value.no_telop {
        output.insert("no_telop".into(), StateValue::Boolean(no_telop));
    }
    output.insert(
        "item_partner_present".into(),
        StateValue::Boolean(value.item_partner.present),
    );
    if value.item_partner.present {
        output.insert(
            "item_partner_runtime_generation".into(),
            StateValue::Unsigned(value.item_partner.runtime_generation.into()),
        );
        output.insert(
            "item_partner_actor_name".into(),
            StateValue::Signed(value.item_partner.actor_name.into()),
        );
    }
    output
}

fn message_session_fields(value: &NativeMessageSessionObservation) -> BTreeMap<String, StateValue> {
    let mut output = fields([
        ("procedure", StateValue::Unsigned(value.procedure.into())),
        ("message_id", StateValue::Unsigned(value.message_id.into())),
        (
            "message_index",
            StateValue::Signed(value.message_index.into()),
        ),
        ("node_index", StateValue::Unsigned(value.node_index.into())),
        ("flow_id", StateValue::Signed(value.flow_id.into())),
        (
            "selection_count",
            StateValue::Unsigned(value.selection_count.into()),
        ),
        (
            "selection_cursor",
            StateValue::Unsigned(value.selection_cursor.into()),
        ),
        (
            "selection_push",
            StateValue::Unsigned(value.selection_push.into()),
        ),
        (
            "output_type",
            StateValue::Unsigned(value.output_type.into()),
        ),
        ("talk_now", StateValue::Boolean(value.talk_now)),
        ("talk_message", StateValue::Boolean(value.talk_message)),
        ("auto_message", StateValue::Boolean(value.auto_message)),
        ("kill_pending", StateValue::Boolean(value.kill_pending)),
        ("camera_cancel", StateValue::Boolean(value.camera_cancel)),
        ("send", StateValue::Boolean(value.send)),
        ("send_control", StateValue::Boolean(value.send_control)),
        (
            "talk_actor_present",
            StateValue::Boolean(value.talk_actor.present),
        ),
    ]);
    if value.talk_actor.present {
        output.insert(
            "talk_actor_runtime_generation".into(),
            StateValue::Unsigned(value.talk_actor.runtime_generation.into()),
        );
        output.insert(
            "talk_actor_name".into(),
            StateValue::Signed(value.talk_actor.actor_name.into()),
        );
    }
    output
}

fn event_actor_reference_fields(
    output: &mut BTreeMap<String, StateValue>,
    prefix: &str,
    reference: &NativeEventActorReferenceObservation,
) {
    output.insert(
        format!("{prefix}.status"),
        StateValue::Text(status_text(reference.status).into()),
    );
    if let Some(actor) = &reference.actor {
        output.insert(
            format!("{prefix}.runtime_generation"),
            StateValue::Unsigned(actor.runtime_generation.into()),
        );
        output.insert(
            format!("{prefix}.actor_name"),
            StateValue::Signed(actor.actor_name.into()),
        );
    }
}

fn event_queue_fields(value: &NativeEventQueueObservation) -> BTreeMap<String, StateValue> {
    let mut output = fields([
        (
            "pending_count",
            StateValue::Unsigned(value.pending_orders.len() as u64),
        ),
        (
            "skip_registered",
            StateValue::Boolean(value.skip_registered),
        ),
    ]);
    for (index, order) in value.pending_orders.iter().enumerate() {
        let prefix = format!("pending.{index}");
        output.insert(
            format!("{prefix}.event_type"),
            StateValue::Unsigned(order.event_type.into()),
        );
        output.insert(
            format!("{prefix}.flags"),
            StateValue::Unsigned(order.flags.into()),
        );
        output.insert(
            format!("{prefix}.hind_flags"),
            StateValue::Unsigned(order.hind_flags.into()),
        );
        output.insert(
            format!("{prefix}.event_id"),
            StateValue::Signed(order.event_id.into()),
        );
        output.insert(
            format!("{prefix}.priority"),
            StateValue::Unsigned(order.priority.into()),
        );
        output.insert(
            format!("{prefix}.map_tool_id"),
            StateValue::Unsigned(order.map_tool_id.into()),
        );
        event_actor_reference_fields(
            &mut output,
            &format!("{prefix}.request_actor"),
            &order.request_actor,
        );
        event_actor_reference_fields(
            &mut output,
            &format!("{prefix}.target_actor"),
            &order.target_actor,
        );
    }
    for (name, reference) in [
        ("active_request_actor", &value.active_request_actor),
        ("active_target_actor", &value.active_target_actor),
        ("active_talk_actor", &value.active_talk_actor),
        ("active_item_actor", &value.active_item_actor),
        ("active_door_actor", &value.active_door_actor),
        ("change_actor", &value.change_actor),
        ("skip_actor", &value.skip_actor),
    ] {
        event_actor_reference_fields(&mut output, name, reference);
    }
    output
}

fn attention_candidate_fields(
    value: &NativeAttentionCandidatesObservation,
) -> BTreeMap<String, StateValue> {
    let mut output = fields([
        (
            "player_attention_flags",
            StateValue::Unsigned(value.player_attention_flags.into()),
        ),
        (
            "attention_status",
            StateValue::Unsigned(value.attention_status.into()),
        ),
        (
            "attention_block_timer",
            StateValue::Signed(value.attention_block_timer.into()),
        ),
        (
            "lock_offset",
            StateValue::Unsigned(value.lock_offset.into()),
        ),
        (
            "action_offset",
            StateValue::Unsigned(value.action_offset.into()),
        ),
        (
            "check_offset",
            StateValue::Unsigned(value.check_offset.into()),
        ),
    ]);
    for (list_name, candidates) in [
        ("lock", value.lock_candidates.as_slice()),
        ("action", value.action_candidates.as_slice()),
        ("check", value.check_candidates.as_slice()),
    ] {
        output.insert(
            format!("{list_name}.count"),
            StateValue::Unsigned(candidates.len() as u64),
        );
        for (index, candidate) in candidates.iter().enumerate() {
            append_attention_candidate_fields(
                &mut output,
                &format!("{list_name}.{index}"),
                candidate,
            );
        }
    }
    output
}

fn append_attention_candidate_fields(
    output: &mut BTreeMap<String, StateValue>,
    prefix: &str,
    candidate: &NativeAttentionCandidateObservation,
) {
    output.insert(
        format!("{prefix}.weight_f32_bits"),
        StateValue::Unsigned(candidate.weight.to_bits().into()),
    );
    output.insert(
        format!("{prefix}.distance_f32_bits"),
        StateValue::Unsigned(candidate.distance.to_bits().into()),
    );
    output.insert(
        format!("{prefix}.angle"),
        StateValue::Signed(candidate.angle.into()),
    );
    output.insert(
        format!("{prefix}.attention_type"),
        StateValue::Unsigned(candidate.attention_type.into()),
    );
    event_actor_reference_fields(output, &format!("{prefix}.actor"), &candidate.actor);
}

fn player_resource_fields(
    value: &NativePlayerResourcesObservation,
) -> BTreeMap<String, StateValue> {
    fields([
        (
            "maximum_life",
            StateValue::Unsigned(value.maximum_life.into()),
        ),
        ("life", StateValue::Unsigned(value.life.into())),
        ("rupees", StateValue::Unsigned(value.rupees.into())),
        ("inventory", StateValue::Bytes(value.inventory.to_vec())),
        (
            "selected_items",
            StateValue::Bytes(value.selected_items.to_vec()),
        ),
        ("mixed_items", StateValue::Bytes(value.mixed_items.to_vec())),
        ("equipment", StateValue::Bytes(value.equipment.to_vec())),
        ("bomb_counts", StateValue::Bytes(value.bomb_counts.to_vec())),
        (
            "bomb_capacities",
            StateValue::Bytes(value.bomb_capacities.to_vec()),
        ),
        (
            "bottle_quantities",
            StateValue::Bytes(value.bottle_quantities.to_vec()),
        ),
        (
            "acquired_item_bits",
            StateValue::Bytes(value.acquired_item_bits.to_vec()),
        ),
        (
            "collect_item_bits",
            StateValue::Bytes(value.collect_item_bits.to_vec()),
        ),
    ])
}

fn observed_mount(observation: &NativeLearningObservation) -> Option<PlayerMount> {
    if observation.player_relationships_status != NativeChannelStatus::Present {
        return Some(PlayerMount::Unknown);
    }
    observation
        .player_relationships
        .as_ref()
        .and_then(|relationships| relationships.ride_actor.as_ref())
        .map(|actor| PlayerMount::Other {
            id: format!("actor-name.{:04x}", actor.actor_name as u16),
        })
}

fn canonical_vec3(mut values: [f32; 3]) -> [f32; 3] {
    for value in &mut values {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    values
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::RUNTIME_CONFIGURATION_SCHEMA;
    use crate::native_observation::{
        NativeActorIdentity, NativeActorObservation, NativeAttentionCandidateObservation,
        NativeAttentionCandidatesObservation, NativeEventActorReferenceObservation,
        NativeEventQueueObservation, NativeMessageFlowObservation,
        NativePendingEventOrderObservation, NativePhysicalSlotObservation,
        NativePlayerActionObservation, NativePlayerRelationshipsObservation,
        NativeReturnPlaceWriterObservation, NativeRuntimeFileObservation,
    };

    fn context(sequence: u64) -> NativeSnapshotContext {
        NativeSnapshotContext {
            snapshot_id: format!("native.snapshot.{sequence}"),
            sequence,
            runtime_configuration: RuntimeConfiguration {
                schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                content_sha256: Digest([1; 32]),
                language: "en".into(),
                settings: BTreeMap::new(),
            },
            runtime_file_id: "runtime.fixture".into(),
            session_id: "session.fixture".into(),
            evidence_id: "native.fixture.v14".into(),
            evidence_sha256: Digest([2; 32]),
        }
    }

    fn observation() -> NativeLearningObservation {
        NativeLearningObservation {
            stage: "F_SP103".into(),
            room: 0,
            layer: 0,
            point: 1,
            player_present: true,
            player_is_link: true,
            player_position: [1.0, 2.0, 3.0],
            player_current_angle: [0, 0x1000, 0],
            player_form_present: true,
            player_action: Some(NativePlayerActionObservation {
                procedure_id: 0x1234,
            }),
            runtime_file_status: NativeChannelStatus::Present,
            runtime_file: Some(NativeRuntimeFileObservation {
                no_file_raw: 1,
                data_num_raw: 2,
                backing_attachment_status: NativeChannelStatus::Present,
                attached_physical_slot: Some(2),
                physical_slots: [
                    NativePhysicalSlotObservation {
                        number: 1,
                        ..Default::default()
                    },
                    NativePhysicalSlotObservation {
                        number: 2,
                        attached_to_runtime: true,
                        ..Default::default()
                    },
                    NativePhysicalSlotObservation {
                        number: 3,
                        ..Default::default()
                    },
                ],
            }),
            event_flags: Some(vec![0; 822]),
            temporary_flags: Some(vec![0; 185]),
            temporary_event_bytes: Some(vec![0; 256]),
            event_handoff_status: NativeChannelStatus::Present,
            event_handoff: Some(NativeEventHandoffObservation {
                get_item_no: 0x43,
                message_flow_status: NativeChannelStatus::Present,
                message_flow: Some(NativeMessageFlowObservation {
                    flow_id: 7,
                    node_index: 2,
                    cut_name_hash: 0,
                }),
                message_cut_status: NativeChannelStatus::Unavailable,
                ..Default::default()
            }),
            message_session_status: NativeChannelStatus::Present,
            message_session: Some(NativeMessageSessionObservation {
                procedure: 6,
                message_id: 0x123456,
                message_index: 17,
                node_index: 9,
                flow_id: 0x777,
                selection_count: 3,
                selection_cursor: 1,
                selection_push: 2,
                output_type: 4,
                talk_now: true,
                talk_message: true,
                send: true,
                talk_actor: NativeActorIdentity {
                    present: true,
                    runtime_generation: 7,
                    actor_name: 0x123,
                },
                ..Default::default()
            }),
            event_queue_status: NativeChannelStatus::Present,
            event_queue: Some(NativeEventQueueObservation {
                pending_orders: vec![NativePendingEventOrderObservation {
                    event_type: 0,
                    event_id: 12,
                    priority: 2,
                    map_tool_id: 3,
                    request_actor: NativeEventActorReferenceObservation {
                        status: NativeChannelStatus::Present,
                        actor: Some(NativeActorIdentity {
                            present: true,
                            runtime_generation: 42,
                            actor_name: 0x123,
                        }),
                    },
                    target_actor: NativeEventActorReferenceObservation {
                        status: NativeChannelStatus::Absent,
                        actor: None,
                    },
                    ..Default::default()
                }],
                active_request_actor: NativeEventActorReferenceObservation {
                    status: NativeChannelStatus::Present,
                    actor: Some(NativeActorIdentity {
                        present: true,
                        runtime_generation: 42,
                        actor_name: 0x123,
                    }),
                },
                skip_actor: NativeEventActorReferenceObservation {
                    status: NativeChannelStatus::Absent,
                    actor: None,
                },
                ..Default::default()
            }),
            attention_candidates_status: NativeChannelStatus::Present,
            attention_candidates: Some(NativeAttentionCandidatesObservation {
                player_attention_flags: 0x1234,
                attention_status: 2,
                attention_block_timer: 3,
                action_candidates: vec![NativeAttentionCandidateObservation {
                    actor: NativeEventActorReferenceObservation {
                        status: NativeChannelStatus::Present,
                        actor: Some(NativeActorIdentity {
                            present: true,
                            runtime_generation: 42,
                            actor_name: 0x123,
                        }),
                    },
                    weight: 0.5,
                    distance: 90.0,
                    angle: 0x200,
                    attention_type: 6,
                }],
                ..Default::default()
            }),
            actors: vec![NativeActorObservation {
                runtime_generation: 42,
                return_place_writer: Some(NativeReturnPlaceWriterObservation {
                    save_room: 3,
                    required_switch_set: 8,
                    ..Default::default()
                }),
            }],
            player_relationships_status: NativeChannelStatus::Present,
            player_relationships: Some(NativePlayerRelationshipsObservation {
                ride_actor: Some(NativeActorIdentity {
                    present: true,
                    runtime_generation: 5,
                    actor_name: 0x123,
                }),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn projects_native_backing_components_writers_and_explicit_unknowns() {
        let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
        snapshot.validate().unwrap();
        assert_eq!(
            snapshot.environment.active_runtime_file.backing,
            BackingAttachment::CardBacked {
                slot: PhysicalSlotId(2)
            }
        );
        assert!(snapshot.environment.physical_slots.is_empty());
        assert_eq!(snapshot.environment.physical_slot_observations.len(), 3);
        assert!(
            snapshot
                .environment
                .physical_slot_observations
                .iter()
                .all(|slot| slot.content_status == CaptureStatus::NotSampled)
        );
        let handoff = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "event-handoff")
            .unwrap();
        let ComponentPayload::Structured { fields } = &handoff.payload else {
            panic!("event handoff must be structured");
        };
        assert_eq!(
            fields["message_flow_status"],
            StateValue::Text("present".into())
        );
        assert_eq!(
            fields["message_cut_status"],
            StateValue::Text("unavailable".into())
        );
        assert!(!fields.contains_key("get_item_no"));
        let recent_item = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "event-recent-item")
            .unwrap();
        assert_eq!(recent_item.component_kind, ComponentKind::Session);
        assert_eq!(recent_item.lifetime, SemanticLifetime::Session);
        let ComponentPayload::Structured { fields } = &recent_item.payload else {
            panic!("recent item must be structured");
        };
        assert_eq!(fields["get_item_no"], StateValue::Unsigned(0x43));
        let writer = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id.ends_with(".return-place-writer"))
            .unwrap();
        assert_eq!(writer.component_kind, ComponentKind::ActorInstance);
        let ComponentPayload::Structured { fields } = &writer.payload else {
            panic!("return-place writer must be structured");
        };
        assert_eq!(fields["save_room"], StateValue::Signed(3));
        assert_eq!(fields["required_switch_set"], StateValue::Unsigned(8));
        assert_eq!(fields["no_telop_clear"], StateValue::Boolean(false));
        assert_eq!(fields["eligible"], StateValue::Boolean(false));
        assert_eq!(snapshot.environment.live_world_objects.len(), 1);
        assert_eq!(
            snapshot.environment.live_world_objects[0].actor_type,
            "kytag14.return-place-writer"
        );
    }

    #[test]
    fn dungeon_resources_remain_in_the_bound_stage_bank_not_runtime_inventory() {
        let mut observation = observation();
        observation.stage = "D_MN05".into();
        observation.player_resources_status = NativeChannelStatus::Present;
        observation.player_resources = Some(NativePlayerResourcesObservation {
            small_keys: 3,
            dungeon_map: true,
            dungeon_compass: true,
            dungeon_boss_key: true,
            dungeon_warp: true,
            ..Default::default()
        });
        let mut stage_memory = vec![0_u8; 0x20];
        stage_memory[0x1c] = 3;
        stage_memory[0x1d] = 0b0100_0111;
        observation.dungeon_flags = Some(stage_memory);

        let snapshot = snapshot_native_observation(&observation, context(1)).unwrap();
        let inventory = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "inventory-and-resources")
            .unwrap();
        let ComponentPayload::Structured { fields } = &inventory.payload else {
            panic!("inventory must be structured");
        };
        for local_field in [
            "small_keys",
            "dungeon_map",
            "dungeon_compass",
            "dungeon_boss_key",
            "dungeon_warp",
        ] {
            assert!(!fields.contains_key(local_field));
        }

        let dungeon = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "flags.dungeon")
            .unwrap();
        assert_eq!(dungeon.component_kind, ComponentKind::DungeonMemory);
        assert_eq!(
            dungeon.binding,
            ComponentBinding::Stage {
                stage: "D_MN05".into()
            }
        );
        assert_eq!(
            dungeon.serialization_owner,
            SerializationOwner::StageBank {
                runtime_file_id: "runtime.fixture".into(),
                stage: "D_MN05".into()
            }
        );
        let ComponentPayload::Raw { bytes, .. } = &dungeon.payload else {
            panic!("stage memory must retain raw bytes");
        };
        assert_eq!(bytes[0x1c], 3);
        assert_eq!(bytes[0x1d], 0b0100_0111);
    }

    #[test]
    fn refuses_to_invent_a_runtime_or_player_for_missing_channels() {
        let mut observation = observation();
        observation.runtime_file_status = NativeChannelStatus::NotSampled;
        observation.runtime_file = None;
        let error = snapshot_native_observation(&observation, context(1)).unwrap_err();
        assert_eq!(error.field(), "native_observation.runtime_file");
    }

    #[test]
    fn projects_global_message_session_as_generic_flow_state() {
        let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
        let message = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "message-session")
            .unwrap();
        assert_eq!(message.component_kind, ComponentKind::MessageFlow);
        let ComponentPayload::Structured { fields } = &message.payload else {
            panic!("message session must be structured");
        };
        assert_eq!(fields["capture_status"], StateValue::Text("present".into()));
        assert_eq!(fields["message_id"], StateValue::Unsigned(0x123456));
        assert_eq!(fields["flow_id"], StateValue::Signed(0x777));
        assert_eq!(fields["node_index"], StateValue::Unsigned(9));
        assert_eq!(fields["talk_now"], StateValue::Boolean(true));
        assert_eq!(
            fields["talk_actor_runtime_generation"],
            StateValue::Unsigned(7)
        );
    }

    #[test]
    fn projects_event_requests_and_participants_without_native_pointers() {
        let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
        let queue = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "event-queue")
            .unwrap();
        assert_eq!(queue.component_kind, ComponentKind::PendingOperation);
        let ComponentPayload::Structured { fields } = &queue.payload else {
            panic!("event queue must be structured");
        };
        assert_eq!(fields["capture_status"], StateValue::Text("present".into()));
        assert_eq!(fields["pending_count"], StateValue::Unsigned(1));
        assert_eq!(fields["pending.0.event_type"], StateValue::Unsigned(0));
        assert_eq!(fields["pending.0.priority"], StateValue::Unsigned(2));
        assert_eq!(
            fields["pending.0.request_actor.runtime_generation"],
            StateValue::Unsigned(42)
        );
        assert_eq!(
            fields["pending.0.target_actor.status"],
            StateValue::Text("absent".into())
        );
    }

    #[test]
    fn projects_attention_candidates_without_selecting_one() {
        let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
        let attention = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "attention-candidates")
            .unwrap();
        assert_eq!(attention.component_kind, ComponentKind::PendingOperation);
        let ComponentPayload::Structured { fields } = &attention.payload else {
            panic!("attention candidates must be structured");
        };
        assert_eq!(fields["capture_status"], StateValue::Text("present".into()));
        assert_eq!(fields["action.count"], StateValue::Unsigned(1));
        assert_eq!(fields["action.0.attention_type"], StateValue::Unsigned(6));
        assert_eq!(
            fields["action.0.actor.runtime_generation"],
            StateValue::Unsigned(42)
        );
        assert_eq!(
            fields["action.0.distance_f32_bits"],
            StateValue::Unsigned(90.0_f32.to_bits().into())
        );
    }

    #[test]
    fn separates_label_observations_from_writable_temporary_register_backing() {
        let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
        let event_labels = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "flags.event")
            .unwrap();
        assert_eq!(
            event_labels.component_kind,
            ComponentKind::Custom {
                id: "observed-event-flag-labels".into()
            }
        );
        let temporary_labels = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "flags.temporary")
            .unwrap();
        assert_eq!(
            temporary_labels.component_kind,
            ComponentKind::Custom {
                id: "observed-temporary-flag-labels".into()
            }
        );
        let temporary_registers = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "flags.temporary-event-registers")
            .unwrap();
        assert_eq!(
            temporary_registers.component_kind,
            ComponentKind::TemporaryFlags
        );
        assert_eq!(
            snapshot
                .environment
                .components
                .iter()
                .filter(|component| {
                    component.component_kind == ComponentKind::TemporaryFlags
                        && matches!(component.payload, ComponentPayload::Raw { .. })
                })
                .count(),
            1
        );
    }

    #[test]
    fn chains_label_boundaries_and_retain_exact_raw_byte_diffs() {
        let first = observation();
        let mut second = first.clone();
        second.temporary_event_bytes.as_mut().unwrap()[19] ^= 0x04;
        second
            .event_handoff
            .as_mut()
            .unwrap()
            .message_flow
            .as_mut()
            .unwrap()
            .node_index += 1;

        let mut evidence = NativeStateEvidence::begin(&first, context(1)).unwrap();
        evidence
            .append(&second, context(2), BoundaryKind::DialogueInterruption)
            .unwrap();
        assert_eq!(evidence.snapshots.len(), 2);
        assert_eq!(evidence.diffs.len(), 1);
        assert_eq!(evidence.chain.entries.len(), 2);
        assert_eq!(
            evidence.chain.entries[1].incoming_boundary,
            Some(BoundaryKind::DialogueInterruption)
        );
        let temporary_delta = evidence.diffs[0]
            .component_deltas
            .iter()
            .find(|delta| delta.component_id == "flags.temporary-event-registers")
            .unwrap();
        assert_eq!(temporary_delta.raw_byte_deltas.len(), 1);
        assert_eq!(temporary_delta.raw_byte_deltas[0].offset, 19);
        assert!(
            evidence.diffs[0]
                .component_deltas
                .iter()
                .any(|delta| delta.component_id == "event-handoff")
        );
        evidence.chain.validate().unwrap();
    }
}
