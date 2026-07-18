#include "dusk/automation/gameplay_trace.hpp"

#include <algorithm>
#include <array>
#include <bit>
#include <cmath>
#include <fstream>
#include <limits>
#include <system_error>
#include <type_traits>
#include <utility>

namespace dusk::automation {
namespace {

constexpr std::uint32_t kFileComplete = 1u << 0;
constexpr std::uint32_t kFileCapacityExhausted = 1u << 1;
constexpr std::uint32_t kFileTriggerRetention = 1u << 2;
constexpr std::uint32_t kChannelRequired = 1u << 0;
constexpr std::uint32_t kChannelDense = 1u << 1;

struct ChannelDefinition {
    GameplayTraceChannel id;
    std::uint16_t version;
    std::uint32_t payloadStride;
    bool required;
};

constexpr std::array kChannels{
    ChannelDefinition{GameplayTraceChannel::Core, 1, 32, true},
    ChannelDefinition{GameplayTraceChannel::Stage, 1, 32, false},
    ChannelDefinition{GameplayTraceChannel::AppliedPads, 1, 52, false},
    ChannelDefinition{GameplayTraceChannel::PlayerMotion, 1, 52, false},
    ChannelDefinition{GameplayTraceChannel::Event, 1, 16, false},
    ChannelDefinition{GameplayTraceChannel::SceneExit, 2, 88, false},
    ChannelDefinition{GameplayTraceChannel::Rng, 1, 64, false},
    ChannelDefinition{GameplayTraceChannel::Camera, 1, 48, false},
    ChannelDefinition{GameplayTraceChannel::PlayerAction, 1, 104, false},
    ChannelDefinition{GameplayTraceChannel::PlayerBackgroundCollision, 1, 128, false},
    ChannelDefinition{GameplayTraceChannel::PlayerCollisionSurfaces, 1, 496, false},
    ChannelDefinition{GameplayTraceChannel::GoalProgress, 1, 32, false},
    ChannelDefinition{GameplayTraceChannel::SelectedActors, 1, 656, false},
};

struct ChannelLayout {
    ChannelDefinition definition{};
    std::uint64_t statusOffset = 0;
    std::uint64_t statusLength = 0;
    std::uint64_t payloadOffset = 0;
    std::uint64_t payloadLength = 0;
};

template <typename T>
void write_integer(std::ostream& stream, T value) {
    using U = std::make_unsigned_t<T>;
    U bits = static_cast<U>(value);
    for (std::size_t byte = 0; byte < sizeof(T); ++byte) {
        stream.put(static_cast<char>((bits >> (byte * 8)) & 0xffu));
    }
}

void write_float(std::ostream& stream, const float value) {
    static_assert(sizeof(float) == sizeof(std::uint32_t));
    write_integer(stream, std::bit_cast<std::uint32_t>(value));
}

void write_vec3(std::ostream& stream, const std::array<float, 3>& value) {
    for (const float component : value)
        write_float(stream, component);
}

void write_core(std::ostream& stream, const GameplayTraceCoreSample& sample) {
    write_integer(stream, sample.boundaryIndex);
    write_integer(stream, sample.simulationTick);
    write_integer(stream, sample.tapeFrame);
    write_integer(stream, sample.flags);
    write_integer(stream, static_cast<std::uint8_t>(sample.phase));
    write_integer(stream, static_cast<std::uint8_t>(sample.boundaryKind));
    write_integer(stream, sample.inputSource);
    write_integer<std::uint8_t>(stream, 0);
}

void write_stage(std::ostream& stream, const GameplayTraceStageSample& sample) {
    stream.write(sample.stageName.data(), sample.stageName.size());
    write_integer(stream, sample.room);
    write_integer(stream, sample.layer);
    write_integer(stream, sample.point);
    stream.write(sample.nextStageName.data(), sample.nextStageName.size());
    write_integer(stream, sample.nextRoom);
    write_integer(stream, sample.nextLayer);
    write_integer(stream, sample.nextPoint);
    write_integer(stream, sample.flags);
    write_integer<std::uint32_t>(stream, 0);
}

void write_applied_pads(std::ostream& stream, const GameplayTraceAppliedPadsSample& sample) {
    write_integer(stream, sample.validPorts);
    write_integer(stream, sample.ownedPorts);
    write_integer<std::uint16_t>(stream, 0);
    std::array<std::uint8_t, kRawPadStateSize> encoded{};
    for (const RawPadState& pad : sample.pads) {
        encode_raw_pad_state(pad, encoded);
        stream.write(reinterpret_cast<const char*>(encoded.data()), encoded.size());
    }
}

void write_player_motion(std::ostream& stream, const GameplayTracePlayerMotionSample& sample) {
    write_integer(stream, sample.sessionProcessId);
    write_integer(stream, sample.actorName);
    write_integer(stream, sample.procedureId);
    for (const std::int16_t angle : sample.currentAngle)
        write_integer(stream, angle);
    for (const std::int16_t angle : sample.shapeAngle)
        write_integer(stream, angle);
    write_vec3(stream, sample.position);
    write_vec3(stream, sample.velocity);
    write_float(stream, sample.forwardSpeed);
    write_integer(stream, sample.flags);
}

void write_event(std::ostream& stream, const GameplayTraceEventSample& sample) {
    write_integer(stream, sample.flags);
    write_integer(stream, sample.eventId);
    write_integer(stream, sample.mode);
    write_integer(stream, sample.status);
    write_integer(stream, sample.mapToolId);
    write_integer<std::uint8_t>(stream, 0);
    write_integer<std::uint16_t>(stream, 0);
    write_integer(stream, sample.nameHash);
}

void write_scene_exit(std::ostream& stream, const GameplayTraceSceneExitSample& sample) {
    write_integer(stream, sample.sessionProcessId);
    write_integer(stream, sample.rawParameters);
    write_integer(stream, sample.flags);
    write_float(stream, sample.signedDistanceToVolume);
    write_integer(stream, sample.actorName);
    write_integer(stream, sample.setId);
    write_integer(stream, sample.exitId);
    write_integer(stream, sample.pathId);
    write_integer(stream, sample.argument1);
    write_integer(stream, sample.switchNo);
    write_integer(stream, sample.kind);
    write_integer(stream, sample.observedCount);
    write_integer(stream, sample.homeRoom);
    write_integer(stream, sample.linkExitDirection);
    write_integer(stream, sample.linkExitId);
    write_integer(stream, sample.shapeYaw);
    write_integer(stream, sample.actorAction);
    write_integer<std::uint8_t>(stream, 0);
    write_integer<std::uint16_t>(stream, 0);
    write_vec3(stream, sample.playerLocalPosition);
    write_vec3(stream, sample.volumeExtent);
    write_vec3(stream, sample.homePosition);
    stream.write(sample.destinationStage.data(), sample.destinationStage.size());
    write_integer(stream, sample.destinationRoom);
    write_integer(stream, sample.destinationLayer);
    write_integer(stream, sample.destinationPoint);
    write_integer(stream, sample.destinationWipe);
    write_integer(stream, sample.destinationWipeTime);
    write_integer(stream, sample.destinationTimeHour);
    write_integer<std::uint8_t>(stream, 0);
}

void write_collision_wall(std::ostream& stream, const GameplayTraceCollisionWallSample& sample) {
    write_integer(stream, sample.bgIndex);
    write_integer(stream, sample.polyIndex);
    write_integer(stream, sample.ownerSessionProcessId);
    write_integer(stream, sample.angleY);
    write_integer(stream, sample.flags);
}

void write_player_background_collision(
    std::ostream& stream, const GameplayTracePlayerBackgroundCollisionSample& sample) {
    write_integer(stream, sample.flags);
    write_float(stream, sample.groundHeight);
    write_float(stream, sample.roofHeight);
    write_float(stream, sample.waterHeight);
    write_integer(stream, sample.groundBgIndex);
    write_integer(stream, sample.groundPolyIndex);
    write_integer(stream, sample.groundOwnerSessionProcessId);
    for (const float component : sample.groundPlane)
        write_float(stream, component);
    write_integer(stream, sample.roofBgIndex);
    write_integer(stream, sample.roofPolyIndex);
    write_integer(stream, sample.roofOwnerSessionProcessId);
    write_integer(stream, sample.waterBgIndex);
    write_integer(stream, sample.waterPolyIndex);
    write_integer(stream, sample.waterOwnerSessionProcessId);
    for (const GameplayTraceCollisionWallSample& wall : sample.walls)
        write_collision_wall(stream, wall);
    write_vec3(stream, sample.oldPosition);
    write_vec3(stream, sample.resolvedFrameDisplacement);
    write_vec3(stream, sample.finalPosition);
}

void write_collision_surface(
    std::ostream& stream, const GameplayTraceCollisionSurfaceSample& sample) {
    write_integer(stream, sample.flags);
    write_integer(stream, sample.kind);
    write_integer(stream, sample.wallSlot);
    write_integer(stream, sample.backingFormat);
    write_integer(stream, sample.rawCodePresenceMask);
    write_integer(stream, sample.bgIndex);
    write_integer(stream, sample.polyIndex);
    write_integer(stream, sample.ownerSessionProcessId);
    write_integer(stream, sample.materialIndex);
    write_integer(stream, sample.groupIndex);
    for (const std::uint32_t code : sample.rawCodes)
        write_integer(stream, code);
    write_integer(stream, sample.rawExitId);
    write_integer(stream, sample.sourceRoom);
    write_integer(stream, sample.sclsSourceRoom);
    write_integer(stream, sample.destinationRoom);
    write_integer(stream, sample.destinationLayer);
    write_integer(stream, sample.destinationWipe);
    write_integer(stream, sample.destinationWipeTime);
    write_integer(stream, sample.destinationTimeHour);
    write_integer(stream, sample.destinationPoint);
    write_integer(stream, sample.sourceGeometryIndexCount);
    write_integer<std::uint8_t>(stream, 0);
    for (const std::uint16_t index : sample.sourceGeometryIndices)
        write_integer(stream, index);
    write_float(stream, sample.kclPrismHeight);
    stream.write(sample.destinationStage.data(), sample.destinationStage.size());
    write_integer<std::uint32_t>(stream, 0);
}

void write_player_collision_surfaces(
    std::ostream& stream, const GameplayTracePlayerCollisionSurfacesSample& sample) {
    write_integer(stream, sample.flags);
    write_integer(stream, sample.currentRoom);
    write_integer(stream, sample.identityCount);
    write_integer(stream, sample.backingCodeCount);
    write_integer(stream, sample.destinationCount);
    write_integer(stream, sample.rawLinkExit);
    write_integer(stream, sample.pendingStageMatchMask);
    for (std::size_t index = 0; index < 5; ++index)
        write_integer<std::uint8_t>(stream, 0);
    for (const GameplayTraceCollisionSurfaceSample& surface : sample.surfaces)
        write_collision_surface(stream, surface);
}

void write_rng(std::ostream& stream, const GameRngSnapshot& sample) {
    write_integer(stream, sample.version);
    write_integer(stream, sample.streamCount);
    for (const GameRngStreamSnapshot& rng : sample.streams) {
        write_integer(stream, static_cast<std::uint8_t>(rng.id));
        for (const std::uint8_t reserved : rng.reserved)
            write_integer(stream, reserved);
        write_integer(stream, rng.algorithmVersion);
        write_integer(stream, rng.state0);
        write_integer(stream, rng.state1);
        write_integer(stream, rng.state2);
        write_integer(stream, rng.callCount);
    }
}

void write_camera(std::ostream& stream, const GameplayTraceCameraSample& sample) {
    write_integer(stream, sample.viewYaw);
    write_integer(stream, sample.controlledYaw);
    write_integer(stream, sample.bank);
    write_integer<std::uint16_t>(stream, 0);
    write_vec3(stream, sample.eye);
    write_vec3(stream, sample.center);
    write_vec3(stream, sample.up);
    write_float(stream, sample.fovy);
}

void write_animation_lane(std::ostream& stream, const GameplayTraceAnimationLane& sample) {
    write_integer(stream, sample.resourceId);
    write_integer<std::uint16_t>(stream, 0);
    write_float(stream, sample.frame);
    write_float(stream, sample.rate);
}

void write_player_action(std::ostream& stream, const GameplayTracePlayerActionSample& sample) {
    write_integer(stream, sample.procedureId);
    write_integer<std::uint16_t>(stream, 0);
    write_integer(stream, sample.modeFlags);
    for (const std::int16_t value : sample.procedureContextRaw)
        write_integer(stream, value);
    write_integer(stream, sample.damageWaitTimer);
    write_integer(stream, sample.swordAtUpTime);
    write_integer(stream, sample.iceDamageWaitTimer);
    write_integer(stream, sample.swordChangeWaitTimer);
    for (std::size_t index = 0; index < 5; ++index)
        write_integer<std::uint8_t>(stream, 0);
    for (const GameplayTraceAnimationLane& lane : sample.underAnimations) {
        write_animation_lane(stream, lane);
    }
    for (const GameplayTraceAnimationLane& lane : sample.upperAnimations) {
        write_animation_lane(stream, lane);
    }
}

void write_goal_progress(std::ostream& stream, const GameplayTraceGoalProgressSample& sample) {
    write_integer(stream, sample.flags);
    write_integer(stream, sample.goalNameHash);
    write_integer(stream, sample.requestedCount);
    write_integer(stream, sample.hitCount);
    write_integer(stream, sample.stableTicks);
    write_integer(stream, sample.consecutiveTicks);
    write_integer(stream, sample.sequenceSteps);
    write_integer(stream, sample.sequenceNextStep);
    write_integer(stream, sample.sequenceWithinTicks);
    write_integer(stream, sample.sequenceElapsedTicks);
    write_integer<std::uint16_t>(stream, 0);
    write_integer(stream, sample.firstHitTick);
}

void write_selected_actor(std::ostream& stream, const GameplayTraceSelectedActorSample& sample) {
    write_integer(stream, sample.sessionProcessId);
    write_integer(stream, sample.actorName);
    write_integer(stream, sample.setId);
    write_integer(stream, sample.homeRoom);
    write_integer(stream, sample.currentRoom);
    write_integer(stream, sample.health);
    write_integer(stream, sample.status);
    write_vec3(stream, sample.position);
    for (const std::int16_t angle : sample.currentAngle)
        write_integer(stream, angle);
    for (const std::int16_t angle : sample.shapeAngle)
        write_integer(stream, angle);
}

void write_selected_actors(
    std::ostream& stream, const GameplayTraceSelectedActorsSample& sample) {
    write_integer(stream, sample.count);
    write_integer(stream, sample.capacity);
    write_integer(stream, sample.flags);
    write_integer(stream, sample.observedCount);
    write_integer<std::uint32_t>(stream, 0);
    for (const GameplayTraceSelectedActorSample& actor : sample.actors)
        write_selected_actor(stream, actor);
}

GameplayTraceChannelStatus status_for(
    const GameplayTraceSample& sample, const GameplayTraceChannel channel) {
    switch (channel) {
    case GameplayTraceChannel::Core:
        return GameplayTraceChannelStatus::Present;
    case GameplayTraceChannel::Stage:
        return sample.stageStatus;
    case GameplayTraceChannel::AppliedPads:
        return sample.appliedPadsStatus;
    case GameplayTraceChannel::PlayerMotion:
        return sample.playerMotionStatus;
    case GameplayTraceChannel::Event:
        return sample.eventStatus;
    case GameplayTraceChannel::SceneExit:
        return sample.sceneExitStatus;
    case GameplayTraceChannel::Rng:
        return sample.rngStatus;
    case GameplayTraceChannel::Camera:
        return sample.cameraStatus;
    case GameplayTraceChannel::PlayerAction:
        return sample.playerActionStatus;
    case GameplayTraceChannel::PlayerBackgroundCollision:
        return sample.playerBackgroundCollisionStatus;
    case GameplayTraceChannel::PlayerCollisionSurfaces:
        return sample.playerCollisionSurfacesStatus;
    case GameplayTraceChannel::GoalProgress:
        return sample.goalProgressStatus;
    case GameplayTraceChannel::SelectedActors:
        return sample.selectedActorsStatus;
    }
    return GameplayTraceChannelStatus::Unavailable;
}

void write_payload(
    std::ostream& stream, const GameplayTraceSample& sample, const GameplayTraceChannel channel) {
    switch (channel) {
    case GameplayTraceChannel::Core:
        write_core(stream, sample.core);
        break;
    case GameplayTraceChannel::Stage:
        write_stage(stream, sample.stage);
        break;
    case GameplayTraceChannel::AppliedPads:
        write_applied_pads(stream, sample.appliedPads);
        break;
    case GameplayTraceChannel::PlayerMotion:
        write_player_motion(stream, sample.playerMotion);
        break;
    case GameplayTraceChannel::Event:
        write_event(stream, sample.event);
        break;
    case GameplayTraceChannel::SceneExit:
        write_scene_exit(stream, sample.sceneExit);
        break;
    case GameplayTraceChannel::Rng:
        write_rng(stream, sample.rng);
        break;
    case GameplayTraceChannel::Camera:
        write_camera(stream, sample.camera);
        break;
    case GameplayTraceChannel::PlayerAction:
        write_player_action(stream, sample.playerAction);
        break;
    case GameplayTraceChannel::PlayerBackgroundCollision:
        write_player_background_collision(stream, sample.playerBackgroundCollision);
        break;
    case GameplayTraceChannel::PlayerCollisionSurfaces:
        write_player_collision_surfaces(stream, sample.playerCollisionSurfaces);
        break;
    case GameplayTraceChannel::GoalProgress:
        write_goal_progress(stream, sample.goalProgress);
        break;
    case GameplayTraceChannel::SelectedActors:
        write_selected_actors(stream, sample.selectedActors);
        break;
    }
}

std::string_view trim(std::string_view value) {
    while (!value.empty() && (value.front() == ' ' || value.front() == '\t')) {
        value.remove_prefix(1);
    }
    while (!value.empty() && (value.back() == ' ' || value.back() == '\t')) {
        value.remove_suffix(1);
    }
    return value;
}

bool checked_add(const std::uint64_t left, const std::uint64_t right, std::uint64_t& output) {
    if (right > std::numeric_limits<std::uint64_t>::max() - left)
        return false;
    output = left + right;
    return true;
}

bool checked_multiply(const std::uint64_t left, const std::uint64_t right, std::uint64_t& output) {
    if (left != 0 && right > std::numeric_limits<std::uint64_t>::max() / left)
        return false;
    output = left * right;
    return true;
}

bool validate_trace(const GameplayTraceRecorder& recorder, std::string& error) {
    const TapeBoot& boot = recorder.bootOrigin();
    const bool validProcess = boot.kind == TapeBootKind::Process && boot.stage.empty() &&
                              boot.saveSlot == 0 && !boot.fixture;
    const bool validStage = boot.kind == TapeBootKind::Stage && !boot.stage.empty() &&
                            boot.stage.size() <= 16 && boot.saveSlot <= 3 &&
                            (!boot.fixture || validate_scenario_fixture(*boot.fixture) ==
                                                      ScenarioFixtureError::None);
    if (!validProcess && !validStage) {
        error = "gameplay trace has an invalid boot origin";
        return false;
    }
    const std::uint64_t requested = recorder.requestedChannels();
    if ((requested & gameplay_trace_channel_bit(GameplayTraceChannel::Core)) == 0) {
        error = "gameplay trace v2 requires the core channel";
        return false;
    }
    if ((requested & ~GameplayTraceKnownChannels) != 0) {
        error = "gameplay trace requests unknown channels";
        return false;
    }
    const auto& retention = recorder.retention();
    if (retention.preTriggerTicks > std::numeric_limits<std::uint32_t>::max() ||
        retention.postTriggerTicks > std::numeric_limits<std::uint32_t>::max() ||
        recorder.observedSampleCount() < recorder.samples().size() ||
        (retention.triggers & ~GameplayTraceKnownRetentionTriggers) != 0 ||
        (!retention.enabled() &&
            (retention.preTriggerTicks != 0 || retention.postTriggerTicks != 0 ||
                recorder.observedTriggers() != 0 || recorder.triggerCount() != 0)) ||
        (recorder.observedTriggers() & ~retention.triggers) != 0 ||
        ((recorder.triggerCount() == 0) != (recorder.observedTriggers() == 0)))
    {
        error = "gameplay trace retention metadata is inconsistent";
        return false;
    }
    if ((requested & gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces)) !=
            0 &&
        (requested & gameplay_trace_channel_bit(GameplayTraceChannel::Stage)) == 0)
    {
        error = "gameplay trace collision surfaces require the stage channel";
        return false;
    }
    for (const GameplayTraceSample& sample : recorder.samples()) {
        const auto validStatus = [](const GameplayTraceChannelStatus status) {
            return static_cast<std::uint8_t>(status) <=
                   static_cast<std::uint8_t>(GameplayTraceChannelStatus::Truncated);
        };
        if (sample.core.phase != GameplayTracePhase::PostSimulation ||
            sample.core.boundaryKind != GameplayTraceBoundaryKind::Tick ||
            (sample.core.flags & GameplayTraceSimulationTickValid) == 0 ||
            (sample.core.flags &
                ~(GameplayTraceSimulationTickValid | GameplayTraceTapeFrameValid)) != 0 ||
            sample.core.simulationTick == GameplayTraceNoTick ||
            sample.core.boundaryIndex != sample.core.simulationTick + 1)
        {
            error = "gameplay trace sample has contradictory post-simulation boundary metadata";
            return false;
        }
        const bool tapeFrameValid = (sample.core.flags & GameplayTraceTapeFrameValid) != 0;
        if (sample.core.inputSource &
                ~(GameplayTraceInputTape | GameplayTraceInputController | GameplayTraceInputLive) ||
            std::popcount(sample.core.inputSource) > 1 ||
            tapeFrameValid == (sample.core.tapeFrame == GameplayTraceNoTapeFrame))
        {
            error = "gameplay trace sample has noncanonical input or tape-frame metadata";
            return false;
        }
        for (const ChannelDefinition& definition : kChannels) {
            const std::uint64_t bit = gameplay_trace_channel_bit(definition.id);
            const GameplayTraceChannelStatus status = status_for(sample, definition.id);
            if (!validStatus(status)) {
                error = "gameplay trace sample has an invalid channel status";
                return false;
            }
            if ((requested & bit) == 0) {
                if (definition.id != GameplayTraceChannel::Core &&
                    status != GameplayTraceChannelStatus::NotSampled)
                {
                    error = "unrequested gameplay trace channel was sampled";
                    return false;
                }
            } else if (status == GameplayTraceChannelStatus::NotSampled) {
                error = "requested gameplay trace channel was not sampled";
                return false;
            }
        }
        if ((sample.stage.flags & ~GameplayTraceNextStageEnabled) != 0 ||
            (sample.playerMotion.flags & ~GameplayTracePlayerIsLink) != 0 ||
            (sample.event.flags &
                ~(GameplayTraceEventRunning | GameplayTraceEventNameHashPresent)) != 0 ||
            (sample.sceneExit.flags &
                ~(GameplayTraceSceneExitVolumeValid | GameplayTraceSceneExitPlayerInside |
                    GameplayTraceSceneExitPlayerLatched | GameplayTraceSceneExitChangeOk |
                    GameplayTraceSceneExitChangeStarted | GameplayTraceSceneExitDestinationValid |
                    GameplayTraceSceneExitObservedCountSaturated)) != 0 ||
            (sample.playerBackgroundCollision.flags &
                ~(GameplayTraceCollisionGroundProbeValid | GameplayTraceCollisionGroundContact |
                    GameplayTraceCollisionLanding | GameplayTraceCollisionAway |
                    GameplayTraceCollisionGroundPlaneValid |
                    GameplayTraceCollisionGroundOwnerPresent | GameplayTraceCollisionWallContact |
                    GameplayTraceCollisionRoofProbeValid | GameplayTraceCollisionRoofContact |
                    GameplayTraceCollisionRoofOwnerPresent |
                    GameplayTraceCollisionWaterProbeEnabled |
                    GameplayTraceCollisionWaterSurfaceFound | GameplayTraceCollisionWaterIn |
                    GameplayTraceCollisionWaterOwnerPresent |
                    GameplayTraceCollisionWallProbeEnabled | GameplayTraceCollisionTrajectoryValid |
                    GameplayTraceCollisionGroundIdentityPresent |
                    GameplayTraceCollisionRoofIdentityPresent |
                    GameplayTraceCollisionWaterIdentityPresent)) != 0 ||
            (sample.playerCollisionSurfaces.flags &
                ~(GameplayTraceCollisionSurfaceCurrentRoomValid |
                    GameplayTraceCollisionSurfaceExplicitLinkExitPresent |
                    GameplayTraceCollisionSurfaceNextStagePending)) != 0 ||
            (sample.goalProgress.flags &
                ~(GameplayTraceGoalConfigured | GameplayTraceGoalReached |
                    GameplayTraceGoalAuthored | GameplayTraceGoalFirstHitTickPresent)) != 0 ||
            (sample.selectedActors.flags & ~GameplayTraceSelectedActorsTruncated) != 0 ||
            (sample.appliedPads.validPorts & ~0x0fu) != 0 ||
            (sample.appliedPads.ownedPorts & ~0x0fu) != 0)
        {
            error = "gameplay trace sample has unknown channel flags";
            return false;
        }
        if (sample.sceneExitStatus == GameplayTraceChannelStatus::Truncated ||
            sample.playerBackgroundCollisionStatus == GameplayTraceChannelStatus::Truncated ||
            sample.playerCollisionSurfacesStatus == GameplayTraceChannelStatus::Truncated ||
            sample.goalProgressStatus == GameplayTraceChannelStatus::Truncated ||
            sample.selectedActorsStatus == GameplayTraceChannelStatus::Truncated)
        {
            error = "gameplay trace scalar query channel cannot be truncated";
            return false;
        }
        for (const GameplayTraceCollisionWallSample& wall : sample.playerBackgroundCollision.walls)
        {
            if ((wall.flags &
                    ~(GameplayTraceCollisionWallHit | GameplayTraceCollisionWallOwnerPresent |
                        GameplayTraceCollisionWallIdentityPresent)) != 0)
            {
                error = "gameplay trace sample has unknown collision wall flags";
                return false;
            }
        }
        const auto finiteFloats = [](const auto& values) {
            return std::all_of(values.begin(), values.end(),
                [](const float value) { return std::isfinite(value); });
        };
        const auto zeroFloats = [](const auto& values) {
            return std::all_of(
                values.begin(), values.end(), [](const float value) { return value == 0.0f; });
        };
        const auto identityCoherent = [](const std::uint16_t bg, const std::uint16_t polygon,
                                          const std::uint32_t owner, const bool identityPresent,
                                          const bool ownerPresent) {
            const bool bgPresent = bg != 0xffff;
            const bool polygonPresent = polygon != 0xffff;
            const bool actualOwnerPresent = owner != 0xffffffffu;
            return bgPresent == polygonPresent && bgPresent == identityPresent &&
                   actualOwnerPresent == ownerPresent && (!actualOwnerPresent || identityPresent);
        };
        if (sample.sceneExitStatus == GameplayTraceChannelStatus::Present) {
            const auto& exit = sample.sceneExit;
            const bool volumeValid = (exit.flags & GameplayTraceSceneExitVolumeValid) != 0;
            const bool inside = (exit.flags & GameplayTraceSceneExitPlayerInside) != 0;
            const bool latched = (exit.flags & GameplayTraceSceneExitPlayerLatched) != 0;
            const bool changeOk = (exit.flags & GameplayTraceSceneExitChangeOk) != 0;
            const bool changeStarted = (exit.flags & GameplayTraceSceneExitChangeStarted) != 0;
            const bool destinationValid =
                (exit.flags & GameplayTraceSceneExitDestinationValid) != 0;
            const bool observedCountSaturated =
                (exit.flags & GameplayTraceSceneExitObservedCountSaturated) != 0;
            const bool box = exit.kind == GameplayTraceSceneExitBox;
            const bool radial = exit.kind == GameplayTraceSceneExitRadialXz;
            const bool signedInside = exit.signedDistanceToVolume < 0.0f ||
                                      (box && std::signbit(exit.signedDistanceToVolume));
            if (!volumeValid || (!box && !radial) || exit.observedCount == 0 ||
                (observedCountSaturated && exit.observedCount != 0xff) ||
                !std::isfinite(exit.signedDistanceToVolume) ||
                !finiteFloats(exit.playerLocalPosition) || !finiteFloats(exit.volumeExtent) ||
                !finiteFloats(exit.homePosition) || exit.volumeExtent[0] < 0.0f ||
                exit.volumeExtent[1] < 0.0f || exit.volumeExtent[2] < 0.0f ||
                inside != signedInside || (latched && !box) || (changeOk && !latched) ||
                (changeStarted && !latched) || (latched != (exit.linkExitId != 0xffff)) ||
                (!latched && exit.linkExitDirection != 0xff) ||
                exit.exitId != (exit.rawParameters & 0xff) ||
                (box && (exit.actorAction != 0xff ||
                            exit.argument1 != ((exit.rawParameters >> 8) & 0xff) ||
                            exit.pathId != ((exit.rawParameters >> 16) & 0xff) ||
                            exit.switchNo != ((exit.rawParameters >> 24) & 0xff))) ||
                (radial && (exit.actorAction > 3 || exit.pathId != 0xff || exit.argument1 != 0xff ||
                               exit.switchNo != 0xff || exit.volumeExtent[1] != 0.0f ||
                               exit.volumeExtent[0] != exit.volumeExtent[2])))
            {
                error = "gameplay trace scene-exit payload is semantically inconsistent";
                return false;
            }
            bool destinationNameValid = exit.destinationStage[0] != '\0';
            bool nameTerminated = false;
            for (const unsigned char character : exit.destinationStage) {
                if (character == 0) {
                    nameTerminated = true;
                } else if (nameTerminated || character < 0x20 || character > 0x7e) {
                    destinationNameValid = false;
                }
            }
            const bool destinationFieldsValid =
                destinationNameValid && exit.destinationRoom >= -1 && exit.destinationRoom < 64 &&
                exit.destinationLayer >= -1 && exit.destinationLayer < 15 &&
                exit.destinationPoint >= 0 && exit.destinationWipeTime <= 7 &&
                exit.destinationTimeHour >= -1 && exit.destinationTimeHour < 31;
            const bool destinationFieldsAbsent =
                std::all_of(exit.destinationStage.begin(), exit.destinationStage.end(),
                    [](const char value) { return value == '\0'; }) &&
                exit.destinationRoom == -1 && exit.destinationLayer == -1 &&
                exit.destinationPoint == -1 && exit.destinationWipe == 0xff &&
                exit.destinationWipeTime == 0xff && exit.destinationTimeHour == -1;
            if ((destinationValid && !destinationFieldsValid) ||
                (!destinationValid && !destinationFieldsAbsent))
            {
                error = "gameplay trace scene-exit destination presence is inconsistent at tick " +
                        std::to_string(sample.core.simulationTick) +
                        " (room=" + std::to_string(exit.destinationRoom) +
                        ", layer=" + std::to_string(exit.destinationLayer) +
                        ", point=" + std::to_string(exit.destinationPoint) +
                        ", wipe=" + std::to_string(exit.destinationWipe) +
                        ", wipe-time=" + std::to_string(exit.destinationWipeTime) +
                        ", hour=" + std::to_string(exit.destinationTimeHour) + ", stage-bytes=";
                for (const unsigned char character : exit.destinationStage)
                    error += std::to_string(character) + ',';
                error += ')';
                return false;
            }
        }
        if (sample.playerBackgroundCollisionStatus == GameplayTraceChannelStatus::Present) {
            const auto& collision = sample.playerBackgroundCollision;
            const auto has = [&collision](const std::uint32_t flag) {
                return (collision.flags & flag) != 0;
            };
            const bool groundValid = has(GameplayTraceCollisionGroundProbeValid);
            const bool groundOwner = has(GameplayTraceCollisionGroundOwnerPresent);
            const bool groundIdentity = has(GameplayTraceCollisionGroundIdentityPresent);
            const bool roofValid = has(GameplayTraceCollisionRoofProbeValid);
            const bool roofOwner = has(GameplayTraceCollisionRoofOwnerPresent);
            const bool roofIdentity = has(GameplayTraceCollisionRoofIdentityPresent);
            const bool waterEnabled = has(GameplayTraceCollisionWaterProbeEnabled);
            const bool waterFound = has(GameplayTraceCollisionWaterSurfaceFound);
            const bool waterOwner = has(GameplayTraceCollisionWaterOwnerPresent);
            const bool waterIdentity = has(GameplayTraceCollisionWaterIdentityPresent);
            if ((groundValid ? (!std::isfinite(collision.groundHeight) ||
                                   collision.groundHeight == -1000000000.0f) :
                               collision.groundHeight != -1000000000.0f) ||
                (!groundValid && (has(GameplayTraceCollisionGroundContact) ||
                                     has(GameplayTraceCollisionLanding))) ||
                !identityCoherent(collision.groundBgIndex, collision.groundPolyIndex,
                    collision.groundOwnerSessionProcessId, groundIdentity, groundOwner) ||
                (!groundValid && collision.groundBgIndex != 0xffff) ||
                (has(GameplayTraceCollisionGroundPlaneValid) ?
                        (!groundValid || !has(GameplayTraceCollisionGroundContact) ||
                            !finiteFloats(collision.groundPlane)) :
                        !zeroFloats(collision.groundPlane)) ||
                (roofValid ? (!std::isfinite(collision.roofHeight) ||
                                 collision.roofHeight == 1000000000.0f) :
                             collision.roofHeight != 1000000000.0f) ||
                (has(GameplayTraceCollisionRoofContact) && !roofValid) ||
                !identityCoherent(collision.roofBgIndex, collision.roofPolyIndex,
                    collision.roofOwnerSessionProcessId, roofIdentity, roofOwner) ||
                (!roofValid && collision.roofBgIndex != 0xffff) ||
                (waterFound ? (!std::isfinite(collision.waterHeight) ||
                                  collision.waterHeight == -1000000000.0f) :
                              collision.waterHeight != -1000000000.0f) ||
                (waterFound && !waterEnabled) ||
                (has(GameplayTraceCollisionWaterIn) && !waterFound) ||
                !identityCoherent(collision.waterBgIndex, collision.waterPolyIndex,
                    collision.waterOwnerSessionProcessId, waterIdentity, waterOwner) ||
                (!waterFound && collision.waterBgIndex != 0xffff))
            {
                error = "gameplay trace background-collision payload is semantically inconsistent";
                return false;
            }
            bool anyWallHit = false;
            for (const GameplayTraceCollisionWallSample& wall : collision.walls) {
                const bool hit = (wall.flags & GameplayTraceCollisionWallHit) != 0;
                const bool owner = (wall.flags & GameplayTraceCollisionWallOwnerPresent) != 0;
                const bool identity = (wall.flags & GameplayTraceCollisionWallIdentityPresent) != 0;
                anyWallHit |= hit;
                if (!identityCoherent(wall.bgIndex, wall.polyIndex, wall.ownerSessionProcessId,
                        identity, owner) ||
                    (!hit && (wall.bgIndex != 0xffff || wall.polyIndex != 0xffff ||
                                 wall.ownerSessionProcessId != 0xffffffffu || wall.angleY != 0 ||
                                 wall.flags != 0)))
                {
                    error = "gameplay trace background-collision wall presence is inconsistent";
                    return false;
                }
            }
            if (anyWallHit != has(GameplayTraceCollisionWallContact) ||
                (anyWallHit && !has(GameplayTraceCollisionWallProbeEnabled)))
            {
                error = "gameplay trace background-collision wall aggregate is inconsistent";
                return false;
            }
            const bool trajectoryValid = has(GameplayTraceCollisionTrajectoryValid);
            if (!trajectoryValid) {
                if (!zeroFloats(collision.oldPosition) ||
                    !zeroFloats(collision.resolvedFrameDisplacement) ||
                    !zeroFloats(collision.finalPosition))
                {
                    error =
                        "gameplay trace background-collision trajectory presence is inconsistent";
                    return false;
                }
            } else if (!finiteFloats(collision.oldPosition) ||
                       !finiteFloats(collision.resolvedFrameDisplacement) ||
                       !finiteFloats(collision.finalPosition))
            {
                error = "gameplay trace background-collision trajectory is nonfinite";
                return false;
            } else {
                for (std::size_t axis = 0; axis < 3; ++axis) {
                    const float reconstructed =
                        collision.oldPosition[axis] + collision.resolvedFrameDisplacement[axis];
                    const float tolerance =
                        0.0001f * std::max(1.0f, std::fabs(collision.finalPosition[axis]));
                    if (std::fabs(reconstructed - collision.finalPosition[axis]) > tolerance) {
                        error = "gameplay trace background-collision trajectory is incoherent";
                        return false;
                    }
                }
            }
        }
        if (sample.playerCollisionSurfacesStatus == GameplayTraceChannelStatus::Present) {
            const auto& set = sample.playerCollisionSurfaces;
            const auto setHas = [&set](
                                    const std::uint32_t flag) { return (set.flags & flag) != 0; };
            const bool roomValid = setHas(GameplayTraceCollisionSurfaceCurrentRoomValid);
            const bool explicitExit = setHas(GameplayTraceCollisionSurfaceExplicitLinkExitPresent);
            const bool pending = setHas(GameplayTraceCollisionSurfaceNextStagePending);
            const bool stagePending = sample.stageStatus == GameplayTraceChannelStatus::Present &&
                                      (sample.stage.flags & GameplayTraceNextStageEnabled) != 0;
            if ((roomValid ? set.currentRoom < -1 || set.currentRoom >= 64 :
                             set.currentRoom != -128) ||
                explicitExit != (set.rawLinkExit != 0x003f) || pending != stagePending ||
                (set.pendingStageMatchMask & ~0x3fu) != 0)
            {
                error = "gameplay trace collision-surface set header is inconsistent";
                return false;
            }

            std::uint8_t identityCount = 0;
            std::uint8_t backingCount = 0;
            std::uint8_t destinationCount = 0;
            std::uint8_t matchMask = 0;
            for (std::size_t slot = 0; slot < set.surfaces.size(); ++slot) {
                const GameplayTraceCollisionSurfaceSample& surface = set.surfaces[slot];
                const std::uint8_t expectedKind = slot == 0 ? GameplayTraceCollisionSurfaceGround :
                                                  slot == 1 ? GameplayTraceCollisionSurfaceRoof :
                                                  slot == 2 ? GameplayTraceCollisionSurfaceWater :
                                                              GameplayTraceCollisionSurfaceWall;
                const std::uint8_t expectedWallSlot =
                    slot < 3 ? 0 : static_cast<std::uint8_t>(slot - 3);
                constexpr std::uint32_t knownSurfaceFlags =
                    GameplayTraceCollisionSurfaceIdentityPresent |
                    GameplayTraceCollisionSurfaceOwnerPresent |
                    GameplayTraceCollisionSurfaceBackingResolved |
                    GameplayTraceCollisionSurfaceRawCodesPresent |
                    GameplayTraceCollisionSurfaceMaterialPresent |
                    GameplayTraceCollisionSurfaceGroupPresent |
                    GameplayTraceCollisionSurfaceSourceRoomPresent |
                    GameplayTraceCollisionSurfaceSourceRoomExact |
                    GameplayTraceCollisionSurfaceSclsSourcePresent |
                    GameplayTraceCollisionSurfaceDestinationPresent |
                    GameplayTraceCollisionSurfaceDestinationMatchesPending |
                    GameplayTraceCollisionSurfaceGeometryPresent |
                    GameplayTraceCollisionSurfaceKclHeightPresent;
                if (surface.kind != expectedKind || surface.wallSlot != expectedWallSlot ||
                    (surface.flags & ~knownSurfaceFlags) != 0 ||
                    surface.backingFormat > GameplayTraceCollisionBackingKcl ||
                    (surface.rawCodePresenceMask & ~0x1fu) != 0)
                {
                    error = "gameplay trace collision-surface discriminator is inconsistent";
                    return false;
                }

                const auto has = [&surface](const std::uint32_t flag) {
                    return (surface.flags & flag) != 0;
                };
                const bool identity = has(GameplayTraceCollisionSurfaceIdentityPresent);
                const bool owner = has(GameplayTraceCollisionSurfaceOwnerPresent);
                const bool backing = has(GameplayTraceCollisionSurfaceBackingResolved);
                const bool codes = has(GameplayTraceCollisionSurfaceRawCodesPresent);
                const bool material = has(GameplayTraceCollisionSurfaceMaterialPresent);
                const bool group = has(GameplayTraceCollisionSurfaceGroupPresent);
                const bool sourceRoom = has(GameplayTraceCollisionSurfaceSourceRoomPresent);
                const bool sourceRoomExact = has(GameplayTraceCollisionSurfaceSourceRoomExact);
                const bool sclsSource = has(GameplayTraceCollisionSurfaceSclsSourcePresent);
                const bool destination = has(GameplayTraceCollisionSurfaceDestinationPresent);
                const bool destinationMatch =
                    has(GameplayTraceCollisionSurfaceDestinationMatchesPending);
                const bool geometry = has(GameplayTraceCollisionSurfaceGeometryPresent);
                const bool kclHeight = has(GameplayTraceCollisionSurfaceKclHeightPresent);
                identityCount += identity ? 1 : 0;
                backingCount += backing ? 1 : 0;
                destinationCount += destination ? 1 : 0;
                if (destinationMatch)
                    matchMask |= static_cast<std::uint8_t>(1u << slot);

                const bool bgPresent = surface.bgIndex != 0xffff;
                const bool polygonPresent = surface.polyIndex != 0xffff;
                if (bgPresent != polygonPresent || identity != bgPresent ||
                    owner != (surface.ownerSessionProcessId != 0xffffffffu) ||
                    (owner && !identity) ||
                    backing != (surface.backingFormat != GameplayTraceCollisionBackingNone) ||
                    (backing && !identity) || codes != (surface.rawCodePresenceMask != 0) ||
                    (codes && (!backing || (surface.rawCodePresenceMask & 1u) == 0)) ||
                    material != (surface.materialIndex != 0xffff) || (material && !backing) ||
                    group != (surface.groupIndex != 0xffff) ||
                    (group &&
                        (!backing || surface.backingFormat != GameplayTraceCollisionBackingDzb)) ||
                    (sourceRoom && !identity) || sourceRoomExact && !sourceRoom ||
                    (sourceRoom ? (surface.sourceRoom < -1 || surface.sourceRoom >= 64) :
                                  surface.sourceRoom != -128) ||
                    (sclsSource ? (slot != 0 || !identity || !roomValid ||
                                      surface.sclsSourceRoom != set.currentRoom ||
                                      surface.sclsSourceRoom < -1 || surface.sclsSourceRoom >= 64) :
                                  surface.sclsSourceRoom != -128) ||
                    (destination && (slot != 0 || !sclsSource || !codes ||
                                        surface.rawExitId == 0x3f || surface.rawExitId == 0xff)) ||
                    (destinationMatch && (!destination || !pending)) ||
                    (geometry ? (!backing || surface.sourceGeometryIndexCount == 0 ||
                                    surface.sourceGeometryIndexCount > 6) :
                                surface.sourceGeometryIndexCount != 0) ||
                    (kclHeight &&
                        (!backing || surface.backingFormat != GameplayTraceCollisionBackingKcl ||
                            !std::isfinite(surface.kclPrismHeight))) ||
                    (!kclHeight && surface.kclPrismHeight != 0.0f))
                {
                    error = "gameplay trace collision-surface presence is inconsistent";
                    return false;
                }
                for (std::size_t word = 0; word < surface.rawCodes.size(); ++word) {
                    if ((surface.rawCodePresenceMask & (1u << word)) == 0 &&
                        surface.rawCodes[word] != 0)
                    {
                        error = "gameplay trace collision-surface raw-code mask is inconsistent";
                        return false;
                    }
                }
                if ((codes && surface.rawExitId != (surface.rawCodes[0] & 0x3fu)) ||
                    (!codes && surface.rawExitId != 0xff))
                {
                    error = "gameplay trace collision-surface exit code is inconsistent";
                    return false;
                }
                for (std::size_t index = 0; index < surface.sourceGeometryIndices.size(); ++index) {
                    const bool indexPresent = geometry && index < surface.sourceGeometryIndexCount;
                    if (indexPresent == (surface.sourceGeometryIndices[index] == 0xffff)) {
                        error =
                            "gameplay trace collision-surface geometry indices are inconsistent";
                        return false;
                    }
                }

                bool destinationNameValid = surface.destinationStage[0] != '\0';
                bool nameTerminated = false;
                for (const unsigned char character : surface.destinationStage) {
                    if (character == 0) {
                        nameTerminated = true;
                    } else if (nameTerminated || character < 0x20 || character > 0x7e) {
                        destinationNameValid = false;
                    }
                }
                const bool destinationFieldsValid =
                    destinationNameValid && surface.destinationRoom >= -1 &&
                    surface.destinationRoom < 64 && surface.destinationLayer >= -1 &&
                    surface.destinationLayer < 15 && surface.destinationPoint >= 0 &&
                    surface.destinationWipeTime <= 7 && surface.destinationTimeHour >= -1 &&
                    surface.destinationTimeHour < 31;
                const bool destinationFieldsAbsent =
                    std::all_of(surface.destinationStage.begin(), surface.destinationStage.end(),
                        [](const char value) { return value == '\0'; }) &&
                    surface.destinationRoom == -128 && surface.destinationLayer == -128 &&
                    surface.destinationPoint == -32768 && surface.destinationWipe == 0xff &&
                    surface.destinationWipeTime == 0xff && surface.destinationTimeHour == -128;
                if ((destination && !destinationFieldsValid) ||
                    (!destination && !destinationFieldsAbsent))
                {
                    error = "gameplay trace collision-surface destination is inconsistent";
                    return false;
                }
                const bool tupleMatchesPending =
                    destination && pending &&
                    surface.destinationStage == sample.stage.nextStageName &&
                    surface.destinationRoom == sample.stage.nextRoom &&
                    surface.destinationLayer == sample.stage.nextLayer &&
                    surface.destinationPoint == sample.stage.nextPoint;
                if (destinationMatch != tupleMatchesPending) {
                    error = "gameplay trace collision-surface pending-stage match is inconsistent";
                    return false;
                }
            }
            if (set.identityCount != identityCount || set.backingCodeCount != backingCount ||
                set.destinationCount != destinationCount || set.pendingStageMatchMask != matchMask)
            {
                error = "gameplay trace collision-surface aggregate counts are inconsistent";
                return false;
            }

            if (sample.playerBackgroundCollisionStatus == GameplayTraceChannelStatus::Present) {
                const auto identitiesAgree =
                    [](const GameplayTraceCollisionSurfaceSample& surface, const std::uint16_t bg,
                        const std::uint16_t polygon, const std::uint32_t owner,
                        const bool identityPresent, const bool ownerPresent) {
                        const bool surfaceIdentity =
                            (surface.flags & GameplayTraceCollisionSurfaceIdentityPresent) != 0;
                        const bool surfaceOwner =
                            (surface.flags & GameplayTraceCollisionSurfaceOwnerPresent) != 0;
                        return surfaceIdentity == identityPresent && surfaceOwner == ownerPresent &&
                               (!identityPresent ||
                                   (surface.bgIndex == bg && surface.polyIndex == polygon)) &&
                               (!ownerPresent || surface.ownerSessionProcessId == owner);
                    };
                const auto& collision = sample.playerBackgroundCollision;
                if (!identitiesAgree(set.surfaces[0], collision.groundBgIndex,
                        collision.groundPolyIndex, collision.groundOwnerSessionProcessId,
                        (collision.flags & GameplayTraceCollisionGroundIdentityPresent) != 0,
                        (collision.flags & GameplayTraceCollisionGroundOwnerPresent) != 0) ||
                    !identitiesAgree(set.surfaces[1], collision.roofBgIndex,
                        collision.roofPolyIndex, collision.roofOwnerSessionProcessId,
                        (collision.flags & GameplayTraceCollisionRoofIdentityPresent) != 0,
                        (collision.flags & GameplayTraceCollisionRoofOwnerPresent) != 0) ||
                    !identitiesAgree(set.surfaces[2], collision.waterBgIndex,
                        collision.waterPolyIndex, collision.waterOwnerSessionProcessId,
                        (collision.flags & GameplayTraceCollisionWaterIdentityPresent) != 0,
                        (collision.flags & GameplayTraceCollisionWaterOwnerPresent) != 0))
                {
                    error = "gameplay trace collision channels disagree on cached surfaces";
                    return false;
                }
                for (std::size_t wall = 0; wall < collision.walls.size(); ++wall) {
                    const auto& legacy = collision.walls[wall];
                    if (!identitiesAgree(set.surfaces[wall + 3], legacy.bgIndex, legacy.polyIndex,
                            legacy.ownerSessionProcessId,
                            (legacy.flags & GameplayTraceCollisionWallIdentityPresent) != 0,
                            (legacy.flags & GameplayTraceCollisionWallOwnerPresent) != 0))
                    {
                        error = "gameplay trace collision channels disagree on cached walls";
                        return false;
                    }
                }
            }
        }
        std::uint8_t connectedPorts = 0;
        for (std::size_t port = 0; port < sample.appliedPads.pads.size(); ++port) {
            const RawPadState& pad = sample.appliedPads.pads[port];
            if ((static_cast<std::uint8_t>(pad.flags) &
                    ~static_cast<std::uint8_t>(RawPadFlags::Connected)) != 0)
            {
                error = "gameplay trace sample has unknown pad flags";
                return false;
            }
            if (has_flag(pad.flags, RawPadFlags::Connected)) {
                connectedPorts |= static_cast<std::uint8_t>(1u << port);
            }
        }
        if (sample.appliedPadsStatus == GameplayTraceChannelStatus::Present &&
            connectedPorts != sample.appliedPads.validPorts)
        {
            error = "gameplay trace sample pad validity disagrees with pad flags";
            return false;
        }
        if (sample.rngStatus == GameplayTraceChannelStatus::Present &&
            (sample.rng.version != kGameRngSnapshotVersion ||
                sample.rng.streamCount != kGameRngStreamCount ||
                sample.rng.streams[0].id != GameRngStreamId::Primary ||
                sample.rng.streams[1].id != GameRngStreamId::Secondary))
        {
            error = "gameplay trace sample contains an incompatible RNG snapshot";
            return false;
        }
        if (sample.goalProgressStatus == GameplayTraceChannelStatus::Present) {
            const auto& goal = sample.goalProgress;
            const bool configured = (goal.flags & GameplayTraceGoalConfigured) != 0;
            const bool reached = (goal.flags & GameplayTraceGoalReached) != 0;
            const bool authored = (goal.flags & GameplayTraceGoalAuthored) != 0;
            const bool firstHit = (goal.flags & GameplayTraceGoalFirstHitTickPresent) != 0;
            if (reached != firstHit || reached && !configured || authored && !configured ||
                (firstHit == (goal.firstHitTick == GameplayTraceNoTick)) ||
                goal.hitCount > goal.requestedCount ||
                goal.consecutiveTicks > goal.stableTicks ||
                goal.sequenceNextStep > goal.sequenceSteps ||
                (!configured && (goal.goalNameHash != 0 || goal.stableTicks != 0 ||
                                    goal.consecutiveTicks != 0 || goal.sequenceSteps != 0 ||
                                    goal.sequenceNextStep != 0 || goal.sequenceWithinTicks != 0 ||
                                    goal.sequenceElapsedTicks != 0)))
            {
                error = "gameplay trace goal-progress payload is inconsistent";
                return false;
            }
        }
        if (sample.selectedActorsStatus == GameplayTraceChannelStatus::Present) {
            const auto& actors = sample.selectedActors;
            const bool truncated =
                (actors.flags & GameplayTraceSelectedActorsTruncated) != 0;
            if (actors.capacity != GameplayTraceSelectedActorCapacity ||
                actors.count > actors.capacity || actors.observedCount < actors.count ||
                truncated != (actors.observedCount > actors.count))
            {
                error = "gameplay trace selected-actor header is inconsistent";
                return false;
            }
            std::uint32_t previousProcess = 0;
            for (std::size_t index = 0; index < actors.actors.size(); ++index) {
                const auto& actor = actors.actors[index];
                const bool retained = index < actors.count;
                if (retained) {
                    if (actor.sessionProcessId == 0xffffffffu ||
                        (index != 0 && actor.sessionProcessId <= previousProcess) ||
                        !finiteFloats(actor.position))
                    {
                        error = "gameplay trace retained actor is invalid or unordered";
                        return false;
                    }
                    previousProcess = actor.sessionProcessId;
                } else if (actor.sessionProcessId != 0xffffffffu || actor.actorName != -1 ||
                           actor.setId != 0xffff || actor.homeRoom != -1 ||
                           actor.currentRoom != -1 || actor.health != 0 || actor.status != 0 ||
                           !zeroFloats(actor.position) ||
                           std::ranges::any_of(actor.currentAngle, [](const auto value) {
                               return value != 0;
                           }) ||
                           std::ranges::any_of(actor.shapeAngle, [](const auto value) {
                               return value != 0;
                           }))
                {
                    error = "gameplay trace unused selected-actor slot is noncanonical";
                    return false;
                }
            }
        }
    }
    return true;
}

bool build_layouts(const GameplayTraceRecorder& recorder,
    std::array<ChannelLayout, kChannels.size()>& layouts, std::size_t& layoutCount,
    std::string& error) {
    layoutCount = 0;
    const std::uint64_t count = recorder.samples().size();
    std::uint64_t cursor = GameplayTraceHeaderSize;
    const std::uint64_t channelCount = std::popcount(recorder.requestedChannels());
    std::uint64_t directoryBytes = 0;
    if (!checked_multiply(channelCount, GameplayTraceDirectoryEntrySize, directoryBytes) ||
        !checked_add(cursor, directoryBytes, cursor))
    {
        error = "gameplay trace directory size overflow";
        return false;
    }
    for (const ChannelDefinition& definition : kChannels) {
        if ((recorder.requestedChannels() & gameplay_trace_channel_bit(definition.id)) == 0) {
            continue;
        }
        ChannelLayout& layout = layouts[layoutCount++];
        layout.definition = definition;
        layout.statusOffset = cursor;
        layout.statusLength = count;
        if (!checked_add(cursor, count, cursor)) {
            error = "gameplay trace status size overflow";
            return false;
        }
        layout.payloadOffset = cursor;
        if (!checked_multiply(count, definition.payloadStride, layout.payloadLength) ||
            !checked_add(cursor, layout.payloadLength, cursor))
        {
            error = "gameplay trace payload size overflow";
            return false;
        }
    }
    return true;
}

}  // namespace

