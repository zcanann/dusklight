#include "dusk/automation/suffix_batch_runner.hpp"

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
    mConsumedPads.reserve(mDefinition.maximumTicks);
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
    return true;
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

void SuffixBatchRunner::finishCandidate(
    const MilestoneObservation& observation, const bool success) {
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

    mResults.push_back(std::move(result));
    const std::size_t resultIndex = mResults.size() - 1;
    if (success && (!mWinnerResultIndex.has_value() ||
                       *mResults[resultIndex].firstHitTick <
                           *mResults[*mWinnerResultIndex].firstHitTick))
    {
        mWinnerResultIndex = resultIndex;
    }
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
    if (!success && !exhausted) {
        ++mCandidateTick;
        return false;
    }

    finishCandidate(observation, success);
    ++mCandidateIndex;
    if (mCandidateIndex == mDefinition.candidates.size()) {
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
        {"schema", "dusklight-suffix-batch-result/v1"},
        {"status", mCompleted ? "passed" : mFailed ? "failed" : "incomplete"},
        {"source_frame", mDefinition.sourceFrame},
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
