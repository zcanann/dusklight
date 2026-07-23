#pragma once

#include "dusk/automation/factorized_pad_policy.hpp"
#include "dusk/automation/input_tape.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace dusk::automation {

inline constexpr std::string_view LegacySuffixBatchSchema = "dusklight-suffix-batch/v2";
inline constexpr std::string_view PreviousSuffixBatchSchema = "dusklight-suffix-batch/v3";
inline constexpr std::string_view FactorizedSuffixBatchSchema = "dusklight-suffix-batch/v4";
inline constexpr std::string_view SuffixBatchSchema = "dusklight-suffix-batch/v5";
inline constexpr std::string_view FrozenPolicySchema = "dusklight-native-frozen-policy/v1";
inline constexpr std::size_t SuffixBatchMaximumBytes = 64 * 1024 * 1024;
inline constexpr std::size_t SuffixBatchMaximumCandidates = 16384;
inline constexpr std::size_t SuffixBatchMaximumTicks = 4096;
inline constexpr std::size_t SuffixBatchMaximumExpandedTicks = 8 * 1024 * 1024;
inline constexpr std::size_t SuffixBatchMaximumValidationTicks = 256;

enum class SuffixCheckpointValidation : std::uint8_t {
    GameplayReadyFSp103 = 1,
    RecordedReplayWindow = 2,
};

enum class SuffixDemonstrationMode : std::uint8_t {
    Absent = 1,
    ReplayOnly = 2,
    BehaviorCloningWarmStart = 3,
    ReverseCurriculumCheckpoints = 4,
};

struct SuffixBatchCandidate {
    std::string id;
    bool tapePassthrough = false;
    bool factorizedPolicy = false;
    bool frozenPolicy = false;
    FactorizedPadPolicyHeadConfig policyHead{};
    std::vector<std::array<float, kFactorizedPadPolicyHeadWidth>> policyOutputs;
    // One bounded index per expanded tick; the native boundary decodes the
    // selected continuous row online and verifies its consumed PAD below.
    std::vector<std::uint32_t> policyOutputIndexByTick;
    // Fully expanded before simulation. The hot path performs one indexed load.
    // For factorized candidates this is the independent expected-PAD oracle.
    std::vector<RawPadState> pads;
};

struct SuffixBatchFrozenPolicy {
    std::string modelPath;
    std::string modelXxh3_128;
    FactorizedPadPolicyHeadConfig policyHead{};
};

struct SuffixBatchDefinition {
    std::size_t sourceFrame = 0;
    std::string sourceBoundaryFingerprint;
    SuffixDemonstrationMode demonstrationMode =
        SuffixDemonstrationMode::BehaviorCloningWarmStart;
    SuffixCheckpointValidation checkpointValidation =
        SuffixCheckpointValidation::GameplayReadyFSp103;
    std::size_t validationTicks = 0;
    std::size_t maximumTicks = 0;
    bool verifyStateHashes = false;
    std::optional<SuffixBatchFrozenPolicy> frozenPolicy;
    std::vector<SuffixBatchCandidate> candidates;
};

/** Parses and bounds a complete batch without retaining the JSON tree. */
[[nodiscard]] bool parse_suffix_batch(
    std::string_view source, SuffixBatchDefinition& output, std::string& error);

}  // namespace dusk::automation