void GameplayTraceRecorder::start(
    const std::size_t capacity, const std::uint64_t requestedChannels, TapeBoot boot,
    const GameplayTraceRetentionConfig retention) {
    const std::size_t boundedCapacity = std::min(capacity, GameplayTraceMaximumSamples);
    mSamples.clear();
    mSamples.reserve(boundedCapacity);
    mPreTrigger.clear();
    mPreTrigger.reserve(std::min(boundedCapacity, retention.preTriggerTicks));
    mRequestedChannels = requestedChannels;
    mBootOrigin = std::move(boot);
    mRetention = retention;
    mPreTriggerHead = 0;
    mPostTriggerRemaining = 0;
    mObservedTriggers = 0;
    mTriggerCount = 0;
    mObservedSampleCount = 0;
    mPreviousSampleValid = false;
    mActive = true;
    mCapacityExhausted = false;
}

void GameplayTraceRecorder::record(const GameplayTraceSample& sample) {
    if (!mActive)
        return;
    ++mObservedSampleCount;
    if (mRetention.enabled()) {
        const std::uint32_t detected = detectTriggers(sample) & mRetention.triggers;
        if (detected != 0) {
            mObservedTriggers |= detected;
            ++mTriggerCount;
            flushPreTrigger();
            mPostTriggerRemaining = std::max(mPostTriggerRemaining, mRetention.postTriggerTicks);
            retain(sample);
        } else if (mPostTriggerRemaining != 0) {
            retain(sample);
            --mPostTriggerRemaining;
        } else if (mRetention.preTriggerTicks != 0) {
            const std::size_t preCapacity =
                std::min(mSamples.capacity(), mRetention.preTriggerTicks);
            if (mPreTrigger.size() < preCapacity) {
                mPreTrigger.push_back(sample);
            } else if (preCapacity != 0) {
                mPreTrigger[mPreTriggerHead] = sample;
                mPreTriggerHead = (mPreTriggerHead + 1) % preCapacity;
            }
        }
        mPreviousSample = sample;
        mPreviousSampleValid = true;
        return;
    }
    retain(sample);
}

