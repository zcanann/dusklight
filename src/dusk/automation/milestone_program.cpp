#include "dusk/automation/milestone_program.hpp"

#include "dusk/automation/milestones.hpp"
#include "dusk/automation/typed_facts.hpp"

#include <algorithm>
#include <array>
#include <bit>
#include <cmath>
#include <cstring>
#include <limits>
#include <string_view>

namespace dusk::automation {
namespace {

constexpr char DefinitionDomainBytes[] = "dusklight.milestone.definition/v1";
constexpr char ProgramDomainBytes[] = "dusklight.milestone.program/v1";
constexpr char ProjectionDomainBytes[] = "dusklight.value-projection.identity/v1";
constexpr std::string_view DefinitionDomain{DefinitionDomainBytes, sizeof(DefinitionDomainBytes)};
constexpr std::string_view ProgramDomain{ProgramDomainBytes, sizeof(ProgramDomainBytes)};
constexpr std::string_view ProjectionDomain{ProjectionDomainBytes, sizeof(ProjectionDomainBytes)};
constexpr std::size_t HeaderSize = 52;

enum class ValueType : std::uint8_t {
    Bool,
    U32,
    U64,
    I32,
    F32,
    Symbol,
    Procedure,
    BoundaryKind,
};

struct Value {
    ValueType type = ValueType::Bool;
    bool available = true;
    std::uint64_t bits = 0;
    std::array<char, kMilestoneProgramMaximumSymbolBytes> symbol{};
    std::uint8_t symbolLength = 0;
};

struct TypeEntry {
    ValueType type;
    std::optional<std::size_t> instruction;
};

constexpr std::array<std::uint32_t, 64> ShaConstants{
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
    0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
};

class Sha256 {
public:
    void update(const std::span<const std::uint8_t> bytes) {
        for (const std::uint8_t byte : bytes) {
            mBlock[mBlockSize++] = byte;
            ++mByteCount;
            if (mBlockSize == mBlock.size()) {
                transform();
                mBlockSize = 0;
            }
        }
    }

    void update(const std::string_view text) {
        update({reinterpret_cast<const std::uint8_t*>(text.data()), text.size()});
    }

    std::array<std::uint8_t, 32> finish() {
        const std::uint64_t bitCount = mByteCount * 8;
        mBlock[mBlockSize++] = 0x80;
        if (mBlockSize > 56) {
            std::fill(mBlock.begin() + mBlockSize, mBlock.end(), 0);
            transform();
            mBlockSize = 0;
        }
        std::fill(mBlock.begin() + mBlockSize, mBlock.begin() + 56, 0);
        for (std::size_t index = 0; index < 8; ++index) {
            mBlock[63 - index] = static_cast<std::uint8_t>(bitCount >> (index * 8));
        }
        transform();

        std::array<std::uint8_t, 32> digest{};
        for (std::size_t word = 0; word < mState.size(); ++word) {
            for (std::size_t byte = 0; byte < 4; ++byte) {
                digest[word * 4 + byte] =
                    static_cast<std::uint8_t>(mState[word] >> ((3 - byte) * 8));
            }
        }
        return digest;
    }

private:
    void transform() {
        std::array<std::uint32_t, 64> words{};
        for (std::size_t index = 0; index < 16; ++index) {
            words[index] = (static_cast<std::uint32_t>(mBlock[index * 4]) << 24) |
                           (static_cast<std::uint32_t>(mBlock[index * 4 + 1]) << 16) |
                           (static_cast<std::uint32_t>(mBlock[index * 4 + 2]) << 8) |
                           static_cast<std::uint32_t>(mBlock[index * 4 + 3]);
        }
        for (std::size_t index = 16; index < words.size(); ++index) {
            const std::uint32_t a = words[index - 15];
            const std::uint32_t b = words[index - 2];
            const std::uint32_t s0 = std::rotr(a, 7) ^ std::rotr(a, 18) ^ (a >> 3);
            const std::uint32_t s1 = std::rotr(b, 17) ^ std::rotr(b, 19) ^ (b >> 10);
            words[index] = words[index - 16] + s0 + words[index - 7] + s1;
        }

        auto [a, b, c, d, e, f, g, h] = mState;
        for (std::size_t index = 0; index < words.size(); ++index) {
            const std::uint32_t s1 = std::rotr(e, 6) ^ std::rotr(e, 11) ^ std::rotr(e, 25);
            const std::uint32_t choice = (e & f) ^ (~e & g);
            const std::uint32_t temp1 = h + s1 + choice + ShaConstants[index] + words[index];
            const std::uint32_t s0 = std::rotr(a, 2) ^ std::rotr(a, 13) ^ std::rotr(a, 22);
            const std::uint32_t majority = (a & b) ^ (a & c) ^ (b & c);
            const std::uint32_t temp2 = s0 + majority;
            h = g;
            g = f;
            f = e;
            e = d + temp1;
            d = c;
            c = b;
            b = a;
            a = temp1 + temp2;
        }
        mState[0] += a;
        mState[1] += b;
        mState[2] += c;
        mState[3] += d;
        mState[4] += e;
        mState[5] += f;
        mState[6] += g;
        mState[7] += h;
    }

