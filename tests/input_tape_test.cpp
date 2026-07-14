#include "dusk/automation/input_tape.hpp"
#include "dusk/automation/file_select_observer.hpp"
#include "dusk/automation/name_entry_observer.hpp"

#include <dolphin/pad.h>

#include <algorithm>
#include <array>
#include <cstdlib>
#include <iostream>
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
    REQUIRE(bytes.size() == kInputTapeHeaderSize + kInputFrameSize);
    REQUIRE(std::equal(kInputTapeMagic.begin(), kInputTapeMagic.end(), bytes.begin()));
    REQUIRE(bytes[8] == 1 && bytes[9] == 0);
    REQUIRE(bytes[10] == kInputTapeMinorVersion && bytes[11] == 0);
    REQUIRE(bytes[12] == kInputTapeHeaderSize && bytes[13] == 0);
    REQUIRE(bytes[14] == kInputFrameSize && bytes[15] == 0);
    REQUIRE(bytes[16] == 60 && bytes[20] == 2);
    REQUIRE(bytes[24] == 1);

    const std::size_t pad0 = kInputTapeHeaderSize + 4;
    REQUIRE(bytes[kInputTapeHeaderSize] == 0b0101);
    REQUIRE(bytes[pad0] == 0x34 && bytes[pad0 + 1] == 0x12);
    REQUIRE(bytes[pad0 + 2] == 0x81);
    REQUIRE(bytes[pad0 + 10] == static_cast<std::uint8_t>(RawPadFlags::Connected));
    REQUIRE(bytes[pad0 + 11] == 0);

    InputTape decoded;
    REQUIRE(decode_input_tape(bytes, decoded) == InputTapeError::None);
    REQUIRE(decoded == tape);
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
    malformed[8] = 2;
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::UnsupportedVersion);

    malformed = bytes;
    malformed.push_back(0);
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::TrailingData);

    malformed = bytes;
    malformed[kInputTapeHeaderSize + 4 + 10] = 0x80;
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::InvalidPadFlags);

    malformed = bytes;
    malformed[kInputTapeHeaderSize + 1] = 0xff;
    REQUIRE(decode_input_tape(malformed, decoded) == InputTapeError::InvalidFrameCondition);

    malformed = bytes;
    malformed[kInputTapeHeaderSize + 1] =
        static_cast<std::uint8_t>(InputFrameCondition::NameEntryActive);
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
    std::vector<std::uint8_t> bytes;
    REQUIRE(encode_input_tape(tape, bytes) == InputTapeError::None);

    bytes[10] = 0; // Minor version 0.
    bytes[11] = 0;
    bytes[kInputTapeHeaderSize + 4 + 11] = 0; // This byte was reserved in v1.0.
    InputTape decoded;
    REQUIRE(decode_input_tape(bytes, decoded) == InputTapeError::None);
    REQUIRE(decoded.frames[0].pads[0].error == PAD_ERR_NO_CONTROLLER);
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
    REQUIRE(bytes[kInputTapeHeaderSize] == 0x0f);
    REQUIRE(bytes[kInputTapeHeaderSize + 1] ==
            static_cast<std::uint8_t>(InputFrameCondition::NameEntryActive));
    REQUIRE(bytes[kInputTapeHeaderSize + 2] == 0xd2);
    REQUIRE(bytes[kInputTapeHeaderSize + 3] == 0x04);

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

int main() {
    testCanonicalRoundTrip();
    testMalformedTapesAreRejected();
    testMinorZeroConnectionErrorsRemainCompatible();
    testConditionedFrameRoundTrip();
    testPlayerOwnsAndReleasesPorts();
    testSuccessfulHoldTapeCanHandOffImmediately();
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
    std::cout << "input tape tests passed\n";
    return 0;
}
