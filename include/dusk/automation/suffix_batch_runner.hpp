#pragma once

#include "dusk/automation/game_state_observer.hpp"
#include "dusk/automation/learning_episode.hpp"
#include "dusk/automation/milestones.hpp"
#include "dusk/automation/state_checkpoint.hpp"
#include "dusk/automation/suffix_batch.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <optional>
#include <span>
#include <string>
#include <vector>

#include <dolphin/pad.h>

namespace dusk::automation {

class SuffixBatchRunner {
public:
    bool configure(SuffixBatchDefinition definition, std::filesystem::path resultPath,
        std::filesystem::path winnerTapePath, std::string gameDataSha256,
        std::string worldContextSha256, std::string& error);
    /**
     * Installs another batch against the already authenticated source image.
     * The source frame/fingerprint and objective are immutable for a session;
     * candidate inputs, horizon, and output paths may change.
     */
    bool configureNextBatch(SuffixBatchDefinition definition,
        std::filesystem::path resultPath, std::filesystem::path winnerTapePath,
        std::string& error);

    /** Stable boundary immediately before InputTapePlayer::tick(). */
    bool preInput(std::uint64_t& simulationTick, std::uint64_t& tapeFrame,
        std::uint64_t& preparedInputFrame, bool& tapeFrameApplied, std::string& error);
    /** After tape injection and immediately before PADRead. */
    void applyCandidateInput();
    /** Exact PADRead output before JUTGamePad clamps it. */
    void recordConsumedPads(std::span<const PADStatus, kInputPortCount> statuses);
    /** After game simulation and deterministic clock advancement. */
    bool postSimulation(std::uint64_t simulationTick, std::uint64_t tapeFrame,
        std::string& error);

    [[nodiscard]] bool enabled() const { return mEnabled; }
    /** True only for a simulation tick belonging to a checkpoint-restored candidate. */
    [[nodiscard]] bool executingCandidate() const { return mPhase == Phase::Candidate; }
    [[nodiscard]] bool completed() const { return mCompleted; }
    [[nodiscard]] bool failed() const { return mFailed; }
    [[nodiscard]] bool writeArtifacts(std::string& error);
    [[nodiscard]] bool artifactsWritten() const { return mArtifactsWritten; }
    [[nodiscard]] const std::filesystem::path& resultPath() const { return mResultPath; }
    [[nodiscard]] const std::filesystem::path& episodeShardPath() const {
        return mEpisodeShardPath;
    }

private:
    enum class Phase { WaitingForSource, Candidate, RestoreNext, Complete, Failed };

    struct HostSnapshot {
        InputTapePlayerState tapePlayer;
        MilestoneTracker milestones;
        PADAutomationState pad{};
        std::uint64_t simulationTick = 0;
        std::uint64_t tapeFrame = 0;
        std::uint64_t preparedInputFrame = 0;
        bool tapeFrameApplied = false;
    };

    struct TerminalObservation {
        std::string stage;
        std::int8_t room = -1;
        std::int16_t point = -1;
        float positionX = 0.0F;
        float positionY = 0.0F;
        float positionZ = 0.0F;
        float velocityX = 0.0F;
        float velocityY = 0.0F;
        float velocityZ = 0.0F;
        float forwardSpeed = 0.0F;
        std::int16_t facing = 0;
        std::uint16_t procedure = 0xffff;
        std::uint32_t modeFlags = 0;
        bool cameraPresent = false;
        float cameraHeadingRadians = 0.0F;
        bool transitionPending = false;
        std::string nextStage;
        std::int8_t nextRoom = -1;
        std::int16_t nextPoint = -1;
        bool collisionCorrectionPresent = false;
        float collisionCorrectionX = 0.0F;
        float collisionCorrectionZ = 0.0F;
        RawPadState previousInput{};
    };

    struct CandidateResult {
        std::string id;
        bool success = false;
        std::size_t ticksExecuted = 0;
        std::optional<std::size_t> firstHitTick;
        std::string stateSequenceDigest;
        std::string predicateEvidence;
        TerminalObservation terminal;
        std::vector<RawPadState> successfulConsumedPads;
    };

    bool captureSource(std::uint64_t simulationTick, std::uint64_t tapeFrame,
        std::uint64_t preparedInputFrame, bool tapeFrameApplied, std::string& error);
    bool beginEpisodeShard(std::string& error);
    bool restoreSource(std::uint64_t& simulationTick, std::uint64_t& tapeFrame,
        std::uint64_t& preparedInputFrame, bool& tapeFrameApplied, std::string& error);
    bool captureEpisodePreInput(std::uint64_t simulationTick, std::string& error);
    bool appendEpisodePostSimulation(const MilestoneObservation& observation,
        const RawPadState& chosenPad, std::uint64_t simulationTick, bool terminal,
        std::string& error);
    bool finishCandidate(
        const MilestoneObservation& observation, bool success, std::string& error);
    void fail(std::string message);

    bool mEnabled = false;
    bool mCompleted = false;
    bool mFailed = false;
    bool mArtifactsWritten = false;
    bool mConsumedCaptureFailed = false;
    bool mAudioCallbackQuiesced = false;
    Phase mPhase = Phase::WaitingForSource;
    SuffixBatchDefinition mDefinition;
    std::filesystem::path mResultPath;
    std::filesystem::path mWinnerTapePath;
    std::filesystem::path mEpisodeShardPath;
    std::string mGameDataSha256;
    std::string mWorldContextSha256;
    StateCheckpoint mCheckpoint;
    StateCheckpointImage mImage;
    HostSnapshot mSource;
    MilestoneTracker mGoalTracker;
    MilestoneObservationStorage mMilestoneStorage;
    MilestoneObservationStorage mSourceMilestoneStorage;
    ControllerObservationStorage mControllerStorage;
    MilestoneObservationStorage mEpisodeMilestoneStorage;
    ControllerObservationStorage mEpisodeControllerStorage;
    std::size_t mCandidateIndex = 0;
    std::size_t mCandidateTick = 0;
    std::vector<RawPadState> mConsumedPads;
    std::vector<std::uint8_t> mCurrentEpisode;
    bool mEpisodePreInputCaptured = false;
    LearningEpisodeShardWriter mEpisodeShard;
    std::string mStateDigestMaterial;
    std::vector<CandidateResult> mResults;
    std::optional<std::size_t> mWinnerResultIndex;
    std::uint64_t mCaptureMicros = 0;
    std::string mActualSourceBoundaryFingerprint;
    std::vector<std::uint64_t> mRestoreMicros;
    std::string mError;
};

SuffixBatchRunner& suffix_batch_runner();

}  // namespace dusk::automation
