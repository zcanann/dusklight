#pragma once

#include "document.hpp"

#include <chrono>
#include <cstdint>

namespace dusk::ui {

// Renderer-only warmup is driven by the host loop while emulated time is gated.
// Keep this state outside the game so displaying progress cannot affect playback.
void set_pipeline_warmup_active(bool active) noexcept;

// Host-only recording handoff state. A value of zero hides the overlay.
void set_recording_handoff_countdown(std::uint8_t remainingSeconds) noexcept;

class Overlay : public Document {
public:
    Overlay();

    void show() override;
    void update() override;

protected:
    bool handle_nav_command(Rml::Event& event, NavCommand cmd) override;

private:
    void update_pipeline_progress();
    void update_recording_handoff_countdown();

    Rml::Element* mFpsCounter = nullptr;
    Rml::Element* mPipelineProgress = nullptr;
    Rml::Element* mPipelineProgressLabel = nullptr;
    Rml::Element* mPipelineProgressBar = nullptr;
    Rml::Element* mRecordingHandoffCountdown = nullptr;
    Rml::Element* mRecordingHandoffCountdownValue = nullptr;
    Rml::Element* mCurrentToast = nullptr;
    Rml::Element* mControllerWarning = nullptr;
    Rml::Element* mMenuNotification = nullptr;
    Rml::Element* mSpeedrunTimer = nullptr;
    Rml::Element* mSpeedrunRta = nullptr;
    Rml::Element* mSpeedrunIgt = nullptr;
    clock::time_point mCurrentToastStartTime;
    clock::time_point mMenuNotificationStartTime;
    clock::time_point mPipelineProgressStartTime;
    Uint64 mFpsLastUpdate = 0;
    uint32_t mPipelineBatchCreatedBase = 0;
    uint32_t mPipelineBatchTotal = 0;
    uint32_t mLastQueuedPipelines = 0;
    bool mPipelineProgressActive = false;
};

}  // namespace dusk::ui
