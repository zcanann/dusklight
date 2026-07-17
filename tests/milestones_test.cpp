#include "dusk/automation/milestones.hpp"
#include "dusk/automation/input_recording.hpp"

#include <bit>
#include <array>
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

constexpr std::array<std::uint8_t, 252> IntroProgram{
0x44,0x4d,0x53,0x50,0x01,0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x02,0x00,0x00,0x00,0xc8,0x00,0x00,0x00,0x10,0xe0,0x59,0x2e,0xb0,0x01,0x14,0x5a,0xb8,0x60,0x27,0x1f,0x1d,0x66,0x77,0x89,0xaa,0xce,0x1e,0x1a,0xa1,0x44,0x2b,0xa7,0x50,0xb2,0x0b,0xd2,0x45,0x38,0x29,0xd2,0x4e,0x00,0x00,0x00,0x0c,0x00,0x70,0x72,0x6f,0x63,0x65,0x73,0x73,0x5f,0x62,0x6f,0x6f,0x74,0x00,0x00,0x01,0x00,0x07,0x00,0x16,0x00,0x00,0x00,0x45,0xdc,0x09,0x9d,0x98,0x3f,0xb5,0x8b,0x7a,0x85,0x6f,0x7e,0x83,0x10,0xe5,0xd2,0xec,0xba,0xc3,0xcd,0xb3,0xa4,0xd7,0xa4,0xc0,0xa2,0x9e,0x75,0x7d,0x3b,0xa5,0xca,0x01,0x01,0x15,0x04,0x62,0x6f,0x6f,0x74,0x20,0x01,0x02,0x12,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x20,0x31,0x72,0x00,0x00,0x00,0x0c,0x00,0x6c,0x69,0x6e,0x6b,0x5f,0x63,0x6f,0x6e,0x74,0x72,0x6f,0x6c,0x01,0x00,0x01,0x00,0x1c,0x00,0x3a,0x00,0x00,0x00,0x78,0x84,0x86,0x28,0x9a,0xa6,0x01,0x35,0x5d,0x3a,0x43,0x6b,0x58,0x25,0xd2,0xf2,0xbd,0xf3,0xe2,0xea,0x2d,0x77,0x04,0x8a,0xcf,0x5b,0x19,0xc0,0x1d,0x9d,0x56,0x15,0x01,0x04,0x15,0x07,0x46,0x5f,0x53,0x50,0x31,0x30,0x33,0x20,0x01,0x05,0x13,0x01,0x00,0x00,0x00,0x20,0x31,0x01,0x07,0x13,0x01,0x00,0x00,0x00,0x20,0x31,0x01,0x08,0x10,0x01,0x20,0x31,0x01,0x15,0x10,0x01,0x20,0x31,0x01,0x0e,0x10,0x01,0x20,0x30,0x31,0x01,0x0f,0x13,0xff,0xff,0xff,0xff,0x20,0x31};

bool noSymbols(dusk::automation::MilestoneProgramSymbolKind, std::string_view, std::uint32_t&) {
    return false;
}

struct BootStartOrder {
    std::vector<int> events;
    bool recording = false;
};

bool beginBootRecorder(void* context) {
    auto& order = *static_cast<BootStartOrder*>(context);
    order.events.push_back(1);
    order.recording = true;
    return true;
}

void releaseBootInput(void* context) {
    auto& order = *static_cast<BootStartOrder*>(context);
    REQUIRE(order.recording);
    order.events.push_back(2);
}

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
    observation.eventId = 2305;  // Pre-opening gap: Link exists but is not durably controllable.
    REQUIRE(!definition->predicate(observation));
    observation.eventId = -1;
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

