#include "dusk/automation/frozen_inference.hpp"

#include <algorithm>
#include <bit>
#include <cmath>
#include <limits>
#include <utility>

namespace dusk::automation {
namespace {

class Reader {
public:
    explicit Reader(const std::span<const std::uint8_t> bytes) : mBytes(bytes) {}

    bool take(const std::size_t count, std::span<const std::uint8_t>& output) {
        if (count > mBytes.size() - mOffset) return false;
        output = mBytes.subspan(mOffset, count);
        mOffset += count;
        return true;
    }

    bool readU8(std::uint8_t& output) {
        std::span<const std::uint8_t> bytes;
        if (!take(1, bytes)) return false;
        output = bytes[0];
        return true;
    }

    bool readU16(std::uint16_t& output) {
        std::span<const std::uint8_t> bytes;
        if (!take(2, bytes)) return false;
        output = static_cast<std::uint16_t>(bytes[0]) |
                 static_cast<std::uint16_t>(bytes[1]) << 8;
        return true;
    }

    bool readU32(std::uint32_t& output) {
        std::span<const std::uint8_t> bytes;
        if (!take(4, bytes)) return false;
        output = static_cast<std::uint32_t>(bytes[0]) |
                 static_cast<std::uint32_t>(bytes[1]) << 8 |
                 static_cast<std::uint32_t>(bytes[2]) << 16 |
                 static_cast<std::uint32_t>(bytes[3]) << 24;
        return true;
    }

    bool readFloat(float& output) {
        std::uint32_t bits = 0;
        if (!readU32(bits)) return false;
        output = std::bit_cast<float>(bits);
        return std::isfinite(output);
    }

    template <std::size_t Size>
    bool readArray(std::array<std::uint8_t, Size>& output) {
        std::span<const std::uint8_t> bytes;
        if (!take(Size, bytes)) return false;
        std::ranges::copy(bytes, output.begin());
        return true;
    }

