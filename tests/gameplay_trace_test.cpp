#include "dusk/automation/gameplay_trace.hpp"

#include <bit>
#include <chrono>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

namespace {

void require(bool condition, const char* expression, int line) {
    if (!condition) {
        std::cerr << "gameplay_trace_test.cpp:" << line << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

template <typename T>
T readLittle(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    using U = std::make_unsigned_t<T>;
    U value = 0;
    for (std::size_t index = 0; index < sizeof(T); ++index) {
        value |= static_cast<U>(bytes.at(offset + index)) << (index * 8);
    }
    return static_cast<T>(value);
}

float readFloat(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    return std::bit_cast<float>(readLittle<std::uint32_t>(bytes, offset));
}

std::vector<std::uint8_t> readFile(const std::filesystem::path& path) {
    std::ifstream stream(path, std::ios::binary);
    REQUIRE(stream.good());
    return {std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>()};
}

dusk::automation::GameplayTraceSample sample(std::uint64_t simulationTick) {
    using namespace dusk::automation;
    GameplayTraceSample value;
    value.core.boundaryIndex = simulationTick + 1;
    value.core.simulationTick = simulationTick;
    value.core.tapeFrame = simulationTick;
    value.core.flags = GameplayTraceSimulationTickValid | GameplayTraceTapeFrameValid;
    value.core.inputSource = GameplayTraceInputTape;
    value.stageStatus = GameplayTraceChannelStatus::Present;
    value.stage.stageName = {'F', '_', 'S', 'P', '1', '0', '3', '\0'};
    value.stage.room = 1;
    value.stage.layer = 3;
    value.stage.point = 7;
    value.cameraStatus = GameplayTraceChannelStatus::Absent;
    return value;
}

void testChannelParser() {
    using namespace dusk::automation;
    std::uint64_t channels = 0;
    std::string error;
    REQUIRE(parse_gameplay_trace_channels("core, stage, camera", channels, error));
    REQUIRE(channels == (gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
                            gameplay_trace_channel_bit(GameplayTraceChannel::Stage) |
                            gameplay_trace_channel_bit(GameplayTraceChannel::Camera)));
    REQUIRE(!parse_gameplay_trace_channels("stage", channels, error));
    REQUIRE(!parse_gameplay_trace_channels("core,core", channels, error));
    REQUIRE(!parse_gameplay_trace_channels("core,collision", channels, error));
    REQUIRE(parse_gameplay_trace_channels(
        "core,scene-exit,player-background-collision", channels, error));
    REQUIRE(channels ==
            (gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
                gameplay_trace_channel_bit(GameplayTraceChannel::SceneExit) |
                gameplay_trace_channel_bit(GameplayTraceChannel::PlayerBackgroundCollision)));
    REQUIRE(parse_gameplay_trace_channels("core,stage,player-collision-surfaces", channels, error));
    REQUIRE((channels &
                gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces)) != 0);
    REQUIRE(!parse_gameplay_trace_channels("core,player-collision-surfaces", channels, error));
    REQUIRE(parse_gameplay_trace_channels("all", channels, error));
    REQUIRE(channels == GameplayTraceKnownChannels);
    REQUIRE((GameplayTraceDefaultChannels &
                gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces)) == 0);
    REQUIRE((GameplayTraceDefaultChannels &
                gameplay_trace_channel_bit(GameplayTraceChannel::GoalProgress)) != 0);
    REQUIRE((GameplayTraceDefaultChannels &
                gameplay_trace_channel_bit(GameplayTraceChannel::SelectedActors)) == 0);
    std::uint32_t triggers = 0;
    REQUIRE(parse_gameplay_trace_retention_triggers(
        "crash, contact, flag, predicate", triggers, error));
    REQUIRE(triggers == GameplayTraceKnownRetentionTriggers);
    REQUIRE(!parse_gameplay_trace_retention_triggers("contact,contact", triggers, error));
    REQUIRE(!parse_gameplay_trace_retention_triggers("visual", triggers, error));
}

void testGoalAndSelectedActorLayout(const std::filesystem::path& path) {
    using namespace dusk::automation;
    constexpr std::uint64_t channels = gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
                                       gameplay_trace_channel_bit(GameplayTraceChannel::GoalProgress) |
                                       gameplay_trace_channel_bit(GameplayTraceChannel::SelectedActors);
    GameplayTraceRecorder recorder;
    recorder.start(1, channels);
    GameplayTraceSample value = sample(12);
    value.stageStatus = GameplayTraceChannelStatus::NotSampled;
    value.cameraStatus = GameplayTraceChannelStatus::NotSampled;
    value.goalProgressStatus = GameplayTraceChannelStatus::Present;
    value.goalProgress.flags = GameplayTraceGoalConfigured | GameplayTraceGoalReached |
                               GameplayTraceGoalAuthored |
                               GameplayTraceGoalFirstHitTickPresent;
    value.goalProgress.goalNameHash = 0x12345678;
    value.goalProgress.requestedCount = 3;
    value.goalProgress.hitCount = 2;
    value.goalProgress.stableTicks = 3;
    value.goalProgress.consecutiveTicks = 3;
    value.goalProgress.sequenceSteps = 2;
    value.goalProgress.sequenceNextStep = 2;
    value.goalProgress.sequenceWithinTicks = 30;
    value.goalProgress.sequenceElapsedTicks = 7;
    value.goalProgress.firstHitTick = 12;
    value.selectedActorsStatus = GameplayTraceChannelStatus::Present;
    value.selectedActors.count = 1;
    value.selectedActors.observedCount = 2;
    value.selectedActors.flags = GameplayTraceSelectedActorsTruncated;
    value.selectedActors.actors[0] = {
        .sessionProcessId = 10,
        .actorName = 77,
        .setId = 4,
        .homeRoom = 1,
        .currentRoom = 2,
        .health = 9,
        .status = 0x1234,
        .position = {1.0f, 2.0f, 3.0f},
        .currentAngle = {4, 5, 6},
        .shapeAngle = {7, 8, 9},
    };
    recorder.record(value);
    recorder.stop();

    std::string error;
    REQUIRE(write_gameplay_trace(path, recorder, error));
    const auto bytes = readFile(path);
    REQUIRE(bytes.size() == 1043);
    REQUIRE(readLittle<std::uint16_t>(bytes, 192) == 11);
    REQUIRE(readLittle<std::uint32_t>(bytes, 200) == 32);
    REQUIRE(readLittle<std::uint16_t>(bytes, 256) == 12);
    REQUIRE(readLittle<std::uint32_t>(bytes, 264) == 656);
    constexpr std::size_t goal = 354;
    constexpr std::size_t actors = 387;
    REQUIRE(readLittle<std::uint32_t>(bytes, goal) == value.goalProgress.flags);
    REQUIRE(readLittle<std::uint64_t>(bytes, goal + 24) == 12);
    REQUIRE(readLittle<std::uint16_t>(bytes, actors) == 1);
    REQUIRE(readLittle<std::uint32_t>(bytes, actors + 8) == 2);
    REQUIRE(readLittle<std::uint32_t>(bytes, actors + 16) == 10);
    REQUIRE(readFloat(bytes, actors + 32) == 1.0f);
    REQUIRE(readLittle<std::uint32_t>(bytes, actors + 56) == 0xffffffffu);
}

void testCollisionSurfaceLayout(const std::filesystem::path& path) {
    using namespace dusk::automation;
    constexpr std::uint64_t channels =
        gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
        gameplay_trace_channel_bit(GameplayTraceChannel::Stage) |
        gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces);
    GameplayTraceRecorder recorder;
    recorder.start(1, channels);
    GameplayTraceSample value = sample(9);
    value.cameraStatus = GameplayTraceChannelStatus::NotSampled;
    value.playerCollisionSurfacesStatus = GameplayTraceChannelStatus::Present;
    recorder.record(value);
    recorder.stop();

