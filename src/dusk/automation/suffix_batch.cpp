#include "dusk/automation/suffix_batch.hpp"

#include <array>
#include <cstdint>
#include <limits>
#include <string>
#include <type_traits>
#include <unordered_set>
#include <utility>

#include <nlohmann/json.hpp>

namespace dusk::automation {
namespace {

using json = nlohmann::json;

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

bool valid_boundary_fingerprint(const json& value) {
    if (!value.is_string()) return false;
    const auto& fingerprint = value.get_ref<const std::string&>();
    if (fingerprint.size() != 32) return false;
    for (const unsigned char byte : fingerprint) {
        if (!((byte >= '0' && byte <= '9') || (byte >= 'a' && byte <= 'f'))) return false;
    }
    return true;
}

bool parse_candidate(const json& value, const std::size_t maximumTicks,
    SuffixBatchCandidate& output, std::string& error) {
    constexpr std::array ActionKeys{
        std::string_view{"id"}, std::string_view{"actions"}};
    constexpr std::array TapeKeys{
        std::string_view{"id"}, std::string_view{"source"}};
    const bool actionCandidate = has_exact_keys(value, ActionKeys);
    const bool tapeCandidate = has_exact_keys(value, TapeKeys);
    if ((!actionCandidate && !tapeCandidate) || !value["id"].is_string())
    {
        error = "candidate must contain id plus either actions or source";
        return false;
    }
    const std::string id = value["id"].get<std::string>();
    if (id.empty() || id.size() > 128) {
        error = "candidate id is empty or exceeds 128 bytes";
        return false;
    }
    for (const unsigned char byte : id) {
        if (byte < 0x21 || byte > 0x7e) {
            error = "candidate id must be printable ASCII without whitespace";
            return false;
        }
    }
    if (tapeCandidate) {
        if (!value["source"].is_string() ||
            value["source"].get_ref<const std::string&>() != "tape")
        {
            error = "candidate source must be tape";
            return false;
        }
        output = {.id = id, .tapePassthrough = true};
        return true;
    }

    const auto& actions = value["actions"];
    if (!actions.is_array()) {
        error = "candidate actions must be an array";
        return false;
    }
    if (actions.empty() || actions.size() > maximumTicks) {
        error = "candidate action count is empty or exceeds maximum_ticks";
        return false;
    }

    SuffixBatchCandidate parsed;
    parsed.id = id;
    parsed.pads.reserve(maximumTicks);
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
        if (!read_integer(action["frames"], std::size_t{1}, maximumTicks, frames) ||
            !parse_pad(action["pad"], pad) || frames > maximumTicks - parsed.pads.size())
        {
            error = "candidate action " + std::to_string(index) +
                    " has invalid pad fields or duration";
            return false;
        }
        parsed.pads.insert(parsed.pads.end(), frames, pad);
    }
    if (parsed.pads.size() != maximumTicks) {
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
    const json root = json::parse(source, nullptr, false);
    constexpr std::array RootKeys{
        std::string_view{"schema"},
        std::string_view{"source_frame"},
        std::string_view{"source_boundary_fingerprint"},
        std::string_view{"maximum_ticks"},
        std::string_view{"verify_state_hashes"},
        std::string_view{"candidates"},
    };
    if (root.is_discarded() || !has_exact_keys(root, RootKeys) ||
        !root["schema"].is_string() ||
        root["schema"].get_ref<const std::string&>() != SuffixBatchSchema ||
        !valid_boundary_fingerprint(root["source_boundary_fingerprint"]) ||
        !root["verify_state_hashes"].is_boolean() || !root["candidates"].is_array())
    {
        error = "suffix batch root or schema is invalid";
        return false;
    }

    SuffixBatchDefinition parsed;
    parsed.sourceBoundaryFingerprint =
        root["source_boundary_fingerprint"].get<std::string>();
    if (!read_integer(root["source_frame"], std::size_t{0},
            std::numeric_limits<std::size_t>::max(), parsed.sourceFrame) ||
        !read_integer(root["maximum_ticks"], std::size_t{1},
            SuffixBatchMaximumTicks, parsed.maximumTicks))
    {
        error = "source_frame or maximum_ticks is out of range";
        return false;
    }
    parsed.verifyStateHashes = root["verify_state_hashes"].get<bool>();
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
        if (!parse_candidate(candidates[index], parsed.maximumTicks, candidate, error)) {
            error = "candidate " + std::to_string(index) + ": " + error;
            return false;
        }
        if (!ids.insert(candidate.id).second) {
            error = "candidate " + std::to_string(index) + " has a duplicate id";
            return false;
        }
        parsed.candidates.push_back(std::move(candidate));
    }
    output = std::move(parsed);
    return true;
}

}  // namespace dusk::automation
