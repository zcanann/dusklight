#ifndef DUSK_AUTOMATION_JUT_GAMEPAD_STATE_HPP
#define DUSK_AUTOMATION_JUT_GAMEPAD_STATE_HPP

#include <array>
#include <cstddef>
#include <cstdint>

#include "JSystem/JUtility/JUTGamePad.h"

namespace dusk::automation {

struct JUTGamePadState {
    std::array<PADStatus, 4> padStatus{};
    std::array<JUTGamePad::CButton, 4> padButton{};
    std::array<JUTGamePad::CStick, 4> mainStick{};
    std::array<JUTGamePad::CStick, 4> subStick{};
    std::array<std::uint8_t, 4> padAssign{};
    std::array<std::uint8_t, 4> rumbleStatus{};
    bool listInitialized = false;
    JUTGamePad::EStickMode stickMode = JUTGamePad::EStickMode1;
    int clampMode = JUTGamePad::EClampStick;
    std::uint32_t suppressPadReset = 0;
    std::int32_t analogMode = 0;
    std::uint32_t rumbleSupported = 0;
    std::uint32_t rumbleEnabled = 0;
    callbackFn resetCallback = nullptr;
    void* resetCallbackArgument = nullptr;
    OSTime resetThreshold = 0;
    bool resetSwitchPushing = false;
    bool resetOccurred = false;
    std::int32_t resetOccurredPort = 0;
    float stickPressPoint = 0.0f;
    float stickReleasePoint = 0.0f;
};

[[nodiscard]] bool capture_jut_gamepad_state(JUTGamePadState& state);
/** Restores scalar/history state and rejects changed callback/list ownership. */
[[nodiscard]] bool restore_jut_gamepad_state(const JUTGamePadState& state);

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_JUT_GAMEPAD_STATE_HPP
