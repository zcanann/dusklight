#include "dusk/automation/input_controller.hpp"

#include <array>
#include <bit>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <limits>
#include <span>
#include <string>
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

void writeU64(std::uint8_t* output, const std::uint64_t value) {
    writeU32(output, static_cast<std::uint32_t>(value));
    writeU32(output + 4, static_cast<std::uint32_t>(value >> 32));
}

void writeF32(std::uint8_t* output, const float value) {
    writeU32(output, std::bit_cast<std::uint32_t>(value));
}

template <std::size_t Size>
std::array<char, 8> stageName(const char (&value)[Size]) {
    static_assert(Size >= 2 && Size <= 9);
    std::array<char, 8> result{};
    std::copy_n(value, Size - 1, result.begin());
    return result;
}

std::vector<std::uint8_t> makeProgram(const std::uint32_t duration, const std::uint16_t layers,
    const std::uint16_t minorVersion = dusk::automation::kInputControllerMinorVersion) {
    using namespace dusk::automation;
    std::vector<std::uint8_t> bytes(
        kInputControllerHeaderSize + layers * kInputControllerRecordSize);
    std::copy(kInputControllerMagic.begin(), kInputControllerMagic.end(), bytes.begin());
    writeU16(bytes.data() + 8, kInputControllerMajorVersion);
    writeU16(bytes.data() + 10, minorVersion);
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
    const float offsetY, const float offsetZ, const float stopRadius, const std::uint8_t magnitude,
    const std::uint8_t selector = 0, const std::int8_t homeRoom = 0,
    const std::uint32_t processId = 0, const std::uint16_t setId = 0,
    const std::array<char, 8>& placedStageName = {}) {
    setCommon(record, 3, blend, start, duration);
    writeI16(record + 12, actorName);
    record[14] = selector;
    record[15] = std::bit_cast<std::uint8_t>(homeRoom);
    writeF32(record + 16, offsetX);
    writeF32(record + 20, offsetY);
    writeF32(record + 24, offsetZ);
    writeF32(record + 28, stopRadius);
    record[32] = magnitude;
    writeU32(record + 33, processId);
    writeU16(record + 37, setId);
    std::copy(placedStageName.begin(), placedStageName.end(), record + 39);
}

void setCoordinate(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const std::uint8_t frame, const float x, const float y,
    const float z, const float offsetX, const float offsetY, const float offsetZ,
    const float stopRadius, const std::uint8_t magnitude) {
    setCommon(record, 5, blend, start, duration);
    record[12] = frame;
    writeF32(record + 16, x);
    writeF32(record + 20, y);
    writeF32(record + 24, z);
    writeF32(record + 28, offsetX);
    writeF32(record + 32, offsetY);
    writeF32(record + 36, offsetZ);
    writeF32(record + 40, stopRadius);
    record[44] = magnitude;
}

void setPlane(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const std::uint8_t frame, const float x, const float y,
    const float z, const float normalX, const float normalY, const float normalZ,
    const float stopRadius, const std::uint8_t magnitude) {
    setCoordinate(record, blend, start, duration, frame, x, y, z, normalX, normalY, normalZ,
        stopRadius, magnitude);
    record[0] = 6;
}

void setResolved(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const std::uint8_t kind, const std::uint64_t identity,
    const std::uint32_t subIndex, const float x, const float y, const float z,
    const float offsetX, const float offsetY, const float offsetZ, const float stopRadius,
    const std::uint8_t magnitude) {
    setCommon(record, 7, blend, start, duration);
    record[12] = kind;
    writeU64(record + 16, identity);
    writeU32(record + 24, subIndex);
    writeF32(record + 28, x);
    writeF32(record + 32, y);
    writeF32(record + 36, z);
    writeF32(record + 40, offsetX);
    writeF32(record + 44, offsetY);
    writeF32(record + 48, offsetZ);
    writeF32(record + 52, stopRadius);
    record[56] = magnitude;
}

void setNeutral(std::uint8_t* record, const std::uint32_t start, const std::uint32_t duration) {
    setCommon(record, 8, 0, start, duration);
}

void setTurn(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const std::uint8_t direction, const std::uint8_t magnitude) {
    setCommon(record, 9, blend, start, duration);
    record[12] = direction;
    record[13] = magnitude;
}