    std::string error;
    REQUIRE(write_gameplay_trace(path, recorder, error));
    const auto bytes = readFile(path);
    REQUIRE(bytes.size() == 883);
    REQUIRE(readLittle<std::uint16_t>(bytes, 256) == 10);
    REQUIRE(readLittle<std::uint16_t>(bytes, 258) == 1);
    REQUIRE(readLittle<std::uint32_t>(bytes, 264) == 496);
    REQUIRE(readLittle<std::uint64_t>(bytes, 272) == 386);
    REQUIRE(readLittle<std::uint64_t>(bytes, 288) == 387);
    REQUIRE(bytes[386] == static_cast<std::uint8_t>(GameplayTraceChannelStatus::Present));
    constexpr std::size_t payload = 387;
    REQUIRE(readLittle<std::uint32_t>(bytes, payload) == 0);
    REQUIRE(readLittle<std::int8_t>(bytes, payload + 4) == -128);
    REQUIRE(readLittle<std::uint16_t>(bytes, payload + 8) == 0x003f);
    REQUIRE(bytes[payload + 16 + 4] == GameplayTraceCollisionSurfaceGround);
    REQUIRE(bytes[payload + 16 + 80 + 4] == GameplayTraceCollisionSurfaceRoof);
    REQUIRE(bytes[payload + 16 + 160 + 4] == GameplayTraceCollisionSurfaceWater);
    REQUIRE(bytes[payload + 16 + 240 + 4] == GameplayTraceCollisionSurfaceWall);
    REQUIRE(bytes[payload + 16 + 240 + 5] == 0);
    REQUIRE(bytes[payload + 16 + 400 + 5] == 2);
    REQUIRE(readLittle<std::uint16_t>(bytes, payload + 16 + 8) == 0xffff);
}

