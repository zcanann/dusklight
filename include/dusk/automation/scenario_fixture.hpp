#pragma once

#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <variant>
#include <vector>

namespace dusk::automation {

inline constexpr char kScenarioFixtureSchema[] = "dusklight-scenario-fixture/v1";
inline constexpr std::uint16_t kScenarioFixtureMajorVersion = 1;
inline constexpr std::uint16_t kScenarioFixtureMinorVersion = 0;
inline constexpr std::size_t kScenarioFixtureHeaderSize = 32;

enum class PlayerFixtureForm : std::uint8_t {
    Human = 0,
    Wolf = 1,
};

enum class FixtureVideoMode : std::uint8_t {
    Automatic = 0,
    NtscInterlaced = 1,
    NtscProgressive = 2,
    Pal50 = 3,
    Pal60 = 4,
};

enum class FixtureRngStream : std::uint8_t {
    Primary = 0,
    Secondary = 1,
};

enum class FixtureFlagDomain : std::uint8_t {
    Event = 0,
    Temporary = 1,
    Dungeon = 2,
    Switch = 3,
};

struct HealthFixture {
    std::uint16_t current = 0;
    std::uint16_t maximum = 0;
    bool operator==(const HealthFixture&) const = default;
};

struct RngFixture {
    FixtureRngStream stream = FixtureRngStream::Primary;
    std::int32_t state0 = 0;
    std::int32_t state1 = 0;
    std::int32_t state2 = 0;
    std::uint64_t callCount = 0;
    bool operator==(const RngFixture&) const = default;
};

struct InventoryFixture {
    std::uint16_t slot = 0;
    std::uint16_t item = 0;
    std::uint16_t quantity = 1;
    bool operator==(const InventoryFixture&) const = default;
};

struct EquipmentFixture {
    std::uint16_t slot = 0;
    std::uint16_t item = 0;
    bool operator==(const EquipmentFixture&) const = default;
};

struct FlagFixture {
    FixtureFlagDomain domain = FixtureFlagDomain::Event;
    std::int8_t room = -1;
    std::uint16_t index = 0;
    bool value = false;
    bool operator==(const FlagFixture&) const = default;
};

struct FixtureFloat {
    // Exact IEEE-754 binary64 representation; non-finite values are invalid.
    std::uint64_t bits = 0;
    bool operator==(const FixtureFloat&) const = default;
};

using FixtureSettingValue = std::variant<bool, std::int64_t, FixtureFloat, std::string>;

struct SettingFixture {
    std::string key;
    FixtureSettingValue value = false;
    bool operator==(const SettingFixture&) const = default;
};

struct ScenarioFixture {
    std::string name;
    std::optional<PlayerFixtureForm> form;
    std::optional<HealthFixture> health;
    std::vector<RngFixture> rng;
    std::optional<FixtureVideoMode> videoMode;
    std::vector<InventoryFixture> inventory;
    std::vector<EquipmentFixture> equipment;
    std::vector<FlagFixture> flags;
    std::vector<SettingFixture> settings;

    bool operator==(const ScenarioFixture&) const = default;
};

enum class ScenarioFixtureError {
    None,
    Truncated,
    BadMagic,
    UnsupportedVersion,
    InvalidHeader,
    InvalidSize,
    InvalidRecordCount,
    InvalidRecord,
    UnknownRecord,
    NonCanonical,
    InvalidName,
    InvalidHealth,
    InvalidInventory,
    InvalidFlag,
    InvalidSetting,
    DuplicateKey,
    LimitExceeded,
};

const char* scenario_fixture_error_message(ScenarioFixtureError error);
ScenarioFixtureError validate_scenario_fixture(const ScenarioFixture& fixture);
ScenarioFixtureError encode_scenario_fixture(
    const ScenarioFixture& fixture, std::vector<std::uint8_t>& output);
ScenarioFixtureError decode_scenario_fixture(
    std::span<const std::uint8_t> bytes, ScenarioFixture& output);

}  // namespace dusk::automation