void setBrake(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const float stopSpeed, const std::uint8_t magnitude) {
    setCommon(record, 10, blend, start, duration);
    record[12] = magnitude;
    writeF32(record + 16, stopSpeed);
}

void setHeading(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const std::uint8_t mode, const std::uint8_t frame,
    const float heading, const float tolerance, const std::uint8_t magnitude) {
    setCommon(record, 11, blend, start, duration);
    record[12] = mode;
    record[13] = frame;
    record[14] = magnitude;
    writeF32(record + 16, heading);
    writeF32(record + 20, tolerance);
}

void setMaintainDistance(std::uint8_t* record, const std::uint8_t blend,
    const std::uint32_t start, const std::uint32_t duration, const std::uint8_t frame,
    const float x, const float y, const float z, const float distance, const float tolerance,
    const std::uint8_t magnitude) {
    setCommon(record, 12, blend, start, duration);
    record[12] = frame;
    record[13] = magnitude;
    writeF32(record + 16, x);
    writeF32(record + 20, y);
    writeF32(record + 24, z);
    writeF32(record + 28, distance);
    writeF32(record + 32, tolerance);
}

void setCamera(std::uint8_t* record, const std::uint8_t blend, const std::uint32_t start,
    const std::uint32_t duration, const std::int16_t x, const std::int16_t y) {
    setCommon(record, 13, blend, start, duration);
    writeI16(record + 12, x);
    writeI16(record + 14, y);
}

