#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace dusk::automation {

struct MilestoneObservation;

inline constexpr std::array<std::uint8_t, 4> kMilestoneProgramMagic{'D', 'M', 'S', 'P'};
inline constexpr std::uint16_t kMilestoneProgramWireMajor = 1;
inline constexpr std::uint16_t kMilestoneProgramWireMinor = 0;
inline constexpr std::uint16_t kMilestoneProgramLanguageMajor = 1;
inline constexpr std::uint16_t kMilestoneProgramLanguageMinor = 0;
inline constexpr std::size_t kMilestoneProgramMaximumBytes = 1024 * 1024;
inline constexpr std::size_t kMilestoneProgramMaximumDefinitions = 256;
inline constexpr std::size_t kMilestoneProgramMaximumNameBytes = 96;
inline constexpr std::size_t kMilestoneProgramMaximumSymbolBytes = 64;
inline constexpr std::size_t kMilestoneProgramMaximumOps = 256;
inline constexpr std::size_t kMilestoneProgramMaximumStackDepth = 32;
inline constexpr std::uint64_t kMilestoneProgramUnavailableTapeFrame = ~std::uint64_t{0};

enum class MilestoneProgramPhase : std::uint8_t {
    PreInput = 0,
    PostSim = 1,
};

enum class MilestoneBoundaryKind : std::uint32_t {
    Boot = 0,
    Tick = 1,
};

enum class MilestoneProgramError {
    None,
    TooLarge,
    Truncated,
    BadMagic,
    UnsupportedWireVersion,
    UnsupportedLanguageVersion,
    InvalidReservedData,
    InvalidPayloadLength,
    InvalidProgramDigest,
    InvalidDefinitionCount,
    InvalidRecordLength,
    InvalidName,
    DuplicateName,
    InvalidPhase,
    InvalidStableTicks,
    InvalidOpCount,
    InvalidBytecodeLength,
    InvalidDefinitionDigest,
    InvalidOpcode,
    InvalidField,
    InvalidBoolean,
    InvalidFloat,
    InvalidSymbol,
    UnknownSymbol,
    StackUnderflow,
    StackOverflow,
    TypeMismatch,
    InvalidResult,
    TrailingData,
};

enum class MilestoneProgramSymbolKind {
    PlayerProcedure,
};

using MilestoneProgramSymbolResolver = bool (*)(MilestoneProgramSymbolKind kind,
    std::string_view symbol, std::uint32_t& value);

struct MilestoneProgramContext {
    const MilestoneObservation& observation;
    MilestoneProgramPhase phase = MilestoneProgramPhase::PostSim;
    MilestoneBoundaryKind boundaryKind = MilestoneBoundaryKind::Tick;
    std::uint64_t boundaryIndex = 0;
    std::optional<std::uint64_t> tapeFrame;
};

struct MilestoneProgramDefinition {
    // Canonical, already-validated stack-machine instruction. Public only so the bounded
    // decoder can be implemented in small helpers; callers should use evaluate().
    struct Instruction {
        std::uint8_t opcode = 0;
        std::uint8_t field = 0;
        std::uint64_t bits = 0;
        std::array<char, kMilestoneProgramMaximumSymbolBytes> symbol{};
        std::uint8_t symbolLength = 0;
        bool available = true;
    };

    std::string id;
    MilestoneProgramPhase phase = MilestoneProgramPhase::PostSim;
    std::uint16_t stableTicks = 1;
    std::string definitionDigest;

    [[nodiscard]] bool evaluate(const MilestoneProgramContext& context) const;

private:
    std::vector<Instruction> mInstructions;

    friend MilestoneProgramError decode_milestone_program(std::span<const std::uint8_t>,
        MilestoneProgramSymbolResolver, class MilestoneProgram&);
};

class MilestoneProgram {
public:
    [[nodiscard]] std::span<const MilestoneProgramDefinition> definitions() const {
        return mDefinitions;
    }
    [[nodiscard]] const MilestoneProgramDefinition* find(std::string_view id) const;
    [[nodiscard]] std::string_view digest() const { return mDigest; }
    [[nodiscard]] bool empty() const { return mDefinitions.empty(); }

private:
    std::vector<MilestoneProgramDefinition> mDefinitions;
    std::string mDigest;

    friend MilestoneProgramError decode_milestone_program(std::span<const std::uint8_t>,
        MilestoneProgramSymbolResolver, MilestoneProgram&);
};

[[nodiscard]] MilestoneProgramError decode_milestone_program(
    std::span<const std::uint8_t> bytes, MilestoneProgramSymbolResolver resolver,
    MilestoneProgram& output);
[[nodiscard]] const char* milestone_program_error_message(MilestoneProgramError error);

MilestoneProgram& milestone_program();

/** Resolves exact game enum tokens such as PROC_CRAWL_MOVE without touching game state. */
bool resolve_game_milestone_symbol(MilestoneProgramSymbolKind kind,
    std::string_view symbol, std::uint32_t& value);

}  // namespace dusk::automation
