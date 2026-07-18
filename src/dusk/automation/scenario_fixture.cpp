#include "dusk/automation/scenario_fixture.hpp"

#include <algorithm>
#include <array>
#include <bit>
#include <cmath>
#include <limits>
#include <tuple>

namespace dusk::automation {
namespace {

constexpr std::array<std::uint8_t, 8> kMagic{'D', 'U', 'S', 'K', 'F', 'X', 'T', 'R'};
constexpr std::size_t kRecordHeaderSize = 8;
constexpr std::size_t kMaximumEncodedSize = std::numeric_limits<std::uint16_t>::max();
constexpr std::size_t kMaximumNameBytes = 64;
constexpr std::size_t kMaximumSettingKeyBytes = 96;
constexpr std::size_t kMaximumSettingStringBytes = 1024;
constexpr std::size_t kMaximumInventory = 256;
constexpr std::size_t kMaximumEquipment = 64;
constexpr std::size_t kMaximumFlags = 4096;
constexpr std::size_t kMaximumSettings = 256;

constexpr std::uint16_t kTagName = 1;
constexpr std::uint16_t kTagForm = 2;
constexpr std::uint16_t kTagHealth = 3;
constexpr std::uint16_t kTagRng = 4;
constexpr std::uint16_t kTagVideoMode = 5;
constexpr std::uint16_t kTagInventory = 6;
constexpr std::uint16_t kTagEquipment = 7;
constexpr std::uint16_t kTagFlag = 8;
constexpr std::uint16_t kTagSetting = 9;

std::uint16_t read_u16(const std::uint8_t* bytes) {
    return static_cast<std::uint16_t>(bytes[0]) |
           (static_cast<std::uint16_t>(bytes[1]) << 8);
}

std::uint32_t read_u32(const std::uint8_t* bytes) {
    return static_cast<std::uint32_t>(read_u16(bytes)) |
           (static_cast<std::uint32_t>(read_u16(bytes + 2)) << 16);
}

std::uint64_t read_u64(const std::uint8_t* bytes) {
    return static_cast<std::uint64_t>(read_u32(bytes)) |
           (static_cast<std::uint64_t>(read_u32(bytes + 4)) << 32);
}

std::int32_t read_i32(const std::uint8_t* bytes) {
    return std::bit_cast<std::int32_t>(read_u32(bytes));
}

void write_u16(std::uint8_t* bytes, const std::uint16_t value) {
    bytes[0] = static_cast<std::uint8_t>(value);
    bytes[1] = static_cast<std::uint8_t>(value >> 8);
}

void write_u32(std::uint8_t* bytes, const std::uint32_t value) {
    write_u16(bytes, static_cast<std::uint16_t>(value));
    write_u16(bytes + 2, static_cast<std::uint16_t>(value >> 16));
}

void write_u64(std::uint8_t* bytes, const std::uint64_t value) {
    write_u32(bytes, static_cast<std::uint32_t>(value));
    write_u32(bytes + 4, static_cast<std::uint32_t>(value >> 32));
}

void write_i32(std::uint8_t* bytes, const std::int32_t value) {
    write_u32(bytes, std::bit_cast<std::uint32_t>(value));
}

bool all_zero(const std::span<const std::uint8_t> bytes) {
    return std::ranges::all_of(bytes, [](const std::uint8_t byte) { return byte == 0; });
}

bool valid_name(const std::string& name) {
    return !name.empty() && name.size() <= kMaximumNameBytes && name.front() != ' ' &&
           name.back() != ' ' &&
           std::ranges::all_of(name, [](const unsigned char byte) {
               return byte >= 0x20 && byte <= 0x7e;
           });
}

bool valid_setting_key(const std::string& key) {
    return !key.empty() && key.size() <= kMaximumSettingKeyBytes &&
           std::ranges::all_of(key, [](const unsigned char byte) {
               return (byte >= 'a' && byte <= 'z') || (byte >= 'A' && byte <= 'Z') ||
                      (byte >= '0' && byte <= '9') || byte == '.' || byte == '_' || byte == '-';
           });
}

std::optional<std::size_t> aligned4(const std::size_t value) {
    if (value > std::numeric_limits<std::size_t>::max() - 3) {
        return std::nullopt;
    }
    return (value + 3) & ~std::size_t{3};
}

bool valid_form(const PlayerFixtureForm form) {
    return form == PlayerFixtureForm::Human || form == PlayerFixtureForm::Wolf;
}

bool valid_video_mode(const FixtureVideoMode mode) {
    return mode == FixtureVideoMode::Automatic || mode == FixtureVideoMode::NtscInterlaced ||
           mode == FixtureVideoMode::NtscProgressive || mode == FixtureVideoMode::Pal50 ||
           mode == FixtureVideoMode::Pal60;
}

bool valid_rng_stream(const FixtureRngStream stream) {
    return stream == FixtureRngStream::Primary || stream == FixtureRngStream::Secondary;
}

bool valid_flag_domain(const FixtureFlagDomain domain) {
    return domain == FixtureFlagDomain::Event || domain == FixtureFlagDomain::Temporary ||
           domain == FixtureFlagDomain::Dungeon || domain == FixtureFlagDomain::Switch;
}

ScenarioFixtureError append_record(std::vector<std::uint8_t>& records, const std::uint16_t tag,
    const std::span<const std::uint8_t> payload)
{
    const auto padded = aligned4(payload.size());
    if (!padded || *padded > kMaximumEncodedSize - kScenarioFixtureHeaderSize ||
        records.size() > kMaximumEncodedSize - kScenarioFixtureHeaderSize -
                                 kRecordHeaderSize - *padded)
    {
        return ScenarioFixtureError::LimitExceeded;
    }
    const std::size_t start = records.size();
    records.resize(start + kRecordHeaderSize + *padded, 0);
    write_u16(records.data() + start, tag);
    write_u32(records.data() + start + 4, static_cast<std::uint32_t>(payload.size()));
    std::copy(payload.begin(), payload.end(), records.begin() + start + kRecordHeaderSize);
    return ScenarioFixtureError::None;
}

template <typename T, typename Key>
bool duplicate_sorted_key(const std::vector<T>& values, Key key) {
    return std::adjacent_find(values.begin(), values.end(), [&](const T& left, const T& right) {
               return key(left) == key(right);
           }) != values.end();
}

ScenarioFixtureError append_setting(
    std::vector<std::uint8_t>& records, const SettingFixture& setting)
{
    std::vector<std::uint8_t> value;
    std::uint8_t kind = 0;
    if (const auto* boolean = std::get_if<bool>(&setting.value)) {
        kind = 1;
        value.push_back(*boolean ? 1 : 0);
    } else if (const auto* integer = std::get_if<std::int64_t>(&setting.value)) {
        kind = 2;
        value.resize(8);
        write_u64(value.data(), std::bit_cast<std::uint64_t>(*integer));
    } else if (const auto* floating = std::get_if<FixtureFloat>(&setting.value)) {
        kind = 3;
        value.resize(8);
        write_u64(value.data(), floating->bits);
    } else {
        kind = 4;
        const auto& string = std::get<std::string>(setting.value);
        value.assign(string.begin(), string.end());
    }

    std::vector<std::uint8_t> payload(4 + setting.key.size() + value.size());
    payload[0] = static_cast<std::uint8_t>(setting.key.size());
    payload[1] = kind;
    write_u16(payload.data() + 2, static_cast<std::uint16_t>(value.size()));
    std::copy(setting.key.begin(), setting.key.end(), payload.begin() + 4);
    std::copy(value.begin(), value.end(), payload.begin() + 4 + setting.key.size());
    return append_record(records, kTagSetting, payload);
}

}  // namespace

const char* scenario_fixture_error_message(const ScenarioFixtureError error) {
    switch (error) {
    case ScenarioFixtureError::None: return "no error";
    case ScenarioFixtureError::Truncated: return "scenario fixture is truncated";
    case ScenarioFixtureError::BadMagic: return "scenario fixture magic is invalid";
    case ScenarioFixtureError::UnsupportedVersion: return "scenario fixture version is unsupported";
    case ScenarioFixtureError::InvalidHeader: return "scenario fixture header is invalid";
    case ScenarioFixtureError::InvalidSize: return "scenario fixture size is invalid";
    case ScenarioFixtureError::InvalidRecordCount: return "scenario fixture record count is invalid";
    case ScenarioFixtureError::InvalidRecord: return "scenario fixture record is invalid";
    case ScenarioFixtureError::UnknownRecord: return "scenario fixture record tag is unknown";
    case ScenarioFixtureError::NonCanonical: return "scenario fixture encoding is noncanonical";
    case ScenarioFixtureError::InvalidName: return "scenario fixture name is invalid";
    case ScenarioFixtureError::InvalidHealth: return "scenario fixture health is invalid";
    case ScenarioFixtureError::InvalidInventory: return "scenario fixture inventory is invalid";
    case ScenarioFixtureError::InvalidFlag: return "scenario fixture flag is invalid";
    case ScenarioFixtureError::InvalidSetting: return "scenario fixture setting is invalid";
    case ScenarioFixtureError::DuplicateKey: return "scenario fixture contains a duplicate keyed record";
    case ScenarioFixtureError::LimitExceeded: return "scenario fixture exceeds a bounded format limit";
    }
    return "unknown scenario fixture error";
}

ScenarioFixtureError validate_scenario_fixture(const ScenarioFixture& fixture) {
    if (!valid_name(fixture.name)) {
        return ScenarioFixtureError::InvalidName;
    }
    if (fixture.form && !valid_form(*fixture.form)) {
        return ScenarioFixtureError::InvalidRecord;
    }
    if (fixture.health && (fixture.health->current == 0 || fixture.health->maximum == 0 ||
                              fixture.health->current > fixture.health->maximum))
    {
        return ScenarioFixtureError::InvalidHealth;
    }
    if (fixture.videoMode && !valid_video_mode(*fixture.videoMode)) {
        return ScenarioFixtureError::InvalidRecord;
    }
    if (fixture.rng.size() > 2 || fixture.inventory.size() > kMaximumInventory ||
        fixture.equipment.size() > kMaximumEquipment || fixture.flags.size() > kMaximumFlags ||
        fixture.settings.size() > kMaximumSettings)
    {
        return ScenarioFixtureError::LimitExceeded;
    }

    auto rng = fixture.rng;
    std::ranges::sort(rng, {}, &RngFixture::stream);
    if (std::ranges::any_of(rng, [](const RngFixture& value) {
            return !valid_rng_stream(value.stream);
        }) || duplicate_sorted_key(rng, [](const RngFixture& value) { return value.stream; }))
    {
        return ScenarioFixtureError::DuplicateKey;
    }

    auto inventory = fixture.inventory;
    std::ranges::sort(inventory, {}, &InventoryFixture::slot);
    if (std::ranges::any_of(inventory, [](const InventoryFixture& value) {
            return value.quantity == 0;
        }))
    {
        return ScenarioFixtureError::InvalidInventory;
    }
    if (duplicate_sorted_key(inventory, [](const InventoryFixture& value) { return value.slot; })) {
        return ScenarioFixtureError::DuplicateKey;
    }

    auto equipment = fixture.equipment;
    std::ranges::sort(equipment, {}, &EquipmentFixture::slot);
    if (duplicate_sorted_key(equipment, [](const EquipmentFixture& value) { return value.slot; })) {
        return ScenarioFixtureError::DuplicateKey;
    }

    auto flags = fixture.flags;
    std::ranges::sort(flags, [](const FlagFixture& left, const FlagFixture& right) {
        return std::tie(left.domain, left.room, left.index, left.value) <
               std::tie(right.domain, right.room, right.index, right.value);
    });
    if (std::ranges::any_of(flags, [](const FlagFixture& flag) {
            return !valid_flag_domain(flag.domain) || flag.room < -1 || flag.room > 63 ||
                   (flag.domain == FixtureFlagDomain::Switch && flag.room < 0);
        }))
    {
        return ScenarioFixtureError::InvalidFlag;
    }
    if (duplicate_sorted_key(flags, [](const FlagFixture& value) {
            return std::tuple{value.domain, value.room, value.index};
        }))
    {
        return ScenarioFixtureError::DuplicateKey;
    }

    auto settings = fixture.settings;
    std::ranges::sort(settings, {}, &SettingFixture::key);
    if (duplicate_sorted_key(settings, [](const SettingFixture& value) { return value.key; })) {
        return ScenarioFixtureError::DuplicateKey;
    }
    for (const SettingFixture& setting : settings) {
        if (!valid_setting_key(setting.key)) {
            return ScenarioFixtureError::InvalidSetting;
        }
        if (const auto* floating = std::get_if<FixtureFloat>(&setting.value);
            floating && !std::isfinite(std::bit_cast<double>(floating->bits)))
        {
            return ScenarioFixtureError::InvalidSetting;
        }
        if (const auto* string = std::get_if<std::string>(&setting.value);
            string && string->size() > kMaximumSettingStringBytes)
        {
            return ScenarioFixtureError::InvalidSetting;
        }
    }
    return ScenarioFixtureError::None;
}

ScenarioFixtureError encode_scenario_fixture(
    const ScenarioFixture& fixture, std::vector<std::uint8_t>& output)
{
    const ScenarioFixtureError validation = validate_scenario_fixture(fixture);
    if (validation != ScenarioFixtureError::None) {
        return validation;
    }
    std::vector<std::uint8_t> records;
    std::uint16_t count = 0;
    const auto append = [&](const std::uint16_t tag, const std::span<const std::uint8_t> payload) {
        const ScenarioFixtureError error = append_record(records, tag, payload);
        if (error == ScenarioFixtureError::None) {
            ++count;
        }
        return error;
    };
    if (auto error = append(kTagName,
            std::span<const std::uint8_t>(reinterpret_cast<const std::uint8_t*>(fixture.name.data()),
                fixture.name.size()));
        error != ScenarioFixtureError::None)
    {
        return error;
    }
    if (fixture.form) {
        const std::array<std::uint8_t, 4> payload{static_cast<std::uint8_t>(*fixture.form), 0, 0, 0};
        if (auto error = append(kTagForm, payload); error != ScenarioFixtureError::None) return error;
    }
    if (fixture.health) {
        std::array<std::uint8_t, 4> payload{};
        write_u16(payload.data(), fixture.health->current);
        write_u16(payload.data() + 2, fixture.health->maximum);
        if (auto error = append(kTagHealth, payload); error != ScenarioFixtureError::None) return error;
    }
    auto rng = fixture.rng;
    std::ranges::sort(rng, {}, &RngFixture::stream);
    for (const RngFixture& value : rng) {
        std::array<std::uint8_t, 24> payload{};
        payload[0] = static_cast<std::uint8_t>(value.stream);
        write_i32(payload.data() + 4, value.state0);
        write_i32(payload.data() + 8, value.state1);
        write_i32(payload.data() + 12, value.state2);
        write_u64(payload.data() + 16, value.callCount);
        if (auto error = append(kTagRng, payload); error != ScenarioFixtureError::None) return error;
    }
    if (fixture.videoMode) {
        const std::array<std::uint8_t, 4> payload{
            static_cast<std::uint8_t>(*fixture.videoMode), 0, 0, 0};
        if (auto error = append(kTagVideoMode, payload); error != ScenarioFixtureError::None) return error;
    }
    auto inventory = fixture.inventory;
    std::ranges::sort(inventory, {}, &InventoryFixture::slot);
    for (const InventoryFixture& value : inventory) {
        std::array<std::uint8_t, 8> payload{};
        write_u16(payload.data(), value.slot);
        write_u16(payload.data() + 2, value.item);
        write_u16(payload.data() + 4, value.quantity);
        if (auto error = append(kTagInventory, payload); error != ScenarioFixtureError::None) return error;
    }
    auto equipment = fixture.equipment;
    std::ranges::sort(equipment, {}, &EquipmentFixture::slot);
    for (const EquipmentFixture& value : equipment) {
        std::array<std::uint8_t, 4> payload{};
        write_u16(payload.data(), value.slot);
        write_u16(payload.data() + 2, value.item);
        if (auto error = append(kTagEquipment, payload); error != ScenarioFixtureError::None) return error;
    }
    auto flags = fixture.flags;
    std::ranges::sort(flags, [](const FlagFixture& left, const FlagFixture& right) {
        return std::tie(left.domain, left.room, left.index, left.value) <
               std::tie(right.domain, right.room, right.index, right.value);
    });
    for (const FlagFixture& value : flags) {
        std::array<std::uint8_t, 8> payload{};
        payload[0] = static_cast<std::uint8_t>(value.domain);
        payload[1] = static_cast<std::uint8_t>(value.room);
        payload[2] = value.value ? 1 : 0;
        write_u16(payload.data() + 4, value.index);
        if (auto error = append(kTagFlag, payload); error != ScenarioFixtureError::None) return error;
    }
    auto settings = fixture.settings;
    std::ranges::sort(settings, {}, &SettingFixture::key);
    for (const SettingFixture& value : settings) {
        const ScenarioFixtureError error = append_setting(records, value);
        if (error != ScenarioFixtureError::None) return error;
        ++count;
    }

    if (records.size() > kMaximumEncodedSize - kScenarioFixtureHeaderSize) {
        return ScenarioFixtureError::LimitExceeded;
    }
    std::vector<std::uint8_t> encoded(kScenarioFixtureHeaderSize + records.size(), 0);
    std::copy(kMagic.begin(), kMagic.end(), encoded.begin());
    write_u16(encoded.data() + 8, kScenarioFixtureMajorVersion);
    write_u16(encoded.data() + 10, kScenarioFixtureMinorVersion);
    write_u16(encoded.data() + 12, static_cast<std::uint16_t>(kScenarioFixtureHeaderSize));
    write_u16(encoded.data() + 14, count);
    write_u32(encoded.data() + 16, static_cast<std::uint32_t>(encoded.size()));
    std::copy(records.begin(), records.end(), encoded.begin() + kScenarioFixtureHeaderSize);
    output = std::move(encoded);
    return ScenarioFixtureError::None;
}

ScenarioFixtureError decode_scenario_fixture(
    const std::span<const std::uint8_t> bytes, ScenarioFixture& output)
{
    if (bytes.size() < kScenarioFixtureHeaderSize) return ScenarioFixtureError::Truncated;
    if (!std::equal(kMagic.begin(), kMagic.end(), bytes.begin())) return ScenarioFixtureError::BadMagic;
    if (read_u16(bytes.data() + 8) != kScenarioFixtureMajorVersion ||
        read_u16(bytes.data() + 10) != kScenarioFixtureMinorVersion)
        return ScenarioFixtureError::UnsupportedVersion;
    if (read_u16(bytes.data() + 12) != kScenarioFixtureHeaderSize ||
        !all_zero(bytes.subspan(20, kScenarioFixtureHeaderSize - 20)))
        return ScenarioFixtureError::InvalidHeader;
    if (read_u32(bytes.data() + 16) != bytes.size() || bytes.size() > kMaximumEncodedSize)
        return ScenarioFixtureError::InvalidSize;

    ScenarioFixture decoded;
    bool hasName = false;
    bool hasForm = false;
    bool hasHealth = false;
    bool hasVideo = false;
    std::size_t cursor = kScenarioFixtureHeaderSize;
    std::size_t count = 0;
    while (cursor < bytes.size()) {
        if (bytes.size() - cursor < kRecordHeaderSize) return ScenarioFixtureError::Truncated;
        const std::uint16_t tag = read_u16(bytes.data() + cursor);
        if (read_u16(bytes.data() + cursor + 2) != 0) return ScenarioFixtureError::InvalidRecord;
        const std::size_t length = read_u32(bytes.data() + cursor + 4);
        cursor += kRecordHeaderSize;
        const auto padded = aligned4(length);
        if (!padded || *padded > bytes.size() - cursor) return ScenarioFixtureError::Truncated;
        const auto payload = bytes.subspan(cursor, length);
        if (!all_zero(bytes.subspan(cursor + length, *padded - length)))
            return ScenarioFixtureError::NonCanonical;

        switch (tag) {
        case kTagName:
            if (hasName) return ScenarioFixtureError::DuplicateKey;
            decoded.name.assign(reinterpret_cast<const char*>(payload.data()), payload.size());
            hasName = true;
            break;
        case kTagForm:
            if (hasForm) return ScenarioFixtureError::DuplicateKey;
            if (payload.size() != 4 || !all_zero(payload.subspan(1)))
                return ScenarioFixtureError::InvalidRecord;
            decoded.form = static_cast<PlayerFixtureForm>(payload[0]);
            hasForm = true;
            break;
        case kTagHealth:
            if (hasHealth) return ScenarioFixtureError::DuplicateKey;
            if (payload.size() != 4) return ScenarioFixtureError::InvalidRecord;
            decoded.health = HealthFixture{read_u16(payload.data()), read_u16(payload.data() + 2)};
            hasHealth = true;
            break;
        case kTagRng:
            if (payload.size() != 24 || !all_zero(payload.subspan(1, 3)))
                return ScenarioFixtureError::InvalidRecord;
            decoded.rng.push_back({static_cast<FixtureRngStream>(payload[0]),
                read_i32(payload.data() + 4), read_i32(payload.data() + 8),
                read_i32(payload.data() + 12), read_u64(payload.data() + 16)});
            break;
        case kTagVideoMode:
            if (hasVideo) return ScenarioFixtureError::DuplicateKey;
            if (payload.size() != 4 || !all_zero(payload.subspan(1)))
                return ScenarioFixtureError::InvalidRecord;
            decoded.videoMode = static_cast<FixtureVideoMode>(payload[0]);
            hasVideo = true;
            break;
        case kTagInventory:
            if (payload.size() != 8 || !all_zero(payload.subspan(6)))
                return ScenarioFixtureError::InvalidRecord;
            decoded.inventory.push_back(
                {read_u16(payload.data()), read_u16(payload.data() + 2), read_u16(payload.data() + 4)});
            break;
        case kTagEquipment:
            if (payload.size() != 4) return ScenarioFixtureError::InvalidRecord;
            decoded.equipment.push_back({read_u16(payload.data()), read_u16(payload.data() + 2)});
            break;
        case kTagFlag:
            if (payload.size() != 8 || payload[2] > 1 || payload[3] != 0 ||
                payload[6] != 0 || payload[7] != 0)
                return ScenarioFixtureError::InvalidRecord;
            decoded.flags.push_back({static_cast<FixtureFlagDomain>(payload[0]),
                static_cast<std::int8_t>(payload[1]), read_u16(payload.data() + 4), payload[2] != 0});
            break;
        case kTagSetting: {
            if (payload.size() < 4) return ScenarioFixtureError::InvalidRecord;
            const std::size_t keySize = payload[0];
            const std::uint8_t kind = payload[1];
            const std::size_t valueSize = read_u16(payload.data() + 2);
            if (4 + keySize + valueSize != payload.size()) return ScenarioFixtureError::InvalidRecord;
            SettingFixture setting;
            setting.key.assign(reinterpret_cast<const char*>(payload.data() + 4), keySize);
            const auto value = payload.subspan(4 + keySize, valueSize);
            if (kind == 1 && value.size() == 1 && value[0] <= 1) {
                setting.value = value[0] != 0;
            } else if (kind == 2 && value.size() == 8) {
                setting.value = std::bit_cast<std::int64_t>(read_u64(value.data()));
            } else if (kind == 3 && value.size() == 8) {
                setting.value = FixtureFloat{read_u64(value.data())};
            } else if (kind == 4) {
                setting.value = std::string(reinterpret_cast<const char*>(value.data()), value.size());
            } else {
                return ScenarioFixtureError::InvalidSetting;
            }
            decoded.settings.push_back(std::move(setting));
            break;
        }
        default: return ScenarioFixtureError::UnknownRecord;
        }
        cursor += *padded;
        ++count;
    }
    if (count != read_u16(bytes.data() + 14)) return ScenarioFixtureError::InvalidRecordCount;
    const ScenarioFixtureError validation = validate_scenario_fixture(decoded);
    if (validation != ScenarioFixtureError::None) return validation;
    std::vector<std::uint8_t> canonical;
    const ScenarioFixtureError encodeError = encode_scenario_fixture(decoded, canonical);
    if (encodeError != ScenarioFixtureError::None) return encodeError;
    if (!std::equal(canonical.begin(), canonical.end(), bytes.begin(), bytes.end()))
        return ScenarioFixtureError::NonCanonical;
    output = std::move(decoded);
    return ScenarioFixtureError::None;
}

}  // namespace dusk::automation
