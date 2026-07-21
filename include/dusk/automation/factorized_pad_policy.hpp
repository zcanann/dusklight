#pragma once

#include "dusk/automation/input_tape.hpp"

#include <algorithm>
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
