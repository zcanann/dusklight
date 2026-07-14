#include "dusk/automation/milestones.hpp"

#include <algorithm>
#include <array>
#include <bit>
#include <cstring>
#include <fstream>
#include <system_error>
#include <type_traits>
#include <vector>

#include <nlohmann/json.hpp>
#include <xxhash.h>

namespace dusk::automation {
namespace {

using nlohmann::json;

template <typename T>
void append_integer(std::vector<std::uint8_t>& output, const T value) {
    using U = std::make_unsigned_t<T>;
    const U bits = static_cast<U>(value);
    for (std::size_t byte = 0; byte < sizeof(T); ++byte) {
        output.push_back(static_cast<std::uint8_t>((bits >> (byte * 8)) & 0xffu));
    }
}

void append_float(std::vector<std::uint8_t>& output, const float value) {
    append_integer(output, std::bit_cast<std::uint32_t>(value));
}

void append_fixed_string(std::vector<std::uint8_t>& output, const std::string_view value) {
    constexpr std::size_t Width = 8;
    for (std::size_t index = 0; index < Width; ++index) {
        output.push_back(index < value.size() ? static_cast<std::uint8_t>(value[index]) : 0);
    }
}

bool stage_is(const char* actual, const char* expected) {
    return actual != nullptr && std::strcmp(actual, expected) == 0;
}

bool gameplay_ready_f_sp103(const MilestoneObservation& observation) {
    return stage_is(observation.stageName, "F_SP103") && observation.room == 1 &&
           observation.point == 1 && observation.playerPresent && observation.playerIsLink &&
           !observation.eventRunning && observation.eventId == -1;
}

bool exit_f_sp103_to_f_sp104(const MilestoneObservation& observation) {
    return stage_is(observation.stageName, "F_SP103") && observation.room == 1 &&
           observation.point == 1 && observation.nextStageEnabled &&
           stage_is(observation.nextStageName, "F_SP104") && observation.nextRoom == 1 &&
           observation.nextPoint == 0;
}

bool entered_f_sp104(const MilestoneObservation& observation) {
    return stage_is(observation.stageName, "F_SP104") && observation.room == 1 &&
           observation.point == 0;
}

constexpr std::array<MilestoneDefinition, 3> Definitions{{
    {MilestoneId::GameplayReadyFSp103, "gameplay-ready-f-sp103",
        "Link is controllable after the opening in F_SP103 room 1 point 1", gameplay_ready_f_sp103},
    {MilestoneId::ExitFSp103ToFSp104, "exit-f-sp103-to-f-sp104",
        "F_SP103 has committed the scene transition to F_SP104 room 1 point 0",
        exit_f_sp103_to_f_sp104},
    {MilestoneId::EnteredFSp104, "entered-f-sp104", "The live stage is F_SP104 room 1 point 0",
        entered_f_sp104},
}};

MilestoneEvidence capture_evidence(const MilestoneObservation& observation) {
    return {
        .stageName = observation.stageName == nullptr ? "" : observation.stageName,
        .room = observation.room,
        .layer = observation.layer,
        .point = observation.point,
        .playerPresent = observation.playerPresent,
        .playerIsLink = observation.playerIsLink,
        .playerProcessId = observation.playerProcessId,
        .playerActorName = observation.playerActorName,
        .playerProcId = observation.playerProcId,
        .playerPositionX = observation.playerPositionX,
        .playerPositionY = observation.playerPositionY,
        .playerPositionZ = observation.playerPositionZ,
        .playerVelocityX = observation.playerVelocityX,
        .playerVelocityY = observation.playerVelocityY,
        .playerVelocityZ = observation.playerVelocityZ,
        .playerForwardSpeed = observation.playerForwardSpeed,
        .playerCurrentAngleX = observation.playerCurrentAngleX,
        .playerCurrentAngleY = observation.playerCurrentAngleY,
        .playerCurrentAngleZ = observation.playerCurrentAngleZ,
        .playerShapeAngleX = observation.playerShapeAngleX,
        .playerShapeAngleY = observation.playerShapeAngleY,
        .playerShapeAngleZ = observation.playerShapeAngleZ,
        .eventRunning = observation.eventRunning,
        .eventId = observation.eventId,
        .eventMode = observation.eventMode,
        .eventStatus = observation.eventStatus,
        .eventMapToolId = observation.eventMapToolId,
        .eventNameHash = observation.eventNameHash,
        .nextStageEnabled = observation.nextStageEnabled,
        .nextStageName = observation.nextStageName == nullptr ? "" : observation.nextStageName,
        .nextRoom = observation.nextRoom,
        .nextLayer = observation.nextLayer,
        .nextPoint = observation.nextPoint,
        .rng = observation.rng,
    };
}

json rng_stream_json(const GameRngStreamSnapshot& stream) {
    return {
        {"id", stream.id == GameRngStreamId::Primary ? "primary" : "secondary"},
        {"algorithm_version", stream.algorithmVersion},
        {"state", {stream.state0, stream.state1, stream.state2}},
        {"call_count", stream.callCount},
    };
}

json evidence_json(const MilestoneEvidence& evidence) {
    return {
        {"stage",
            {
                {"name", evidence.stageName},
                {"room", evidence.room},
                {"layer", evidence.layer},
                {"point", evidence.point},
            }},
        {"player",
            {
                {"present", evidence.playerPresent},
                {"is_link", evidence.playerIsLink},
                {"process_id", evidence.playerProcessId},
                {"actor_name", evidence.playerActorName},
                {"procedure_id", evidence.playerProcId},
                {"position",
                    {evidence.playerPositionX, evidence.playerPositionY, evidence.playerPositionZ}},
                {"velocity",
                    {evidence.playerVelocityX, evidence.playerVelocityY, evidence.playerVelocityZ}},
                {"forward_speed", evidence.playerForwardSpeed},
                {"current_angle", {evidence.playerCurrentAngleX, evidence.playerCurrentAngleY,
                                      evidence.playerCurrentAngleZ}},
                {"shape_angle", {evidence.playerShapeAngleX, evidence.playerShapeAngleY,
                                    evidence.playerShapeAngleZ}},
            }},
        {"event",
            {
                {"running", evidence.eventRunning},
                {"id", evidence.eventId},
                {"mode", evidence.eventMode},
                {"status", evidence.eventStatus},
                {"map_tool_id", evidence.eventMapToolId},
                {"name_fnv1a", evidence.eventNameHash},
            }},
        // Retained for additive compatibility with v1 consumers.
        {"event_running", evidence.eventRunning},
        {"next_stage",
            {
                {"enabled", evidence.nextStageEnabled},
                {"name", evidence.nextStageName},
                {"room", evidence.nextRoom},
                {"layer", evidence.nextLayer},
                {"point", evidence.nextPoint},
            }},
        {"rng",
            {
                {"snapshot_version", evidence.rng.version},
                {"stream_count", evidence.rng.streamCount},
                {"streams", {rng_stream_json(evidence.rng.streams[0]),
                                rng_stream_json(evidence.rng.streams[1])}},
            }},
        {"boundary_fingerprint",
            {
                {"schema", "dusklight.milestone-boundary/v1"},
                {"algorithm", "xxh3-128"},
                {"canonical_encoding", "little-endian-fixed-v1"},
                {"digest", evidence.boundaryFingerprint},
            }},
    };
}

}  // namespace

std::span<const MilestoneDefinition> milestone_definitions() {
    return Definitions;
}

const MilestoneDefinition* find_milestone(const MilestoneId id) {
    const auto found = std::ranges::find(Definitions, id, &MilestoneDefinition::id);
    return found == Definitions.end() ? nullptr : &*found;
}

const MilestoneDefinition* find_milestone(const std::string_view name) {
    const auto found = std::ranges::find(Definitions, name, &MilestoneDefinition::name);
    return found == Definitions.end() ? nullptr : &*found;
}

std::string_view milestone_name(const MilestoneId id) {
    const MilestoneDefinition* definition = find_milestone(id);
    return definition == nullptr ? "unknown" : definition->name;
}

std::string compute_milestone_boundary_fingerprint(const MilestoneEvidence& evidence) {
    std::vector<std::uint8_t> canonical;
    canonical.reserve(160);
    append_integer(canonical, MilestoneBoundaryFingerprintVersion);
    append_fixed_string(canonical, evidence.stageName);
    append_integer(canonical, evidence.room);
    append_integer(canonical, evidence.layer);
    append_integer(canonical, evidence.point);
    append_integer<std::uint8_t>(canonical, evidence.playerPresent ? 1 : 0);
    append_integer<std::uint8_t>(canonical, evidence.playerIsLink ? 1 : 0);
    append_integer(canonical, evidence.playerProcessId);
    append_integer(canonical, evidence.playerActorName);
    append_integer(canonical, evidence.playerProcId);
    append_float(canonical, evidence.playerPositionX);
    append_float(canonical, evidence.playerPositionY);
    append_float(canonical, evidence.playerPositionZ);
    append_float(canonical, evidence.playerVelocityX);
    append_float(canonical, evidence.playerVelocityY);
    append_float(canonical, evidence.playerVelocityZ);
    append_float(canonical, evidence.playerForwardSpeed);
    append_integer(canonical, evidence.playerCurrentAngleX);
    append_integer(canonical, evidence.playerCurrentAngleY);
    append_integer(canonical, evidence.playerCurrentAngleZ);
    append_integer(canonical, evidence.playerShapeAngleX);
    append_integer(canonical, evidence.playerShapeAngleY);
    append_integer(canonical, evidence.playerShapeAngleZ);
    append_integer<std::uint8_t>(canonical, evidence.eventRunning ? 1 : 0);
    append_integer(canonical, evidence.eventId);
    append_integer(canonical, evidence.eventMode);
    append_integer(canonical, evidence.eventStatus);
    append_integer(canonical, evidence.eventMapToolId);
    append_integer(canonical, evidence.eventNameHash);
    append_integer<std::uint8_t>(canonical, evidence.nextStageEnabled ? 1 : 0);
    append_fixed_string(canonical, evidence.nextStageName);
    append_integer(canonical, evidence.nextRoom);
    append_integer(canonical, evidence.nextLayer);
    append_integer(canonical, evidence.nextPoint);
    append_integer(canonical, evidence.rng.version);
    append_integer(canonical, evidence.rng.streamCount);
    for (const GameRngStreamSnapshot& stream : evidence.rng.streams) {
        append_integer(canonical, static_cast<std::uint8_t>(stream.id));
        append_integer(canonical, stream.algorithmVersion);
        append_integer(canonical, stream.state0);
        append_integer(canonical, stream.state1);
        append_integer(canonical, stream.state2);
        append_integer(canonical, stream.callCount);
    }

    const XXH128_hash_t hash = XXH3_128bits(canonical.data(), canonical.size());
    XXH128_canonical_t digest;
    XXH128_canonicalFromHash(&digest, hash);
    constexpr char Hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(sizeof(digest.digest) * 2);
    for (const unsigned char byte : digest.digest) {
        output.push_back(Hex[byte >> 4]);
        output.push_back(Hex[byte & 0x0f]);
    }
    return output;
}

bool parse_milestone_list(
    const std::string_view text, std::vector<MilestoneId>& output, std::string& error) {
    output.clear();
    if (text.empty()) {
        error = "milestone list cannot be empty";
        return false;
    }

    std::size_t begin = 0;
    while (begin <= text.size()) {
        const std::size_t end = text.find(',', begin);
        const std::string_view name =
            text.substr(begin, end == std::string_view::npos ? text.size() - begin : end - begin);
        const MilestoneDefinition* definition = find_milestone(name);
        if (definition == nullptr) {
            error = "unknown milestone '" + std::string(name) + "'";
            output.clear();
            return false;
        }
        if (std::ranges::find(output, definition->id) == output.end()) {
            output.push_back(definition->id);
        }
        if (end == std::string_view::npos) {
            break;
        }
        begin = end + 1;
    }
    return true;
}

bool MilestoneTracker::configure(const std::span<const MilestoneId> requested,
    const std::optional<MilestoneId> goal, std::string& error) {
    mHits.clear();
    mGoal.reset();
    if (requested.empty()) {
        error = "at least one milestone must be requested";
        return false;
    }
    for (const MilestoneId id : requested) {
        if (find_milestone(id) == nullptr) {
            error = "requested milestone is not registered";
            mHits.clear();
            return false;
        }
        if (std::ranges::find(mHits, id, &MilestoneHit::id) == mHits.end()) {
            mHits.push_back({.id = id});
        }
    }
    if (goal.has_value() && std::ranges::find(mHits, *goal, &MilestoneHit::id) == mHits.end()) {
        error = "goal '" + std::string(milestone_name(*goal)) + "' was not requested";
        mHits.clear();
        return false;
    }
    mGoal = goal;
    return true;
}

void MilestoneTracker::reset() {
    for (MilestoneHit& hit : mHits) {
        hit.hit = false;
        hit.simulationTick = 0;
        hit.tapeFrame = MilestoneNoTapeFrame;
        hit.evidence = {};
    }
}

void MilestoneTracker::observe(const MilestoneObservation& observation,
    const std::uint64_t simulationTick, const std::uint64_t tapeFrame) {
    for (MilestoneHit& hit : mHits) {
        if (hit.hit) {
            continue;
        }
        const MilestoneDefinition* definition = find_milestone(hit.id);
        if (definition != nullptr && definition->predicate(observation)) {
            hit.hit = true;
            hit.simulationTick = simulationTick;
            hit.tapeFrame = tapeFrame;
            hit.evidence = capture_evidence(observation);
            hit.evidence.boundaryFingerprint = compute_milestone_boundary_fingerprint(hit.evidence);
        }
    }
}

bool MilestoneTracker::goalReached() const {
    if (!mGoal.has_value()) {
        return false;
    }
    const auto found = std::ranges::find(mHits, *mGoal, &MilestoneHit::id);
    return found != mHits.end() && found->hit;
}

MilestoneTracker& milestone_tracker() {
    static MilestoneTracker tracker;
    return tracker;
}

std::string serialize_milestone_result(const MilestoneTracker& tracker) {
    json milestones = json::array();
    for (const MilestoneHit& hit : tracker.hits()) {
        json item{
            {"id", milestone_name(hit.id)},
            {"hit", hit.hit},
        };
        if (hit.hit) {
            item["sim_tick"] = hit.simulationTick;
            item["tape_frame"] =
                hit.tapeFrame == MilestoneNoTapeFrame ? json(nullptr) : json(hit.tapeFrame);
            item["evidence"] = evidence_json(hit.evidence);
        } else {
            item["sim_tick"] = nullptr;
            item["tape_frame"] = nullptr;
            item["evidence"] = nullptr;
        }
        milestones.push_back(std::move(item));
    }

    return json{
        {"schema",
            {
                {"name", "dusklight.automation.milestones"},
                {"version", MilestoneResultSchemaVersion},
            }},
        {"goal",
            tracker.goal().has_value() ? json(milestone_name(*tracker.goal())) : json(nullptr)},
        {"goal_reached", tracker.goalReached()},
        {"milestones", std::move(milestones)},
    }
        .dump(2);
}

bool write_milestone_result(
    const std::filesystem::path& path, const MilestoneTracker& tracker, std::string& error) {
    std::error_code filesystemError;
    if (const auto parent = path.parent_path(); !parent.empty()) {
        std::filesystem::create_directories(parent, filesystemError);
        if (filesystemError) {
            error = filesystemError.message();
            return false;
        }
    }
    std::ofstream stream(path, std::ios::binary | std::ios::trunc);
    if (!stream) {
        error = "could not open milestone result for writing";
        return false;
    }
    stream << serialize_milestone_result(tracker) << '\n';
    if (!stream) {
        error = "failed while writing milestone result";
        return false;
    }
    return true;
}

}  // namespace dusk::automation