void GameplayTraceRecorder::retain(const GameplayTraceSample& sample) {
    if (mSamples.size() == mSamples.capacity()) {
        mActive = false;
        mCapacityExhausted = true;
        return;
    }
    mSamples.push_back(sample);
}

void GameplayTraceRecorder::flushPreTrigger() {
    if (mPreTrigger.empty())
        return;
    const std::size_t logicalCapacity =
        std::min(mSamples.capacity(), mRetention.preTriggerTicks);
    const bool wrapped = logicalCapacity != 0 && mPreTrigger.size() == logicalCapacity;
    const std::size_t first = wrapped ? mPreTriggerHead : 0;
    for (std::size_t index = 0; index < mPreTrigger.size() && mActive; ++index) {
        retain(mPreTrigger[(first + index) % mPreTrigger.size()]);
    }
    mPreTrigger.clear();
    mPreTriggerHead = 0;
}

void GameplayTraceRecorder::trigger(const GameplayTraceRetentionTrigger trigger) {
    const std::uint32_t bit = static_cast<std::uint32_t>(trigger);
    if (!mActive || !mRetention.enabled() || (mRetention.triggers & bit) == 0)
        return;
    mObservedTriggers |= bit;
    ++mTriggerCount;
    flushPreTrigger();
    mPostTriggerRemaining = std::max(mPostTriggerRemaining, mRetention.postTriggerTicks);
}

