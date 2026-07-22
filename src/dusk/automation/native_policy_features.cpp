#include "dusk/automation/native_policy_features.hpp"

#include <cmath>

namespace dusk::automation {
namespace {

float signed_axis(const std::int8_t value) {
    return value < 0 ? static_cast<float>(value) / 128.0F
                     : static_cast<float>(value) / 127.0F;
}

}  // namespace

bool encode_native_policy_features(
    const NativePolicyFeatureInput& input, NativePolicyFeatureRow& output, std::string& error) {
    output.fill(0.0F);
    std::size_t index = 0;
    const auto push = [&](const float value) { output[index++] = value; };
    const auto pushBool = [&](const bool value) { push(value ? 1.0F : 0.0F); };
    const auto pushPlayer = [&](const float value) {
        push(input.playerPresent ? value : 0.0F);
    };

    pushBool(input.playerPresent);
    pushBool(input.playerPresent && input.playerIsLink);
    for (const float value : input.playerPosition) pushPlayer(value);
    for (const float value : input.playerVelocity) pushPlayer(value);
    pushPlayer(input.playerForwardSpeed);
    pushPlayer(static_cast<float>(input.playerCurrentYaw) / 32768.0F);
    pushPlayer(static_cast<float>(input.playerShapeYaw) / 32768.0F);
    for (const bool value : {input.playerGroundContact, input.playerWallContact,
             input.playerRoofContact, input.playerWaterContact, input.playerWaterIn})
        pushBool(input.playerPresent && value);
    const bool groundPresent = input.playerPresent && input.playerGroundHeightPresent;
    pushBool(groundPresent);
    push(groundPresent ? input.playerGroundHeight : 0.0F);
    const bool roofPresent = input.playerPresent && input.playerRoofHeightPresent;
    pushBool(roofPresent);
    push(roofPresent ? input.playerRoofHeight : 0.0F);
    pushBool(input.eventRunning);
    push(static_cast<float>(input.eventMode) / 255.0F);
    push(static_cast<float>(input.eventStatus) / 255.0F);
    push(static_cast<float>(input.eventMapToolId) / 255.0F);
    pushBool(input.nextStageEnabled);
    pushBool(input.cameraPresent);
    push(input.cameraPresent ? input.cameraYawRadians : 0.0F);
    pushBool(input.collisionCorrectionPresent);
    push(input.collisionCorrectionPresent ? input.collisionCorrectionX : 0.0F);
    push(input.collisionCorrectionPresent ? input.collisionCorrectionZ : 0.0F);
    push(static_cast<float>(input.remainingTicks));
    pushBool(has_flag(input.previousInput.flags, RawPadFlags::Connected));
    push(signed_axis(input.previousInput.stickX));
    push(signed_axis(input.previousInput.stickY));
    push(signed_axis(input.previousInput.substickX));
    push(signed_axis(input.previousInput.substickY));
    for (const std::uint8_t value : {input.previousInput.triggerLeft,
             input.previousInput.triggerRight, input.previousInput.analogA,
             input.previousInput.analogB})
        push(static_cast<float>(value) / 255.0F);
    for (std::size_t bit = 0; bit < 16; ++bit)
        pushBool((input.previousInput.buttons & (std::uint16_t{1} << bit)) != 0);
    push(static_cast<float>(input.previousInput.error) / 128.0F);
    pushPlayer(static_cast<float>(input.playerDamageWaitTimer) / 32768.0F);
    pushPlayer(static_cast<float>(input.playerIceDamageWaitTimer) / 32768.0F);
    pushPlayer(static_cast<float>(input.playerSwordChangeWaitTimer) / 255.0F);
    pushPlayer(static_cast<float>(input.playerDoStatus) / 255.0F);
    for (const char byte : input.stageName) {
        const auto value = static_cast<unsigned char>(byte);
        if (value > 0x7f) {
            error = "native policy stage name is not canonical ASCII";
            return false;
        }
        push(static_cast<float>(value) / 127.0F);
    }
    push(static_cast<float>(input.room) / 128.0F);
    push(static_cast<float>(input.layer) / 128.0F);
    push(static_cast<float>(input.point) / 32768.0F);
    for (std::size_t bit = 0; bit < 16; ++bit)
        pushBool(input.playerPresent &&
                 (input.playerProcedure & (std::uint16_t{1} << bit)) != 0);
    for (std::size_t bit = 0; bit < 32; ++bit)
        pushBool(input.playerPresent &&
                 (input.playerModeFlags & (std::uint32_t{1} << bit)) != 0);

    if (index != output.size()) {
        error = "native policy feature schema width drifted";
        return false;
    }
    for (const float value : output) {
        if (!std::isfinite(value)) {
            error = "native policy feature row contains a non-finite value";
            return false;
        }
    }
    error.clear();
    return true;
}

}  // namespace dusk::automation
