#include "dusk/automation/worker.hpp"

#include <iostream>
#include <string>
#include <string_view>

#include <nlohmann/json.hpp>

#include "dusk/automation/build_identity.hpp"

namespace dusk::automation {
namespace {

using json = nlohmann::ordered_json;

constexpr std::string_view ProtocolName = "dusklight-automation";
constexpr std::size_t MaximumCommandBytes = 1024 * 1024;

enum class InvocationMode {
    None,
    Hello,
    Worker,
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

json build_identity_json() {
    const BuildIdentity build = current_build_identity();
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
        {"platform", build.platform},
        {"architecture", build.architecture},
        {"pointer_bits", build.pointerBits},
        {"dirty", build.dirty},
    };
}

json hello_response() {
    json response = protocol_fields("hello", true);
    response["build"] = build_identity_json();
    response["capabilities"] = {
        {"persistent_control", true},
        {"engine_session", false},
        {"headless", true},
        {"scenario_load", false},
        {"input_tape", true},
        {"batch_run", false},
        {"commands", {"hello", "ping", "shutdown"}},
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

void write_response(const json& response) {
    std::cout << response.dump() << '\n' << std::flush;
}

int run_worker() {
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
            json response = hello_response();
            copy_request_id(request, response);
            write_response(response);
        } else if (command == "ping") {
            json response = protocol_fields("pong", true);
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

}  // namespace

std::optional<int> run_from_command_line(int argc, char* argv[]) {
    switch (detect_invocation(argc, argv)) {
    case InvocationMode::None:
        return std::nullopt;
    case InvocationMode::Hello:
        write_response(hello_response());
        return 0;
    case InvocationMode::Worker:
        return run_worker();
    case InvocationMode::Conflict:
        write_response(error_response(
            "conflicting_mode", "choose either --automation-hello or --automation-worker"));
        return 2;
    }

    return 2;
}

}  // namespace dusk::automation
