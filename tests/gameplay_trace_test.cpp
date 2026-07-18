#include "dusk/automation/gameplay_trace.hpp"

#include <bit>
#include <chrono>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

namespace {

void require(bool condition, const char* expression, int line) {
    if (!condition) {
        std::cerr << "gameplay_trace_test.cpp:" << line << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

template <typename T>
T readLittle(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    using U = std::make_unsigned_t<T>;
    U value = 0;
    for (std::size_t index = 0; index < sizeof(T); ++index) {
        value |= static_cast<U>(bytes.at(offset + index)) << (index * 8);
    }
    return static_cast<T>(value);
}

float readFloat(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    return std::bit_cast<float>(readLittle<std::uint32_t>(bytes, offset));
}

std::vector<std::uint8_t> readFile(const std::filesystem::path& path) {
    std::ifstream stream(path, std::ios::binary);
    REQUIRE(stream.good());
    return {std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>()};
}

dusk::automation::GameplayTraceSample sample(std::uint64_t simulationTick) {
    using namespace dusk::automation;
    GameplayTraceSample value;
    value.core.boundaryIndex = simulationTick + 1;
    value.core.simulationTick = simulationTick;
    value.core.tapeFrame = simulationTick;
    value.core.flags = GameplayTraceSimulationTickValid | GameplayTraceTapeFrameValid;
    value.core.inputSource = GameplayTraceInputTape;
    value.stageStatus = GameplayTraceChannelStatus::Present;
    value.stage.stageName = {'F', '_', 'S', 'P', '1', '0', '3', '\0'};
    value.stage.room = 1;
    value.stage.layer = 3;
    value.stage.point = 7;
    value.cameraStatus = GameplayTraceChannelStatus::Absent;
    return value;
}

void testChannelParser() {
    using namespace dusk::automation;
    std::uint64_t channels = 0;
    std::string error;
    REQUIRE(parse_gameplay_trace_channels("core, stage, camera", channels, error));
    REQUIRE(channels == (gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
                            gameplay_trace_channel_bit(GameplayTraceChannel::Stage) |
                            gameplay_trace_channel_bit(GameplayTraceChannel::Camera)));
    REQUIRE(!parse_gameplay_trace_channels("stage", channels, error));
    REQUIRE(!parse_gameplay_trace_channels("core,core", channels, error));
    REQUIRE(!parse_gameplay_trace_channels("core,collision", channels, error));
}

void testExactV2Layout(const std::filesystem::path& path) {
    using namespace dusk::automation;
    constexpr std::uint64_t channels = gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
                                       gameplay_trace_channel_bit(GameplayTraceChannel::Stage) |
                                       gameplay_trace_channel_bit(GameplayTraceChannel::Camera);

    GameplayTraceRecorder recorder;
    recorder.start(2, channels);
    recorder.record(sample(0));
    GameplayTraceSample second = sample(1);
    second.core.tapeFrame = GameplayTraceNoTapeFrame;
    second.core.flags = GameplayTraceSimulationTickValid;
    second.core.inputSource = GameplayTraceInputNone;
    second.stage.nextStageName = {'F', '_', 'S', 'P', '1', '0', '4', '\0'};
    second.stage.nextRoom = 0;
    second.stage.nextLayer = 0;
    second.stage.nextPoint = 26;
    second.stage.flags = GameplayTraceNextStageEnabled;
    second.cameraStatus = GameplayTraceChannelStatus::Present;
    second.camera.viewYaw = -123;
    second.camera.controlledYaw = 456;
    second.camera.bank = -7;
    second.camera.eye = {1.0f, 2.0f, 3.0f};
    second.camera.center = {4.0f, 5.0f, 6.0f};
    second.camera.up = {0.0f, 1.0f, 0.0f};
    second.camera.fovy = 45.0f;
    recorder.record(second);
    recorder.stop();

    std::string error;
    REQUIRE(write_gameplay_trace(path, recorder, error));
    const auto bytes = readFile(path);
    REQUIRE(bytes.size() == 486);
    REQUIRE(std::string(bytes.begin(), bytes.begin() + 8) == "DUSKTRCE");
    REQUIRE(readLittle<std::uint16_t>(bytes, 8) == GameplayTraceVersion);
    REQUIRE(readLittle<std::uint16_t>(bytes, 10) == GameplayTraceHeaderSize);
    REQUIRE(readLittle<std::uint32_t>(bytes, 12) == 30);
    REQUIRE(readLittle<std::uint32_t>(bytes, 16) == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 20) == 2);
    REQUIRE(readLittle<std::uint32_t>(bytes, 28) == 1);
    REQUIRE(readLittle<std::uint16_t>(bytes, 32) == 3);
    REQUIRE(readLittle<std::uint64_t>(bytes, 44) == 256);
    REQUIRE(readLittle<std::uint64_t>(bytes, 52) == channels);

    // Core descriptor and columns.
    REQUIRE(readLittle<std::uint16_t>(bytes, 64) == 0);
    REQUIRE(readLittle<std::uint32_t>(bytes, 68) == 3);
    REQUIRE(readLittle<std::uint32_t>(bytes, 72) == 32);
    REQUIRE(readLittle<std::uint64_t>(bytes, 80) == 256);
    REQUIRE(readLittle<std::uint64_t>(bytes, 96) == 258);
    REQUIRE(bytes[256] == 1 && bytes[257] == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 258) == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 290) == 2);
    REQUIRE(readLittle<std::uint64_t>(bytes, 298) == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 306) == GameplayTraceNoTapeFrame);

    // Stage descriptor and its explicit transition state.
    REQUIRE(readLittle<std::uint16_t>(bytes, 128) == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 144) == 322);
    REQUIRE(readLittle<std::uint64_t>(bytes, 160) == 324);
    REQUIRE(bytes[322] == 1 && bytes[323] == 1);
    REQUIRE(std::string(bytes.begin() + 368, bytes.begin() + 375) == "F_SP104");
    REQUIRE(readLittle<std::int16_t>(bytes, 378) == 26);
    REQUIRE(readLittle<std::uint32_t>(bytes, 380) == GameplayTraceNextStageEnabled);

    // Camera is explicitly absent at boundary one and present with exact bits at two.
    REQUIRE(readLittle<std::uint16_t>(bytes, 192) == 7);
    REQUIRE(readLittle<std::uint64_t>(bytes, 208) == 388);
    REQUIRE(readLittle<std::uint64_t>(bytes, 224) == 390);
    REQUIRE(bytes[388] == 2 && bytes[389] == 1);
    REQUIRE(readLittle<std::int16_t>(bytes, 438) == -123);
    REQUIRE(readLittle<std::int16_t>(bytes, 440) == 456);
    REQUIRE(readLittle<std::int16_t>(bytes, 442) == -7);
    REQUIRE(readFloat(bytes, 446) == 1.0f);
    REQUIRE(readFloat(bytes, 482) == 45.0f);

    REQUIRE(!write_gameplay_trace(path, recorder, error));
    REQUIRE(error == "gameplay trace output already exists");
}

