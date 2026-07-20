#include "dusk/automation/jut_gamepad_state.hpp"

#include <algorithm>

namespace dusk::automation {

bool capture_jut_gamepad_state(JUTGamePadState& state) {
    std::ranges::copy(JUTGamePad::mPadStatus, state.padStatus.begin());
    std::ranges::copy(JUTGamePad::mPadButton, state.padButton.begin());
    std::ranges::copy(JUTGamePad::mPadMStick, state.mainStick.begin());
    std::ranges::copy(JUTGamePad::mPadSStick, state.subStick.begin());
    std::ranges::copy(JUTGamePad::mPadAssign, state.padAssign.begin());
    std::ranges::copy(JUTGamePad::CRumble::mStatus, state.rumbleStatus.begin());
    state.listInitialized = JUTGamePad::mListInitialized;
    state.stickMode = JUTGamePad::sStickMode;
    state.clampMode = JUTGamePad::sClampMode;
    state.suppressPadReset = JUTGamePad::sSuppressPadReset;
    state.analogMode = JUTGamePad::sAnalogMode;
    state.rumbleSupported = JUTGamePad::sRumbleSupported;
    state.rumbleEnabled = JUTGamePad::CRumble::mEnabled;
    state.resetCallback = JUTGamePad::C3ButtonReset::sCallback;
    state.resetCallbackArgument = JUTGamePad::C3ButtonReset::sCallbackArg;
    state.resetThreshold = JUTGamePad::C3ButtonReset::sThreshold;
    state.resetSwitchPushing = JUTGamePad::C3ButtonReset::sResetSwitchPushing;
    state.resetOccurred = JUTGamePad::C3ButtonReset::sResetOccurred;
    state.resetOccurredPort = JUTGamePad::C3ButtonReset::sResetOccurredPort;
    state.stickPressPoint = JUTGamePad::CStick::sPressPoint;
    state.stickReleasePoint = JUTGamePad::CStick::sReleasePoint;
    return true;
}

bool restore_jut_gamepad_state(const JUTGamePadState& state) {
    if (JUTGamePad::mListInitialized != state.listInitialized ||
        JUTGamePad::C3ButtonReset::sCallback != state.resetCallback ||
        JUTGamePad::C3ButtonReset::sCallbackArg != state.resetCallbackArgument)
    {
        return false;
    }
    std::ranges::copy(state.padStatus, JUTGamePad::mPadStatus);
    std::ranges::copy(state.padButton, JUTGamePad::mPadButton);
    std::ranges::copy(state.mainStick, JUTGamePad::mPadMStick);
    std::ranges::copy(state.subStick, JUTGamePad::mPadSStick);
    std::ranges::copy(state.padAssign, JUTGamePad::mPadAssign);
    std::ranges::copy(state.rumbleStatus, JUTGamePad::CRumble::mStatus);
    JUTGamePad::sStickMode = state.stickMode;
    JUTGamePad::sClampMode = state.clampMode;
    JUTGamePad::sSuppressPadReset = state.suppressPadReset;
    JUTGamePad::sAnalogMode = state.analogMode;
    JUTGamePad::sRumbleSupported = state.rumbleSupported;
    JUTGamePad::CRumble::mEnabled = state.rumbleEnabled;
    JUTGamePad::C3ButtonReset::sThreshold = state.resetThreshold;
    JUTGamePad::C3ButtonReset::sResetSwitchPushing = state.resetSwitchPushing;
    JUTGamePad::C3ButtonReset::sResetOccurred = state.resetOccurred;
    JUTGamePad::C3ButtonReset::sResetOccurredPort = state.resetOccurredPort;
    JUTGamePad::CStick::sPressPoint = state.stickPressPoint;
    JUTGamePad::CStick::sReleasePoint = state.stickReleasePoint;
    return true;
}

}  // namespace dusk::automation