void setClamp(std::uint8_t* record, const std::uint32_t start, const std::uint32_t duration,
    const std::uint8_t mainLimit, const std::uint8_t substickLimit) {
    setCommon(record, 14, 0, start, duration);
    record[12] = mainLimit;
    record[13] = substickLimit;
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

std::uint8_t hexNibble(const char value) {
    if (value >= '0' && value <= '9') {
        return static_cast<std::uint8_t>(value - '0');
    }
    if (value >= 'a' && value <= 'f') {
        return static_cast<std::uint8_t>(value - 'a' + 10);
    }
    if (value >= 'A' && value <= 'F') {
        return static_cast<std::uint8_t>(value - 'A' + 10);
    }
    REQUIRE(false);
    return 0;
}

std::vector<std::uint8_t> readGoldenProgram() {
    std::ifstream input(DUSK_INPUT_CONTROLLER_GOLDEN_PATH);
    REQUIRE(input.good());
    std::string digits;
    for (char value; input.get(value);) {
        if (value != ' ' && value != '\t' && value != '\r' && value != '\n') {
            digits.push_back(value);
        }
    }
    REQUIRE(digits.size() % 2 == 0);
    std::vector<std::uint8_t> bytes;
    bytes.reserve(digits.size() / 2);
    for (std::size_t index = 0; index < digits.size(); index += 2) {
        bytes.push_back(static_cast<std::uint8_t>(
            (hexNibble(digits[index]) << 4) | hexNibble(digits[index + 1])));
    }
    return bytes;
}

void testRustGoldenMoveTargets() {
    using namespace dusk::automation;
    const InputControllerProgram program = decode(readGoldenProgram());
    REQUIRE(program.duration() == 10);
    REQUIRE(program.layerCount() == 13);
    REQUIRE(program.layers()[0].kind == InputControllerLayerKind::SeekCoordinate);
    REQUIRE(program.layers()[0].coordinateFrame == InputControllerCoordinateFrame::Player);
    REQUIRE(program.layers()[0].targetZ == 10.0F);
    REQUIRE(program.layers()[1].kind == InputControllerLayerKind::SeekPlane);
    REQUIRE(program.layers()[1].targetZ == 20.0F);
    REQUIRE(program.layers()[2].resolvedTarget == InputControllerResolvedTarget::PathPoint);
    REQUIRE(program.layers()[2].targetIdentity == 42);
    REQUIRE(program.layers()[2].targetSubIndex == 7);
    REQUIRE(program.layers()[3].resolvedTarget == InputControllerResolvedTarget::Opening);
    REQUIRE(program.layers()[3].targetIdentity == 99);
    REQUIRE(program.layers()[4].kind == InputControllerLayerKind::Neutral);
    REQUIRE(program.layers()[5].kind == InputControllerLayerKind::Turn);
    REQUIRE(program.layers()[6].kind == InputControllerLayerKind::Brake);
    REQUIRE(program.layers()[7].kind == InputControllerLayerKind::Heading);
    REQUIRE(program.layers()[7].headingMode == InputControllerHeadingMode::Align);
    REQUIRE(program.layers()[8].headingMode == InputControllerHeadingMode::Maintain);
    REQUIRE(program.layers()[9].kind == InputControllerLayerKind::MaintainDistance);
    REQUIRE(program.layers()[10].kind == InputControllerLayerKind::Camera);
    REQUIRE(program.layers()[11].kind == InputControllerLayerKind::Camera);
    REQUIRE(program.layers()[12].kind == InputControllerLayerKind::SafetyClamp);

    const ControllerObservation observation{
        .playerPresent = true,
        .playerYawPresent = true,
        .cameraPresent = true,
    };
    REQUIRE(program.evaluate(0, observation).stickX == -10);
    REQUIRE(program.evaluate(0, observation).stickY == 100);
    REQUIRE(program.evaluate(1, observation).stickX == 0);
    REQUIRE(program.evaluate(1, observation).stickY == 90);
    REQUIRE(program.evaluate(2, observation).stickX == -80);
    REQUIRE(program.evaluate(2, observation).stickY == 8);
    REQUIRE(program.evaluate(3, observation).stickX == 2);
    REQUIRE(program.evaluate(3, observation).stickY == 70);
    REQUIRE(program.evaluate(0, observation).substickX == 90);
    REQUIRE(program.evaluate(0, observation).substickY == -60);
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
    // Native PAD X is mirrored relative to world/camera right.
    REQUIRE(program.evaluate(0, observation).stickX == -127);
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
    REQUIRE(first.stickX == 100);
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

void testVersion10ActorCompatibility() {
    using namespace dusk::automation;
    auto bytes = makeProgram(2, 1, 0);
    setActor(layer(bytes, 0), 0, 0, 2, 42, 0.0F, 0.0F, 0.0F, 0.0F, 100);
    const InputControllerProgram program = decode(bytes);
    REQUIRE(program.layers()[0].actorSelector == InputControllerActorSelector::Nearest);

    const std::array<ControllerActor, 2> actors{{
        {.actorName = 42, .stableId = 9, .x = 10.0F},
        {.actorName = 42, .stableId = 3, .x = -2.0F},
    }};
    const ControllerObservation observation{
        .playerPresent = true,
        .cameraPresent = true,
        .actors = actors,
    };
    REQUIRE(program.evaluate(0, observation).stickX == 100);
    ControllerObservation missing{
        .playerPresent = true,
        .cameraPresent = true,
    };
    REQUIRE(program.evaluateDetailed(0, missing).terminalReason ==
            InputControllerTerminalReason::None);
}

void testExactProcessActorSelectionNeverFallsBack() {
    using namespace dusk::automation;
    auto bytes = makeProgram(2, 1);
    setActor(layer(bytes, 0), 0, 0, 2, 42, 0.0F, 0.0F, 0.0F, 0.0F, 100,
        static_cast<std::uint8_t>(InputControllerActorSelector::Process), 0, 9);
    const InputControllerProgram program = decode(bytes);
    REQUIRE(program.layers()[0].processId == 9);

    std::array<ControllerActor, 2> actors{{
        {.actorName = 42, .stableId = 3, .x = -2.0F},
        {.actorName = 42, .stableId = 9, .x = 10.0F},
    }};
    const ControllerObservation observation{
        .playerPresent = true,
        .cameraPresent = true,
        .actors = actors,
    };
    REQUIRE(program.evaluate(0, observation).stickX == -100);

    // The exact ID still requires the declared actor type and cannot fall back
    // to the nearer actor when that guard fails.
    actors[1].actorName = 7;
    REQUIRE(program.evaluate(0, observation).stickX == 0);
    REQUIRE(program.evaluate(0, observation).stickY == 0);
    const InputControllerEvaluation lost = program.evaluateDetailed(0, observation);
    REQUIRE(lost.terminalReason == InputControllerTerminalReason::TargetLost);
    REQUIRE(lost.terminalLayer == 0);
    REQUIRE(lost.input == RawPadState{});

    ControllerObservation incomplete = observation;
    incomplete.actorsTruncated = true;
    const InputControllerEvaluation unknown = program.evaluateDetailed(0, incomplete);
    REQUIRE(unknown.terminalReason == InputControllerTerminalReason::None);
    REQUIRE(unknown.input == RawPadState{});
}

void testPlacedActorSelectionRequiresExactStageAndPlacement() {
    using namespace dusk::automation;
    auto bytes = makeProgram(2, 1);
    setActor(layer(bytes, 0), 0, 0, 2, 42, 0.0F, 0.0F, 0.0F, 0.0F, 100,
        static_cast<std::uint8_t>(InputControllerActorSelector::Placed), -3, 0,
        std::numeric_limits<std::uint16_t>::max(), stageName("F_SP103"));
    const InputControllerProgram program = decode(bytes);
    REQUIRE(program.layers()[0].setId == std::numeric_limits<std::uint16_t>::max());
    REQUIRE(program.layers()[0].homeRoom == -3);
    REQUIRE(program.layers()[0].placedStageName == stageName("F_SP103"));

    std::array<ControllerActor, 4> actors{{
        {.actorName = 42,
            .stableId = 9,
            .setId = std::numeric_limits<std::uint16_t>::max(),
            .homeRoom = -3,
            .x = 10.0F},
        {.actorName = 42,
            .stableId = 3,
            .setId = std::numeric_limits<std::uint16_t>::max(),
            .homeRoom = -3,
            .x = -10.0F},
        {.actorName = 42, .stableId = 1, .setId = 8, .homeRoom = -3, .x = 1.0F},
        {.actorName = 7, .stableId = 2, .setId = 7, .homeRoom = -3, .z = 1.0F},
    }};
    ControllerObservation observation{
        .playerPresent = true,
        .cameraPresent = true,
        .stageName = stageName("F_SP103"),
        .actors = actors,
    };
    REQUIRE(program.evaluate(0, observation).stickX == 100);

    // A placed selector is the full stage/type/set/room tuple. It never
    // falls back to a nearby actor when any part of that tuple differs.
    observation.stageName = stageName("F_SP104");
    REQUIRE(program.evaluate(0, observation) == RawPadState{});
    observation.stageName = stageName("F_SP103");
    actors[0].setId = 8;
    actors[1].setId = 8;
    REQUIRE(program.evaluate(0, observation) == RawPadState{});
    actors[0].setId = std::numeric_limits<std::uint16_t>::max();
    actors[1].setId = std::numeric_limits<std::uint16_t>::max();
    actors[0].homeRoom = -2;
    actors[1].homeRoom = -2;
    REQUIRE(program.evaluate(0, observation) == RawPadState{});
    actors[0].homeRoom = -3;
    actors[1].homeRoom = -3;
    actors[0].actorName = 7;
    actors[1].actorName = 7;
    REQUIRE(program.evaluate(0, observation) == RawPadState{});
}

void testFramedCoordinatesPlanesAndResolvedTargets() {
    using namespace dusk::automation;
    constexpr float HalfPi = 1.57079632679489661923F;
    auto bytes = makeProgram(6, 6);
    setCoordinate(layer(bytes, 0), 0, 0, 1, 0, 0.0F, 0.0F, 10.0F, 0.0F, 0.0F,
        0.0F, 0.0F, 100);
    setCoordinate(layer(bytes, 1), 0, 1, 1, 1, 0.0F, 0.0F, 10.0F, 0.0F, 0.0F,
        0.0F, 0.0F, 90);
    setCoordinate(layer(bytes, 2), 0, 2, 1, 2, 0.0F, 0.0F, 10.0F, 0.0F, 0.0F,
        0.0F, 0.0F, 80);
    setPlane(layer(bytes, 3), 0, 3, 1, 0, 0.0F, 0.0F, 10.0F, 0.0F, 0.0F, 2.0F,
        0.0F, 70);
    setResolved(layer(bytes, 4), 0, 4, 1, 0, 42, 7, 10.0F, 0.0F, 0.0F, 0.0F,
        0.0F, 0.0F, 0.0F, 60);
    setResolved(layer(bytes, 5), 0, 5, 1, 1, 99, 0, 0.0F, 0.0F, 10.0F, 0.0F,
        0.0F, 0.0F, 0.0F, 50);
    const InputControllerProgram program = decode(bytes);
    REQUIRE(program.layers()[1].coordinateFrame == InputControllerCoordinateFrame::Player);
    REQUIRE(program.layers()[4].resolvedTarget == InputControllerResolvedTarget::PathPoint);
    REQUIRE(program.layers()[4].targetIdentity == 42);
    REQUIRE(program.layers()[4].targetSubIndex == 7);
    REQUIRE(program.layers()[5].resolvedTarget == InputControllerResolvedTarget::Opening);
    REQUIRE(program.layers()[5].targetIdentity == 99);

    ControllerObservation observation{
        .playerPresent = true,
        .playerX = 0.0F,
        .playerY = 0.0F,
        .playerZ = 0.0F,
        .playerYawPresent = true,
        .playerYawRadians = HalfPi,
        .cameraPresent = true,
        .cameraYawRadians = 0.0F,
    };
    REQUIRE(program.evaluate(0, observation).stickX == 0);
    REQUIRE(program.evaluate(0, observation).stickY == 100);
    REQUIRE(program.evaluate(1, observation).stickX == -90);
    REQUIRE(program.evaluate(1, observation).stickY == 0);

    observation.cameraYawRadians = HalfPi;
    REQUIRE(program.evaluate(2, observation).stickX == 0);
    REQUIRE(program.evaluate(2, observation).stickY == 80);
    observation.cameraYawRadians = 0.0F;
    REQUIRE(program.evaluate(3, observation).stickX == 0);
    REQUIRE(program.evaluate(3, observation).stickY == 70);
    REQUIRE(program.evaluate(4, observation).stickX == -60);
    REQUIRE(program.evaluate(4, observation).stickY == 0);
    REQUIRE(program.evaluate(5, observation).stickX == 0);
    REQUIRE(program.evaluate(5, observation).stickY == 50);

    observation.playerYawPresent = false;
    REQUIRE(program.evaluate(1, observation) == RawPadState{});
    observation.playerYawPresent = true;
    observation.cameraPresent = false;
    REQUIRE(program.evaluate(2, observation) == RawPadState{});
}

void testVersion12TargetValidation() {
    using namespace dusk::automation;
    InputControllerProgram output;
    auto bytes = makeProgram(1, 1);
    setCoordinate(layer(bytes, 0), 0, 0, 1, 3, 0.0F, 0.0F, 1.0F, 0.0F, 0.0F,
        0.0F, 0.0F, 1);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidCoordinateFrame);

    bytes = makeProgram(1, 1);
    setPlane(layer(bytes, 0), 0, 0, 1, 0, 0.0F, 0.0F, 1.0F, 0.0F, 1.0F, 0.0F,
        0.0F, 1);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidPlaneNormal);

    bytes = makeProgram(1, 1);
    setCoordinate(layer(bytes, 0), 0, 0, 1, 0, 0.0F, 0.0F, 1.0F, 0.0F, 0.0F,
        0.0F, -1.0F, 1);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidStopRadius);
    setCoordinate(layer(bytes, 0), 0, 0, 1, 0, 0.0F, 0.0F, 1.0F, 0.0F, 0.0F,
        0.0F, 0.0F, 0);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidMagnitude);

    bytes = makeProgram(1, 1);
    setResolved(layer(bytes, 0), 0, 0, 1, 1, 0, 0, 0.0F, 0.0F, 1.0F, 0.0F, 0.0F,
        0.0F, 0.0F, 1);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidResolvedTarget);
    setResolved(layer(bytes, 0), 0, 0, 1, 1, 9, 1, 0.0F, 0.0F, 1.0F, 0.0F,
        0.0F, 0.0F, 0.0F, 1);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidResolvedTarget);

    bytes = makeProgram(1, 1, 1);
    setCoordinate(layer(bytes, 0), 0, 0, 1, 0, 0.0F, 0.0F, 1.0F, 0.0F, 0.0F,
        0.0F, 0.0F, 1);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidLayerKind);
}

