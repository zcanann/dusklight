#pragma once

#include "dusk/automation/factorized_pad_policy.hpp"
#include "dusk/automation/input_controller.hpp"
#include "dusk/automation/input_tape.hpp"

#include <algorithm>
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
inline constexpr std::string_view ReactiveSuffixBatchSchema = "dusklight-suffix-batch/v8";
inline constexpr std::string_view FactorizedSuffixBatchSchema = "dusklight-suffix-batch/v4";
inline constexpr std::string_view FrozenPolicySuffixBatchSchemaV6 = "dusklight-suffix-batch/v6";
inline constexpr std::string_view SuffixBatchSchema = "dusklight-suffix-batch/v7";
inline constexpr std::string_view FrozenPolicySchemaV1 = "dusklight-native-frozen-policy/v1";
inline constexpr std::string_view FrozenPolicySchema = "dusklight-native-frozen-policy/v2";
inline constexpr std::string_view PolicyRolloutExplorationSchema =
    "dusklight-native-policy-rollout-exploration/v1";
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

enum class SuffixPolicyActionAuthority : std::uint8_t {
    EpisodePolicy = 1,
};

struct SuffixBatchCandidate {
    std::string id;
    bool tapePassthrough = false;
    bool factorizedPolicy = false;
    bool frozenPolicy = false;
    bool controllerProgram = false;
    std::size_t controllerStartTick = 0;
    InputControllerProgram controller{};
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
    struct RolloutExploration {
        std::uint64_t seed = 0;
        std::uint32_t stickAxisDeltaProbabilityMillionths = 0;
        std::uint8_t maximumStickAxisDelta = 0;
        std::uint32_t buttonFlipProbabilityMillionths = 0;
        std::uint16_t buttonFlipMask = 0;
    };
    std::optional<RolloutExploration> rolloutExploration;
};

[[nodiscard]] inline std::uint64_t policy_exploration_sample(
    const std::uint64_t seed, const std::uint64_t tick, const std::uint64_t stream) noexcept
{
    std::uint64_t value = seed + 0x9e3779b97f4a7c15ULL * (tick + 1) +
                          0xd1b54a32d192ed03ULL * (stream + 1);
    value = (value ^ (value >> 30)) * 0xbf58476d1ce4e5b9ULL;
    value = (value ^ (value >> 27)) * 0x94d049bb133111ebULL;
    return value ^ (value >> 31);
}

inline void apply_policy_rollout_exploration(RawPadState& pad,
    const SuffixBatchFrozenPolicy::RolloutExploration& config, const std::uint64_t tick) noexcept
{
    constexpr std::uint64_t Scale = 1'000'000;
    std::array<std::int8_t*, 4> axes{
        &pad.stickX, &pad.stickY, &pad.substickX, &pad.substickY};
    const std::int16_t maximumDelta = config.maximumStickAxisDelta;
    const std::uint64_t width = std::uint64_t{config.maximumStickAxisDelta} * 2 + 1;
    for (std::size_t stream = 0; stream < axes.size(); ++stream) {
        const std::uint64_t sample = policy_exploration_sample(config.seed, tick, stream);
        if (sample % Scale < config.stickAxisDeltaProbabilityMillionths) {
            const std::int16_t delta = static_cast<std::int16_t>((sample / Scale) % width) -
                                       maximumDelta;
            const auto explored = static_cast<std::int16_t>(
                static_cast<std::int16_t>(*axes[stream]) + delta);
            *axes[stream] = static_cast<std::int8_t>(std::clamp(
                explored,
                std::int16_t{-128}, std::int16_t{127}));
        }
    }
    for (std::uint32_t bit = 0; bit < 16; ++bit) {
        const auto mask = static_cast<std::uint16_t>(std::uint16_t{1} << bit);
        if ((config.buttonFlipMask & mask) != 0 &&
            policy_exploration_sample(config.seed, tick, std::uint64_t{bit} + 4) % Scale <
                config.buttonFlipProbabilityMillionths)
            pad.buttons ^= mask;
    }
}

struct SuffixBatchDefinition {
    std::size_t sourceFrame = 0;
    std::string sourceBoundaryFingerprint;
    SuffixDemonstrationMode demonstrationMode =
        SuffixDemonstrationMode::BehaviorCloningWarmStart;
    SuffixPolicyActionAuthority policyActionAuthority =
        SuffixPolicyActionAuthority::EpisodePolicy;
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
