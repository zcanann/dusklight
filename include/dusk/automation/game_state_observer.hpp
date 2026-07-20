#pragma once

#include "dusk/automation/eye_shredder_oracle.hpp"
#include "dusk/automation/input_controller.hpp"
#include "dusk/automation/milestones.hpp"

#include <array>
#include <cstddef>
#include <vector>

namespace dusk::automation {

// Fork-owned storage for one bounded controller observation. No game pointer
// escapes the observer boundary; ControllerObservation::actors refers only to
// this copied array.
struct ControllerObservationStorage {
    std::array<ControllerActor, kInputControllerMaximumActors> actors{};
    std::size_t count = 0;
    bool truncated = false;
};

inline constexpr std::size_t kMilestoneEventFlagCount = 822;
inline constexpr std::size_t kMilestoneTemporaryFlagCount = 185;
inline constexpr std::size_t kMilestoneTemporaryEventByteCount = 256;
inline constexpr std::size_t kMilestoneDungeonFlagCount = 64;
inline constexpr std::size_t kMilestoneSwitchFlagCount = 240;

struct MilestoneObservationStorage {
    // Learning observations retain the complete process actor population. The
    // controller's bounded actor view is a separate, explicitly lossy hot-path
    // contract and must never silently constrain the training corpus.
    std::vector<MilestoneObservation::Actor> actors;
    std::uint32_t actorObservedCount = 0;
    std::vector<MilestoneObservation::DynamicCollider> dynamicColliders;
    std::array<std::uint8_t, kMilestoneEventFlagCount> eventFlags{};
    std::array<std::uint8_t, kMilestoneTemporaryFlagCount> temporaryFlags{};
    std::array<std::uint8_t, kMilestoneTemporaryEventByteCount> temporaryEventBytes{};
    std::array<std::uint8_t, kMilestoneDungeonFlagCount> dungeonFlags{};
    std::array<std::uint8_t, kMilestoneSwitchFlagCount> switchFlags{};
};

[[nodiscard]] bool game_state_observers_enabled();

// These functions copy already-realized state only. Their implementation is
// compiled in one explicitly gated translation unit and contains the complete
// field-access audit surface for these legacy automation consumers.
[[nodiscard]] ControllerObservation capture_controller_observation(
    ControllerObservationStorage& storage);
[[nodiscard]] MilestoneObservation capture_milestone_observation(
    MilestoneObservationStorage& storage);
[[nodiscard]] EyeShredderGameplayTelemetry capture_eye_shredder_gameplay_telemetry();

}  // namespace dusk::automation
