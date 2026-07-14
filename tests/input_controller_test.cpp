#include "dusk/automation/input_controller.hpp"

#include <array>
#include <bit>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <limits>
#include <span>
#include <vector>

namespace {

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "input_controller_test.cpp:" << line << ": check failed: " << expression
                  << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

void writeU16(std::uint8_t* output, const std::uint16_t value) {
    output[0] = static_cast<std::uint8_t>(value);
    output[1] = static_cast<std::uint8_t>(value >> 8);
}

void writeI16(std::uint8_t* output, const std::int16_t value) {
    writeU16(output, std::bit_cast<std::uint16_t>(value));
}

void writeU32(std::uint8_t* output, const std::uint32_t value) {
    for (unsigned byte = 0; byte < 4; ++byte) {
        output[byte] = static_cast<std::uint8_t>(value >> (byte * 8));
    }
}

void writeF32(std::uint8_t* output, const float value) {
    writeU32(output, std::bit_cast<std::uint32_t>(value));
}

std::vector<std::uint8_t> makeProgram(const std::uint32_t duration, const std::uint16_t layers) {
    using namespace dusk::automation;
    std::vector<std::uint8_t> bytes(
        kInputControllerHeaderSize + layers * kInputControllerRecordSize);
    std::copy(kInputControllerMagic.begin(), kInputControllerMagic.end(), bytes.begin());
    writeU16(bytes.data() + 8, kInputControllerMajorVersion);
    writeU16(bytes.data() + 10, kInputControllerMinorVersion);
    writeU16(bytes.data() + 12, kInputControllerHeaderSize);
    writeU16(bytes.data() + 14, kInputControllerRecordSize);
    writeU32(bytes.data() + 16, duration);
    writeU16(bytes.data() + 20, layers);
    writeU32(bytes.data() + 24, layers * kInputControllerRecordSize);
    return bytes;
}

std::uint8_t* layer(std::vector<std::uint8_t>& bytes, const std::size_t index) {
    return bytes.data() + dusk::automation::kInputControllerHeaderSize +
           index * dusk::automation::kInputControllerRecordSize;
}

void setCommon(std::uint8_t* record, const std::uint8_t kind, const std::uint8_t blend,
    const std::uint32_t start, const std::uint32_t duration) {
    record[0] = kind;
    record[1] = blend;
    writeU32(record + 4, start);
    writeU32(record + 8, duration);
}

void setBezier(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const std::array<std::int16_t, 8>& points) {
    setCommon(record, 1, blend, start, duration);
    for (std::size_t index = 0; index < points.size(); ++index) {
        writeI16(record + 12 + index * 2, points[index]);
    }
}

void setPoint(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const float x, const float y, const float z, const float offsetX,
    const float offsetY, const float offsetZ, const float stopRadius,
    const std::uint8_t magnitude) {
    setCommon(record, 2, blend, start, duration);
    writeF32(record + 12, x);
    writeF32(record + 16, y);
    writeF32(record + 20, z);
    writeF32(record + 24, offsetX);
    writeF32(record + 28, offsetY);
    writeF32(record + 32, offsetZ);
    writeF32(record + 36, stopRadius);
    record[40] = magnitude;
}

void setActor(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const std::int16_t actorName, const float offsetX,
    const float offsetY, const float offsetZ, const float stopRadius,
    const std::uint8_t magnitude) {
    setCommon(record, 3, blend, start, duration);
    writeI16(record + 12, actorName);
    writeF32(record + 16, offsetX);
    writeF32(record + 20, offsetY);
    writeF32(record + 24, offsetZ);
    writeF32(record + 28, stopRadius);
    record[32] = magnitude;
}

void setButtons(std::uint8_t* record, const std::uint32_t start, const std::uint32_t duration,
    const std::uint16_t buttons) {
    setCommon(record, 4, 2, start, duration);
    writeU16(record + 12, buttons);
}

dusk::automation::InputControllerProgram decode(const std::vector<std::uint8_t>& bytes) {
    dusk::automation::InputControllerProgram program;
    REQUIRE(dusk::automation::decode_input_controller(bytes, program) ==
            dusk::automation::InputControllerError::None);
    return program;
}

void testBezierLayeringAndButtons() {
    using namespace dusk::automation;
    auto bytes = makeProgram(5, 4);
    setBezier(layer(bytes, 0), 0, 0, 5, {0, 0, 0, 0, 0, 0, 4, -4});
    setBezier(layer(bytes, 1), 1, 2, 2, {126, -126, 126, -126, 126, -126, 126, -126});
    setButtons(layer(bytes, 2), 1, 3, 0x0100);
    setButtons(layer(bytes, 3), 2, 1, 0x0020);

    const InputControllerProgram program = decode(bytes);
    REQUIRE(program.duration() == 5);
    REQUIRE(program.layerCount() == 4);
    REQUIRE(program.evaluate(0, {}).stickX == 0);
    REQUIRE(program.evaluate(0, {}).stickY == 0);
    // Exact midpoint is +/- 0.5 and therefore rounds away from zero.
    REQUIRE(program.evaluate(2, {}).stickX == 127);
    REQUIRE(program.evaluate(2, {}).stickY == -127);
    REQUIRE(program.evaluate(2, {}).buttons == 0x0120);
    REQUIRE(program.evaluate(4, {}).stickX == 4);
    REQUIRE(program.evaluate(4, {}).stickY == -4);
    REQUIRE(program.evaluate(5, {}) == RawPadState{});
}

void testPointSeekAndStopRadius() {
    using namespace dusk::automation;
    auto bytes = makeProgram(3, 1);
    setPoint(layer(bytes, 0), 0, 0, 3, 10.0F, 4.0F, 0.0F, 0.0F, 2.0F, 0.0F, 1.0F, 127);
    const InputControllerProgram program = decode(bytes);

    ControllerObservation observation{
        .playerPresent = true,
        .playerX = 0.0F,
        .playerY = 0.0F,
        .playerZ = 0.0F,
        .cameraPresent = true,
        .cameraYawRadians = 0.0F,
    };
    REQUIRE(program.evaluate(0, observation).stickX == 127);
    REQUIRE(program.evaluate(0, observation).stickY == 0);

    observation.playerX = 9.5F;
    REQUIRE(program.evaluate(0, observation).stickX == 0);
    REQUIRE(program.evaluate(0, observation).stickY == 0);
    observation.playerPresent = false;
    REQUIRE(program.evaluate(0, observation).stickX == 0);
}

void testActorSelectionIsStableAndPure() {
    using namespace dusk::automation;
    auto bytes = makeProgram(2, 1);
    setActor(layer(bytes, 0), 0, 0, 2, 42, 0.0F, 0.0F, 0.0F, 0.0F, 100);
    const InputControllerProgram program = decode(bytes);

    std::array<ControllerActor, 3> actors{{
        {.actorName = 42, .stableId = 9, .x = 10.0F},
        {.actorName = 7, .stableId = 1, .z = 1.0F},
        {.actorName = 42, .stableId = 3, .x = -10.0F},
    }};
    const auto before = actors;
    const ControllerObservation observation{
        .playerPresent = true,
        .cameraPresent = true,
        .actors = actors,
    };
    const RawPadState first = program.evaluate(0, observation);
    const RawPadState second = program.evaluate(0, observation);
    REQUIRE(first == second);
    REQUIRE(first.stickX == -100);
    REQUIRE(first.stickY == 0);
    REQUIRE(actors[0].actorName == before[0].actorName);
    REQUIRE(actors[0].stableId == before[0].stableId);
    REQUIRE(actors[0].x == before[0].x);
    REQUIRE(actors[1].actorName == before[1].actorName);
    REQUIRE(actors[2].stableId == before[2].stableId);
    REQUIRE(observation.playerPresent);
    REQUIRE(observation.cameraYawRadians == 0.0F);

    std::swap(actors[0], actors[2]);
    REQUIRE(program.evaluate(0, observation) == first);
}

void testStrictCanonicalValidation() {
    using namespace dusk::automation;
    InputControllerProgram output;

    auto valid = makeProgram(10, 1);
    setPoint(layer(valid, 0), 0, 0, 10, 0.0F, 0.0F, 10.0F, 0.0F, 0.0F, 0.0F, 0.0F, 127);
    REQUIRE(decode_input_controller(valid, output) == InputControllerError::None);

    auto bad = valid;
    bad[22] = 1;
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidReservedData);
    bad = valid;
    layer(bad, 0)[63] = 1;
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidUnusedData);
    bad = valid;
    writeF32(layer(bad, 0) + 12, std::numeric_limits<float>::quiet_NaN());
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidFloat);
    bad = valid;
    layer(bad, 0)[40] = 0;
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidMagnitude);
    bad = valid;
    bad.push_back(0);
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::TrailingData);

    bad = makeProgram(10, 1);
    setButtons(layer(bad, 0), 0, 10, 0);
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidButtonMask);

    bad = makeProgram(10, 2);
    setBezier(layer(bad, 0), 0, 0, 6, {});
    setPoint(layer(bad, 1), 0, 5, 5, 0.0F, 0.0F, 1.0F, 0.0F, 0.0F, 0.0F, 0.0F, 1);
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::OverlappingReplaceLayers);
}

