#include "dusk/automation/input_tape.hpp"

#include "dusk/automation/file_select_observer.hpp"
#include "dusk/automation/name_entry_observer.hpp"

#include <dolphin/pad.h>
#include <zstd.h>

#include <algorithm>
#include <limits>
#include <utility>

namespace dusk::automation {
namespace {

constexpr std::uint8_t kAllPortsMask = (1u << kInputPortCount) - 1u;
constexpr std::uint8_t kKnownPadFlags = static_cast<std::uint8_t>(RawPadFlags::Connected);
constexpr std::uint16_t kLegacyMajorVersion = 1;
constexpr std::uint16_t kLegacyMaximumMinorVersion = 2;
constexpr std::size_t kLegacyHeaderSize = 32;

bool valid_frame_condition(const InputFrameCondition condition) {
    return condition == InputFrameCondition::None ||
           condition == InputFrameCondition::NameEntryActive ||
           condition == InputFrameCondition::NameEntryCharacterSelect ||
           condition == InputFrameCondition::NameEntryInputReady ||
           condition == InputFrameCondition::FileSelectNoSaveReady ||
           condition == InputFrameCondition::FileSelectDataSelectReady ||
           condition == InputFrameCondition::FileSelectAcceptReady;
}

bool valid_condition_frame(const InputFrame& frame) {
    if (!valid_frame_condition(frame.condition)) {
        return false;
    }
    if (frame.condition == InputFrameCondition::None) {
        return frame.timeoutTicks == 0;
    }
    return frame.timeoutTicks != 0;
}

std::uint16_t read_u16(const std::uint8_t* input) {
    return static_cast<std::uint16_t>(input[0]) | (static_cast<std::uint16_t>(input[1]) << 8);
}

std::uint32_t read_u32(const std::uint8_t* input) {
    return static_cast<std::uint32_t>(input[0]) | (static_cast<std::uint32_t>(input[1]) << 8) |
           (static_cast<std::uint32_t>(input[2]) << 16) | (static_cast<std::uint32_t>(input[3]) << 24);
}

std::uint64_t read_u64(const std::uint8_t* input) {
    return static_cast<std::uint64_t>(read_u32(input)) |
           (static_cast<std::uint64_t>(read_u32(input + 4)) << 32);
}

void write_u16(std::uint8_t* output, std::uint16_t value) {
    output[0] = static_cast<std::uint8_t>(value);
    output[1] = static_cast<std::uint8_t>(value >> 8);
}

void write_u32(std::uint8_t* output, std::uint32_t value) {
    output[0] = static_cast<std::uint8_t>(value);
    output[1] = static_cast<std::uint8_t>(value >> 8);
    output[2] = static_cast<std::uint8_t>(value >> 16);
    output[3] = static_cast<std::uint8_t>(value >> 24);
}

void write_u64(std::uint8_t* output, std::uint64_t value) {
    write_u32(output, static_cast<std::uint32_t>(value));
    write_u32(output + 4, static_cast<std::uint32_t>(value >> 32));
}

RawPadState decode_pad(const std::uint8_t* input) {
    RawPadState pad;
    pad.buttons = read_u16(input);
    pad.stickX = static_cast<std::int8_t>(input[2]);
    pad.stickY = static_cast<std::int8_t>(input[3]);
    pad.substickX = static_cast<std::int8_t>(input[4]);
    pad.substickY = static_cast<std::int8_t>(input[5]);
    pad.triggerLeft = input[6];
    pad.triggerRight = input[7];
    pad.analogA = input[8];
    pad.analogB = input[9];
    pad.flags = static_cast<RawPadFlags>(input[10]);
    pad.error = static_cast<std::int8_t>(input[11]);
    return pad;
}

void encode_pad(const RawPadState& pad, std::uint8_t* output) {
    write_u16(output, pad.buttons);
    output[2] = static_cast<std::uint8_t>(pad.stickX);
    output[3] = static_cast<std::uint8_t>(pad.stickY);
    output[4] = static_cast<std::uint8_t>(pad.substickX);
    output[5] = static_cast<std::uint8_t>(pad.substickY);
    output[6] = pad.triggerLeft;
    output[7] = pad.triggerRight;
    output[8] = pad.analogA;
    output[9] = pad.analogB;
    output[10] = static_cast<std::uint8_t>(pad.flags);
    output[11] = static_cast<std::uint8_t>(pad.error);
}

PADStatus to_pad_status(const RawPadState& input) {
    PADStatus status{};
    status.button = input.buttons;
    status.stickX = input.stickX;
    status.stickY = input.stickY;
    status.substickX = input.substickX;
    status.substickY = input.substickY;
    status.triggerLeft = input.triggerLeft;
    status.triggerRight = input.triggerRight;
    status.analogA = input.analogA;
    status.analogB = input.analogB;
    status.err = input.error;
    return status;
}

RawPadState from_pad_status(const PADStatus& input) {
    RawPadState status;
    status.buttons = input.button;
    status.stickX = input.stickX;
    status.stickY = input.stickY;
    status.substickX = input.substickX;
    status.substickY = input.substickY;
    status.triggerLeft = input.triggerLeft;
    status.triggerRight = input.triggerRight;
    status.analogA = input.analogA;
    status.analogB = input.analogB;
    status.flags = input.err == PAD_ERR_NONE ? RawPadFlags::Connected : RawPadFlags::None;
    status.error = input.err;
    return status;
}

InputTapeError decode_frame_stream(const std::span<const std::uint8_t> bytes,
    const std::size_t frameCount, const std::uint16_t frameMinorVersion, InputTape& tape) {
    tape.frames.resize(frameCount);
    const std::uint8_t* input = bytes.data();
    for (InputFrame& frame : tape.frames) {
        frame.ownedPorts = input[0];
        if ((frame.ownedPorts & ~kAllPortsMask) != 0) {
            return InputTapeError::InvalidOwnedPorts;
        }
        if (frameMinorVersion < 2) {
            if (input[1] != 0 || input[2] != 0 || input[3] != 0) {
                return InputTapeError::InvalidFrameSize;
            }
        } else {
            frame.condition = static_cast<InputFrameCondition>(input[1]);
            frame.timeoutTicks = read_u16(input + 2);
        }
        input += 4;

        for (RawPadState& pad : frame.pads) {
            pad = decode_pad(input);
            if ((static_cast<std::uint8_t>(pad.flags) & ~kKnownPadFlags) != 0) {
                return InputTapeError::InvalidPadFlags;
            }
            if (frameMinorVersion == 0) {
                pad.error = has_flag(pad.flags, RawPadFlags::Connected) ? PAD_ERR_NONE :
                                                                          PAD_ERR_NO_CONTROLLER;
            }
            input += kRawPadStateSize;
        }
        if (!valid_condition_frame(frame)) {
            return InputTapeError::InvalidFrameCondition;
        }
    }
    return InputTapeError::None;
}

void encode_frame_stream(const InputTape& tape, const std::span<std::uint8_t> output) {
    std::uint8_t* destination = output.data();
    for (const InputFrame& frame : tape.frames) {
        destination[0] = frame.ownedPorts;
        destination[1] = static_cast<std::uint8_t>(frame.condition);
        write_u16(destination + 2, frame.timeoutTicks);
        destination += 4;
        for (const RawPadState& pad : frame.pads) {
            encode_pad(pad, destination);
            destination += kRawPadStateSize;
        }
    }
}

} // namespace

PADStatus raw_pad_state_to_pad_status(const RawPadState& input) {
    return to_pad_status(input);
}

const char* input_tape_error_message(const InputTapeError error) {
    switch (error) {
    case InputTapeError::None:
        return "no error";
    case InputTapeError::Truncated:
        return "input tape is truncated";
    case InputTapeError::BadMagic:
        return "input tape has an invalid magic value";
    case InputTapeError::UnsupportedVersion:
        return "input tape version is unsupported";
    case InputTapeError::InvalidHeaderSize:
        return "input tape header size is invalid";
    case InputTapeError::InvalidFrameSize:
        return "input tape frame size is invalid";
    case InputTapeError::InvalidTickRate:
        return "input tape tick rate is invalid";
    case InputTapeError::InvalidOwnedPorts:
        return "input tape owns an invalid controller port";
    case InputTapeError::InvalidFrameCondition:
        return "input tape contains an invalid conditioned frame";
    case InputTapeError::InvalidPadFlags:
        return "input tape contains unknown controller flags";
    case InputTapeError::InvalidCompressedPayload:
        return "input tape contains an invalid compressed payload";
    case InputTapeError::TrailingData:
        return "input tape contains trailing data";
    case InputTapeError::TooManyFrames:
        return "input tape frame count is too large";
    }
    return "unknown input tape error";
}

const char* input_frame_condition_name(const InputFrameCondition condition) {
    switch (condition) {
    case InputFrameCondition::None:
        return "none";
    case InputFrameCondition::NameEntryActive:
        return "name_entry_active";
    case InputFrameCondition::NameEntryCharacterSelect:
        return "name_entry_character_select";
    case InputFrameCondition::NameEntryInputReady:
        return "name_entry_input_ready";
    case InputFrameCondition::FileSelectNoSaveReady:
        return "file_select_no_save_ready";
    case InputFrameCondition::FileSelectDataSelectReady:
        return "file_select_data_select_ready";
    case InputFrameCondition::FileSelectAcceptReady:
        return "file_select_accept_ready";
    }
    return "unknown";
}

InputTapeError validate_input_tape(const InputTape& tape) {
    if (tape.tickRateNumerator == 0 || tape.tickRateDenominator == 0) {
        return InputTapeError::InvalidTickRate;
    }

    for (const InputFrame& frame : tape.frames) {
        if ((frame.ownedPorts & ~kAllPortsMask) != 0) {
            return InputTapeError::InvalidOwnedPorts;
        }
        if (!valid_condition_frame(frame)) {
            return InputTapeError::InvalidFrameCondition;
        }
        for (const RawPadState& pad : frame.pads) {
            if ((static_cast<std::uint8_t>(pad.flags) & ~kKnownPadFlags) != 0) {
                return InputTapeError::InvalidPadFlags;
            }
        }
    }
    return InputTapeError::None;
}

bool input_tape_maximum_execution_ticks(const InputTape& tape, std::size_t& output) {
    std::size_t total = 0;
    for (const InputFrame& frame : tape.frames) {
        const std::size_t ticks = frame.condition == InputFrameCondition::None ?
                                      1 : static_cast<std::size_t>(frame.timeoutTicks);
        if (ticks > std::numeric_limits<std::size_t>::max() - total) {
            return false;
        }
        total += ticks;
    }
    output = total;
    return true;
}

bool input_tape_is_absolute(const InputTape& tape) {
    return std::all_of(tape.frames.begin(), tape.frames.end(), [](const InputFrame& frame) {
        return frame.condition == InputFrameCondition::None;
    });
}

InputTapeError decode_input_tape(const std::span<const std::uint8_t> bytes, InputTape& output) {
    if (bytes.size() < kLegacyHeaderSize) {
        return InputTapeError::Truncated;
    }
    if (!std::equal(kInputTapeMagic.begin(), kInputTapeMagic.end(), bytes.begin())) {
        return InputTapeError::BadMagic;
    }

    const std::uint16_t majorVersion = read_u16(bytes.data() + 8);
    const std::uint16_t minorVersion = read_u16(bytes.data() + 10);
    const bool legacy =
        majorVersion == kLegacyMajorVersion && minorVersion <= kLegacyMaximumMinorVersion;
    const bool compressed =
        majorVersion == kInputTapeMajorVersion && minorVersion == kInputTapeMinorVersion;
    if (!legacy && !compressed) {
        return InputTapeError::UnsupportedVersion;
    }
    const std::size_t headerSize = legacy ? kLegacyHeaderSize : kInputTapeHeaderSize;
    if (bytes.size() < headerSize) {
        return InputTapeError::Truncated;
    }
    if (read_u16(bytes.data() + 12) != headerSize) {
        return InputTapeError::InvalidHeaderSize;
    }
    if (read_u16(bytes.data() + 14) != kInputFrameSize) {
        return InputTapeError::InvalidFrameSize;
    }

    InputTape decoded;
    decoded.tickRateNumerator = read_u32(bytes.data() + 16);
    decoded.tickRateDenominator = read_u32(bytes.data() + 20);
    if (decoded.tickRateNumerator == 0 || decoded.tickRateDenominator == 0) {
        return InputTapeError::InvalidTickRate;
    }

    const std::uint64_t frameCount = read_u64(bytes.data() + 24);
    if (frameCount > std::numeric_limits<std::size_t>::max() / kInputFrameSize) {
        return InputTapeError::TooManyFrames;
    }
    const std::size_t expandedSize = static_cast<std::size_t>(frameCount) * kInputFrameSize;

    if (legacy) {
        if (expandedSize > std::numeric_limits<std::size_t>::max() - kLegacyHeaderSize) {
            return InputTapeError::TooManyFrames;
        }
        const std::size_t expectedSize = kLegacyHeaderSize + expandedSize;
        if (bytes.size() < expectedSize) {
            return InputTapeError::Truncated;
        }
        if (bytes.size() != expectedSize) {
            return InputTapeError::TrailingData;
        }
        const InputTapeError frameError =
            decode_frame_stream(bytes.subspan(kLegacyHeaderSize, expandedSize),
                static_cast<std::size_t>(frameCount), minorVersion, decoded);
        if (frameError != InputTapeError::None) {
            return frameError;
        }
        output = std::move(decoded);
        return InputTapeError::None;
    }

    const std::uint64_t payloadSize64 = read_u64(bytes.data() + 32);
    if (payloadSize64 > std::numeric_limits<std::size_t>::max()) {
        return InputTapeError::TooManyFrames;
    }
    const std::size_t payloadSize = static_cast<std::size_t>(payloadSize64);
    const std::size_t availablePayload = bytes.size() - kInputTapeHeaderSize;
    if (payloadSize > availablePayload) {
        return InputTapeError::Truncated;
    }
    if (payloadSize < availablePayload) {
        return InputTapeError::TrailingData;
    }

    const std::span<const std::uint8_t> payload = bytes.subspan(kInputTapeHeaderSize, payloadSize);
    const std::size_t zstdFrameSize = ZSTD_findFrameCompressedSize(payload.data(), payload.size());
    if (ZSTD_isError(zstdFrameSize) || zstdFrameSize != payload.size()) {
        return InputTapeError::InvalidCompressedPayload;
    }
    const unsigned long long zstdContentSize =
        ZSTD_getFrameContentSize(payload.data(), payload.size());
    if (zstdContentSize == ZSTD_CONTENTSIZE_ERROR || zstdContentSize == ZSTD_CONTENTSIZE_UNKNOWN ||
        zstdContentSize != expandedSize)
    {
        return InputTapeError::InvalidCompressedPayload;
    }

    std::vector<std::uint8_t> expanded(expandedSize);
    std::uint8_t emptyDestination = 0;
    void* destination = expanded.empty() ? static_cast<void*>(&emptyDestination) : expanded.data();
    const std::size_t decompressedSize =
        ZSTD_decompress(destination, expanded.size(), payload.data(), payload.size());
    if (ZSTD_isError(decompressedSize) || decompressedSize != expanded.size()) {
        return InputTapeError::InvalidCompressedPayload;
    }
    const InputTapeError frameError = decode_frame_stream(
        expanded, static_cast<std::size_t>(frameCount), kLegacyMaximumMinorVersion, decoded);
    if (frameError != InputTapeError::None) {
        return frameError;
    }

    output = std::move(decoded);
    return InputTapeError::None;
}

InputTapeError encode_input_tape(const InputTape& tape, std::vector<std::uint8_t>& output) {
    const InputTapeError validationError = validate_input_tape(tape);
    if (validationError != InputTapeError::None) {
        return validationError;
    }
    if (tape.frames.size() >
        (std::numeric_limits<std::size_t>::max() - kInputTapeHeaderSize) / kInputFrameSize)
    {
        return InputTapeError::TooManyFrames;
    }

    std::vector<std::uint8_t> expanded(tape.frames.size() * kInputFrameSize);
    encode_frame_stream(tape, expanded);

    const std::size_t compressionBound = ZSTD_compressBound(expanded.size());
    if (ZSTD_isError(compressionBound) ||
        compressionBound > std::numeric_limits<std::size_t>::max() - kInputTapeHeaderSize)
    {
        return InputTapeError::TooManyFrames;
    }
    std::vector<std::uint8_t> compressed(compressionBound);
    const std::uint8_t emptySource = 0;
    const void* source =
        expanded.empty() ? static_cast<const void*>(&emptySource) : expanded.data();
    const std::size_t compressedSize = ZSTD_compress(
        compressed.data(), compressed.size(), source, expanded.size(), ZSTD_CLEVEL_DEFAULT);
    if (ZSTD_isError(compressedSize)) {
        return InputTapeError::InvalidCompressedPayload;
    }
    compressed.resize(compressedSize);

    std::vector<std::uint8_t> encoded(kInputTapeHeaderSize + compressed.size(), 0);
    std::copy(kInputTapeMagic.begin(), kInputTapeMagic.end(), encoded.begin());
    write_u16(encoded.data() + 8, kInputTapeMajorVersion);
    write_u16(encoded.data() + 10, kInputTapeMinorVersion);
    write_u16(encoded.data() + 12, static_cast<std::uint16_t>(kInputTapeHeaderSize));
    write_u16(encoded.data() + 14, static_cast<std::uint16_t>(kInputFrameSize));
    write_u32(encoded.data() + 16, tape.tickRateNumerator);
    write_u32(encoded.data() + 20, tape.tickRateDenominator);
    write_u64(encoded.data() + 24, tape.frames.size());
    write_u64(encoded.data() + 32, compressed.size());
    std::copy(compressed.begin(), compressed.end(), encoded.begin() + kInputTapeHeaderSize);

    output = std::move(encoded);
    return InputTapeError::None;
}

void InputTapePlayer::install(InputTape tape) {
    stop();
    mTape = std::move(tape);
}

InputTapeError InputTapePlayer::install(const std::span<const std::uint8_t> bytes) {
    InputTape tape;
    const InputTapeError error = decode_input_tape(bytes, tape);
    if (error == InputTapeError::None) {
        install(std::move(tape));
    }
    return error;
}

bool InputTapePlayer::start(const TapeEndBehavior endBehavior) {
    releaseOwnedPorts();
    mNextFrame = 0;
    mEndBehavior = endBehavior;
    mReleasePending = false;
    mConditionWaitTicks = 0;
    mConditionPulseNeutral = false;
    mPlaybackError = InputTapePlaybackError::None;
    mFailedFrame = 0;
    mFailedCondition = InputFrameCondition::None;
    mPlaying = !mTape.frames.empty();
    return mPlaying;
}

void InputTapePlayer::stop() {
    releaseOwnedPorts();
    mNextFrame = 0;
    mPlaying = false;
    mReleasePending = false;
    mConditionWaitTicks = 0;
    mConditionPulseNeutral = false;
    mPlaybackError = InputTapePlaybackError::None;
    mFailedFrame = 0;
    mFailedCondition = InputFrameCondition::None;
}

void InputTapePlayer::handoffToLiveInput() {
    releaseOwnedPorts();
    mReleasePending = false;
}

void InputTapePlayer::tick() {
    if (mReleasePending) {
        releaseOwnedPorts();
        mReleasePending = false;
    }
    if (!mPlaying) {
        return;
    }

    std::size_t satisfiedConditions = 0;
    while (mPlaying) {
        const InputFrame& frame = mTape.frames[mNextFrame];
        if (frame.condition != InputFrameCondition::None) {
            if (conditionSatisfied(frame.condition)) {
                mConditionWaitTicks = 0;
                mConditionPulseNeutral = false;
                advanceFrame();
                ++satisfiedConditions;
                if (mPlaying && satisfiedConditions >= mTape.frames.size()) {
                    // A looping tape made entirely of already-satisfied gates
                    // has no input frame to consume this tick. Keep ownership
                    // neutral and resume from the wrapped frame next tick.
                    applyNeutral(mTape.frames[mNextFrame].ownedPorts);
                    return;
                }
                continue;
            }

            if (mConditionPulseNeutral) {
                applyNeutral(frame.ownedPorts);
            } else {
                apply(frame);
            }
            mConditionPulseNeutral = !mConditionPulseNeutral;
            ++mConditionWaitTicks;
            if (mConditionWaitTicks >= frame.timeoutTicks) {
                mPlaying = false;
                mPlaybackError = InputTapePlaybackError::ConditionTimedOut;
                mFailedFrame = mNextFrame;
                mFailedCondition = frame.condition;
            }
            return;
        }

        apply(frame);
        advanceFrame();
        return;
    }
}

void InputTapePlayer::advanceFrame() {
    ++mNextFrame;
    if (mNextFrame < mTape.frames.size()) {
        return;
    }

    switch (mEndBehavior) {
    case TapeEndBehavior::Release:
        mPlaying = false;
        mReleasePending = true;
        break;
    case TapeEndBehavior::Hold:
        mPlaying = false;
        break;
    case TapeEndBehavior::Loop:
        mNextFrame = 0;
        mConditionWaitTicks = 0;
        mConditionPulseNeutral = false;
        break;
    }
}

const char* input_tape_playback_error_message(const InputTapePlaybackError error) {
    switch (error) {
    case InputTapePlaybackError::None:
        return "no error";
    case InputTapePlaybackError::ConditionTimedOut:
        return "input tape condition timed out";
    }
    return "unknown input tape playback error";
}

void InputTapePlayer::apply(const InputFrame& frame) {
    for (std::size_t port = 0; port < kInputPortCount; ++port) {
        const std::uint8_t portBit = static_cast<std::uint8_t>(1u << port);
        if ((frame.ownedPorts & portBit) != 0) {
            const PADStatus status = to_pad_status(frame.pads[port]);
            PADSetAutomationStatus(static_cast<u32>(port), &status);
        } else if ((mOwnedPorts & portBit) != 0) {
            PADClearAutomationStatus(static_cast<u32>(port));
        }
    }
    mOwnedPorts = frame.ownedPorts;
}

void InputTapePlayer::applyNeutral(const std::uint8_t ownedPorts) {
    InputFrame neutral;
    neutral.ownedPorts = ownedPorts;
    apply(neutral);
}

bool InputTapePlayer::conditionSatisfied(const InputFrameCondition condition) const {
    switch (condition) {
    case InputFrameCondition::None:
        return true;
    case InputFrameCondition::NameEntryActive:
        return name_entry_observer().latest().active != 0;
    case InputFrameCondition::NameEntryCharacterSelect: {
        const NameEntryObservation& observation = name_entry_observer().latest();
        return observation.active != 0 && name_entry_observer().inputProcessed() &&
               observation.selectionProcedure == 0;
    }
    case InputFrameCondition::NameEntryInputReady: {
        const NameEntryObservation& observation = name_entry_observer().latest();
        return observation.active != 0 && name_entry_observer().inputProcessed() &&
               (observation.selectionProcedure == 0 || observation.selectionProcedure == 4);
    }
    case InputFrameCondition::FileSelectNoSaveReady:
        return file_select_observer().noSavePromptReady();
    case InputFrameCondition::FileSelectDataSelectReady:
        return file_select_observer().dataSelectReady();
    case InputFrameCondition::FileSelectAcceptReady:
        return file_select_observer().acceptReady();
    }
    return false;
}

void InputTapePlayer::releaseOwnedPorts() {
    for (std::size_t port = 0; port < kInputPortCount; ++port) {
        if ((mOwnedPorts & (1u << port)) != 0) {
            PADClearAutomationStatus(static_cast<u32>(port));
        }
    }
    mOwnedPorts = 0;
}

InputTapePlayer& input_tape_player() {
    static InputTapePlayer player;
    return player;
}

InputTapeError InputTapeRecorder::arm(const std::uint8_t ownedPorts,
                                      const std::size_t frameCapacity,
                                      const std::uint32_t tickRateNumerator,
                                      const std::uint32_t tickRateDenominator) {
    mTape = {};
    mFrameCapacity = 0;
    mOwnedPorts = 0;
    mArmed = false;
    mRecording = false;
    mCapacityExhausted = false;
    if ((ownedPorts & ~kAllPortsMask) != 0) {
        return InputTapeError::InvalidOwnedPorts;
    }
    if (tickRateNumerator == 0 || tickRateDenominator == 0) {
        return InputTapeError::InvalidTickRate;
    }
    if (frameCapacity > mTape.frames.max_size()) {
        return InputTapeError::TooManyFrames;
    }

    mTape = {};
    mTape.tickRateNumerator = tickRateNumerator;
    mTape.tickRateDenominator = tickRateDenominator;
    mTape.frames.reserve(frameCapacity);
    mFrameCapacity = frameCapacity;
    mOwnedPorts = ownedPorts;
    mArmed = true;
    return InputTapeError::None;
}

bool InputTapeRecorder::begin() {
    if (!mArmed || mCapacityExhausted) {
        return false;
    }
    mRecording = true;
    return true;
}

InputTapeError InputTapeRecorder::start(const std::uint8_t ownedPorts,
                                        const std::size_t frameCapacity,
                                        const std::uint32_t tickRateNumerator,
                                        const std::uint32_t tickRateDenominator) {
    const InputTapeError error =
        arm(ownedPorts, frameCapacity, tickRateNumerator, tickRateDenominator);
    if (error != InputTapeError::None) {
        return error;
    }
    begin();
    return InputTapeError::None;
}

InputRecordResult InputTapeRecorder::recordTick(
    const std::span<const PADStatus, kInputPortCount> statuses) {
    if (!mRecording) {
        return InputRecordResult::Inactive;
    }
    if (mTape.frames.size() >= mFrameCapacity) {
        mRecording = false;
        mCapacityExhausted = true;
        return InputRecordResult::CapacityExhausted;
    }

    InputFrame frame;
    frame.ownedPorts = mOwnedPorts;
    for (std::size_t port = 0; port < kInputPortCount; ++port) {
        frame.pads[port] = from_pad_status(statuses[port]);
    }
    mTape.frames.push_back(frame);
    return InputRecordResult::Recorded;
}

void InputTapeRecorder::stop() {
    mRecording = false;
}

InputTape InputTapeRecorder::take() {
    stop();
    InputTape tape = std::move(mTape);
    mTape = {};
    mFrameCapacity = 0;
    mOwnedPorts = 0;
    mArmed = false;
    mCapacityExhausted = false;
    return tape;
}

InputTapeRecorder& input_tape_recorder() {
    static InputTapeRecorder recorder;
    return recorder;
}

} // namespace dusk::automation