void testSceneExitAndBackgroundCollisionLayout(const std::filesystem::path& path) {
    using namespace dusk::automation;
    constexpr std::uint64_t channels =
        gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
        gameplay_trace_channel_bit(GameplayTraceChannel::SceneExit) |
        gameplay_trace_channel_bit(GameplayTraceChannel::PlayerBackgroundCollision);
    GameplayTraceRecorder recorder;
    recorder.start(1, channels);
    GameplayTraceSample value = sample(7);
    value.stageStatus = GameplayTraceChannelStatus::NotSampled;
    value.cameraStatus = GameplayTraceChannelStatus::NotSampled;

    value.sceneExitStatus = GameplayTraceChannelStatus::Present;
    auto& exit = value.sceneExit;
    exit.sessionProcessId = 0x10203040;
    exit.rawParameters = 0xa1b2c3d4;
    exit.flags = GameplayTraceSceneExitVolumeValid | GameplayTraceSceneExitPlayerInside |
                 GameplayTraceSceneExitPlayerLatched | GameplayTraceSceneExitChangeOk |
                 GameplayTraceSceneExitChangeStarted | GameplayTraceSceneExitDestinationValid;
    exit.signedDistanceToVolume = -2.5f;
    exit.actorName = -321;
    exit.setId = 0x4567;
    exit.exitId = 0xd4;
    exit.pathId = 0xb2;
    exit.argument1 = 0xc3;
    exit.switchNo = 0xa1;
    exit.kind = GameplayTraceSceneExitBox;
    exit.observedCount = 3;
    exit.homeRoom = -1;
    exit.linkExitDirection = 0xff;
    exit.linkExitId = 0x8012;
    exit.shapeYaw = -1234;
    exit.actorAction = 0xff;
    exit.playerLocalPosition = {1.0f, 2.0f, 3.0f};
    exit.volumeExtent = {4.0f, 5.0f, 6.0f};
    exit.homePosition = {7.0f, 8.0f, 9.0f};
    exit.destinationStage = {'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H'};
    exit.destinationRoom = 2;
    exit.destinationLayer = -1;
    exit.destinationPoint = 26;
    // SCLS wipe is an arbitrary u8 narrowed to the native signed wipe field;
    // only raw 15 is remapped to zero by dStage_changeScene.
    exit.destinationWipe = 17;
    exit.destinationWipeTime = 3;
    exit.destinationTimeHour = 12;

    value.playerBackgroundCollisionStatus = GameplayTraceChannelStatus::Present;
    auto& collision = value.playerBackgroundCollision;
    collision.flags = 0x7ffff;
    collision.groundHeight = 10.0f;
    collision.roofHeight = 20.0f;
    collision.waterHeight = 30.0f;
    collision.groundBgIndex = 1;
    collision.groundPolyIndex = 2;
    collision.groundOwnerSessionProcessId = 3;
    collision.groundPlane = {0.0f, 1.0f, 0.0f, -10.0f};
    collision.roofBgIndex = 4;
    collision.roofPolyIndex = 5;
    collision.roofOwnerSessionProcessId = 6;
    collision.waterBgIndex = 7;
    collision.waterPolyIndex = 8;
    collision.waterOwnerSessionProcessId = 9;
    collision.walls[0] = {10, 11, 12, -13,
        GameplayTraceCollisionWallHit | GameplayTraceCollisionWallOwnerPresent |
            GameplayTraceCollisionWallIdentityPresent};
    collision.walls[1] = {14, 15, 0xffffffffu, 16,
        GameplayTraceCollisionWallHit | GameplayTraceCollisionWallIdentityPresent};
    collision.oldPosition = {21.0f, 22.0f, 23.0f};
    collision.resolvedFrameDisplacement = {1.0f, -2.0f, 3.0f};
    collision.finalPosition = {22.0f, 20.0f, 26.0f};
    recorder.record(value);
    recorder.stop();

    std::string error;
    REQUIRE(write_gameplay_trace(path, recorder, error));
    const auto bytes = readFile(path);
    REQUIRE(bytes.size() == 571);

    REQUIRE(readLittle<std::uint16_t>(bytes, 192) == 5);
    REQUIRE(readLittle<std::uint16_t>(bytes, 194) == 2);
    REQUIRE(readLittle<std::uint32_t>(bytes, 200) == 88);
    REQUIRE(readLittle<std::uint64_t>(bytes, 208) == 353);
    REQUIRE(readLittle<std::uint64_t>(bytes, 224) == 354);
    REQUIRE(bytes[353] == static_cast<std::uint8_t>(GameplayTraceChannelStatus::Present));
    constexpr std::size_t scene = 354;
    REQUIRE(readLittle<std::uint32_t>(bytes, scene) == 0x10203040);
    REQUIRE(readLittle<std::uint32_t>(bytes, scene + 4) == 0xa1b2c3d4);
    REQUIRE(readLittle<std::uint32_t>(bytes, scene + 8) == 0x3f);
    REQUIRE(readFloat(bytes, scene + 12) == -2.5f);
    REQUIRE(readLittle<std::int16_t>(bytes, scene + 16) == -321);
    REQUIRE(bytes[scene + 24] == GameplayTraceSceneExitBox);
    REQUIRE(bytes[scene + 33] == 0 && bytes[scene + 34] == 0 && bytes[scene + 35] == 0);
    REQUIRE(readFloat(bytes, scene + 36) == 1.0f);
    REQUIRE(readFloat(bytes, scene + 68) == 9.0f);
    REQUIRE(std::string(bytes.begin() + scene + 72, bytes.begin() + scene + 80) == "ABCDEFGH");
    REQUIRE(readLittle<std::int16_t>(bytes, scene + 82) == 26);
    REQUIRE(bytes[scene + 87] == 0);

    REQUIRE(readLittle<std::uint16_t>(bytes, 256) == 9);
    REQUIRE(readLittle<std::uint16_t>(bytes, 258) == 1);
    REQUIRE(readLittle<std::uint32_t>(bytes, 264) == 128);
    REQUIRE(readLittle<std::uint64_t>(bytes, 272) == 442);
    REQUIRE(readLittle<std::uint64_t>(bytes, 288) == 443);
    REQUIRE(bytes[442] == static_cast<std::uint8_t>(GameplayTraceChannelStatus::Present));
    constexpr std::size_t collisionOffset = 443;
    REQUIRE(readLittle<std::uint32_t>(bytes, collisionOffset) == 0x7ffff);
    REQUIRE(readFloat(bytes, collisionOffset + 4) == 10.0f);
    REQUIRE(readLittle<std::uint16_t>(bytes, collisionOffset + 16) == 1);
    REQUIRE(readFloat(bytes, collisionOffset + 28) == 1.0f);
    REQUIRE(readLittle<std::uint16_t>(bytes, collisionOffset + 40) == 4);
    REQUIRE(readLittle<std::uint32_t>(bytes, collisionOffset + 52) == 9);
    REQUIRE(readLittle<std::uint16_t>(bytes, collisionOffset + 56) == 10);
    REQUIRE(readLittle<std::int16_t>(bytes, collisionOffset + 64) == -13);
    REQUIRE(readLittle<std::uint16_t>(bytes, collisionOffset + 66) == 7);
    REQUIRE(readFloat(bytes, collisionOffset + 92) == 21.0f);
    REQUIRE(readFloat(bytes, collisionOffset + 104) == 1.0f);
    REQUIRE(readFloat(bytes, collisionOffset + 124) == 26.0f);
}

