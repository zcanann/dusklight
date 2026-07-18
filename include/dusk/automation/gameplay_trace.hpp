#pragma once

#include "dusk/automation/input_tape.hpp"
#include "dusk/automation/rng.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>
#include <vector>

namespace dusk::automation {

inline constexpr std::uint16_t GameplayTraceVersion = 2;
inline constexpr std::uint16_t GameplayTraceHeaderSize = 64;
inline constexpr std::uint16_t GameplayTraceDirectoryEntrySize = 64;
inline constexpr std::uint16_t GameplayTraceV1HeaderSize = 36;
inline constexpr std::uint16_t GameplayTraceV1RecordSize = 102;
inline constexpr std::uint64_t GameplayTraceNoTick = ~std::uint64_t{0};
inline constexpr std::uint64_t GameplayTraceNoTapeFrame = ~std::uint64_t{0};

enum class GameplayTraceChannel : std::uint16_t {
    Core = 0,
    Stage = 1,
    AppliedPads = 2,
    PlayerMotion = 3,
    Event = 4,
    SceneExit = 5,
    Rng = 6,
    Camera = 7,
    PlayerAction = 8,
};

constexpr std::uint64_t gameplay_trace_channel_bit(const GameplayTraceChannel channel) {
    return std::uint64_t{1} << static_cast<std::uint16_t>(channel);
}

inline constexpr std::uint64_t GameplayTraceKnownChannels =
    gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Stage) |
    gameplay_trace_channel_bit(GameplayTraceChannel::AppliedPads) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerMotion) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Event) |
    gameplay_trace_channel_bit(GameplayTraceChannel::SceneExit) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Rng) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Camera) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerAction);
inline constexpr std::uint64_t GameplayTraceDefaultChannels = GameplayTraceKnownChannels;

enum class GameplayTraceChannelStatus : std::uint8_t {
    NotSampled = 0,
    Present = 1,
    Absent = 2,
    Unavailable = 3,
    Truncated = 4,
};

enum class GameplayTracePhase : std::uint8_t {
    PreInput = 1,
    PostSimulation = 2,
};

enum class GameplayTraceBoundaryKind : std::uint8_t {
    Boot = 0,
    Tick = 1,
};

enum GameplayTraceInputSource : std::uint8_t {
    GameplayTraceInputNone = 0,
    GameplayTraceInputTape = 1u << 0,
    GameplayTraceInputController = 1u << 1,
    GameplayTraceInputLive = 1u << 2,
};

enum GameplayTraceCoreFlags : std::uint32_t {
    GameplayTraceSimulationTickValid = 1u << 0,
    GameplayTraceTapeFrameValid = 1u << 1,
};

enum GameplayTraceStageFlags : std::uint32_t {
    GameplayTraceNextStageEnabled = 1u << 0,
};

enum GameplayTracePlayerFlags : std::uint32_t {
    GameplayTracePlayerIsLink = 1u << 0,
};

enum GameplayTraceEventFlags : std::uint32_t {
    GameplayTraceEventRunning = 1u << 0,
    GameplayTraceEventNameHashPresent = 1u << 1,
};

struct GameplayTraceCoreSample {
    std::uint64_t boundaryIndex = 0;
    std::uint64_t simulationTick = GameplayTraceNoTick;
    std::uint64_t tapeFrame = GameplayTraceNoTapeFrame;
    std::uint32_t flags = 0;
    GameplayTracePhase phase = GameplayTracePhase::PostSimulation;
    GameplayTraceBoundaryKind boundaryKind = GameplayTraceBoundaryKind::Tick;
    std::uint8_t inputSource = GameplayTraceInputNone;
};

struct GameplayTraceStageSample {
    std::array<char, 8> stageName{};
    std::int8_t room = 0;
    std::int8_t layer = -1;
    std::int16_t point = 0;
    std::array<char, 8> nextStageName{};
    std::int8_t nextRoom = 0;
    std::int8_t nextLayer = -1;
    std::int16_t nextPoint = 0;
    std::uint32_t flags = 0;
};