    std::array<std::uint32_t, 8> mState{
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    };
    std::array<std::uint8_t, 64> mBlock{};
    std::size_t mBlockSize = 0;
    std::uint64_t mByteCount = 0;
};

std::string hex_digest(const std::array<std::uint8_t, 32>& digest) {
    constexpr char Hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(64);
    for (const std::uint8_t byte : digest) {
        output.push_back(Hex[byte >> 4]);
        output.push_back(Hex[byte & 0x0f]);
    }
    return output;
}

bool digest_matches(const std::array<std::uint8_t, 32>& actual,
                    const std::span<const std::uint8_t, 32> expected) {
    std::uint8_t difference = 0;
    for (std::size_t index = 0; index < actual.size(); ++index) {
        difference |= actual[index] ^ expected[index];
    }
    return difference == 0;
}

std::uint16_t read_u16(const std::uint8_t* input) {
    return static_cast<std::uint16_t>(input[0]) |
           (static_cast<std::uint16_t>(input[1]) << 8);
}

std::uint32_t read_u32(const std::uint8_t* input) {
    return static_cast<std::uint32_t>(input[0]) |
           (static_cast<std::uint32_t>(input[1]) << 8) |
           (static_cast<std::uint32_t>(input[2]) << 16) |
           (static_cast<std::uint32_t>(input[3]) << 24);
}

std::uint64_t read_u64(const std::uint8_t* input) {
    return static_cast<std::uint64_t>(read_u32(input)) |
           (static_cast<std::uint64_t>(read_u32(input + 4)) << 32);
}

bool valid_utf8(const std::string_view value) {
    const auto* bytes = reinterpret_cast<const unsigned char*>(value.data());
    std::size_t index = 0;
    while (index < value.size()) {
        const unsigned char lead = bytes[index++];
        if (lead < 0x80) {
            if (lead == 0) {
                return false;
            }
            continue;
        }
        std::size_t continuation = 0;
        std::uint32_t codepoint = 0;
        if ((lead & 0xe0) == 0xc0) {
            continuation = 1;
            codepoint = lead & 0x1f;
        } else if ((lead & 0xf0) == 0xe0) {
            continuation = 2;
            codepoint = lead & 0x0f;
        } else if ((lead & 0xf8) == 0xf0) {
            continuation = 3;
            codepoint = lead & 0x07;
        } else {
            return false;
        }
        if (index + continuation > value.size()) {
            return false;
        }
        for (std::size_t count = 0; count < continuation; ++count) {
            const unsigned char next = bytes[index++];
            if ((next & 0xc0) != 0x80) {
                return false;
            }
            codepoint = (codepoint << 6) | (next & 0x3f);
        }
        const std::uint32_t minimum = continuation == 1 ? 0x80 : continuation == 2 ? 0x800 : 0x10000;
        if (codepoint < minimum || codepoint > 0x10ffff ||
            (codepoint >= 0xd800 && codepoint <= 0xdfff)) {
            return false;
        }
    }
    return true;
}

bool valid_stage_symbol(const std::string_view symbol) {
    if (symbol.empty() || symbol.size() > 8) {
        return false;
    }
    return std::ranges::all_of(symbol, [](const char character) {
        return (character >= 'A' && character <= 'Z') ||
               (character >= '0' && character <= '9') || character == '_';
    });
}

std::optional<ValueType> field_type(const std::uint8_t field) {
    switch (field) {
    case 1: return ValueType::BoundaryKind;
    case 2: case 3: return ValueType::U64;
    case 4: case 16: return ValueType::Symbol;
    case 5: case 6: case 7: case 15: case 17: case 18: case 19: return ValueType::I32;
    case 8: case 14: case 20: case 21: case 22: return ValueType::Bool;
    case 9: case 10: case 11: case 12: return ValueType::F32;
    case 13: return ValueType::Procedure;
    case 23: case 34: case 37: case 38: case 39: case 40: case 42: case 59: case 62:
    case 67: return ValueType::U32;
    case 24: case 28: case 29: case 30: case 31: case 32: case 33: case 35: case 36:
    case 43: case 44: case 45: case 47: case 48: case 49: case 61: case 63: case 64:
    case 66: case 68: case 69: return ValueType::I32;
    case 25: case 26: case 27: case 56: case 57: case 58: case 70: case 71: case 72:
    case 73: case 74: case 75: return ValueType::F32;
    case 41: case 51: case 52: case 53: case 54: case 55: case 60: case 65:
        return ValueType::Bool;
    case 46: case 50: return ValueType::U64;
    default: return std::nullopt;
    }
}

std::optional<ValueType> actor_fact_type(const std::uint8_t field) {
    switch (field) {
    case 1: return ValueType::Bool;
    case 2: case 3: case 4: case 5: return ValueType::F32;
    case 6: case 7: return ValueType::I32;
    case 8: return ValueType::U32;
    default: return std::nullopt;
    }
}

bool fixed_stage_valid(const std::array<char, 8>& stage) {
    const auto end = std::find(stage.begin(), stage.end(), '\0');
    if (end == stage.begin()) return false;
    if (end != stage.end() && std::any_of(end, stage.end(), [](char value) { return value != 0; }))
        return false;
    return valid_stage_symbol(std::string_view(stage.data(), end - stage.begin()));
}

bool fixed_stage_matches(const std::array<char, 8>& expected, const char* actual) {
    if (actual == nullptr) return false;
    std::array<char, 8> canonical{};
    std::size_t length = 0;
    while (length < 9 && actual[length] != '\0') ++length;
    if (length == 0 || length > canonical.size()) return false;
    std::copy_n(actual, length, canonical.begin());
    return expected == canonical;
}

bool normalize_symbol_comparison(MilestoneProgramDefinition::Instruction& instruction,
    ValueType& symbolType, const ValueType otherType) {
    const std::string_view symbol(instruction.symbol.data(), instruction.symbolLength);
    if (otherType == ValueType::BoundaryKind) {
        if (symbol == "boot") {
            instruction.bits = static_cast<std::uint32_t>(MilestoneBoundaryKind::Boot);
        } else if (symbol == "tick") {
            instruction.bits = static_cast<std::uint32_t>(MilestoneBoundaryKind::Tick);
        } else {
            return false;
        }
        instruction.symbolLength = 0;
        instruction.field = 1;
        symbolType = ValueType::BoundaryKind;
        return true;
    }
    return otherType == ValueType::Symbol && valid_stage_symbol(symbol);
}

Value load_field(const std::uint8_t field, const MilestoneProgramContext& context) {
    const MilestoneObservation& observation = context.observation;
    Value value{.type = *field_type(field)};
    const auto set_symbol = [&value](const char* source, const bool enabled = true) {
        if (!enabled || source == nullptr) {
            value.available = false;
            return;
        }
        std::size_t length = 0;
        while (length < 9 && source[length] != '\0') {
            ++length;
        }
        if (length == 0 || length > 8) {
            value.available = false;
            return;
        }
        std::copy_n(source, length, value.symbol.begin());
        value.symbolLength = static_cast<std::uint8_t>(length);
    };
    const auto present_fact = [&context](const TypedFactId id,
                                  const TypedFactValueType type) -> const TypedFactEntry* {
        if (context.facts == nullptr) return nullptr;
        const TypedFactEntry* entry = context.facts->find(id);
        return entry != nullptr && entry->status == TypedFactStatus::Present &&
                       entry->type == type ?
                   entry :
                   nullptr;
    };
    const auto actor_exists = [&context, &value](const TypedFactId id) {
        if (context.facts == nullptr) return false;
        const TypedFactEntry* entry = context.facts->find(id);
        if (entry == nullptr || entry->type != TypedFactValueType::ActorIdentity ||
            (entry->status != TypedFactStatus::Present &&
                entry->status != TypedFactStatus::Absent)) {
            value.available = false;
        } else {
            value.bits = entry->status == TypedFactStatus::Present ? 1 : 0;
        }
        return true;
    };
    switch (field) {
    case 1: value.bits = static_cast<std::uint32_t>(context.boundaryKind); break;
    case 2: value.bits = context.boundaryIndex; break;
    case 3:
        value.available = context.tapeFrame.has_value();
        value.bits = context.tapeFrame.value_or(kMilestoneProgramUnavailableTapeFrame);
        break;
    case 4:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::StageName, TypedFactValueType::StageCode);
            if (entry == nullptr) {
                value.available = false;
            } else {
                std::copy(entry->value.stageCode.begin(), entry->value.stageCode.end(),
                    value.symbol.begin());
                while (value.symbolLength < entry->value.stageCode.size() &&
                       value.symbol[value.symbolLength] != '\0') {
                    ++value.symbolLength;
                }
                value.available = value.symbolLength != 0;
            }
        } else {
            set_symbol(observation.stageName);
        }
        break;
    case 5:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::StageRoom, TypedFactValueType::I32);
            value.available = entry != nullptr;
            if (entry != nullptr) value.bits = static_cast<std::uint32_t>(entry->value.i32);
        } else {
            value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.room));
        }
        break;
    case 6: value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.layer)); break;
    case 7:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::StageSpawn, TypedFactValueType::I32);
            value.available = entry != nullptr;
            if (entry != nullptr) value.bits = static_cast<std::uint32_t>(entry->value.i32);
        } else {
            value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.point));
        }
        break;
    case 8:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::PlayerExists, TypedFactValueType::Boolean);
            value.available = entry != nullptr;
            if (entry != nullptr) value.bits = entry->value.boolean ? 1 : 0;
        } else {
            value.bits = observation.playerPresent ? 1 : 0;
        }
        break;
    case 9:
    case 10:
    case 11:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::PlayerPosition, TypedFactValueType::Vec3F32);
            value.available = entry != nullptr;
            if (entry != nullptr) {
                value.bits = std::bit_cast<std::uint32_t>(entry->value.vec3[field - 9]);
            }
        } else {
            value.available = observation.playerPresent;
            const float positions[] = {observation.playerPositionX, observation.playerPositionY,
                observation.playerPositionZ};
            value.bits = std::bit_cast<std::uint32_t>(positions[field - 9]);
        }
        break;
    case 12: value.available = observation.playerPresent; value.bits = std::bit_cast<std::uint32_t>(observation.playerForwardSpeed); break;
    case 13: value.available = observation.playerPresent && observation.playerIsLink; value.bits = observation.playerProcId; break;
    case 14:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::EventRunning, TypedFactValueType::Boolean);
            value.available = entry != nullptr;
            if (entry != nullptr) value.bits = entry->value.boolean ? 1 : 0;
        } else {
            value.bits = observation.eventRunning ? 1 : 0;
        }
        break;
    case 15:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::EventId, TypedFactValueType::I32);
            value.available = entry != nullptr;
            if (entry != nullptr) value.bits = static_cast<std::uint32_t>(entry->value.i32);
        } else {
            value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.eventId));
        }
        break;
    case 16: set_symbol(observation.nextStageName, observation.nextStageEnabled); break;
    case 17: value.available = observation.nextStageEnabled; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.nextRoom)); break;
    case 18: value.available = observation.nextStageEnabled; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.nextLayer)); break;
    case 19: value.available = observation.nextStageEnabled; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.nextPoint)); break;
    case 20: value.bits = 1; break;
    case 21:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::PlayerIsLink, TypedFactValueType::Boolean);
            value.available = entry != nullptr;
            if (entry != nullptr) value.bits = entry->value.boolean ? 1 : 0;
        } else {
            value.bits = observation.playerIsLink ? 1 : 0;
        }
        break;
    case 22: value.bits = observation.nextStageEnabled ? 1 : 0; break;
    case 23: value.available = observation.playerPresent; value.bits = observation.playerProcessId; break;
    case 24: value.available = observation.playerPresent; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.playerActorName)); break;
    case 25: value.available = observation.playerPresent; value.bits = std::bit_cast<std::uint32_t>(observation.playerVelocityX); break;
    case 26: value.available = observation.playerPresent; value.bits = std::bit_cast<std::uint32_t>(observation.playerVelocityY); break;
    case 27: value.available = observation.playerPresent; value.bits = std::bit_cast<std::uint32_t>(observation.playerVelocityZ); break;
    case 28: value.available = observation.playerPresent; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.playerCurrentAngleX)); break;
    case 29: value.available = observation.playerPresent; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.playerCurrentAngleY)); break;
    case 30: value.available = observation.playerPresent; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.playerCurrentAngleZ)); break;
    case 31: value.available = observation.playerPresent; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.playerShapeAngleX)); break;
    case 32: value.available = observation.playerPresent; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.playerShapeAngleY)); break;
    case 33: value.available = observation.playerPresent; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.playerShapeAngleZ)); break;
    case 34: value.available = observation.playerPresent && observation.playerIsLink; value.bits = observation.playerModeFlags; break;
    case 35: value.available = observation.playerPresent && observation.playerIsLink; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.playerDamageWaitTimer)); break;
    case 36: value.available = observation.playerPresent && observation.playerIsLink; value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(observation.playerIceDamageWaitTimer)); break;
    case 37: value.available = observation.playerPresent && observation.playerIsLink; value.bits = observation.playerSwordChangeWaitTimer; break;
    case 38: value.bits = observation.eventMode; break;
    case 39: value.bits = observation.eventStatus; break;
    case 40: value.bits = observation.eventMapToolId; break;
    case 41: value.bits = observation.eventNameHashPresent ? 1 : 0; break;
    case 42: value.available = observation.eventNameHashPresent; value.bits = observation.eventNameHash; break;
    case 43: value.bits = static_cast<std::uint32_t>(observation.rng.streams[0].state0); break;
    case 44: value.bits = static_cast<std::uint32_t>(observation.rng.streams[0].state1); break;
    case 45: value.bits = static_cast<std::uint32_t>(observation.rng.streams[0].state2); break;
    case 46: value.bits = observation.rng.streams[0].callCount; break;
    case 47: value.bits = static_cast<std::uint32_t>(observation.rng.streams[1].state0); break;
    case 48: value.bits = static_cast<std::uint32_t>(observation.rng.streams[1].state1); break;
    case 49: value.bits = static_cast<std::uint32_t>(observation.rng.streams[1].state2); break;
    case 50: value.bits = observation.rng.streams[1].callCount; break;
    case 51: value.available = observation.playerPresent && observation.playerIsLink; value.bits = observation.playerGroundContact ? 1 : 0; break;
    case 52: value.available = observation.playerPresent && observation.playerIsLink; value.bits = observation.playerWallContact ? 1 : 0; break;
    case 53: value.available = observation.playerPresent && observation.playerIsLink; value.bits = observation.playerRoofContact ? 1 : 0; break;
    case 54: value.available = observation.playerPresent && observation.playerIsLink; value.bits = observation.playerWaterContact ? 1 : 0; break;
    case 55: value.available = observation.playerPresent && observation.playerIsLink; value.bits = observation.playerWaterIn ? 1 : 0; break;
    case 56: value.available = observation.playerGroundHeightPresent; value.bits = std::bit_cast<std::uint32_t>(observation.playerGroundHeight); break;
    case 57: value.available = observation.playerRoofHeightPresent; value.bits = std::bit_cast<std::uint32_t>(observation.playerRoofHeight); break;
    case 58: value.available = observation.playerPresent && observation.playerGroundHeightPresent; value.bits = std::bit_cast<std::uint32_t>(observation.playerPositionY - observation.playerGroundHeight); break;
    case 59:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::PlayerDoStatus, TypedFactValueType::U32);
            value.available = entry != nullptr;
            if (entry != nullptr) value.bits = entry->value.u32;
        } else {
            value.available = observation.playerPresent && observation.playerIsLink;
            value.bits = observation.playerDoStatus;
        }
        break;
    case 60:
        if (!actor_exists(TypedFactId::TalkPartner)) {
            value.available = observation.playerPresent && observation.playerIsLink;
            value.bits = observation.talkPartner.present ? 1 : 0;
        }
        break;
    case 61:
    case 62:
    case 63:
    case 64:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::TalkPartner, TypedFactValueType::ActorIdentity);
            value.available = entry != nullptr;
            if (entry != nullptr) {
                const auto& actor = entry->value.actor;
                const std::int32_t values[] = {actor.actorName, actor.setId, actor.homeRoom,
                    actor.currentRoom};
                value.bits = static_cast<std::uint32_t>(values[field - 61]);
            }
        } else {
            value.available = observation.talkPartner.present;
            const std::int32_t values[] = {observation.talkPartner.actorName,
                observation.talkPartner.setId, observation.talkPartner.homeRoom,
                observation.talkPartner.currentRoom};
            value.bits = static_cast<std::uint32_t>(values[field - 61]);
        }
        break;
    case 65:
        if (!actor_exists(TypedFactId::GrabbedActor)) {
            value.available = observation.playerPresent && observation.playerIsLink;
            value.bits = observation.grabbedActor.present ? 1 : 0;
        }
        break;
    case 66:
    case 67:
    case 68:
    case 69:
        if (context.facts != nullptr) {
            const auto* entry = present_fact(TypedFactId::GrabbedActor, TypedFactValueType::ActorIdentity);
            value.available = entry != nullptr;
            if (entry != nullptr) {
                const auto& actor = entry->value.actor;
                const std::int32_t values[] = {actor.actorName, actor.setId, actor.homeRoom,
                    actor.currentRoom};
                value.bits = static_cast<std::uint32_t>(values[field - 66]);
            }
        } else {
            value.available = observation.grabbedActor.present;
            const std::int32_t values[] = {observation.grabbedActor.actorName,
                observation.grabbedActor.setId, observation.grabbedActor.homeRoom,
                observation.grabbedActor.currentRoom};
            value.bits = static_cast<std::uint32_t>(values[field - 66]);
        }
        break;
    case 70:
    case 71:
    case 72:
        if (context.facts != nullptr) {
            const auto* entry =
                present_fact(TypedFactId::TalkPartner, TypedFactValueType::ActorIdentity);
            value.available = entry != nullptr && entry->value.actor.homePositionPresent;
            if (value.available) {
                value.bits = std::bit_cast<std::uint32_t>(
                    entry->value.actor.homePosition[field - 70]);
            }
        } else {
            value.available = observation.talkPartner.present &&
                              observation.talkPartner.homePositionPresent;
            const float values[] = {observation.talkPartner.homePositionX,
                observation.talkPartner.homePositionY, observation.talkPartner.homePositionZ};
            value.bits = std::bit_cast<std::uint32_t>(values[field - 70]);
        }
        break;
    case 73:
    case 74:
    case 75:
        if (context.facts != nullptr) {
            const auto* entry =
                present_fact(TypedFactId::GrabbedActor, TypedFactValueType::ActorIdentity);
            value.available = entry != nullptr && entry->value.actor.homePositionPresent;
            if (value.available) {
                value.bits = std::bit_cast<std::uint32_t>(
                    entry->value.actor.homePosition[field - 73]);
            }
        } else {
            value.available = observation.grabbedActor.present &&
                              observation.grabbedActor.homePositionPresent;
            const float values[] = {observation.grabbedActor.homePositionX,
                observation.grabbedActor.homePositionY, observation.grabbedActor.homePositionZ};
            value.bits = std::bit_cast<std::uint32_t>(values[field - 73]);
        }
        break;
    default: value.available = false; break;
    }
    return value;
}

