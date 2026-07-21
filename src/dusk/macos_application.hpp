#pragma once

namespace dusk {

// Prevent AppKit's crash-recovery state prompt from blocking a headless child before the
// automation watchdog can observe it. This changes only the current process's volatile defaults.
void ConfigureMacOSHeadlessLaunch();

}  // namespace dusk
