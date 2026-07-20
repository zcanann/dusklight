#include "dusk/automation/suffix_batch_runner.hpp"

#include "dusk/automation/build_identity.hpp"
#include "dusk/automation/io_mode.hpp"
#include "dusk/automation/gameplay_trace_observer.hpp"
#include "dusk/audio/DuskAudioSystem.h"

#include <chrono>
#include <fstream>
#include <system_error>
#include <utility>

#include <nlohmann/json.hpp>
#include <xxhash.h>

#include <aurora/dvd.h>

namespace dusk::automation {
namespace {

constexpr std::uint64_t LearningTraceChannels =
    gameplay_trace_channel_bit(GameplayTraceChannel::Stage) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Camera) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerAction) |
    gameplay_trace_channel_bit(GameplayTraceChannel::SceneExit) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerBackgroundCollision) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces);

std::string xxh3_128_hex(const std::string_view value) {
    const XXH128_hash_t hash = XXH3_128bits(value.data(), value.size());
    XXH128_canonical_t canonical{};
    XXH128_canonicalFromHash(&canonical, hash);
    constexpr char Hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(sizeof(canonical.digest) * 2);
    for (const unsigned char byte : canonical.digest) {
        output.push_back(Hex[byte >> 4]);
        output.push_back(Hex[byte & 0xf]);
    }
    return output;
}

nlohmann::json pad_json(const RawPadState& pad) {
    return {
        {"buttons", pad.buttons},
        {"stick_x", pad.stickX},
        {"stick_y", pad.stickY},
        {"substick_x", pad.substickX},
        {"substick_y", pad.substickY},
        {"trigger_left", pad.triggerLeft},
        {"trigger_right", pad.triggerRight},
        {"analog_a", pad.analogA},
        {"analog_b", pad.analogB},
        {"connected", has_flag(pad.flags, RawPadFlags::Connected)},
        {"error", pad.error},
    };
}

bool write_atomic(const std::filesystem::path& path, const std::string_view bytes,
    std::string& error) {
    std::error_code filesystemError;
    const std::filesystem::path parent = path.parent_path();
    if (!parent.empty()) {
        std::filesystem::create_directories(parent, filesystemError);
        if (filesystemError) {
            error = "could not create suffix batch output directory: " +
                    filesystemError.message();
            return false;
        }
    }
    const std::filesystem::path temporary = path.string() + ".tmp";
    {
        std::ofstream stream(temporary, std::ios::binary | std::ios::trunc);
        if (!stream || !stream.write(bytes.data(), static_cast<std::streamsize>(bytes.size()))) {
            error = "could not write suffix batch temporary artifact";
            return false;
        }
    }
    std::filesystem::remove(path, filesystemError);
    filesystemError.clear();
    std::filesystem::rename(temporary, path, filesystemError);
    if (filesystemError) {
        error = "could not publish suffix batch artifact: " + filesystemError.message();
        return false;
    }
    return true;
}

}  // namespace

bool SuffixBatchRunner::configure(SuffixBatchDefinition definition,
    std::filesystem::path resultPath, std::filesystem::path winnerTapePath,
    std::string& error) {
    if (mEnabled || definition.candidates.empty() || definition.maximumTicks == 0 ||
        resultPath.empty())
    {
        error = "suffix batch runner configuration is empty or already installed";
        return false;
    }
    mEnabled = true;
    mDefinition = std::move(definition);
    mResultPath = std::move(resultPath);
    mWinnerTapePath = std::move(winnerTapePath);
    mEpisodeShardPath = mResultPath;
    mEpisodeShardPath += ".episodes.dseps";
    mConsumedPads.reserve(mDefinition.maximumTicks);
    mCurrentEpisode.reserve(
        std::min<std::size_t>(mDefinition.maximumTicks * 4096, 16 * 1024 * 1024));
    if (mDefinition.verifyStateHashes)
        mStateDigestMaterial.reserve(mDefinition.maximumTicks * 32);
    mResults.reserve(mDefinition.candidates.size());
    mRestoreMicros.reserve(mDefinition.candidates.size() - 1);
    return true;
}