Value load_query(const MilestoneProgramDefinition::Instruction& instruction,
    const MilestoneProgramContext& context) {
    const MilestoneObservation& observation = context.observation;
    if (instruction.queryKind == 1) {
        Value value{.type = *actor_fact_type(instruction.field)};
        if (!fixed_stage_matches(instruction.queryStage, observation.stageName)) {
            value.available = instruction.field == 1;
            value.bits = 0;
            return value;
        }
        const MilestoneObservation::Actor* selected = nullptr;
        std::size_t matches = 0;
        for (const auto& actor : observation.actors) {
            if (actor.actorName == instruction.queryActorName &&
                actor.setId == instruction.querySetId && actor.homeRoom == instruction.queryRoom) {
                selected = &actor;
                ++matches;
            }
        }
        if (observation.actorsTruncated || matches > 1) {
            value.available = false;
            return value;
        }
        if (instruction.field == 1) {
            value.bits = matches == 1 ? 1 : 0;
            return value;
        }
        if (selected == nullptr) {
            value.available = false;
            return value;
        }
        switch (instruction.field) {
        case 2: value.bits = std::bit_cast<std::uint32_t>(selected->positionX); break;
        case 3: value.bits = std::bit_cast<std::uint32_t>(selected->positionY); break;
        case 4: value.bits = std::bit_cast<std::uint32_t>(selected->positionZ); break;
        case 5: {
            if (!observation.playerPresent) {
                value.available = false;
                break;
            }
            const double dx = static_cast<double>(selected->positionX) - observation.playerPositionX;
            const double dy = static_cast<double>(selected->positionY) - observation.playerPositionY;
            const double dz = static_cast<double>(selected->positionZ) - observation.playerPositionZ;
            const float distance = static_cast<float>(std::sqrt(dx * dx + dy * dy + dz * dz));
            value.available = std::isfinite(distance);
            value.bits = std::bit_cast<std::uint32_t>(distance);
            break;
        }
        case 6: value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(selected->currentRoom)); break;
        case 7: value.bits = static_cast<std::uint32_t>(static_cast<std::int32_t>(selected->health)); break;
        case 8: value.bits = selected->status; break;
        default: value.available = false; break;
        }
        if (instruction.field >= 2 && instruction.field <= 4) {
            value.available = std::isfinite(std::bit_cast<float>(static_cast<std::uint32_t>(value.bits)));
        }
        return value;
    }

    if (instruction.queryKind == 3) {
        Value value{.type = ValueType::Bool, .available = observation.playerPresent};
        if (!value.available || !std::isfinite(observation.playerPositionX) ||
            !std::isfinite(observation.playerPositionY) ||
            !std::isfinite(observation.playerPositionZ)) {
            value.available = false;
            return value;
        }
        const std::array<float, 3> position{observation.playerPositionX,
            observation.playerPositionY, observation.playerPositionZ};
        value.bits = 1;
        for (std::size_t axis = 0; axis < position.size(); ++axis) {
            if (position[axis] < instruction.queryValues[axis] ||
                position[axis] > instruction.queryValues[axis + 3]) {
                value.bits = 0;
                break;
            }
        }
        return value;
    }
    if (instruction.queryKind == 4) {
        Value value{.type = ValueType::F32, .available = observation.playerPresent};
        if (!value.available || !std::isfinite(observation.playerPositionX) ||
            !std::isfinite(observation.playerPositionY) ||
            !std::isfinite(observation.playerPositionZ)) {
            value.available = false;
            return value;
        }
        const double dx = static_cast<double>(observation.playerPositionX) -
                          instruction.queryValues[0];
        const double dy = static_cast<double>(observation.playerPositionY) -
                          instruction.queryValues[1];
        const double dz = static_cast<double>(observation.playerPositionZ) -
                          instruction.queryValues[2];
        const double nx = instruction.queryValues[3];
        const double ny = instruction.queryValues[4];
        const double nz = instruction.queryValues[5];
        const float distance = static_cast<float>(
            (dx * nx + dy * ny + dz * nz) / std::sqrt(nx * nx + ny * ny + nz * nz));
        value.available = std::isfinite(distance);
        value.bits = std::bit_cast<std::uint32_t>(distance == 0.0F ? 0.0F : distance);
        return value;
    }

    Value value{.type = ValueType::Bool, .available = observation.flagsPresent};
    if (!value.available) return value;
    std::span<const std::uint8_t> flags;
    switch (instruction.field) {
    case 0: flags = observation.eventFlags; break;
    case 1: flags = observation.temporaryFlags; break;
    case 2: flags = observation.dungeonFlags; break;
    case 3:
        if (instruction.queryRoom != observation.switchFlagRoom) {
            value.available = false;
            return value;
        }
        flags = observation.switchFlags;
        break;
    default:
        value.available = false;
        return value;
    }
    if (instruction.queryIndex >= flags.size()) {
        value.available = false;
        return value;
    }
    value.bits = flags[instruction.queryIndex] != 0 ? 1 : 0;
    return value;
}

