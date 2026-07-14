#pragma once

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

namespace dusk::automation {

inline constexpr std::uint16_t GameplayTraceVersion = 1;
inline constexpr std::uint16_t GameplayTraceRecordSize = 102;
inline constexpr std::uint64_t GameplayTraceNoTapeFrame = ~std::uint64_t{0};

enum GameplayTraceFlags : std::uint32_t {
    GameplayTracePlayerPresent = 1u << 0,
    GameplayTracePlayerIsLink = 1u << 1,
    GameplayTraceEventRunning = 1u << 2,
    GameplayTraceTapePlaying = 1u << 3,
    GameplayTraceControllerPlaying = 1u << 4,
};

struct GameplayTraceSample {
    std::uint64_t simulationTick = 0;
    std::uint64_t tapeFrame = GameplayTraceNoTapeFrame;
    char stageName[8]{};
    std::int8_t room = 0;
    std::int8_t layer = -1;
    std::int16_t point = 0;
    std::uint32_t flags = 0;
    std::int16_t playerActorName = -1;
    std::int16_t currentAngleY = 0;
    std::int16_t shapeAngleY = 0;
    std::uint16_t buttons = 0;
    std::int8_t stickX = 0;
    std::int8_t stickY = 0;
    float positionX = 0.0f;
    float positionY = 0.0f;
    float positionZ = 0.0f;
    float velocityX = 0.0f;
    float velocityY = 0.0f;
    float velocityZ = 0.0f;
    float forwardSpeed = 0.0f;
    std::uint16_t playerProcId = 0xffff;
    std::int16_t eventId = -1;
    std::uint8_t eventMode = 0;
    std::uint8_t eventStatus = 0;
    std::uint8_t eventMapToolId = 0xff;
    std::int8_t padError = -1;
    std::uint32_t eventNameHash = 0;
    std::int16_t nearestSceneExitActorName = -1;
    float nearestSceneExitX = 0.0f;
    float nearestSceneExitY = 0.0f;
    float nearestSceneExitZ = 0.0f;
    float nearestSceneExitDistance = -1.0f;
};

class GameplayTraceRecorder {
public:
    void start(std::size_t capacity);
    void record(const GameplayTraceSample& sample);
    void stop();

    bool active() const { return mActive; }
    bool capacityExhausted() const { return mCapacityExhausted; }
    const std::vector<GameplayTraceSample>& samples() const { return mSamples; }

private:
    std::vector<GameplayTraceSample> mSamples;
    bool mActive = false;
    bool mCapacityExhausted = false;
};

GameplayTraceRecorder& gameplay_trace_recorder();

bool write_gameplay_trace(const std::filesystem::path& path,
                          const GameplayTraceRecorder& recorder, std::string& error);

} // namespace dusk::automation
