#include "dusk/automation/name_entry_trace.hpp"

#include <array>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>

#include <nlohmann/json.hpp>

namespace {

void require(bool condition, const char* expression, int line) {
    if (!condition) {
        std::cerr << "name_entry_trace_test.cpp:" << line << ": check failed: "
                  << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

} // namespace

int main() {
    using namespace dusk::automation;

    NameEntryObserver observer;
    observer.setFidelityProfile(NameEntryFidelityProfile::CursorBreakoutShadow);
    observer.beginSession();
    std::array<NameEntryCharacterObservation, NameEntryOriginalLayout::CharacterCount> characters{};
    observer.observe(9, 8, 8, 0, 2, 3, 4, characters);
    observer.noteCursorMove(8, 9, 1, true);
    REQUIRE(observer.noteCharacterWrite(9, 2, 3, 4, 0x11223344, true, true));

    const NameEntryTraceArtifact artifact = drain_name_entry_trace(observer);
    REQUIRE(observer.eventCount() == 0);
    REQUIRE(artifact.events.size() == 3);

    const auto root = nlohmann::json::parse(serialize_name_entry_trace(artifact));
    REQUIRE(root["schema"]["version"] == 1);
    REQUIRE(root["fidelity_profile"] == "cursor_breakout_shadow");
    REQUIRE(root["snapshot"]["logical_cursor"] == 9);
    REQUIRE(root["snapshot"]["counters"]["out_of_range_move_attempts"] == 1);
    REQUIRE(root["snapshot"]["counters"]["out_of_range_write_attempts"] == 1);
    REQUIRE(root["snapshot"]["shadow"]["bytes"][8] == 2);
    REQUIRE(root["snapshot"]["shadow"]["bytes"][15] == 0x44);
    REQUIRE(root["event_stream"]["dropped_count"] == 0);
    REQUIRE(root["event_stream"]["drained_count"] == 3);
    REQUIRE(root["event_stream"]["events"][2]["kind"] == "character_write_attempt");
    REQUIRE(root["event_stream"]["events"][2]["original_offset"] == 0x314);

    const std::filesystem::path outputPath =
        std::filesystem::temp_directory_path() / "dusklight-name-entry-trace-test.json";
    std::string error;
    REQUIRE(write_name_entry_trace(outputPath, artifact, error));
    std::ifstream input(outputPath, std::ios::binary);
    const auto writtenRoot = nlohmann::json::parse(input);
    REQUIRE(writtenRoot == root);
    input.close();
    std::error_code removeError;
    std::filesystem::remove(outputPath, removeError);
    REQUIRE(!removeError);

    std::cout << "name entry trace tests passed\n";
    return 0;
}
