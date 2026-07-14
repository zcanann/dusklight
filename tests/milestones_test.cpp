#include "dusk/automation/milestones.hpp"

#include <bit>
#include <cstdlib>
#include <iostream>

#include <nlohmann/json.hpp>

namespace {

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "milestones_test.cpp:" << line << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

dusk::automation::MilestoneObservation f_sp103() {
    dusk::automation::MilestoneObservation observation{
        .stageName = "F_SP103",
        .room = 1,
        .layer = -1,
        .point = 1,
        .playerPresent = true,
        .playerIsLink = true,
        .playerProcessId = 0x12345678,
        .playerActorName = 0x00fd,
        .playerProcId = 4,
        .playerPositionX = 666.0f,
        .playerPositionY = 800.0f,
        .playerPositionZ = -2431.0f,
        .playerVelocityX = 1.5f,
        .playerVelocityZ = -2.5f,
        .playerForwardSpeed = 3.0f,
        .playerCurrentAngleY = 100,
        .playerShapeAngleY = 101,
        .eventRunning = false,
        .eventId = -1,
        .eventMapToolId = 0xff,
    };
    observation.rng.streams[0] = {
        .id = dusk::automation::GameRngStreamId::Primary,
        .state0 = 11,
        .state1 = 12,
        .state2 = 13,
        .callCount = 100,
    };
    observation.rng.streams[1] = {
        .id = dusk::automation::GameRngStreamId::Secondary,
        .state0 = 21,
        .state1 = 22,
        .state2 = 23,
        .callCount = 200,
    };
    return observation;
}

void testRegistryAndParsing() {
    using namespace dusk::automation;
    REQUIRE(milestone_definitions().size() == 3);
    REQUIRE(find_milestone("gameplay-ready-f-sp103") != nullptr);
    REQUIRE(find_milestone("not-real") == nullptr);

    std::vector<MilestoneId> parsed;
    std::string error;
    REQUIRE(parse_milestone_list(
        "gameplay-ready-f-sp103,exit-f-sp103-to-f-sp104,gameplay-ready-f-sp103", parsed, error));
    REQUIRE(parsed.size() == 2);
    REQUIRE(parsed[0] == MilestoneId::GameplayReadyFSp103);
    REQUIRE(parsed[1] == MilestoneId::ExitFSp103ToFSp104);
    REQUIRE(!parse_milestone_list("nope", parsed, error));
    REQUIRE(error.find("unknown milestone") != std::string::npos);
}

void testGameplayReadinessRequiresExactMemoryState() {
    using namespace dusk::automation;
    const auto* definition = find_milestone(MilestoneId::GameplayReadyFSp103);
    REQUIRE(definition != nullptr);
    MilestoneObservation observation = f_sp103();
    REQUIRE(definition->predicate(observation));

    observation.eventRunning = true;
    REQUIRE(!definition->predicate(observation));
    observation.eventRunning = false;
    observation.playerIsLink = false;
    REQUIRE(!definition->predicate(observation));
    observation.playerIsLink = true;
    observation.room = 0;
    REQUIRE(!definition->predicate(observation));
    observation.room = 1;
    observation.point = 0;
    REQUIRE(!definition->predicate(observation));
}

void testExitRequiresCommittedExactDestination() {
    using namespace dusk::automation;
    const auto* definition = find_milestone(MilestoneId::ExitFSp103ToFSp104);
    REQUIRE(definition != nullptr);
    MilestoneObservation observation = f_sp103();
    observation.nextStageEnabled = true;
    observation.nextStageName = "F_SP104";
    observation.nextRoom = 1;
    observation.nextPoint = 0;
    REQUIRE(definition->predicate(observation));

    observation.nextStageName = "F_SP103";  // Ordon Village exit is not the goal.
    REQUIRE(!definition->predicate(observation));
    observation.nextStageName = "F_SP104";
    observation.nextPoint = 1;
    REQUIRE(!definition->predicate(observation));
    observation.nextPoint = 0;
    observation.nextStageEnabled = false;
    REQUIRE(!definition->predicate(observation));
}

void testTrackerCapturesOnlyTheFirstHitAndSerializesEvidence() {
    using namespace dusk::automation;
    MilestoneTracker tracker;
    constexpr MilestoneId requested[]{
        MilestoneId::GameplayReadyFSp103,
        MilestoneId::ExitFSp103ToFSp104,
        MilestoneId::EnteredFSp104,
    };
    std::string error;
    REQUIRE(tracker.configure(requested, MilestoneId::ExitFSp103ToFSp104, error));

    MilestoneObservation observation = f_sp103();
    tracker.observe(observation, 500, 480);
    tracker.observe(observation, 501, 481);
    REQUIRE(tracker.hits()[0].hit);
    REQUIRE(tracker.hits()[0].simulationTick == 500);
    REQUIRE(!tracker.goalReached());

    observation.nextStageEnabled = true;
    observation.nextStageName = "F_SP104";
    observation.nextRoom = 1;
    observation.nextPoint = 0;
    tracker.observe(observation, 700, 680);
    REQUIRE(tracker.goalReached());
    REQUIRE(tracker.hits()[1].simulationTick == 700);
    REQUIRE(tracker.hits()[1].evidence.nextStageName == "F_SP104");

    MilestoneObservation entered{
        .stageName = "F_SP104",
        .room = 1,
        .layer = 1,
        .point = 0,
    };
    tracker.observe(entered, 730, MilestoneNoTapeFrame);

    const auto result = nlohmann::json::parse(serialize_milestone_result(tracker));
    REQUIRE(result["schema"]["name"] == "dusklight.automation.milestones");
    REQUIRE(result["schema"]["version"] == 1);
    REQUIRE(result["goal"] == "exit-f-sp103-to-f-sp104");
    REQUIRE(result["goal_reached"] == true);
    REQUIRE(result["milestones"].size() == 3);
    REQUIRE(result["milestones"][0]["sim_tick"] == 500);
    REQUIRE(result["milestones"][1]["evidence"]["next_stage"]["name"] == "F_SP104");
    REQUIRE(result["milestones"][1]["evidence"]["player"]["process_id"] == 0x12345678);
    REQUIRE(result["milestones"][1]["evidence"]["player"]["position"][0] == 666.0f);
    REQUIRE(result["milestones"][1]["evidence"]["event"]["id"] == -1);
    REQUIRE(result["milestones"][1]["evidence"]["rng"]["streams"][0]["state"][0] == 11);
    REQUIRE(result["milestones"][1]["evidence"]["rng"]["streams"][1]["call_count"] == 200);
    REQUIRE(result["milestones"][1]["evidence"]["boundary_fingerprint"]["schema"] ==
            "dusklight.milestone-boundary/v1");
    REQUIRE(result["milestones"][1]["evidence"]["boundary_fingerprint"]["algorithm"] == "xxh3-128");
    REQUIRE(result["milestones"][1]["evidence"]["boundary_fingerprint"]["digest"]
                .get<std::string>()
                .size() == 32);
    REQUIRE(result["milestones"][2]["hit"] == true);
    REQUIRE(result["milestones"][2]["tape_frame"] == nullptr);
}

void testBoundaryFingerprintIsStableAndSensitiveToExplicitState() {
    using namespace dusk::automation;
    MilestoneTracker tracker;
    constexpr MilestoneId requested[]{MilestoneId::GameplayReadyFSp103};
    std::string error;
    REQUIRE(tracker.configure(requested, std::nullopt, error));
    tracker.observe(f_sp103(), 10, 10);
    const MilestoneEvidence baseline = tracker.hits()[0].evidence;
    const std::string digest = compute_milestone_boundary_fingerprint(baseline);
    REQUIRE(digest == baseline.boundaryFingerprint);
    REQUIRE(digest.size() == 32);
    REQUIRE(compute_milestone_boundary_fingerprint(baseline) == digest);

    MilestoneEvidence changed = baseline;
    changed.rng.streams[0].callCount++;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.playerPositionX =
        std::bit_cast<float>(std::bit_cast<std::uint32_t>(changed.playerPositionX) + 1);
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.playerShapeAngleY++;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.eventStatus++;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.nextStageEnabled = true;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
}

void testGoalMustBeRequested() {
    using namespace dusk::automation;
    MilestoneTracker tracker;
    constexpr MilestoneId requested[]{MilestoneId::GameplayReadyFSp103};
    std::string error;
    REQUIRE(!tracker.configure(requested, MilestoneId::EnteredFSp104, error));
    REQUIRE(error.find("was not requested") != std::string::npos);
}

}  // namespace

int main() {
    testRegistryAndParsing();
    testGameplayReadinessRequiresExactMemoryState();
    testExitRequiresCommittedExactDestination();
    testTrackerCapturesOnlyTheFirstHitAndSerializesEvidence();
    testBoundaryFingerprintIsStableAndSensitiveToExplicitState();
    testGoalMustBeRequested();
    std::cout << "milestone tests passed\n";
    return 0;
}
