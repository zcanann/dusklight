#pragma once

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>

namespace dusk::automation {

inline constexpr std::size_t AutomationCardFixtureMaximumFiles = 64;
inline constexpr std::uint64_t AutomationCardFixtureMaximumBytes = 64u * 1024u * 1024u;
inline constexpr std::string_view EmptyAutomationCardFixtureIdentity = "card-fixture:empty/v1";

struct AutomationCardFixtureResult {
    std::string identity;
    std::size_t fileCount = 0;
    std::uint64_t byteCount = 0;
};

// Copies an immutable, region-shaped GCI fixture tree into a fresh automation
// card root. Any existing destination entry fails closed; automation never
// borrows or overwrites durable card state from an earlier run.
[[nodiscard]] bool materialize_automation_card_fixture(const std::filesystem::path& sourceRoot,
    const std::filesystem::path& destinationCardRoot, AutomationCardFixtureResult& result,
    std::string& error);

void set_active_automation_card_fixture_identity(std::string identity);
std::string_view active_automation_card_fixture_identity();

}  // namespace dusk::automation
