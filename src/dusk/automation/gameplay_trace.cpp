#include "dusk/automation/gameplay_trace.hpp"

#include <algorithm>
#include <array>
#include <bit>
#include <cmath>
#include <fstream>
#include <limits>
#include <system_error>
#include <type_traits>

namespace dusk::automation {
namespace {

constexpr std::uint32_t kFileComplete = 1u << 0;
constexpr std::uint32_t kFileCapacityExhausted = 1u << 1;
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
    const std::uint64_t requested = recorder.requestedChannels();
    if ((requested & gameplay_trace_channel_bit(GameplayTraceChannel::Core)) == 0) {
        error = "gameplay trace v2 requires the core channel";
        return false;
    }
    if ((requested & ~GameplayTraceKnownChannels) != 0) {
        error = "gameplay trace requests unknown channels";
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
            (sample.appliedPads.validPorts & ~0x0fu) != 0 ||
            (sample.appliedPads.ownedPorts & ~0x0fu) != 0)
        {
            error = "gameplay trace sample has unknown channel flags";
            return false;
        }
        if (sample.sceneExitStatus == GameplayTraceChannelStatus::Truncated ||
            sample.playerBackgroundCollisionStatus == GameplayTraceChannelStatus::Truncated)
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
    const std::size_t capacity, const std::uint64_t requestedChannels) {
    mSamples.clear();
    mSamples.reserve(capacity);
    mRequestedChannels = requestedChannels;
    mActive = true;
    mCapacityExhausted = false;
}

void GameplayTraceRecorder::record(const GameplayTraceSample& sample) {
    if (!mActive)
        return;
    if (mSamples.size() == mSamples.capacity()) {
        mActive = false;
        mCapacityExhausted = true;
        return;
    }
    mSamples.push_back(sample);
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
    return true;
}

bool write_gameplay_trace(
    const std::filesystem::path& path, const GameplayTraceRecorder& recorder, std::string& error) {
    if (!validate_trace(recorder, error))
        return false;

    std::array<ChannelLayout, kChannels.size()> layouts{};
    std::size_t layoutCount = 0;
    if (!build_layouts(recorder, layouts, layoutCount, error))
        return false;

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
    write_integer<std::uint32_t>(
        stream, kFileComplete | (recorder.capacityExhausted() ? kFileCapacityExhausted : 0));
    write_integer(stream, static_cast<std::uint16_t>(layoutCount));
    write_integer(stream, GameplayTraceDirectoryEntrySize);
    write_integer<std::uint64_t>(stream, GameplayTraceHeaderSize);
    write_integer<std::uint64_t>(
        stream, GameplayTraceHeaderSize + layoutCount * GameplayTraceDirectoryEntrySize);
    write_integer(stream, recorder.requestedChannels());
    write_integer<std::uint32_t>(stream, 0);

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