bool compare_values(const Value& lhs, const Value& rhs, const std::uint8_t opcode) {
    if (!lhs.available || !rhs.available) {
        return false;
    }
    if (lhs.type == ValueType::Symbol) {
        const int ordering = std::string_view(lhs.symbol.data(), lhs.symbolLength).compare(
            std::string_view(rhs.symbol.data(), rhs.symbolLength));
        return opcode == 0x20 ? ordering == 0 : opcode == 0x21 ? ordering != 0 : false;
    }
    if (lhs.type == ValueType::F32) {
        const float a = std::bit_cast<float>(static_cast<std::uint32_t>(lhs.bits));
        const float b = std::bit_cast<float>(static_cast<std::uint32_t>(rhs.bits));
        switch (opcode) {
        case 0x20: return a == b;
        case 0x21: return a != b;
        case 0x22: return a < b;
        case 0x23: return a <= b;
        case 0x24: return a > b;
        case 0x25: return a >= b;
        }
    }
    if (lhs.type == ValueType::I32) {
        const auto a = static_cast<std::int32_t>(lhs.bits);
        const auto b = static_cast<std::int32_t>(rhs.bits);
        switch (opcode) {
        case 0x20: return a == b;
        case 0x21: return a != b;
        case 0x22: return a < b;
        case 0x23: return a <= b;
        case 0x24: return a > b;
        case 0x25: return a >= b;
        }
    }
    switch (opcode) {
    case 0x20: return lhs.bits == rhs.bits;
    case 0x21: return lhs.bits != rhs.bits;
    case 0x22: return lhs.bits < rhs.bits;
    case 0x23: return lhs.bits <= rhs.bits;
    case 0x24: return lhs.bits > rhs.bits;
    case 0x25: return lhs.bits >= rhs.bits;
    case 0x26: return (lhs.bits & rhs.bits) == rhs.bits;
    case 0x27: return (lhs.bits & rhs.bits) != 0;
    }
    return false;
}