std::uint32_t GameplayTraceRecorder::detectTriggers(const GameplayTraceSample& sample) const {
    if (!mPreviousSampleValid)
        return 0;
    std::uint32_t result = 0;
    const auto reached = [](const GameplayTraceSample& value) {
        return value.goalProgressStatus == GameplayTraceChannelStatus::Present &&
               (value.goalProgress.flags & GameplayTraceGoalReached) != 0;
    };
    if (reached(sample) && !reached(mPreviousSample))
        result |= GameplayTraceTriggerPredicateHit;

    const auto contacts = [](const GameplayTraceSample& value) {
        if (value.playerBackgroundCollisionStatus != GameplayTraceChannelStatus::Present)
            return std::uint64_t{0};
        constexpr std::uint32_t mask = GameplayTraceCollisionGroundContact |
                                       GameplayTraceCollisionLanding |
                                       GameplayTraceCollisionWallContact |
                                       GameplayTraceCollisionRoofContact |
                                       GameplayTraceCollisionWaterIn;
        const auto& collision = value.playerBackgroundCollision;
        const std::uint32_t contactFlags = collision.flags & mask;
        if (contactFlags == 0)
            return std::uint64_t{0};
        std::uint64_t signature = 14695981039346656037ull;
        const auto mix = [&signature](const std::uint64_t fact) {
            signature ^= fact;
            signature *= 1099511628211ull;
        };
        mix(contactFlags);
        for (const auto fact : {collision.groundBgIndex, collision.groundPolyIndex,
                 collision.roofBgIndex, collision.roofPolyIndex, collision.waterBgIndex,
                 collision.waterPolyIndex})
            mix(fact);
        for (const auto fact : {collision.groundOwnerSessionProcessId,
                 collision.roofOwnerSessionProcessId, collision.waterOwnerSessionProcessId})
            mix(fact);
        for (const auto& wall : collision.walls) {
            mix(wall.flags);
            mix(wall.bgIndex);
            mix(wall.polyIndex);
            mix(wall.ownerSessionProcessId);
        }
        return signature;
    };
    const std::uint64_t currentContacts = contacts(sample);
    if (currentContacts != 0 && currentContacts != contacts(mPreviousSample))
        result |= GameplayTraceTriggerNovelContact;

    const auto flagsChanged = [](const GameplayTraceSample& left,
                                  const GameplayTraceSample& right) {
        return left.stage.flags != right.stage.flags ||
               left.playerMotion.flags != right.playerMotion.flags ||
               left.event.flags != right.event.flags || left.event.eventId != right.event.eventId ||
               left.event.mode != right.event.mode || left.event.status != right.event.status ||
               left.playerAction.modeFlags != right.playerAction.modeFlags ||
               left.goalProgress.flags != right.goalProgress.flags ||
               left.selectedActors.flags != right.selectedActors.flags;
    };
    if (flagsChanged(sample, mPreviousSample))
        result |= GameplayTraceTriggerFlagChange;
    return result;
}