void testAuthoredBootStableAndExactFirstHit() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(decode_milestone_program(IntroProgram, noSymbols, program) == MilestoneProgramError::None);
    REQUIRE(program.digest() == "10e0592eb001145ab860271f1d667789aace1e1aa1442ba750b20bd2453829d2");
    REQUIRE(program.find("process_boot")->definitionDigest ==
            "45dc099d983fb58b7a856f7e8310e5d2ecbac3cdb3a4d7a4c0a29e757d3ba5ca");

    MilestoneTracker bootTracker;
    const std::vector<std::string> bootNames{"process_boot"};
    std::string error;
    REQUIRE(bootTracker.configureNames(bootNames, std::string("process_boot"), program, error));
    MilestoneObservation empty;
    bootTracker.observeBoundary(empty, MilestoneProgramPhase::PreInput,
        MilestoneBoundaryKind::Boot, 0, 0, MilestoneNoTapeFrame);
    REQUIRE(bootTracker.goalReached());
    REQUIRE(bootTracker.authoredHits()[0].boundaryIndex == 0);
    REQUIRE(bootTracker.authoredHits()[0].tapeFrame == MilestoneNoTapeFrame);

    // Stable evaluation records the boundary on which the required run completes, never its start.
    auto* link = const_cast<MilestoneProgramDefinition*>(program.find("link_control"));
    link->stableTicks = 2;
    MilestoneTracker tracker;
    const std::vector<std::string> names{"link_control"};
    REQUIRE(tracker.configureNames(names, std::string("link_control"), program, error));
    MilestoneObservation observation = f_sp103();
    tracker.observeBoundary(observation, MilestoneProgramPhase::PostSim,
        MilestoneBoundaryKind::Tick, 11, 10, 7);
    REQUIRE(!tracker.goalReached());
    observation.eventRunning = true;
    tracker.observeBoundary(observation, MilestoneProgramPhase::PostSim,
        MilestoneBoundaryKind::Tick, 12, 11, 8);
    observation.eventRunning = false;
    tracker.observeBoundary(observation, MilestoneProgramPhase::PostSim,
        MilestoneBoundaryKind::Tick, 13, 12, 9);
    tracker.observeBoundary(observation, MilestoneProgramPhase::PostSim,
        MilestoneBoundaryKind::Tick, 14, 13, 10);
    REQUIRE(tracker.goalReached());
    REQUIRE(tracker.authoredHits()[0].boundaryIndex == 14);
    REQUIRE(tracker.authoredHits()[0].tapeFrame == 10);
    const auto result = nlohmann::json::parse(serialize_milestone_result(tracker));
    REQUIRE(result["program_digest"] == std::string(program.digest()));
    REQUIRE(result["milestones"][0]["definition_digest"] ==
            "788486289aa601355d3a436b5825d2f2bdf3e2ea2d77048acf5b19c01d9d5615");
}

void testMalformedAuthoredProgramIsRejected() {
    using namespace dusk::automation;
    auto corrupt = IntroProgram;
    corrupt.back() ^= 1;
    MilestoneProgram program;
    REQUIRE(decode_milestone_program(corrupt, noSymbols, program) ==
            MilestoneProgramError::InvalidProgramDigest);
    REQUIRE(program.empty());
}

void testBootRecordingGuardrailsAndBeginOrdering() {
    using namespace dusk::automation;
    BootRecordingCliRequest request{.enabled = true};
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingOutputTape);
    request.hasOutputTape = true;
    request.hasAutomationInput = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::HasAutomationInput);
    request.hasAutomationInput = false;
    request.headless = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::Headless);
    request.headless = false;
    request.unpaced = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::Unpaced);
    request.unpaced = false;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingMilestoneProgram);
    request.hasMilestoneProgram = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingMilestoneSelection);
    request.hasMilestoneSelection = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingMilestoneResult);
    request.hasMilestoneResult = true;
    request.hasMilestoneGoal = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::HasMilestoneGoal);
    request.hasMilestoneGoal = false;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingStartMilestone);
    request.hasStartMilestone = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::None);

    MilestoneProgram program;
    REQUIRE(decode_milestone_program(IntroProgram, noSymbols, program) == MilestoneProgramError::None);
    REQUIRE(validate_authored_boot_definition(program, "process_boot") ==
            BootRecordingError::None);
    REQUIRE(validate_authored_boot_definition(program, "link_control") ==
            BootRecordingError::StartMilestoneNotPreInput);

    MilestoneTracker tracker;
    const std::vector<std::string> requested{"process_boot"};
    std::string error;
    REQUIRE(tracker.configureNames(requested, std::nullopt, program, error));
    BootStartOrder order;
    BootRecordingBinding binding;
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot", "",
                beginBootRecorder, releaseBootInput, &order, binding) ==
            BootRecordingError::StartMilestoneNotHit);
    REQUIRE(order.events.empty());

    tracker.observeBoundary({}, MilestoneProgramPhase::PreInput,
        MilestoneBoundaryKind::Boot, 0, 0, MilestoneNoTapeFrame);
    const std::string fingerprint = tracker.authoredHits()[0].evidence.boundaryFingerprint;
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot",
                "00000000000000000000000000000000", beginBootRecorder, releaseBootInput,
                &order, binding) == BootRecordingError::FingerprintMismatch);
    REQUIRE(order.events.empty());
    auto& mutableHit = const_cast<AuthoredMilestoneHit&>(tracker.authoredHits()[0]);
    mutableHit.boundaryIndex = 1;
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot", fingerprint,
                beginBootRecorder, releaseBootInput, &order, binding) ==
            BootRecordingError::InvalidBootBoundary);
    mutableHit.boundaryIndex = 0;
    mutableHit.programDigest[0] = mutableHit.programDigest[0] == '0' ? '1' : '0';
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot", fingerprint,
                beginBootRecorder, releaseBootInput, &order, binding) ==
            BootRecordingError::StaleProgram);
    mutableHit.programDigest = std::string(program.digest());
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot", fingerprint,
                beginBootRecorder, releaseBootInput, &order, binding) ==
            BootRecordingError::None);
    // The actual first PAD read happens after this gate returns.
    REQUIRE(order.recording);
    order.events.push_back(3);
    REQUIRE(order.events == std::vector<int>({1, 2, 3}));
    REQUIRE(binding.milestone == "process_boot");
    REQUIRE(binding.boundaryIndex == 0);
    REQUIRE(binding.boundaryFingerprint == fingerprint);
    REQUIRE(binding.programDigest == program.digest());
    REQUIRE(binding.definitionDigest == program.find("process_boot")->definitionDigest);
}

