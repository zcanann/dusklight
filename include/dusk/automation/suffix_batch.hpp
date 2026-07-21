#pragma once

#include "dusk/automation/factorized_pad_policy.hpp"
#include "dusk/automation/input_tape.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace dusk::automation {

inline constexpr std::string_view LegacySuffixBatchSchema = "dusklight-suffix-batch/v2";
inline constexpr std::string_view PreviousSuffixBatchSchema = "dusklight-suffix-batch/v3";
inline constexpr std::string_view SuffixBatchSchema = "dusklight-suffix-batch/v4";
inline constexpr std::size_t SuffixBatchMaximumBytes = 64 * 1024 * 1024;
inline constexpr std::size_t SuffixBatchMaximumCandidates = 16384;
inline constexpr std::size_t SuffixBatchMaximumTicks = 4096;
inline constexpr std::size_t SuffixBatchMaximumExpandedTicks = 8 * 1024 * 1024;
inline constexpr std::size_t SuffixBatchMaximumValidationTicks = 256;

enum class SuffixCheckpointValidation : std::uint8_t {
    GameplayReadyFSp103 = 1,
    RecordedReplayWindow = 2,
};

struct SuffixBatchCandidate {
    std::string id;
    bool tapePassthrough = false;
    bool factorizedPolicy = false;
    FactorizedPadPolicyHeadConfig policyHead{};
    std::vector<std::array<float, kFactorizedPadPolicyHeadWidth>> policyOutputs;
    // One bounded index per expanded tick; the native boundary decodes the
    // selected continuous row online and verifies its consumed PAD below.
    std::vector<std::uint32_t> policyOutputIndexByTick;
    // Fully expanded before simulation. The hot path performs one indexed load.
    // For factorized candidates this is the independent expected-PAD oracle.
    std::vector<RawPadState> pads;
};

struct SuffixBatchDefinition {
    std::size_t sourceFrame = 0;
    std::string sourceBoundaryFingerprint;
    SuffixCheckpointValidation checkpointValidation =
        SuffixCheckpointValidation::GameplayReadyFSp103;
    std::size_t validationTicks = 0;
    std::size_t maximumTicks = 0;
    bool verifyStateHashes = false;
    std::vector<SuffixBatchCandidate> candidates;
};

/** Parses and bounds a complete batch without retaining the JSON tree. */
[[nodiscard]] bool parse_suffix_batch(
    std::string_view source, SuffixBatchDefinition& output, std::string& error);

}  // namespace dusk::automation