void testVersion13MotionControls() {
    using namespace dusk::automation;
    constexpr float HalfPi = 1.57079632679489661923F;
    auto bytes = makeProgram(6, 6);
    setNeutral(layer(bytes, 0), 0, 1);
    setTurn(layer(bytes, 1), 0, 1, 1, 0, 40);
    setBrake(layer(bytes, 2), 0, 2, 1, 0.5F, 50);
    setHeading(layer(bytes, 3), 0, 3, 1, 0, 0, HalfPi, 0.1F, 60);
    setHeading(layer(bytes, 4), 0, 4, 1, 1, 2, HalfPi, 0.0F, 70);
    setMaintainDistance(layer(bytes, 5), 0, 5, 1, 0, 0.0F, 0.0F, 10.0F, 5.0F,
        1.0F, 80);
    const InputControllerProgram program = decode(bytes);

    ControllerObservation observation{
        .playerPresent = true,
        .playerYawPresent = true,
        .playerVelocityPresent = true,
        .playerVelocityZ = 5.0F,
        .cameraPresent = true,
    };
    REQUIRE(program.evaluate(0, observation) == RawPadState{});
    REQUIRE(program.evaluate(1, observation).stickX == -40);
    REQUIRE(program.evaluate(1, observation).stickY == 0);
    REQUIRE(program.evaluate(2, observation).stickX == 0);
    REQUIRE(program.evaluate(2, observation).stickY == -50);
    REQUIRE(program.evaluate(3, observation).stickX == -60);
    REQUIRE(program.evaluate(3, observation).stickY == 0);
    observation.playerYawRadians = HalfPi;
    REQUIRE(program.evaluate(3, observation) == RawPadState{});
    observation.playerYawRadians = 0.0F;
    REQUIRE(program.evaluate(4, observation).stickX == -70);
    REQUIRE(program.evaluate(4, observation).stickY == 0);
    REQUIRE(program.evaluate(5, observation).stickX == 0);
    REQUIRE(program.evaluate(5, observation).stickY == 80);
    observation.playerZ = 5.0F;
    REQUIRE(program.evaluate(5, observation) == RawPadState{});
    observation.playerZ = 9.0F;
    REQUIRE(program.evaluate(5, observation).stickX == 0);
    REQUIRE(program.evaluate(5, observation).stickY == -80);

    observation.playerVelocityZ = 0.25F;
    REQUIRE(program.evaluate(2, observation) == RawPadState{});
    observation.playerVelocityPresent = false;
    REQUIRE(program.evaluate(2, observation) == RawPadState{});
}