void testExactV2Layout(const std::filesystem::path& path) {
    using namespace dusk::automation;
    constexpr std::uint64_t channels = gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
                                       gameplay_trace_channel_bit(GameplayTraceChannel::Stage) |
                                       gameplay_trace_channel_bit(GameplayTraceChannel::Camera);

    GameplayTraceRecorder recorder;
    recorder.start(2, channels,
        TapeBoot{.kind = TapeBootKind::Stage,
            .stage = "F_SP103",
            .room = 1,
            .point = 1,
            .layer = 3,
            .saveSlot = 2});
    recorder.record(sample(0));
    GameplayTraceSample second = sample(1);
    second.core.tapeFrame = GameplayTraceNoTapeFrame;
    second.core.flags = GameplayTraceSimulationTickValid;
    second.core.inputSource = GameplayTraceInputNone;
    second.stage.nextStageName = {'F', '_', 'S', 'P', '1', '0', '4', '\0'};
    second.stage.nextRoom = 0;
    second.stage.nextLayer = 0;
    second.stage.nextPoint = 26;
    second.stage.flags = GameplayTraceNextStageEnabled;
    second.cameraStatus = GameplayTraceChannelStatus::Present;
    second.camera.viewYaw = -123;
    second.camera.controlledYaw = 456;
    second.camera.bank = -7;
    second.camera.eye = {1.0f, 2.0f, 3.0f};
    second.camera.center = {4.0f, 5.0f, 6.0f};
    second.camera.up = {0.0f, 1.0f, 0.0f};
    second.camera.fovy = 45.0f;
    recorder.record(second);
    recorder.stop();

    std::string error;
    REQUIRE(write_gameplay_trace(path, recorder, error));
    const auto bytes = readFile(path);
    REQUIRE(bytes.size() == 550);
    REQUIRE(std::string(bytes.begin(), bytes.begin() + 8) == "DUSKTRCE");
    REQUIRE(readLittle<std::uint16_t>(bytes, 8) == GameplayTraceVersion);
    REQUIRE(readLittle<std::uint16_t>(bytes, 10) == GameplayTraceHeaderSize);
    REQUIRE(readLittle<std::uint32_t>(bytes, 12) == 30);
    REQUIRE(readLittle<std::uint32_t>(bytes, 16) == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 20) == 2);
    REQUIRE(readLittle<std::uint32_t>(bytes, 28) == 1);
    REQUIRE(readLittle<std::uint16_t>(bytes, 32) == 3);
    REQUIRE(readLittle<std::uint64_t>(bytes, 36) == GameplayTraceHeaderSize);
    REQUIRE(readLittle<std::uint64_t>(bytes, 44) == 320);
    REQUIRE(readLittle<std::uint64_t>(bytes, 52) == channels);
    REQUIRE(bytes[64] == static_cast<std::uint8_t>(TapeBootKind::Stage));
    REQUIRE(bytes[65] == 2);
    REQUIRE(readLittle<std::int8_t>(bytes, 66) == 1);
    REQUIRE(readLittle<std::int8_t>(bytes, 67) == 3);
    REQUIRE(readLittle<std::int16_t>(bytes, 68) == 1);
    REQUIRE(bytes[70] == 7);
    REQUIRE(std::string(bytes.begin() + 72, bytes.begin() + 79) == "F_SP103");
    REQUIRE(std::all_of(bytes.begin() + 79, bytes.begin() + 120,
        [](const std::uint8_t byte) { return byte == 0; }));
    REQUIRE(readLittle<std::uint64_t>(bytes, 120) == 2);

    // Core descriptor and columns.
    REQUIRE(readLittle<std::uint16_t>(bytes, 128) == 0);
    REQUIRE(readLittle<std::uint32_t>(bytes, 132) == 3);
    REQUIRE(readLittle<std::uint32_t>(bytes, 136) == 32);
    REQUIRE(readLittle<std::uint64_t>(bytes, 144) == 320);
    REQUIRE(readLittle<std::uint64_t>(bytes, 160) == 322);
    REQUIRE(bytes[320] == 1 && bytes[321] == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 322) == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 354) == 2);
    REQUIRE(readLittle<std::uint64_t>(bytes, 362) == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 370) == GameplayTraceNoTapeFrame);

    // Stage descriptor and its explicit transition state.
    REQUIRE(readLittle<std::uint16_t>(bytes, 192) == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 208) == 386);
    REQUIRE(readLittle<std::uint64_t>(bytes, 224) == 388);
    REQUIRE(bytes[386] == 1 && bytes[387] == 1);
    REQUIRE(std::string(bytes.begin() + 432, bytes.begin() + 439) == "F_SP104");
    REQUIRE(readLittle<std::int16_t>(bytes, 442) == 26);
    REQUIRE(readLittle<std::uint32_t>(bytes, 444) == GameplayTraceNextStageEnabled);

    // Camera is explicitly absent at boundary one and present with exact bits at two.
    REQUIRE(readLittle<std::uint16_t>(bytes, 256) == 7);
    REQUIRE(readLittle<std::uint64_t>(bytes, 272) == 452);
    REQUIRE(readLittle<std::uint64_t>(bytes, 288) == 454);
    REQUIRE(bytes[452] == 2 && bytes[453] == 1);
    REQUIRE(readLittle<std::int16_t>(bytes, 502) == -123);
    REQUIRE(readLittle<std::int16_t>(bytes, 504) == 456);
    REQUIRE(readLittle<std::int16_t>(bytes, 506) == -7);
    REQUIRE(readFloat(bytes, 510) == 1.0f);
    REQUIRE(readFloat(bytes, 546) == 45.0f);

    REQUIRE(!write_gameplay_trace(path, recorder, error));
    REQUIRE(error == "gameplay trace output already exists");
}