bool evaluate_instructions(
    const std::span<const MilestoneProgramDefinition::Instruction> instructions,
    const MilestoneProgramContext& context) {
    std::array<Value, kMilestoneProgramMaximumStackDepth> stack{};
    std::size_t depth = 0;
    for (const auto& instruction : instructions) {
        if (instruction.opcode == 0x01) {
            stack[depth++] = load_field(instruction.field, context);
        } else if (instruction.opcode == 0x02) {
            stack[depth++] = load_query(instruction, context);
        } else if (instruction.opcode >= 0x10 && instruction.opcode <= 0x17) {
            Value value{};
            value.available = instruction.available;
            value.bits = instruction.bits;
            value.symbol = instruction.symbol;
            value.symbolLength = instruction.symbolLength;
            switch (instruction.opcode) {
            case 0x10: value.type = ValueType::Bool; break;
            case 0x11: value.type = ValueType::U32; break;
            case 0x12: value.type = ValueType::U64; break;
            case 0x13: value.type = ValueType::I32; break;
            case 0x14: value.type = ValueType::F32; break;
            case 0x15: value.type = instruction.field == 1 ? ValueType::BoundaryKind : ValueType::Symbol; break;
            case 0x16: case 0x17: value.type = ValueType::Procedure; break;
            }
            stack[depth++] = value;
        } else if (instruction.opcode >= 0x20 && instruction.opcode <= 0x27) {
            const Value rhs = stack[--depth];
            const Value lhs = stack[--depth];
            stack[depth++] = Value{.type = ValueType::Bool,
                                   .available = lhs.available && rhs.available,
                                   .bits = compare_values(lhs, rhs, instruction.opcode) ? 1u : 0u};
        } else if (instruction.opcode == 0x30) {
            stack[depth - 1].bits = stack[depth - 1].bits == 0 ? 1 : 0;
        } else {
            const Value rhs = stack[--depth];
            Value& lhs = stack[depth - 1];
            lhs.available = lhs.available && rhs.available;
            lhs.bits = instruction.opcode == 0x31 ? (lhs.bits != 0 && rhs.bits != 0)
                                                  : (lhs.bits != 0 || rhs.bits != 0);
        }
    }
    return depth == 1 && stack[0].available && stack[0].bits != 0;
}

}  // namespace

bool MilestoneProgramDefinition::evaluate(const MilestoneProgramContext& context) const {
    return mSequenceStepEnds.empty() && evaluate_instructions(mInstructions, context);
}

bool MilestoneProgramDefinition::evaluateSequenceStep(
    const std::size_t step, const MilestoneProgramContext& context) const {
    if (step >= mSequenceStepEnds.size()) return false;
    const std::size_t begin = step == 0 ? 0 : mSequenceStepEnds[step - 1];
    const std::size_t end = mSequenceStepEnds[step];
    return evaluate_instructions(
        std::span<const Instruction>(mInstructions).subspan(begin, end - begin), context);
}

const MilestoneProgramDefinition* MilestoneProgram::find(const std::string_view id) const {
    const auto found = std::ranges::find(mDefinitions, id, &MilestoneProgramDefinition::id);
    return found == mDefinitions.end() ? nullptr : &*found;
}