bool SuffixBatchRunner::captureSource(const std::uint64_t simulationTick,
    const std::uint64_t tapeFrame, const std::uint64_t preparedInputFrame,
    const bool tapeFrameApplied, std::string& error) {
    if (!synchronous_io_enabled() || !aurora_dvd_is_synchronous()) {
        error = "suffix batch capture requires synchronous simulation-thread I/O";
        return false;
    }
    const MilestoneObservation sourceObservation =
        capture_milestone_observation(mSourceMilestoneStorage);
    mActualSourceBoundaryFingerprint = compute_milestone_boundary_fingerprint(
        sourceObservation, input_tape_player().tape().boot);
    const MilestoneDefinition* sourceMilestone =
        find_milestone(MilestoneId::GameplayReadyFSp103);
    if (sourceMilestone == nullptr || !sourceMilestone->predicate(sourceObservation)) {
        error = "suffix batch source does not satisfy gameplay-ready-f-sp103; expected " +
                mDefinition.sourceBoundaryFingerprint + ", observed " +
                mActualSourceBoundaryFingerprint;
        return false;
    }
    if (mActualSourceBoundaryFingerprint != mDefinition.sourceBoundaryFingerprint) {
        error = "suffix batch source boundary fingerprint mismatch; expected " +
                mDefinition.sourceBoundaryFingerprint + ", observed " +
                mActualSourceBoundaryFingerprint;
        return false;
    }
    mAudioCallbackQuiesced = dusk::audio::QuiesceForStateCheckpoint();
    if (!mAudioCallbackQuiesced) {
        error = "could not quiesce host audio callback for suffix batch";
        return false;
    }
    StateCheckpointError checkpointError = register_emulated_machine_checkpoint(mCheckpoint);
    if (checkpointError != StateCheckpointError::None) {
        error = state_checkpoint_error_message(checkpointError);
        return false;
    }
    mSource.tapePlayer = input_tape_player().captureState();
    mSource.milestones = milestone_tracker();
    if (!PADCaptureAutomationState(&mSource.pad)) {
        error = "could not capture suffix batch PAD state";
        return false;
    }
    mSource.simulationTick = simulationTick;
    mSource.tapeFrame = tapeFrame;
    mSource.preparedInputFrame = preparedInputFrame;
    mSource.tapeFrameApplied = tapeFrameApplied;

    const auto start = std::chrono::steady_clock::now();
    checkpointError = mCheckpoint.capture(mImage);
    const auto end = std::chrono::steady_clock::now();
    mCaptureMicros = static_cast<std::uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(end - start).count());
    if (checkpointError != StateCheckpointError::None) {
        error = state_checkpoint_error_message(checkpointError);
        return false;
    }

    const std::array goal{MilestoneId::ExitFSp103ToFSp104};
    if (!mGoalTracker.configure(goal, MilestoneId::ExitFSp103ToFSp104, error)) return false;
    mGoalTracker.setBootOrigin(input_tape_player().tape().boot);

    const BuildIdentity build = current_build_identity("native-read-only-checkpoint-batch");
    const std::string objective(milestone_name(MilestoneId::ExitFSp103ToFSp104));
    std::string objectiveIdentityMaterial(build.revision);
    objectiveIdentityMaterial.push_back('\0');
    objectiveIdentityMaterial += objective;
    LearningEpisodeShardMetadata metadata{
        .sourceFrame = mDefinition.sourceFrame,
        .maximumTicks = mDefinition.maximumTicks,
        .sourceBoundaryFingerprint = mDefinition.sourceBoundaryFingerprint,
        .checkpointIdentity = mImage.digest,
        .objective = objective,
        .objectiveIdentity = xxh3_128_hex(objectiveIdentityMaterial),
        .buildRevision = std::string(build.revision),
        .auroraRevision = std::string(build.auroraRevision),
        .featureDigest = std::string(build.featureDigest),
        .fidelityProfile = std::string(build.fidelityProfile),
        .gameDataIdentity = {},
    };
    if (!mEpisodeShard.begin(mEpisodeShardPath, metadata, error))
        return false;
    return true;
}