void testMaximumDurationAndEmptyLayerSet() {
    using namespace dusk::automation;
    InputControllerProgram output;
    const auto maximum = makeProgram(kInputControllerMaximumDuration, 0);
    REQUIRE(decode_input_controller(maximum, output) == InputControllerError::None);
    REQUIRE(output.duration() == 1'000'000);
    REQUIRE(output.evaluate(999'999, {}) == RawPadState{});

    auto maximumBezier = makeProgram(kInputControllerMaximumDuration, 1);
    setBezier(layer(maximumBezier, 0), 0, 0, kInputControllerMaximumDuration,
        {-32'768, 32'767, -32'768, 32'767, -32'768, 32'767, -32'768, 32'767});
    REQUIRE(decode_input_controller(maximumBezier, output) == InputControllerError::None);
    // Exercise the portable two-limb exact Bernstein path at the format limit.
    REQUIRE(output.evaluate(500'000, {}).stickX == -128);
    REQUIRE(output.evaluate(500'000, {}).stickY == 127);
    REQUIRE(output.evaluate(999'999, {}).stickX == -128);
    REQUIRE(output.evaluate(999'999, {}).stickY == 127);

    const auto tooLong = makeProgram(kInputControllerMaximumDuration + 1, 0);
    REQUIRE(decode_input_controller(tooLong, output) == InputControllerError::InvalidDuration);
}

}  // namespace

int main() {
    testBezierLayeringAndButtons();
    testPointSeekAndStopRadius();
    testActorSelectionIsStableAndPure();
    testStrictCanonicalValidation();
    testMaximumDurationAndEmptyLayerSet();
    return 0;
}
