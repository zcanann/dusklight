#include "dusk/automation/file_select_observer.hpp"
#include "dusk/automation/input_tape.hpp"
#include "dusk/automation/name_entry_observer.hpp"
#include "dusk/automation/scenario_fixture.hpp"

#include <dolphin/pad.h>
#include <zstd.h>

#include <algorithm>
#include <array>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <limits>
#include <string_view>
#include <thread>
#include <utility>
#include <vector>

namespace {

std::array<PADStatus, PAD_CHANMAX> gStatuses{};
std::array<bool, PAD_CHANMAX> gActive{};
std::array<unsigned, PAD_CHANMAX> gSetCalls{};
std::array<unsigned, PAD_CHANMAX> gClearCalls{};

void require(bool condition, const char* expression, int line) {
    if (!condition) {
        std::cerr << "input_tape_test.cpp:" << line << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

void resetPadSpies() {
    gStatuses = {};
    gActive = {};
    gSetCalls = {};
    gClearCalls = {};
}

std::uint64_t readU64(const std::uint8_t* input) {
    std::uint64_t value = 0;
    for (unsigned byte = 0; byte < 8; ++byte) {
        value |= static_cast<std::uint64_t>(input[byte]) << (byte * 8);
    }
    return value;
}

void writeU16(std::uint8_t* output, const std::uint16_t value) {
    output[0] = static_cast<std::uint8_t>(value);
    output[1] = static_cast<std::uint8_t>(value >> 8);
}

void writeU32(std::uint8_t* output, const std::uint32_t value) {
    for (unsigned byte = 0; byte < 4; ++byte) {
        output[byte] = static_cast<std::uint8_t>(value >> (byte * 8));
    }
}

void writeU64(std::uint8_t* output, const std::uint64_t value) {
    for (unsigned byte = 0; byte < 8; ++byte) {
        output[byte] = static_cast<std::uint8_t>(value >> (byte * 8));
    }
}

std::vector<std::uint8_t> expandV2(const std::vector<std::uint8_t>& bytes) {
    using namespace dusk::automation;

    REQUIRE(bytes.size() >= kInputTapeHeaderSize);
    const std::size_t frameCount = static_cast<std::size_t>(readU64(bytes.data() + 24));
    std::vector<std::uint8_t> expanded(frameCount * kInputFrameSize);
    std::uint8_t empty = 0;
    void* destination = expanded.empty() ? static_cast<void*>(&empty) : expanded.data();
    const std::size_t result = ZSTD_decompress(destination, expanded.size(),
        bytes.data() + kInputTapeHeaderSize, bytes.size() - kInputTapeHeaderSize);
    REQUIRE(!ZSTD_isError(result));
    REQUIRE(result == expanded.size());
    return expanded;
}

std::vector<std::uint8_t> replaceV2Payload(
    std::vector<std::uint8_t> bytes, const std::vector<std::uint8_t>& expanded) {
    using namespace dusk::automation;

    std::vector<std::uint8_t> compressed(ZSTD_compressBound(expanded.size()));
    const std::uint8_t empty = 0;
    const void* source = expanded.empty() ? static_cast<const void*>(&empty) : expanded.data();
    const std::size_t result = ZSTD_compress(
        compressed.data(), compressed.size(), source, expanded.size(), ZSTD_CLEVEL_DEFAULT);
    REQUIRE(!ZSTD_isError(result));
    compressed.resize(result);
    bytes.resize(kInputTapeHeaderSize + compressed.size());
    writeU64(bytes.data() + 32, compressed.size());
    std::copy(compressed.begin(), compressed.end(), bytes.begin() + kInputTapeHeaderSize);
    return bytes;
}

std::vector<std::uint8_t> makeLegacyV1(
    const dusk::automation::InputTape& tape, const std::uint16_t minorVersion) {
    using namespace dusk::automation;

    std::vector<std::uint8_t> v2;
    REQUIRE(encode_input_tape(tape, v2) == InputTapeError::None);
    std::vector<std::uint8_t> frames = expandV2(v2);
    constexpr std::size_t legacyHeaderSize = 32;
    std::vector<std::uint8_t> legacy(legacyHeaderSize + frames.size(), 0);
    std::copy(kInputTapeMagic.begin(), kInputTapeMagic.end(), legacy.begin());
    writeU16(legacy.data() + 8, 1);
    writeU16(legacy.data() + 10, minorVersion);
    writeU16(legacy.data() + 12, legacyHeaderSize);
    writeU16(legacy.data() + 14, kInputFrameSize);
    writeU32(legacy.data() + 16, tape.tickRateNumerator);
    writeU32(legacy.data() + 20, tape.tickRateDenominator);
    writeU64(legacy.data() + 24, tape.frames.size());
    std::copy(frames.begin(), frames.end(), legacy.begin() + legacyHeaderSize);
    return legacy;
}

void testCanonicalRoundTrip() {
    using namespace dusk::automation;

    InputTape tape;
    tape.tickRateNumerator = 60;
    tape.tickRateDenominator = 2;
    tape.frames.resize(1);
    tape.frames[0].ownedPorts = 0b0101;
    tape.frames[0].pads[0] = {
        .buttons = 0x1234,
        .stickX = -127,
        .stickY = 126,
        .substickX = -3,
        .substickY = 4,
        .triggerLeft = 5,
        .triggerRight = 6,
        .analogA = 7,
        .analogB = 8,
        .flags = RawPadFlags::Connected,
    };
    tape.frames[0].pads[2].flags = RawPadFlags::None;
    tape.frames[0].pads[2].error = PAD_ERR_NO_CONTROLLER;

    std::vector<std::uint8_t> bytes;
    REQUIRE(encode_input_tape(tape, bytes) == InputTapeError::None);
    REQUIRE(bytes.size() > kInputTapeHeaderSize);
    REQUIRE(std::equal(kInputTapeMagic.begin(), kInputTapeMagic.end(), bytes.begin()));
    REQUIRE(bytes[8] == kInputTapeMajorVersion && bytes[9] == 0);
    REQUIRE(bytes[10] == kInputTapeMinorVersion && bytes[11] == 0);
    REQUIRE(bytes[12] == kInputTapeHeaderSize && bytes[13] == 0);
    REQUIRE(bytes[14] == kInputFrameSize && bytes[15] == 0);
    REQUIRE(bytes[16] == 60 && bytes[20] == 2);
    REQUIRE(bytes[24] == 1);
    REQUIRE(readU64(bytes.data() + 32) == bytes.size() - kInputTapeHeaderSize);

    const std::vector<std::uint8_t> expanded = expandV2(bytes);
    const std::size_t pad0 = 4;
    REQUIRE(expanded[0] == 0b0101);
    REQUIRE(expanded[pad0] == 0x34 && expanded[pad0 + 1] == 0x12);
    REQUIRE(expanded[pad0 + 2] == 0x81);
    REQUIRE(expanded[pad0 + 10] == static_cast<std::uint8_t>(RawPadFlags::Connected));
    REQUIRE(expanded[pad0 + 11] == 0);

    InputTape decoded;
    REQUIRE(decode_input_tape(bytes, decoded) == InputTapeError::None);
    REQUIRE(decoded == tape);
}

void testFullPadSurfaceOnAllFourPorts() {
    using namespace dusk::automation;

    InputTape tape;
    tape.frames.resize(1);
    InputFrame& frame = tape.frames[0];
    frame.ownedPorts = 0x0f;
    for (std::size_t port = 0; port < kInputPortCount; ++port) {
        frame.pads[port] = {
            .buttons = static_cast<std::uint16_t>(0xffffu - port),
            .stickX = static_cast<std::int8_t>(-128 + port),
            .stickY = static_cast<std::int8_t>(127 - port),
            .substickX = static_cast<std::int8_t>(-64 + port),
            .substickY = static_cast<std::int8_t>(64 - port),
            .triggerLeft = static_cast<std::uint8_t>(255 - port),
            .triggerRight = static_cast<std::uint8_t>(port),
            .analogA = static_cast<std::uint8_t>(17 + port),
            .analogB = static_cast<std::uint8_t>(239 - port),
            .flags = port == 3 ? RawPadFlags::None : RawPadFlags::Connected,
            .error = static_cast<std::int8_t>(port == 3 ? PAD_ERR_NO_CONTROLLER : port),
        };
    }

    std::vector<std::uint8_t> encoded;
    REQUIRE(encode_input_tape(tape, encoded) == InputTapeError::None);
    InputTape decoded;
    REQUIRE(decode_input_tape(encoded, decoded) == InputTapeError::None);
    REQUIRE(decoded == tape);
    REQUIRE(decoded.frames[0].ownedPorts == 0x0f);

    for (std::size_t port = 0; port < kInputPortCount; ++port) {
        const PADStatus native = raw_pad_state_to_pad_status(decoded.frames[0].pads[port]);
        REQUIRE(native.button == frame.pads[port].buttons);
        REQUIRE(native.stickX == frame.pads[port].stickX);
        REQUIRE(native.stickY == frame.pads[port].stickY);
        REQUIRE(native.substickX == frame.pads[port].substickX);
        REQUIRE(native.substickY == frame.pads[port].substickY);
        REQUIRE(native.triggerLeft == frame.pads[port].triggerLeft);
        REQUIRE(native.triggerRight == frame.pads[port].triggerRight);
        REQUIRE(native.analogA == frame.pads[port].analogA);
        REQUIRE(native.analogB == frame.pads[port].analogB);
        REQUIRE(native.err == frame.pads[port].error);
    }
}

void testStageBootAndCompressedV2Compatibility() {
    using namespace dusk::automation;

    InputTape stageTape;
    stageTape.boot = {
        .kind = TapeBootKind::Stage,
        .stage = "F_SP103",
        .room = 1,
        .point = 257,
        .layer = -1,
        .saveSlot = 2,
    };
    stageTape.frames.resize(1);
    std::vector<std::uint8_t> stageBytes;
    REQUIRE(encode_input_tape(stageTape, stageBytes) == InputTapeError::None);
    InputTape decoded;
    REQUIRE(decode_input_tape(stageBytes, decoded) == InputTapeError::None);
    REQUIRE(decoded.boot == stageTape.boot);

    std::vector<std::uint8_t> v30 = stageBytes;
    writeU16(v30.data() + 10, 0);
    v30[46] = 0;
    REQUIRE(decode_input_tape(v30, decoded) == InputTapeError::None);
    REQUIRE(decoded.boot.kind == TapeBootKind::Stage);
    REQUIRE(decoded.boot.saveSlot == 0);

    InputTape processTape;
    processTape.frames.resize(1);
    std::vector<std::uint8_t> v3;
    REQUIRE(encode_input_tape(processTape, v3) == InputTapeError::None);
    constexpr std::size_t v2HeaderSize = 40;
    std::vector<std::uint8_t> v2;
    v2.insert(v2.end(), v3.begin(), v3.begin() + v2HeaderSize);
    v2.insert(v2.end(), v3.begin() + kInputTapeHeaderSize, v3.end());
    writeU16(v2.data() + 8, 2);
    writeU16(v2.data() + 10, 0);
    writeU16(v2.data() + 12, v2HeaderSize);
    REQUIRE(decode_input_tape(v2, decoded) == InputTapeError::None);
    REQUIRE(decoded.boot.kind == TapeBootKind::Process);

    stageTape.boot.stage.clear();
    REQUIRE(encode_input_tape(stageTape, stageBytes) == InputTapeError::InvalidBoot);
    stageTape.boot.stage = "F_SP103";
    stageTape.boot.saveSlot = 4;
    REQUIRE(encode_input_tape(stageTape, stageBytes) == InputTapeError::InvalidBoot);
}

void testMalformedTapesAreRejected() {
    using namespace dusk::automation;

    InputTape tape;
    tape.frames.resize(1);
    std::vector<std::uint8_t> bytes;
    REQUIRE(encode_input_tape(tape, bytes) == InputTapeError::None);

    std::vector<std::uint8_t> malformed = bytes;
    malformed[0] ^= 0xff;
    InputTape decoded;
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::BadMagic);

    malformed = bytes;
    malformed[8] = 4;
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::UnsupportedVersion);

    malformed = bytes;
    malformed.push_back(0);
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::TrailingData);

