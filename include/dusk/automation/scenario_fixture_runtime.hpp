#pragma once

#include "dusk/automation/scenario_fixture.hpp"

#include <cstdint>
#include <array>
#include <optional>
#include <string_view>

namespace dusk::automation {

struct StageBootReadinessObservation {
    std::array<char, 9> stage{};
    bool stagePresent = false;
    std::int8_t room = -1;
    std::int8_t layer = -1;
    std::int16_t point = -1;
    bool playerReady = false;
};

// Narrow bootstrap-only read boundary used before tape tick zero. This is
// separate from the optional general-purpose gameplay observer surface.
StageBootReadinessObservation capture_stage_boot_readiness();

// Installs the fixture bound to the authoritative stage-boot tape and validates every field
// against the native engine limits. A fixture is never partially or silently applied.
bool install_scenario_fixture_runtime(
    const std::optional<ScenarioFixture>& fixture, std::int8_t bootRoom);
void clear_scenario_fixture_runtime();
std::string_view scenario_fixture_runtime_error();

// Ordered boot phases. Startup settings/video run before subsystem initialization; save state
// runs before the play scene is requested; stage and room flags run after their backing stores
// exist but before stage actors are created; RNG is restored at the tick-zero boundary.
bool apply_scenario_fixture_startup();
bool apply_scenario_fixture_save_state();
bool apply_scenario_fixture_stage_flags();
bool apply_scenario_fixture_room_flags(std::int8_t room);
bool establish_scenario_fixture_tick_zero();

}  // namespace dusk::automation