LearningGoalObservation summarize_learning_goal(const MilestoneTracker& tracker) {
    LearningGoalObservation result;
    result.configured = tracker.goalConfigured();
    result.reached = tracker.goalReached();
    result.requestedCount = static_cast<std::uint16_t>(std::min<std::size_t>(
        tracker.hits().size() + tracker.authoredHits().size(), 0xffff));
    result.hitCount = static_cast<std::uint16_t>(std::min<std::size_t>(
        std::ranges::count(tracker.hits(), true, &MilestoneHit::hit) +
            std::ranges::count(tracker.authoredHits(), true, &AuthoredMilestoneHit::hit),
        0xffff));
    if (const auto goal = tracker.goal(); goal.has_value()) {
        const auto found = std::ranges::find(tracker.hits(), *goal, &MilestoneHit::id);
        result.stableTicks = 1;
        result.consecutiveTicks = found != tracker.hits().end() && found->hit ? 1 : 0;
        if (found != tracker.hits().end() && found->hit)
            result.firstHitTick = found->simulationTick;
    } else if (const auto name = tracker.goalName(); name.has_value()) {
        const auto found = std::ranges::find(tracker.authoredHits(), *name,
            &AuthoredMilestoneHit::id);
        if (found != tracker.authoredHits().end()) {
            result.stableTicks = found->stableTicks;
            result.consecutiveTicks = found->consecutiveTicks;
            result.sequenceSteps = found->sequenceSteps;
            result.sequenceNextStep = found->sequenceNextStep;
            result.sequenceWithinTicks = found->sequenceWithinTicks;
            result.sequenceElapsedTicks = found->sequenceElapsedTicks;
            if (found->hit)
                result.firstHitTick = found->simulationTick;
        }
    }
    return result;
}

bool SuffixBatchRunner::restoreSource(std::uint64_t& simulationTick,
    std::uint64_t& tapeFrame, std::uint64_t& preparedInputFrame, bool& tapeFrameApplied,
    std::string& error) {
    const auto start = std::chrono::steady_clock::now();
    const StateCheckpointError checkpointError = mCheckpoint.restoreTrusted(mImage);
    const auto end = std::chrono::steady_clock::now();
    mRestoreMicros.push_back(static_cast<std::uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(end - start).count()));
    if (checkpointError != StateCheckpointError::None) {
        error = state_checkpoint_error_message(checkpointError);
        return false;
    }
    if (!input_tape_player().restoreState(mSource.tapePlayer) ||
        !PADRestoreAutomationState(&mSource.pad))
    {
        error = "suffix batch host state restore failed";
        return false;
    }
    milestone_tracker() = mSource.milestones;
    simulationTick = mSource.simulationTick;
    tapeFrame = mSource.tapeFrame;
    preparedInputFrame = mSource.preparedInputFrame;
    tapeFrameApplied = mSource.tapeFrameApplied;
    mGoalTracker.reset();
    mCandidateTick = 0;
    mConsumedPads.clear();
    mStateDigestMaterial.clear();
    mConsumedCaptureFailed = false;
    mEpisodePreInputCaptured = false;
    return true;
}

bool SuffixBatchRunner::captureEpisodePreInput(
    const std::uint64_t simulationTick, std::string& error) {
    if (mEpisodePreInputCaptured || mCandidateTick >= mDefinition.maximumTicks) {
        error = "learning episode pre-input boundary was captured twice or out of range";
        return false;
    }
    if (mCandidateTick == 0)
        begin_learning_episode(mCurrentEpisode);
    const MilestoneObservation observation =
        capture_milestone_observation(mEpisodeMilestoneStorage);
    const ControllerObservation controller =
        capture_controller_observation(mEpisodeControllerStorage);
    const GameplayCollisionCorrectionObservation collision =
        capture_gameplay_collision_correction();
    GameplayTraceSample gameplayTrace;
    if (!capture_gameplay_trace_sample(
            {
                .boundaryIndex = simulationTick,
                .simulationTick = simulationTick,
                .tapeFrame = static_cast<std::uint64_t>(
                    mDefinition.sourceFrame + mCandidateTick),
                .phase = GameplayTracePhase::PreInput,
            },
            LearningTraceChannels, gameplayTrace))
    {
        error = "native learning mechanics observation is unavailable";
        return false;
    }
    const GameplayCollisionPlanesObservation collisionPlanes =
        capture_gameplay_collision_planes();
    const GameplayPlayerFormObservation playerForm = capture_gameplay_player_form();
    RawPadState previousInput{};
    if (mCandidateTick != 0) {
        previousInput = mConsumedPads.back();
    } else if (mSource.pad.active[0]) {
        previousInput = raw_pad_state_from_pad_status(mSource.pad.status[0]);
    }
    const LearningObservationContext context{
        .phase = LearningObservationPhase::PreInput,
        .boundaryIndex = simulationTick,
        .simulationTick = simulationTick,
        .tapeFrame = static_cast<std::uint64_t>(mDefinition.sourceFrame + mCandidateTick),
        .remainingTicks = static_cast<std::uint32_t>(
            mDefinition.maximumTicks - mCandidateTick),
        .stateIdentity = compute_milestone_observation_fingerprint(
            observation, input_tape_player().tape().boot),
        .previousInput = previousInput,
        .cameraPresent = controller.cameraPresent,
        .cameraYawRadians = controller.cameraYawRadians,
        .collisionCorrectionPresent = collision.present,
        .collisionCorrectionX = collision.x,
        .collisionCorrectionZ = collision.z,
        .gameplayTrace = &gameplayTrace,
        .collisionPlanes = collisionPlanes,
        .playerForm = playerForm,
        .goal = summarize_learning_goal(mGoalTracker),
    };
    if (!append_learning_observation(mCurrentEpisode, observation, context, error))
        return false;
    mEpisodePreInputCaptured = true;
    return true;
}