void testAcceleratedParentRecordingBoundaryAndRevealOrdering() {
    using namespace dusk::automation;
    REQUIRE(validate_fast_forward_boundary(9, 10, false, true) ==
            FastForwardBoundaryError::None);
    REQUIRE(validate_fast_forward_boundary(10, 10, false, true) ==
            FastForwardBoundaryError::TapeEndRequiresRecording);
    REQUIRE(validate_fast_forward_boundary(10, 10, true, false) ==
            FastForwardBoundaryError::TapeEndRequiresRelease);
    REQUIRE(validate_fast_forward_boundary(11, 10, true, true) ==
            FastForwardBoundaryError::PastTapeEnd);
    REQUIRE(validate_fast_forward_boundary(10, 10, true, true) ==
            FastForwardBoundaryError::None);
    const ParentRecordingBoundary boundary = exact_parent_recording_boundary(10);
    REQUIRE(boundary.boundaryIndex == 10);
    REQUIRE(boundary.tapeFrame == 9);

    // Exact tape-end reveal is forbidden until the normal handoff has both marked the boundary
    // reached and begun the armed recorder. The first live PAD read follows reveal.
    std::vector<int> order;
    bool handoffReached = false;
    bool recorderRecording = false;
    REQUIRE(!accelerated_recording_reveal_ready(true, handoffReached, recorderRecording));
    recorderRecording = true;
    order.push_back(1);  // recorder.begin()
    REQUIRE(!accelerated_recording_reveal_ready(true, handoffReached, recorderRecording));
    handoffReached = true;
    order.push_back(2);  // live-input handoff/quarantine release
    REQUIRE(accelerated_recording_reveal_ready(true, handoffReached, recorderRecording));
    order.push_back(3);  // window reveal after hidden parent frame submission
    order.push_back(4);  // first visible live PAD read
    REQUIRE(order == std::vector<int>({1, 2, 3, 4}));
    REQUIRE(accelerated_recording_reveal_ready(false, false, false));
}

void testAuthoredParentRecordingStartBinding() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(
        decode_milestone_program(IntroProgram, noSymbols, program) == MilestoneProgramError::None);
    MilestoneTracker tracker;
    const std::vector<std::string> requested{"link_control"};
    std::string error;
    REQUIRE(tracker.configureNames(requested, std::nullopt, program, error));

    tracker.observeBoundary(
        f_sp103(), MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 440, 439, 439);
    REQUIRE(tracker.authoredHits()[0].hit);
    const std::string fingerprint = tracker.authoredHits()[0].evidence.boundaryFingerprint;
    RecordingStartBinding binding;
    REQUIRE(bind_recording_start(tracker, program, "link_control", fingerprint, 438, binding) ==
            RecordingStartError::WrongTapeFrame);
    REQUIRE(
        bind_recording_start(tracker, program, "link_control", "00000000000000000000000000000000",
            439, binding) == RecordingStartError::FingerprintMismatch);
    REQUIRE(bind_recording_start(tracker, program, "link_control", fingerprint, 439, binding) ==
            RecordingStartError::None);
    REQUIRE(binding.milestone == "link_control");
    REQUIRE(binding.tapeFrame == 439);
    REQUIRE(binding.boundaryIndex == 440);
    REQUIRE(binding.boundaryFingerprint == fingerprint);
    REQUIRE(binding.programDigest == program.digest());
    REQUIRE(binding.definitionDigest == program.find("link_control")->definitionDigest);
}

}  // namespace

int main() {
    testRegistryAndParsing();
    testGameplayReadinessRequiresExactMemoryState();
    testExitRequiresCommittedExactDestination();
    testTrackerCapturesOnlyTheFirstHitAndSerializesEvidence();
    testBoundaryFingerprintIsStableAndSensitiveToExplicitState();
    testGoalMustBeRequested();
    testAuthoredBootStableAndExactFirstHit();
    testMalformedAuthoredProgramIsRejected();
    testBootRecordingGuardrailsAndBeginOrdering();
    testAcceleratedParentRecordingBoundaryAndRevealOrdering();
    testAuthoredParentRecordingStartBinding();
    std::cout << "milestone tests passed\n";
    return 0;
}