    malformed = bytes;
    malformed.pop_back();
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::Truncated);

    malformed = bytes;
    writeU64(malformed.data() + 32, readU64(malformed.data() + 32) - 1);
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::TrailingData);

    malformed = bytes;
    writeU64(malformed.data() + 32, readU64(malformed.data() + 32) + 1);
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::Truncated);

    malformed = bytes;
    malformed[kInputTapeHeaderSize] ^= 0xff;
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::InvalidCompressedPayload);

    malformed = bytes;
    writeU64(malformed.data() + 24, 2);
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::InvalidCompressedPayload);

    malformed = bytes;
    writeU64(malformed.data() + 24, std::numeric_limits<std::uint64_t>::max());
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::TooManyFrames);

    malformed = bytes;
    const std::vector<std::uint8_t> secondFrame(
        malformed.begin() + kInputTapeHeaderSize, malformed.end());
    malformed.insert(malformed.end(), secondFrame.begin(), secondFrame.end());
    writeU64(malformed.data() + 32, malformed.size() - kInputTapeHeaderSize);
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::InvalidCompressedPayload);

    std::vector<std::uint8_t> expanded = expandV2(bytes);
    expanded[4 + 10] = 0x80;
    malformed = replaceV2Payload(bytes, expanded);
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::InvalidPadFlags);

    expanded = expandV2(bytes);
    expanded[1] = 0xff;
    malformed = replaceV2Payload(bytes, expanded);
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::InvalidFrameCondition);

    expanded = expandV2(bytes);
    expanded[1] = static_cast<std::uint8_t>(InputFrameCondition::NameEntryActive);
    malformed = replaceV2Payload(bytes, expanded);
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::InvalidFrameCondition);

    tape.frames[0].ownedPorts = 0x80;
    REQUIRE(encode_input_tape(tape, bytes) == InputTapeError::InvalidOwnedPorts);
}