MilestoneProgramError decode_milestone_program(const std::span<const std::uint8_t> bytes,
    const MilestoneProgramSymbolResolver resolver, MilestoneProgram& output) {
    output = {};
    if (bytes.size() > kMilestoneProgramMaximumBytes) return MilestoneProgramError::TooLarge;
    if (bytes.size() < HeaderSize) return MilestoneProgramError::Truncated;
    if (!std::equal(kMilestoneProgramMagic.begin(), kMilestoneProgramMagic.end(), bytes.begin()))
        return MilestoneProgramError::BadMagic;
    const std::uint16_t wireMinor = read_u16(bytes.data() + 6);
    if (read_u16(bytes.data() + 4) != kMilestoneProgramWireMajor ||
        wireMinor > kMilestoneProgramWireMinor)
        return MilestoneProgramError::UnsupportedWireVersion;
    const std::uint16_t languageMinor = read_u16(bytes.data() + 10);
    if (read_u16(bytes.data() + 8) != kMilestoneProgramLanguageMajor ||
        languageMinor > kMilestoneProgramLanguageMinor || languageMinor != wireMinor)
        return MilestoneProgramError::UnsupportedLanguageVersion;
    const std::uint16_t definitionCount = read_u16(bytes.data() + 12);
    if (definitionCount == 0 || definitionCount > kMilestoneProgramMaximumDefinitions)
        return MilestoneProgramError::InvalidDefinitionCount;
    if (read_u16(bytes.data() + 14) != 0) return MilestoneProgramError::InvalidReservedData;
    if (read_u32(bytes.data() + 16) != bytes.size() - HeaderSize)
        return MilestoneProgramError::InvalidPayloadLength;

    Sha256 programHasher;
    programHasher.update(ProgramDomain);
    programHasher.update(bytes.first(20));
    programHasher.update(bytes.subspan(HeaderSize));
    const auto programDigest = programHasher.finish();
    if (!digest_matches(programDigest,
            std::span<const std::uint8_t, 32>(bytes.data() + 20, 32)))
        return MilestoneProgramError::InvalidProgramDigest;

    std::size_t cursor = HeaderSize;
    output.mDefinitions.reserve(definitionCount);
    for (std::size_t definitionIndex = 0; definitionIndex < definitionCount; ++definitionIndex) {
        if (bytes.size() - cursor < 4) return MilestoneProgramError::Truncated;
        const std::size_t recordStart = cursor;
        const std::uint32_t recordLength = read_u32(bytes.data() + cursor);
        cursor += 4;
        if (recordLength > bytes.size() - cursor || recordLength < 44)
            return MilestoneProgramError::InvalidRecordLength;
        const std::size_t recordEnd = cursor + recordLength;
        if (recordEnd - cursor < 2) return MilestoneProgramError::Truncated;
        const std::uint16_t nameLength = read_u16(bytes.data() + cursor);
        cursor += 2;
        if (nameLength == 0 || nameLength > kMilestoneProgramMaximumNameBytes ||
            nameLength > recordEnd - cursor)
            return MilestoneProgramError::InvalidName;
        const std::string_view name(reinterpret_cast<const char*>(bytes.data() + cursor), nameLength);
        if (!valid_utf8(name)) return MilestoneProgramError::InvalidName;
        if (output.find(name) != nullptr || find_milestone(name) != nullptr)
            return MilestoneProgramError::DuplicateName;
        cursor += nameLength;
        if (recordEnd - cursor < 42) return MilestoneProgramError::Truncated;
        const std::uint8_t phaseByte = bytes[cursor++];
        if (phaseByte > 1) return MilestoneProgramError::InvalidPhase;
        if (bytes[cursor++] != 0) return MilestoneProgramError::InvalidReservedData;
        const std::uint16_t stableTicks = read_u16(bytes.data() + cursor);
        cursor += 2;
        if (stableTicks == 0) return MilestoneProgramError::InvalidStableTicks;
        const std::uint16_t opCount = read_u16(bytes.data() + cursor);
        cursor += 2;
        if (opCount == 0 || opCount > kMilestoneProgramMaximumOps)
            return MilestoneProgramError::InvalidOpCount;
        const std::uint32_t bytecodeLength = read_u32(bytes.data() + cursor);
        cursor += 4;
        if (bytecodeLength == 0 || bytecodeLength > recordEnd - cursor - 32)
            return MilestoneProgramError::InvalidBytecodeLength;
        const std::size_t digestOffset = cursor;
        cursor += 32;
        if (bytecodeLength != recordEnd - cursor)
            return MilestoneProgramError::InvalidRecordLength;

        Sha256 definitionHasher;
        definitionHasher.update(DefinitionDomain);
        definitionHasher.update(bytes.subspan(recordStart + 4, digestOffset - (recordStart + 4)));
        definitionHasher.update(bytes.subspan(cursor, bytecodeLength));
        const auto definitionDigest = definitionHasher.finish();
        if (!digest_matches(definitionDigest,
                std::span<const std::uint8_t, 32>(bytes.data() + digestOffset, 32)))
            return MilestoneProgramError::InvalidDefinitionDigest;

        MilestoneProgramDefinition definition;
        definition.id = std::string(name);
        definition.phase = static_cast<MilestoneProgramPhase>(phaseByte);
        definition.stableTicks = stableTicks;
        definition.definitionDigest = hex_digest(definitionDigest);
        definition.mInstructions.reserve(opCount);
        std::array<TypeEntry, kMilestoneProgramMaximumStackDepth> types{};
        std::size_t depth = 0;
        std::uint8_t expectedSequenceSteps = 0;
        std::uint8_t projectionItemsRemaining = 0;
        std::optional<std::size_t> projectionStart;
        bool projectionMetadataStarted = false;
        const std::size_t bytecodeEnd = cursor + bytecodeLength;
        for (std::size_t opIndex = 0; opIndex < opCount; ++opIndex) {
            if (cursor >= bytecodeEnd) return MilestoneProgramError::Truncated;
            const std::size_t operationStart = cursor;
            MilestoneProgramDefinition::Instruction instruction{.opcode = bytes[cursor++]};
            bool retainInstruction = true;
            const auto push = [&](const ValueType type, const std::optional<std::size_t> index) {
                if (depth >= types.size()) return false;
                types[depth++] = {type, index};
                return true;
            };
            if (projectionMetadataStarted && instruction.opcode < 0x50)
                return MilestoneProgramError::InvalidOpcode;
            if (instruction.opcode == 0x50) {
                const bool expressionComplete = expectedSequenceSteps != 0
                    ? depth == 0 &&
                          definition.mSequenceStepEnds.size() == expectedSequenceSteps
                    : depth == 1 && types[0].type == ValueType::Bool;
                if (languageMinor < 4 || !expressionComplete || projectionItemsRemaining != 0 ||
                    definition.mValueProjections.size() >=
                        kMilestoneProgramMaximumProjections ||
                    cursor >= bytecodeEnd)
                    return MilestoneProgramError::InvalidOpcode;
                projectionMetadataStarted = true;
                const std::uint8_t nameLength = bytes[cursor++];
                if (nameLength == 0 || nameLength > kMilestoneProgramMaximumNameBytes ||
                    bytecodeEnd - cursor < static_cast<std::size_t>(nameLength) + 1)
                    return MilestoneProgramError::InvalidName;
                const std::string_view projectionName(
                    reinterpret_cast<const char*>(bytes.data() + cursor), nameLength);
                if (!valid_utf8(projectionName)) return MilestoneProgramError::InvalidName;
                if (std::ranges::any_of(definition.mValueProjections,
                        [&](const MilestoneValueProjection& projection) {
                            return projection.name == projectionName;
                        }))
                    return MilestoneProgramError::DuplicateName;
                cursor += nameLength;
                projectionItemsRemaining = bytes[cursor++];
                if (projectionItemsRemaining == 0 ||
                    projectionItemsRemaining > kMilestoneProgramMaximumProjectionItems)
                    return MilestoneProgramError::InvalidOpCount;
                definition.mValueProjections.push_back(
                    {.name = std::string(projectionName), .identity = {}, .items = {}});
                definition.mValueProjections.back().items.reserve(projectionItemsRemaining);
                projectionStart = operationStart;
                retainInstruction = false;
            } else if (instruction.opcode >= 0x51 && instruction.opcode <= 0x53) {
                if (languageMinor < 4 || projectionItemsRemaining == 0 ||
                    definition.mValueProjections.empty() || !projectionStart.has_value())
                    return MilestoneProgramError::InvalidOpcode;
                MilestoneValueProjectionItem item;
                item.kind = static_cast<MilestoneValueProjectionKind>(instruction.opcode - 0x50);
                if (instruction.opcode == 0x51) {
                    if (cursor >= bytecodeEnd) return MilestoneProgramError::Truncated;
                    item.selector = bytes[cursor++];
                    if (item.selector > 1) return MilestoneProgramError::InvalidField;
                } else if (instruction.opcode == 0x52) {
                    if (bytecodeEnd - cursor < 9) return MilestoneProgramError::Truncated;
                    std::copy_n(reinterpret_cast<const char*>(bytes.data() + cursor),
                        item.stage.size(), item.stage.begin());
                    cursor += item.stage.size();
                    item.room = static_cast<std::int8_t>(bytes[cursor++]);
                    if (!fixed_stage_valid(item.stage) || item.room < -1 || item.room > 63)
                        return MilestoneProgramError::InvalidField;
                } else {
                    if (bytecodeEnd - cursor < 4) return MilestoneProgramError::Truncated;
                    item.selector = bytes[cursor++];
                    item.room = static_cast<std::int8_t>(bytes[cursor++]);
                    item.index = read_u16(bytes.data() + cursor);
                    cursor += 2;
                    const std::array<std::uint16_t, 4> limits{822, 185, 64, 240};
                    if (item.selector >= limits.size() || item.index >= limits[item.selector] ||
                        (item.selector == 3 && (item.room < 0 || item.room > 63)) ||
                        (item.selector != 3 && item.room != -1))
                        return MilestoneProgramError::InvalidField;
                }
                auto& projection = definition.mValueProjections.back();
                const bool duplicate = std::ranges::any_of(
                    projection.items, [&](const MilestoneValueProjectionItem& existing) {
                        return existing.kind == item.kind && existing.selector == item.selector &&
                               existing.stage == item.stage && existing.room == item.room &&
                               existing.index == item.index;
                    });
                if (duplicate) return MilestoneProgramError::InvalidField;
                projection.items.push_back(item);
                --projectionItemsRemaining;
                if (projectionItemsRemaining == 0) {
                    Sha256 projectionHasher;
                    projectionHasher.update(ProjectionDomain);
                    projectionHasher.update(bytes.subspan(*projectionStart, cursor - *projectionStart));
                    projection.identity = hex_digest(projectionHasher.finish());
                    projectionStart.reset();
                }
                retainInstruction = false;
            } else if (instruction.opcode == 0x40) {
                if (languageMinor < 3 || opIndex != 0 || stableTicks != 1 || depth != 0 ||
                    bytecodeEnd - cursor < 3)
                    return MilestoneProgramError::InvalidOpcode;
                definition.mSequenceWithinTicks = read_u16(bytes.data() + cursor);
                cursor += 2;
                expectedSequenceSteps = bytes[cursor++];
                if (definition.mSequenceWithinTicks == 0 || expectedSequenceSteps < 2 ||
                    expectedSequenceSteps > 16)
                    return MilestoneProgramError::InvalidOpcode;
                retainInstruction = false;
            } else if (instruction.opcode == 0x41) {
                if (languageMinor < 3 || expectedSequenceSteps == 0 || depth != 1 ||
                    types[0].type != ValueType::Bool ||
                    definition.mSequenceStepEnds.size() >= expectedSequenceSteps)
                    return MilestoneProgramError::InvalidResult;
                definition.mSequenceStepEnds.push_back(definition.mInstructions.size());
                depth = 0;
                retainInstruction = false;
            } else if (instruction.opcode == 0x01) {
                if (cursor >= bytecodeEnd) return MilestoneProgramError::Truncated;
                instruction.field = bytes[cursor++];
                if (languageMinor == 0 && instruction.field > 22)
                    return MilestoneProgramError::InvalidField;
                if (languageMinor < 5 && instruction.field >= 59)
                    return MilestoneProgramError::InvalidField;
                if (languageMinor < 6 && instruction.field >= 70)
                    return MilestoneProgramError::InvalidField;
                const auto type = field_type(instruction.field);
                if (!type.has_value()) return MilestoneProgramError::InvalidField;
                if (!push(*type, std::nullopt)) return MilestoneProgramError::StackOverflow;
            } else if (instruction.opcode == 0x02) {
                if (languageMinor < 2) return MilestoneProgramError::InvalidOpcode;
                if (cursor >= bytecodeEnd) return MilestoneProgramError::Truncated;
                instruction.queryKind = bytes[cursor++];
                if (instruction.queryKind >= 3 && languageMinor < 3)
                    return MilestoneProgramError::InvalidOpcode;
                ValueType type = ValueType::Bool;
                if (instruction.queryKind == 1) {
                    if (bytecodeEnd - cursor < 14) return MilestoneProgramError::Truncated;
                    instruction.field = bytes[cursor++];
                    const auto actorType = actor_fact_type(instruction.field);
                    if (!actorType.has_value()) return MilestoneProgramError::InvalidField;
                    type = *actorType;
                    std::copy_n(reinterpret_cast<const char*>(bytes.data() + cursor),
                        instruction.queryStage.size(), instruction.queryStage.begin());
                    cursor += instruction.queryStage.size();
                    instruction.queryRoom = static_cast<std::int8_t>(bytes[cursor++]);
                    instruction.querySetId = read_u16(bytes.data() + cursor);
                    cursor += 2;
                    instruction.queryActorName =
                        static_cast<std::int16_t>(read_u16(bytes.data() + cursor));
                    cursor += 2;
                    if (!fixed_stage_valid(instruction.queryStage) ||
                        instruction.queryRoom < -1 || instruction.queryRoom > 63 ||
                        instruction.querySetId == std::numeric_limits<std::uint16_t>::max() ||
                        instruction.queryActorName < 0)
                        return MilestoneProgramError::InvalidField;
                } else if (instruction.queryKind == 2) {
                    if (bytecodeEnd - cursor < 4) return MilestoneProgramError::Truncated;
                    instruction.field = bytes[cursor++];
                    instruction.queryRoom = static_cast<std::int8_t>(bytes[cursor++]);
                    instruction.queryIndex = read_u16(bytes.data() + cursor);
                    cursor += 2;
                    const std::array<std::uint16_t, 4> limits{822, 185, 64, 240};
                    if (instruction.field >= limits.size() ||
                        instruction.queryIndex >= limits[instruction.field] ||
                        (instruction.field == 3 &&
                            (instruction.queryRoom < 0 || instruction.queryRoom > 63)) ||
                        (instruction.field != 3 && instruction.queryRoom != -1))
                        return MilestoneProgramError::InvalidField;
                } else if (instruction.queryKind == 3 || instruction.queryKind == 4) {
                    if (bytecodeEnd - cursor < 24) return MilestoneProgramError::Truncated;
                    for (float& value : instruction.queryValues) {
                        value = std::bit_cast<float>(read_u32(bytes.data() + cursor));
                        cursor += 4;
                        if (!std::isfinite(value) || (value == 0.0F && std::signbit(value)))
                            return MilestoneProgramError::InvalidFloat;
                    }
                    if (instruction.queryKind == 3) {
                        type = ValueType::Bool;
                        for (std::size_t axis = 0; axis < 3; ++axis) {
                            if (instruction.queryValues[axis] >
                                instruction.queryValues[axis + 3])
                                return MilestoneProgramError::InvalidField;
                        }
                    } else {
                        type = ValueType::F32;
                        const double nx = instruction.queryValues[3];
                        const double ny = instruction.queryValues[4];
                        const double nz = instruction.queryValues[5];
                        const double lengthSquared = nx * nx + ny * ny + nz * nz;
                        if (lengthSquared == 0.0 || !std::isfinite(lengthSquared))
                            return MilestoneProgramError::InvalidField;
                    }
                } else {
                    return MilestoneProgramError::InvalidField;
                }
                if (!push(type, std::nullopt)) return MilestoneProgramError::StackOverflow;
            } else if (instruction.opcode == 0x10) {
                if (cursor >= bytecodeEnd) return MilestoneProgramError::Truncated;
                instruction.bits = bytes[cursor++];
                if (instruction.bits > 1) return MilestoneProgramError::InvalidBoolean;
                if (!push(ValueType::Bool, definition.mInstructions.size()))
                    return MilestoneProgramError::StackOverflow;
            } else if (instruction.opcode >= 0x11 && instruction.opcode <= 0x14) {
                const std::size_t width = instruction.opcode == 0x12 ? 8 : 4;
                if (bytecodeEnd - cursor < width) return MilestoneProgramError::Truncated;
                instruction.bits = width == 8 ? read_u64(bytes.data() + cursor)
                                              : read_u32(bytes.data() + cursor);
                cursor += width;
                const ValueType type = instruction.opcode == 0x11 ? ValueType::U32
                                     : instruction.opcode == 0x12 ? ValueType::U64
                                     : instruction.opcode == 0x13 ? ValueType::I32
                                                                  : ValueType::F32;
                if (type == ValueType::F32) {
                    const float value = std::bit_cast<float>(
                        static_cast<std::uint32_t>(instruction.bits));
                    if (!std::isfinite(value) || (value == 0.0F && std::signbit(value)))
                        return MilestoneProgramError::InvalidFloat;
                }
                if (!push(type, definition.mInstructions.size()))
                    return MilestoneProgramError::StackOverflow;
            } else if (instruction.opcode == 0x15 || instruction.opcode == 0x17) {
                if (cursor >= bytecodeEnd) return MilestoneProgramError::Truncated;
                const std::uint8_t length = bytes[cursor++];
                if (length == 0 || length > kMilestoneProgramMaximumSymbolBytes ||
                    bytecodeEnd - cursor < length)
                    return MilestoneProgramError::InvalidSymbol;
                const std::string_view symbol(
                    reinterpret_cast<const char*>(bytes.data() + cursor), length);
                if (!valid_utf8(symbol)) return MilestoneProgramError::InvalidSymbol;
                std::copy_n(symbol.data(), length, instruction.symbol.begin());
                instruction.symbolLength = length;
                cursor += length;
                ValueType type = ValueType::Symbol;
                if (instruction.opcode == 0x17) {
                    std::uint32_t resolved = 0;
                    if (resolver == nullptr ||
                        !resolver(MilestoneProgramSymbolKind::PlayerProcedure, symbol, resolved))
                        return MilestoneProgramError::UnknownSymbol;
                    instruction.bits = resolved;
                    instruction.symbolLength = 0;
                    type = ValueType::Procedure;
                }
                if (!push(type, definition.mInstructions.size()))
                    return MilestoneProgramError::StackOverflow;
            } else if (instruction.opcode == 0x16) {
                if (bytecodeEnd - cursor < 4) return MilestoneProgramError::Truncated;
                instruction.bits = read_u32(bytes.data() + cursor);
                cursor += 4;
                if (!push(ValueType::Procedure, definition.mInstructions.size()))
                    return MilestoneProgramError::StackOverflow;
            } else if (instruction.opcode >= 0x20 && instruction.opcode <= 0x27) {
                if (languageMinor == 0 && instruction.opcode > 0x25)
                    return MilestoneProgramError::InvalidOpcode;
                if (depth < 2) return MilestoneProgramError::StackUnderflow;
                TypeEntry rhs = types[--depth];
                TypeEntry lhs = types[--depth];
                if (lhs.type == ValueType::Symbol && rhs.type == ValueType::BoundaryKind &&
                    lhs.instruction.has_value()) {
                    if (!normalize_symbol_comparison(
                            definition.mInstructions[*lhs.instruction], lhs.type, rhs.type))
                        return MilestoneProgramError::UnknownSymbol;
                } else if (rhs.type == ValueType::Symbol && lhs.type == ValueType::BoundaryKind &&
                           rhs.instruction.has_value()) {
                    if (!normalize_symbol_comparison(
                            definition.mInstructions[*rhs.instruction], rhs.type, lhs.type))
                        return MilestoneProgramError::UnknownSymbol;
                }
                if (lhs.type != rhs.type) return MilestoneProgramError::TypeMismatch;
                if (lhs.type == ValueType::Symbol) {
                    const auto valid_constant = [&](const TypeEntry& entry) {
                        if (!entry.instruction.has_value()) return true;
                        const auto& constant = definition.mInstructions[*entry.instruction];
                        return valid_stage_symbol(std::string_view(
                            constant.symbol.data(), constant.symbolLength));
                    };
                    if (!valid_constant(lhs) || !valid_constant(rhs))
                        return MilestoneProgramError::InvalidSymbol;
                }
                const bool equality = instruction.opcode == 0x20 || instruction.opcode == 0x21;
                const bool bitmask = instruction.opcode == 0x26 || instruction.opcode == 0x27;
                const bool ordered = lhs.type == ValueType::U32 || lhs.type == ValueType::U64 ||
                                     lhs.type == ValueType::I32 || lhs.type == ValueType::F32;
                if ((bitmask && lhs.type != ValueType::U32 && lhs.type != ValueType::U64) ||
                    (bitmask && rhs.instruction.has_value() &&
                        definition.mInstructions[*rhs.instruction].bits == 0) ||
                    (!equality && !bitmask && !ordered) ||
                    (lhs.type == ValueType::Bool && !equality))
                    return MilestoneProgramError::TypeMismatch;
                if (!push(ValueType::Bool, std::nullopt))
                    return MilestoneProgramError::StackOverflow;
            } else if (instruction.opcode == 0x30) {
                if (depth < 1) return MilestoneProgramError::StackUnderflow;
                if (types[depth - 1].type != ValueType::Bool)
                    return MilestoneProgramError::TypeMismatch;
                types[depth - 1].instruction.reset();
            } else if (instruction.opcode == 0x31 || instruction.opcode == 0x32) {
                if (depth < 2) return MilestoneProgramError::StackUnderflow;
                const TypeEntry rhs = types[--depth];
                const TypeEntry lhs = types[--depth];
                if (lhs.type != ValueType::Bool || rhs.type != ValueType::Bool)
                    return MilestoneProgramError::TypeMismatch;
                if (!push(ValueType::Bool, std::nullopt))
                    return MilestoneProgramError::StackOverflow;
            } else {
                return MilestoneProgramError::InvalidOpcode;
            }
            if (retainInstruction) definition.mInstructions.push_back(instruction);
        }
        if (cursor != bytecodeEnd || projectionItemsRemaining != 0)
            return MilestoneProgramError::InvalidBytecodeLength;
        if (expectedSequenceSteps != 0) {
            if (depth != 0 || definition.mSequenceStepEnds.size() != expectedSequenceSteps)
                return MilestoneProgramError::InvalidResult;
        } else if (depth != 1 || types[0].type != ValueType::Bool) {
            return MilestoneProgramError::InvalidResult;
        }
        output.mDefinitions.push_back(std::move(definition));
    }
    if (cursor != bytes.size()) return MilestoneProgramError::TrailingData;
    output.mDigest = hex_digest(programDigest);
    return MilestoneProgramError::None;
}

