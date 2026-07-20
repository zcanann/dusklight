#pragma once

#include "dusk/automation/gameplay_trace_observer.hpp"
#include "dusk/automation/input_tape.hpp"
#include "dusk/automation/milestones.hpp"

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace dusk::automation {

inline constexpr std::uint16_t LearningEpisodeShardVersion = 1;
inline constexpr std::uint16_t LearningObservationVersion = 3;
inline constexpr std::uint16_t LearningActionVersion = 2;
inline constexpr std::uint32_t LearningEpisodeMaximumTicks = 4096;
inline constexpr std::string_view LearningObservationSchema = "dusklight-learning-observation/v3";
inline constexpr std::string_view LearningActionSchema = "dusklight-raw-pad-action/v2";
inline constexpr std::string_view LearningEpisodeShardSchema = "dusklight-native-episode-shard/v1";

enum class LearningObservationPhase : std::uint8_t {
    PreInput = 1,
    PostSimulation = 2,
};

enum class LearningTerminalReason : std::uint8_t {
    None = 0,
    GoalReached = 1,
    TickBudgetExhausted = 2,
};

enum class LearningActorSelectionRule : std::uint8_t {
    Complete = 0,
    LowestRuntimeGeneration = 1,
};

struct LearningGoalObservation {
    bool configured = false;
    bool reached = false;
    std::uint16_t requestedCount = 0;
    std::uint16_t hitCount = 0;
    std::uint16_t stableTicks = 0;
    std::uint16_t consecutiveTicks = 0;
    std::uint8_t sequenceSteps = 0;
    std::uint8_t sequenceNextStep = 0;
    std::uint16_t sequenceWithinTicks = 0;
    std::uint16_t sequenceElapsedTicks = 0;
    std::optional<std::uint64_t> firstHitTick;
};

struct LearningObservationContext {
    LearningObservationPhase phase = LearningObservationPhase::PreInput;
    LearningTerminalReason terminalReason = LearningTerminalReason::None;
    std::uint64_t boundaryIndex = 0;
    std::uint64_t simulationTick = 0;
    std::uint64_t tapeFrame = 0;
    std::uint32_t remainingTicks = 0;
    std::string stateIdentity;
    RawPadState previousInput{};
    bool cameraPresent = false;
    float cameraYawRadians = 0.0F;
    bool collisionCorrectionPresent = false;
    float collisionCorrectionX = 0.0F;
    float collisionCorrectionZ = 0.0F;
    // Pointer-free observer copy, consumed synchronously by append_learning_observation.
    const GameplayTraceSample* gameplayTrace = nullptr;
    GameplayCollisionPlanesObservation collisionPlanes;
    GameplayPlayerFormObservation playerForm;
    LearningGoalObservation goal;
};

/**
 * Appends one canonical, pointer-free observation to an episode payload.
 * The observation is copied immediately; none of its game-backed spans escape.
 */
[[nodiscard]] bool append_learning_observation(std::vector<std::uint8_t>& output,
    const MilestoneObservation& observation, const LearningObservationContext& context,
    std::string& error);

/**
 * Appends both the policy-selected raw PAD and the exact PADRead result consumed
 * between two observation boundaries. Keeping both makes any input-boundary
 * disagreement observable in the corpus instead of silently relabeling it.
 */
void append_learning_action(std::vector<std::uint8_t>& output, const RawPadState& chosenPad,
    const RawPadState& consumedPad);

/** Starts/finishes the uncompressed payload for one candidate episode. */
void begin_learning_episode(std::vector<std::uint8_t>& output);
[[nodiscard]] bool finish_learning_episode(
    std::vector<std::uint8_t>& output, std::uint32_t stepCount, std::string& error);

struct LearningEpisodeShardMetadata {
    std::size_t sourceFrame = 0;
    std::size_t maximumTicks = 0;
    std::string sourceBoundaryFingerprint;
    std::string checkpointIdentity;
    std::string objective;
    std::string objectiveIdentity;
    std::string buildRevision;
    std::string auroraRevision;
    std::string featureDigest;
    std::string fidelityProfile;
    // Supplied by the launching harness when known. Empty is explicit unknown,
    // never a claim that two game images are compatible.
    std::string gameDataIdentity;
};

struct LearningEpisodeDescriptor {
    std::string id;
    bool success = false;
    std::uint32_t ticksExecuted = 0;
    std::optional<std::uint32_t> firstHitTick;
    std::uint32_t remainingTicks = 0;
};

/**
 * Incremental one-file writer for checkpoint-restored sibling episodes.
 * Each episode is independently zstd-compressed, so memory remains bounded by
 * one candidate and a crash cannot confuse two episode boundaries.
 */
class LearningEpisodeShardWriter {
public:
    LearningEpisodeShardWriter() = default;
    LearningEpisodeShardWriter(const LearningEpisodeShardWriter&) = delete;
    LearningEpisodeShardWriter& operator=(const LearningEpisodeShardWriter&) = delete;
    ~LearningEpisodeShardWriter();

    [[nodiscard]] bool begin(const std::filesystem::path& path,
        const LearningEpisodeShardMetadata& metadata, std::string& error);
    [[nodiscard]] bool append(const LearningEpisodeDescriptor& descriptor,
        std::span<const std::uint8_t> uncompressedEpisode, std::string& error);
    [[nodiscard]] bool finish(std::string& error);
    void abandon();

    [[nodiscard]] bool active() const { return mStream.is_open(); }
    [[nodiscard]] std::uint32_t episodeCount() const { return mEpisodeCount; }
    [[nodiscard]] std::uint64_t compressedBytes() const { return mCompressedBytes; }
    [[nodiscard]] std::uint64_t uncompressedBytes() const { return mUncompressedBytes; }
    [[nodiscard]] const std::filesystem::path& path() const { return mPath; }

private:
    std::filesystem::path mPath;
    std::filesystem::path mTemporaryPath;
    std::ofstream mStream;
    std::uint32_t mEpisodeCount = 0;
    std::uint64_t mCompressedBytes = 0;
    std::uint64_t mUncompressedBytes = 0;
    std::uint64_t mMetadataBytes = 0;
    std::uint32_t mMaximumTicks = 0;
};

}  // namespace dusk::automation
