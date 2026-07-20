#include "dusk/automation/game_state_observer.hpp"
#include "dusk/automation/learning_episode.hpp"

#include <array>
#include <chrono>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <optional>
#include <string>
#include <type_traits>
#include <vector>

#include <zstd.h>

using namespace dusk::automation;

namespace {

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "learning_episode_test.cpp:" << line << ": check failed: " << expression
                  << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

template <typename T>
T read_little(const std::vector<std::uint8_t>& bytes, const std::size_t offset) {
    using U = std::make_unsigned_t<T>;
    U value = 0;
    for (std::size_t index = 0; index < sizeof(T); ++index)
        value |= static_cast<U>(bytes.at(offset + index)) << (index * 8);
    return static_cast<T>(value);
}

std::vector<std::uint8_t> read_file(const std::filesystem::path& path) {
    std::ifstream stream(path, std::ios::binary);
    REQUIRE(stream.good());
    return {std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>()};
}

struct ObservationFixture {
    std::array<MilestoneObservation::Actor, 1> actors{};
    std::array<MilestoneObservation::DynamicCollider, 1> dynamicColliders{};
    std::array<std::uint8_t, kMilestoneEventFlagCount> eventFlags{};
    std::array<std::uint8_t, kMilestoneTemporaryFlagCount> temporaryFlags{};
    std::array<std::uint8_t, kMilestoneTemporaryEventByteCount> temporaryEventBytes{};
    std::array<std::uint8_t, kMilestoneDungeonFlagCount> dungeonFlags{};
    std::array<std::uint8_t, kMilestoneSwitchFlagCount> switchFlags{};
    MilestoneObservation observation;
    GameplayTraceSample gameplayTrace;
    GameplayCollisionPlanesObservation collisionPlanes;
    GameplayPlayerFormObservation playerForm{.present = true};

