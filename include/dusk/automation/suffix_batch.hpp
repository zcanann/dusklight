#pragma once

#include "dusk/automation/input_tape.hpp"

#include <cstddef>
#include <string>
#include <string_view>
#include <vector>

namespace dusk::automation {

inline constexpr std::string_view SuffixBatchSchema = "dusklight-suffix-batch/v1";
inline constexpr std::size_t SuffixBatchMaximumBytes = 64 * 1024 * 1024;
inline constexpr std::size_t SuffixBatchMaximumCandidates = 16384;
inline constexpr std::size_t SuffixBatchMaximumTicks = 4096;
inline constexpr std::size_t SuffixBatchMaximumExpandedTicks = 8 * 1024 * 1024;

struct SuffixBatchCandidate {
    std::string id;
    // Fully expanded before simulation. The hot path performs one indexed load.
    std::vector<RawPadState> pads;
};

struct SuffixBatchDefinition {
    std::size_t sourceFrame = 0;
    std::size_t maximumTicks = 0;
    bool verifyStateHashes = false;
    std::vector<SuffixBatchCandidate> candidates;
};

/** Parses and bounds a complete batch without retaining the JSON tree. */
[[nodiscard]] bool parse_suffix_batch(
    std::string_view source, SuffixBatchDefinition& output, std::string& error);

}  // namespace dusk::automation
