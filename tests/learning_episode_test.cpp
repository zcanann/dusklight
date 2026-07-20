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
    std::array<std::uint8_t, kMilestoneEventFlagCount> eventFlags{};
    std::array<std::uint8_t, kMilestoneTemporaryFlagCount> temporaryFlags{};
    std::array<std::uint8_t, kMilestoneDungeonFlagCount> dungeonFlags{};
    std::array<std::uint8_t, kMilestoneSwitchFlagCount> switchFlags{};
    MilestoneObservation observation;

    ObservationFixture() {
        actors[0] = {
            .runtimeGeneration = 7,
            .actorName = 0x123,
            .setId = 4,
            .homeRoom = 0,
            .currentRoom = 0,
            .positionX = 12.5F,
            .positionY = 3.0F,
            .positionZ = -8.0F,
            .health = 3,
            .status = 9,
            .parentRuntimeGeneration = 3,
            .parameters = 0x12345678,
            .profileName = 0x124,
            .group = 2,
            .argument = -3,
            .homePositionX = 10.0F,
            .homePositionY = 2.0F,
            .homePositionZ = -10.0F,
            .velocityX = 0.25F,
            .forwardSpeed = 0.25F,
            .currentAngleY = 100,
            .shapeAngleY = 90,
        };
        eventFlags[3] = 1;
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
        observation.actors = actors;
        observation.actorObservedCount = 1;
        observation.flagsPresent = true;
        observation.eventFlags = eventFlags;
        observation.temporaryFlags = temporaryFlags;
        observation.dungeonFlags = dungeonFlags;
        observation.switchFlags = switchFlags;
        observation.switchFlagRoom = 0;
    }
};

void test_episode_and_shard_are_compact_and_self_delimiting(
    const std::optional<std::filesystem::path>& fixturePath) {
    ObservationFixture fixture;
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
        .goal = {.configured = true, .requestedCount = 1},
    };
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
        .goal = {.configured = true, .requestedCount = 1},
    };
    REQUIRE(append_learning_observation(episode, fixture.observation, post, error));
    REQUIRE(finish_learning_episode(episode, 1, error));
    REQUIRE(read_little<std::uint32_t>(episode, 12) == 1);

    std::vector<std::uint8_t> successEpisode;
    fixture.observation.playerPositionX = -1.0F;
    begin_learning_episode(successEpisode);
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
    };
    LearningEpisodeShardWriter writer;
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
    REQUIRE(error.find("bounded channels") != std::string::npos);
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

}  // namespace

int main(const int argc, char** argv) {
    REQUIRE(argc <= 2);
    test_episode_and_shard_are_compact_and_self_delimiting(
        argc == 2 ? std::optional(std::filesystem::path(argv[1])) : std::nullopt);
    test_inconsistent_actor_completeness_fails_closed();
    test_duplicate_actor_identity_fails_closed();
    std::cout << "learning episode tests passed\n";
    return 0;
}