void testValidationAndCapacity(
    const std::filesystem::path& invalidPath, const std::filesystem::path& exhaustedPath) {
    using namespace dusk::automation;
    constexpr std::uint64_t core = gameplay_trace_channel_bit(GameplayTraceChannel::Core);
    GameplayTraceRecorder invalid;
    invalid.start(1, core);
    GameplayTraceSample invalidSample = sample(0);
    invalid.record(invalidSample);
    invalid.stop();
    std::string error;
    REQUIRE(!write_gameplay_trace(invalidPath, invalid, error));
    REQUIRE(!std::filesystem::exists(invalidPath));

    GameplayTraceRecorder exhausted;
    exhausted.start(1, core);
    GameplayTraceSample coreSample = sample(0);
    coreSample.stageStatus = GameplayTraceChannelStatus::NotSampled;
    coreSample.cameraStatus = GameplayTraceChannelStatus::NotSampled;
    exhausted.record(coreSample);
    exhausted.record(coreSample);
    REQUIRE(exhausted.capacityExhausted());
    REQUIRE(write_gameplay_trace(exhaustedPath, exhausted, error));
    REQUIRE((readLittle<std::uint32_t>(readFile(exhaustedPath), 28) & 2) != 0);
}

}  // namespace

int main() {
    testChannelParser();
    const auto nonce = std::chrono::high_resolution_clock::now().time_since_epoch().count();
    const auto directory = std::filesystem::temp_directory_path() /
                           ("dusklight-gameplay-trace-test-" + std::to_string(nonce));
    REQUIRE(std::filesystem::create_directory(directory));
    const auto exactPath = directory / "exact.trace";
    const auto invalidPath = directory / "invalid.trace";
    const auto exhaustedPath = directory / "exhausted.trace";
    testExactV2Layout(exactPath);
    testValidationAndCapacity(invalidPath, exhaustedPath);
    REQUIRE(std::filesystem::remove(exactPath));
    REQUIRE(std::filesystem::remove(exhaustedPath));
    REQUIRE(std::filesystem::remove(directory));
    std::cout << "Gameplay trace tests passed\n";
    return 0;
}