void testMinorZeroConnectionErrorsRemainCompatible() {
    using namespace dusk::automation;

    InputTape tape;
    tape.frames.resize(1);
    tape.frames[0].pads[0].flags = RawPadFlags::None;
    tape.frames[0].pads[0].error = PAD_ERR_NO_CONTROLLER;
    std::vector<std::uint8_t> bytes = makeLegacyV1(tape, 0);
    constexpr std::size_t legacyHeaderSize = 32;
    bytes[legacyHeaderSize + 4 + 11] = 0;  // This byte was reserved in v1.0.
    InputTape decoded;
    REQUIRE(decode_input_tape(bytes, decoded) == InputTapeError::None);
    REQUIRE(decoded.frames[0].pads[0].error == PAD_ERR_NO_CONTROLLER);
}

void testLegacyMinorOneAndTwoRemainCompatible() {
    using namespace dusk::automation;

    InputTape plain;
    plain.frames.resize(1);
    plain.frames[0].ownedPorts = 1;
    plain.frames[0].pads[0].buttons = PAD_BUTTON_START;
    plain.frames[0].pads[0].error = PAD_ERR_TRANSFER;
    InputTape decoded;
    REQUIRE(decode_input_tape(makeLegacyV1(plain, 1), decoded) == InputTapeError::None);
    REQUIRE(decoded == plain);

    InputTape conditioned = plain;
    conditioned.frames[0].condition = InputFrameCondition::NameEntryActive;
    conditioned.frames[0].timeoutTicks = 9;
    REQUIRE(decode_input_tape(makeLegacyV1(conditioned, 2), decoded) == InputTapeError::None);
    REQUIRE(decoded == conditioned);
}

void testEmptyTapeRoundTrip() {
    using namespace dusk::automation;

    const InputTape tape;
    std::vector<std::uint8_t> bytes;
    REQUIRE(encode_input_tape(tape, bytes) == InputTapeError::None);
    REQUIRE(bytes.size() > kInputTapeHeaderSize);
    REQUIRE(readU64(bytes.data() + 24) == 0);
    InputTape decoded;
    REQUIRE(decode_input_tape(bytes, decoded) == InputTapeError::None);
    REQUIRE(decoded == tape);
}

void testRepeatedTapeIsCompact() {
    using namespace dusk::automation;

    InputTape tape;
    tape.frames.resize(1024);
    for (InputFrame& frame : tape.frames) {
        frame.ownedPorts = 0x0f;
    }
    std::vector<std::uint8_t> bytes;
    REQUIRE(encode_input_tape(tape, bytes) == InputTapeError::None);
    REQUIRE(bytes.size() < kInputTapeHeaderSize + tape.frames.size() * kInputFrameSize);
    InputTape decoded;
    REQUIRE(decode_input_tape(bytes, decoded) == InputTapeError::None);
    REQUIRE(decoded == tape);
}

void testConditionedFrameRoundTrip() {
    using namespace dusk::automation;

    InputTape tape;
    tape.frames.resize(1);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::NameEntryActive;
    tape.frames[0].timeoutTicks = 1234;

    std::vector<std::uint8_t> bytes;
    REQUIRE(encode_input_tape(tape, bytes) == InputTapeError::None);
    const std::vector<std::uint8_t> expanded = expandV2(bytes);
    REQUIRE(expanded[0] == 0x0f);
    REQUIRE(expanded[1] == static_cast<std::uint8_t>(InputFrameCondition::NameEntryActive));
    REQUIRE(expanded[2] == 0xd2);
    REQUIRE(expanded[3] == 0x04);

    InputTape decoded;
    REQUIRE(decode_input_tape(bytes, decoded) == InputTapeError::None);
    REQUIRE(decoded == tape);
}