void testVersion13MotionValidation() {
    using namespace dusk::automation;
    InputControllerProgram output;
    auto bytes = makeProgram(1, 1, 2);
    setNeutral(layer(bytes, 0), 0, 1);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidLayerKind);

    bytes = makeProgram(1, 1);
    setTurn(layer(bytes, 0), 0, 0, 1, 2, 10);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidMotionControl);
    setTurn(layer(bytes, 0), 0, 0, 1, 0, 0);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidMagnitude);

    bytes = makeProgram(1, 1);
    setHeading(layer(bytes, 0), 0, 0, 1, 0, 0, 4.0F, 0.1F, 10);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidHeading);
    setHeading(layer(bytes, 0), 0, 0, 1, 1, 0, 0.0F, 0.1F, 10);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidHeading);

    bytes = makeProgram(1, 1);
    setMaintainDistance(
        layer(bytes, 0), 0, 0, 1, 0, 0.0F, 0.0F, 1.0F, 1.0F, 2.0F, 10);
    REQUIRE(decode_input_controller(bytes, output) == InputControllerError::InvalidDistance);
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

    bad = makeProgram(10, 1);
    setActor(layer(bad, 0), 0, 0, 10, 42, 0.0F, 0.0F, 0.0F, 0.0F, 1, 3);
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidActorSelector);

    bad = makeProgram(10, 1);
    setActor(layer(bad, 0), 0, 0, 10, 42, 0.0F, 0.0F, 0.0F, 0.0F, 1,
        static_cast<std::uint8_t>(InputControllerActorSelector::Process));
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidProcessId);
    writeU32(layer(bad, 0) + 33, std::numeric_limits<std::uint32_t>::max());
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidProcessId);

    bad = makeProgram(10, 1);
    setActor(layer(bad, 0), 0, 0, 10, 42, 0.0F, 0.0F, 0.0F, 0.0F, 1,
        static_cast<std::uint8_t>(InputControllerActorSelector::Placed), -1, 0,
        std::numeric_limits<std::uint16_t>::max(), stageName("F_SP103"));
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::None);
    REQUIRE(output.layers()[0].setId == std::numeric_limits<std::uint16_t>::max());

    bad = makeProgram(10, 1);
    setActor(layer(bad, 0), 0, 0, 10, 42, 0.0F, 0.0F, 0.0F, 0.0F, 1,
        static_cast<std::uint8_t>(InputControllerActorSelector::Placed), -1, 0, 7);
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidStageName);
    const auto validStageName = stageName("F_SP103");
    std::copy(validStageName.begin(), validStageName.end(), layer(bad, 0) + 39);
    layer(bad, 0)[41] = 0;
    layer(bad, 0)[42] = 'X';
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidStageName);
    std::copy(validStageName.begin(), validStageName.end(), layer(bad, 0) + 39);
    layer(bad, 0)[39] = 0x80;
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidStageName);

    bad = makeProgram(10, 1);
    setActor(layer(bad, 0), 0, 0, 10, 42, 0.0F, 0.0F, 0.0F, 0.0F, 1,
        static_cast<std::uint8_t>(InputControllerActorSelector::Placed), -1, 0, 7,
        stageName("F_SP103"));
    layer(bad, 0)[47] = 1;
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidUnusedData);

    bad = makeProgram(10, 1);
    setActor(layer(bad, 0), 0, 0, 10, 42, 0.0F, 0.0F, 0.0F, 0.0F, 1);
    layer(bad, 0)[39] = 'X';
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidUnusedData);

    bad = makeProgram(10, 1);
    setActor(layer(bad, 0), 0, 0, 10, 42, 0.0F, 0.0F, 0.0F, 0.0F, 1,
        static_cast<std::uint8_t>(InputControllerActorSelector::Process), 0, 9);
    layer(bad, 0)[39] = 'X';
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidUnusedData);

    bad = makeProgram(10, 1, 0);
    setActor(layer(bad, 0), 0, 0, 10, 42, 0.0F, 0.0F, 0.0F, 0.0F, 1,
        static_cast<std::uint8_t>(InputControllerActorSelector::Process), 0, 1);
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::InvalidReservedData);

    bad = makeProgram(10, 0);
    writeU16(bad.data() + 10, kInputControllerMinorVersion + 1);
    REQUIRE(decode_input_controller(bad, output) == InputControllerError::UnsupportedVersion);
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

