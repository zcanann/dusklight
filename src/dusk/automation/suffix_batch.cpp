#include "dusk/automation/suffix_batch.hpp"

#include <array>
#include <cmath>
#include <cstdint>
#include <limits>
#include <span>
#include <string>
#include <type_traits>
#include <unordered_set>
#include <utility>

#include <nlohmann/json.hpp>
#include <xxhash.h>

namespace dusk::automation {
namespace {

using json = nlohmann::json;

constexpr std::array<std::uint8_t, 8> CompactSuffixBatchMagic{
    'D', 'S', 'K', 'S', 'B', 'X', 1, 0};
constexpr std::size_t CompactSuffixBatchHeaderBytes = 28;
constexpr std::uint8_t CompactVerifyStateHashes = 1 << 0;
constexpr std::uint8_t CompactCheckpointCache = 1 << 1;
constexpr std::uint8_t CompactSourceIdentity = 1 << 2;
constexpr std::uint8_t CompactRetainCandidateCheckpoints = 1 << 3;
constexpr std::uint8_t CompactRetainLiveEndpoint = 1 << 4;
constexpr std::uint8_t CompactVariableCandidateTicks = 1 << 5;
constexpr std::uint8_t CompactRetainCandidateIndex = 1 << 6;
constexpr std::uint8_t CompactKnownFlags = CompactVerifyStateHashes |
                                           CompactCheckpointCache |
                                           CompactSourceIdentity |
                                           CompactRetainCandidateCheckpoints |
                                           CompactRetainLiveEndpoint |
                                           CompactVariableCandidateTicks |
                                           CompactRetainCandidateIndex;
constexpr std::uint8_t CompactRecordedReplayWindow = 2;

class CompactReader {
public:
    explicit CompactReader(const std::span<const std::uint8_t> bytes) : mBytes(bytes) {}

    template <typename T>
    bool readLittle(T& output) {
        static_assert(std::is_unsigned_v<T>);
        if (remaining() < sizeof(T)) return false;
        output = 0;
        for (std::size_t index = 0; index < sizeof(T); ++index)
            output |= static_cast<T>(mBytes[mOffset + index]) << (index * 8);
        mOffset += sizeof(T);
        return true;
    }

    bool readByte(std::uint8_t& output) {
        if (remaining() == 0) return false;
        output = mBytes[mOffset++];
        return true;
    }

    bool readBytes(const std::size_t count, std::span<const std::uint8_t>& output) {
        if (count > remaining()) return false;
        output = mBytes.subspan(mOffset, count);
        mOffset += count;
        return true;
    }