void testScenarioFixtureIsAuthenticatedInV4(const std::filesystem::path& path) {
    using namespace dusk::automation;
    ScenarioFixture fixture;
    fixture.name = "low-health wolf";
    fixture.form = PlayerFixtureForm::Wolf;
    fixture.health = HealthFixture{4, 20};
    TapeBoot boot;
    boot.kind = TapeBootKind::Stage;
    boot.stage = "F_SP103";
    boot.room = 1;
    boot.point = 1;
    boot.layer = 3;
    boot.fixture = fixture;

    GameplayTraceRecorder recorder;
    recorder.start(1, gameplay_trace_channel_bit(GameplayTraceChannel::Core), boot);
    GameplayTraceSample value = sample(0);
    value.stageStatus = GameplayTraceChannelStatus::NotSampled;
    value.cameraStatus = GameplayTraceChannelStatus::NotSampled;
    recorder.record(value);
    recorder.stop();
    std::string error;
    const bool wrote = write_gameplay_trace(path, recorder, error);
    if (!wrote) std::cerr << "fixture trace error: " << error << '\n';
    REQUIRE(wrote);
    const auto bytes = readFile(path);
    REQUIRE(readLittle<std::uint16_t>(bytes, 8) == GameplayTraceVersion);
    const std::size_t fixtureOffset = readLittle<std::uint64_t>(bytes, 88);
    const std::size_t fixtureSize = readLittle<std::uint32_t>(bytes, 96);
    REQUIRE(fixtureOffset == 225);
    REQUIRE(fixtureOffset + fixtureSize == bytes.size());
    ScenarioFixture decoded;
    REQUIRE(decode_scenario_fixture(
                std::span<const std::uint8_t>(bytes.data() + fixtureOffset, fixtureSize), decoded) ==
            ScenarioFixtureError::None);
    REQUIRE(decoded == fixture);
}