void testPlayerOwnsAndReleasesPorts() {
    using namespace dusk::automation;

    resetPadSpies();
    InputTape tape;
    tape.frames.resize(2);
    tape.frames[0].ownedPorts = 1 << 0;
    tape.frames[0].pads[0].buttons = PAD_BUTTON_A;
    tape.frames[0].pads[0].stickX = -42;
    tape.frames[1].ownedPorts = 1 << 1;
    tape.frames[1].pads[1].buttons = PAD_BUTTON_START;
    tape.frames[1].pads[1].flags = RawPadFlags::None;
    tape.frames[1].pads[1].error = PAD_ERR_NO_CONTROLLER;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Release));

    player.tick();
    REQUIRE(gActive[0]);
    REQUIRE(gSetCalls[0] == 1);
    REQUIRE(gStatuses[0].button == PAD_BUTTON_A);
    REQUIRE(gStatuses[0].stickX == -42);
    REQUIRE(gStatuses[0].err == PAD_ERR_NONE);

    player.tick();
    REQUIRE(!gActive[0]);
    REQUIRE(gClearCalls[0] == 1);
    REQUIRE(gActive[1]);
    REQUIRE(gStatuses[1].button == PAD_BUTTON_START);
    REQUIRE(gStatuses[1].err == PAD_ERR_NO_CONTROLLER);
    REQUIRE(!player.isPlaying());

    // Release occurs on the following input tick, after the final frame has
    // been visible to PADRead for exactly one tick.
    player.tick();
    REQUIRE(!gActive[1]);
    REQUIRE(gClearCalls[1] == 1);
}

void testSuccessfulHoldTapeCanHandOffImmediately() {
    using namespace dusk::automation;

    resetPadSpies();
    InputTape tape;
    tape.frames.resize(1);
    tape.frames[0].ownedPorts = 1 << 0;
    tape.frames[0].pads[0].buttons = PAD_BUTTON_START;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Hold));
    player.tick();
    REQUIRE(!player.isPlaying());
    REQUIRE(!player.hasFailed());
    REQUIRE(player.nextFrameIndex() == 1);
    REQUIRE(gActive[0]);

    player.handoffToLiveInput();
    REQUIRE(!gActive[0]);
    REQUIRE(gClearCalls[0] == 1);
    REQUIRE(player.nextFrameIndex() == 1);

    player.handoffToLiveInput();
    REQUIRE(gClearCalls[0] == 1);
}

void testCompletedFrameCountMarksExactFastForwardBoundary() {
    using namespace dusk::automation;

    resetPadSpies();
    InputTape tape;
    tape.frames.resize(3);
    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Release));

    constexpr std::size_t revealAfter = 2;
    REQUIRE(player.consumedFrameCount() == 0);
    player.tick();
    REQUIRE(player.consumedFrameCount() == revealAfter - 1);
    REQUIRE(player.consumedFrameCount() != revealAfter);
    player.tick();
    REQUIRE(player.consumedFrameCount() == revealAfter);
    REQUIRE(player.isPlaying());
    player.tick();
    REQUIRE(player.consumedFrameCount() == revealAfter + 1);
    REQUIRE(!player.isPlaying());
}

void testMaximumExecutionTicksAccountsForConditionTimeouts() {
    using namespace dusk::automation;

    InputTape tape;
    tape.frames.resize(3);
    REQUIRE(input_tape_is_absolute(tape));
    tape.frames[1].condition = InputFrameCondition::NameEntryActive;
    tape.frames[1].timeoutTicks = 9;
    tape.frames[2].condition = InputFrameCondition::FileSelectAcceptReady;
    tape.frames[2].timeoutTicks = 17;
    REQUIRE(!input_tape_is_absolute(tape));

    std::size_t ticks = 0;
    REQUIRE(input_tape_maximum_execution_ticks(tape, ticks));
    REQUIRE(ticks == 27);
}

void testPlayerWaitsNeutrallyForCondition() {
    using namespace dusk::automation;

    resetPadSpies();
    name_entry_observer().endSession();

    InputTape tape;
    tape.frames.resize(2);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::NameEntryActive;
    tape.frames[0].timeoutTicks = 3;
    tape.frames[1].ownedPorts = 0x0f;
    tape.frames[1].pads[0].buttons = PAD_BUTTON_B;

    InputTapePlayer player;
    player.install(tape);
    REQUIRE(player.start(TapeEndBehavior::Hold));

    player.tick();
    REQUIRE(player.isPlaying());
    REQUIRE(player.nextFrameIndex() == 0);
    for (std::size_t port = 0; port < PAD_CHANMAX; ++port) {
        REQUIRE(gActive[port]);
        REQUIRE(gStatuses[port].button == 0);
        REQUIRE(gStatuses[port].stickX == 0);
    }

    name_entry_observer().beginSession();
    player.tick();
    REQUIRE(!player.isPlaying());
    REQUIRE(!player.hasFailed());
    REQUIRE(player.nextFrameIndex() == 2);
    REQUIRE(gStatuses[0].button == PAD_BUTTON_B);
    name_entry_observer().endSession();
}

void testPlayerConditionTimeoutIsTerminal() {
    using namespace dusk::automation;

    resetPadSpies();
    name_entry_observer().endSession();

    InputTape tape;
    tape.frames.resize(1);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::NameEntryActive;
    tape.frames[0].timeoutTicks = 2;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Hold));
    player.tick();
    REQUIRE(player.isPlaying());
    player.tick();
    REQUIRE(!player.isPlaying());
    REQUIRE(player.hasFailed());
    REQUIRE(player.playbackError() == InputTapePlaybackError::ConditionTimedOut);
    REQUIRE(player.failedFrameIndex() == 0);
    REQUIRE(player.failedCondition() == InputFrameCondition::NameEntryActive);
    REQUIRE(gActive[0]);
    REQUIRE(gStatuses[0].button == 0);
}

