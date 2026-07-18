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
inline constexpr std::uint16_t kMilestoneProgramWireMinor = 5;
inline constexpr std::uint16_t kMilestoneProgramLanguageMajor = 1;
inline constexpr std::uint16_t kMilestoneProgramLanguageMinor = 5;
inline constexpr std::size_t kMilestoneProgramMaximumBytes = 1024 * 1024;
inline constexpr std::size_t kMilestoneProgramMaximumDefinitions = 256;
inline constexpr std::size_t kMilestoneProgramMaximumNameBytes = 96;
inline constexpr std::size_t kMilestoneProgramMaximumSymbolBytes = 64;
inline constexpr std::size_t kMilestoneProgramMaximumOps = 256;
inline constexpr std::size_t kMilestoneProgramMaximumProjections = 8;
inline constexpr std::size_t kMilestoneProgramMaximumProjectionItems = 32;
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

enum class MilestoneValueProjectionKind : std::uint8_t {
    Rng = 1,
    ActorPopulation = 2,
    Flag = 3,
};

struct MilestoneValueProjectionItem {
    MilestoneValueProjectionKind kind = MilestoneValueProjectionKind::Rng;
    std::uint8_t selector = 0;
    std::array<char, 8> stage{};
    std::int8_t room = -1;
    std::uint16_t index = 0;
};

struct MilestoneValueProjection {
    std::string name;
    std::string identity;
    std::vector<MilestoneValueProjectionItem> items;
};

struct MilestoneProgramDefinition {
    // Canonical, already-validated stack-machine instruction. Public only so the bounded
    // decoder can be implemented in small helpers; callers should use evaluate().
    struct Instruction {
        std::uint8_t opcode = 0;
        std::uint8_t field = 0;
        std::uint8_t queryKind = 0;
        std::array<char, 8> queryStage{};
        std::int8_t queryRoom = -1;
        std::uint16_t querySetId = 0xffff;
        std::int16_t queryActorName = -1;
        std::uint16_t queryIndex = 0;
        std::array<float, 6> queryValues{};
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
    [[nodiscard]] std::size_t sequenceStepCount() const { return mSequenceStepEnds.size(); }
    [[nodiscard]] std::uint16_t sequenceWithinTicks() const { return mSequenceWithinTicks; }
    [[nodiscard]] bool evaluateSequenceStep(
        std::size_t step, const MilestoneProgramContext& context) const;
    [[nodiscard]] std::span<const MilestoneValueProjection> valueProjections() const {
        return mValueProjections;
    }

private:
    std::vector<Instruction> mInstructions;
    std::vector<std::size_t> mSequenceStepEnds;
    std::uint16_t mSequenceWithinTicks = 0;
    std::vector<MilestoneValueProjection> mValueProjections;

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
