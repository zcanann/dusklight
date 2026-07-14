#pragma once

#include <atomic>

namespace dusk::automation {

// Fixed-step tape playback cannot allow host worker scheduling to decide which
// simulation frame observes an I/O completion. Keep this process-local: normal
// gameplay retains the original asynchronous worker behavior.
inline std::atomic_bool synchronousIoEnabled = false;

inline void set_synchronous_io_enabled(const bool enabled) {
    synchronousIoEnabled.store(enabled, std::memory_order_release);
}

[[nodiscard]] inline bool synchronous_io_enabled() {
    return synchronousIoEnabled.load(std::memory_order_acquire);
}

}  // namespace dusk::automation