void testPlayerPulsesConditionedInputUntilSatisfied() {
    using namespace dusk::automation;

    resetPadSpies();
    name_entry_observer().endSession();

    InputTape tape;
    tape.frames.resize(2);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::NameEntryActive;
    tape.frames[0].timeoutTicks = 4;
    tape.frames[0].pads[0].buttons = PAD_BUTTON_A;
    tape.frames[1].ownedPorts = 0x0f;
    tape.frames[1].pads[0].buttons = PAD_BUTTON_B;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Hold));

    player.tick();
    REQUIRE(player.nextFrameIndex() == 0);
    REQUIRE(gStatuses[0].button == PAD_BUTTON_A);
    player.tick();
    REQUIRE(gStatuses[0].button == 0);

    name_entry_observer().beginSession();
    player.tick();
    REQUIRE(!player.isPlaying());
    REQUIRE(!player.hasFailed());
    REQUIRE(gStatuses[0].button == PAD_BUTTON_B);
    name_entry_observer().endSession();
}

void testSatisfiedPulseDoesNotApplyItsAction() {
    using namespace dusk::automation;

    resetPadSpies();
    name_entry_observer().beginSession();

    InputTape tape;
    tape.frames.resize(2);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::NameEntryActive;
    tape.frames[0].timeoutTicks = 2;
    tape.frames[0].pads[0].buttons = PAD_BUTTON_A;
    tape.frames[1].ownedPorts = 0x0f;
    tape.frames[1].pads[0].buttons = PAD_BUTTON_B;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Hold));
    player.tick();
    REQUIRE(!player.isPlaying());
    REQUIRE(gStatuses[0].button == PAD_BUTTON_B);
    name_entry_observer().endSession();
}

void testPlayerWaitsForInteractiveCharacterSelection() {
    using namespace dusk::automation;

    resetPadSpies();
    auto& observer = name_entry_observer();
    observer.endSession();
    observer.beginSession();

    InputTape tape;
    tape.frames.resize(2);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::NameEntryCharacterSelect;
    tape.frames[0].timeoutTicks = 4;
    tape.frames[1].ownedPorts = 0x0f;
    tape.frames[1].pads[0].buttons = PAD_BUTTON_A;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Hold));

    player.tick();
    REQUIRE(player.nextFrameIndex() == 0);
    std::array<NameEntryCharacterObservation, NameEntryOriginalLayout::CharacterCount>
        characters{};
    observer.observe(0, 0, 0, 4, 0, 0, 0, characters);
    player.tick();
    REQUIRE(player.nextFrameIndex() == 0);
    REQUIRE(gStatuses[0].button == 0);

    observer.observe(0, 0, 0, 0, 0, 0, 0, characters);
    observer.markInputProcessed();
    player.tick();
    REQUIRE(!player.isPlaying());
    REQUIRE(!player.hasFailed());
    REQUIRE(gStatuses[0].button == PAD_BUTTON_A);
    observer.endSession();
}

void testPlayerWaitsForStableNameEntryInputHandler() {
    using namespace dusk::automation;

    resetPadSpies();
    auto& observer = name_entry_observer();
    observer.endSession();
    observer.beginSession();

    InputTape tape;
    tape.frames.resize(2);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::NameEntryInputReady;
    tape.frames[0].timeoutTicks = 4;
    tape.frames[1].ownedPorts = 0x0f;
    tape.frames[1].pads[0].buttons = PAD_BUTTON_B;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Hold));

    std::array<NameEntryCharacterObservation, NameEntryOriginalLayout::CharacterCount>
        characters{};
    observer.observe(0, 0, 0, 2, 0, 0, 0, characters);
    observer.markInputProcessed();
    player.tick();
    REQUIRE(player.nextFrameIndex() == 0);
    REQUIRE(gStatuses[0].button == 0);

    observer.observe(0, 0, 0, 4, 0, 0, 0, characters);
    player.tick();
    REQUIRE(!player.isPlaying());
    REQUIRE(!player.hasFailed());
    REQUIRE(gStatuses[0].button == PAD_BUTTON_B);
    observer.endSession();
}

void testPlayerWaitsForNoSavePromptHandler() {
    using namespace dusk::automation;

    resetPadSpies();
    file_select_observer().setNoSavePromptReady(false);

    InputTape tape;
    tape.frames.resize(2);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::FileSelectNoSaveReady;
    tape.frames[0].timeoutTicks = 3;
    tape.frames[1].ownedPorts = 0x0f;
    tape.frames[1].pads[0].buttons = PAD_BUTTON_A;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Hold));
    player.tick();
    REQUIRE(player.nextFrameIndex() == 0);
    REQUIRE(gStatuses[0].button == 0);

    file_select_observer().setNoSavePromptReady(true);
    player.tick();
    REQUIRE(!player.isPlaying());
    REQUIRE(!player.hasFailed());
    REQUIRE(gStatuses[0].button == PAD_BUTTON_A);
    file_select_observer().setNoSavePromptReady(false);
}

void testPlayerWaitsForStableFileDataSelection() {
    using namespace dusk::automation;

    resetPadSpies();
    file_select_observer().setDataSelectReady(false);

    InputTape tape;
    tape.frames.resize(2);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::FileSelectDataSelectReady;
    tape.frames[0].timeoutTicks = 3;
    tape.frames[1].ownedPorts = 0x0f;
    tape.frames[1].pads[0].buttons = PAD_BUTTON_A;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Hold));
    player.tick();
    REQUIRE(player.nextFrameIndex() == 0);

    file_select_observer().setDataSelectReady(true);
    player.tick();
    REQUIRE(!player.isPlaying());
    REQUIRE(!player.hasFailed());
    REQUIRE(gStatuses[0].button == PAD_BUTTON_A);
    file_select_observer().setDataSelectReady(false);
}

