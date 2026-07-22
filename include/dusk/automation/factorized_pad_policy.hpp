#pragma once

#include "dusk/automation/input_tape.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <span>
#include <string>

namespace dusk::automation {

inline constexpr const char* kFactorizedPadPolicyHeadSchema =
    "dusklight-factorized-pad-policy-head/v1";
inline constexpr std::size_t kFactorizedPadPolicyHeadWidth = 25;
inline constexpr std::uint32_t kMaximumFactorizedPadDuration = 4096;
// SHA-256 of the v1 head with width 25, one-tick duration and a zero button
// threshold. Native online inference admits exactly this stateless decoder.
inline constexpr std::array<std::uint8_t, 32> kOnlineFactorizedPadActionSchemaDigest{
    0x48, 0x39, 0x53, 0xb6, 0x63, 0xe3, 0x27, 0xc6,
    0xd8, 0x39, 0xd8, 0x8b, 0x72, 0xce, 0x65, 0x2c,
    0xfc, 0xc5, 0xd0, 0xae, 0xb4, 0x20, 0x57, 0x0e,
    0x0c, 0x07, 0x6a, 0x4c, 0x62, 0xe3, 0xab, 0x7c,
};

struct FactorizedPadPolicyHeadConfig {
    std::uint32_t maximumDurationTicks = 1;
    float buttonLogitThreshold = 0.0F;
};

struct FactorizedPadPolicyDecision {
    RawPadState pad{};
    std::uint32_t durationTicks = 0;
};

[[nodiscard]] inline bool decode_factorized_pad_policy(
    const FactorizedPadPolicyHeadConfig& config, const std::span<const float> output,
    FactorizedPadPolicyDecision& decision, std::string& error)
{
    if (config.maximumDurationTicks == 0 ||
        config.maximumDurationTicks > kMaximumFactorizedPadDuration ||
        !std::isfinite(config.buttonLogitThreshold))
    {
        error = "invalid factorized PAD policy head";
        return false;
    }
    if (output.size() != kFactorizedPadPolicyHeadWidth ||
        std::any_of(output.begin(), output.end(), [](const float value) {
            return !std::isfinite(value);
        }))
    {
        error = "invalid factorized PAD policy output";
        return false;
    }

    const auto signedAxis = [](const float raw) {
        const float value = std::clamp(raw, -1.0F, 1.0F);
        const float scaled = value < 0.0F ? value * 128.0F : value * 127.0F;
        return static_cast<std::int8_t>(std::round(scaled));
    };
    const auto unitByte = [](const float raw) {
        return static_cast<std::uint8_t>(std::round(std::clamp(raw, 0.0F, 1.0F) * 255.0F));
    };

    RawPadState pad{};
    pad.stickX = signedAxis(output[0]);
    pad.stickY = signedAxis(output[1]);
    pad.substickX = signedAxis(output[2]);
    pad.substickY = signedAxis(output[3]);
    pad.triggerLeft = unitByte(output[4]);
    pad.triggerRight = unitByte(output[5]);
    pad.analogA = unitByte(output[6]);
    pad.analogB = unitByte(output[7]);
    for (std::size_t bit = 0; bit < 16; ++bit) {
        if (output[8 + bit] > config.buttonLogitThreshold) {
            pad.buttons |= static_cast<std::uint16_t>(std::uint16_t{1} << bit);
        }
    }
    pad.flags = RawPadFlags::Connected;
    pad.error = 0;

    const std::uint32_t duration = config.maximumDurationTicks == 1
        ? 1
        : static_cast<std::uint32_t>(std::round(
              std::clamp(output[24], 0.0F, 1.0F) *
              static_cast<float>(config.maximumDurationTicks - 1))) +
            1;
    decision = {.pad = pad, .durationTicks = duration};
    error.clear();
    return true;
}

}  // namespace dusk::automation
