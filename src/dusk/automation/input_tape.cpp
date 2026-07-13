#include "dusk/automation/input_tape.hpp"

#include <dolphin/pad.h>

#include <algorithm>
#include <limits>
#include <utility>

namespace dusk::automation {
namespace {

constexpr std::uint8_t kAllPortsMask = (1u << kInputPortCount) - 1u;
constexpr std::uint8_t kKnownPadFlags = static_cast<std::uint8_t>(RawPadFlags::Connected);

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
    output[11] = 0;
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
    status.err = has_flag(input.flags, RawPadFlags::Connected) ? PAD_ERR_NONE : PAD_ERR_NO_CONTROLLER;
    return status;
}

} // namespace

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
    case InputTapeError::InvalidPadFlags:
        return "input tape contains unknown controller flags";
    case InputTapeError::TrailingData:
        return "input tape contains trailing data";
    case InputTapeError::TooManyFrames:
        return "input tape frame count is too large";
    }
    return "unknown input tape error";
}

InputTapeError validate_input_tape(const InputTape& tape) {
    if (tape.tickRateNumerator == 0 || tape.tickRateDenominator == 0) {
        return InputTapeError::InvalidTickRate;
    }

    for (const InputFrame& frame : tape.frames) {
        if ((frame.ownedPorts & ~kAllPortsMask) != 0) {
            return InputTapeError::InvalidOwnedPorts;
        }
        for (const RawPadState& pad : frame.pads) {
            if ((static_cast<std::uint8_t>(pad.flags) & ~kKnownPadFlags) != 0) {
                return InputTapeError::InvalidPadFlags;
            }
        }
    }
    return InputTapeError::None;
}

InputTapeError decode_input_tape(const std::span<const std::uint8_t> bytes, InputTape& output) {
    if (bytes.size() < kInputTapeHeaderSize) {
        return InputTapeError::Truncated;
    }
    if (!std::equal(kInputTapeMagic.begin(), kInputTapeMagic.end(), bytes.begin())) {
        return InputTapeError::BadMagic;
    }

    const std::uint16_t majorVersion = read_u16(bytes.data() + 8);
    const std::uint16_t minorVersion = read_u16(bytes.data() + 10);
    if (majorVersion != kInputTapeMajorVersion || minorVersion != kInputTapeMinorVersion) {
        return InputTapeError::UnsupportedVersion;
    }
    if (read_u16(bytes.data() + 12) != kInputTapeHeaderSize) {
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
    const std::size_t availableFrames = (bytes.size() - kInputTapeHeaderSize) / kInputFrameSize;
    if (frameCount > availableFrames || frameCount > std::numeric_limits<std::size_t>::max()) {
        return InputTapeError::TooManyFrames;
    }
    const std::size_t expectedSize = kInputTapeHeaderSize + static_cast<std::size_t>(frameCount) * kInputFrameSize;
    if (bytes.size() < expectedSize) {
        return InputTapeError::Truncated;
    }
    if (bytes.size() != expectedSize) {
        return InputTapeError::TrailingData;
    }

    decoded.frames.resize(static_cast<std::size_t>(frameCount));
    const std::uint8_t* input = bytes.data() + kInputTapeHeaderSize;
    for (InputFrame& frame : decoded.frames) {
        frame.ownedPorts = input[0];
        if ((frame.ownedPorts & ~kAllPortsMask) != 0) {
            return InputTapeError::InvalidOwnedPorts;
        }
        if (input[1] != 0 || input[2] != 0 || input[3] != 0) {
            return InputTapeError::InvalidFrameSize;
        }
        input += 4;

        for (RawPadState& pad : frame.pads) {
            pad = decode_pad(input);
            if ((static_cast<std::uint8_t>(pad.flags) & ~kKnownPadFlags) != 0 || input[11] != 0) {
                return InputTapeError::InvalidPadFlags;
            }
            input += kRawPadStateSize;
        }
    }

    output = std::move(decoded);
    return InputTapeError::None;
}

InputTapeError encode_input_tape(const InputTape& tape, std::vector<std::uint8_t>& output) {
    const InputTapeError validationError = validate_input_tape(tape);
    if (validationError != InputTapeError::None) {
        return validationError;
    }
    if (tape.frames.size() > (std::numeric_limits<std::size_t>::max() - kInputTapeHeaderSize) / kInputFrameSize) {
        return InputTapeError::TooManyFrames;
    }

    std::vector<std::uint8_t> encoded(kInputTapeHeaderSize + tape.frames.size() * kInputFrameSize, 0);
    std::copy(kInputTapeMagic.begin(), kInputTapeMagic.end(), encoded.begin());
    write_u16(encoded.data() + 8, kInputTapeMajorVersion);
    write_u16(encoded.data() + 10, kInputTapeMinorVersion);
    write_u16(encoded.data() + 12, static_cast<std::uint16_t>(kInputTapeHeaderSize));
    write_u16(encoded.data() + 14, static_cast<std::uint16_t>(kInputFrameSize));
    write_u32(encoded.data() + 16, tape.tickRateNumerator);
    write_u32(encoded.data() + 20, tape.tickRateDenominator);
    write_u64(encoded.data() + 24, tape.frames.size());

    std::uint8_t* destination = encoded.data() + kInputTapeHeaderSize;
    for (const InputFrame& frame : tape.frames) {
        destination[0] = frame.ownedPorts;
        destination += 4;
        for (const RawPadState& pad : frame.pads) {
            encode_pad(pad, destination);
            destination += kRawPadStateSize;
        }
    }

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
    mPlaying = !mTape.frames.empty();
    return mPlaying;
}

void InputTapePlayer::stop() {
    releaseOwnedPorts();
    mNextFrame = 0;
    mPlaying = false;
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

    apply(mTape.frames[mNextFrame]);
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
        break;
    }
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

} // namespace dusk::automation