void testVersion14CompositionSurfacesAndClamps() {
    using namespace dusk::automation;
    auto bytes = makeProgram(2, 6);
    setBezier(layer(bytes, 0), 0, 0, 2, {100, 100, 100, 100, 100, 100, 100, 100});
    setBezier(layer(bytes, 1), 1, 0, 2, {50, -250, 50, -250, 50, -250, 50, -250});
    setCamera(layer(bytes, 2), 0, 0, 2, 100, -100);
    setCamera(layer(bytes, 3), 1, 0, 2, 40, 40);
    setButtons(layer(bytes, 4), 0, 2, 0x0200);
    setClamp(layer(bytes, 5), 0, 2, 90, 80);
    const InputControllerProgram program = decode(bytes);
    const RawPadState output = program.evaluate(0, {});
    REQUIRE(output.stickX == 90);
    REQUIRE(output.stickY == -90);
    REQUIRE(output.substickX == 80);
    REQUIRE(output.substickY == -60);
    REQUIRE(output.buttons == 0x0200);

    auto reordered = makeProgram(2, 6);
    setClamp(layer(reordered, 0), 0, 2, 90, 80);
    setButtons(layer(reordered, 1), 0, 2, 0x0200);
    setCamera(layer(reordered, 2), 1, 0, 2, 40, 40);
    setBezier(layer(reordered, 3), 1, 0, 2, {50, -250, 50, -250, 50, -250, 50, -250});
    setCamera(layer(reordered, 4), 0, 0, 2, 100, -100);
    setBezier(layer(reordered, 5), 0, 0, 2, {100, 100, 100, 100, 100, 100, 100, 100});
    REQUIRE(decode(reordered).evaluate(0, {}) == output);

    auto cameraOverlap = makeProgram(2, 2);
    setCamera(layer(cameraOverlap, 0), 0, 0, 2, 1, 1);
    setCamera(layer(cameraOverlap, 1), 0, 1, 1, 2, 2);
    InputControllerProgram rejected;
    REQUIRE(decode_input_controller(cameraOverlap, rejected) ==
            InputControllerError::OverlappingReplaceLayers);

    auto clampOverlap = makeProgram(2, 2);
    setClamp(layer(clampOverlap, 0), 0, 2, 10, 10);
    setClamp(layer(clampOverlap, 1), 1, 1, 20, 20);
    REQUIRE(decode_input_controller(clampOverlap, rejected) ==
            InputControllerError::OverlappingSafetyClamps);

    auto oldVersion = makeProgram(1, 1, 3);
    setCamera(layer(oldVersion, 0), 0, 0, 1, 1, 1);
    REQUIRE(decode_input_controller(oldVersion, rejected) == InputControllerError::InvalidLayerKind);
}