void testValidationAndCapacity(
    const std::filesystem::path& invalidPath, const std::filesystem::path& exhaustedPath) {
    using namespace dusk::automation;
    constexpr std::uint64_t core = gameplay_trace_channel_bit(GameplayTraceChannel::Core);
    GameplayTraceRecorder invalid;
    invalid.start(1, core);
    GameplayTraceSample invalidSample = sample(0);
    invalid.record(invalidSample);
    invalid.stop();
    std::string error;
    REQUIRE(!write_gameplay_trace(invalidPath, invalid, error));
    REQUIRE(!std::filesystem::exists(invalidPath));

    GameplayTraceRecorder exhausted;
    exhausted.start(1, core);
    GameplayTraceSample coreSample = sample(0);
    coreSample.stageStatus = GameplayTraceChannelStatus::NotSampled;
    coreSample.cameraStatus = GameplayTraceChannelStatus::NotSampled;
    exhausted.record(coreSample);
    exhausted.record(coreSample);
    REQUIRE(exhausted.capacityExhausted());
    REQUIRE(write_gameplay_trace(exhaustedPath, exhausted, error));
    REQUIRE((readLittle<std::uint32_t>(readFile(exhaustedPath), 28) & 2) != 0);
}

void testTriggeredRetention(const std::filesystem::path& path) {
    using namespace dusk::automation;
    constexpr std::uint64_t channels = gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
                                       gameplay_trace_channel_bit(GameplayTraceChannel::GoalProgress);
    GameplayTraceRecorder recorder;
    recorder.start(8, channels, {},
        {.preTriggerTicks = 2,
            .postTriggerTicks = 1,
            .triggers = GameplayTraceTriggerPredicateHit});
    for (std::uint64_t tick = 0; tick < 6; ++tick) {
        auto value = sample(tick);
        value.stageStatus = GameplayTraceChannelStatus::NotSampled;
        value.cameraStatus = GameplayTraceChannelStatus::NotSampled;
        value.goalProgressStatus = GameplayTraceChannelStatus::Present;
        value.goalProgress.flags = GameplayTraceGoalConfigured | GameplayTraceGoalAuthored;
        value.goalProgress.goalNameHash = 0x12345678;
        value.goalProgress.requestedCount = 1;
        if (tick >= 3) {
            value.goalProgress.flags |=
                GameplayTraceGoalReached | GameplayTraceGoalFirstHitTickPresent;
            value.goalProgress.hitCount = 1;
            value.goalProgress.firstHitTick = 3;
        }
        recorder.record(value);
    }
    recorder.stop();
    REQUIRE(recorder.samples().size() == 4);
    REQUIRE(recorder.samples()[0].core.simulationTick == 1);
    REQUIRE(recorder.samples()[1].core.simulationTick == 2);
    REQUIRE(recorder.samples()[2].core.simulationTick == 3);
    REQUIRE(recorder.samples()[3].core.simulationTick == 4);
    REQUIRE(recorder.observedTriggers() == GameplayTraceTriggerPredicateHit);
    REQUIRE(recorder.triggerCount() == 1);
    REQUIRE(recorder.observedSampleCount() == 6);
    std::string error;
    REQUIRE(write_gameplay_trace(path, recorder, error));
    const auto bytes = readFile(path);
    REQUIRE((readLittle<std::uint32_t>(bytes, 28) & 4) != 0);
    REQUIRE(readLittle<std::uint32_t>(bytes, 100) == GameplayTraceTriggerPredicateHit);
    REQUIRE(readLittle<std::uint32_t>(bytes, 104) == GameplayTraceTriggerPredicateHit);
    REQUIRE(readLittle<std::uint32_t>(bytes, 108) == 2);
    REQUIRE(readLittle<std::uint32_t>(bytes, 112) == 1);
    REQUIRE(readLittle<std::uint32_t>(bytes, 116) == 1);
    REQUIRE(readLittle<std::uint64_t>(bytes, 120) == 6);

    GameplayTraceRecorder crash;
    crash.start(4, gameplay_trace_channel_bit(GameplayTraceChannel::Core), {},
        {.preTriggerTicks = 2, .postTriggerTicks = 0, .triggers = GameplayTraceTriggerCrash});
    for (std::uint64_t tick = 0; tick < 3; ++tick) {
        auto value = sample(tick);
        value.stageStatus = GameplayTraceChannelStatus::NotSampled;
        value.cameraStatus = GameplayTraceChannelStatus::NotSampled;
        crash.record(value);
    }
    crash.trigger(GameplayTraceTriggerCrash);
    crash.stop();
    REQUIRE(crash.samples().size() == 2);
    REQUIRE(crash.samples()[0].core.simulationTick == 1);
    REQUIRE(crash.samples()[1].core.simulationTick == 2);
    REQUIRE(crash.observedTriggers() == GameplayTraceTriggerCrash);

    GameplayTraceRecorder contact;
    contact.start(3, gameplay_trace_channel_bit(GameplayTraceChannel::Core), {},
        {.preTriggerTicks = 1,
            .postTriggerTicks = 0,
            .triggers = GameplayTraceTriggerNovelContact});
    auto beforeContact = sample(0);
    beforeContact.playerBackgroundCollisionStatus = GameplayTraceChannelStatus::Present;
    contact.record(beforeContact);
    auto firstContact = sample(1);
    firstContact.playerBackgroundCollisionStatus = GameplayTraceChannelStatus::Present;
    firstContact.playerBackgroundCollision.flags = GameplayTraceCollisionWallContact;
    contact.record(firstContact);
    REQUIRE(contact.samples().size() == 2);
    REQUIRE(contact.observedTriggers() == GameplayTraceTriggerNovelContact);

    GameplayTraceRecorder flag;
    flag.start(3, gameplay_trace_channel_bit(GameplayTraceChannel::Core), {},
        {.preTriggerTicks = 1,
            .postTriggerTicks = 0,
            .triggers = GameplayTraceTriggerFlagChange});
    flag.record(sample(0));
    auto changed = sample(1);
    changed.event.flags = GameplayTraceEventRunning;
    flag.record(changed);
    REQUIRE(flag.samples().size() == 2);
    REQUIRE(flag.observedTriggers() == GameplayTraceTriggerFlagChange);
}

