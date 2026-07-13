#include "dusk/automation/name_entry_trace.hpp"

#include <array>
#include <fstream>
#include <string_view>

#include <nlohmann/json.hpp>

namespace dusk::automation {
namespace {

using json = nlohmann::ordered_json;

std::string_view fidelity_profile_name(const NameEntryFidelityProfile profile) {
    switch (profile) {
    case NameEntryFidelityProfile::ObserveOnly:
        return "observe_only";
    case NameEntryFidelityProfile::CursorBreakoutShadow:
        return "cursor_breakout_shadow";
    }
    return "unknown";
}

std::string_view event_kind_name(const NameEntryEventKind kind) {
    switch (kind) {
    case NameEntryEventKind::SessionStarted:
        return "session_started";
    case NameEntryEventKind::SessionEnded:
        return "session_ended";
    case NameEntryEventKind::CursorMoveAttempt:
        return "cursor_move_attempt";
    case NameEntryEventKind::CharacterWriteAttempt:
        return "character_write_attempt";
    case NameEntryEventKind::CharacterReadBlocked:
        return "character_read_blocked";
    }
    return "unknown";
}

json character_json(const NameEntryCharacterObservation& character) {
    return {
        {"column", character.column},
        {"row", character.row},
        {"character_set", character.characterSet},
        {"active", character.active != 0},
        {"character", character.character},
    };
}

json event_json(const NameEntryEvent& event) {
    json flags = json::array();
    if ((event.flags & NameEntryEventAccepted) != 0) {
        flags.push_back("accepted");
    }
    if ((event.flags & NameEntryEventOutOfRange) != 0) {
        flags.push_back("out_of_range");
    }
    if ((event.flags & NameEntryEventShadowModeled) != 0) {
        flags.push_back("shadow_modeled");
    }

    return {
        {"sequence", event.sequence},
        {"sim_tick", event.simTick},
        {"tape_frame", event.tapeFrame},
        {"kind", event_kind_name(event.kind)},
        {"flags_raw", event.flags},
        {"flags", std::move(flags)},
        {"cursor_before", event.cursorBefore},
        {"cursor_requested", event.cursorRequested},
        {"visual_cursor", event.visualCursor},
        {"character_index", event.characterIndex},
        {"original_offset", event.originalOffset},
        {"access_size", event.accessSize},
        {"direction", event.direction},
        {"character", event.character},
    };
}

json write_observation_json(const NameEntryWriteObservation& write) {
    json bytes = json::array();
    for (const std::uint8_t byte : write.bytes) {
        bytes.push_back(byte);
    }
    return {
        {"attempt", write.attempt},
        {"sim_tick", write.simTick},
        {"tape_frame", write.tapeFrame},
        {"character_index", write.characterIndex},
        {"original_offset", write.originalOffset},
        {"flags_raw", write.flags},
        {"bytes", std::move(bytes)},
    };
}

} // namespace

NameEntryTraceArtifact drain_name_entry_trace(NameEntryObserver& observer) {
    NameEntryTraceArtifact artifact;
    artifact.fidelityProfile = observer.fidelityProfile();
    artifact.capabilities = observer.capabilities();
    artifact.snapshot = observer.latest();
    artifact.droppedEventCount = observer.droppedEventCount();
    artifact.events.resize(observer.eventCount());
    const std::size_t drained = observer.drainEvents(artifact.events);
    artifact.events.resize(drained);
    return artifact;
}

std::string serialize_name_entry_trace(const NameEntryTraceArtifact& artifact) {
    const NameEntryObservation& snapshot = artifact.snapshot;

    json characters = json::array();
    for (const NameEntryCharacterObservation& character : snapshot.characters) {
        characters.push_back(character_json(character));
    }

    json shadowBytes = json::array();
    for (const std::uint8_t byte : snapshot.modeledRetailBytes) {
        shadowBytes.push_back(byte);
    }

    json events = json::array();
    for (const NameEntryEvent& event : artifact.events) {
        events.push_back(event_json(event));
    }

    json root = {
        {"schema", {{"name", "dusklight.name_entry_trace"},
                    {"version", NameEntryTraceSchemaVersion}}},
        {"fidelity_profile", fidelity_profile_name(artifact.fidelityProfile)},
        {"capabilities_raw", static_cast<std::uint32_t>(artifact.capabilities)},
        {"snapshot",
         {
             {"revision", snapshot.revision},
             {"sim_tick", snapshot.simTick},
             {"tape_frame", snapshot.tapeFrame},
             {"active", snapshot.active != 0},
             {"logical_cursor", snapshot.logicalCursor},
             {"last_logical_cursor", snapshot.lastLogicalCursor},
             {"visual_cursor", snapshot.visualCursor},
             {"name_length", snapshot.nameLength},
             {"selection_procedure", snapshot.selectionProcedure},
             {"character_column", snapshot.characterColumn},
             {"character_row", snapshot.characterRow},
             {"character_set", snapshot.characterSet},
             {"counters",
              {
                  {"out_of_range_move_attempts", snapshot.outOfRangeMoveAttempts},
                  {"out_of_range_write_attempts", snapshot.outOfRangeWriteAttempts},
                  {"blocked_read_attempts", snapshot.blockedReadAttempts},
              }},
             {"characters", std::move(characters)},
             {"last_write", write_observation_json(snapshot.lastWrite)},
             {"shadow",
              {
                  {"original_offset", NameEntryOriginalLayout::NeighborWindow},
                  {"end_offset", NameEntryOriginalLayout::EyeShredderWindowEnd},
                  {"bytes", std::move(shadowBytes)},
              }},
         }},
        {"event_stream",
         {
             {"dropped_count", artifact.droppedEventCount},
             {"drained_count", artifact.events.size()},
             {"events", std::move(events)},
         }},
    };

    return root.dump(2) + '\n';
}

bool write_name_entry_trace(const std::filesystem::path& path,
                            const NameEntryTraceArtifact& artifact,
                            std::string& error) noexcept {
    try {
        std::ofstream output(path, std::ios::binary | std::ios::trunc);
        if (!output) {
            error = "could not open output file";
            return false;
        }
        const std::string encoded = serialize_name_entry_trace(artifact);
        output.write(encoded.data(), static_cast<std::streamsize>(encoded.size()));
        output.flush();
        if (!output) {
            error = "failed while writing output file";
            return false;
        }
        return true;
    } catch (const std::exception& exception) {
        error = exception.what();
        return false;
    }
}

} // namespace dusk::automation