bool SuffixBatchRunner::preInput(std::uint64_t& simulationTick, std::uint64_t& tapeFrame,
    std::uint64_t& preparedInputFrame, bool& tapeFrameApplied, std::string& error) {
    if (!mEnabled || mCompleted || mFailed) return true;
    if (mPhase == Phase::WaitingForSource) {
        const std::size_t nextFrame = input_tape_player().nextFrameIndex();
        if (nextFrame < mDefinition.sourceFrame) return true;
        if (nextFrame != mDefinition.sourceFrame) {
            error = "input tape passed suffix batch source before capture";
            fail(error);
            return false;
        }
        if (!captureSource(simulationTick, tapeFrame, preparedInputFrame, tapeFrameApplied, error)) {
            fail(error);
            return false;
        }
        mPhase = Phase::Candidate;
    } else if (mPhase == Phase::RestoreNext) {
        if (!restoreSource(simulationTick, tapeFrame, preparedInputFrame, tapeFrameApplied, error)) {
            fail(error);
            return false;
        }
        mPhase = Phase::Candidate;
    }
    if (mPhase == Phase::Candidate && !captureEpisodePreInput(simulationTick, error)) {
        fail(error);
        return false;
    }
    return true;
}

void SuffixBatchRunner::applyCandidateInput() {
    if (!mEnabled || mPhase != Phase::Candidate || mFailed || mCompleted ||
        mCandidateIndex >= mDefinition.candidates.size() ||
        mCandidateTick >= mDefinition.maximumTicks)
        return;
    if (mDefinition.candidates[mCandidateIndex].tapePassthrough) return;
    const PADStatus status = raw_pad_state_to_pad_status(
        mDefinition.candidates[mCandidateIndex].pads[mCandidateTick]);
    PADSetAutomationStatus(0, &status);
}

void SuffixBatchRunner::recordConsumedPads(
    const std::span<const PADStatus, kInputPortCount> statuses) {
    if (!mEnabled || mPhase != Phase::Candidate || mFailed || mCompleted) return;
    if (statuses.size() != kInputPortCount || mConsumedPads.size() != mCandidateTick) {
        mConsumedCaptureFailed = true;
        return;
    }
    mConsumedPads.push_back(raw_pad_state_from_pad_status(statuses[0]));
}