void testFileSelectAcceptReadyCoversStableHandlers() {
    using namespace dusk::automation;

    auto& observer = file_select_observer();
    observer.setNoSavePromptReady(false);
    observer.setDataSelectReady(false);
    observer.setKeyWaitReady(false);
    observer.setYesNoSelectReady(false);
    REQUIRE(!observer.acceptReady());
    observer.setKeyWaitReady(true);
    REQUIRE(observer.acceptReady());
    observer.setKeyWaitReady(false);
    observer.setNoSavePromptReady(true);
    REQUIRE(observer.acceptReady());
    observer.setNoSavePromptReady(false);
    observer.setYesNoSelectReady(true);
    REQUIRE(observer.acceptReady());
    observer.setYesNoSelectReady(false);
}

void testSatisfiedConditionOnlyLoopConsumesOneTick() {
    using namespace dusk::automation;

    resetPadSpies();
    name_entry_observer().beginSession();

    InputTape tape;
    tape.frames.resize(1);
    tape.frames[0].ownedPorts = 0x0f;
    tape.frames[0].condition = InputFrameCondition::NameEntryActive;
    tape.frames[0].timeoutTicks = 2;

    InputTapePlayer player;
    player.install(std::move(tape));
    REQUIRE(player.start(TapeEndBehavior::Loop));
    player.tick();
    REQUIRE(player.isPlaying());
    REQUIRE(player.nextFrameIndex() == 0);
    REQUIRE(!player.hasFailed());
    REQUIRE(gSetCalls[0] == 1);
    REQUIRE(gStatuses[0].button == 0);
    name_entry_observer().endSession();
}

void testRecorderCapturesAllPortsWithoutGrowing() {
    using namespace dusk::automation;

    std::array<PADStatus, kInputPortCount> statuses{};
    statuses[0].button = PAD_BUTTON_A;
    statuses[0].stickX = -71;
    statuses[0].triggerRight = 193;
    statuses[0].err = PAD_ERR_NONE;
    statuses[1].button = PAD_BUTTON_B;
    statuses[1].substickY = 54;
    statuses[1].err = PAD_ERR_NONE;
    statuses[2].err = PAD_ERR_NO_CONTROLLER;
    statuses[3].err = PAD_ERR_TRANSFER;

    InputTapeRecorder recorder;
    REQUIRE(recorder.start(0b0011, 2, 60, 2) == InputTapeError::None);
    REQUIRE(recorder.isRecording());
    REQUIRE(recorder.frameCapacity() == 2);
    REQUIRE(recorder.recordTick(statuses) == InputRecordResult::Recorded);

    statuses[0].button = PAD_BUTTON_START;
    REQUIRE(recorder.recordTick(statuses) == InputRecordResult::Recorded);
    REQUIRE(recorder.frameCount() == 2);
    REQUIRE(recorder.recordTick(statuses) == InputRecordResult::CapacityExhausted);
    REQUIRE(!recorder.isRecording());
    REQUIRE(recorder.capacityExhausted());

    InputTape tape = recorder.take();
    REQUIRE(tape.tickRateNumerator == 60);
    REQUIRE(tape.tickRateDenominator == 2);
    REQUIRE(tape.frames.size() == 2);
    REQUIRE(tape.frames.capacity() >= 2);
    REQUIRE(tape.frames[0].ownedPorts == 0b0011);
    REQUIRE(tape.frames[0].pads[0].buttons == PAD_BUTTON_A);
    REQUIRE(tape.frames[0].pads[0].stickX == -71);
    REQUIRE(tape.frames[0].pads[0].triggerRight == 193);
    REQUIRE(tape.frames[0].pads[1].buttons == PAD_BUTTON_B);
    REQUIRE(tape.frames[0].pads[1].substickY == 54);
    // Unowned ports are still captured so ownership can be changed when a
    // recording is edited or minimized later.
    REQUIRE(tape.frames[0].pads[2].error == PAD_ERR_NO_CONTROLLER);
    REQUIRE(tape.frames[0].pads[3].error == PAD_ERR_TRANSFER);
    REQUIRE(tape.frames[1].pads[0].buttons == PAD_BUTTON_START);
    REQUIRE(!recorder.capacityExhausted());
    REQUIRE(recorder.frameCount() == 0);
    REQUIRE(!recorder.isArmed());
}

void testRecorderArmsUntilExactHandoff() {
    using namespace dusk::automation;

    std::array<PADStatus, kInputPortCount> statuses{};
    statuses[0].button = PAD_BUTTON_A;
    statuses[0].err = PAD_ERR_NONE;

    InputTapeRecorder recorder;
    REQUIRE(recorder.arm(1, 1) == InputTapeError::None);
    REQUIRE(recorder.isArmed());
    REQUIRE(!recorder.isRecording());
    REQUIRE(recorder.recordTick(statuses) == InputRecordResult::Inactive);
    REQUIRE(recorder.frameCount() == 0);

    REQUIRE(recorder.begin());
    REQUIRE(recorder.recordTick(statuses) == InputRecordResult::Recorded);
    REQUIRE(recorder.recordTick(statuses) == InputRecordResult::CapacityExhausted);
    REQUIRE(recorder.isArmed());
    REQUIRE(!recorder.isRecording());
    REQUIRE(recorder.capacityExhausted());
    // Remaining armed is a runtime guardrail: mouse and gyro must stay
    // suppressed once subsequent PAD frames can no longer be retained.
    REQUIRE(!recorder.begin());
}

