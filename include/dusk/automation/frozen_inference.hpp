#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <string>
#include <vector>

namespace dusk::automation {

inline constexpr std::array<std::uint8_t, 8> kFrozenInferenceMagic{
    'D', 'S', 'K', 'F', 'R', 'Z', 'N', 0,
};
inline constexpr std::uint16_t kFrozenInferenceVersion = 1;
inline constexpr std::size_t kFrozenInferenceMaximumLayers = 16;
inline constexpr std::size_t kFrozenInferenceMaximumTensorWidth = 4096;
inline constexpr std::size_t kFrozenInferenceMaximumParameters = 16'000'000;

enum class FrozenInferenceActivation : std::uint8_t {
    Linear = 0,
    Relu = 1,
};

struct FrozenInferenceLayer {
    std::size_t inputWidth = 0;
    std::size_t outputWidth = 0;
    FrozenInferenceActivation activation = FrozenInferenceActivation::Linear;
    // Row-major `[output][input]`, matching the Rust artifact exactly.
    std::vector<float> weights;
    std::vector<float> biases;
};

/**
 * Bounded, allocation-free-after-load dense inference for
 * `dusklight-frozen-inference/v1`.
 *
 * The model owns no game pointer and can only transform a caller-provided
 * finite feature row into one finite output row. `infer` is synchronous and
 * intentionally single-threaded; one worker owns one instance.
 */
class FrozenInferenceModel {
public:
    [[nodiscard]] bool decode(std::span<const std::uint8_t> bytes, std::string& error);
    [[nodiscard]] bool infer(
        std::span<const float> input, std::span<float> output, std::string& error);

    [[nodiscard]] const std::array<std::uint8_t, 32>& featureSchemaSha256() const {
        return mFeatureSchemaSha256;
    }
    [[nodiscard]] const std::array<std::uint8_t, 32>& actionSchemaSha256() const {
        return mActionSchemaSha256;
    }
    [[nodiscard]] const std::array<std::uint8_t, 32>& objectiveSha256() const {
        return mObjectiveSha256;
    }
    [[nodiscard]] std::size_t inputWidth() const { return mInputWidth; }
    [[nodiscard]] std::span<const std::uint32_t> actions() const { return mActions; }
    [[nodiscard]] std::span<const FrozenInferenceLayer> layers() const { return mLayers; }
    [[nodiscard]] std::size_t parameterCount() const { return mParameterCount; }
    [[nodiscard]] bool loaded() const { return !mLayers.empty(); }

private:
    std::array<std::uint8_t, 32> mFeatureSchemaSha256{};
    std::array<std::uint8_t, 32> mActionSchemaSha256{};
    std::array<std::uint8_t, 32> mObjectiveSha256{};
    std::size_t mInputWidth = 0;
    std::size_t mParameterCount = 0;
    std::vector<std::uint32_t> mActions;
    std::vector<FrozenInferenceLayer> mLayers;
    std::vector<float> mScratchA;
    std::vector<float> mScratchB;
};

}  // namespace dusk::automation