bool SuffixBatchRunner::appendEpisodePostSimulation(const MilestoneObservation& observation,
    const RawPadState& chosenPad, const std::uint64_t simulationTick, const bool terminal,
    std::string& error) {
    if (!mEpisodePreInputCaptured || mConsumedPads.size() != mCandidateTick + 1) {
        error = "learning episode post-simulation boundary lacks its pre-input action";
        return false;
    }
    append_learning_action(mCurrentEpisode, chosenPad, mConsumedPads.back());
    const ControllerObservation controller =
        capture_controller_observation(mEpisodeControllerStorage);
    const GameplayCollisionCorrectionObservation collision =
        capture_gameplay_collision_correction();
    GameplayTraceSample gameplayTrace;
    if (!capture_gameplay_trace_sample(
            {
                .boundaryIndex = simulationTick + 1,
                .simulationTick = simulationTick,
                .tapeFrame = static_cast<std::uint64_t>(
                    mDefinition.sourceFrame + mCandidateTick),
                .phase = GameplayTracePhase::PostSimulation,
            },
            LearningTraceChannels, gameplayTrace))
    {
        error = "native learning mechanics observation is unavailable";
        return false;
    }
    const GameplayCollisionPlanesObservation collisionPlanes =
        capture_gameplay_collision_planes();
    const GameplayPlayerFormObservation playerForm = capture_gameplay_player_form();
    const LearningObservationContext context{
        .phase = LearningObservationPhase::PostSimulation,
        .terminalReason = !terminal ? LearningTerminalReason::None :
            (mGoalTracker.goalReached() ? LearningTerminalReason::GoalReached :
                                         LearningTerminalReason::TickBudgetExhausted),
        .boundaryIndex = simulationTick + 1,
        .simulationTick = simulationTick,
        .tapeFrame = static_cast<std::uint64_t>(mDefinition.sourceFrame + mCandidateTick),
        .remainingTicks = static_cast<std::uint32_t>(
            mDefinition.maximumTicks - (mCandidateTick + 1)),
        .stateIdentity = compute_milestone_observation_fingerprint(
            observation, input_tape_player().tape().boot),
        .previousInput = mConsumedPads.back(),
        .cameraPresent = controller.cameraPresent,
        .cameraYawRadians = controller.cameraYawRadians,
        .collisionCorrectionPresent = collision.present,
        .collisionCorrectionX = collision.x,
        .collisionCorrectionZ = collision.z,
        .gameplayTrace = &gameplayTrace,
        .collisionPlanes = collisionPlanes,
        .playerForm = playerForm,
        .goal = summarize_learning_goal(mGoalTracker),
    };
    if (!append_learning_observation(mCurrentEpisode, observation, context, error))
        return false;
    mEpisodePreInputCaptured = false;
    if (!terminal)
        return true;
    return finish_learning_episode(
        mCurrentEpisode, static_cast<std::uint32_t>(mCandidateTick + 1), error);
}

bool SuffixBatchRunner::finishCandidate(
    const MilestoneObservation& observation, const bool success, std::string& error) {
    CandidateResult result;
    result.id = mDefinition.candidates[mCandidateIndex].id;
    result.success = success;
    result.ticksExecuted = mCandidateTick + 1;
    if (success) result.firstHitTick = mCandidateTick;
    if (mDefinition.verifyStateHashes)
        result.stateSequenceDigest = xxh3_128_hex(mStateDigestMaterial);
    result.predicateEvidence = serialize_milestone_result(mGoalTracker);

    const ControllerObservation controller = capture_controller_observation(mControllerStorage);
    auto& terminal = result.terminal;
    terminal.stage = observation.stageName == nullptr ? "" : observation.stageName;
    terminal.room = observation.room;
    terminal.point = observation.point;
    terminal.positionX = observation.playerPositionX;
    terminal.positionY = observation.playerPositionY;
    terminal.positionZ = observation.playerPositionZ;
    terminal.velocityX = observation.playerVelocityX;
    terminal.velocityY = observation.playerVelocityY;
    terminal.velocityZ = observation.playerVelocityZ;
    terminal.forwardSpeed = observation.playerForwardSpeed;
    terminal.facing = observation.playerCurrentAngleY;
    terminal.procedure = observation.playerProcId;
    terminal.modeFlags = observation.playerModeFlags;
    terminal.cameraPresent = controller.cameraPresent;
    terminal.cameraHeadingRadians = controller.cameraYawRadians;
    terminal.transitionPending = observation.nextStageEnabled;
    terminal.nextStage = observation.nextStageName;
    terminal.nextRoom = observation.nextRoom;
    terminal.nextPoint = observation.nextPoint;
    const GameplayCollisionCorrectionObservation collision =
        capture_gameplay_collision_correction();
    terminal.collisionCorrectionPresent = collision.present;
    terminal.collisionCorrectionX = collision.x;
    terminal.collisionCorrectionZ = collision.z;
    terminal.previousInput = mConsumedPads.back();
    if (success) result.successfulConsumedPads = mConsumedPads;

    const LearningEpisodeDescriptor episode{
        .id = result.id,
        .success = success,
        .ticksExecuted = static_cast<std::uint32_t>(result.ticksExecuted),
        .firstHitTick = result.firstHitTick.has_value() ?
            std::optional<std::uint32_t>(static_cast<std::uint32_t>(*result.firstHitTick)) :
            std::nullopt,
        .remainingTicks = static_cast<std::uint32_t>(
            mDefinition.maximumTicks - result.ticksExecuted),
    };
    if (!mEpisodeShard.append(episode, mCurrentEpisode, error))
        return false;

    mResults.push_back(std::move(result));
    const std::size_t resultIndex = mResults.size() - 1;
    if (success && (!mWinnerResultIndex.has_value() ||
                       *mResults[resultIndex].firstHitTick <
                           *mResults[*mWinnerResultIndex].firstHitTick))
    {
        mWinnerResultIndex = resultIndex;
    }
    return true;
}