struct GameplayTraceAppliedPadsSample {
    std::uint8_t validPorts = 0;
    std::uint8_t ownedPorts = 0;
    std::array<RawPadState, kInputPortCount> pads{};
};

struct GameplayTracePlayerMotionSample {
    std::uint32_t sessionProcessId = 0;
    std::int16_t actorName = -1;
    std::uint16_t procedureId = 0xffff;
    std::array<std::int16_t, 3> currentAngle{};
    std::array<std::int16_t, 3> shapeAngle{};
    std::array<float, 3> position{};
    std::array<float, 3> velocity{};
    float forwardSpeed = 0.0f;
    std::uint32_t flags = 0;
};

struct GameplayTraceEventSample {
    std::uint32_t flags = 0;
    std::int16_t eventId = -1;
    std::uint8_t mode = 0;
    std::uint8_t status = 0;
    std::uint8_t mapToolId = 0xff;
    std::uint32_t nameHash = 0;
};

struct GameplayTraceSceneExitSample {
    std::uint32_t sessionProcessId = 0;
    std::int16_t actorName = -1;
    std::array<float, 3> position{};
    float distance = 0.0f;
};

struct GameplayTraceCameraSample {
    std::int16_t viewYaw = 0;
    std::int16_t controlledYaw = 0;
    std::int16_t bank = 0;
    std::array<float, 3> eye{};
    std::array<float, 3> center{};
    std::array<float, 3> up{};
    float fovy = 0.0f;
};

struct GameplayTraceAnimationLane {
    std::uint16_t resourceId = 0xffff;
    float frame = 0.0f;
    float rate = 0.0f;
};

struct GameplayTracePlayerActionSample {
    std::uint16_t procedureId = 0xffff;
    std::uint32_t modeFlags = 0;
    std::array<std::int16_t, 6> procedureContextRaw{};
    std::int16_t damageWaitTimer = 0;
    std::uint16_t swordAtUpTime = 0;
    std::int16_t iceDamageWaitTimer = 0;
    std::uint8_t swordChangeWaitTimer = 0;
    std::array<GameplayTraceAnimationLane, 3> underAnimations{};
    std::array<GameplayTraceAnimationLane, 3> upperAnimations{};
};

struct GameplayTraceSample {
    GameplayTraceCoreSample core{};
    GameplayTraceStageSample stage{};
    GameplayTraceAppliedPadsSample appliedPads{};
    GameplayTracePlayerMotionSample playerMotion{};
    GameplayTraceEventSample event{};
    GameplayTraceSceneExitSample sceneExit{};
    GameRngSnapshot rng{};
    GameplayTraceCameraSample camera{};
    GameplayTracePlayerActionSample playerAction{};

    GameplayTraceChannelStatus stageStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus appliedPadsStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus playerMotionStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus eventStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus sceneExitStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus rngStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus cameraStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus playerActionStatus = GameplayTraceChannelStatus::NotSampled;
};

class GameplayTraceRecorder {
public:
    void start(
        std::size_t capacity, std::uint64_t requestedChannels = GameplayTraceDefaultChannels);
    void record(const GameplayTraceSample& sample);
    void stop();

    bool active() const { return mActive; }
    bool capacityExhausted() const { return mCapacityExhausted; }
    std::uint64_t requestedChannels() const { return mRequestedChannels; }
    const std::vector<GameplayTraceSample>& samples() const { return mSamples; }

private:
    std::vector<GameplayTraceSample> mSamples;
    std::uint64_t mRequestedChannels = GameplayTraceDefaultChannels;
    bool mActive = false;
    bool mCapacityExhausted = false;
};

GameplayTraceRecorder& gameplay_trace_recorder();

bool parse_gameplay_trace_channels(
    std::string_view text, std::uint64_t& channels, std::string& error);
bool write_gameplay_trace(
    const std::filesystem::path& path, const GameplayTraceRecorder& recorder, std::string& error);

}  // namespace dusk::automation
