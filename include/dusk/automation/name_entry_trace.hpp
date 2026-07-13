#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

#include "dusk/automation/name_entry_observer.hpp"

namespace dusk::automation {

inline constexpr std::uint32_t NameEntryTraceSchemaVersion = 1;

struct NameEntryTraceArtifact {
    NameEntryFidelityProfile fidelityProfile = NameEntryFidelityProfile::ObserveOnly;
    NameEntryCapability capabilities = NameEntryCapability::None;
    NameEntryObservation snapshot{};
    std::uint64_t droppedEventCount = 0;
    std::vector<NameEntryEvent> events;
};

/** Copies the final snapshot and drains all currently buffered events. */
NameEntryTraceArtifact drain_name_entry_trace(NameEntryObserver& observer);

/** Produces stable, versioned JSON with a trailing newline. */
std::string serialize_name_entry_trace(const NameEntryTraceArtifact& artifact);

/** Writes a serialized trace. Returns false and fills error on failure. */
bool write_name_entry_trace(const std::filesystem::path& path,
                            const NameEntryTraceArtifact& artifact,
                            std::string& error) noexcept;

} // namespace dusk::automation
