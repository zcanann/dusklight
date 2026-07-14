#include "dusk/automation/gameplay_trace.hpp"

#include <array>
#include <bit>
#include <fstream>
#include <limits>
#include <system_error>
#include <type_traits>

namespace dusk::automation {
namespace {

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

void write_sample(std::ostream& stream, const GameplayTraceSample& sample) {
    write_integer(stream, sample.simulationTick);
    write_integer(stream, sample.tapeFrame);
    stream.write(sample.stageName, sizeof(sample.stageName));
    write_integer(stream, sample.room);
    write_integer(stream, sample.layer);
    write_integer(stream, sample.point);
    write_integer(stream, sample.flags);
    write_integer(stream, sample.playerActorName);
    write_integer(stream, sample.currentAngleY);
    write_integer(stream, sample.shapeAngleY);
    write_integer(stream, sample.buttons);
    write_integer(stream, sample.stickX);
    write_integer(stream, sample.stickY);
    write_float(stream, sample.positionX);
    write_float(stream, sample.positionY);
    write_float(stream, sample.positionZ);
    write_float(stream, sample.velocityX);
    write_float(stream, sample.velocityY);
    write_float(stream, sample.velocityZ);
    write_float(stream, sample.forwardSpeed);
    write_integer(stream, sample.playerProcId);
    write_integer(stream, sample.eventId);
    write_integer(stream, sample.eventMode);
    write_integer(stream, sample.eventStatus);
    write_integer(stream, sample.eventMapToolId);
    write_integer(stream, sample.padError);
    write_integer(stream, sample.eventNameHash);
    write_integer(stream, sample.nearestSceneExitActorName);
    write_float(stream, sample.nearestSceneExitX);
    write_float(stream, sample.nearestSceneExitY);
    write_float(stream, sample.nearestSceneExitZ);
    write_float(stream, sample.nearestSceneExitDistance);
    write_integer<std::uint16_t>(stream, 0);
}

} // namespace

void GameplayTraceRecorder::start(const std::size_t capacity) {
    mSamples.clear();
    mSamples.reserve(capacity);
    mActive = true;
    mCapacityExhausted = false;
}

void GameplayTraceRecorder::record(const GameplayTraceSample& sample) {
    if (!mActive) {
        return;
    }
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

bool write_gameplay_trace(const std::filesystem::path& path,
                          const GameplayTraceRecorder& recorder, std::string& error) {
    if (recorder.samples().size() > std::numeric_limits<std::uint64_t>::max()) {
        error = "gameplay trace contains too many samples";
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

    std::ofstream stream(path, std::ios::binary | std::ios::trunc);
    if (!stream) {
        error = "could not open trace for writing";
        return false;
    }

    constexpr std::array<char, 8> magic{'D', 'U', 'S', 'K', 'T', 'R', 'C', 'E'};
    stream.write(magic.data(), magic.size());
    write_integer(stream, GameplayTraceVersion);
    write_integer(stream, GameplayTraceRecordSize);
    write_integer<std::uint32_t>(stream, 30);
    write_integer<std::uint32_t>(stream, 1);
    write_integer(stream, static_cast<std::uint64_t>(recorder.samples().size()));
    write_integer<std::uint32_t>(stream, recorder.capacityExhausted() ? 1u : 0u);
    write_integer<std::uint32_t>(stream, 0);
    for (const GameplayTraceSample& sample : recorder.samples()) {
        write_sample(stream, sample);
    }
    if (!stream) {
        error = "failed while writing gameplay trace";
        return false;
    }
    return true;
}

} // namespace dusk::automation