void testNewChannelValidation(const std::filesystem::path& directory) {
    using namespace dusk::automation;
    const auto reject = [&directory](GameplayTraceSample value, const std::uint64_t channels,
                            const char* name) {
        GameplayTraceRecorder recorder;
        recorder.start(1, channels);
        recorder.record(value);
        recorder.stop();
        std::string error;
        const auto path = directory / name;
        REQUIRE(!write_gameplay_trace(path, recorder, error));
        REQUIRE(!std::filesystem::exists(path));
    };

    constexpr std::uint64_t sceneChannels =
        gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
        gameplay_trace_channel_bit(GameplayTraceChannel::SceneExit);
    GameplayTraceSample validScene = sample(20);
    validScene.stageStatus = GameplayTraceChannelStatus::NotSampled;
    validScene.cameraStatus = GameplayTraceChannelStatus::NotSampled;
    validScene.sceneExitStatus = GameplayTraceChannelStatus::Present;
    validScene.sceneExit.flags = GameplayTraceSceneExitVolumeValid;
    validScene.sceneExit.kind = GameplayTraceSceneExitRadialXz;
    validScene.sceneExit.actorAction = 0;
    validScene.sceneExit.observedCount = 1;
    validScene.sceneExit.signedDistanceToVolume = 1.0f;
    validScene.sceneExit.volumeExtent = {2.0f, 0.0f, 2.0f};

    GameplayTraceSample invalidScene = validScene;
    invalidScene.sceneExit.flags = 0;
    reject(invalidScene, sceneChannels, "scene-missing-volume.trace");
    invalidScene = validScene;
    invalidScene.sceneExit.flags |= GameplayTraceSceneExitPlayerInside;
    reject(invalidScene, sceneChannels, "scene-inside-parity.trace");
    invalidScene = validScene;
    invalidScene.sceneExit.observedCount = 0;
    reject(invalidScene, sceneChannels, "scene-count.trace");
    invalidScene = validScene;
    invalidScene.sceneExit.flags |= GameplayTraceSceneExitPlayerLatched;
    reject(invalidScene, sceneChannels, "scene-latch.trace");
    invalidScene = validScene;
    invalidScene.sceneExit.flags |= GameplayTraceSceneExitDestinationValid;
    reject(invalidScene, sceneChannels, "scene-destination.trace");
    invalidScene = validScene;
    invalidScene.sceneExit.flags |= GameplayTraceSceneExitObservedCountSaturated;
    reject(invalidScene, sceneChannels, "scene-count-saturation.trace");
    invalidScene = validScene;
    invalidScene.sceneExit.signedDistanceToVolume = -0.0f;
    invalidScene.sceneExit.flags |= GameplayTraceSceneExitPlayerInside;
    reject(invalidScene, sceneChannels, "scene-radial-negative-zero.trace");
    invalidScene = validScene;
    invalidScene.sceneExit.linkExitDirection = 4;
    reject(invalidScene, sceneChannels, "scene-unlatched-direction.trace");
    invalidScene = validScene;
    invalidScene.sceneExit.exitId = 1;
    reject(invalidScene, sceneChannels, "scene-exit-id.trace");

    GameplayTraceSample invalidBox = validScene;
    invalidBox.sceneExit.kind = GameplayTraceSceneExitBox;
    invalidBox.sceneExit.actorAction = 0xff;
    invalidBox.sceneExit.rawParameters = 0x04030201;
    invalidBox.sceneExit.exitId = 1;
    invalidBox.sceneExit.argument1 = 2;
    invalidBox.sceneExit.pathId = 3;
    invalidBox.sceneExit.switchNo = 4;
    invalidBox.sceneExit.volumeExtent = {2.0f, 3.0f, 4.0f};
    invalidBox.sceneExit.argument1 = 9;
    reject(invalidBox, sceneChannels, "scene-decoded-parameters.trace");

    constexpr std::uint64_t collisionChannels =
        gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
        gameplay_trace_channel_bit(GameplayTraceChannel::PlayerBackgroundCollision);
    GameplayTraceSample validCollision = sample(21);
    validCollision.stageStatus = GameplayTraceChannelStatus::NotSampled;
    validCollision.cameraStatus = GameplayTraceChannelStatus::NotSampled;
    validCollision.playerBackgroundCollisionStatus = GameplayTraceChannelStatus::Present;

    GameplayTraceSample invalidCollision = validCollision;
    invalidCollision.playerBackgroundCollision.flags = GameplayTraceCollisionGroundContact;
    reject(invalidCollision, collisionChannels, "collision-contact.trace");
    invalidCollision = validCollision;
    invalidCollision.playerBackgroundCollision.flags = GameplayTraceCollisionGroundIdentityPresent;
    reject(invalidCollision, collisionChannels, "collision-identity.trace");
    invalidCollision = validCollision;
    invalidCollision.playerBackgroundCollision.walls[0].flags = GameplayTraceCollisionWallHit;
    reject(invalidCollision, collisionChannels, "collision-wall.trace");
    invalidCollision = validCollision;
    invalidCollision.playerBackgroundCollision.flags = GameplayTraceCollisionTrajectoryValid;
    invalidCollision.playerBackgroundCollision.finalPosition[0] = 1.0f;
    reject(invalidCollision, collisionChannels, "collision-trajectory.trace");
    invalidCollision = validCollision;
    invalidCollision.playerBackgroundCollision.flags = GameplayTraceCollisionWaterOwnerPresent;
    reject(invalidCollision, collisionChannels, "collision-owner.trace");
    invalidCollision = validCollision;
    invalidCollision.playerBackgroundCollision.flags = GameplayTraceCollisionGroundProbeValid;
    reject(invalidCollision, collisionChannels, "collision-ground-sentinel.trace");
    invalidCollision = validCollision;
    invalidCollision.playerBackgroundCollision.flags = GameplayTraceCollisionGroundPlaneValid;
    reject(invalidCollision, collisionChannels, "collision-plane.trace");
    invalidCollision = validCollision;
    invalidCollision.playerBackgroundCollision.flags =
        GameplayTraceCollisionWaterProbeEnabled | GameplayTraceCollisionWaterIn;
    reject(invalidCollision, collisionChannels, "collision-water-in.trace");
}

}  // namespace