    ObservationFixture() {
        actors[0] = {
            .runtimeGeneration = 7,
            .actorType = 5,
            .processSubtype = 6,
            .actorName = 0x123,
            .setId = 4,
            .homeRoom = 0,
            .oldRoom = 1,
            .currentRoom = 0,
            .positionX = 12.5F,
            .positionY = 3.0F,
            .positionZ = -8.0F,
            .health = 3,
            .status = 9,
            .condition = 0x12,
            .parentRuntimeGeneration = 3,
            .parameters = 0x12345678,
            .profileName = 0x124,
            .group = 2,
            .argument = -3,
            .pauseFlag = 4,
            .processInitState = -2,
            .processCreatePhase = 7,
            .cullType = 8,
            .demoActorId = 9,
            .carryType = 10,
            .heapPresent = true,
            .modelPresent = true,
            .jointCollisionPresent = true,
            .homePositionX = 10.0F,
            .homePositionY = 2.0F,
            .homePositionZ = -10.0F,
            .oldPositionX = 12.0F,
            .oldPositionY = 2.5F,
            .oldPositionZ = -8.5F,
            .velocityX = 0.25F,
            .forwardSpeed = 0.25F,
            .scaleX = 1.0F,
            .scaleY = 2.0F,
            .scaleZ = 3.0F,
            .gravity = -3.0F,
            .maxFallSpeed = -20.0F,
            .eyePositionX = 12.5F,
            .eyePositionY = 7.0F,
            .eyePositionZ = -8.0F,
            .homeAngleX = 11,
            .homeAngleY = 12,
            .homeAngleZ = 13,
            .oldAngleX = 14,
            .oldAngleY = 15,
            .oldAngleZ = 16,
            .currentAngleY = 100,
            .shapeAngleY = 90,
            .attentionPresent = true,
            .attention =
                {
                    .flags = 0x20000002,
                    .positionX = 11.0F,
                    .positionY = 4.0F,
                    .positionZ = -7.0F,
                    .distanceIndices = {1, 2, 3, 4, 5, 6, 7, 8, 9},
                    .auxiliary = -4,
                },
            .eventParticipationPresent = true,
            .eventParticipation =
                {
                    .command = 1,
                    .condition = 3,
                    .eventId = 27,
                    .mapToolId = 8,
                    .index = 2,
                },
        };
        dynamicColliders[0] = {
            .registrationIndex = 0,
            .ownerRuntimeGeneration = 7,
            .attackHitOwnerRuntimeGeneration = 9,
            .ownerPresent = true,
            .statusPresent = true,
            .shapePresent = true,
            .attackSet = true,
            .targetSet = true,
            .correctionSet = true,
            .attackHit = true,
            .attackHitOwnerPresent = true,
            .shape = MilestoneObservation::DynamicColliderShape::Cylinder,
            .attackType = 0x20,
            .targetType = 0xd8fbfdff,
            .attackSourceParameters = 0x101,
            .attackResultParameters = 0x202,
            .targetSourceParameters = 0x303,
            .targetResultParameters = 0x404,
            .correctionSourceParameters = 0x505,
            .correctionResultParameters = 0x606,
            .attackPower = 4,
            .weight = 120,
            .damage = 3,
            .centerX = 12.5F,
            .centerY = 2.0F,
            .centerZ = -8.0F,
            .radius = 35.0F,
            .height = 80.0F,
            .aabbMinX = -22.5F,
            .aabbMinY = 2.0F,
            .aabbMinZ = -43.0F,
            .aabbMaxX = 47.5F,
            .aabbMaxY = 82.0F,
            .aabbMaxZ = 27.0F,
            .correctionX = 0.25F,
            .correctionZ = -0.5F,
        };
        eventFlags[3] = 1;
        temporaryEventBytes[0] = 0x06;
        temporaryEventBytes[1] = 0xa5;
        temporaryEventBytes[5] = 0xc0;
        switchFlags[8] = 1;
        observation.stageName = "F_SP103";
        observation.room = 0;
        observation.layer = 0;
        observation.point = 0;
        observation.playerPresent = true;
        observation.playerIsLink = true;
        observation.playerProcessId = 2;
        observation.playerActorName = 0x00fd;
        observation.playerProcId = 0x42;
        observation.playerPositionX = -1.0F;
        observation.playerPositionY = 2.0F;
        observation.playerPositionZ = 3.0F;
        observation.playerVelocityX = 0.5F;
        observation.playerForwardSpeed = 0.5F;
        observation.playerGroundContact = true;
        observation.playerGroundHeightPresent = true;
        observation.playerGroundHeight = 2.0F;
        observation.eventId = -1;
        observation.eventMapToolId = 0xff;
        observation.nextStageName = "";
        observation.nextRoom = -1;
        observation.nextLayer = -1;
        observation.nextPoint = -1;
        observation.rng.streams[0].id = GameRngStreamId::Primary;
        observation.rng.streams[1].id = GameRngStreamId::Secondary;
        observation.playerResourcesPresent = true;
        observation.playerResources.maximumLife = 20;
        observation.playerResources.life = 16;
        observation.playerResources.rupees = 123;
        observation.playerResources.rupeeCapacity = 600;
        observation.playerResources.maximumOil = 1200;
        observation.playerResources.oil = 875;
        observation.playerResources.maximumMagic = 32;
        observation.playerResources.magic = 17;
        observation.playerResources.wallet = 1;
        observation.playerResources.transformStatus = 0;
        observation.playerResources.worldTime = 210.5F;
        observation.playerResources.date = 3;
        observation.playerResources.arrows = 22;
        observation.playerResources.arrowCapacity = 30;
        observation.playerResources.pachinko = 11;
        observation.playerResources.poeSouls = 4;
        observation.playerResources.smallKeys = 2;
        observation.playerResources.dungeonMap = true;
        observation.playerResources.dungeonBossKey = true;
        observation.playerResources.inventory[1] = 0x48;
        observation.playerResources.inventory[4] = 0x43;
        observation.playerResources.selectedItems = {1, 4, 0xff, 0xff};
        observation.playerResources.mixedItems = {0xff, 0xff, 0xff, 0xff};
        observation.playerResources.equipment = {0x2e, 0x28, 0x2a, 0xff, 0x28, 0xff};
        observation.playerResources.bombCounts = {12, 0, 0};
        observation.playerResources.bombCapacities = {30, 0, 0};
        observation.playerResources.bottleQuantities = {1, 0, 0, 0};
        observation.playerResources.acquiredItemBits[8] = 0x04;
        observation.playerResources.collectItemBits[0] = 0x03;
        observation.playerResources.collectedCrystalBits = 0x02;
        observation.playerResources.collectedMirrorBits = 0x01;
        observation.runtimeFile.status = MilestoneObservation::ChannelStatus::Present;
        observation.runtimeFile.backingAttachmentStatus =
            MilestoneObservation::ChannelStatus::Present;
        observation.runtimeFile.dataNumRaw = 1;
        observation.runtimeFile.attachedPhysicalSlot = 2;
        for (std::size_t index = 0; index < observation.runtimeFile.physicalSlots.size(); ++index) {
            observation.runtimeFile.physicalSlots[index].number =
                static_cast<std::uint8_t>(index + 1);
            observation.runtimeFile.physicalSlots[index].contentStatus =
                MilestoneObservation::ChannelStatus::NotSampled;
        }
        observation.runtimeFile.physicalSlots[1].attachedToRuntime = true;
        observation.returnPlace.status = MilestoneObservation::ChannelStatus::Present;
        observation.returnPlace.stage = {'F', '_', 'S', 'P', '1', '0', '3', '\0'};
        observation.returnPlace.room = 0;
        observation.returnPlace.playerStatus = 2;
        observation.restart.status = MilestoneObservation::ChannelStatus::Present;
        observation.restart.room = 1;
        observation.restart.startPoint = 3;
        observation.restart.angleY = 0x1200;
        observation.restart.positionX = 10.0F;
        observation.restart.positionY = 20.0F;
        observation.restart.positionZ = 30.0F;
        observation.restart.roomParam = 0x01020304;
        observation.restart.lastSpeed = 4.5F;
        observation.restart.lastMode = 0x05060708;
        observation.restart.lastAngleY = -0x1200;
        observation.eventHandoff.status = MilestoneObservation::ChannelStatus::Present;
        observation.eventHandoff.preItemNo = 0x48;
        observation.eventHandoff.getItemNo = 0x43;
        observation.eventHandoff.eventFlags = 0x11;
        observation.eventHandoff.secondaryFlags = 0x22;
        observation.eventHandoff.hindFlags = 0x44;
        observation.eventHandoff.talkXyType = 2;
        observation.eventHandoff.compulsory = 1;
        observation.eventHandoff.roomInfoSet = true;
        observation.eventHandoff.skipTimer = 7;
        observation.eventHandoff.skipParameter = 9;
        observation.eventHandoff.itemPartner = {
            .present = true,
            .runtimeGeneration = 7,
            .actorName = 0x123,
            .setId = 4,
            .homeRoom = 0,
            .currentRoom = 0,
            .homePositionPresent = true,
            .homePositionX = 10.0F,
            .homePositionY = 2.0F,
            .homePositionZ = -10.0F,
        };
        observation.eventHandoff.eventNameStatus = MilestoneObservation::ChannelStatus::Present;
        observation.eventHandoff.eventName =
            {'D', 'E', 'F', 'A', 'U', 'L', 'T', '_', 'G', 'E', 'T', 'I', 'T', 'E', 'M', '\0'};
        observation.eventHandoff.messageFlowStatus =
            MilestoneObservation::ChannelStatus::Unavailable;
        observation.eventHandoff.pendingCleanupStatus =
            MilestoneObservation::ChannelStatus::Unavailable;
        observation.eventHandoff.playerControlStatus =
            MilestoneObservation::ChannelStatus::Present;
        observation.eventHandoff.playerControlModeFlags = 0x1234;
        observation.eventHandoff.playerControlDoStatus = 0x15;
        observation.eventHandoff.noTelopStatus = MilestoneObservation::ChannelStatus::Present;
        observation.eventHandoff.noTelop = true;
        observation.playerRelationshipsPresent = true;
        observation.playerRelationships.targetedActor = {
            .present = true,
            .runtimeGeneration = 7,
            .actorName = 0x123,
            .setId = 4,
            .homeRoom = 0,
            .currentRoom = 0,
            .homePositionPresent = true,
            .homePositionX = 10.0F,
            .homePositionY = 2.0F,
            .homePositionZ = -10.0F,
        };
        observation.playerCollisionSolverPresent = true;
        observation.playerCollisionSolver.flags = 0x2020;
        observation.playerCollisionSolver.wallTableSize = 3;
        observation.playerCollisionSolver.waterMode = 1;
        observation.playerCollisionSolver.lineStart = {-1.5F, 2.0F, 3.0F};
        observation.playerCollisionSolver.lineEnd = {-1.0F, 2.0F, 3.0F};
        observation.playerCollisionSolver.wallCylinderCenter = {-1.0F, 2.0F, 3.0F};
        observation.playerCollisionSolver.wallCylinderRadius = 35.0F;
        observation.playerCollisionSolver.wallCylinderHeight = 70.0F;
        observation.playerCollisionSolver.groundCheckOffset = 10.0F;
        observation.playerCollisionSolver.roofCorrectionHeight = 5.0F;
        observation.playerCollisionSolver.waterCheckOffset = 15.0F;
        observation.playerCollisionSolver.wallCircles[0].flags = 2;
        observation.playerCollisionSolver.wallCircles[0].angleY = 0x1200;
        observation.playerCollisionSolver.wallCircles[0].wallRadiusSquared = 1225.0F;
        observation.playerCollisionSolver.wallCircles[0].wallHeight = 35.0F;
        observation.playerCollisionSolver.wallCircles[0].wallRadius = 35.0F;
        observation.playerCollisionSolver.wallCircles[0].directWallHeight = 30.0F;
        observation.playerCollisionSolver.wallCircles[0].realizedCenter = {-1.0F, 37.0F, 3.0F};
        observation.playerCollisionSolver.wallCircles[0].realizedRadius = 35.0F;
        observation.actors = actors;
        observation.actorObservedCount = 1;
        observation.dynamicColliders = dynamicColliders;
        observation.dynamicCollidersPresent = true;
        observation.flagsPresent = true;
        observation.eventFlags = eventFlags;
        observation.temporaryFlags = temporaryFlags;
        observation.temporaryEventBytes = temporaryEventBytes;
        observation.dungeonFlags = dungeonFlags;
        observation.switchFlags = switchFlags;
        observation.switchFlagRoom = 0;

        gameplayTrace.cameraStatus = GameplayTraceChannelStatus::Present;
        gameplayTrace.camera.viewYaw = 0x1200;
        gameplayTrace.camera.controlledYaw = 0x1000;
        gameplayTrace.camera.bank = -20;
        gameplayTrace.camera.eye = {-25.0F, 40.0F, 80.0F};
        gameplayTrace.camera.center = {-1.0F, 12.0F, 3.0F};
        gameplayTrace.camera.up = {0.0F, 1.0F, 0.0F};
        gameplayTrace.camera.fovy = 45.0F;
        gameplayTrace.playerActionStatus = GameplayTraceChannelStatus::Present;
        gameplayTrace.playerAction.procedureId = 0x42;
        gameplayTrace.playerAction.modeFlags = 0x1234;
        gameplayTrace.playerAction.procedureContextRaw = {1, 2, 3, 4, 5, 6};
        gameplayTrace.playerAction.damageWaitTimer = 7;
        gameplayTrace.playerAction.swordAtUpTime = 8;
        gameplayTrace.playerAction.iceDamageWaitTimer = 9;
        gameplayTrace.playerAction.swordChangeWaitTimer = 10;
        gameplayTrace.playerAction.underAnimations[0] = {0x55, 12.5F, 1.0F};
        gameplayTrace.playerAction.upperAnimations[1] = {0x66, 4.25F, 0.5F};
        gameplayTrace.playerAction.doStatus = 0x15;
        gameplayTrace.sceneExitStatus = GameplayTraceChannelStatus::Present;
        gameplayTrace.sceneExit.sessionProcessId = 44;
        gameplayTrace.sceneExit.rawParameters = 0x00112233;
        gameplayTrace.sceneExit.flags =
            GameplayTraceSceneExitVolumeValid | GameplayTraceSceneExitDestinationValid;
        gameplayTrace.sceneExit.signedDistanceToVolume = 15.5F;
        gameplayTrace.sceneExit.actorName = 0x101;
        gameplayTrace.sceneExit.setId = 9;
        gameplayTrace.sceneExit.exitId = 0x33;
        gameplayTrace.sceneExit.kind = GameplayTraceSceneExitBox;
        gameplayTrace.sceneExit.observedCount = 2;
        gameplayTrace.sceneExit.playerLocalPosition = {1.0F, 2.0F, 3.0F};
        gameplayTrace.sceneExit.volumeExtent = {10.0F, 20.0F, 30.0F};
        gameplayTrace.sceneExit.homePosition = {100.0F, 200.0F, 300.0F};
        gameplayTrace.sceneExit.destinationStage = {'F', '_', 'S', 'P', '1', '0', '4', '\0'};
        gameplayTrace.sceneExit.destinationRoom = 0;
        gameplayTrace.sceneExit.destinationLayer = 1;
        gameplayTrace.sceneExit.destinationPoint = 2;
        gameplayTrace.sceneExit.destinationWipe = 0;
        gameplayTrace.sceneExit.destinationWipeTime = 0;
        gameplayTrace.sceneExit.destinationTimeHour = -1;
        gameplayTrace.sceneExit.pathId = 0x11;
        gameplayTrace.sceneExit.argument1 = 0x22;
        gameplayTrace.sceneExit.switchNo = 0;
        gameplayTrace.playerBackgroundCollisionStatus = GameplayTraceChannelStatus::Present;
        gameplayTrace.playerBackgroundCollision.flags =
            GameplayTraceCollisionGroundProbeValid | GameplayTraceCollisionGroundContact |
            GameplayTraceCollisionGroundPlaneValid | GameplayTraceCollisionTrajectoryValid |
            GameplayTraceCollisionGroundIdentityPresent;
        gameplayTrace.playerBackgroundCollision.groundHeight = 2.0F;
        gameplayTrace.playerBackgroundCollision.groundBgIndex = 1;
        gameplayTrace.playerBackgroundCollision.groundPolyIndex = 17;
        gameplayTrace.playerBackgroundCollision.groundPlane = {0.0F, 1.0F, 0.0F, -2.0F};
        gameplayTrace.playerBackgroundCollision.oldPosition = {-1.5F, 2.0F, 3.0F};
        gameplayTrace.playerBackgroundCollision.resolvedFrameDisplacement = {0.5F, 0.0F, 0.0F};
        gameplayTrace.playerBackgroundCollision.finalPosition = {-1.0F, 2.0F, 3.0F};
        gameplayTrace.playerCollisionSurfacesStatus = GameplayTraceChannelStatus::Present;
        gameplayTrace.playerCollisionSurfaces.flags = GameplayTraceCollisionSurfaceCurrentRoomValid;
        gameplayTrace.playerCollisionSurfaces.currentRoom = 0;
        gameplayTrace.playerCollisionSurfaces.identityCount = 1;
        gameplayTrace.playerCollisionSurfaces.backingCodeCount = 1;
        auto& ground = gameplayTrace.playerCollisionSurfaces.surfaces[0];
        ground.flags = GameplayTraceCollisionSurfaceIdentityPresent |
                       GameplayTraceCollisionSurfaceBackingResolved |
                       GameplayTraceCollisionSurfaceRawCodesPresent |
                       GameplayTraceCollisionSurfaceMaterialPresent |
                       GameplayTraceCollisionSurfaceGroupPresent |
                       GameplayTraceCollisionSurfaceSourceRoomPresent |
                       GameplayTraceCollisionSurfaceSourceRoomExact |
                       GameplayTraceCollisionSurfaceGeometryPresent;
        ground.backingFormat = GameplayTraceCollisionBackingDzb;
        ground.rawCodePresenceMask = 0x1f;
        ground.bgIndex = 1;
        ground.polyIndex = 17;
        ground.materialIndex = 4;
        ground.groupIndex = 2;
        ground.rawCodes = {1, 2, 3, 4, 5};
        ground.rawExitId = 1;
        ground.sourceRoom = 0;
        ground.sourceGeometryIndexCount = 3;
        ground.sourceGeometryIndices = {10, 11, 12, 0xffff, 0xffff, 0xffff};
        collisionPlanes.validMask = 1;
        collisionPlanes.planes[0] = {0.0F, 1.0F, 0.0F, -2.0F};
    }