void GameplayTraceRecorder::stop() {
    mActive = false;
}

GameplayTraceRecorder& gameplay_trace_recorder() {
    static GameplayTraceRecorder recorder;
    return recorder;
}

bool parse_gameplay_trace_channels(
    const std::string_view text, std::uint64_t& channels, std::string& error) {
    channels = 0;
    std::size_t start = 0;
    while (start <= text.size()) {
        const std::size_t comma = text.find(',', start);
        const std::string_view token = trim(text.substr(
            start, comma == std::string_view::npos ? text.size() - start : comma - start));
        if (token.empty()) {
            error = "gameplay trace channel list contains an empty name";
            return false;
        }

        std::uint64_t bit = 0;
        if (token == "core")
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::Core);
        else if (token == "stage")
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::Stage);
        else if (token == "applied-pads") {
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::AppliedPads);
        } else if (token == "player-motion") {
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::PlayerMotion);
        } else if (token == "event")
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::Event);
        else if (token == "scene-exit") {
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::SceneExit);
        } else if (token == "rng")
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::Rng);
        else if (token == "camera")
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::Camera);
        else if (token == "player-action") {
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::PlayerAction);
        } else if (token == "player-background-collision") {
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::PlayerBackgroundCollision);
        } else if (token == "player-collision-surfaces") {
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces);
        } else if (token == "goal-progress") {
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::GoalProgress);
        } else if (token == "selected-actors") {
            bit = gameplay_trace_channel_bit(GameplayTraceChannel::SelectedActors);
        } else if (token == "all")
            bit = GameplayTraceKnownChannels;
        else {
            error = "unknown gameplay trace channel '" + std::string(token) + "'";
            return false;
        }
        if ((channels & bit) != 0) {
            error = "duplicate gameplay trace channel '" + std::string(token) + "'";
            return false;
        }
        channels |= bit;
        if (comma == std::string_view::npos)
            break;
        start = comma + 1;
    }
    if ((channels & gameplay_trace_channel_bit(GameplayTraceChannel::Core)) == 0) {
        error = "gameplay trace channel list must include core";
        return false;
    }
    if ((channels & gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces)) !=
            0 &&
        (channels & gameplay_trace_channel_bit(GameplayTraceChannel::Stage)) == 0)
    {
        error = "gameplay trace collision surfaces require the stage channel";
        return false;
    }
    return true;
}

