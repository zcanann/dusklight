#include "dusk/automation/input_tape.hpp"

#include <dolphin/pad.h>

namespace dusk::automation {

PADStatus raw_pad_state_to_pad_status(const RawPadState& input) {
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

RawPadState raw_pad_state_from_pad_status(const PADStatus& input) {
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

void encode_raw_pad_state(
    const RawPadState& input, const std::span<std::uint8_t, kRawPadStateSize> output) {
    output[0] = static_cast<std::uint8_t>(input.buttons);
    output[1] = static_cast<std::uint8_t>(input.buttons >> 8);
    output[2] = static_cast<std::uint8_t>(input.stickX);
    output[3] = static_cast<std::uint8_t>(input.stickY);
    output[4] = static_cast<std::uint8_t>(input.substickX);
    output[5] = static_cast<std::uint8_t>(input.substickY);
    output[6] = input.triggerLeft;
    output[7] = input.triggerRight;
    output[8] = input.analogA;
    output[9] = input.analogB;
    output[10] = static_cast<std::uint8_t>(input.flags);
    output[11] = static_cast<std::uint8_t>(input.error);
}

}  // namespace dusk::automation