int main() {
    testChannelParser();
    const auto nonce = std::chrono::high_resolution_clock::now().time_since_epoch().count();
    const auto directory = std::filesystem::temp_directory_path() /
                           ("dusklight-gameplay-trace-test-" + std::to_string(nonce));
    REQUIRE(std::filesystem::create_directory(directory));
    const auto exactPath = directory / "exact.trace";
    const auto invalidPath = directory / "invalid.trace";
    const auto exhaustedPath = directory / "exhausted.trace";
    const auto observerChannelsPath = directory / "observer-channels.trace";
    const auto collisionSurfacesPath = directory / "collision-surfaces.trace";
    const auto fixturePath = directory / "fixture.trace";
    const auto goalActorsPath = directory / "goal-actors.trace";
    const auto retentionPath = directory / "retention.trace";
    testExactV2Layout(exactPath);
    testScenarioFixtureIsAuthenticatedInV4(fixturePath);
    testSceneExitAndBackgroundCollisionLayout(observerChannelsPath);
    testCollisionSurfaceLayout(collisionSurfacesPath);
    testGoalAndSelectedActorLayout(goalActorsPath);
    testValidationAndCapacity(invalidPath, exhaustedPath);
    testTriggeredRetention(retentionPath);
    testNewChannelValidation(directory);
    REQUIRE(std::filesystem::remove(exactPath));
    REQUIRE(std::filesystem::remove(exhaustedPath));
    REQUIRE(std::filesystem::remove(observerChannelsPath));
    REQUIRE(std::filesystem::remove(collisionSurfacesPath));
    REQUIRE(std::filesystem::remove(fixturePath));
    REQUIRE(std::filesystem::remove(goalActorsPath));
    REQUIRE(std::filesystem::remove(retentionPath));
    REQUIRE(std::filesystem::remove(directory));
    std::cout << "Gameplay trace tests passed\n";
    return 0;
}