    [[nodiscard]] bool empty() const { return mOffset == mBytes.size(); }

private:
    std::span<const std::uint8_t> mBytes;
    std::size_t mOffset = 0;
};

bool checkedProduct(const std::size_t left, const std::size_t right, std::size_t& output) {
    if (left != 0 && right > std::numeric_limits<std::size_t>::max() / left) return false;
    output = left * right;
    return true;
}

bool checkedAdd(const std::size_t left, const std::size_t right, std::size_t& output) {
    if (right > std::numeric_limits<std::size_t>::max() - left) return false;
    output = left + right;
    return true;
}

bool zeroDigest(const std::array<std::uint8_t, 32>& digest) {
    return std::ranges::all_of(digest, [](const std::uint8_t value) { return value == 0; });
}

}  // namespace

bool FrozenInferenceModel::decode(
    const std::span<const std::uint8_t> bytes, std::string& error) {
    error.clear();
    Reader reader(bytes);
    std::span<const std::uint8_t> magic;
    std::uint16_t version = 0;
    std::uint16_t reserved = 0;
    std::array<std::uint8_t, 32> featureSchema{};
    std::array<std::uint8_t, 32> actionSchema{};
    std::array<std::uint8_t, 32> objective{};
    std::uint32_t inputWidthValue = 0;
    std::uint32_t actionCountValue = 0;
    std::uint32_t layerCountValue = 0;
    std::uint32_t declaredParameterCountValue = 0;
    if (!reader.take(kFrozenInferenceMagic.size(), magic) ||
        !std::ranges::equal(magic, kFrozenInferenceMagic) || !reader.readU16(version) ||
        !reader.readU16(reserved) || version != kFrozenInferenceVersion || reserved != 0 ||
        !reader.readArray(featureSchema) || !reader.readArray(actionSchema) ||
        !reader.readArray(objective) || !reader.readU32(inputWidthValue) ||
        !reader.readU32(actionCountValue) || !reader.readU32(layerCountValue) ||
        !reader.readU32(declaredParameterCountValue))
    {
        error = "frozen model header or version is invalid";
        return false;
    }
    const std::size_t inputWidth = inputWidthValue;
    const std::size_t actionCount = actionCountValue;
    const std::size_t layerCount = layerCountValue;
    const std::size_t declaredParameterCount = declaredParameterCountValue;
    if (zeroDigest(featureSchema) || zeroDigest(actionSchema) || zeroDigest(objective) ||
        inputWidth == 0 || inputWidth > kFrozenInferenceMaximumTensorWidth || actionCount == 0 ||
        actionCount > kFrozenInferenceMaximumTensorWidth || layerCount == 0 ||
        layerCount > kFrozenInferenceMaximumLayers ||
        declaredParameterCount > kFrozenInferenceMaximumParameters)
    {
        error = "frozen model identity or dimensions exceed format bounds";
        return false;
    }

    std::vector<std::uint32_t> actions;
    actions.reserve(actionCount);
    for (std::size_t index = 0; index < actionCount; ++index) {
        std::uint32_t action = 0;
        if (!reader.readU32(action) || (!actions.empty() && actions.back() >= action)) {
            error = "frozen model action IDs are truncated or noncanonical";
            return false;
        }
        actions.push_back(action);
    }

    std::vector<FrozenInferenceLayer> layers;
    layers.reserve(layerCount);
    std::size_t layerInputWidth = inputWidth;
    std::size_t decodedParameterCount = 0;
    std::size_t maximumWidth = inputWidth;
    for (std::size_t layerIndex = 0; layerIndex < layerCount; ++layerIndex) {
        std::uint8_t activationValue = 0;
        std::span<const std::uint8_t> layerReserved;
        std::uint32_t outputWidthValue = 0;
        if (!reader.readU8(activationValue) || !reader.take(3, layerReserved) ||
            !std::ranges::all_of(layerReserved, [](const std::uint8_t value) {
                return value == 0;
            }) ||
            !reader.readU32(outputWidthValue))
        {
            error = "frozen layer header is truncated or noncanonical";
            return false;
        }
        FrozenInferenceActivation activation;
        if (activationValue == static_cast<std::uint8_t>(FrozenInferenceActivation::Linear)) {
            activation = FrozenInferenceActivation::Linear;
        } else if (activationValue == static_cast<std::uint8_t>(FrozenInferenceActivation::Relu)) {
            activation = FrozenInferenceActivation::Relu;
        } else {
            error = "frozen model activation is unsupported";
            return false;
        }
        const std::size_t outputWidth = outputWidthValue;
        std::size_t weightCount = 0;
        std::size_t layerParameterCount = 0;
        std::size_t nextParameterCount = 0;
        if (outputWidth == 0 || outputWidth > kFrozenInferenceMaximumTensorWidth ||
            !checkedProduct(layerInputWidth, outputWidth, weightCount) ||
            !checkedAdd(weightCount, outputWidth, layerParameterCount) ||
            !checkedAdd(decodedParameterCount, layerParameterCount, nextParameterCount) ||
            nextParameterCount > declaredParameterCount ||
            nextParameterCount > kFrozenInferenceMaximumParameters)
        {
            error = "frozen layer parameters exceed the declared bounded total";
            return false;
        }
        FrozenInferenceLayer layer{
            .inputWidth = layerInputWidth,
            .outputWidth = outputWidth,
            .activation = activation,
        };
        layer.weights.reserve(weightCount);
        layer.biases.reserve(outputWidth);
        for (std::size_t index = 0; index < weightCount; ++index) {
            float value = 0.0F;
            if (!reader.readFloat(value)) {
                error = "frozen layer weights are truncated or non-finite";
                return false;
            }
            layer.weights.push_back(value);
        }
        for (std::size_t index = 0; index < outputWidth; ++index) {
            float value = 0.0F;
            if (!reader.readFloat(value)) {
                error = "frozen layer biases are truncated or non-finite";
                return false;
            }
            layer.biases.push_back(value);
        }
        decodedParameterCount = nextParameterCount;
        layerInputWidth = outputWidth;
        maximumWidth = std::max(maximumWidth, outputWidth);
        layers.push_back(std::move(layer));
    }
    if (!reader.empty() || decodedParameterCount != declaredParameterCount ||
        layers.back().outputWidth != actions.size() ||
        layers.back().activation != FrozenInferenceActivation::Linear)
    {
        error = "frozen model topology, parameter count, or trailing data is invalid";
        return false;
    }

    mFeatureSchemaSha256 = featureSchema;
    mActionSchemaSha256 = actionSchema;
    mObjectiveSha256 = objective;
    mInputWidth = inputWidth;
    mParameterCount = decodedParameterCount;
    mActions = std::move(actions);
    mLayers = std::move(layers);
    mScratchA.assign(maximumWidth, 0.0F);
    mScratchB.assign(maximumWidth, 0.0F);
    return true;
}

bool FrozenInferenceModel::infer(
    const std::span<const float> input, const std::span<float> output, std::string& error) {
    error.clear();
    if (!loaded() || input.size() != mInputWidth || output.size() != mActions.size() ||
        std::ranges::any_of(input, [](const float value) { return !std::isfinite(value); }))
    {
        error = "frozen inference row is invalid";
        return false;
    }
    std::ranges::copy(input, mScratchA.begin());
    std::span<const float> current(mScratchA.data(), input.size());
    bool destinationIsB = true;
    for (const FrozenInferenceLayer& layer : mLayers) {
        std::vector<float>& destination = destinationIsB ? mScratchB : mScratchA;
        for (std::size_t row = 0; row < layer.outputWidth; ++row) {
            float value = layer.biases[row];
            const std::span weights(
                layer.weights.data() + row * layer.inputWidth, layer.inputWidth);
            for (std::size_t column = 0; column < layer.inputWidth; ++column)
                value += weights[column] * current[column];
            if (layer.activation == FrozenInferenceActivation::Relu && !(value > 0.0F))
                value = 0.0F;
            if (!std::isfinite(value)) {
                error = "frozen inference output became non-finite";
                return false;
            }
            destination[row] = value;
        }
        current = std::span<const float>(destination.data(), layer.outputWidth);
        destinationIsB = !destinationIsB;
    }
    std::ranges::copy(current, output.begin());
    return true;
}

}  // namespace dusk::automation