bool parse_gameplay_trace_retention_triggers(
    const std::string_view text, std::uint32_t& triggers, std::string& error) {
    triggers = 0;
    std::size_t start = 0;
    while (start <= text.size()) {
        const std::size_t comma = text.find(',', start);
        const std::string_view token = trim(text.substr(
            start, comma == std::string_view::npos ? text.size() - start : comma - start));
        std::uint32_t bit = 0;
        if (token == "crash")
            bit = GameplayTraceTriggerCrash;
        else if (token == "contact")
            bit = GameplayTraceTriggerNovelContact;
        else if (token == "flag")
            bit = GameplayTraceTriggerFlagChange;
        else if (token == "predicate")
            bit = GameplayTraceTriggerPredicateHit;
        else if (token == "all")
            bit = GameplayTraceKnownRetentionTriggers;
        else {
            error = token.empty() ? "gameplay trace trigger list contains an empty name" :
                                    "unknown gameplay trace trigger '" + std::string(token) + "'";
            return false;
        }
        if ((triggers & bit) != 0) {
            error = "duplicate gameplay trace trigger '" + std::string(token) + "'";
            return false;
        }
        triggers |= bit;
        if (comma == std::string_view::npos)
            break;
        start = comma + 1;
    }
    return triggers != 0;
}