    [[nodiscard]] std::size_t remaining() const { return mBytes.size() - mOffset; }

private:
    std::span<const std::uint8_t> mBytes;
    std::size_t mOffset = 0;
};

std::string compact_hex(const std::span<const std::uint8_t> bytes) {
    constexpr char Hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(bytes.size() * 2);
    for (const std::uint8_t byte : bytes) {
        output.push_back(Hex[byte >> 4]);
        output.push_back(Hex[byte & 0xf]);
    }
    return output;
}

bool compact_suffix_batch_magic(const std::string_view source) {
    return source.size() >= CompactSuffixBatchMagic.size() &&
           std::equal(CompactSuffixBatchMagic.begin(), CompactSuffixBatchMagic.end(),
               reinterpret_cast<const std::uint8_t*>(source.data()));
}

bool parse_compact_suffix_batch(
    const std::string_view source, SuffixBatchDefinition& output, std::string& error) {
    output = {};
    error.clear();
    if (source.size() < CompactSuffixBatchHeaderBytes ||
        source.size() > SuffixBatchMaximumBytes) {
        error = "compact suffix batch is truncated or exceeds 64 MiB";
        return false;
    }
    const auto bytes = std::span{
        reinterpret_cast<const std::uint8_t*>(source.data()), source.size()};
    CompactReader header(bytes.subspan(CompactSuffixBatchMagic.size()));
    std::uint32_t payloadSize = 0;
    std::span<const std::uint8_t> expectedDigest;
    if (!header.readLittle(payloadSize) || !header.readBytes(16, expectedDigest) ||
        payloadSize != bytes.size() - CompactSuffixBatchHeaderBytes) {
        error = "compact suffix batch payload length is invalid";
        return false;
    }
    const auto payload = bytes.subspan(CompactSuffixBatchHeaderBytes);
    const XXH128_hash_t hash = XXH3_128bits(payload.data(), payload.size());
    XXH128_canonical_t canonical{};
    XXH128_canonicalFromHash(&canonical, hash);
    if (!std::equal(expectedDigest.begin(), expectedDigest.end(), canonical.digest)) {
        error = "compact suffix batch payload digest differs";
        return false;
    }

    CompactReader reader(payload);
    std::uint8_t flags = 0;
    std::uint64_t sourceFrame = 0;
    std::span<const std::uint8_t> sourceBoundary;
    std::uint8_t validationKind = 0;
    std::uint16_t validationTicks = 0;
    std::uint16_t maximumTicks = 0;
    std::uint32_t capacityBytes = 0;
    std::uint8_t capacityEntries = 0;
    std::uint32_t sourceRouteTicks = 0;
    if (!reader.readByte(flags) || (flags & ~CompactKnownFlags) != 0 ||
        (flags & CompactCheckpointCache) == 0 ||
        !reader.readLittle(sourceFrame) || !reader.readBytes(16, sourceBoundary) ||
        !reader.readByte(validationKind) ||
        validationKind != CompactRecordedReplayWindow ||
        !reader.readLittle(validationTicks) || validationTicks == 0 ||
        validationTicks > SuffixBatchMaximumValidationTicks ||
        !reader.readLittle(maximumTicks) || maximumTicks == 0 ||
        maximumTicks > SuffixBatchMaximumTicks ||
        !reader.readLittle(capacityBytes) || capacityBytes == 0 ||
        capacityBytes > SuffixBatchMaximumCheckpointCacheBytes ||
        !reader.readByte(capacityEntries) || capacityEntries == 0 ||
        capacityEntries > SuffixBatchMaximumCheckpointCacheEntries ||
        !reader.readLittle(sourceRouteTicks) ||
        sourceRouteTicks > SuffixBatchMaximumExpandedTicks ||
        sourceFrame > std::numeric_limits<std::size_t>::max()) {
        error = "compact suffix batch contract is invalid";
        return false;
    }

    SuffixBatchDefinition parsed;
    parsed.sourceFrame = static_cast<std::size_t>(sourceFrame);
    parsed.sourceBoundaryFingerprint = compact_hex(sourceBoundary);
    parsed.checkpointValidation = SuffixCheckpointValidation::RecordedReplayWindow;
    parsed.validationTicks = validationTicks;
    parsed.maximumTicks = maximumTicks;
    parsed.verifyStateHashes = (flags & CompactVerifyStateHashes) != 0;
    SuffixCheckpointCachePolicy cache;
    cache.capacityBytes = capacityBytes;
    cache.capacityEntries = capacityEntries;
    cache.sourceRouteTicks = sourceRouteTicks;
    cache.retainCandidateCheckpoints =
        (flags & CompactRetainCandidateCheckpoints) != 0;
    cache.retainLiveEndpoint = (flags & CompactRetainLiveEndpoint) != 0;
    if (static_cast<unsigned>(cache.retainCandidateCheckpoints) +
            static_cast<unsigned>(cache.retainLiveEndpoint) +
            static_cast<unsigned>((flags & CompactRetainCandidateIndex) != 0) >
        1) {
        error = "compact suffix batch checkpoint retention conflicts";
        return false;
    }
    if ((flags & CompactSourceIdentity) != 0) {
        std::span<const std::uint8_t> sourceIdentity;
        if (!reader.readBytes(16, sourceIdentity)) {
            error = "compact suffix batch source identity is truncated";
            return false;
        }
        cache.sourceIdentity = compact_hex(sourceIdentity);
    }
    std::uint16_t candidateCount = 0;
    if (!reader.readLittle(candidateCount) || candidateCount == 0 ||
        candidateCount > SuffixBatchMaximumCandidates ||
        candidateCount > SuffixBatchMaximumExpandedTicks / parsed.maximumTicks) {
        error = "compact suffix batch candidate count is invalid";
        return false;
    }
    if ((flags & CompactRetainCandidateIndex) != 0) {
        std::uint16_t retainedIndex = 0;
        if (!reader.readLittle(retainedIndex) || retainedIndex >= candidateCount) {
            error = "compact suffix retained candidate index is invalid";
            return false;
        }
        cache.retainCandidateIndex = retainedIndex;
    }
    parsed.checkpointCache = std::move(cache);
    parsed.candidates.reserve(candidateCount);
    std::unordered_set<std::string> ids;
    ids.reserve(candidateCount);
    for (std::size_t candidateIndex = 0; candidateIndex < candidateCount; ++candidateIndex) {
        std::uint8_t idLength = 0;
        std::span<const std::uint8_t> idBytes;
        std::uint16_t candidateMaximumTicks = maximumTicks;
        std::uint16_t controllerLength = 0;
        std::span<const std::uint8_t> controllerBytes;
        std::uint16_t runCount = 0;
        if (!reader.readByte(idLength) || idLength == 0 || idLength > 128 ||
            !reader.readBytes(idLength, idBytes) ||
            !std::ranges::all_of(idBytes, [](const std::uint8_t byte) {
                return byte >= 0x21 && byte <= 0x7e;
            }) ||
            ((flags & CompactVariableCandidateTicks) != 0 &&
                (!reader.readLittle(candidateMaximumTicks) || candidateMaximumTicks == 0 ||
                    candidateMaximumTicks > maximumTicks)) ||
            !reader.readLittle(controllerLength) ||
            !reader.readBytes(controllerLength, controllerBytes) ||
            !reader.readLittle(runCount) || runCount > candidateMaximumTicks) {
            error = "compact suffix candidate header is invalid";
            return false;
        }
        SuffixBatchCandidate candidate;
        candidate.id.assign(
            reinterpret_cast<const char*>(idBytes.data()), idBytes.size());
        candidate.maximumTicks = candidateMaximumTicks;
        if (!ids.insert(candidate.id).second) {
            error = "compact suffix candidate has a duplicate id";
            return false;
        }
        candidate.pads.reserve(candidate.maximumTicks);
        for (std::size_t runIndex = 0; runIndex < runCount; ++runIndex) {
            std::uint16_t frames = 0;
            std::uint16_t buttons = 0;
            std::uint8_t stickX = 0;
            std::uint8_t stickY = 0;
            std::uint8_t substickX = 0;
            std::uint8_t substickY = 0;
            std::uint8_t triggerLeft = 0;
            std::uint8_t triggerRight = 0;
            std::uint8_t analogA = 0;
            std::uint8_t analogB = 0;
            std::uint8_t connected = 0;
            std::uint8_t padError = 0;
            if (!reader.readLittle(frames) || frames == 0 ||
                frames > candidate.maximumTicks - candidate.pads.size() ||
                !reader.readLittle(buttons) || !reader.readByte(stickX) ||
                !reader.readByte(stickY) || !reader.readByte(substickX) ||
                !reader.readByte(substickY) || !reader.readByte(triggerLeft) ||
                !reader.readByte(triggerRight) || !reader.readByte(analogA) ||
                !reader.readByte(analogB) || !reader.readByte(connected) ||
                connected > 1 || !reader.readByte(padError)) {
                error = "compact suffix candidate PAD run is invalid";
                return false;
            }
            RawPadState pad;
            pad.buttons = buttons;
            pad.stickX = static_cast<std::int8_t>(stickX);
            pad.stickY = static_cast<std::int8_t>(stickY);
            pad.substickX = static_cast<std::int8_t>(substickX);
            pad.substickY = static_cast<std::int8_t>(substickY);
            pad.triggerLeft = triggerLeft;
            pad.triggerRight = triggerRight;
            pad.analogA = analogA;
            pad.analogB = analogB;
            pad.flags = connected != 0 ? RawPadFlags::Connected : RawPadFlags::None;
            pad.error = static_cast<std::int8_t>(padError);
            candidate.pads.insert(candidate.pads.end(), frames, pad);
        }
        if (!controllerBytes.empty()) {
            const std::size_t maximumControllerBytes =
                kInputControllerHeaderSize +
                kInputControllerMaximumLayers * kInputControllerRecordSize;
            if (controllerBytes.size() > maximumControllerBytes ||
                decode_input_controller(controllerBytes, candidate.controller) !=
                    InputControllerError::None) {
                error = "compact suffix candidate controller program is invalid";
                return false;
            }
            candidate.controllerProgram = true;
            candidate.controllerStartTick = candidate.pads.size();
            if (candidate.controller.duration() !=
                candidate.maximumTicks - candidate.controllerStartTick) {
                error = "compact suffix controller duration differs from maximum_ticks";
                return false;
            }
        } else if (candidate.pads.size() != candidate.maximumTicks) {
            error = "compact suffix candidate PAD runs differ from maximum_ticks";
            return false;
        }
        parsed.candidates.push_back(std::move(candidate));
    }
    if (reader.remaining() != 0) {
        error = "compact suffix batch has trailing payload bytes";
        return false;
    }
    if (parsed.checkpointCache->retainLiveEndpoint && parsed.candidates.size() != 1) {
        error = "compact live endpoint retention requires exactly one candidate";
        return false;
    }
    output = std::move(parsed);
    return true;
}

template <std::size_t Size>
bool has_exact_keys(const json& value, const std::array<std::string_view, Size>& allowed) {
    if (!value.is_object() || value.size() != allowed.size()) return false;
    for (const auto& [key, ignored] : value.items()) {
        (void)ignored;
        bool found = false;
        for (const std::string_view candidate : allowed) {
            if (key == candidate) {
                found = true;
                break;
            }
        }
        if (!found) return false;
    }
    return true;
}

template <typename T>
bool read_integer(const json& value, const T minimum, const T maximum, T& output) {
    if (!value.is_number_integer()) return false;
    if constexpr (std::is_signed_v<T>) {
        if (value.is_number_unsigned()) {
            const std::uint64_t parsed = value.get<std::uint64_t>();
            if (parsed > static_cast<std::uint64_t>(maximum)) return false;
            output = static_cast<T>(parsed);
            return true;
        }
        const std::int64_t parsed = value.get<std::int64_t>();
        if (parsed < static_cast<std::int64_t>(minimum) ||
            parsed > static_cast<std::int64_t>(maximum))
            return false;
        output = static_cast<T>(parsed);
    } else {
        if (value.is_number_unsigned()) {
            const std::uint64_t parsed = value.get<std::uint64_t>();
            if (parsed < static_cast<std::uint64_t>(minimum) ||
                parsed > static_cast<std::uint64_t>(maximum))
                return false;
            output = static_cast<T>(parsed);
        } else {
            const std::int64_t parsed = value.get<std::int64_t>();
            if (parsed < 0 || static_cast<std::uint64_t>(parsed) < minimum ||
                static_cast<std::uint64_t>(parsed) > maximum)
                return false;
            output = static_cast<T>(parsed);
        }
    }
    return true;
}

bool parse_pad(const json& value, RawPadState& output) {
    constexpr std::array Keys{
        std::string_view{"buttons"},
        std::string_view{"stick_x"},
        std::string_view{"stick_y"},
        std::string_view{"substick_x"},
        std::string_view{"substick_y"},
        std::string_view{"trigger_left"},
        std::string_view{"trigger_right"},
        std::string_view{"analog_a"},
        std::string_view{"analog_b"},
        std::string_view{"connected"},
        std::string_view{"error"},
    };
    if (!has_exact_keys(value, Keys) || !value["connected"].is_boolean()) return false;

    RawPadState parsed;
    if (!read_integer(value["buttons"], std::uint16_t{0},
            std::numeric_limits<std::uint16_t>::max(), parsed.buttons) ||
        !read_integer(value["stick_x"], std::numeric_limits<std::int8_t>::min(),
            std::numeric_limits<std::int8_t>::max(), parsed.stickX) ||
        !read_integer(value["stick_y"], std::numeric_limits<std::int8_t>::min(),
            std::numeric_limits<std::int8_t>::max(), parsed.stickY) ||
        !read_integer(value["substick_x"], std::numeric_limits<std::int8_t>::min(),
            std::numeric_limits<std::int8_t>::max(), parsed.substickX) ||
        !read_integer(value["substick_y"], std::numeric_limits<std::int8_t>::min(),
            std::numeric_limits<std::int8_t>::max(), parsed.substickY) ||
        !read_integer(value["trigger_left"], std::uint8_t{0},
            std::numeric_limits<std::uint8_t>::max(), parsed.triggerLeft) ||
        !read_integer(value["trigger_right"], std::uint8_t{0},
            std::numeric_limits<std::uint8_t>::max(), parsed.triggerRight) ||
        !read_integer(value["analog_a"], std::uint8_t{0},
            std::numeric_limits<std::uint8_t>::max(), parsed.analogA) ||
        !read_integer(value["analog_b"], std::uint8_t{0},
            std::numeric_limits<std::uint8_t>::max(), parsed.analogB) ||
        !read_integer(value["error"], std::numeric_limits<std::int8_t>::min(),
            std::numeric_limits<std::int8_t>::max(), parsed.error))
        return false;

    parsed.flags = value["connected"].get<bool>() ? RawPadFlags::Connected : RawPadFlags::None;
    output = parsed;
    return true;
}

bool parse_controller_program_hex(
    const json& value, InputControllerProgram& output, std::string& error) {
    if (!value.is_string()) {
        error = "controller_program_hex must be a string";
        return false;
    }
    const auto& encoded = value.get_ref<const std::string&>();
    const std::size_t maximumBytes =
        kInputControllerHeaderSize +
        kInputControllerMaximumLayers * kInputControllerRecordSize;
    if (encoded.empty() || (encoded.size() & 1) != 0 ||
        encoded.size() > maximumBytes * 2)
    {
        error = "controller_program_hex has an invalid encoded length";
        return false;
    }
    const auto nibble = [](const unsigned char byte) -> std::optional<std::uint8_t> {
        if (byte >= '0' && byte <= '9') return byte - '0';
        if (byte >= 'a' && byte <= 'f') return byte - 'a' + 10;
        return std::nullopt;
    };
    std::vector<std::uint8_t> bytes(encoded.size() / 2);
    for (std::size_t index = 0; index < bytes.size(); ++index) {
        const auto high = nibble(static_cast<unsigned char>(encoded[index * 2]));
        const auto low = nibble(static_cast<unsigned char>(encoded[index * 2 + 1]));
        if (!high.has_value() || !low.has_value()) {
            error = "controller_program_hex must use canonical lowercase hexadecimal";
            return false;
        }
        bytes[index] = static_cast<std::uint8_t>((*high << 4) | *low);
    }
    const InputControllerError controllerError =
        decode_input_controller(bytes, output);
    if (controllerError != InputControllerError::None) {
        error = std::string{"controller_program_hex is invalid: "} +
                input_controller_error_message(controllerError);
        return false;
    }
    return true;
}

bool valid_boundary_fingerprint(const json& value) {
    if (!value.is_string()) return false;
    const auto& fingerprint = value.get_ref<const std::string&>();
    if (fingerprint.size() != 32) return false;
    for (const unsigned char byte : fingerprint) {
        if (!((byte >= '0' && byte <= '9') || (byte >= 'a' && byte <= 'f'))) return false;
    }
    return true;
}

bool parse_checkpoint_validation(const json& value, SuffixBatchDefinition& output) {
    constexpr std::array Keys{std::string_view{"kind"}, std::string_view{"ticks"}};
    if (!has_exact_keys(value, Keys) || !value["kind"].is_string())
        return false;
    const auto& kind = value["kind"].get_ref<const std::string&>();
    if (kind != "recorded_replay_window")
        return false;
    std::size_t ticks = 0;
    if (!read_integer(value["ticks"], std::size_t{1}, SuffixBatchMaximumValidationTicks, ticks))
        return false;
    output.checkpointValidation = SuffixCheckpointValidation::RecordedReplayWindow;
    output.validationTicks = ticks;
    return true;
}

bool parse_policy_head(const json& value, FactorizedPadPolicyHeadConfig& output) {
    constexpr std::array Keys{std::string_view{"schema"},
        std::string_view{"maximum_duration_ticks"},
        std::string_view{"button_logit_threshold"}};
    if (!has_exact_keys(value, Keys) || !value["schema"].is_string() ||
        value["schema"].get_ref<const std::string&>() != kFactorizedPadPolicyHeadSchema ||
        !value["button_logit_threshold"].is_number())
        return false;
    FactorizedPadPolicyHeadConfig parsed;
    if (!read_integer(value["maximum_duration_ticks"], std::uint32_t{1},
            kMaximumFactorizedPadDuration, parsed.maximumDurationTicks))
        return false;
    const double threshold = value["button_logit_threshold"].get<double>();
    if (!std::isfinite(threshold) || threshold < -std::numeric_limits<float>::max() ||
        threshold > std::numeric_limits<float>::max())
        return false;
    parsed.buttonLogitThreshold = static_cast<float>(threshold);
    output = parsed;
    return true;
}

bool parse_rollout_exploration(
    const json& value, SuffixBatchFrozenPolicy::RolloutExploration& output) {
    constexpr std::array Keys{std::string_view{"schema"}, std::string_view{"seed"},
        std::string_view{"stick_axis_delta_probability_millionths"},
        std::string_view{"maximum_stick_axis_delta"},
        std::string_view{"button_flip_probability_millionths"},
        std::string_view{"button_flip_mask"}};
    if (!has_exact_keys(value, Keys) || !value["schema"].is_string() ||
        value["schema"].get_ref<const std::string&>() != PolicyRolloutExplorationSchema)
        return false;
    SuffixBatchFrozenPolicy::RolloutExploration parsed;
    if (!read_integer(value["seed"], std::uint64_t{1}, std::numeric_limits<std::uint64_t>::max(),
            parsed.seed) ||
        !read_integer(value["stick_axis_delta_probability_millionths"], std::uint32_t{0},
            std::uint32_t{1'000'000}, parsed.stickAxisDeltaProbabilityMillionths) ||
        !read_integer(value["maximum_stick_axis_delta"], std::uint8_t{1}, std::uint8_t{64},
            parsed.maximumStickAxisDelta) ||
        !read_integer(value["button_flip_probability_millionths"], std::uint32_t{0},
            std::uint32_t{1'000'000}, parsed.buttonFlipProbabilityMillionths) ||
        !read_integer(value["button_flip_mask"], std::uint16_t{1},
            std::numeric_limits<std::uint16_t>::max(), parsed.buttonFlipMask))
        return false;
    output = parsed;
    return true;
}

bool parse_frozen_policy(
    const json& value, const bool requiresExploration, SuffixBatchFrozenPolicy& output) {
    constexpr std::array LegacyKeys{std::string_view{"schema"}, std::string_view{"model_path"},
        std::string_view{"model_xxh3_128"}, std::string_view{"policy_head"}};
    constexpr std::array ExplorationKeys{std::string_view{"schema"}, std::string_view{"model_path"},
        std::string_view{"model_xxh3_128"}, std::string_view{"policy_head"},
        std::string_view{"rollout_exploration"}};
    if (!(requiresExploration ? has_exact_keys(value, ExplorationKeys) :
                               has_exact_keys(value, LegacyKeys)) ||
        !value["schema"].is_string() ||
        value["schema"].get_ref<const std::string&>() !=
            (requiresExploration ? FrozenPolicySchema : FrozenPolicySchemaV1) ||
        !value["model_path"].is_string() || !valid_boundary_fingerprint(value["model_xxh3_128"]))
        return false;
    SuffixBatchFrozenPolicy parsed;
    parsed.modelPath = value["model_path"].get<std::string>();
    if (parsed.modelPath.empty() || parsed.modelPath.size() > 4096 ||
        parsed.modelPath.find('\0') != std::string::npos ||
        !parse_policy_head(value["policy_head"], parsed.policyHead) ||
        parsed.policyHead.maximumDurationTicks != 1 || parsed.policyHead.buttonLogitThreshold != 0.0F)
        return false;
    parsed.modelXxh3_128 = value["model_xxh3_128"].get<std::string>();
    if (requiresExploration) {
        SuffixBatchFrozenPolicy::RolloutExploration exploration;
        if (!parse_rollout_exploration(value["rollout_exploration"], exploration)) return false;
        parsed.rolloutExploration = exploration;
    }
    output = std::move(parsed);
    return true;
}

bool parse_policy_output(const json& value,
    std::array<float, kFactorizedPadPolicyHeadWidth>& output) {
    if (!value.is_array() || value.size() != output.size()) return false;
    for (std::size_t index = 0; index < output.size(); ++index) {
        if (!value[index].is_number()) return false;
        const double parsed = value[index].get<double>();
        if (!std::isfinite(parsed) || parsed < -std::numeric_limits<float>::max() ||
            parsed > std::numeric_limits<float>::max())
            return false;
        output[index] = static_cast<float>(parsed);
    }
    return true;
}

bool parse_candidate(const json& value, const std::size_t maximumTicks,
    const bool allowFactorizedPolicy, const bool allowFrozenPolicy,
    const bool allowControllerProgram, const bool allowVariableTicks,
    SuffixBatchCandidate& output, std::string& error) {
    constexpr std::array ActionKeys{
        std::string_view{"id"}, std::string_view{"actions"}};
    constexpr std::array TapeKeys{
        std::string_view{"id"}, std::string_view{"source"}};
    constexpr std::array PolicyKeys{std::string_view{"id"},
        std::string_view{"policy_head"}, std::string_view{"policy_outputs"}};
    constexpr std::array ControllerKeys{std::string_view{"id"},
        std::string_view{"actions"}, std::string_view{"controller_program_hex"}};
    constexpr std::array VariableActionKeys{std::string_view{"id"},
        std::string_view{"actions"}, std::string_view{"maximum_ticks"}};
    constexpr std::array VariableControllerKeys{std::string_view{"id"},
        std::string_view{"actions"}, std::string_view{"controller_program_hex"},
        std::string_view{"maximum_ticks"}};
    const bool variableActionCandidate =
        allowVariableTicks && has_exact_keys(value, VariableActionKeys);
    const bool variableControllerCandidate =
        allowVariableTicks && has_exact_keys(value, VariableControllerKeys);
    const bool actionCandidate = has_exact_keys(value, ActionKeys) || variableActionCandidate;
    const bool sourceCandidate = has_exact_keys(value, TapeKeys);
    const bool policyCandidate = has_exact_keys(value, PolicyKeys);
    const bool controllerCandidate =
        has_exact_keys(value, ControllerKeys) || variableControllerCandidate;
    if ((!actionCandidate && !sourceCandidate && !policyCandidate && !controllerCandidate) ||
        (policyCandidate && !allowFactorizedPolicy) || !value["id"].is_string())
    {
        error = "candidate must contain id plus actions, source, controller, or policy outputs";
        return false;
    }
    if (controllerCandidate && !allowControllerProgram) {
        error = "controller candidates require the reactive suffix-batch schema";
        return false;
    }
    const std::string id = value["id"].get<std::string>();
    if (id.empty() || id.size() > 128) {
        error = "candidate id is empty or exceeds 128 bytes";
        return false;
    }
    std::size_t candidateMaximumTicks = maximumTicks;
    if ((variableActionCandidate || variableControllerCandidate) &&
        !read_integer(value["maximum_ticks"], std::size_t{1}, maximumTicks,
            candidateMaximumTicks)) {
        error = "candidate maximum_ticks is invalid or exceeds the batch maximum";
        return false;
    }
    for (const unsigned char byte : id) {
        if (byte < 0x21 || byte > 0x7e) {
            error = "candidate id must be printable ASCII without whitespace";
            return false;
        }
    }
    if (sourceCandidate) {
        if (!value["source"].is_string()) {
            error = "candidate source must be a string";
            return false;
        }
        const auto& source = value["source"].get_ref<const std::string&>();
        if (source == "tape") {
            output = {.id = id, .maximumTicks = maximumTicks, .tapePassthrough = true};
        } else if (source == "frozen_policy" && allowFrozenPolicy) {
            output = {.id = id, .maximumTicks = maximumTicks, .frozenPolicy = true};
        } else {
            error = "candidate source must be tape or an admitted frozen_policy";
            return false;
        }
        return true;
    }
    if (policyCandidate) {
        SuffixBatchCandidate parsed;
        parsed.id = id;
        parsed.maximumTicks = maximumTicks;
        parsed.factorizedPolicy = true;
        if (!parse_policy_head(value["policy_head"], parsed.policyHead) ||
            !value["policy_outputs"].is_array() || value["policy_outputs"].empty() ||
            value["policy_outputs"].size() > maximumTicks)
        {
            error = "candidate factorized policy head or output rows are invalid";
            return false;
        }
        parsed.policyOutputs.reserve(value["policy_outputs"].size());
        parsed.policyOutputIndexByTick.reserve(maximumTicks);
        parsed.pads.reserve(maximumTicks);
        for (std::size_t index = 0; index < value["policy_outputs"].size(); ++index) {
            std::array<float, kFactorizedPadPolicyHeadWidth> row{};
            FactorizedPadPolicyDecision decision;
            std::string decodeError;
            if (!parse_policy_output(value["policy_outputs"][index], row) ||
                !decode_factorized_pad_policy(parsed.policyHead, row, decision, decodeError) ||
                decision.durationTicks > maximumTicks - parsed.pads.size())
            {
                error = "candidate policy output " + std::to_string(index) +
                        " is invalid or exceeds maximum_ticks";
                return false;
            }
            parsed.policyOutputs.push_back(row);
            parsed.pads.insert(parsed.pads.end(), decision.durationTicks, decision.pad);
            parsed.policyOutputIndexByTick.insert(parsed.policyOutputIndexByTick.end(),
                decision.durationTicks, static_cast<std::uint32_t>(index));
        }
        if (parsed.pads.size() != maximumTicks) {
            error = "candidate factorized policy outputs expand to " +
                    std::to_string(parsed.pads.size()) + " ticks instead of maximum_ticks";
            return false;
        }
        output = std::move(parsed);
        return true;
    }

    const auto& actions = value["actions"];
    if (!actions.is_array()) {
        error = "candidate actions must be an array";
        return false;
    }
    if ((!controllerCandidate && actions.empty()) || actions.size() > maximumTicks) {
        error = "candidate action count is empty or exceeds maximum_ticks";
        return false;
    }

    SuffixBatchCandidate parsed;
    parsed.id = id;
    parsed.maximumTicks = candidateMaximumTicks;
    parsed.pads.reserve(candidateMaximumTicks);
    constexpr std::array PadRunKeys{
        std::string_view{"op"}, std::string_view{"pad"}, std::string_view{"frames"}};
    for (std::size_t index = 0; index < actions.size(); ++index) {
        const json& action = actions[index];
        if (!has_exact_keys(action, PadRunKeys) || !action["op"].is_string() ||
            action["op"].get_ref<const std::string&>() != "pad_run")
        {
            error = "candidate action " + std::to_string(index) +
                    " is not an exact pad_run";
            return false;
        }
        std::size_t frames = 0;
        RawPadState pad;
        if (!read_integer(action["frames"], std::size_t{1}, candidateMaximumTicks, frames) ||
            !parse_pad(action["pad"], pad) ||
            frames > candidateMaximumTicks - parsed.pads.size())
        {
            error = "candidate action " + std::to_string(index) +
                    " has invalid pad fields or duration";
            return false;
        }
        parsed.pads.insert(parsed.pads.end(), frames, pad);
    }
    if (controllerCandidate) {
        if (!parse_controller_program_hex(
                value["controller_program_hex"], parsed.controller, error))
            return false;
        parsed.controllerProgram = true;
        parsed.controllerStartTick = parsed.pads.size();
        if (parsed.controller.duration() != candidateMaximumTicks - parsed.controllerStartTick) {
            error = "controller duration plus static prefix differs from maximum_ticks";
            return false;
        }
    } else if (parsed.pads.size() != candidateMaximumTicks) {
        error = "candidate expands to " + std::to_string(parsed.pads.size()) +
                " ticks instead of maximum_ticks";
        return false;
    }
    output = std::move(parsed);
    return true;
}

}  // namespace

bool parse_suffix_batch(
    const std::string_view source, SuffixBatchDefinition& output, std::string& error) {
    output = {};
    error.clear();
    if (source.empty() || source.size() > SuffixBatchMaximumBytes) {
        error = "suffix batch is empty or exceeds 64 MiB";
        return false;
    }
    if (compact_suffix_batch_magic(source))
        return parse_compact_suffix_batch(source, output, error);
    const json root = json::parse(source, nullptr, false);
    constexpr std::array LegacyRootKeys{
        std::string_view{"schema"},
        std::string_view{"source_frame"},
        std::string_view{"source_boundary_fingerprint"},
        std::string_view{"maximum_ticks"},
        std::string_view{"verify_state_hashes"},
        std::string_view{"candidates"},
    };
    constexpr std::array RootKeys{
        std::string_view{"schema"},
        std::string_view{"source_frame"},
        std::string_view{"source_boundary_fingerprint"},
        std::string_view{"checkpoint_validation"},
        std::string_view{"maximum_ticks"},
        std::string_view{"verify_state_hashes"},
        std::string_view{"candidates"},
    };
    constexpr std::array CachedRootKeys{
        std::string_view{"schema"},
        std::string_view{"source_frame"},
        std::string_view{"source_boundary_fingerprint"},
        std::string_view{"checkpoint_validation"},
        std::string_view{"maximum_ticks"},
        std::string_view{"verify_state_hashes"},
        std::string_view{"checkpoint_cache"},
        std::string_view{"candidates"},
    };
    constexpr std::array FrozenRootKeys{
        std::string_view{"schema"},
        std::string_view{"demonstration_mode"},
        std::string_view{"action_authority"},
        std::string_view{"source_frame"},
        std::string_view{"source_boundary_fingerprint"},
        std::string_view{"checkpoint_validation"},
        std::string_view{"maximum_ticks"},
        std::string_view{"verify_state_hashes"},
        std::string_view{"frozen_policy"},
        std::string_view{"candidates"},
    };
    if (root.is_discarded() || !root.is_object() || !root.contains("schema") ||
        !root["schema"].is_string())
    {
        error = "suffix batch root or schema is invalid";
        return false;
    }
    const auto& schema = root["schema"].get_ref<const std::string&>();
    const bool legacy = schema == LegacySuffixBatchSchema;
    const bool previous = schema == PreviousSuffixBatchSchema;
    const bool reactive = schema == ReactiveSuffixBatchSchema;
    const bool cached = schema == CachedSuffixBatchSchema ||
                        schema == VariableCachedSuffixBatchSchema;
    const bool variableCached = schema == VariableCachedSuffixBatchSchema;
    const bool factorized = schema == FactorizedSuffixBatchSchema;
    const bool legacyFrozen = schema == FrozenPolicySuffixBatchSchemaV6;
    const bool exploratoryFrozen = schema == SuffixBatchSchema;
    const bool frozen = legacyFrozen || exploratoryFrozen;
    if ((!legacy && !previous && !reactive && !cached && !factorized && !frozen) ||
        !(legacy ? has_exact_keys(root, LegacyRootKeys) :
                   frozen ? has_exact_keys(root, FrozenRootKeys) :
                   cached ? has_exact_keys(root, CachedRootKeys) :
                            has_exact_keys(root, RootKeys)) ||
        !valid_boundary_fingerprint(root["source_boundary_fingerprint"]) ||
        !root["verify_state_hashes"].is_boolean() || !root["candidates"].is_array())
    {
        error = "suffix batch root or schema is invalid";
        return false;
    }

    SuffixBatchDefinition parsed;
    if (!legacy && !parse_checkpoint_validation(root["checkpoint_validation"], parsed)) {
        error = "suffix batch checkpoint validation is invalid";
        return false;
    }
    parsed.sourceBoundaryFingerprint = root["source_boundary_fingerprint"].get<std::string>();
    if (!read_integer(root["source_frame"], std::size_t{0}, std::numeric_limits<std::size_t>::max(),
            parsed.sourceFrame) ||
        !read_integer(
            root["maximum_ticks"], std::size_t{1}, SuffixBatchMaximumTicks, parsed.maximumTicks))
    {
        error = "source_frame or maximum_ticks is out of range";
        return false;
    }
    parsed.verifyStateHashes = root["verify_state_hashes"].get<bool>();
    if (cached) {
        constexpr std::array CacheKeys{
            std::string_view{"capacity_bytes"},
            std::string_view{"capacity_entries"},
            std::string_view{"source_identity"},
            std::string_view{"source_route_ticks"},
            std::string_view{"retain_candidate_checkpoints"},
            std::string_view{"retain_live_endpoint"},
        };
        constexpr std::array VariableCacheKeys{
            std::string_view{"capacity_bytes"},
            std::string_view{"capacity_entries"},
            std::string_view{"source_identity"},
            std::string_view{"source_route_ticks"},
            std::string_view{"retain_candidate_checkpoints"},
            std::string_view{"retain_live_endpoint"},
            std::string_view{"retain_candidate_index"},
        };
        const json& cache = root["checkpoint_cache"];
        SuffixCheckpointCachePolicy policy;
        const bool variableCacheShape =
            has_exact_keys(cache, CacheKeys) || has_exact_keys(cache, VariableCacheKeys);
        if (!(variableCached ? variableCacheShape : has_exact_keys(cache, CacheKeys)) ||
            !read_integer(cache["capacity_bytes"], std::size_t{1},
                SuffixBatchMaximumCheckpointCacheBytes, policy.capacityBytes) ||
            !read_integer(cache["capacity_entries"], std::size_t{1},
                SuffixBatchMaximumCheckpointCacheEntries, policy.capacityEntries) ||
            !read_integer(cache["source_route_ticks"], std::size_t{0},
                SuffixBatchMaximumExpandedTicks, policy.sourceRouteTicks) ||
            !cache["retain_candidate_checkpoints"].is_boolean() ||
            !cache["retain_live_endpoint"].is_boolean() ||
            !(cache["source_identity"].is_null() ||
                valid_boundary_fingerprint(cache["source_identity"])))
        {
            error = "suffix batch checkpoint cache policy is invalid";
            return false;
        }
        if (cache["source_identity"].is_string())
            policy.sourceIdentity = cache["source_identity"].get<std::string>();
        policy.retainCandidateCheckpoints =
            cache["retain_candidate_checkpoints"].get<bool>();
        policy.retainLiveEndpoint = cache["retain_live_endpoint"].get<bool>();
        if (variableCached && cache.contains("retain_candidate_index")) {
            std::size_t retainedIndex = 0;
            if (!read_integer(cache["retain_candidate_index"], std::size_t{0},
                    SuffixBatchMaximumCandidates - 1, retainedIndex)) {
                error = "suffix batch retained candidate index is invalid";
                return false;
            }
            policy.retainCandidateIndex = retainedIndex;
        }
        if (static_cast<unsigned>(policy.retainCandidateCheckpoints) +
                static_cast<unsigned>(policy.retainLiveEndpoint) +
                static_cast<unsigned>(policy.retainCandidateIndex.has_value()) >
            1) {
            error = "suffix batch checkpoint cache requests conflicting retention modes";
            return false;
        }
        parsed.checkpointCache = std::move(policy);
    }
    if (frozen) {
        if (!root["demonstration_mode"].is_string()) {
            error = "suffix batch demonstration mode is invalid";
            return false;
        }
        if (!root["action_authority"].is_string() ||
            root["action_authority"].get_ref<const std::string&>() != "episode_policy") {
            error = "suffix batch action authority must be episode_policy";
            return false;
        }
        const auto& mode = root["demonstration_mode"].get_ref<const std::string&>();
        if (mode == "absent") {
            parsed.demonstrationMode = SuffixDemonstrationMode::Absent;
        } else if (mode == "replay_only") {
            parsed.demonstrationMode = SuffixDemonstrationMode::ReplayOnly;
        } else if (mode == "behavior_cloning_warm_start") {
            parsed.demonstrationMode = SuffixDemonstrationMode::BehaviorCloningWarmStart;
        } else if (mode == "reverse_curriculum_checkpoints") {
            parsed.demonstrationMode = SuffixDemonstrationMode::ReverseCurriculumCheckpoints;
        } else {
            error = "suffix batch demonstration mode is invalid";
            return false;
        }
        SuffixBatchFrozenPolicy frozenPolicy;
        if (!parse_frozen_policy(root["frozen_policy"], exploratoryFrozen, frozenPolicy)) {
            error = "suffix batch frozen policy is invalid";
            return false;
        }
        parsed.frozenPolicy = std::move(frozenPolicy);
    }
    const json& candidates = root["candidates"];
    if (candidates.empty() || candidates.size() > SuffixBatchMaximumCandidates) {
        error = "candidate count is empty or exceeds the bounded maximum";
        return false;
    }
    if (candidates.size() > SuffixBatchMaximumExpandedTicks / parsed.maximumTicks) {
        error = "expanded candidate ticks exceed the bounded in-memory maximum";
        return false;
    }
    parsed.candidates.reserve(candidates.size());
    std::unordered_set<std::string> ids;
    ids.reserve(candidates.size());
    for (std::size_t index = 0; index < candidates.size(); ++index) {
        SuffixBatchCandidate candidate;
        if (!parse_candidate(candidates[index], parsed.maximumTicks,
                factorized || frozen, frozen,
                reactive || (cached && candidates[index].is_object() &&
                    candidates[index].contains("controller_program_hex")), variableCached,
                candidate, error)) {
            error = "candidate " + std::to_string(index) + ": " + error;
            return false;
        }
        if (!ids.insert(candidate.id).second) {
            error = "candidate " + std::to_string(index) + " has a duplicate id";
            return false;
        }
        parsed.candidates.push_back(std::move(candidate));
    }
    if (parsed.checkpointCache.has_value() &&
        parsed.checkpointCache->retainLiveEndpoint && parsed.candidates.size() != 1)
    {
        error = "live endpoint retention requires exactly one candidate";
        return false;
    }
    if (parsed.checkpointCache.has_value() &&
        parsed.checkpointCache->retainCandidateIndex.has_value() &&
        *parsed.checkpointCache->retainCandidateIndex >= parsed.candidates.size()) {
        error = "suffix batch retained candidate index exceeds the candidate batch";
        return false;
    }
    const bool hasFrozenCandidate = std::ranges::any_of(parsed.candidates,
        [](const SuffixBatchCandidate& candidate) { return candidate.frozenPolicy; });
    if (hasFrozenCandidate != parsed.frozenPolicy.has_value()) {
        error = "suffix batch frozen policy and candidate sources disagree";
        return false;
    }
    output = std::move(parsed);
    return true;
}

}  // namespace dusk::automation
