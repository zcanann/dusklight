#include "dusk/automation/worker.hpp"

#include <cstdio>
#include <iostream>
#include <string>
#include <string_view>

#if defined(_WIN32)
#include <io.h>
#define DUSK_CLOSE _close
#define DUSK_DUP _dup
#define DUSK_DUP2 _dup2
#define DUSK_FDOPEN _fdopen
#define DUSK_FILENO _fileno
#else
#include <unistd.h>
#define DUSK_CLOSE close
#define DUSK_DUP dup
#define DUSK_DUP2 dup2
#define DUSK_FDOPEN fdopen
#define DUSK_FILENO fileno
#endif

#include <nlohmann/json.hpp>

#include "dusk/automation/build_identity.hpp"
#include "dusk/automation/engine_session.hpp"
#include "dusk/logging.h"

namespace dusk::automation {
namespace {

using json = nlohmann::ordered_json;

constexpr std::string_view ProtocolName = "dusklight-automation";
constexpr std::size_t MaximumCommandBytes = 1024 * 1024;
FILE* engineProtocolOutput;

enum class InvocationMode {
    None,
    Hello,
    Worker,
    EngineWorker,
    Conflict,
};

InvocationMode detect_invocation(int argc, char* argv[]) {
    InvocationMode mode = InvocationMode::None;

    for (int i = 1; i < argc; ++i) {
        const std::string_view argument(argv[i]);
        InvocationMode requested = InvocationMode::None;
        if (argument == "--automation-hello") {
            requested = InvocationMode::Hello;
        } else if (argument == "--automation-worker") {
            requested = InvocationMode::Worker;
        } else if (argument == "--automation-engine-worker") {
            requested = InvocationMode::EngineWorker;
        }

        if (requested == InvocationMode::None) {
            continue;
        }
        if (mode != InvocationMode::None && mode != requested) {
            return InvocationMode::Conflict;
        }
        mode = requested;
    }

    return mode;
}

json protocol_fields(std::string_view type, bool ok) {
    return {
        {"protocol", {{"name", ProtocolName}, {"version", WorkerProtocolVersion}}},
        {"type", type},
        {"ok", ok},
    };
}

std::string_view fidelity_profile_from_command_line(int, char*[]) {
    return "cursor_breakout_shadow";
}

json build_identity_json(const std::string_view fidelityProfile) {
    const BuildIdentity build = current_build_identity(fidelityProfile);
    return {
        {"version", build.version},
        {"describe", build.describe},
        {"revision", build.revision},
        {"dirty_digest", build.dirtyDigest},
        {"branch", build.branch},
        {"source_date", build.sourceDate},
        {"aurora_revision", build.auroraRevision},
        {"compiler", build.compiler},
        {"compiler_target", build.compilerTarget},
        {"build_type", build.buildType},
        {"feature_switches", build.featureSwitches},
        {"feature_digest", build.featureDigest},
        {"fidelity_profile", build.fidelityProfile},
        {"platform", build.platform},
        {"architecture", build.architecture},
        {"pointer_bits", build.pointerBits},
        {"dirty", build.dirty},
    };
}

json hello_response(const std::string_view fidelityProfile, const bool engineWorker = false) {
    json response = protocol_fields("hello", true);
    response["build"] = build_identity_json(fidelityProfile);
    response["capabilities"] = {
        {"persistent_control", true},
        // The engine worker reuses one authenticated source checkpoint across
        // batches. Full post-run engine reconstruction remains unclaimed.
        {"engine_session", false},
        {"headless", true},
        {"scenario_load", false},
        {"input_tape", true},
        {"batch_run", engineWorker},
        {"commands", engineWorker
                ? json{"hello", "run_batch", "shutdown"}
                : json{"hello", "ping", "session_audit", "shutdown"}},
    };
    return response;
}

void copy_request_id(const json& request, json& response) {
    const auto id = request.find("id");
    if (id != request.end()) {
        response["id"] = *id;
    }
}

json error_response(std::string_view code, std::string_view message) {
    json response = protocol_fields("error", false);
    response["error"] = {{"code", code}, {"message", message}};
    return response;
}

json session_audit_response() {
    json response = protocol_fields("session_audit", true);
    response["audit"] = {
        {"schema", EngineSessionReuseAuditSchema},
        {"reusable", false},
        {"evaluated_boundary", "pre_engine_boot"},
        {"target_boundary", "post_authenticated_run"},
        {"blockers", json::array()},
    };
    for (const auto& blocker : engine_session_reuse_blockers()) {
        response["audit"]["blockers"].push_back({
            {"code", blocker.code},
            {"subsystem", blocker.subsystem},
            {"required_guarantee", blocker.requiredGuarantee},
        });
    }
    return response;
}

void write_response(const json& response) {
    const std::string bytes = response.dump();
    if (engineProtocolOutput != nullptr) {
        std::fwrite(bytes.data(), 1, bytes.size(), engineProtocolOutput);
        std::fputc('\n', engineProtocolOutput);
        std::fflush(engineProtocolOutput);
        return;
    }
    std::cout << bytes << '\n' << std::flush;
}

bool reserve_engine_protocol_output() {
    std::fflush(stdout);
    std::fflush(stderr);
    const int protocolDescriptor = DUSK_DUP(DUSK_FILENO(stdout));
    if (protocolDescriptor < 0)
        return false;
    FILE* protocolOutput = DUSK_FDOPEN(protocolDescriptor, "w");
    if (protocolOutput == nullptr) {
        DUSK_CLOSE(protocolDescriptor);
        return false;
    }
    if (DUSK_DUP2(DUSK_FILENO(stderr), DUSK_FILENO(stdout)) < 0) {
        std::fclose(protocolOutput);
        return false;
    }
    engineProtocolOutput = protocolOutput;
    return true;
}

int run_worker(const std::string_view fidelityProfile) {
    std::string line;
    while (std::getline(std::cin, line)) {
        if (line.empty()) {
            continue;
        }
        if (line.size() > MaximumCommandBytes) {
            write_response(error_response("command_too_large", "command exceeds the 1 MiB limit"));
            continue;
        }

        const json request = json::parse(line, nullptr, false);
        if (request.is_discarded() || !request.is_object()) {
            write_response(error_response("invalid_json", "expected one JSON object per line"));
            continue;
        }

        const auto commandValue = request.find("command");
        if (commandValue == request.end() || !commandValue->is_string()) {
            json response = error_response("missing_command", "command must be a string");
            copy_request_id(request, response);
            write_response(response);
            continue;
        }

        const std::string command = commandValue->get<std::string>();
        if (command == "hello") {
            json response = hello_response(fidelityProfile);
            copy_request_id(request, response);
            write_response(response);
        } else if (command == "ping") {
            json response = protocol_fields("pong", true);
            copy_request_id(request, response);
            write_response(response);
        } else if (command == "session_audit") {
            json response = session_audit_response();
            copy_request_id(request, response);
            write_response(response);
        } else if (command == "shutdown") {
            json response = protocol_fields("shutdown", true);
            copy_request_id(request, response);
            write_response(response);
            return 0;
        } else {
            json response = error_response("unknown_command", "unsupported command");
            copy_request_id(request, response);
            write_response(response);
        }
    }

    return 0;
}

bool engineWorkerEnabled;

bool begin_engine_worker(const std::string_view fidelityProfile) {
    std::string line;
    if (!std::getline(std::cin, line) || line.empty() || line.size() > MaximumCommandBytes)
        return false;
    const json request = json::parse(line, nullptr, false);
    if (request.is_discarded() || !request.is_object() || request.size() != 2 ||
        !request.contains("id") || !request["id"].is_number_unsigned() ||
        !request.contains("command") || !request["command"].is_string() ||
        request["command"].get_ref<const std::string&>() != "hello")
    {
        write_response(error_response("invalid_handshake",
            "engine worker requires an exact hello request before engine boot"));
        return false;
    }
    json response = hello_response(fidelityProfile, true);
    copy_request_id(request, response);
    write_response(response);
    if (!reserve_engine_protocol_output()) {
        write_response(error_response("protocol_transport",
            "could not reserve stdout for the engine worker protocol"));
        return false;
    }
    // The engine's ordinary INFO/DEBUG sink also uses stdout. Reserve it now,
    // before boot, so every subsequent stdout line remains an NDJSON frame.
    dusk::ReserveStdoutForAutomationProtocol();
    engineWorkerEnabled = true;
    return true;
}

}  // namespace

bool engine_worker_enabled() {
    return engineWorkerEnabled;
}

bool wait_for_engine_worker_command(EngineWorkerCommand& command, std::string& error) {
    command = {};
    error.clear();
    if (!engineWorkerEnabled) {
        error = "engine worker control is not enabled";
        return false;
    }
    std::string line;
    while (std::getline(std::cin, line)) {
        if (line.empty()) continue;
        if (line.size() > MaximumCommandBytes) {
            write_response(error_response("command_too_large", "command exceeds the 1 MiB limit"));
            continue;
        }
        const json request = json::parse(line, nullptr, false);
        if (request.is_discarded() || !request.is_object() || !request.contains("id") ||
            !request["id"].is_number_unsigned() || !request.contains("command") ||
            !request["command"].is_string())
        {
            write_response(error_response("invalid_json",
                "engine worker command requires unsigned id and string command"));
            continue;
        }
        const std::uint64_t requestId = request["id"].get<std::uint64_t>();
        const std::string& name = request["command"].get_ref<const std::string&>();
        if (name == "shutdown" && request.size() == 2) {
            json response = protocol_fields("shutdown", true);
            copy_request_id(request, response);
            write_response(response);
            command.kind = EngineWorkerCommandKind::Shutdown;
            command.requestId = requestId;
            return true;
        }
        if (name != "run_batch" || request.size() != 5 || !request.contains("batch") ||
            !request["batch"].is_string() || !request.contains("result") ||
            !request["result"].is_string() || !request.contains("winner_tape") ||
            !(request["winner_tape"].is_null() || request["winner_tape"].is_string()))
        {
            json response = error_response("invalid_run_batch",
                "run_batch requires exact batch, result, and nullable winner_tape paths");
            copy_request_id(request, response);
            write_response(response);
            continue;
        }
        const std::string batch = request["batch"].get<std::string>();
        const std::string result = request["result"].get<std::string>();
        const std::string winner = request["winner_tape"].is_string()
            ? request["winner_tape"].get<std::string>() : std::string{};
        if (batch.empty() || result.empty() ||
            (request["winner_tape"].is_string() && winner.empty()))
        {
            json response = error_response("invalid_run_batch", "batch output paths cannot be empty");
            copy_request_id(request, response);
            write_response(response);
            continue;
        }
        command.kind = EngineWorkerCommandKind::RunBatch;
        command.requestId = requestId;
        command.batchPath = std::filesystem::u8path(batch);
        command.resultPath = std::filesystem::u8path(result);
        command.winnerTapePath = winner.empty() ? std::filesystem::path{} :
                                                  std::filesystem::u8path(winner);
        return true;
    }
    error = "engine worker control stream closed";
    return false;
}

void publish_engine_worker_batch_complete(const std::uint64_t requestId,
    const std::filesystem::path& resultPath, const std::filesystem::path& episodeShardPath) {
    json response = protocol_fields("batch_complete", true);
    response["id"] = requestId;
    response["result"] = resultPath.string();
    response["episode_shard"] = episodeShardPath.string();
    write_response(response);
}

void reject_engine_worker_batch(const std::uint64_t requestId, const std::string_view message) {
    json response = error_response("batch_rejected", message);
    response["id"] = requestId;
    write_response(response);
}

std::optional<int> run_from_command_line(int argc, char* argv[]) {
    const std::string_view fidelityProfile = fidelity_profile_from_command_line(argc, argv);
    switch (detect_invocation(argc, argv)) {
    case InvocationMode::None:
        return std::nullopt;
    case InvocationMode::Hello:
        write_response(hello_response(fidelityProfile));
        return 0;
    case InvocationMode::Worker:
        return run_worker(fidelityProfile);
    case InvocationMode::EngineWorker:
        if (!begin_engine_worker(fidelityProfile)) return 2;
        return std::nullopt;
    case InvocationMode::Conflict:
        write_response(error_response(
            "conflicting_mode", "choose either --automation-hello or --automation-worker"));
        return 2;
    }

    return 2;
}

}  // namespace dusk::automation