bool write_gameplay_trace(
    const std::filesystem::path& path, const GameplayTraceRecorder& recorder, std::string& error) {
    if (!validate_trace(recorder, error))
        return false;

    std::array<ChannelLayout, kChannels.size()> layouts{};
    std::size_t layoutCount = 0;
    if (!build_layouts(recorder, layouts, layoutCount, error))
        return false;

    std::vector<std::uint8_t> fixtureBytes;
    const TapeBoot& boot = recorder.bootOrigin();
    if (boot.fixture &&
        encode_scenario_fixture(*boot.fixture, fixtureBytes) != ScenarioFixtureError::None)
    {
        error = "gameplay trace could not encode its scenario fixture";
        return false;
    }
    const std::uint64_t fixtureOffset = fixtureBytes.empty() ? 0 :
        layouts[layoutCount - 1].payloadOffset + layouts[layoutCount - 1].payloadLength;

    std::error_code filesystemError;
    if (const auto parent = path.parent_path(); !parent.empty()) {
        std::filesystem::create_directories(parent, filesystemError);
        if (filesystemError) {
            error = filesystemError.message();
            return false;
        }
    }
    if (std::filesystem::exists(path, filesystemError) || filesystemError) {
        error =
            filesystemError ? filesystemError.message() : "gameplay trace output already exists";
        return false;
    }
    std::filesystem::path temporary = path;
    temporary += ".tmp";
    if (std::filesystem::exists(temporary, filesystemError) || filesystemError) {
        error = filesystemError ? filesystemError.message() :
                                  "gameplay trace temporary output already exists";
        return false;
    }

    std::ofstream stream(temporary, std::ios::binary | std::ios::trunc);
    if (!stream) {
        error = "could not open temporary gameplay trace for writing";
        return false;
    }

    constexpr std::array<char, 8> magic{'D', 'U', 'S', 'K', 'T', 'R', 'C', 'E'};
    stream.write(magic.data(), magic.size());
    write_integer(stream, GameplayTraceVersion);
    write_integer(stream, GameplayTraceHeaderSize);
    write_integer<std::uint32_t>(stream, 30);
    write_integer<std::uint32_t>(stream, 1);
    write_integer(stream, static_cast<std::uint64_t>(recorder.samples().size()));
    write_integer<std::uint32_t>(stream,
        kFileComplete | (recorder.capacityExhausted() ? kFileCapacityExhausted : 0) |
            (recorder.retention().enabled() ? kFileTriggerRetention : 0));
    write_integer(stream, static_cast<std::uint16_t>(layoutCount));
    write_integer(stream, GameplayTraceDirectoryEntrySize);
    write_integer<std::uint64_t>(stream, GameplayTraceHeaderSize);
    write_integer<std::uint64_t>(
        stream, GameplayTraceHeaderSize + layoutCount * GameplayTraceDirectoryEntrySize);
    write_integer(stream, recorder.requestedChannels());
    write_integer<std::uint32_t>(stream, 0);
    write_integer(stream, static_cast<std::uint8_t>(boot.kind));
    const bool stageBoot = boot.kind == TapeBootKind::Stage;
    write_integer(stream, stageBoot ? boot.saveSlot : std::uint8_t{0});
    write_integer(stream, stageBoot ? boot.room : std::int8_t{0});
    write_integer(stream, stageBoot ? boot.layer : std::int8_t{0});
    write_integer(stream, stageBoot ? boot.point : std::int16_t{0});
    write_integer(stream, stageBoot ? static_cast<std::uint8_t>(boot.stage.size()) :
                                     std::uint8_t{0});
    write_integer<std::uint8_t>(stream, 0);
    std::array<char, 16> stage{};
    if (stageBoot)
        std::ranges::copy(boot.stage, stage.begin());
    stream.write(stage.data(), stage.size());
    write_integer(stream, fixtureOffset);
    write_integer(stream, static_cast<std::uint32_t>(fixtureBytes.size()));
    write_integer(stream, recorder.retention().triggers);
    write_integer(stream, recorder.observedTriggers());
    write_integer(stream, static_cast<std::uint32_t>(recorder.retention().preTriggerTicks));
    write_integer(stream, static_cast<std::uint32_t>(recorder.retention().postTriggerTicks));
    write_integer(stream, recorder.triggerCount());
    write_integer(stream, recorder.observedSampleCount());

    for (std::size_t index = 0; index < layoutCount; ++index) {
        const ChannelLayout& layout = layouts[index];
        write_integer(stream, static_cast<std::uint16_t>(layout.definition.id));
        write_integer(stream, layout.definition.version);
        write_integer<std::uint32_t>(
            stream, kChannelDense | (layout.definition.required ? kChannelRequired : 0));
        write_integer(stream, layout.definition.payloadStride);
        write_integer<std::uint32_t>(stream, 1);
        write_integer(stream, layout.statusOffset);
        write_integer(stream, layout.statusLength);
        write_integer(stream, layout.payloadOffset);
        write_integer(stream, layout.payloadLength);
        write_integer<std::uint64_t>(stream, 0);
        write_integer<std::uint64_t>(stream, 0);
    }
    for (std::size_t index = 0; index < layoutCount; ++index) {
        const GameplayTraceChannel channel = layouts[index].definition.id;
        for (const GameplayTraceSample& sample : recorder.samples()) {
            write_integer(stream, static_cast<std::uint8_t>(status_for(sample, channel)));
        }
        for (const GameplayTraceSample& sample : recorder.samples()) {
            write_payload(stream, sample, channel);
        }
    }
    if (!fixtureBytes.empty()) {
        stream.write(reinterpret_cast<const char*>(fixtureBytes.data()), fixtureBytes.size());
    }

    stream.flush();
    stream.close();
    if (!stream) {
        error = "failed while writing gameplay trace";
        std::filesystem::remove(temporary, filesystemError);
        return false;
    }
    std::filesystem::rename(temporary, path, filesystemError);
    if (filesystemError) {
        error = filesystemError.message();
        std::filesystem::remove(temporary, filesystemError);
        return false;
    }
    return true;
}

}  // namespace dusk::automation
