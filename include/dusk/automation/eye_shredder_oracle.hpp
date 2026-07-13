#pragma once

#include <array>
#include <cstdint>
#include <filesystem>
#include <string>

#include "dusk/automation/name_entry_observer.hpp"

namespace dusk::automation {

inline constexpr std::uint32_t EyeShredderOracleSchemaVersion = 2;

struct EyeShredderExpectedWrite {
    static constexpr std::uint8_t CharacterIndex = 113;
    static constexpr std::uint16_t OriginalOffset =
        NameEntryOriginalLayout::CharacterInfo +
        CharacterIndex * NameEntryOriginalLayout::CharacterInfoSize;
    static constexpr std::uint32_t FreshUsaDNameAddress = 0x81457034;
    static constexpr std::uint32_t FreshUsaGcCachedAddress = 0x81457688;
    static constexpr std::array<std::uint8_t, NameEntryOriginalLayout::CharacterInfoSize> Bytes{
        0x0C,
        0x00,
        0x02,
        0x01,
        0x00,
        0x00,
        0x00,
        0x4D,
    };
};

enum class EyeShredderOracleStatus : std::uint8_t {
    Idle,
    Running,
    Passed,
    Failed,
    Incomplete,
};

// Deliberately independent of Aurora's public ABI. The runtime may copy any
// renderer telemetry source into this stable oracle-facing snapshot.
struct EyeShredderRendererTelemetry {
    std::uint32_t xfNumChansRaw = 0;
    std::uint32_t bpNumChansRaw = 0;
    bool mismatchLatched = false;
    bool eyeShredderMismatchLatched = false;
    std::uint64_t mismatchDrawCount = 0;
};

struct EyeShredderOracleResult {
    EyeShredderOracleStatus status = EyeShredderOracleStatus::Idle;
    std::string reason;
    bool memoryMatched = false;
    bool hasActualWrite = false;
    NameEntryWriteObservation actualWrite{};
    std::uint64_t memoryMatchSimTick = NameEntryNoTick;
    std::uint64_t memoryMatchTapeFrame = NameEntryNoTick;
    bool rendererMatched = false;
    bool hasRendererTelemetry = false;
    EyeShredderRendererTelemetry rendererTelemetry{};
    std::uint64_t rendererMatchSimTick = NameEntryNoTick;
    std::uint64_t rendererMatchTapeFrame = NameEntryNoTick;
    std::uint64_t simTick = NameEntryNoTick;
    std::uint64_t tapeFrame = NameEntryNoTick;
};

class EyeShredderOracle {
public:
    void start();
    void evaluate(
        const NameEntryObservation& observation, std::uint64_t simTick, std::uint64_t tapeFrame);
    void observeRendererTelemetry(const EyeShredderRendererTelemetry& telemetry,
        std::uint64_t simTick, std::uint64_t tapeFrame);
    void finish(std::uint64_t simTick, std::uint64_t tapeFrame);
    void reject(std::string reason);

    bool isTerminal() const;
    const EyeShredderOracleResult& result() const { return mResult; }

private:
    void fail(std::string reason, const NameEntryWriteObservation* actual, std::uint64_t simTick,
        std::uint64_t tapeFrame);

    EyeShredderOracleResult mResult{};
    std::uint64_t mLastWriteAttempt = 0;
    std::uint64_t mRendererMismatchDrawCountAtMemoryMatch = 0;
    bool mSawNameEntry = false;
};

std::string serialize_eye_shredder_oracle_result(const EyeShredderOracleResult& result);
bool write_eye_shredder_oracle_result(const std::filesystem::path& path,
    const EyeShredderOracleResult& result, std::string& error) noexcept;

}  // namespace dusk::automation