    void setTraceBoundary(const GameplayTracePhase phase, const std::uint64_t boundary,
        const std::uint64_t simulationTick, const std::uint64_t tapeFrame) {
        gameplayTrace.core.phase = phase;
        gameplayTrace.core.boundaryIndex = boundary;
        gameplayTrace.core.simulationTick = simulationTick;
        gameplayTrace.core.tapeFrame = tapeFrame;
    }
};

void test_episode_and_shard_are_compact_and_self_delimiting(
    const std::optional<std::filesystem::path>& fixturePath) {
    ObservationFixture fixture;
    std::vector<MilestoneObservation::Actor> completeActors(
        kInputControllerMaximumActors + 1, fixture.actors[0]);
    for (std::size_t index = 0; index < completeActors.size(); ++index)
        completeActors[index].runtimeGeneration = index + 1;
    completeActors.back().attentionPresent = false;
    completeActors.back().eventParticipationPresent = false;
    fixture.observation.actors = completeActors;
    fixture.observation.actorObservedCount = static_cast<std::uint32_t>(completeActors.size());
    RawPadState pad;
    pad.buttons = 0x0100;
    pad.stickX = 100;

    std::vector<std::uint8_t> episode;
    begin_learning_episode(episode);
    std::string error;
    LearningObservationContext pre{
        .phase = LearningObservationPhase::PreInput,
        .boundaryIndex = 10,
        .simulationTick = 10,
        .tapeFrame = 440,
        .remainingTicks = 1,
        .stateIdentity = "11111111111111111111111111111111",
        .gameplayTrace = &fixture.gameplayTrace,
        .collisionPlanes = fixture.collisionPlanes,
        .playerForm = fixture.playerForm,
        .goal = {.configured = true, .requestedCount = 1},
    };
    fixture.setTraceBoundary(GameplayTracePhase::PreInput, 10, 10, 440);
    REQUIRE(append_learning_observation(episode, fixture.observation, pre, error));
    append_learning_action(episode, pad, pad);
    fixture.observation.playerPositionX = -0.5F;
    LearningObservationContext post{
        .phase = LearningObservationPhase::PostSimulation,
        .terminalReason = LearningTerminalReason::TickBudgetExhausted,
        .boundaryIndex = 11,
        .simulationTick = 10,
        .tapeFrame = 440,
        .remainingTicks = 0,
        .stateIdentity = "22222222222222222222222222222222",
        .previousInput = pad,
        .gameplayTrace = &fixture.gameplayTrace,
        .collisionPlanes = fixture.collisionPlanes,
        .playerForm = fixture.playerForm,
        .goal = {.configured = true, .requestedCount = 1},
    };
    fixture.setTraceBoundary(GameplayTracePhase::PostSimulation, 11, 10, 440);
    REQUIRE(append_learning_observation(episode, fixture.observation, post, error));
    REQUIRE(finish_learning_episode(episode, 1, error));
    REQUIRE(read_little<std::uint32_t>(episode, 12) == 1);

    std::vector<std::uint8_t> successEpisode;
    fixture.observation.playerPositionX = -1.0F;
    begin_learning_episode(successEpisode);
    fixture.setTraceBoundary(GameplayTracePhase::PreInput, 10, 10, 440);
    REQUIRE(append_learning_observation(successEpisode, fixture.observation, pre, error));
    append_learning_action(successEpisode, pad, pad);
    fixture.observation.playerPositionX = 0.0F;
    LearningObservationContext successPost = post;
    successPost.terminalReason = LearningTerminalReason::GoalReached;
    successPost.stateIdentity = "33333333333333333333333333333333";
    successPost.goal.reached = true;
    successPost.goal.hitCount = 1;
    successPost.goal.consecutiveTicks = 1;
    successPost.goal.firstHitTick = 10;
    fixture.setTraceBoundary(GameplayTracePhase::PostSimulation, 11, 10, 440);
    REQUIRE(append_learning_observation(successEpisode, fixture.observation, successPost, error));
    REQUIRE(finish_learning_episode(successEpisode, 1, error));

    const auto nonce = std::chrono::steady_clock::now().time_since_epoch().count();
    const std::filesystem::path path =
        fixturePath.value_or(std::filesystem::temp_directory_path() /
                             ("dusklight-learning-episode-" + std::to_string(nonce) + ".dseps"));
    if (fixturePath.has_value()) {
        std::filesystem::remove(path);
        std::filesystem::remove(path.string() + ".tmp");
    }
    const LearningEpisodeShardMetadata metadata{
        .sourceFrame = 440,
        .maximumTicks = 1,
        .sourceBoundaryFingerprint = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        .checkpointIdentity = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        .objective = "exit-f-sp103-to-f-sp104",
        .objectiveIdentity = "cccccccccccccccccccccccccccccccc",
        .buildRevision = "deadbeef",
        .auroraRevision = "cafebabe",
        .featureDigest = "dddddddddddddddddddddddddddddddd",
        .fidelityProfile = "native-read-only-checkpoint-batch",
        .gameDataSha256 = "1111111111111111111111111111111111111111111111111111111111111111",
        .cardFixtureIdentity = "card-fixture:xxh3-128:22222222222222222222222222222222",
        .actorProfileCatalogIdentity =
            "actor-profile-catalog:xxh3-128:33333333333333333333333333333333",
        .worldContextSha256 = "4444444444444444444444444444444444444444444444444444444444444444",
    };
    LearningEpisodeShardWriter writer;
    LearningEpisodeShardMetadata invalidMetadata = metadata;
    invalidMetadata.gameDataSha256.clear();
    REQUIRE(!writer.begin(path, invalidMetadata, error));
    REQUIRE(error.find("metadata is incomplete") != std::string::npos);
    REQUIRE(writer.begin(path, metadata, error));
    REQUIRE(!writer.append(
        {.id = "invalid-success", .success = true, .ticksExecuted = 1, .remainingTicks = 0},
        successEpisode, error));
    REQUIRE(error.find("block is invalid") != std::string::npos);
    REQUIRE(writer.append(
        {.id = "failure-0", .ticksExecuted = 1, .remainingTicks = 0}, episode, error));
    REQUIRE(writer.append({.id = "success-0",
                              .success = true,
                              .ticksExecuted = 1,
                              .firstHitTick = 0,
                              .remainingTicks = 0},
        successEpisode, error));
    REQUIRE(writer.finish(error));
    REQUIRE(writer.episodeCount() == 2);
    REQUIRE(writer.uncompressedBytes() == episode.size() + successEpisode.size());

    const std::vector<std::uint8_t> shard = read_file(path);
    REQUIRE(shard.size() > 128);
    REQUIRE(std::string(shard.begin(), shard.begin() + 7) == "DUSKEPS");
    REQUIRE(read_little<std::uint16_t>(shard, 8) == LearningEpisodeShardVersion);
    REQUIRE(read_little<std::uint16_t>(shard, 128) == 15);
    REQUIRE(read_little<std::uint32_t>(shard, 12) == 1);
    REQUIRE(read_little<std::uint32_t>(shard, 16) == 2);
    const std::size_t payloadOffset = read_little<std::uint64_t>(shard, 56);
    REQUIRE(
        std::string(shard.begin() + payloadOffset, shard.begin() + payloadOffset + 4) == "EPIS");
    const std::size_t idLength = read_little<std::uint16_t>(shard, payloadOffset + 20);
    const std::size_t expandedSize = read_little<std::uint64_t>(shard, payloadOffset + 24);
    const std::size_t compressedSize = read_little<std::uint64_t>(shard, payloadOffset + 32);
    const std::size_t compressedOffset = payloadOffset + 64 + idLength;
    std::vector<std::uint8_t> expanded(expandedSize);
    REQUIRE(ZSTD_decompress(expanded.data(), expanded.size(), shard.data() + compressedOffset,
                compressedSize) == expanded.size());
    REQUIRE(expanded == episode);
    if (!fixturePath.has_value()) {
        LearningEpisodeShardWriter replacement;
        REQUIRE(replacement.begin(path, metadata, error));
        REQUIRE(replacement.append(
            {.id = "replacement", .ticksExecuted = 1, .remainingTicks = 0}, episode, error));
        REQUIRE(replacement.finish(error));
        REQUIRE(read_little<std::uint32_t>(read_file(path), 16) == 1);
        std::filesystem::remove(path);
    }
}

void test_inconsistent_actor_completeness_fails_closed() {
    ObservationFixture fixture;
    fixture.observation.actorObservedCount = 2;
    std::vector<std::uint8_t> bytes;
    begin_learning_episode(bytes);
    std::string error;
    REQUIRE(!append_learning_observation(bytes, fixture.observation,
        {
            .stateIdentity = "11111111111111111111111111111111",
        },
        error));
    REQUIRE(error.find("incomplete or inconsistent channels") != std::string::npos);
}

void test_actor_population_is_not_limited_by_controller_capacity() {
    ObservationFixture fixture;
    std::vector<MilestoneObservation::Actor> actors(
        kInputControllerMaximumActors + 1, fixture.actors[0]);
    for (std::size_t index = 0; index < actors.size(); ++index)
        actors[index].runtimeGeneration = index + 1;
    fixture.observation.actors = actors;
    fixture.observation.actorObservedCount = static_cast<std::uint32_t>(actors.size());

    std::vector<std::uint8_t> bytes;
    begin_learning_episode(bytes);
    std::string error;
    REQUIRE(append_learning_observation(bytes, fixture.observation,
        {
            .stateIdentity = "11111111111111111111111111111111",
        },
        error));
    REQUIRE(read_little<std::uint16_t>(bytes, 28) == actors.size());
    REQUIRE(read_little<std::uint32_t>(bytes, 34) == actors.size());
}

void test_duplicate_actor_identity_fails_closed() {
    ObservationFixture fixture;
    std::array<MilestoneObservation::Actor, 2> actors{fixture.actors[0], fixture.actors[0]};
    fixture.observation.actors = actors;
    fixture.observation.actorObservedCount = 2;
    std::vector<std::uint8_t> bytes;
    begin_learning_episode(bytes);
    std::string error;
    REQUIRE(!append_learning_observation(bytes, fixture.observation,
        {
            .stateIdentity = "11111111111111111111111111111111",
        },
        error));
    REQUIRE(error.find("strictly ordered") != std::string::npos);
}

void test_temporary_event_register_bank_is_required() {
    ObservationFixture fixture;
    fixture.observation.temporaryEventBytes = {};
    std::vector<std::uint8_t> bytes;
    begin_learning_episode(bytes);
    std::string error;
    REQUIRE(!append_learning_observation(bytes, fixture.observation,
        {
            .stateIdentity = "11111111111111111111111111111111",
        },
        error));
    REQUIRE(error.find("incomplete or inconsistent channels") != std::string::npos);
}

void test_player_resources_presence_matches_player_presence() {
    ObservationFixture fixture;
    fixture.observation.playerResourcesPresent = false;
    std::vector<std::uint8_t> bytes;
    begin_learning_episode(bytes);
    std::string error;
    REQUIRE(!append_learning_observation(bytes, fixture.observation,
        {
            .stateIdentity = "11111111111111111111111111111111",
        },
        error));
    REQUIRE(error.find("incomplete or inconsistent channels") != std::string::npos);
}

void test_runtime_file_attachment_fails_closed() {
    ObservationFixture fixture;
    fixture.observation.runtimeFile.backingAttachmentStatus =
        MilestoneObservation::ChannelStatus::Unavailable;
    std::vector<std::uint8_t> bytes;
    begin_learning_episode(bytes);
    std::string error;
    REQUIRE(!append_learning_observation(bytes, fixture.observation,
        {
            .stateIdentity = "11111111111111111111111111111111",
        },
        error));
    REQUIRE(error.find("inconsistent runtime-file backing") != std::string::npos);
}

void test_player_relationships_join_complete_actor_population() {
    ObservationFixture fixture;
    fixture.observation.playerRelationships.targetedActor.runtimeGeneration = 99;
    std::vector<std::uint8_t> bytes;
    begin_learning_episode(bytes);
    std::string error;
    REQUIRE(!append_learning_observation(bytes, fixture.observation,
        {
            .stateIdentity = "11111111111111111111111111111111",
        },
        error));
    REQUIRE(error.find("relationship is inconsistent with actor set") != std::string::npos);
}

void test_mechanics_boundary_and_surface_identity_fail_closed() {
    ObservationFixture fixture;
    std::vector<std::uint8_t> bytes;
    begin_learning_episode(bytes);
    std::string error;
    fixture.setTraceBoundary(GameplayTracePhase::PostSimulation, 11, 10, 440);
    REQUIRE(!append_learning_observation(bytes, fixture.observation,
        {
            .phase = LearningObservationPhase::PreInput,
            .boundaryIndex = 10,
            .simulationTick = 10,
            .tapeFrame = 440,
            .remainingTicks = 1,
            .stateIdentity = "11111111111111111111111111111111",
            .gameplayTrace = &fixture.gameplayTrace,
            .collisionPlanes = fixture.collisionPlanes,
            .playerForm = {.present = true},
        },
        error));
    REQUIRE(error.find("incomplete or inconsistent channels") != std::string::npos);

    fixture.setTraceBoundary(GameplayTracePhase::PreInput, 10, 10, 440);
    fixture.gameplayTrace.playerCollisionSurfaces.surfaces[0].flags = 0;
    REQUIRE(!append_learning_observation(bytes, fixture.observation,
        {
            .phase = LearningObservationPhase::PreInput,
            .boundaryIndex = 10,
            .simulationTick = 10,
            .tapeFrame = 440,
            .remainingTicks = 1,
            .stateIdentity = "11111111111111111111111111111111",
            .gameplayTrace = &fixture.gameplayTrace,
            .collisionPlanes = fixture.collisionPlanes,
            .playerForm = {.present = true},
        },
        error));
    REQUIRE(error.find("lacks a surface identity") != std::string::npos);
}

}  // namespace

int main(const int argc, char** argv) {
    REQUIRE(argc <= 2);
    test_episode_and_shard_are_compact_and_self_delimiting(
        argc == 2 ? std::optional(std::filesystem::path(argv[1])) : std::nullopt);
    test_inconsistent_actor_completeness_fails_closed();
    test_actor_population_is_not_limited_by_controller_capacity();
    test_duplicate_actor_identity_fails_closed();
    test_temporary_event_register_bank_is_required();
    test_player_resources_presence_matches_player_presence();
    test_runtime_file_attachment_fails_closed();
    test_player_relationships_join_complete_actor_population();
    test_mechanics_boundary_and_surface_identity_fail_closed();
    std::cout << "learning episode tests passed\n";
    return 0;
}