bool SuffixBatchRunner::postSimulation(const std::uint64_t simulationTick,
    const std::uint64_t tapeFrame, std::string& error) {
    if (!mEnabled || mPhase != Phase::Candidate || mCompleted || mFailed) return false;
    const auto& candidate = mDefinition.candidates[mCandidateIndex];
    const RawPadState& expectedPad = candidate.tapePassthrough
        ? input_tape_player().tape().frames[mDefinition.sourceFrame + mCandidateTick].pads[0]
        : candidate.pads[mCandidateTick];
    if (mConsumedCaptureFailed || mConsumedPads.size() != mCandidateTick + 1 ||
        mConsumedPads.back() != expectedPad)
    {
        error = "candidate PAD state was not consumed exactly at the input boundary";
        fail(error);
        return true;
    }
    if (mDefinition.verifyStateHashes) {
        std::string digest;
        const StateCheckpointError checkpointError = mCheckpoint.currentDigest(digest);
        if (checkpointError != StateCheckpointError::None) {
            error = state_checkpoint_error_message(checkpointError);
            fail(error);
            return true;
        }
        mStateDigestMaterial += digest;
    }

    const MilestoneObservation observation = capture_milestone_observation(mMilestoneStorage);
    mGoalTracker.observe(observation, simulationTick, tapeFrame);
    const bool success = mGoalTracker.goalReached();
    const bool exhausted = mCandidateTick + 1 == mDefinition.maximumTicks;
    if (!appendEpisodePostSimulation(
            observation, expectedPad, simulationTick, success || exhausted, error)) {
        fail(error);
        return true;
    }
    if (!success && !exhausted) {
        ++mCandidateTick;
        return false;
    }

    if (!finishCandidate(observation, success, error)) {
        fail(error);
        return true;
    }
    ++mCandidateIndex;
    if (mCandidateIndex == mDefinition.candidates.size()) {
        if (!mEpisodeShard.finish(error)) {
            fail(error);
            return true;
        }
        mPhase = Phase::Complete;
        mCompleted = true;
        return true;
    }
    mPhase = Phase::RestoreNext;
    return false;
}

void SuffixBatchRunner::fail(std::string message) {
    mFailed = true;
    mPhase = Phase::Failed;
    mError = std::move(message);
}