const char* milestone_program_error_message(const MilestoneProgramError error) {
    switch (error) {
    case MilestoneProgramError::None: return "no error";
    case MilestoneProgramError::TooLarge: return "program exceeds the 1 MiB limit";
    case MilestoneProgramError::Truncated: return "truncated milestone program";
    case MilestoneProgramError::BadMagic: return "bad milestone program magic";
    case MilestoneProgramError::UnsupportedWireVersion: return "unsupported milestone wire version";
    case MilestoneProgramError::UnsupportedLanguageVersion: return "unsupported milestone language version";
    case MilestoneProgramError::InvalidReservedData: return "reserved milestone data is nonzero";
    case MilestoneProgramError::InvalidPayloadLength: return "invalid milestone payload length";
    case MilestoneProgramError::InvalidProgramDigest: return "milestone program SHA-256 mismatch";
    case MilestoneProgramError::InvalidDefinitionCount: return "invalid milestone definition count";
    case MilestoneProgramError::InvalidRecordLength: return "invalid milestone record length";
    case MilestoneProgramError::InvalidName: return "invalid milestone name";
    case MilestoneProgramError::DuplicateName: return "duplicate milestone name";
    case MilestoneProgramError::InvalidPhase: return "invalid milestone phase";
    case MilestoneProgramError::InvalidStableTicks: return "stable_ticks must be at least one";
    case MilestoneProgramError::InvalidOpCount: return "invalid milestone operation count";
    case MilestoneProgramError::InvalidBytecodeLength: return "invalid milestone bytecode length";
    case MilestoneProgramError::InvalidDefinitionDigest: return "milestone definition SHA-256 mismatch";
    case MilestoneProgramError::InvalidOpcode: return "invalid milestone opcode";
    case MilestoneProgramError::InvalidField: return "invalid milestone field";
    case MilestoneProgramError::InvalidBoolean: return "invalid milestone boolean constant";
    case MilestoneProgramError::InvalidFloat: return "invalid milestone float constant";
    case MilestoneProgramError::InvalidSymbol: return "invalid milestone symbol";
    case MilestoneProgramError::UnknownSymbol: return "unknown milestone symbol";
    case MilestoneProgramError::StackUnderflow: return "milestone stack underflow";
    case MilestoneProgramError::StackOverflow: return "milestone stack overflow";
    case MilestoneProgramError::TypeMismatch: return "milestone operand type mismatch";
    case MilestoneProgramError::InvalidResult: return "milestone predicate must leave one boolean";
    case MilestoneProgramError::TrailingData: return "trailing milestone program data";
    }
    return "unknown milestone program error";
}

MilestoneProgram& milestone_program() {
    static MilestoneProgram program;
    return program;
}

}  // namespace dusk::automation