void testRecordedRawInputReplaysThroughExactlyOneClamp() {
    using namespace dusk::automation;

    std::array<PADStatus, kInputPortCount> raw{};
    raw[0].button = PAD_BUTTON_A | PAD_TRIGGER_Z;
    raw[0].stickX = 72;
    raw[0].stickY = -91;
    raw[0].substickX = 59;
    raw[0].substickY = -67;
    raw[0].triggerLeft = 180;
    raw[0].triggerRight = 93;
    raw[0].analogA = 211;
    raw[0].analogB = 37;
    raw[0].err = PAD_ERR_NONE;
    for (std::size_t port = 1; port < raw.size(); ++port) {
        raw[port].err = PAD_ERR_NO_CONTROLLER;
    }

    auto expectedPostClamp = raw;
    PADClamp(expectedPostClamp.data());
    auto doubleClamped = expectedPostClamp;
    PADClamp(doubleClamped.data());
    REQUIRE(doubleClamped[0].stickX != expectedPostClamp[0].stickX);
    REQUIRE(doubleClamped[0].triggerLeft != expectedPostClamp[0].triggerLeft);

    InputTapeRecorder recorder;
    REQUIRE(recorder.start(1, 1) == InputTapeError::None);
    REQUIRE(recorder.recordTick(raw) == InputRecordResult::Recorded);
    std::vector<std::uint8_t> encoded;
    REQUIRE(encode_input_tape(recorder.take(), encoded) == InputTapeError::None);

    InputTape decoded;
    REQUIRE(decode_input_tape(encoded, decoded) == InputTapeError::None);
    InputTapePlayer player;
    player.install(std::move(decoded));
    resetPadSpies();
    REQUIRE(player.start(TapeEndBehavior::Release));
    player.tick();
    // Playback injection is the same pre-clamp state captured by PADRead.
    REQUIRE(gStatuses[0].stickX == raw[0].stickX);
    REQUIRE(gStatuses[0].triggerLeft == raw[0].triggerLeft);
    PADClamp(gStatuses.data());

    constexpr std::size_t port = 0;
    REQUIRE(gStatuses[port].button == expectedPostClamp[port].button);
    REQUIRE(gStatuses[port].stickX == expectedPostClamp[port].stickX);
    REQUIRE(gStatuses[port].stickY == expectedPostClamp[port].stickY);
    REQUIRE(gStatuses[port].substickX == expectedPostClamp[port].substickX);
    REQUIRE(gStatuses[port].substickY == expectedPostClamp[port].substickY);
    REQUIRE(gStatuses[port].triggerLeft == expectedPostClamp[port].triggerLeft);
    REQUIRE(gStatuses[port].triggerRight == expectedPostClamp[port].triggerRight);
    REQUIRE(gStatuses[port].analogA == expectedPostClamp[port].analogA);
    REQUIRE(gStatuses[port].analogB == expectedPostClamp[port].analogB);
    REQUIRE(gStatuses[port].err == expectedPostClamp[port].err);
}

void testRecorderOutputDependsOnPadTicksNotHostPacing() {
    using namespace dusk::automation;

    std::array<std::array<PADStatus, kInputPortCount>, 4> ticks{};
    for (auto& statuses : ticks) {
        for (std::size_t port = 1; port < statuses.size(); ++port) {
            statuses[port].err = PAD_ERR_NO_CONTROLLER;
        }
    }
    ticks[0][0].stickX = 64;
    ticks[1][0].button = PAD_BUTTON_A;
    ticks[2][0].button = PAD_BUTTON_A | PAD_BUTTON_B;
    ticks[3][0].button = 0;

    const auto record = [&ticks](const std::chrono::milliseconds hostDelay) {
        InputTapeRecorder recorder;
        REQUIRE(recorder.start(1, ticks.size(), 30, 1) == InputTapeError::None);
        for (const auto& statuses : ticks) {
            if (hostDelay.count() != 0) {
                std::this_thread::sleep_for(hostDelay);
            }
            REQUIRE(recorder.recordTick(statuses) == InputRecordResult::Recorded);
        }
        std::vector<std::uint8_t> encoded;
        REQUIRE(encode_input_tape(recorder.take(), encoded) == InputTapeError::None);
        return encoded;
    };

    const auto normalHostPacing = record(std::chrono::milliseconds(0));
    const auto slowedHostPacing = record(std::chrono::milliseconds(2));
    REQUIRE(slowedHostPacing == normalHostPacing);

    InputTape roundTrip;
    REQUIRE(decode_input_tape(slowedHostPacing, roundTrip) == InputTapeError::None);
    REQUIRE(roundTrip.tickRateNumerator == 30);
    REQUIRE(roundTrip.tickRateDenominator == 1);
    REQUIRE(roundTrip.frames.size() == ticks.size());
    REQUIRE(roundTrip.frames[1].pads[0].buttons == PAD_BUTTON_A);
    REQUIRE(roundTrip.frames[2].pads[0].buttons == (PAD_BUTTON_A | PAD_BUTTON_B));
}