void testVersionedPreInputStepContract() {
    using namespace dusk::automation;
    auto bytes = makeProgram(2, 1);
    setBezier(layer(bytes, 0), 0, 0, 2, {12, -34, 12, -34, 12, -34, 12, -34});
    const InputControllerProgram program = decode(bytes);
    const InputControllerStepRequest request{
        .majorVersion = kInputControllerStepMajorVersion,
        .minorVersion = kInputControllerStepMinorVersion,
        .phase = InputControllerObservationPhase::PreInput,
        .simulationTick = 77,
        .inputFrame = 41,
        .controllerFrame = 0,
        .facts = build_typed_fact_response(ControllerObservation{}, TypedFactPhase::PreInput, 77, 41),
        .observation = {},
    };
    const InputControllerStepResponse response = program.respond(request);
    REQUIRE(response.error == InputControllerStepError::None);
    REQUIRE(response.majorVersion == request.majorVersion);
    REQUIRE(response.minorVersion == request.minorVersion);
    REQUIRE(response.simulationTick == request.simulationTick);
    REQUIRE(response.inputFrame == request.inputFrame);
    REQUIRE(response.controllerFrame == request.controllerFrame);
    REQUIRE(response.evaluation.input.stickX == 12);
    REQUIRE(response.evaluation.input.stickY == -34);
    REQUIRE(program.respond(request).evaluation.input == response.evaluation.input);

    auto seekBytes = makeProgram(1, 1);
    setPoint(layer(seekBytes, 0), 0, 0, 1, 10.0F, 0.0F, 0.0F, 0.0F, 0.0F, 0.0F,
        0.5F, 100);
    const InputControllerProgram seekProgram = decode(seekBytes);
    ControllerObservation rawObservation{
        .playerPresent = true,
        .cameraPresent = true,
    };
    ControllerObservation factObservation = rawObservation;
    factObservation.playerX = 10.0F;
    InputControllerStepRequest factRequest = request;
    factRequest.facts =
        build_typed_fact_response(factObservation, TypedFactPhase::PreInput, 77, 41);
    factRequest.observation = rawObservation;
    REQUIRE(seekProgram.evaluate(0, rawObservation).stickX != 0);
    REQUIRE(seekProgram.respond(factRequest).evaluation.input.stickX == 0);

    InputControllerStepRequest invalid = request;
    ++invalid.minorVersion;
    REQUIRE(program.respond(invalid).error == InputControllerStepError::UnsupportedVersion);
    REQUIRE(program.respond(invalid).evaluation.input == RawPadState{});
    invalid = request;
    invalid.phase = static_cast<InputControllerObservationPhase>(0);
    REQUIRE(program.respond(invalid).error == InputControllerStepError::InvalidPhase);
    invalid = request;
    invalid.facts.phase = TypedFactPhase::PostSimulation;
    REQUIRE(program.respond(invalid).error == InputControllerStepError::InvalidFacts);
    invalid = request;
    invalid.facts.count = 0;
    REQUIRE(program.respond(invalid).error == InputControllerStepError::InvalidFacts);
    invalid = request;
    invalid.controllerFrame = program.duration();
    REQUIRE(program.respond(invalid).error == InputControllerStepError::InvalidFrame);
}

}  // namespace

int main() {
    testRustGoldenMoveTargets();
    testBezierLayeringAndButtons();
    testPointSeekAndStopRadius();
    testActorSelectionIsStableAndPure();
    testVersion10ActorCompatibility();
    testExactProcessActorSelectionNeverFallsBack();
    testPlacedActorSelectionRequiresExactStageAndPlacement();
    testFramedCoordinatesPlanesAndResolvedTargets();
    testVersion12TargetValidation();
    testVersion13MotionControls();
    testVersion13MotionValidation();
    testStrictCanonicalValidation();
    testMaximumDurationAndEmptyLayerSet();
    testVersion14CompositionSurfacesAndClamps();
    testVersionedPreInputStepContract();
    return 0;
}
