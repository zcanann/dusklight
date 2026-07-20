#pragma once

#include <cstdint>

namespace dusk::automation {

// Fork-only, read-only observation boundary. This translation unit is compiled
// only when DUSK_ENABLE_AUTOMATION_OBSERVERS is enabled. It may copy public game
// state through const access, but it must never call a mutating/lazy game helper
// or expose a live game pointer outside the capture call.
struct GameplayTracePostSimulationContext {
    std::uint64_t simulationTick = 0;
    std::uint64_t tapeFrame = ~std::uint64_t{0};
    std::uint8_t tapeOwnedPorts = 0;
    bool tapeFrameApplied = false;
    bool controllerFrameApplied = false;
};

struct GameplayCollisionCorrectionObservation {
    bool present = false;
    float x = 0.0F;
    float z = 0.0F;
};

bool gameplay_trace_observer_enabled();
GameplayCollisionCorrectionObservation capture_gameplay_collision_correction();
void record_gameplay_trace_post_simulation(const GameplayTracePostSimulationContext& context);

}  // namespace dusk::automation
