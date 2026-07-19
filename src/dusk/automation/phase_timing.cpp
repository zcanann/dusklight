#include "dusk/automation/phase_timing.hpp"

#include <array>
#include <chrono>
#include <fstream>
#include <optional>
#include <system_error>

#include <nlohmann/json.hpp>

#include "dusk/automation/engine_session.hpp"

namespace dusk::automation {
namespace {

using Clock = std::chrono::steady_clock;
using Phase = NativeLifecyclePhase;
constexpr std::size_t PhaseCount = static_cast<std::size_t>(Phase::Count);

struct Recorder {
    Clock::time_point start{};
    std::array<std::optional<std::uint64_t>, PhaseCount> microseconds{};
    std::optional<std::string> sessionReuseBoundary;
    bool active = false;
};

Recorder recorder;

constexpr std::size_t index(const Phase phase) {
    return static_cast<std::size_t>(phase);
}

std::uint64_t elapsed_microseconds() {
    const auto elapsed = std::chrono::duration_cast<std::chrono::microseconds>(
        Clock::now() - recorder.start);
    return elapsed.count() < 0 ? 0 : static_cast<std::uint64_t>(elapsed.count());
}

constexpr std::array<const char*, PhaseCount> PhaseNames{
    "process_entry_micros",
    "cli_configured_micros",
    "aurora_initialized_micros",
    "engine_ready_micros",
    "stage_ready_micros",
    "first_simulation_tick_micros",
    "last_simulation_tick_micros",
    "proof_artifacts_written_micros",
    "engine_shutdown_micros",
    "exit_ready_micros",
};

}  // namespace

void begin_native_lifecycle_timing() {
    recorder = {};
    recorder.start = Clock::now();
    recorder.active = true;
    recorder.microseconds[index(Phase::ProcessEntry)] = 0;
}

void mark_native_lifecycle_phase(const Phase phase) {
    if (!recorder.active || phase == Phase::Count) {
        return;
    }
    auto& value = recorder.microseconds[index(phase)];
    if (!value.has_value()) {
        value = elapsed_microseconds();
    }
}

void update_native_lifecycle_phase(const Phase phase) {
    if (!recorder.active || phase == Phase::Count) {
        return;
    }
    recorder.microseconds[index(phase)] = elapsed_microseconds();
}

void record_native_session_reuse_audit(const std::string_view evaluatedBoundary) {
    if (recorder.active && !evaluatedBoundary.empty()) {
        recorder.sessionReuseBoundary = evaluatedBoundary;
    }
}

bool write_native_lifecycle_timing(const std::filesystem::path& path, std::string& error) {
    error.clear();
    if (!recorder.active || path.empty()) {
        error = "native lifecycle timing is inactive or has no output path";
        return false;
    }
    std::uint64_t previous = 0;
    for (std::size_t phase = 0; phase < PhaseCount; ++phase) {
        const auto& value = recorder.microseconds[phase];
        if (!value.has_value()) {
            error = std::string("native lifecycle phase is missing: ") + PhaseNames[phase];
            return false;
        }
        if (*value < previous) {
            error = std::string("native lifecycle phase is nonmonotonic: ") +
                    PhaseNames[phase] + "=" + std::to_string(*value) +
                    " precedes " + std::to_string(previous);
            return false;
        }
        previous = *value;
    }
    if (recorder.sessionReuseBoundary != "post_authenticated_run") {
        error = "native lifecycle timing is missing the post-run session reuse audit";
        return false;
    }

    nlohmann::ordered_json document{
        {"schema", "dusklight-native-lifecycle-timing/v2"},
        {"clock", "steady_clock"},
    };
    for (std::size_t phase = 0; phase < PhaseCount; ++phase) {
        document[PhaseNames[phase]] = *recorder.microseconds[phase];
    }
    const auto blockers = engine_session_reuse_blockers();
    document["session_reuse_audit"] = {
        {"schema", EngineSessionReuseAuditSchema},
        {"reusable", blockers.empty()},
        {"evaluated_boundary", *recorder.sessionReuseBoundary},
        {"target_boundary", "post_authenticated_run"},
        {"blockers", nlohmann::ordered_json::array()},
    };
    for (const auto& blocker : blockers) {
        document["session_reuse_audit"]["blockers"].push_back({
            {"code", blocker.code},
            {"subsystem", blocker.subsystem},
            {"required_guarantee", blocker.requiredGuarantee},
        });
    }
    const std::string encoded = document.dump(2) + '\n';
    auto temporary = path;
    temporary += ".tmp";
    std::error_code filesystemError;
    if (std::filesystem::exists(path, filesystemError) ||
        std::filesystem::exists(temporary, filesystemError)) {
        error = "native lifecycle timing output already exists";
        return false;
    }
    {
        std::ofstream output(temporary, std::ios::binary | std::ios::out);
        output.write(encoded.data(), static_cast<std::streamsize>(encoded.size()));
        output.flush();
        if (!output) {
            error = "cannot write native lifecycle timing temporary file";
            std::filesystem::remove(temporary, filesystemError);
            return false;
        }
    }
    std::filesystem::rename(temporary, path, filesystemError);
    if (filesystemError) {
        error = filesystemError.message();
        std::filesystem::remove(temporary, filesystemError);
        return false;
    }
    return true;
}

}  // namespace dusk::automation
