#include "dusk/automation/gameplay_trace.hpp"

#include <algorithm>
#include <array>
#include <bit>
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
    ChannelDefinition{GameplayTraceChannel::SceneExit, 1, 24, false},
    ChannelDefinition{GameplayTraceChannel::Rng, 1, 64, false},
    ChannelDefinition{GameplayTraceChannel::Camera, 1, 48, false},
    ChannelDefinition{GameplayTraceChannel::PlayerAction, 1, 104, false},
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
    write_integer(stream, sample.actorName);
    write_integer<std::uint16_t>(stream, 0);
    write_vec3(stream, sample.position);
    write_float(stream, sample.distance);
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
            (sample.appliedPads.validPorts & ~0x0fu) != 0 ||
            (sample.appliedPads.ownedPorts & ~0x0fu) != 0)
        {
            error = "gameplay trace sample has unknown channel flags";
            return false;
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