void testScenarioFixtureCanonicalRoundTripAndCorruption() {
    using namespace dusk::automation;

    constexpr std::string_view goldenHex =
        "4455534b465854520100000020000c002c010000000000000000000000000000"
        "0100000013000000776f6c6620636f6d626174206c6f61646f75740002000000"
        "0400000001000000030000000400000014002800040000001800000000000000"
        "0100000002000000030000000700000000000000040000001800000001000000"
        "0400000005000000060000000800000000000000050000000400000002000000"
        "0600000008000000010010001e000000060000000800000004002a0001000000"
        "0700000004000000020009000800000008000000030101000c00000009000000"
        "210000001502080067616d652e64616d6167654d756c7469706c696572020000"
        "0000000000000000090000001a0000001501010067616d652e656e61626c654d"
        "6972726f724d6f6465000000";
    const auto nibble = [](const char value) -> std::uint8_t {
        return static_cast<std::uint8_t>(value <= '9' ? value - '0' : value - 'a' + 10);
    };
    std::vector<std::uint8_t> golden;
    golden.reserve(goldenHex.size() / 2);
    for (std::size_t index = 0; index < goldenHex.size(); index += 2) {
        golden.push_back(static_cast<std::uint8_t>(
            (nibble(goldenHex[index]) << 4) | nibble(goldenHex[index + 1])));
    }

    ScenarioFixture fixture;
    fixture.name = "wolf combat loadout";
    fixture.form = PlayerFixtureForm::Wolf;
    fixture.health = HealthFixture{20, 40};
    fixture.rng = {
        {FixtureRngStream::Secondary, 4, 5, 6, 8},
        {FixtureRngStream::Primary, 1, 2, 3, 7},
    };
    fixture.videoMode = FixtureVideoMode::NtscProgressive;
    fixture.inventory = {{4, 0x2a, 1}, {1, 0x10, 30}};
    fixture.equipment = {{2, 9}};
    fixture.flags = {{FixtureFlagDomain::Switch, 1, 12, true}};
    fixture.settings = {
        {"game.enableMirrorMode", false},
        {"game.damageMultiplier", std::int64_t{2}},
    };

    std::vector<std::uint8_t> encoded;
    REQUIRE(encode_scenario_fixture(fixture, encoded) == ScenarioFixtureError::None);
    REQUIRE(encoded == golden);
    REQUIRE(encoded.size() > kScenarioFixtureHeaderSize);
    REQUIRE(encoded[0] == 'D' && encoded[4] == 'F');
    REQUIRE(encoded[14] == 12 && encoded[15] == 0);

    ScenarioFixture decoded;
    REQUIRE(decode_scenario_fixture(encoded, decoded) == ScenarioFixtureError::None);
    REQUIRE(decoded.name == fixture.name);
    REQUIRE(decoded.form == fixture.form);
    REQUIRE(decoded.health == fixture.health);
    REQUIRE(decoded.rng.front().stream == FixtureRngStream::Primary);
    REQUIRE(decoded.inventory.front().slot == 1);
    REQUIRE(decoded.settings.front().key == "game.damageMultiplier");

    std::vector<std::uint8_t> second;
    REQUIRE(encode_scenario_fixture(decoded, second) == ScenarioFixtureError::None);
    REQUIRE(second == encoded);

    InputTape tape;
    tape.boot.kind = TapeBootKind::Stage;
    tape.boot.stage = "F_SP103";
    tape.boot.room = 1;
    tape.boot.point = 1;
    tape.boot.layer = 3;
    tape.boot.fixture = decoded;
    tape.frames.resize(1);
    std::vector<std::uint8_t> tapeBytes;
    REQUIRE(encode_input_tape(tape, tapeBytes) == InputTapeError::None);
    REQUIRE(tapeBytes[47] == 1);
    InputTape decodedTape;
    REQUIRE(decode_input_tape(tapeBytes, decodedTape) == InputTapeError::None);
    REQUIRE(decodedTape == tape);

    for (std::size_t end = 0; end < encoded.size(); ++end) {
        ScenarioFixture truncated;
        REQUIRE(decode_scenario_fixture(
                    std::span<const std::uint8_t>(encoded.data(), end), truncated) !=
                ScenarioFixtureError::None);
    }
    auto reserved = encoded;
    reserved[20] = 1;
    REQUIRE(decode_scenario_fixture(reserved, decoded) == ScenarioFixtureError::InvalidHeader);

    fixture.inventory.push_back({1, 99, 1});
    REQUIRE(encode_scenario_fixture(fixture, second) == ScenarioFixtureError::DuplicateKey);
}

} // namespace

extern "C" void PADSetAutomationStatus(const u32 port, const PADStatus* status) {
    REQUIRE(port < PAD_CHANMAX);
    REQUIRE(status != nullptr);
    gStatuses[port] = *status;
    gActive[port] = true;
    ++gSetCalls[port];
}

extern "C" void PADClearAutomationStatus(const u32 port) {
    REQUIRE(port < PAD_CHANMAX);
    gStatuses[port] = {};
    gActive[port] = false;
    ++gClearCalls[port];
}

extern "C" void PADClamp(PADStatus* statuses) {
    const auto clampAxis = [](const s8 value) -> s8 {
        if (value > 15) {
            return static_cast<s8>(value - 15);
        }
        if (value < -15) {
            return static_cast<s8>(value + 15);
        }
        return 0;
    };
    const auto clampTrigger = [](const u8 value) -> u8 {
        return value <= 30 ? 0 : static_cast<u8>(std::min<unsigned>(value, 180) - 30);
    };
    for (std::size_t port = 0; port < PAD_CHANMAX; ++port) {
        if (statuses[port].err != PAD_ERR_NONE) {
            continue;
        }
        statuses[port].stickX = clampAxis(statuses[port].stickX);
        statuses[port].stickY = clampAxis(statuses[port].stickY);
        statuses[port].substickX = clampAxis(statuses[port].substickX);
        statuses[port].substickY = clampAxis(statuses[port].substickY);
        statuses[port].triggerLeft = clampTrigger(statuses[port].triggerLeft);
        statuses[port].triggerRight = clampTrigger(statuses[port].triggerRight);
    }
}

int main() {
    testCanonicalRoundTrip();
    testFullPadSurfaceOnAllFourPorts();
    testStageBootAndCompressedV2Compatibility();
    testMalformedTapesAreRejected();
    testMinorZeroConnectionErrorsRemainCompatible();
    testLegacyMinorOneAndTwoRemainCompatible();
    testEmptyTapeRoundTrip();
    testRepeatedTapeIsCompact();
    testConditionedFrameRoundTrip();
    testPlayerOwnsAndReleasesPorts();
    testSuccessfulHoldTapeCanHandOffImmediately();
    testCompletedFrameCountMarksExactFastForwardBoundary();
    testMaximumExecutionTicksAccountsForConditionTimeouts();
    testPlayerWaitsNeutrallyForCondition();
    testPlayerConditionTimeoutIsTerminal();
    testPlayerPulsesConditionedInputUntilSatisfied();
    testSatisfiedPulseDoesNotApplyItsAction();
    testPlayerWaitsForInteractiveCharacterSelection();
    testPlayerWaitsForStableNameEntryInputHandler();
    testPlayerWaitsForNoSavePromptHandler();
    testPlayerWaitsForStableFileDataSelection();
    testFileSelectAcceptReadyCoversStableHandlers();
    testSatisfiedConditionOnlyLoopConsumesOneTick();
    testRecorderCapturesAllPortsWithoutGrowing();
    testRecorderArmsUntilExactHandoff();
    testRecordedRawInputReplaysThroughExactlyOneClamp();
    testRecorderOutputDependsOnPadTicksNotHostPacing();
    testScenarioFixtureCanonicalRoundTripAndCorruption();
    std::cout << "input tape tests passed\n";
    return 0;
}
