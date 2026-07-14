#pragma once

#include "dusk/automation/input_controller.hpp"

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <string>

namespace dusk::automation {

// Match the live controller observation bound: every retained catalog entry is
// eligible for selection under the same lowest-process-ID retention rule.
inline constexpr std::size_t ActorCatalogMaximumEntries = kInputControllerMaximumActors;

// Captures a bounded, read-only snapshot of the live actor list and writes it
// as a versioned JSON artifact. No actor or process field is modified.
bool write_actor_catalog(
    const std::filesystem::path& path, std::uint64_t simulationTick, std::string& error);

}  // namespace dusk::automation
