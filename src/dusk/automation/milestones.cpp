#include "dusk/automation/milestones.hpp"

#include <algorithm>
#include <array>
#include <cstring>
#include <fstream>
#include <system_error>

#include <nlohmann/json.hpp>

namespace dusk::automation {
namespace {

using nlohmann::json;

bool stage_is(const char* actual, const char* expected) {
    return actual != nullptr && std::strcmp(actual, expected) == 0;
}

bool gameplay_ready_f_sp103(const MilestoneObservation& observation) {
    return stage_is(observation.stageName, "F_SP103") && observation.room == 1 &&
           observation.point == 1 && observation.playerPresent && observation.playerIsLink &&
           !observation.eventRunning;
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
        .eventRunning = observation.eventRunning,
        .nextStageEnabled = observation.nextStageEnabled,
        .nextStageName = observation.nextStageName == nullptr ? "" : observation.nextStageName,
        .nextRoom = observation.nextRoom,
        .nextLayer = observation.nextLayer,
        .nextPoint = observation.nextPoint,
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
            }},
        {"event_running", evidence.eventRunning},
        {"next_stage",
            {
                {"enabled", evidence.nextStageEnabled},
                {"name", evidence.nextStageName},
                {"room", evidence.nextRoom},
                {"layer", evidence.nextLayer},
                {"point", evidence.nextPoint},
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
