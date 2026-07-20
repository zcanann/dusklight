#pragma once

#include "dusk/automation/gameplay_trace.hpp"

#include <array>
#include <cstdint>
#include <span>

#include <dolphin/pad.h>

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

struct GameplayTraceCaptureContext {
    std::uint64_t boundaryIndex = 0;
    std::uint64_t simulationTick = 0;
    std::uint64_t tapeFrame = GameplayTraceNoTapeFrame;
    GameplayTracePhase phase = GameplayTracePhase::PreInput;
    GameplayTraceBoundaryKind boundaryKind = GameplayTraceBoundaryKind::Tick;
    std::uint8_t inputSource = GameplayTraceInputNone;
    std::uint8_t ownedPorts = 0;
};

struct GameplayCollisionCorrectionObservation {
    bool present = false;
    float x = 0.0F;
    float z = 0.0F;
};

struct GameplayCollisionPlanesObservation {
    // Slots match GameplayTracePlayerCollisionSurfacesSample: ground, roof,
    // water, then Link's three wall circles.
    std::uint8_t validMask = 0;
    std::array<std::array<float, 4>, 6> planes{};
};

struct GameplayPlayerFormObservation {
    bool present = false;
    bool wolf = false;
};

bool gameplay_trace_observer_enabled();
/** Retains the exact PADRead result before JUTGamePad applies its non-idempotent clamp. */
void record_gameplay_trace_consumed_pads(
    std::span<const PADStatus, kInputPortCount> statuses);
GameplayCollisionCorrectionObservation capture_gameplay_collision_correction();
GameplayCollisionPlanesObservation capture_gameplay_collision_planes();
GameplayPlayerFormObservation capture_gameplay_player_form();
[[nodiscard]] bool capture_gameplay_trace_sample(const GameplayTraceCaptureContext& context,
    std::uint64_t requestedChannels, GameplayTraceSample& output);
void record_gameplay_trace_post_simulation(const GameplayTracePostSimulationContext& context);

}  // namespace dusk::automation
