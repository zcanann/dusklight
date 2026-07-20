#ifndef DUSK_AUTOMATION_VI_STATE_HPP
#define DUSK_AUTOMATION_VI_STATE_HPP

#include <dolphin/vi.h>

namespace dusk::automation {

struct VIState {
    u32 retraceCount = 0;
    VIRetraceCallback preRetraceCallback = nullptr;
    VIRetraceCallback postRetraceCallback = nullptr;
};

/** Captures the PC substrate's emulated VI counter and callback identity. */
[[nodiscard]] bool capture_vi_state(VIState& state);
/** Restores the counter only when callback ownership is unchanged. */
[[nodiscard]] bool restore_vi_state(const VIState& state);

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_VI_STATE_HPP