bool SuffixBatchRunner::writeArtifacts(std::string& error) const {
    if (!mEnabled) return true;
    nlohmann::json candidates = nlohmann::json::array();
    for (const CandidateResult& result : mResults) {
        nlohmann::json consumed = nullptr;
        if (result.success) {
            consumed = nlohmann::json::array();
            for (const RawPadState& pad : result.successfulConsumedPads)
                consumed.push_back(pad_json(pad));
        }
        const auto& terminal = result.terminal;
        candidates.push_back({
            {"id", result.id},
            {"success", result.success},
            {"ticks_executed", result.ticksExecuted},
            {"first_hit_tick", result.firstHitTick.has_value()
                    ? nlohmann::json(*result.firstHitTick) : nlohmann::json(nullptr)},
            {"state_sequence_digest", result.stateSequenceDigest.empty()
                    ? nlohmann::json(nullptr) : nlohmann::json(result.stateSequenceDigest)},
            {"predicate_evidence", nlohmann::json::parse(result.predicateEvidence)},
            {"consumed_pad_states", std::move(consumed)},
            {"terminal_observation", {
                {"stage", terminal.stage}, {"room", terminal.room},
                {"point", terminal.point},
                {"position", {terminal.positionX, terminal.positionY, terminal.positionZ}},
                {"velocity", {terminal.velocityX, terminal.velocityY, terminal.velocityZ}},
                {"forward_speed", terminal.forwardSpeed}, {"facing", terminal.facing},
                {"procedure", terminal.procedure}, {"mode_flags", terminal.modeFlags},
                {"camera_heading_radians", terminal.cameraPresent
                        ? nlohmann::json(terminal.cameraHeadingRadians) : nlohmann::json(nullptr)},
                {"transition_pending", terminal.transitionPending},
                {"next_stage", terminal.nextStage}, {"next_room", terminal.nextRoom},
                {"next_point", terminal.nextPoint},
                {"collision_correction", terminal.collisionCorrectionPresent
                        ? nlohmann::json::array({terminal.collisionCorrectionX,
                              terminal.collisionCorrectionZ}) : nlohmann::json(nullptr)},
                {"previous_input", pad_json(terminal.previousInput)},
            }},
        });
    }
    nlohmann::json result{
        {"schema", "dusklight-suffix-batch-result/v2"},
        {"status", mCompleted ? "passed" : mFailed ? "failed" : "incomplete"},
        {"source_frame", mDefinition.sourceFrame},
        {"source_boundary", {
            {"milestone", milestone_name(MilestoneId::GameplayReadyFSp103)},
            {"expected_fingerprint", mDefinition.sourceBoundaryFingerprint},
            {"actual_fingerprint", mActualSourceBoundaryFingerprint.empty()
                    ? nlohmann::json(nullptr) :
                      nlohmann::json(mActualSourceBoundaryFingerprint)},
            {"verified", !mActualSourceBoundaryFingerprint.empty() &&
                    mActualSourceBoundaryFingerprint ==
                        mDefinition.sourceBoundaryFingerprint},
        }},
        {"maximum_ticks", mDefinition.maximumTicks},
        {"candidate_count", mDefinition.candidates.size()},
        {"completed_candidates", mResults.size()},
        {"verify_state_hashes", mDefinition.verifyStateHashes},
        {"checkpoint_bytes", mCheckpoint.byteCount()},
        {"restore_identity", mImage.digest.empty() ? nlohmann::json(nullptr)
                                                      : nlohmann::json(mImage.digest)},
        {"capture_micros", mCaptureMicros},
        {"restore_micros", mRestoreMicros},
        {"audio_callback_quiesced", mAudioCallbackQuiesced},
        {"episode_shard", {
            {"schema", LearningEpisodeShardSchema},
            {"path", mEpisodeShardPath.string()},
            {"observation_schema", LearningObservationSchema},
            {"action_schema", LearningActionSchema},
            {"episode_count", mEpisodeShard.episodeCount()},
            {"uncompressed_bytes", mEpisodeShard.uncompressedBytes()},
            {"compressed_bytes", mEpisodeShard.compressedBytes()},
        }},
        {"winner_id", mWinnerResultIndex.has_value()
                ? nlohmann::json(mResults[*mWinnerResultIndex].id) : nlohmann::json(nullptr)},
        {"candidates", std::move(candidates)},
        {"error", mError.empty() ? nlohmann::json(nullptr) : nlohmann::json(mError)},
    };
    if (!write_atomic(mResultPath, result.dump(2) + '\n', error)) return false;

    if (mWinnerTapePath.empty() || !mWinnerResultIndex.has_value()) return true;
    InputTape tape;
    tape.frames.reserve(mResults[*mWinnerResultIndex].successfulConsumedPads.size());
    for (const RawPadState& pad : mResults[*mWinnerResultIndex].successfulConsumedPads) {
        InputFrame frame;
        frame.ownedPorts = 1;
        frame.pads[0] = pad;
        for (std::size_t port = 1; port < frame.pads.size(); ++port) {
            frame.pads[port].flags = RawPadFlags::None;
            frame.pads[port].error = PAD_ERR_NO_CONTROLLER;
        }
        tape.frames.push_back(frame);
    }
    std::vector<std::uint8_t> bytes;
    const InputTapeError tapeError = encode_input_tape(tape, bytes);
    if (tapeError != InputTapeError::None) {
        error = input_tape_error_message(tapeError);
        return false;
    }
    return write_atomic(mWinnerTapePath,
        std::string_view(reinterpret_cast<const char*>(bytes.data()), bytes.size()), error);
}

SuffixBatchRunner& suffix_batch_runner() {
    static SuffixBatchRunner runner;
    return runner;
}

}  // namespace dusk::automation
