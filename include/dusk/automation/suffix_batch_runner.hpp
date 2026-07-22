#pragma once

#include "dusk/automation/game_state_observer.hpp"
#include "dusk/automation/frozen_inference.hpp"
#include "dusk/automation/learning_episode.hpp"
#include "dusk/automation/milestones.hpp"
#include "dusk/automation/state_checkpoint.hpp"
#include "dusk/automation/suffix_batch.hpp"
#include "dusk/automation/native_policy_features.hpp"

#include <array>
#include <chrono>
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
    /** Exact exclusive game/audio simulation interval after PADRead. */
    void beginSimulationProfile();
    /** CPU traversal of gameplay draw callbacks inside fpcM_Management. */
    void beginCpuDrawTraversalProfile();
    void endCpuDrawTraversalProfile();
    /** CPU-side GX painter/submission work inside fpcM_Management. */
    void beginCpuRendererSubmissionProfile();
    void endCpuRendererSubmissionProfile();
    /** Exact PADRead output before JUTGamePad clamps it. */
    void recordConsumedPads(std::span<const PADStatus, kInputPortCount> statuses);
    /** After game simulation and deterministic clock advancement. */
    bool postSimulation(std::uint64_t simulationTick, std::uint64_t tapeFrame,
        std::uint64_t preparedInputFrame, bool tapeFrameApplied, std::string& error);

    [[nodiscard]] bool enabled() const { return mEnabled; }
    /** True only for a simulation tick belonging to a checkpoint-restored candidate. */
    [[nodiscard]] bool executingCandidate() const { return mPhase == Phase::Candidate; }
    /** True when this runner exclusively owns the current post-simulation boundary. */
    [[nodiscard]] bool ownsPostSimulation() const {
        return mPhase == Phase::ValidateFresh || mPhase == Phase::ValidateRestored ||
               mPhase == Phase::Candidate;
    }
    [[nodiscard]] bool completed() const { return mCompleted; }
    [[nodiscard]] bool failed() const { return mFailed; }
    [[nodiscard]] bool writeArtifacts(std::string& error);
    [[nodiscard]] bool artifactsWritten() const { return mArtifactsWritten; }
    [[nodiscard]] const std::filesystem::path& resultPath() const { return mResultPath; }
    [[nodiscard]] const std::filesystem::path& episodeShardPath() const {
        return mEpisodeShardPath;
    }

private:
    enum class Phase {
        WaitingForSource,
        ValidateFresh,
        RestoreValidation,
        ValidateRestored,
        Candidate,
        RestoreNext,
        Complete,
        Failed,
    };

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
        std::vector<std::string> stateTickDigests;
        std::string predicateEvidence;
        TerminalObservation terminal;
        std::vector<RawPadState> successfulConsumedPads;
    };

    bool captureSource(std::uint64_t simulationTick, std::uint64_t tapeFrame,
        std::uint64_t preparedInputFrame, bool tapeFrameApplied, std::string& error);
    bool beginEpisodeShard(std::string& error);
    bool restoreSource(std::uint64_t& simulationTick, std::uint64_t& tapeFrame,
        std::uint64_t& preparedInputFrame, bool& tapeFrameApplied, std::string& error);
    bool captureValidationTickDigest(std::uint64_t simulationTick, std::uint64_t tapeFrame,
        std::uint64_t preparedInputFrame, bool tapeFrameApplied, std::string& output,
        std::string& error);
    bool captureEpisodePreInput(std::uint64_t simulationTick, std::string& error);
    bool loadFrozenPolicy(const SuffixBatchDefinition& definition,
        FrozenInferenceModel& output, std::string& error) const;
    bool appendEpisodePostSimulation(const MilestoneObservation& observation,
        const RawPadState& chosenPad, std::uint64_t simulationTick, bool terminal,
        std::string& error);
    bool finishCandidate(
        const MilestoneObservation& observation, bool success, std::string& error);
    void resetBatchProfile(bool sourceCheckpointReused);
    void finishSimulationProfile();
    void finishBatchProfile();
    void fail(std::string message);

    using ProfileClock = std::chrono::steady_clock;

    struct BatchProfile {
        ProfileClock::time_point batchStart{};
        ProfileClock::time_point simulationStart{};
        ProfileClock::time_point cpuDrawStart{};
        ProfileClock::time_point cpuRendererStart{};
        std::uint64_t batchWallMicros = 0;
        std::uint64_t policyHeadDecodeNanos = 0;
        std::uint64_t policyInferenceNanos = 0;
        std::uint64_t policyApplicationNanos = 0;
        std::uint64_t simulationMicros = 0;
        std::uint64_t observationCaptureMicros = 0;
        std::uint64_t stateValidationMicros = 0;
        std::uint64_t corpusEncodingMicros = 0;
        std::uint64_t cpuDrawTraversalMicros = 0;
        std::uint64_t cpuRendererSubmissionMicros = 0;
        std::uint64_t policyHeadDecodeSamples = 0;
        std::uint64_t policyInferenceSamples = 0;
        std::uint64_t policyApplicationSamples = 0;
        std::uint64_t simulationSamples = 0;
        std::uint64_t observationCaptureSamples = 0;
        std::uint64_t stateValidationSamples = 0;
        std::uint64_t cpuDrawTraversalSamples = 0;
        std::uint64_t cpuRendererSubmissionSamples = 0;
        std::uint64_t submittedCommandBuffersAtStart = 0;
        std::uint64_t discardedGpuFramesAtStart = 0;
        bool active = false;
        bool complete = false;
        bool sourceCheckpointReused = false;
        bool simulationActive = false;
        bool cpuDrawActive = false;
        bool cpuRendererActive = false;
    };

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
    std::size_t mValidationTick = 0;
    std::vector<RawPadState> mConsumedPads;
    std::vector<std::uint8_t> mCurrentEpisode;
    bool mEpisodePreInputCaptured = false;
    bool mPolicyFeatureRowReady = false;
    bool mCandidateChosenPadReady = false;
    NativePolicyFeatureRow mPolicyFeatureRow{};
    std::array<float, kFactorizedPadPolicyHeadWidth> mPolicyOutput{};
    RawPadState mCandidateChosenPad{};
    FrozenInferenceModel mFrozenPolicyModel;
    LearningEpisodeShardWriter mEpisodeShard;
    std::string mStateDigestMaterial;
    std::vector<std::string> mStateTickDigests;
    std::vector<CandidateResult> mResults;
    std::optional<std::size_t> mWinnerResultIndex;
    std::uint64_t mCaptureMicros = 0;
    std::string mActualSourceBoundaryFingerprint;
    std::string mSourceSemanticDigest;
    std::vector<std::string> mValidationFreshDigests;
    std::string mValidationRestoredDigestMaterial;
    std::string mValidationFreshSequenceDigest;
    std::string mValidationRestoredSequenceDigest;
    std::optional<std::size_t> mValidationFirstDivergence;
    std::string mValidationExpectedDigest;
    std::string mValidationActualDigest;
    std::uint64_t mValidationMicros = 0;
    std::uint64_t mValidationSamples = 0;
    bool mValidationVerified = false;
    std::vector<std::uint64_t> mRestoreMicros;
    BatchProfile mProfile;
    std::string mError;
};

SuffixBatchRunner& suffix_batch_runner();

}  // namespace dusk::automation
