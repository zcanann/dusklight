#include "dusk/automation/learning_episode.hpp"

#include "f_pc/f_pc_name.h"

#include "dusk/automation/game_state_observer.hpp"

#include <algorithm>
#include <array>
#include <bit>
#include <cmath>
#include <cstring>
#include <limits>
#include <system_error>
#include <type_traits>

#include <xxhash.h>
#include <zstd.h>

namespace dusk::automation {
namespace {

constexpr std::size_t ShardHeaderSize = 128;
constexpr std::size_t EpisodeBlockHeaderSize = 64;
constexpr std::size_t EpisodePayloadHeaderSize = 24;
constexpr std::uint32_t ShardComplete = 1u << 0;
constexpr std::uint16_t EpisodeSuccess = 1u << 0;
// Stable fopAc_Group_e wire value. Keep the serializer independent of the
// heavyweight game actor header used only by the read-only observer adapter.
constexpr std::uint8_t EnemyActorGroup = 2;

bool is_lower_hex(const std::string_view value, const std::size_t size) {
    return value.size() == size && std::ranges::all_of(value, [](const char byte) {
        return (byte >= '0' && byte <= '9') || (byte >= 'a' && byte <= 'f');
    });
}

template <typename T>
void append_integer(std::vector<std::uint8_t>& output, const T value) {
    using U = std::make_unsigned_t<T>;
    U bits = static_cast<U>(value);
    for (std::size_t index = 0; index < sizeof(T); ++index) {
        output.push_back(static_cast<std::uint8_t>(bits & 0xff));
        bits >>= 8;
    }
}

template <typename T>
void write_integer(std::ostream& output, const T value) {
    std::array<std::uint8_t, sizeof(T)> bytes{};
    using U = std::make_unsigned_t<T>;
    U bits = static_cast<U>(value);
    for (std::size_t index = 0; index < bytes.size(); ++index) {
        bytes[index] = static_cast<std::uint8_t>(bits & 0xff);
        bits >>= 8;
    }
    output.write(reinterpret_cast<const char*>(bytes.data()), bytes.size());
}

void patch_u32(
    std::vector<std::uint8_t>& output, const std::size_t offset, const std::uint32_t value) {
    for (std::size_t index = 0; index < sizeof(value); ++index)
        output[offset + index] = static_cast<std::uint8_t>(value >> (index * 8));
}

bool append_float(std::vector<std::uint8_t>& output, float value, std::string& error) {
    if (!std::isfinite(value)) {
        error = "learning observation contains a nonfinite float";
        return false;
    }
    if (value == 0.0F)
        value = 0.0F;
    append_integer(output, std::bit_cast<std::uint32_t>(value));
    return true;
}

bool decode_hex_128(const std::string_view text, std::array<std::uint8_t, 16>& output) {
    if (text.size() != output.size() * 2)
        return false;
    const auto nibble = [](const char value) -> int {
        if (value >= '0' && value <= '9')
            return value - '0';
        if (value >= 'a' && value <= 'f')
            return value - 'a' + 10;
        return -1;
    };
    for (std::size_t index = 0; index < output.size(); ++index) {
        const int high = nibble(text[index * 2]);
        const int low = nibble(text[index * 2 + 1]);
        if (high < 0 || low < 0)
            return false;
        output[index] = static_cast<std::uint8_t>((high << 4) | low);
    }
    return true;
}

void append_fixed_name(std::vector<std::uint8_t>& output, const char* name) {
    std::array<char, 8> bytes{};
    if (name != nullptr) {
        for (std::size_t index = 0; index < bytes.size() && name[index] != '\0'; ++index)
            bytes[index] = name[index];
    }
    output.insert(output.end(), bytes.begin(), bytes.end());
}

void append_raw_pad(std::vector<std::uint8_t>& output, const RawPadState& pad) {
    std::array<std::uint8_t, kRawPadStateSize> bytes{};
    encode_raw_pad_state(pad, bytes);
    output.insert(output.end(), bytes.begin(), bytes.end());
}

bool append_actor_identity(std::vector<std::uint8_t>& output,
    const MilestoneObservation::ActorIdentity& actor, std::string& error) {
    append_integer(output, static_cast<std::uint8_t>(actor.present));
    append_integer(output, actor.runtimeGeneration);
    append_integer(output, actor.actorName);
    append_integer(output, actor.setId);
    append_integer(output, actor.homeRoom);
    append_integer(output, actor.currentRoom);
    append_integer(output, static_cast<std::uint8_t>(actor.homePositionPresent));
    append_integer<std::uint8_t>(output, 0);
    return append_float(output, actor.homePositionPresent ? actor.homePositionX : 0.0F, error) &&
           append_float(output, actor.homePositionPresent ? actor.homePositionY : 0.0F, error) &&
           append_float(output, actor.homePositionPresent ? actor.homePositionZ : 0.0F, error);
}

template <std::size_t Size>
bool append_float_array(
    std::vector<std::uint8_t>& output, const std::array<float, Size>& values, std::string& error) {
    for (const float value : values) {
        if (!append_float(output, value, error))
            return false;
    }
    return true;
}

bool append_trace_actor_identity(std::vector<std::uint8_t>& output,
    const GameplayTraceActorIdentitySample& actor, std::string& error) {
    append_integer(output, actor.sessionProcessId);
    append_integer(output, actor.actorName);
    append_integer(output, actor.setId);
    append_integer(output, actor.homeRoom);
    append_integer(output, actor.currentRoom);
    append_integer<std::uint16_t>(output, 0);
    return append_float_array(output, actor.homePosition, error);
}

bool append_mechanics_channels(std::vector<std::uint8_t>& output, const GameplayTraceSample* trace,
    const GameplayCollisionPlanesObservation& planes, const GameplayPlayerFormObservation& form,
    std::string& error) {
    const GameplayTraceSample empty{};
    const GameplayTraceSample& sample = trace == nullptr ? empty : *trace;
    const auto valid_status = [](const GameplayTraceChannelStatus status) {
        return status == GameplayTraceChannelStatus::NotSampled ||
               status == GameplayTraceChannelStatus::Present ||
               status == GameplayTraceChannelStatus::Absent ||
               status == GameplayTraceChannelStatus::Unavailable;
    };
    if (!valid_status(sample.cameraStatus) || !valid_status(sample.playerActionStatus) ||
        !valid_status(sample.sceneExitStatus) ||
        !valid_status(sample.playerBackgroundCollisionStatus) ||
        !valid_status(sample.playerCollisionSurfacesStatus) || (planes.validMask & ~0x3fu) != 0 ||
        (trace == nullptr && (planes.validMask != 0 || form.present || form.wolf)) ||
        (form.wolf && !form.present))
    {
        error = "learning observation has inconsistent mechanics channels";
        return false;
    }
    for (std::size_t index = 0; index < sample.playerCollisionSurfaces.surfaces.size(); ++index) {
        if ((planes.validMask & (1u << index)) != 0 &&
            (sample.playerCollisionSurfaces.surfaces[index].flags &
                GameplayTraceCollisionSurfaceIdentityPresent) == 0)
        {
            error = "learning observation collision plane lacks a surface identity";
            return false;
        }
    }
    append_integer(output, static_cast<std::uint8_t>(sample.cameraStatus));
    append_integer(output, static_cast<std::uint8_t>(sample.playerActionStatus));
    append_integer(output, static_cast<std::uint8_t>(sample.playerBackgroundCollisionStatus));
    append_integer(output, static_cast<std::uint8_t>(sample.playerCollisionSurfacesStatus));
    append_integer(output, static_cast<std::uint8_t>(sample.sceneExitStatus));
    append_integer(output, planes.validMask);
    const std::uint8_t formFlags = (form.present ? 1u : 0u) | (form.wolf ? 2u : 0u);
    append_integer(output, formFlags);
    append_integer<std::uint8_t>(output, 0);

    const GameplayTraceCameraSample& camera = sample.camera;
    append_integer(output, camera.viewYaw);
    append_integer(output, camera.controlledYaw);
    append_integer(output, camera.bank);
    append_integer<std::uint16_t>(output, 0);
    if (!append_float_array(output, camera.eye, error) ||
        !append_float_array(output, camera.center, error) ||
        !append_float_array(output, camera.up, error) || !append_float(output, camera.fovy, error))
        return false;

    const GameplayTracePlayerActionSample& action = sample.playerAction;
    append_integer(output, action.procedureId);
    append_integer<std::uint16_t>(output, 0);
    append_integer(output, action.modeFlags);
    for (const std::int16_t value : action.procedureContextRaw)
        append_integer(output, value);
    append_integer(output, action.damageWaitTimer);
    append_integer(output, action.swordAtUpTime);
    append_integer(output, action.iceDamageWaitTimer);
    append_integer(output, action.swordChangeWaitTimer);
    for (std::size_t index = 0; index < 5; ++index)
        append_integer<std::uint8_t>(output, 0);
    for (const GameplayTraceAnimationLane& lane : action.underAnimations) {
        append_integer(output, lane.resourceId);
        append_integer<std::uint16_t>(output, 0);
        if (!append_float(output, lane.frame, error) || !append_float(output, lane.rate, error))
            return false;
    }
    for (const GameplayTraceAnimationLane& lane : action.upperAnimations) {
        append_integer(output, lane.resourceId);
        append_integer<std::uint16_t>(output, 0);
        if (!append_float(output, lane.frame, error) || !append_float(output, lane.rate, error))
            return false;
    }
    append_integer(output, action.flags);
    append_integer(output, action.doStatus);
    append_integer<std::uint8_t>(output, 0);
    append_integer<std::uint16_t>(output, 0);
    if (!append_trace_actor_identity(output, action.talkPartner, error) ||
        !append_trace_actor_identity(output, action.grabbedActor, error))
        return false;

    const GameplayTraceSceneExitSample& exit = sample.sceneExit;
    append_integer(output, exit.sessionProcessId);
    append_integer(output, exit.rawParameters);
    append_integer(output, exit.flags);
    if (!append_float(output, exit.signedDistanceToVolume, error))
        return false;
    append_integer(output, exit.actorName);
    append_integer(output, exit.setId);
    append_integer(output, exit.exitId);
    append_integer(output, exit.pathId);
    append_integer(output, exit.argument1);
    append_integer(output, exit.switchNo);
    append_integer(output, exit.kind);
    append_integer(output, exit.observedCount);
    append_integer(output, exit.homeRoom);
    append_integer(output, exit.linkExitDirection);
    append_integer(output, exit.linkExitId);
    append_integer(output, exit.shapeYaw);
    append_integer(output, exit.actorAction);
    append_integer<std::uint8_t>(output, 0);
    append_integer<std::uint16_t>(output, 0);
    if (!append_float_array(output, exit.playerLocalPosition, error) ||
        !append_float_array(output, exit.volumeExtent, error) ||
        !append_float_array(output, exit.homePosition, error))
        return false;
    output.insert(output.end(), exit.destinationStage.begin(), exit.destinationStage.end());
    append_integer(output, exit.destinationRoom);
    append_integer(output, exit.destinationLayer);
    append_integer(output, exit.destinationPoint);
    append_integer(output, exit.destinationWipe);
    append_integer(output, exit.destinationWipeTime);
    append_integer(output, exit.destinationTimeHour);
    append_integer<std::uint8_t>(output, 0);

    const GameplayTracePlayerBackgroundCollisionSample& background =
        sample.playerBackgroundCollision;
    append_integer(output, background.flags);
    if (!append_float(output, background.groundHeight, error) ||
        !append_float(output, background.roofHeight, error) ||
        !append_float(output, background.waterHeight, error))
        return false;
    append_integer(output, background.groundBgIndex);
    append_integer(output, background.groundPolyIndex);
    append_integer(output, background.groundOwnerSessionProcessId);
    if (!append_float_array(output, background.groundPlane, error))
        return false;
    append_integer(output, background.roofBgIndex);
    append_integer(output, background.roofPolyIndex);
    append_integer(output, background.roofOwnerSessionProcessId);
    append_integer(output, background.waterBgIndex);
    append_integer(output, background.waterPolyIndex);
    append_integer(output, background.waterOwnerSessionProcessId);
    for (const GameplayTraceCollisionWallSample& wall : background.walls) {
        append_integer(output, wall.bgIndex);
        append_integer(output, wall.polyIndex);
        append_integer(output, wall.ownerSessionProcessId);
        append_integer(output, wall.angleY);
        append_integer(output, wall.flags);
    }
    if (!append_float_array(output, background.oldPosition, error) ||
        !append_float_array(output, background.resolvedFrameDisplacement, error) ||
        !append_float_array(output, background.finalPosition, error))
        return false;

    const GameplayTracePlayerCollisionSurfacesSample& surfaces = sample.playerCollisionSurfaces;
    append_integer(output, surfaces.flags);
    append_integer(output, surfaces.currentRoom);
    append_integer(output, surfaces.identityCount);
    append_integer(output, surfaces.backingCodeCount);
    append_integer(output, surfaces.destinationCount);
    append_integer(output, surfaces.rawLinkExit);
    append_integer(output, surfaces.pendingStageMatchMask);
    append_integer<std::uint8_t>(output, 0);
    for (std::size_t index = 0; index < surfaces.surfaces.size(); ++index) {
        const GameplayTraceCollisionSurfaceSample& surface = surfaces.surfaces[index];
        append_integer(output, surface.flags);
        append_integer(output, surface.kind);
        append_integer(output, surface.wallSlot);
        append_integer(output, surface.backingFormat);
        append_integer(output, surface.rawCodePresenceMask);
        append_integer(output, surface.bgIndex);
        append_integer(output, surface.polyIndex);
        append_integer(output, surface.ownerSessionProcessId);
        append_integer(output, surface.materialIndex);
        append_integer(output, surface.groupIndex);
        for (const std::uint32_t code : surface.rawCodes)
            append_integer(output, code);
        append_integer(output, surface.rawExitId);
        append_integer(output, surface.sourceRoom);
        append_integer(output, surface.sclsSourceRoom);
        append_integer(output, surface.destinationRoom);
        append_integer(output, surface.destinationLayer);
        append_integer(output, surface.destinationWipe);
        append_integer(output, surface.destinationWipeTime);
        append_integer(output, surface.destinationTimeHour);
        append_integer(output, surface.destinationPoint);
        append_integer(output, surface.sourceGeometryIndexCount);
        append_integer<std::uint8_t>(output, 0);
        for (const std::uint16_t geometryIndex : surface.sourceGeometryIndices)
            append_integer(output, geometryIndex);
        if (!append_float(output, surface.kclPrismHeight, error))
            return false;
        output.insert(
            output.end(), surface.destinationStage.begin(), surface.destinationStage.end());
        if (!append_float_array(output, planes.planes[index], error))
            return false;
    }
    return true;
}

bool append_string16(
    std::vector<std::uint8_t>& output, const std::string_view value, std::string& error) {
    if (value.size() > std::numeric_limits<std::uint16_t>::max()) {
        error = "learning episode metadata string exceeds u16 length";
        return false;
    }
    append_integer(output, static_cast<std::uint16_t>(value.size()));
    output.insert(output.end(), value.begin(), value.end());
    return true;
}

bool append_planner_runtime_state(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, std::string& error) {
    using Status = MilestoneObservation::ChannelStatus;
    const auto validStatus = [](const Status status) {
        return status == Status::NotSampled || status == Status::Present ||
               status == Status::Absent || status == Status::Unavailable;
    };
    const auto statusByte = [](const Status status) { return static_cast<std::uint8_t>(status); };
    const auto& runtime = observation.runtimeFile;
    const bool attached = runtime.backingAttachmentStatus == Status::Present;
    std::uint8_t attachedSlotMask = 0;
    for (std::size_t index = 0; index < runtime.physicalSlots.size(); ++index) {
        const auto& slot = runtime.physicalSlots[index];
        if (slot.number != index + 1 || !validStatus(slot.contentStatus)) {
            error = "learning observation has invalid physical-slot state";
            return false;
        }
        if (slot.attachedToRuntime)
            attachedSlotMask |= static_cast<std::uint8_t>(1u << index);
    }
    if (runtime.status != Status::Present || !validStatus(runtime.backingAttachmentStatus) ||
        (attached && (runtime.noFileRaw != 0 || runtime.dataNumRaw >= 3 ||
                         runtime.attachedPhysicalSlot != runtime.dataNumRaw + 1 ||
                         attachedSlotMask != (1u << runtime.dataNumRaw))) ||
        (!attached && (runtime.attachedPhysicalSlot != -1 || attachedSlotMask != 0)))
    {
        error = "learning observation has inconsistent runtime-file backing";
        return false;
    }

    const auto& returnPlace = observation.returnPlace;
    const auto& restart = observation.restart;
    const auto& handoff = observation.eventHandoff;
    const auto fixedStringIsCanonical = [](const auto& value) {
        const auto terminator = std::ranges::find(value, '\0');
        return terminator == value.end() || std::ranges::all_of(terminator, value.end(),
                                                [](const char byte) { return byte == 0; });
    };
    if (returnPlace.status != Status::Present || restart.status != Status::Present ||
        !fixedStringIsCanonical(returnPlace.stage) || !validStatus(handoff.status) ||
        !validStatus(handoff.eventNameStatus) || !validStatus(handoff.messageFlowStatus) ||
        !validStatus(handoff.messageCutStatus) || !validStatus(handoff.pendingCleanupStatus) ||
        !validStatus(handoff.playerControlStatus) || !validStatus(handoff.noTelopStatus) ||
        (handoff.eventNameStatus == Status::Present &&
            !fixedStringIsCanonical(handoff.eventName)) ||
        (handoff.messageFlowStatus != Status::Present &&
            (handoff.messageFlowId != 0 || handoff.messageNodeIndex != 0)) ||
        (handoff.messageCutStatus != Status::Present && handoff.messageCutHash != 0) ||
        (handoff.pendingCleanupStatus != Status::Present && handoff.pendingCleanupFlags != 0) ||
        (handoff.playerControlStatus != Status::Present &&
            (handoff.playerControlModeFlags != 0 || handoff.playerControlDoStatus != 0)) ||
        handoff.noTelopStatus != Status::Present)
    {
        error = "learning observation has inconsistent planner runtime channels";
        return false;
    }

    append_integer(output, statusByte(runtime.status));
    append_integer(output, statusByte(runtime.backingAttachmentStatus));
    append_integer(output, runtime.noFileRaw);
    append_integer(output, runtime.dataNumRaw);
    append_integer(output, runtime.attachedPhysicalSlot);
    append_integer(output, attachedSlotMask);
    for (const auto& slot : runtime.physicalSlots)
        append_integer(output, statusByte(slot.contentStatus));
    append_integer<std::uint8_t>(output, 0);

    append_integer(output, statusByte(returnPlace.status));
    append_integer(output, returnPlace.room);
    append_integer(output, returnPlace.playerStatus);
    append_integer<std::uint8_t>(output, 0);
    output.insert(output.end(), returnPlace.stage.begin(), returnPlace.stage.end());

    append_integer(output, statusByte(restart.status));
    append_integer(output, restart.room);
    append_integer(output, restart.startPoint);
    append_integer(output, restart.angleY);
    append_integer(output, restart.lastAngleY);
    for (const float value :
        {restart.positionX, restart.positionY, restart.positionZ, restart.lastSpeed})
    {
        if (!append_float(output, value, error))
            return false;
    }
    append_integer(output, restart.roomParam);
    append_integer(output, restart.lastMode);

    for (const Status status : {handoff.status, handoff.eventNameStatus, handoff.messageFlowStatus,
             handoff.pendingCleanupStatus, handoff.playerControlStatus, handoff.noTelopStatus})
        append_integer(output, statusByte(status));
    append_integer(output, handoff.preItemNo);
    append_integer(output, handoff.getItemNo);
    append_integer(output, handoff.talkXyType);
    append_integer(output, handoff.compulsory);
    append_integer(output, static_cast<std::uint8_t>(handoff.roomInfoSet));
    append_integer(output, static_cast<std::uint8_t>(handoff.noTelop));
    append_integer(output, handoff.eventFlags);
    append_integer(output, handoff.secondaryFlags);
    append_integer(output, handoff.hindFlags);
    append_integer<std::uint16_t>(output, 0);
    append_integer(output, handoff.skipTimer);
    append_integer(output, handoff.skipParameter);
    append_integer(output, handoff.messageFlowId);
    append_integer(output, handoff.messageNodeIndex);
    append_integer(output, handoff.messageCutHash);
    append_integer(output, handoff.pendingCleanupFlags);
    append_integer(output, handoff.playerControlModeFlags);
    append_integer(output, handoff.playerControlDoStatus);
    append_integer(output, statusByte(handoff.messageCutStatus));
    append_integer<std::uint16_t>(output, 0);
    if (!append_actor_identity(output, handoff.itemPartner, error))
        return false;
    output.insert(output.end(), handoff.eventName.begin(), handoff.eventName.end());
    return true;
}

bool append_message_session_state(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, std::string& error) {
    using Status = MilestoneObservation::ChannelStatus;
    using Message = MilestoneObservation::MessageSessionState;
    const auto& message = observation.messageSession;
    constexpr std::uint16_t KnownFlags =
        Message::TalkNow | Message::TalkMessage | Message::AutoMessage | Message::KillPending |
        Message::CameraCancel | Message::Send | Message::SendControl;
    const bool statusValid = message.status == Status::Present ||
                             message.status == Status::Absent ||
                             message.status == Status::Unavailable;
    const bool emptyPayload =
        message.procedure == 0 && message.messageId == 0 && message.messageIndex == 0 &&
        message.nodeIndex == 0 && message.flowId == 0 && message.selectionCount == 0 &&
        message.selectionCursor == 0 && message.selectionPush == 0 && message.outputType == 0 &&
        message.flags == 0 && !message.talkActor.present;
    const bool talkActorJoined =
        !message.talkActor.present ||
        std::ranges::binary_search(observation.actors, message.talkActor.runtimeGeneration, {},
            &MilestoneObservation::Actor::runtimeGeneration);
    if (!statusValid || (message.flags & ~KnownFlags) != 0 ||
        (message.status != Status::Present && !emptyPayload) ||
        message.talkActor.present != message.talkActor.homePositionPresent || !talkActorJoined)
    {
        error = "learning observation has inconsistent message-session state";
        return false;
    }

    append_integer(output, static_cast<std::uint8_t>(message.status));
    append_integer<std::uint8_t>(output, 0);
    append_integer(output, message.procedure);
    append_integer(output, message.messageId);
    append_integer(output, message.messageIndex);
    append_integer(output, message.nodeIndex);
    append_integer(output, message.flowId);
    append_integer(output, message.selectionCount);
    append_integer(output, message.selectionCursor);
    append_integer(output, message.selectionPush);
    append_integer(output, message.outputType);
    append_integer(output, message.flags);
    append_integer<std::uint16_t>(output, 0);
    return append_actor_identity(output, message.talkActor, error);
}

bool append_event_queue_state(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, std::string& error) {
    using Status = MilestoneObservation::ChannelStatus;
    using EventQueue = MilestoneObservation::EventQueueState;
    const auto& queue = observation.eventQueue;
    const bool statusValid = queue.status == Status::Present || queue.status == Status::Unavailable;
    const bool countValid = queue.pendingCount <= EventQueue::MaximumPendingOrders;
    const auto joined = [&](const MilestoneObservation::ActorIdentity& identity) {
        if (!identity.present)
            return false;
        const auto found = std::ranges::lower_bound(observation.actors, identity.runtimeGeneration,
            {}, &MilestoneObservation::Actor::runtimeGeneration);
        return found != observation.actors.end() &&
               found->runtimeGeneration == identity.runtimeGeneration &&
               found->actorName == identity.actorName && found->setId == identity.setId &&
               found->homeRoom == identity.homeRoom && found->currentRoom == identity.currentRoom &&
               identity.homePositionPresent && found->homePositionX == identity.homePositionX &&
               found->homePositionY == identity.homePositionY &&
               found->homePositionZ == identity.homePositionZ;
    };
    const auto referenceValid = [&](const EventQueue::ActorReference& reference,
                                    const bool channelPresent) {
        if (!channelPresent)
            return reference.status == Status::NotSampled && !reference.identity.present;
        if (reference.status == Status::Present)
            return joined(reference.identity);
        return (reference.status == Status::Absent || reference.status == Status::Unavailable) &&
               !reference.identity.present;
    };
    const std::array<const EventQueue::ActorReference*, 7> participants{
        &queue.activeRequestActor,
        &queue.activeTargetActor,
        &queue.activeTalkActor,
        &queue.activeItemActor,
        &queue.activeDoorActor,
        &queue.changeActor,
        &queue.skipActor,
    };
    const bool channelPresent = queue.status == Status::Present;
    const bool participantsValid = std::ranges::all_of(participants,
        [&](const auto* reference) { return referenceValid(*reference, channelPresent); });
    bool pendingReferencesValid = true;
    bool pendingOrderValid = true;
    std::uint16_t previousPriority = 0;
    for (std::size_t index = 0; index < queue.pendingCount; ++index) {
        const auto& order = queue.pendingOrders[index];
        pendingReferencesValid &= referenceValid(order.requestActor, channelPresent) &&
                                  referenceValid(order.targetActor, channelPresent);
        const bool knownType = order.type <= 7 || (order.type >= 10 && order.type <= 13);
        pendingOrderValid &=
            knownType && order.priority != 0 && (index == 0 || previousPriority <= order.priority);
        previousPriority = order.priority;
    }
    const bool unavailablePayloadEmpty = queue.pendingCount == 0 && !queue.skipRegistered;
    const bool skipConsistent = queue.skipRegistered || queue.skipActor.status != Status::Present;
    if (!statusValid || !countValid || !participantsValid || !pendingReferencesValid ||
        !pendingOrderValid || !skipConsistent || (!channelPresent && !unavailablePayloadEmpty))
    {
        error = "learning observation has inconsistent event-queue state";
        return false;
    }

    append_integer(output, static_cast<std::uint8_t>(queue.status));
    append_integer(output, queue.pendingCount);
    append_integer(output, static_cast<std::uint8_t>(queue.skipRegistered));
    append_integer<std::uint8_t>(output, 0);
    const auto appendReference = [&](const EventQueue::ActorReference& reference) {
        append_integer(output, static_cast<std::uint8_t>(reference.status));
        append_integer<std::uint8_t>(output, 0);
        append_integer<std::uint16_t>(output, 0);
        return append_actor_identity(output, reference.identity, error);
    };
    for (std::size_t index = 0; index < queue.pendingCount; ++index) {
        const auto& order = queue.pendingOrders[index];
        append_integer(output, order.type);
        append_integer(output, order.flags);
        append_integer(output, order.hindFlags);
        append_integer(output, order.eventId);
        append_integer(output, order.priority);
        append_integer(output, order.mapToolId);
        append_integer<std::uint8_t>(output, 0);
        if (!appendReference(order.requestActor) || !appendReference(order.targetActor))
            return false;
    }
    for (const auto* participant : participants) {
        if (!appendReference(*participant))
            return false;
    }
    return true;
}

bool append_process_lifecycle_state(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, std::string& error) {
    using Status = MilestoneObservation::ChannelStatus;
    const auto& lifecycle = observation.processLifecycle;
    const bool present = lifecycle.status == Status::Present;
    const bool unavailable = lifecycle.status == Status::Unavailable;
    if ((!present && !unavailable) ||
        (present && lifecycle.activeActorCount != observation.actors.size()) ||
        (present && (lifecycle.pendingCreateCount != lifecycle.pendingCreates.size() ||
                        lifecycle.pendingDeleteCount != lifecycle.pendingDeletes.size())) ||
        (!present && (lifecycle.activeActorCount != 0 || lifecycle.pendingCreateCount != 0 ||
                         lifecycle.pendingDeleteCount != 0 || !lifecycle.pendingCreates.empty() ||
                         !lifecycle.pendingDeletes.empty())))
    {
        error = "learning observation has inconsistent process-lifecycle state";
        return false;
    }

    append_integer(output, static_cast<std::uint8_t>(lifecycle.status));
    append_integer<std::uint8_t>(output, 0);
    append_integer<std::uint16_t>(output, 0);
    append_integer(output, lifecycle.activeActorCount);
    append_integer(output, lifecycle.pendingCreateCount);
    append_integer(output, lifecycle.pendingDeleteCount);
    return true;
}

bool append_process_lifecycle_records(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, std::string& error) {
    using Status = MilestoneObservation::ChannelStatus;
    const auto& lifecycle = observation.processLifecycle;
    const bool present = lifecycle.status == Status::Present;
    if ((present && (lifecycle.pendingCreateCount != lifecycle.pendingCreates.size() ||
                        lifecycle.pendingDeleteCount != lifecycle.pendingDeletes.size())) ||
        (!present && (!lifecycle.pendingCreates.empty() || !lifecycle.pendingDeletes.empty()))) {
        error = "learning observation has inconsistent process-lifecycle records";
        return false;
    }
    using ProcessState = MilestoneObservation::ProcessLifecycleState::ProcessState;
    const auto processEmpty = [](const ProcessState& process) {
        return process.runtimeGeneration == 0 && process.processName == -1 &&
               process.profileName == -1 && process.processType == 0 &&
               process.processSubtype == 0 && process.parameters == 0 && process.initState == 0 &&
               process.createPhase == 0;
    };
    const auto appendProcess = [&](const ProcessState& process) {
        append_integer(output, process.runtimeGeneration);
        append_integer(output, process.processName);
        append_integer(output, process.profileName);
        append_integer(output, process.processType);
        append_integer(output, process.processSubtype);
        append_integer(output, process.parameters);
        append_integer(output, process.initState);
        append_integer(output, process.createPhase);
        append_integer<std::uint16_t>(output, 0);
    };
    for (const auto& pending : lifecycle.pendingCreates) {
        const bool processPresent = pending.processStatus == Status::Present;
        const bool processAbsent = pending.processStatus == Status::Absent;
        if ((!processPresent && !processAbsent) ||
            (processPresent && pending.process.runtimeGeneration != pending.runtimeGeneration) ||
            (processAbsent && !processEmpty(pending.process))) {
            error = "learning observation has inconsistent pending-create process state";
            return false;
        }
        append_integer(output, pending.runtimeGeneration);
        append_integer(output,
            static_cast<std::uint8_t>((pending.doing ? 1 : 0) | (pending.cancelled ? 2 : 0)));
        append_integer(output, static_cast<std::uint8_t>(pending.processStatus));
        append_integer<std::uint16_t>(output, 0);
        appendProcess(pending.process);
    }
    for (const auto& pending : lifecycle.pendingDeletes) {
        appendProcess(pending.process);
        append_integer(output, pending.timer);
        append_integer<std::uint16_t>(output, 0);
    }
    return true;
}

bool append_attention_candidate_state(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, std::string& error) {
    using Status = MilestoneObservation::ChannelStatus;
    using Attention = MilestoneObservation::AttentionCandidateState;
    using Reference = MilestoneObservation::EventQueueState::ActorReference;
    const auto& attention = observation.attentionCandidates;
    const bool statusValid =
        observation.playerPresent ?
            (attention.status == Status::Present || attention.status == Status::Unavailable) :
            attention.status == Status::Absent;
    const auto countOffsetValid = [](const std::uint8_t count, const std::uint8_t offset,
                                      const std::size_t capacity) {
        return count <= capacity && ((count == 0 && offset == 0) || (count != 0 && offset < count));
    };
    const auto joined = [&](const MilestoneObservation::ActorIdentity& identity) {
        if (!identity.present)
            return false;
        const auto found = std::ranges::lower_bound(observation.actors, identity.runtimeGeneration,
            {}, &MilestoneObservation::Actor::runtimeGeneration);
        return found != observation.actors.end() &&
               found->runtimeGeneration == identity.runtimeGeneration &&
               found->actorName == identity.actorName && found->setId == identity.setId &&
               found->homeRoom == identity.homeRoom && found->currentRoom == identity.currentRoom &&
               identity.homePositionPresent && found->homePositionX == identity.homePositionX &&
               found->homePositionY == identity.homePositionY &&
               found->homePositionZ == identity.homePositionZ;
    };
    const auto candidateValid = [&](const Attention::Candidate& candidate, const bool retained) {
        if (!retained)
            return candidate.actor.status == Status::NotSampled &&
                   !candidate.actor.identity.present && candidate.weight == 0.0F &&
                   candidate.distance == 0.0F && candidate.angle == 0 && candidate.type == 0;
        return candidate.actor.status == Status::Present && joined(candidate.actor.identity) &&
               std::isfinite(candidate.weight) && std::isfinite(candidate.distance) &&
               candidate.distance >= 0.0F && candidate.type < Attention::AttentionTypeCount;
    };
    const auto listValid = [&](const auto& list, const std::uint8_t count) {
        for (std::size_t index = 0; index < list.size(); ++index) {
            if (!candidateValid(list[index], index < count))
                return false;
        }
        return true;
    };
    const bool channelPresent = attention.status == Status::Present;
    const bool countsValid = countOffsetValid(attention.lockCount, attention.lockOffset,
                                 Attention::MaximumLockCandidates) &&
                             countOffsetValid(attention.actionCount, attention.actionOffset,
                                 Attention::MaximumActionCandidates) &&
                             countOffsetValid(attention.checkCount, attention.checkOffset,
                                 Attention::MaximumCheckCandidates);
    const bool payloadEmpty =
        attention.playerAttentionFlags == 0 && attention.attentionStatus == 0 &&
        attention.attentionBlockTimer == 0 && attention.lockCount == 0 &&
        attention.lockOffset == 0 && attention.actionCount == 0 && attention.actionOffset == 0 &&
        attention.checkCount == 0 && attention.checkOffset == 0;
    if (!statusValid || !countsValid ||
        (channelPresent && (!listValid(attention.lockCandidates, attention.lockCount) ||
                               !listValid(attention.actionCandidates, attention.actionCount) ||
                               !listValid(attention.checkCandidates, attention.checkCount))) ||
        (!channelPresent && (!payloadEmpty || !listValid(attention.lockCandidates, 0) ||
                                !listValid(attention.actionCandidates, 0) ||
                                !listValid(attention.checkCandidates, 0))))
    {
        error = "learning observation has inconsistent attention-candidate state";
        return false;
    }

    append_integer(output, static_cast<std::uint8_t>(attention.status));
    append_integer(output, attention.attentionStatus);
    append_integer(output, attention.lockCount);
    append_integer(output, attention.lockOffset);
    append_integer(output, attention.actionCount);
    append_integer(output, attention.actionOffset);
    append_integer(output, attention.checkCount);
    append_integer(output, attention.checkOffset);
    append_integer(output, attention.playerAttentionFlags);
    append_integer(output, attention.attentionBlockTimer);
    const auto appendReference = [&](const Reference& reference) {
        append_integer(output, static_cast<std::uint8_t>(reference.status));
        append_integer<std::uint8_t>(output, 0);
        append_integer<std::uint16_t>(output, 0);
        return append_actor_identity(output, reference.identity, error);
    };
    const auto appendList = [&](const auto& list, const std::uint8_t count) {
        for (std::size_t index = 0; index < count; ++index) {
            const auto& candidate = list[index];
            if (!append_float(output, candidate.weight, error) ||
                !append_float(output, candidate.distance, error))
                return false;
            append_integer(output, candidate.angle);
            append_integer<std::uint16_t>(output, 0);
            append_integer(output, candidate.type);
            if (!appendReference(candidate.actor))
                return false;
        }
        return true;
    };
    return appendList(attention.lockCandidates, attention.lockCount) &&
           appendList(attention.actionCandidates, attention.actionCount) &&
           appendList(attention.checkCandidates, attention.checkCount);
}

bool append_event_transition_state(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, std::string& error) {
    using Status = MilestoneObservation::ChannelStatus;
    const auto& transition = observation.eventTransition;
    const bool present = transition.status == Status::Present;
    const bool unavailable = transition.status == Status::Unavailable;
    const bool currentPresent = transition.currentEventStatus == Status::Present;
    const bool currentAbsent = transition.currentEventStatus == Status::Absent;
    const bool goalPresent = transition.eventGoalStatus == Status::Present;
    const bool goalAbsent = transition.eventGoalStatus == Status::Absent;
    const bool nextPresent = transition.nextStageStatus == Status::Present;
    const bool nextAbsent = transition.nextStageStatus == Status::Absent;
    const bool currentEmpty = transition.currentEventId == -1 &&
                              transition.currentEventType == 0 &&
                              transition.currentEventRoom == -1;
    const bool goalEmpty = std::ranges::all_of(
        transition.eventGoal, [](const float value) { return value == 0.0F; });
    const bool nextEmpty = transition.nextStage[0] == '\0' && transition.nextRoom == -1 &&
                           transition.nextLayer == -1 && transition.nextPoint == -1 &&
                           transition.nextWipe == 0 && transition.nextWipeSpeed == 0;
    const bool nextMatchesCore =
        nextPresent && observation.nextStageEnabled && observation.nextStageName != nullptr &&
        std::strncmp(transition.nextStage.data(), observation.nextStageName,
            transition.nextStage.size()) == 0 &&
        transition.nextRoom == observation.nextRoom &&
        transition.nextLayer == observation.nextLayer &&
        transition.nextPoint == observation.nextPoint;
    if ((!present && !unavailable) ||
        (present && (!((currentPresent && goalPresent) || (currentAbsent && goalAbsent)) ||
                        !((nextPresent && nextMatchesCore) ||
                            (nextAbsent && !observation.nextStageEnabled && nextEmpty)))) ||
        (!currentPresent && !currentEmpty) || (!goalPresent && !goalEmpty) ||
        (goalPresent && !std::ranges::all_of(transition.eventGoal,
                            [](const float value) { return std::isfinite(value); })) ||
        (!present && (transition.eventDataLoaded || transition.cameraPlay != 0 ||
                         transition.currentEventStatus != Status::NotSampled ||
                         transition.eventGoalStatus != Status::NotSampled ||
                         transition.nextStageStatus != Status::NotSampled || !currentEmpty ||
                         !goalEmpty || !nextEmpty)))
    {
        error = "learning observation has inconsistent event-transition state";
        return false;
    }

    append_integer(output, static_cast<std::uint8_t>(transition.status));
    append_integer(output, static_cast<std::uint8_t>(transition.eventDataLoaded));
    append_integer(output, static_cast<std::uint8_t>(transition.currentEventStatus));
    append_integer(output, static_cast<std::uint8_t>(transition.eventGoalStatus));
    append_integer(output, static_cast<std::uint8_t>(transition.nextStageStatus));
    append_integer(output, transition.nextWipe);
    append_integer(output, transition.nextWipeSpeed);
    append_integer<std::uint8_t>(output, 0);
    append_integer(output, transition.cameraPlay);
    append_integer(output, transition.currentEventId);
    append_integer<std::uint16_t>(output, 0);
    append_integer(output, transition.currentEventType);
    append_integer(output, transition.currentEventRoom);
    if (!append_float_array(output, transition.eventGoal, error))
        return false;
    output.insert(output.end(), transition.nextStage.begin(), transition.nextStage.end());
    append_integer(output, transition.nextRoom);
    append_integer(output, transition.nextLayer);
    append_integer(output, transition.nextPoint);
    return true;
}

bool append_clock_domain_state(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, std::string& error) {
    using Status = MilestoneObservation::ChannelStatus;
    const auto& clocks = observation.clockDomains;
    const bool present = clocks.status == Status::Present;
    const bool unavailable = clocks.status == Status::Unavailable;
    const bool demoPresent = clocks.demoStatus == Status::Present;
    const bool demoAbsent = clocks.demoStatus == Status::Absent;
    const bool demoEmpty = clocks.demoMode == 0 && clocks.demoFrame == 0 &&
                           clocks.demoFrameNoMessage == 0 && clocks.demoFlags == 0;
    const bool timerPresent = clocks.timerStatus == Status::Present;
    const bool timerAbsent = clocks.timerStatus == Status::Absent;
    const bool timerUnavailable = clocks.timerStatus == Status::Unavailable;
    const bool timerEmpty = clocks.timerMode == -1 && clocks.timerNowMs == 0 &&
                            clocks.timerLimitMs == 0;
    const bool outerEmpty = clocks.frameworkFrames == 0 && clocks.gameplayFrames == 0 &&
                            !clocks.globalPause && !clocks.scenePaused &&
                            clocks.scenePauseTimer == 0 && clocks.sceneNextPauseTimer == 0 &&
                            !clocks.overlapRequestActive && !clocks.overlapFadeoutPeek &&
                            clocks.demoStatus == Status::NotSampled && demoEmpty &&
                            clocks.timerStatus == Status::NotSampled && timerEmpty;
    if ((!present && !unavailable) ||
        (present && (!(demoPresent || demoAbsent) ||
                        !(timerPresent || timerAbsent || timerUnavailable))) ||
        (!demoPresent && !demoEmpty) ||
        (demoPresent && (clocks.demoMode == 0 ||
                            clocks.demoFrameNoMessage > clocks.demoFrame)) ||
        (timerPresent && clocks.timerMode < 0) || (!timerPresent && !timerEmpty) ||
        (!present && !outerEmpty))
    {
        error = "learning observation has inconsistent clock-domain state";
        return false;
    }

    std::uint8_t flags = 0;
    flags |= clocks.globalPause ? 1u << 0 : 0;
    flags |= clocks.scenePaused ? 1u << 1 : 0;
    flags |= clocks.overlapRequestActive ? 1u << 2 : 0;
    flags |= clocks.overlapFadeoutPeek ? 1u << 3 : 0;
    append_integer(output, static_cast<std::uint8_t>(clocks.status));
    append_integer(output, flags);
    append_integer(output, clocks.scenePauseTimer);
    append_integer(output, clocks.sceneNextPauseTimer);
    append_integer(output, static_cast<std::uint8_t>(clocks.demoStatus));
    append_integer(output, static_cast<std::uint8_t>(clocks.timerStatus));
    append_integer<std::uint16_t>(output, 0);
    append_integer(output, clocks.frameworkFrames);
    append_integer(output, clocks.gameplayFrames);
    append_integer(output, clocks.demoMode);
    append_integer(output, clocks.demoFrame);
    append_integer(output, clocks.demoFrameNoMessage);
    append_integer(output, clocks.demoFlags);
    append_integer(output, clocks.timerMode);
    append_integer(output, clocks.timerNowMs);
    append_integer(output, clocks.timerLimitMs);
    return true;
}

bool append_room_load_state(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, std::string& error) {
    using Status = MilestoneObservation::ChannelStatus;
    using Room = MilestoneObservation::RoomLoadState::Room;
    const auto& load = observation.roomLoad;
    const bool present = load.status == Status::Present;
    const bool unavailable = load.status == Status::Unavailable;
    const auto validRoom = [](const std::int8_t room) { return room >= -1 && room < 64; };
    const auto emptyRoom = [](const Room& room) {
        const Room empty{};
        return room.statusFlags == empty.statusFlags && room.draw == empty.draw &&
               room.zoneCount == empty.zoneCount && room.zone == empty.zone &&
               room.memoryBlock == empty.memoryBlock && room.region == empty.region &&
               room.sceneStatus == empty.sceneStatus && room.scenePhase == empty.scenePhase &&
               room.scenePhaseActive == empty.scenePhaseActive;
    };
    const bool outerEmpty = load.roomRead == -1 && load.stayRoom == -1 &&
                            load.oldStayRoom == -1 && load.nextStayRoom == -1 &&
                            !load.noChangeRoom && !load.timePass &&
                            std::ranges::all_of(load.rooms, emptyRoom);
    if ((!present && !unavailable) ||
        (present && (!validRoom(load.roomRead) || !validRoom(load.stayRoom) ||
                        !validRoom(load.oldStayRoom) || !validRoom(load.nextStayRoom))) ||
        (!present && !outerEmpty))
    {
        error = "learning observation has inconsistent room-load state";
        return false;
    }
    if (present) {
        for (const auto& room : load.rooms) {
            const bool scenePresent = room.sceneStatus == Status::Present;
            const bool sceneAbsent = room.sceneStatus == Status::Absent;
            const bool sceneUnavailable = room.sceneStatus == Status::Unavailable;
            if (!(scenePresent || sceneAbsent || sceneUnavailable) || room.zone < -1 ||
                room.memoryBlock < -1 ||
                room.memoryBlock >= MilestoneObservation::RoomLoadState::MemoryBlockCount ||
                (scenePresent && (room.statusFlags == 0 || room.scenePhase < 0 ||
                                     room.scenePhase > 4)) ||
                (!scenePresent && (room.scenePhase != 0 || room.scenePhaseActive)))
            {
                error = "learning observation has inconsistent room-load entry";
                return false;
            }
        }
    }

    std::uint8_t flags = 0;
    flags |= load.noChangeRoom ? 1u << 0 : 0;
    flags |= load.timePass ? 1u << 1 : 0;
    append_integer(output, static_cast<std::uint8_t>(load.status));
    append_integer(output, flags);
    append_integer(output, load.roomRead);
    append_integer(output, load.stayRoom);
    append_integer(output, load.oldStayRoom);
    append_integer(output, load.nextStayRoom);
    append_integer<std::uint16_t>(output, 0);
    for (const auto& room : load.rooms) {
        std::uint8_t roomFlags = 0;
        roomFlags |= room.draw ? 1u << 0 : 0;
        roomFlags |= room.scenePhaseActive ? 1u << 1 : 0;
        append_integer(output, room.statusFlags);
        append_integer(output, roomFlags);
        append_integer(output, room.zoneCount);
        append_integer(output, room.zone);
        append_integer(output, room.memoryBlock);
        append_integer(output, room.region);
        append_integer(output, static_cast<std::uint8_t>(room.sceneStatus));
        append_integer<std::uint8_t>(output, 0);
        append_integer(output, room.scenePhase);
    }
    return true;
}

std::array<std::uint8_t, 16> xxh128(const std::span<const std::uint8_t> value) {
    const XXH128_hash_t hash = XXH3_128bits(value.data(), value.size());
    XXH128_canonical_t canonical{};
    XXH128_canonicalFromHash(&canonical, hash);
    std::array<std::uint8_t, 16> output{};
    std::ranges::copy(canonical.digest, output.begin());
    return output;
}

}  // namespace

bool append_learning_observation(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, const LearningObservationContext& context,
    std::string& error) {
    error.clear();
    const bool validTerminalReason =
        context.terminalReason == LearningTerminalReason::None ||
        context.terminalReason == LearningTerminalReason::GoalReached ||
        context.terminalReason == LearningTerminalReason::TickBudgetExhausted;
    const bool terminalPhaseIsValid = context.terminalReason == LearningTerminalReason::None ||
                                      context.phase == LearningObservationPhase::PostSimulation;
    const bool terminalOutcomeIsValid =
        (context.terminalReason != LearningTerminalReason::GoalReached || context.goal.reached) &&
        (context.terminalReason != LearningTerminalReason::TickBudgetExhausted ||
            (!context.goal.reached && context.remainingTicks == 0));
    const GameplayTracePhase expectedTracePhase =
        context.phase == LearningObservationPhase::PreInput ? GameplayTracePhase::PreInput :
                                                              GameplayTracePhase::PostSimulation;
    const bool traceBoundaryIsValid =
        context.gameplayTrace == nullptr ||
        (context.gameplayTrace->core.phase == expectedTracePhase &&
            context.gameplayTrace->core.boundaryIndex == context.boundaryIndex &&
            context.gameplayTrace->core.simulationTick == context.simulationTick &&
            context.gameplayTrace->core.tapeFrame == context.tapeFrame);
    if ((context.phase != LearningObservationPhase::PreInput &&
            context.phase != LearningObservationPhase::PostSimulation) ||
        !validTerminalReason || !terminalPhaseIsValid || !terminalOutcomeIsValid ||
        !traceBoundaryIsValid ||
        observation.actors.size() > std::numeric_limits<std::uint16_t>::max() ||
        observation.dynamicColliders.size() > std::numeric_limits<std::uint16_t>::max() ||
        observation.actorObservedCount != observation.actors.size() ||
        observation.actorsTruncated || observation.dynamicCollidersTruncated ||
        (!observation.dynamicCollidersPresent && !observation.dynamicColliders.empty()) ||
        observation.playerResourcesPresent != observation.playerPresent ||
        observation.playerRelationshipsPresent != observation.playerIsLink ||
        observation.playerCollisionSolverPresent != observation.playerIsLink ||
        (observation.flagsPresent &&
            (observation.eventFlags.size() != kMilestoneEventFlagCount ||
                observation.temporaryFlags.size() != kMilestoneTemporaryFlagCount ||
                observation.temporaryEventBytes.size() != kMilestoneTemporaryEventByteCount ||
                observation.dungeonFlags.size() != kMilestoneDungeonFlagCount ||
                observation.switchFlags.size() != kMilestoneSwitchFlagCount)))
    {
        error = "learning observation has incomplete or inconsistent channels";
        return false;
    }
    if (std::ranges::adjacent_find(observation.actors,
            [](const MilestoneObservation::Actor& left, const MilestoneObservation::Actor& right) {
                return left.runtimeGeneration >= right.runtimeGeneration;
            }) != observation.actors.end())
    {
        error = "learning observation actor set is not strictly ordered";
        return false;
    }
    for (const MilestoneObservation::Actor& actor : observation.actors) {
        if (actor.returnPlaceWriterPresent) {
            const auto& writer = actor.returnPlaceWriter;
            const bool guardsSatisfied = writer.noTelopClear && writer.eventSetSatisfied &&
                                         writer.eventUnsetSatisfied && writer.switchSetSatisfied &&
                                         writer.switchUnsetSatisfied;
            if (actor.actorName != fpcNm_KYTAG14_e ||
                (writer.requiredEventSet == 0xffff && !writer.eventSetSatisfied) ||
                (writer.requiredEventUnset == 0xffff && !writer.eventUnsetSatisfied) ||
                (writer.requiredSwitchSet == 0xff && !writer.switchSetSatisfied) ||
                (writer.requiredSwitchUnset == 0xff && !writer.switchUnsetSatisfied) ||
                writer.eligible != guardsSatisfied)
            {
                error = "learning observation has inconsistent return-place writer";
                return false;
            }
        }
        if (actor.enemyBasePresent != (actor.group == EnemyActorGroup)) {
            error = "learning observation has inconsistent enemy-base component";
            return false;
        }
        if (actor.triggerVolumePresent) {
            const auto& trigger = actor.triggerVolume;
            using Kind = MilestoneObservation::Actor::TriggerVolumeKind;
            using Shape = MilestoneObservation::Actor::TriggerVolumeShape;
            const bool kindMatchesActor =
                (trigger.kind == Kind::SceneExit && actor.actorName == fpcNm_SCENE_EXIT_e) ||
                (trigger.kind == Kind::SceneExitCylinder &&
                    actor.actorName == fpcNm_SCENE_EXIT2_e) ||
                (trigger.kind == Kind::EventArea && actor.actorName == fpcNm_TAG_EVTAREA_e) ||
                (trigger.kind == Kind::ScriptedEvent && actor.actorName == fpcNm_TAG_EVT_e) ||
                (trigger.kind == Kind::MappedEvent && actor.actorName == fpcNm_TAG_EVENT_e);
            const bool shapeValid =
                trigger.shape == Shape::Box || trigger.shape == Shape::EllipticCylinder;
            if (!kindMatchesActor || !shapeValid || trigger.halfExtentX < 0.0f ||
                trigger.halfExtentY < 0.0f || trigger.halfExtentZ < 0.0f ||
                (trigger.verticalUnbounded && trigger.shape != Shape::EllipticCylinder))
            {
                error = "learning observation has inconsistent trigger-volume component";
                return false;
            }
        }
    }
    const auto& relationships = observation.playerRelationships;
    const std::array<const MilestoneObservation::ActorIdentity*, 11> relationshipActors{
        &relationships.targetedActor,
        &relationships.rideActor,
        &relationships.heldItemActor,
        &relationships.grabbedActor,
        &relationships.thrownBoomerangActor,
        &relationships.copyRodActor,
        &relationships.hookshotRoofWaitActor,
        &relationships.chainGrabActor,
        &relationships.attentionHintActor,
        &relationships.attentionCatchActor,
        &relationships.attentionLookActor,
    };
    for (const MilestoneObservation::ActorIdentity* identity : relationshipActors) {
        const bool joined =
            !identity->present ||
            std::ranges::binary_search(observation.actors, identity->runtimeGeneration, {},
                &MilestoneObservation::Actor::runtimeGeneration);
        if (identity->present != identity->homePositionPresent ||
            (!observation.playerRelationshipsPresent && identity->present) || !joined)
        {
            error = "learning observation player relationship is inconsistent with actor set";
            return false;
        }
    }
    constexpr std::uint32_t knownSolverFlags = 0x00f1fffeu;
    if ((observation.playerCollisionSolver.flags & ~knownSolverFlags) != 0) {
        error = "learning observation player collision solver has unknown flags";
        return false;
    }
    for (const auto& wall : observation.playerCollisionSolver.wallCircles) {
        if ((wall.flags & ~0x6u) != 0) {
            error = "learning observation player collision solver wall has unknown flags";
            return false;
        }
    }
    for (std::size_t index = 0; index < observation.dynamicColliders.size(); ++index) {
        if (observation.dynamicColliders[index].registrationIndex != index) {
            error = "learning observation collider set is not canonically ordered";
            return false;
        }
    }
    std::array<std::uint8_t, 16> stateIdentity{};
    if (!decode_hex_128(context.stateIdentity, stateIdentity)) {
        error = "learning observation state identity is not XXH3-128 hex";
        return false;
    }

    std::uint32_t flags = 0;
    flags |= observation.playerPresent ? 1u << 0 : 0;
    flags |= observation.playerIsLink ? 1u << 1 : 0;
    flags |= observation.nextStageEnabled ? 1u << 2 : 0;
    flags |= context.cameraPresent ? 1u << 3 : 0;
    flags |= context.collisionCorrectionPresent ? 1u << 4 : 0;
    flags |= observation.flagsPresent ? 1u << 6 : 0;
    flags |= context.goal.configured ? 1u << 7 : 0;
    flags |= context.goal.reached ? 1u << 8 : 0;
    flags |= observation.playerGroundHeightPresent ? 1u << 9 : 0;
    flags |= observation.playerRoofHeightPresent ? 1u << 10 : 0;
    flags |= observation.eventNameHashPresent ? 1u << 11 : 0;

    append_integer(output, static_cast<std::uint8_t>(context.phase));
    append_integer(output, static_cast<std::uint8_t>(LearningActorSelectionRule::Complete));
    append_integer(output, static_cast<std::uint8_t>(context.terminalReason));
    append_integer<std::uint8_t>(output, 0);
    append_integer(output, static_cast<std::uint16_t>(observation.actors.size()));
    append_integer(output, flags);
    append_integer(output, observation.actorObservedCount);
    append_integer(output, context.remainingTicks);
    append_integer(output, context.boundaryIndex);
    append_integer(output, context.simulationTick);
    append_integer(output, context.tapeFrame);
    output.insert(output.end(), stateIdentity.begin(), stateIdentity.end());

    append_fixed_name(output, observation.stageName);
    append_integer(output, observation.room);
    append_integer(output, observation.layer);
    append_integer(output, observation.point);
    append_fixed_name(output, observation.nextStageName);
    append_integer(output, observation.nextRoom);
    append_integer(output, observation.nextLayer);
    append_integer(output, observation.nextPoint);

    append_integer(output, observation.playerProcessId);
    append_integer(output, observation.playerActorName);
    append_integer(output, observation.playerProcId);
    for (const float value : {observation.playerPositionX, observation.playerPositionY,
             observation.playerPositionZ, observation.playerVelocityX, observation.playerVelocityY,
             observation.playerVelocityZ, observation.playerForwardSpeed})
    {
        if (!append_float(output, observation.playerPresent ? value : 0.0F, error))
            return false;
    }
    for (const std::int16_t value :
        {observation.playerCurrentAngleX, observation.playerCurrentAngleY,
            observation.playerCurrentAngleZ, observation.playerShapeAngleX,
            observation.playerShapeAngleY, observation.playerShapeAngleZ})
        append_integer(output, observation.playerPresent ? value : std::int16_t{0});
    append_integer(output, observation.playerModeFlags);
    append_integer(output, observation.playerDamageWaitTimer);
    append_integer(output, observation.playerIceDamageWaitTimer);
    append_integer(output, observation.playerSwordChangeWaitTimer);
    append_integer(output, observation.playerDoStatus);
    std::uint8_t contacts = 0;
    contacts |= observation.playerGroundContact ? 1u << 0 : 0;
    contacts |= observation.playerWallContact ? 1u << 1 : 0;
    contacts |= observation.playerRoofContact ? 1u << 2 : 0;
    contacts |= observation.playerWaterContact ? 1u << 3 : 0;
    contacts |= observation.playerWaterIn ? 1u << 4 : 0;
    append_integer(output, contacts);
    append_integer<std::uint8_t>(output, 0);
    if (!append_float(output,
            observation.playerGroundHeightPresent ? observation.playerGroundHeight : 0.0F, error) ||
        !append_float(output,
            observation.playerRoofHeightPresent ? observation.playerRoofHeight : 0.0F, error))
        return false;

    append_integer(output, static_cast<std::uint8_t>(observation.eventRunning));
    append_integer(output, observation.eventId);
    append_integer(output, observation.eventMode);
    append_integer(output, observation.eventStatus);
    append_integer(output, observation.eventMapToolId);
    append_integer(output, observation.eventNameHashPresent ? observation.eventNameHash : 0u);

    std::uint16_t menuFlags = 0;
    menuFlags |= observation.titlePresent ? 1u << 0 : 0;
    menuFlags |= observation.titleLogoSkipReady ? 1u << 1 : 0;
    menuFlags |= observation.titleStartReady ? 1u << 2 : 0;
    menuFlags |= observation.nameEntryActive ? 1u << 3 : 0;
    menuFlags |= observation.nameEntryCharacterSelectReady ? 1u << 4 : 0;
    menuFlags |= observation.nameEntryInputReady ? 1u << 5 : 0;
    menuFlags |= observation.fileSelectNoSaveReady ? 1u << 6 : 0;
    menuFlags |= observation.fileSelectDataSelectReady ? 1u << 7 : 0;
    menuFlags |= observation.fileSelectKeyWaitReady ? 1u << 8 : 0;
    menuFlags |= observation.fileSelectYesNoReady ? 1u << 9 : 0;
    menuFlags |= observation.nameScenePresent ? 1u << 10 : 0;
    menuFlags |= observation.fileSelectPresent ? 1u << 11 : 0;
    append_integer(output, menuFlags);
    append_integer(output, observation.titleProcedure);
    append_integer(output, observation.nameEntrySelectionProcedure);
    append_integer(output, observation.nameSceneProcedure);
    append_integer(output, observation.fileSelectProcedure);
    append_integer(output, observation.fileSelectCardCheckProcedure);
    append_integer<std::uint8_t>(output, 0);

    if (!append_float(output, context.cameraPresent ? context.cameraYawRadians : 0.0F, error) ||
        !append_float(output,
            context.collisionCorrectionPresent ? context.collisionCorrectionX : 0.0F, error) ||
        !append_float(output,
            context.collisionCorrectionPresent ? context.collisionCorrectionZ : 0.0F, error))
        return false;
    if (!append_mechanics_channels(
            output, context.gameplayTrace, context.collisionPlanes, context.playerForm, error))
        return false;
    append_raw_pad(output, context.previousInput);

    if (observation.rng.version != kGameRngSnapshotVersion ||
        observation.rng.streamCount != kGameRngStreamCount)
    {
        error = "learning observation has unsupported RNG snapshot identity";
        return false;
    }
    for (std::size_t index = 0; index < observation.rng.streams.size(); ++index) {
        const auto& stream = observation.rng.streams[index];
        if (stream.id != static_cast<GameRngStreamId>(index) ||
            std::ranges::any_of(
                stream.reserved, [](const std::uint8_t value) { return value != 0; }) ||
            stream.algorithmVersion != kGameRngAlgorithmVersion)
        {
            error = "learning observation has unsupported RNG stream identity";
            return false;
        }
    }
    append_integer(output, observation.rng.version);
    append_integer(output, observation.rng.streamCount);
    for (const GameRngStreamSnapshot& stream : observation.rng.streams) {
        append_integer(output, static_cast<std::uint8_t>(stream.id));
        output.insert(output.end(), stream.reserved.begin(), stream.reserved.end());
        append_integer(output, stream.algorithmVersion);
        append_integer(output, stream.state0);
        append_integer(output, stream.state1);
        append_integer(output, stream.state2);
        append_integer(output, stream.callCount);
    }
    if (!append_actor_identity(output, observation.talkPartner, error) ||
        !append_actor_identity(output, observation.grabbedActor, error))
        return false;

    append_integer(output, context.goal.requestedCount);
    append_integer(output, context.goal.hitCount);
    append_integer(output, context.goal.stableTicks);
    append_integer(output, context.goal.consecutiveTicks);
    append_integer(output, context.goal.sequenceSteps);
    append_integer(output, context.goal.sequenceNextStep);
    append_integer(output, context.goal.sequenceWithinTicks);
    append_integer(output, context.goal.sequenceElapsedTicks);
    append_integer(
        output, context.goal.firstHitTick.value_or(std::numeric_limits<std::uint64_t>::max()));

    for (const MilestoneObservation::Actor& actor : observation.actors) {
        append_integer(output, actor.runtimeGeneration);
        append_integer(output, actor.parentRuntimeGeneration);
        append_integer(output, actor.parameters);
        append_integer(output, actor.status);
        append_integer(output, actor.actorName);
        append_integer(output, actor.profileName);
        append_integer(output, actor.setId);
        append_integer(output, actor.homeRoom);
        append_integer(output, actor.currentRoom);
        append_integer(output, actor.group);
        append_integer(output, actor.argument);
        append_integer(output, actor.health);
        if (!append_float(output, actor.positionX, error) ||
            !append_float(output, actor.positionY, error) ||
            !append_float(output, actor.positionZ, error) ||
            !append_float(output, actor.homePositionX, error) ||
            !append_float(output, actor.homePositionY, error) ||
            !append_float(output, actor.homePositionZ, error) ||
            !append_float(output, actor.velocityX, error) ||
            !append_float(output, actor.velocityY, error) ||
            !append_float(output, actor.velocityZ, error) ||
            !append_float(output, actor.forwardSpeed, error))
            return false;
        append_integer(output, actor.currentAngleX);
        append_integer(output, actor.currentAngleY);
        append_integer(output, actor.currentAngleZ);
        append_integer(output, actor.shapeAngleX);
        append_integer(output, actor.shapeAngleY);
        append_integer(output, actor.shapeAngleZ);
        std::uint16_t componentMask = 0;
        componentMask |= actor.attentionPresent ? 1u << 0 : 0;
        componentMask |= actor.eventParticipationPresent ? 1u << 1 : 0;
        componentMask |= actor.returnPlaceWriterPresent ? 1u << 2 : 0;
        componentMask |= actor.enemyBasePresent ? 1u << 3 : 0;
        componentMask |= actor.triggerVolumePresent ? 1u << 4 : 0;
        append_integer(output, componentMask);
        append_integer<std::uint16_t>(output, 0);

        const auto& attention = actor.attention;
        append_integer(output, actor.attentionPresent ? attention.flags : 0u);
        if (!append_float(output, actor.attentionPresent ? attention.positionX : 0.0F, error) ||
            !append_float(output, actor.attentionPresent ? attention.positionY : 0.0F, error) ||
            !append_float(output, actor.attentionPresent ? attention.positionZ : 0.0F, error))
            return false;
        if (actor.attentionPresent) {
            output.insert(
                output.end(), attention.distanceIndices.begin(), attention.distanceIndices.end());
        } else {
            output.insert(output.end(), attention.distanceIndices.size(), std::uint8_t{0});
        }
        append_integer(output, actor.attentionPresent ? attention.auxiliary : std::int16_t{0});
        append_integer<std::uint8_t>(output, 0);

        const auto& participation = actor.eventParticipation;
        append_integer(
            output, actor.eventParticipationPresent ? participation.command : std::uint16_t{0});
        append_integer(
            output, actor.eventParticipationPresent ? participation.condition : std::uint16_t{0});
        append_integer(
            output, actor.eventParticipationPresent ? participation.eventId : std::int16_t{0});
        append_integer(
            output, actor.eventParticipationPresent ? participation.mapToolId : std::uint8_t{0});
        append_integer(
            output, actor.eventParticipationPresent ? participation.index : std::uint8_t{0});

        const auto& writer = actor.returnPlaceWriter;
        std::uint8_t writerGuardMask = 0;
        if (actor.returnPlaceWriterPresent) {
            writerGuardMask |= writer.noTelopClear ? 1u << 0 : 0;
            writerGuardMask |= writer.eventSetSatisfied ? 1u << 1 : 0;
            writerGuardMask |= writer.eventUnsetSatisfied ? 1u << 2 : 0;
            writerGuardMask |= writer.switchSetSatisfied ? 1u << 3 : 0;
            writerGuardMask |= writer.switchUnsetSatisfied ? 1u << 4 : 0;
            writerGuardMask |= writer.eligible ? 1u << 5 : 0;
        }
        append_integer(output, actor.returnPlaceWriterPresent ? writer.saveRoom : std::int8_t{0});
        append_integer(output, actor.returnPlaceWriterPresent ? writer.savePoint : std::uint8_t{0});
        append_integer(output, actor.returnPlaceWriterPresent ? writer.switchRoom : std::int8_t{0});
        append_integer(output, writerGuardMask);
        append_integer(
            output, actor.returnPlaceWriterPresent ? writer.requiredEventSet : std::uint16_t{0});
        append_integer(
            output, actor.returnPlaceWriterPresent ? writer.requiredEventUnset : std::uint16_t{0});
        append_integer(
            output, actor.returnPlaceWriterPresent ? writer.requiredSwitchSet : std::uint8_t{0});
        append_integer(
            output, actor.returnPlaceWriterPresent ? writer.requiredSwitchUnset : std::uint8_t{0});
        append_integer<std::uint16_t>(output, 0);

        const auto& enemy = actor.enemyBase;
        append_integer(output, actor.enemyBasePresent ? enemy.flags : std::uint16_t{0});
        append_integer(output, actor.enemyBasePresent ? enemy.throwMode : std::uint8_t{0});
        append_integer<std::uint8_t>(output, 0);
        for (const float value : {enemy.downPositionX, enemy.downPositionY, enemy.downPositionZ,
                 enemy.headLockPositionX, enemy.headLockPositionY, enemy.headLockPositionZ})
        {
            if (!append_float(output, actor.enemyBasePresent ? value : 0.0F, error))
                return false;
        }

        const auto& trigger = actor.triggerVolume;
        std::uint8_t triggerFlags = 0;
        if (actor.triggerVolumePresent) {
            triggerFlags |= trigger.enabled ? 1u << 0 : 0;
            triggerFlags |= trigger.verticalUnbounded ? 1u << 1 : 0;
        }
        append_integer(output,
            actor.triggerVolumePresent ? static_cast<std::uint8_t>(trigger.kind) : std::uint8_t{0});
        append_integer(output, actor.triggerVolumePresent ?
                                   static_cast<std::uint8_t>(trigger.shape) :
                                   std::uint8_t{0});
        append_integer(output, triggerFlags);
        append_integer<std::uint8_t>(output, 0);
        append_integer(output, actor.triggerVolumePresent ? trigger.behavior : std::uint16_t{0});
        append_integer(output, actor.triggerVolumePresent ? trigger.yaw : std::int16_t{0});
        for (const float value : {trigger.centerX, trigger.centerY, trigger.centerZ,
                 trigger.halfExtentX, trigger.halfExtentY, trigger.halfExtentZ})
        {
            if (!append_float(output, actor.triggerVolumePresent ? value : 0.0F, error))
                return false;
        }

        std::uint8_t backingMask = 0;
        backingMask |= actor.heapPresent ? 1u << 0 : 0;
        backingMask |= actor.modelPresent ? 1u << 1 : 0;
        backingMask |= actor.jointCollisionPresent ? 1u << 2 : 0;
        append_integer(output, backingMask);
        append_integer<std::uint8_t>(output, 0);
        append_integer(output, actor.actorType);
        append_integer(output, actor.processSubtype);
        append_integer(output, actor.condition);
        append_integer(output, actor.pauseFlag);
        append_integer(output, actor.processInitState);
        append_integer(output, actor.processCreatePhase);
        append_integer(output, actor.cullType);
        append_integer(output, actor.demoActorId);
        append_integer(output, actor.carryType);
        append_integer(output, actor.oldRoom);
        append_integer<std::uint8_t>(output, 0);
        for (const float value : {actor.oldPositionX, actor.oldPositionY, actor.oldPositionZ,
                 actor.scaleX, actor.scaleY, actor.scaleZ, actor.gravity, actor.maxFallSpeed,
                 actor.eyePositionX, actor.eyePositionY, actor.eyePositionZ})
        {
            if (!append_float(output, value, error))
                return false;
        }
        for (const std::int16_t value : {actor.homeAngleX, actor.homeAngleY, actor.homeAngleZ,
                 actor.oldAngleX, actor.oldAngleY, actor.oldAngleZ})
            append_integer(output, value);
    }

    append_integer(output, static_cast<std::uint8_t>(observation.dynamicCollidersPresent ? 1 : 3));
    append_integer<std::uint8_t>(output, 0);
    append_integer(output, static_cast<std::uint16_t>(observation.dynamicColliders.size()));
    for (const MilestoneObservation::DynamicCollider& collider : observation.dynamicColliders) {
        std::uint16_t colliderFlags = 0;
        colliderFlags |= collider.ownerPresent ? 1u << 0 : 0;
        colliderFlags |= collider.statusPresent ? 1u << 1 : 0;
        colliderFlags |= collider.shapePresent ? 1u << 2 : 0;
        colliderFlags |= collider.attackSet ? 1u << 3 : 0;
        colliderFlags |= collider.targetSet ? 1u << 4 : 0;
        colliderFlags |= collider.correctionSet ? 1u << 5 : 0;
        colliderFlags |= collider.attackHit ? 1u << 6 : 0;
        colliderFlags |= collider.targetHit ? 1u << 7 : 0;
        colliderFlags |= collider.correctionHit ? 1u << 8 : 0;
        colliderFlags |= collider.attackHitOwnerPresent ? 1u << 9 : 0;
        colliderFlags |= collider.targetHitOwnerPresent ? 1u << 10 : 0;
        colliderFlags |= collider.correctionHitOwnerPresent ? 1u << 11 : 0;
        append_integer(output, collider.registrationIndex);
        append_integer(output, colliderFlags);
        append_integer(output, collider.ownerRuntimeGeneration);
        append_integer(output, collider.attackHitOwnerRuntimeGeneration);
        append_integer(output, collider.targetHitOwnerRuntimeGeneration);
        append_integer(output, collider.correctionHitOwnerRuntimeGeneration);
        for (const std::uint32_t value : {collider.attackType, collider.targetType,
                 collider.attackSourceParameters, collider.attackResultParameters,
                 collider.targetSourceParameters, collider.targetResultParameters,
                 collider.correctionSourceParameters, collider.correctionResultParameters})
            append_integer(output, value);
        append_integer(output, collider.attackPower);
        append_integer(output, collider.weight);
        append_integer(output, collider.damage);
        append_integer(output, static_cast<std::uint8_t>(collider.shape));
        for (const float value : {collider.centerX, collider.centerY, collider.centerZ,
                 collider.radius, collider.height, collider.aabbMinX, collider.aabbMinY,
                 collider.aabbMinZ, collider.aabbMaxX, collider.aabbMaxY, collider.aabbMaxZ,
                 collider.correctionX, collider.correctionY, collider.correctionZ})
        {
            if (!append_float(output, value, error))
                return false;
        }
    }

    if (observation.flagsPresent) {
        output.insert(output.end(), observation.eventFlags.begin(), observation.eventFlags.end());
        output.insert(
            output.end(), observation.temporaryFlags.begin(), observation.temporaryFlags.end());
        output.insert(output.end(), observation.temporaryEventBytes.begin(),
            observation.temporaryEventBytes.end());
        output.insert(
            output.end(), observation.dungeonFlags.begin(), observation.dungeonFlags.end());
        output.insert(output.end(), observation.switchFlags.begin(), observation.switchFlags.end());
    }
    append_integer(output, observation.switchFlagRoom);

    const MilestoneObservation::PlayerResources emptyResources{};
    const auto& resources =
        observation.playerResourcesPresent ? observation.playerResources : emptyResources;
    append_integer(output, static_cast<std::uint8_t>(observation.playerResourcesPresent ? 1 : 3));
    append_integer<std::uint8_t>(output, 0);
    for (const std::uint16_t value : {resources.maximumLife, resources.life, resources.rupees,
             resources.rupeeCapacity, resources.maximumOil, resources.oil})
        append_integer(output, value);
    append_integer(output, resources.maximumMagic);
    append_integer(output, resources.magic);
    append_integer(output, resources.wallet);
    append_integer(output, resources.transformStatus);
    if (!append_float(output, resources.worldTime, error))
        return false;
    append_integer(output, resources.date);
    append_integer(output, resources.arrows);
    append_integer(output, resources.arrowCapacity);
    append_integer(output, resources.pachinko);
    append_integer(output, resources.poeSouls);
    append_integer(output, resources.smallKeys);
    const std::uint8_t dungeonItems =
        (resources.dungeonMap ? 1u << 0 : 0) | (resources.dungeonCompass ? 1u << 1 : 0) |
        (resources.dungeonBossKey ? 1u << 2 : 0) | (resources.dungeonWarp ? 1u << 3 : 0);
    append_integer(output, dungeonItems);
    append_integer<std::uint16_t>(output, 0);
    output.insert(output.end(), resources.inventory.begin(), resources.inventory.end());
    output.insert(output.end(), resources.selectedItems.begin(), resources.selectedItems.end());
    output.insert(output.end(), resources.mixedItems.begin(), resources.mixedItems.end());
    output.insert(output.end(), resources.equipment.begin(), resources.equipment.end());
    output.insert(output.end(), resources.bombCounts.begin(), resources.bombCounts.end());
    output.insert(output.end(), resources.bombCapacities.begin(), resources.bombCapacities.end());
    output.insert(
        output.end(), resources.bottleQuantities.begin(), resources.bottleQuantities.end());
    output.insert(
        output.end(), resources.acquiredItemBits.begin(), resources.acquiredItemBits.end());
    output.insert(output.end(), resources.collectItemBits.begin(), resources.collectItemBits.end());
    append_integer(output, resources.collectedCrystalBits);
    append_integer(output, resources.collectedMirrorBits);

    const MilestoneObservation::PlayerRelationships emptyRelationships{};
    const auto& retainedRelationships = observation.playerRelationshipsPresent ?
                                            observation.playerRelationships :
                                            emptyRelationships;
    const std::uint8_t relationshipStatus = observation.playerRelationshipsPresent ? 1 :
                                            observation.playerPresent              ? 3 :
                                                                                     2;
    append_integer(output, relationshipStatus);
    append_integer(output, static_cast<std::uint8_t>(relationshipActors.size()));
    append_integer<std::uint16_t>(output, 0);
    for (const MilestoneObservation::ActorIdentity* identity :
        std::array<const MilestoneObservation::ActorIdentity*, 11>{
            &retainedRelationships.targetedActor,
            &retainedRelationships.rideActor,
            &retainedRelationships.heldItemActor,
            &retainedRelationships.grabbedActor,
            &retainedRelationships.thrownBoomerangActor,
            &retainedRelationships.copyRodActor,
            &retainedRelationships.hookshotRoofWaitActor,
            &retainedRelationships.chainGrabActor,
            &retainedRelationships.attentionHintActor,
            &retainedRelationships.attentionCatchActor,
            &retainedRelationships.attentionLookActor,
        })
    {
        if (!append_actor_identity(output, *identity, error))
            return false;
    }

    const MilestoneObservation::PlayerCollisionSolver emptySolver{};
    const auto& solver =
        observation.playerCollisionSolverPresent ? observation.playerCollisionSolver : emptySolver;
    const std::uint8_t solverStatus = observation.playerCollisionSolverPresent ? 1 :
                                      observation.playerPresent                ? 3 :
                                                                                 2;
    append_integer(output, solverStatus);
    append_integer(output, static_cast<std::uint8_t>(solver.wallCircles.size()));
    append_integer<std::uint16_t>(output, 0);
    append_integer(output, solver.flags);
    append_integer(output, solver.wallTableSize);
    append_integer(output, solver.waterMode);
    append_integer<std::uint8_t>(output, 0);
    append_integer<std::uint16_t>(output, 0);
    if (!append_float_array(output, solver.lineStart, error) ||
        !append_float_array(output, solver.lineEnd, error) ||
        !append_float_array(output, solver.wallCylinderCenter, error) ||
        !append_float(output, solver.wallCylinderRadius, error) ||
        !append_float(output, solver.wallCylinderHeight, error) ||
        !append_float(output, solver.groundCheckOffset, error) ||
        !append_float(output, solver.roofCorrectionHeight, error) ||
        !append_float(output, solver.waterCheckOffset, error))
        return false;
    for (const auto& wall : solver.wallCircles) {
        append_integer(output, wall.flags);
        append_integer(output, wall.angleY);
        append_integer<std::uint16_t>(output, 0);
        if (!append_float(output, wall.wallRadiusSquared, error) ||
            !append_float(output, wall.wallHeight, error) ||
            !append_float(output, wall.wallRadius, error) ||
            !append_float(output, wall.directWallHeight, error) ||
            !append_float_array(output, wall.realizedCenter, error) ||
            !append_float(output, wall.realizedRadius, error))
            return false;
    }
    if (!append_planner_runtime_state(output, observation, error) ||
        !append_message_session_state(output, observation, error) ||
        !append_event_queue_state(output, observation, error) ||
        !append_process_lifecycle_state(output, observation, error))
        return false;
    if (!append_attention_candidate_state(output, observation, error))
        return false;
    if (!append_process_lifecycle_records(output, observation, error))
        return false;
    if (!append_event_transition_state(output, observation, error))
        return false;
    if (!append_clock_domain_state(output, observation, error))
        return false;
    return append_room_load_state(output, observation, error);
}

void append_learning_action(std::vector<std::uint8_t>& output, const RawPadState& chosenPad,
    const RawPadState& consumedPad) {
    append_raw_pad(output, chosenPad);
    append_raw_pad(output, consumedPad);
}

void begin_learning_episode(std::vector<std::uint8_t>& output) {
    output.clear();
    constexpr std::array<std::uint8_t, 8> Magic{'D', 'U', 'S', 'K', 'E', 'P', 0, 0};
    output.insert(output.end(), Magic.begin(), Magic.end());
    append_integer(output, LearningObservationVersion);
    append_integer(output, static_cast<std::uint16_t>(EpisodePayloadHeaderSize));
    append_integer<std::uint32_t>(output, 0);
    append_integer<std::uint32_t>(output, 0);
    append_integer<std::uint32_t>(output, 0);
}

bool finish_learning_episode(
    std::vector<std::uint8_t>& output, const std::uint32_t stepCount, std::string& error) {
    if (output.size() < EpisodePayloadHeaderSize || stepCount == 0) {
        error = "learning episode is empty or truncated";
        return false;
    }
    patch_u32(output, 12, stepCount);
    return true;
}

LearningEpisodeShardWriter::~LearningEpisodeShardWriter() {
    abandon();
}

bool LearningEpisodeShardWriter::begin(const std::filesystem::path& path,
    const LearningEpisodeShardMetadata& metadata, std::string& error) {
    error.clear();
    if (active() || path.empty() || metadata.sourceBoundaryFingerprint.empty() ||
        metadata.sourceBoundaryFingerprint.size() != 32 ||
        metadata.checkpointIdentity.size() != 32 || metadata.objective.empty() ||
        metadata.objectiveIdentity.size() != 32 || metadata.buildRevision.empty() ||
        metadata.featureDigest.empty() || metadata.fidelityProfile.empty() ||
        !is_lower_hex(metadata.gameDataSha256, 64) ||
        !metadata.cardFixtureIdentity.starts_with("card-fixture:") ||
        !metadata.actorProfileCatalogIdentity.starts_with("actor-profile-catalog:") ||
        !is_lower_hex(metadata.worldContextSha256, 64) || metadata.maximumTicks == 0 ||
        metadata.maximumTicks > LearningEpisodeMaximumTicks)
    {
        error = "learning episode shard metadata is incomplete";
        return false;
    }
    std::error_code filesystemError;
    if (const auto parent = path.parent_path(); !parent.empty()) {
        std::filesystem::create_directories(parent, filesystemError);
        if (filesystemError) {
            error = filesystemError.message();
            return false;
        }
    }
    mPath = path;
    mTemporaryPath = path;
    mTemporaryPath += ".tmp";
    std::filesystem::remove(mTemporaryPath, filesystemError);
    if (filesystemError) {
        error = filesystemError.message();
        mPath.clear();
        mTemporaryPath.clear();
        return false;
    }

    std::vector<std::uint8_t> encodedMetadata;
    const std::array<std::string_view, 15> fields{
        LearningEpisodeShardSchema,
        LearningObservationSchema,
        LearningActionSchema,
        metadata.sourceBoundaryFingerprint,
        metadata.checkpointIdentity,
        metadata.objective,
        metadata.objectiveIdentity,
        metadata.buildRevision,
        metadata.auroraRevision,
        metadata.featureDigest,
        metadata.fidelityProfile,
        metadata.gameDataSha256,
        metadata.cardFixtureIdentity,
        metadata.actorProfileCatalogIdentity,
        metadata.worldContextSha256,
    };
    append_integer(encodedMetadata, static_cast<std::uint16_t>(fields.size()));
    for (const auto field : fields) {
        if (!append_string16(encodedMetadata, field, error)) {
            abandon();
            return false;
        }
    }

    mStream.open(mTemporaryPath, std::ios::binary | std::ios::trunc);
    if (!mStream) {
        error = "could not open learning episode shard temporary output";
        abandon();
        return false;
    }
    std::array<std::uint8_t, ShardHeaderSize> emptyHeader{};
    mStream.write(reinterpret_cast<const char*>(emptyHeader.data()), emptyHeader.size());
    mStream.write(reinterpret_cast<const char*>(encodedMetadata.data()), encodedMetadata.size());
    if (!mStream) {
        error = "could not write learning episode shard metadata";
        abandon();
        return false;
    }
    mMetadataBytes = encodedMetadata.size();
    mEpisodeCount = 0;
    mCompressedBytes = 0;
    mUncompressedBytes = 0;
    mMaximumTicks = static_cast<std::uint32_t>(metadata.maximumTicks);

    // Header fields that do not change are written now; completion and sizes
    // are patched only after every episode block is durable.
    mStream.seekp(0);
    constexpr std::array<char, 8> Magic{'D', 'U', 'S', 'K', 'E', 'P', 'S', 0};
    mStream.write(Magic.data(), Magic.size());
    write_integer(mStream, LearningEpisodeShardVersion);
    write_integer(mStream, static_cast<std::uint16_t>(ShardHeaderSize));
    write_integer<std::uint32_t>(mStream, 0);
    write_integer<std::uint32_t>(mStream, 0);
    write_integer(mStream, LearningObservationVersion);
    write_integer(mStream, LearningActionVersion);
    write_integer(mStream, static_cast<std::uint64_t>(metadata.sourceFrame));
    write_integer(mStream, static_cast<std::uint32_t>(metadata.maximumTicks));
    write_integer<std::uint32_t>(mStream, 0);
    write_integer<std::uint64_t>(mStream, ShardHeaderSize);
    write_integer(mStream, mMetadataBytes);
    write_integer(mStream, ShardHeaderSize + mMetadataBytes);
    write_integer<std::uint64_t>(mStream, 0);
    write_integer<std::uint64_t>(mStream, 0);
    write_integer<std::uint64_t>(mStream, 0);
    std::array<std::uint8_t, ShardHeaderSize - 88> reserved{};
    mStream.write(reinterpret_cast<const char*>(reserved.data()), reserved.size());
    mStream.seekp(static_cast<std::streamoff>(ShardHeaderSize + mMetadataBytes));
    return static_cast<bool>(mStream);
}

bool LearningEpisodeShardWriter::append(const LearningEpisodeDescriptor& descriptor,
    const std::span<const std::uint8_t> uncompressedEpisode, std::string& error) {
    error.clear();
    const bool outcomeIsConsistent =
        descriptor.success == descriptor.firstHitTick.has_value() &&
        (!descriptor.firstHitTick.has_value() ||
            (descriptor.ticksExecuted != 0 &&
                *descriptor.firstHitTick == descriptor.ticksExecuted - 1));
    if (!active() || descriptor.id.empty() ||
        descriptor.id.size() > std::numeric_limits<std::uint16_t>::max() ||
        descriptor.ticksExecuted == 0 || uncompressedEpisode.empty() ||
        uncompressedEpisode.size() > std::numeric_limits<std::uint32_t>::max() ||
        descriptor.ticksExecuted > mMaximumTicks ||
        descriptor.remainingTicks != mMaximumTicks - descriptor.ticksExecuted ||
        !outcomeIsConsistent)
    {
        error = "learning episode block is invalid";
        return false;
    }
    const std::size_t bound = ZSTD_compressBound(uncompressedEpisode.size());
    if (ZSTD_isError(bound)) {
        error = "could not bound learning episode compression";
        return false;
    }
    std::vector<std::uint8_t> compressed(bound);
    const std::size_t compressedSize = ZSTD_compress(compressed.data(), compressed.size(),
        uncompressedEpisode.data(), uncompressedEpisode.size(), ZSTD_CLEVEL_DEFAULT);
    if (ZSTD_isError(compressedSize) || compressedSize > std::numeric_limits<std::uint32_t>::max())
    {
        error = std::string("could not compress learning episode: ") +
                ZSTD_getErrorName(compressedSize);
        return false;
    }
    compressed.resize(compressedSize);
    const auto digest = xxh128(uncompressedEpisode);

    constexpr std::array<char, 4> Magic{'E', 'P', 'I', 'S'};
    mStream.write(Magic.data(), Magic.size());
    write_integer(mStream, static_cast<std::uint16_t>(EpisodeBlockHeaderSize));
    write_integer(mStream, static_cast<std::uint16_t>(descriptor.success ? EpisodeSuccess : 0));
    write_integer(mStream, descriptor.ticksExecuted);
    write_integer(
        mStream, descriptor.firstHitTick.value_or(std::numeric_limits<std::uint32_t>::max()));
    write_integer(mStream, descriptor.remainingTicks);
    write_integer(mStream, static_cast<std::uint16_t>(descriptor.id.size()));
    write_integer<std::uint16_t>(mStream, 0);
    write_integer(mStream, static_cast<std::uint64_t>(uncompressedEpisode.size()));
    write_integer(mStream, static_cast<std::uint64_t>(compressed.size()));
    mStream.write(reinterpret_cast<const char*>(digest.data()), digest.size());
    write_integer<std::uint64_t>(mStream, 0);
    mStream.write(descriptor.id.data(), descriptor.id.size());
    mStream.write(reinterpret_cast<const char*>(compressed.data()), compressed.size());
    mStream.flush();
    if (!mStream) {
        error = "failed while appending learning episode block";
        return false;
    }
    ++mEpisodeCount;
    mUncompressedBytes += uncompressedEpisode.size();
    mCompressedBytes += compressed.size();
    return true;
}

bool LearningEpisodeShardWriter::finish(std::string& error) {
    error.clear();
    if (!active() || mEpisodeCount == 0) {
        error = "learning episode shard has no episodes";
        return false;
    }
    const std::uint64_t payloadOffset = ShardHeaderSize + mMetadataBytes;
    const std::uint64_t payloadLength = static_cast<std::uint64_t>(mStream.tellp()) - payloadOffset;
    mStream.seekp(12);
    write_integer(mStream, static_cast<std::uint32_t>(ShardComplete));
    write_integer(mStream, mEpisodeCount);
    mStream.seekp(64);
    write_integer(mStream, payloadLength);
    write_integer(mStream, mUncompressedBytes);
    write_integer(mStream, mCompressedBytes);
    mStream.flush();
    mStream.close();
    if (!mStream) {
        error = "failed while finalizing learning episode shard";
        abandon();
        return false;
    }
    std::error_code filesystemError;
    std::filesystem::remove(mPath, filesystemError);
    if (filesystemError) {
        error = filesystemError.message();
        abandon();
        return false;
    }
    std::filesystem::rename(mTemporaryPath, mPath, filesystemError);
    if (filesystemError) {
        error = filesystemError.message();
        abandon();
        return false;
    }
    mTemporaryPath.clear();
    return true;
}

void LearningEpisodeShardWriter::abandon() {
    if (mStream.is_open())
        mStream.close();
    if (!mTemporaryPath.empty()) {
        std::error_code ignored;
        std::filesystem::remove(mTemporaryPath, ignored);
    }
    mPath.clear();
    mTemporaryPath.clear();
    mMaximumTicks = 0;
}

}  // namespace dusk::automation
