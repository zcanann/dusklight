#include "dusk/automation/learning_episode.hpp"

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
        observation.actorObservedCount != observation.actors.size() ||
        observation.actorsTruncated ||
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
    return true;
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
        !is_lower_hex(metadata.worldContextSha256, 64) ||
        metadata.maximumTicks == 0 || metadata.maximumTicks > LearningEpisodeMaximumTicks)
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
