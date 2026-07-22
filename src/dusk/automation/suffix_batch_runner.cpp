#include "dusk/automation/suffix_batch_runner.hpp"

#include "dusk/automation/build_identity.hpp"
#include "dusk/automation/actor_profile_catalog.hpp"
#include "dusk/automation/card_fixture.hpp"
#include "dusk/automation/io_mode.hpp"
#include "dusk/automation/gameplay_trace_observer.hpp"
#include "dusk/audio/DuskAudioSystem.h"

#include <algorithm>
#include <chrono>
#include <fstream>
#include <numeric>
#include <system_error>
#include <utility>

#include <nlohmann/json.hpp>
#include <xxhash.h>

#include <aurora/dvd.h>
#include <aurora/gfx.h>

namespace dusk::automation {
namespace {

using ProfileClock = std::chrono::steady_clock;

std::uint64_t elapsed_micros(const ProfileClock::time_point start) {
    const auto elapsed = std::chrono::duration_cast<std::chrono::microseconds>(
        ProfileClock::now() - start).count();
    return elapsed < 0 ? 0 : static_cast<std::uint64_t>(elapsed);
}

std::uint64_t elapsed_nanos(const ProfileClock::time_point start) {
    const auto elapsed = std::chrono::duration_cast<std::chrono::nanoseconds>(
        ProfileClock::now() - start).count();
    return elapsed < 0 ? 0 : static_cast<std::uint64_t>(elapsed);
}

class AccumulateMicros {
public:
    explicit AccumulateMicros(std::uint64_t& destination) : mDestination(destination) {}
    ~AccumulateMicros() { mDestination += elapsed_micros(mStart); }

private:
    std::uint64_t& mDestination;
    ProfileClock::time_point mStart = ProfileClock::now();
};

class AccumulateNanos {
public:
    explicit AccumulateNanos(std::uint64_t& destination) : mDestination(destination) {}
    ~AccumulateNanos() { mDestination += elapsed_nanos(mStart); }

private:
    std::uint64_t& mDestination;
    ProfileClock::time_point mStart = ProfileClock::now();
};

constexpr std::uint64_t LearningTraceChannels =
    gameplay_trace_channel_bit(GameplayTraceChannel::Stage) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Camera) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerAction) |
    gameplay_trace_channel_bit(GameplayTraceChannel::SceneExit) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerBackgroundCollision) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces);

bool is_lower_hex(const std::string_view value, const std::size_t width) {
    return value.size() == width && std::ranges::all_of(value, [](const char byte) {
        return (byte >= '0' && byte <= '9') || (byte >= 'a' && byte <= 'f');
    });
}

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

std::string digest_hex(const std::span<const std::uint8_t> bytes) {
    constexpr char Hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(bytes.size() * 2);
    for (const std::uint8_t byte : bytes) {
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

bool SuffixBatchRunner::loadFrozenPolicy(const SuffixBatchDefinition& definition,
    FrozenInferenceModel& output, std::string& error) const {
    output = {};
    if (!definition.frozenPolicy.has_value()) return true;
    constexpr std::uintmax_t MaximumFrozenPolicyBytes = 70 * 1024 * 1024;
    const auto& policy = *definition.frozenPolicy;
    std::error_code filesystemError;
    const std::filesystem::path path(policy.modelPath);
    const std::uintmax_t size = std::filesystem::file_size(path, filesystemError);
    if (filesystemError || size == 0 || size > MaximumFrozenPolicyBytes) {
        error = "frozen policy model is missing, empty, or exceeds 70 MiB";
        return false;
    }
    std::vector<std::uint8_t> bytes(static_cast<std::size_t>(size));
    std::ifstream stream(path, std::ios::binary);
    if (!stream || !stream.read(reinterpret_cast<char*>(bytes.data()),
                       static_cast<std::streamsize>(bytes.size())))
    {
        error = "frozen policy model could not be read completely";
        return false;
    }
    const std::string_view byteView(
        reinterpret_cast<const char*>(bytes.data()), bytes.size());
    if (xxh3_128_hex(byteView) != policy.modelXxh3_128) {
        error = "frozen policy model content identity differs from the batch";
        return false;
    }
    FrozenInferenceModel parsed;
    if (!parsed.decode(bytes, error)) {
        error = "frozen policy model is invalid: " + error;
        return false;
    }
    if (parsed.inputWidth() != kNativePolicyFeatureWidth ||
        parsed.featureSchemaSha256() != kNativePolicyFeatureSchemaDigest ||
        parsed.actionSchemaSha256() != kOnlineFactorizedPadActionSchemaDigest ||
        parsed.actions().size() != kFactorizedPadPolicyHeadWidth)
    {
        error = "frozen policy model feature or action schema is incompatible";
        return false;
    }
    for (std::size_t index = 0; index < parsed.actions().size(); ++index) {
        if (parsed.actions()[index] != index) {
            error = "frozen policy model action IDs are not the canonical output coordinates";
            return false;
        }
    }
    output = std::move(parsed);
    return true;
}

bool SuffixBatchRunner::configure(SuffixBatchDefinition definition,
    std::filesystem::path resultPath, std::filesystem::path winnerTapePath,
    std::string gameDataSha256, std::string worldContextSha256, std::string& error) {
    const bool validValidation =
        (definition.checkpointValidation == SuffixCheckpointValidation::GameplayReadyFSp103 &&
            definition.validationTicks == 0) ||
        (definition.checkpointValidation == SuffixCheckpointValidation::RecordedReplayWindow &&
            definition.validationTicks != 0 &&
            definition.validationTicks <= SuffixBatchMaximumValidationTicks);
    if (mEnabled || !validValidation || definition.candidates.empty() ||
        definition.maximumTicks == 0 || resultPath.empty() || !is_lower_hex(gameDataSha256, 64) ||
        !is_lower_hex(worldContextSha256, 64))
    {
        error = "suffix batch runner configuration is empty or already installed";
        return false;
    }
    FrozenInferenceModel frozenPolicy;
    if (!loadFrozenPolicy(definition, frozenPolicy, error)) return false;
    mEnabled = true;
    mDefinition = std::move(definition);
    mFrozenPolicyModel = std::move(frozenPolicy);
    mResultPath = std::move(resultPath);
    mWinnerTapePath = std::move(winnerTapePath);
    mGameDataSha256 = std::move(gameDataSha256);
    mWorldContextSha256 = std::move(worldContextSha256);
    mEpisodeShardPath = mResultPath;
    mEpisodeShardPath += ".episodes.dseps";
    mConsumedPads.reserve(mDefinition.maximumTicks);
    mCurrentEpisode.reserve(
        std::min<std::size_t>(mDefinition.maximumTicks * 4096, 16 * 1024 * 1024));
    if (mDefinition.verifyStateHashes) {
        mStateDigestMaterial.reserve(mDefinition.maximumTicks * 32);
        mStateTickDigests.reserve(mDefinition.maximumTicks);
    }
    mResults.reserve(mDefinition.candidates.size());
    mRestoreMicros.reserve(mDefinition.candidates.size() - 1);
    return true;
}

bool SuffixBatchRunner::configureNextBatch(SuffixBatchDefinition definition,
    std::filesystem::path resultPath, std::filesystem::path winnerTapePath,
    std::string& error) {
    error.clear();
    if (!mEnabled || !mCompleted || mFailed || !mArtifactsWritten || !mValidationVerified ||
        mImage.entries.empty() || mEpisodeShard.active() || definition.candidates.empty() ||
        definition.maximumTicks == 0 || definition.sourceFrame != mDefinition.sourceFrame ||
        definition.sourceBoundaryFingerprint != mDefinition.sourceBoundaryFingerprint ||
        definition.checkpointValidation != mDefinition.checkpointValidation ||
        definition.validationTicks != mDefinition.validationTicks || resultPath.empty())
    {
        error = "next suffix batch is incompatible with the authenticated session source";
        return false;
    }

    FrozenInferenceModel frozenPolicy;
    if (!loadFrozenPolicy(definition, frozenPolicy, error)) return false;
    mDefinition = std::move(definition);
    mFrozenPolicyModel = std::move(frozenPolicy);
    mResultPath = std::move(resultPath);
    mWinnerTapePath = std::move(winnerTapePath);
    mEpisodeShardPath = mResultPath;
    mEpisodeShardPath += ".episodes.dseps";
    mCandidateIndex = 0;
    mCandidateTick = 0;
    mConsumedPads.clear();
    mConsumedPads.reserve(mDefinition.maximumTicks);
    mCurrentEpisode.clear();
    mCurrentEpisode.reserve(
        std::min<std::size_t>(mDefinition.maximumTicks * 4096, 16 * 1024 * 1024));
    mStateDigestMaterial.clear();
    mStateTickDigests.clear();
    if (mDefinition.verifyStateHashes) {
        mStateDigestMaterial.reserve(mDefinition.maximumTicks * 32);
        mStateTickDigests.reserve(mDefinition.maximumTicks);
    }
    mResults.clear();
    mResults.reserve(mDefinition.candidates.size());
    mWinnerResultIndex.reset();
    mRestoreMicros.clear();
    mRestoreMicros.reserve(mDefinition.candidates.size());
    mConsumedCaptureFailed = false;
    mEpisodePreInputCaptured = false;
    mPolicyFeatureRowReady = false;
    mCandidateChosenPadReady = false;
    resetBatchProfile(true);
    mError.clear();
    mCompleted = false;
    mArtifactsWritten = false;
    mPhase = Phase::RestoreNext;
    if (!beginEpisodeShard(error)) {
        fail(error);
        return false;
    }
    return true;
}

bool SuffixBatchRunner::captureSource(const std::uint64_t simulationTick,
    const std::uint64_t tapeFrame, const std::uint64_t preparedInputFrame,
    const bool tapeFrameApplied, std::string& error) {
    if (!synchronous_io_enabled() || !aurora_dvd_is_synchronous()) {
        error = "suffix batch capture requires synchronous simulation-thread I/O";
        return false;
    }
    resetBatchProfile(false);
    const MilestoneObservation sourceObservation =
        capture_milestone_observation(mSourceMilestoneStorage);
    mActualSourceBoundaryFingerprint =
        compute_milestone_boundary_fingerprint(sourceObservation, input_tape_player().tape().boot);
    if (mDefinition.checkpointValidation == SuffixCheckpointValidation::GameplayReadyFSp103) {
        const MilestoneDefinition* sourceMilestone =
            find_milestone(MilestoneId::GameplayReadyFSp103);
        if (sourceMilestone == nullptr || !sourceMilestone->predicate(sourceObservation)) {
            error = "suffix batch source does not satisfy gameplay-ready-f-sp103; expected " +
                    mDefinition.sourceBoundaryFingerprint + ", observed " +
                    mActualSourceBoundaryFingerprint + "; stage=" +
                    (sourceObservation.stageName == nullptr ? std::string{"<null>"} :
                                                              sourceObservation.stageName) +
                    ", room=" + std::to_string(sourceObservation.room) +
                    ", point=" + std::to_string(sourceObservation.point) +
                    ", player_present=" + (sourceObservation.playerPresent ? "true" : "false") +
                    ", player_is_link=" + (sourceObservation.playerIsLink ? "true" : "false") +
                    ", event_running=" + (sourceObservation.eventRunning ? "true" : "false") +
                    ", event_id=" + std::to_string(sourceObservation.eventId);
            return false;
        }
    } else if (mDefinition.validationTicks == 0 ||
               mDefinition.sourceFrame > input_tape_player().frameCount() ||
               mDefinition.validationTicks >
                   input_tape_player().frameCount() - mDefinition.sourceFrame)
    {
        error = "suffix batch replay validation window exceeds the source tape";
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
    checkpointError = mCheckpoint.currentSemanticDigest(mSourceSemanticDigest);
    if (checkpointError != StateCheckpointError::None) {
        error = state_checkpoint_error_message(checkpointError);
        return false;
    }

    const MilestoneTracker& configuredTracker = milestone_tracker();
    if (configuredTracker.goalConfigured()) {
        mGoalTracker = configuredTracker;
        mGoalTracker.reset();
        if (!mGoalTracker.goal().has_value()) {
            const auto goalName = mGoalTracker.goalName();
            const auto authored = goalName.has_value()
                ? std::ranges::find(mGoalTracker.authoredHits(), *goalName,
                      &AuthoredMilestoneHit::id)
                : mGoalTracker.authoredHits().end();
            if (authored == mGoalTracker.authoredHits().end() ||
                authored->phase != MilestoneProgramPhase::PostSim ||
                authored->definitionDigest.size() != 64 ||
                authored->programDigest.size() != 64)
            {
                error = "suffix batch authored goals must be exact post-simulation definitions";
                return false;
            }
        }
    } else {
        const std::array goal{MilestoneId::ExitFSp103ToFSp104};
        if (!mGoalTracker.configure(goal, MilestoneId::ExitFSp103ToFSp104, error)) return false;
    }
    mGoalTracker.setBootOrigin(input_tape_player().tape().boot);

    if (mFrozenPolicyModel.loaded()) {
        const auto goalName = mGoalTracker.goalName();
        const auto authored = goalName.has_value()
            ? std::ranges::find(mGoalTracker.authoredHits(), *goalName,
                  &AuthoredMilestoneHit::id)
            : mGoalTracker.authoredHits().end();
        if (mGoalTracker.goal().has_value() || authored == mGoalTracker.authoredHits().end() ||
            digest_hex(mFrozenPolicyModel.objectiveSha256()) != authored->definitionDigest)
        {
            error = "frozen policy objective is not the exact authored goal definition";
            return false;
        }
    }

    return true;
}

bool SuffixBatchRunner::beginEpisodeShard(std::string& error) {
    const BuildIdentity build = current_build_identity("native-read-only-checkpoint-batch");
    const std::string objective(*mGoalTracker.goalName());
    std::string objectiveIdentityMaterial;
    if (mGoalTracker.goal().has_value()) {
        objectiveIdentityMaterial = "builtin-milestone";
        objectiveIdentityMaterial.push_back('\0');
        objectiveIdentityMaterial += build.revision;
        objectiveIdentityMaterial.push_back('\0');
        objectiveIdentityMaterial += objective;
    } else {
        const auto authored = std::ranges::find(mGoalTracker.authoredHits(), objective,
            &AuthoredMilestoneHit::id);
        objectiveIdentityMaterial = "authored-milestone";
        objectiveIdentityMaterial.push_back('\0');
        objectiveIdentityMaterial += authored->programDigest;
        objectiveIdentityMaterial.push_back('\0');
        objectiveIdentityMaterial += authored->definitionDigest;
    }
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
        .gameDataSha256 = mGameDataSha256,
        .cardFixtureIdentity = std::string(active_automation_card_fixture_identity()),
        .actorProfileCatalogIdentity = std::string(actor_profile_catalog_identity()),
        .worldContextSha256 = mWorldContextSha256,
        .policyModelSchema = mFrozenPolicyModel.loaded() ?
            "dusklight-frozen-inference/v1" : "",
        .policyModelXxh3_128 = mFrozenPolicyModel.loaded() ?
            mDefinition.frozenPolicy->modelXxh3_128 : "",
        .policyFeatureSchemaSha256 = mFrozenPolicyModel.loaded() ?
            digest_hex(mFrozenPolicyModel.featureSchemaSha256()) : "",
        .policyActionSchemaSha256 = mFrozenPolicyModel.loaded() ?
            digest_hex(mFrozenPolicyModel.actionSchemaSha256()) : "",
        .policyObjectiveSha256 = mFrozenPolicyModel.loaded() ?
            digest_hex(mFrozenPolicyModel.objectiveSha256()) : "",
        .policyFeatureWidth = mFrozenPolicyModel.loaded() ?
            static_cast<std::uint32_t>(mFrozenPolicyModel.inputWidth()) : 0,
    };
    return mEpisodeShard.begin(mEpisodeShardPath, metadata, error);
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
    std::string restoredSemanticDigest;
    const StateCheckpointError semanticError =
        mCheckpoint.currentSemanticDigest(restoredSemanticDigest);
    if (semanticError != StateCheckpointError::None) {
        error = state_checkpoint_error_message(semanticError);
        return false;
    }
    if (restoredSemanticDigest != mSourceSemanticDigest) {
        error = "suffix checkpoint restore does not reproduce its source semantic identity; "
                "expected " +
                mSourceSemanticDigest + ", observed " + restoredSemanticDigest;
        return false;
    }
    mGoalTracker.reset();
    mCandidateTick = 0;
    mConsumedPads.clear();
    mStateDigestMaterial.clear();
    mStateTickDigests.clear();
    mConsumedCaptureFailed = false;
    mEpisodePreInputCaptured = false;
    return true;
}

bool SuffixBatchRunner::captureValidationTickDigest(const std::uint64_t simulationTick,
    const std::uint64_t tapeFrame, const std::uint64_t preparedInputFrame,
    const bool tapeFrameApplied, std::string& output, std::string& error) {
    const auto start = ProfileClock::now();
    std::string machine;
    const StateCheckpointError checkpointError = mCheckpoint.currentSemanticDigest(machine);
    if (checkpointError != StateCheckpointError::None) {
        error = state_checkpoint_error_message(checkpointError);
        return false;
    }
    const InputTapePlayerState player = input_tape_player().captureState();
    PADAutomationState pad{};
    if (!PADCaptureAutomationState(&pad)) {
        error = "could not capture suffix replay-validation PAD state";
        return false;
    }
    const MilestoneObservation observation = capture_milestone_observation(mSourceMilestoneStorage);
    const std::string replay =
        compute_milestone_observation_fingerprint(observation, milestone_tracker().bootOrigin());
    if (replay.empty()) {
        error = "could not fingerprint suffix replay-validation gameplay state";
        return false;
    }
    nlohmann::json padState = nlohmann::json::array();
    for (std::uint32_t port = 0; port < PAD_CHANMAX; ++port) {
        const PADStatus& status = pad.status[port];
        padState.push_back({
            {"active", pad.active[port] != FALSE},
            {"button", status.button},
            {"stick_x", status.stickX},
            {"stick_y", status.stickY},
            {"substick_x", status.substickX},
            {"substick_y", status.substickY},
            {"trigger_left", status.triggerLeft},
            {"trigger_right", status.triggerRight},
            {"analog_a", status.analogA},
            {"analog_b", status.analogB},
            {"error", status.err},
#ifdef TARGET_PC
            {"extended_button", status.extButton},
#endif
        });
    }
    const std::string material =
        nlohmann::json{
            {"machine", machine},
            {"simulation_tick", simulationTick},
            {"tape_frame", tapeFrame},
            {"prepared_input_frame", preparedInputFrame},
            {"tape_frame_applied", tapeFrameApplied},
            {"player_next_frame", player.nextFrame},
            {"player_owned_ports", player.ownedPorts},
            {"player_end_behavior", static_cast<unsigned>(player.endBehavior)},
            {"player_playing", player.playing},
            {"player_release_pending", player.releasePending},
            {"player_condition_wait", player.conditionWaitTicks},
            {"player_condition_pulse_neutral", player.conditionPulseNeutral},
            {"player_playback_error", static_cast<unsigned>(player.playbackError)},
            {"player_failed_frame", player.failedFrame},
            {"player_failed_condition", static_cast<unsigned>(player.failedCondition)},
            {"pad", std::move(padState)},
            {"replay", replay},
            {"milestones", serialize_milestone_result(milestone_tracker())},
        }
            .dump();
    output = xxh3_128_hex(material);
    mValidationMicros += elapsed_micros(start);
    ++mValidationSamples;
    return true;
}

bool SuffixBatchRunner::captureEpisodePreInput(
    const std::uint64_t simulationTick, std::string& error) {
    if (mEpisodePreInputCaptured || mCandidateTick >= mDefinition.maximumTicks) {
        error = "learning episode pre-input boundary was captured twice or out of range";
        return false;
    }
    mPolicyFeatureRowReady = false;
    mCandidateChosenPadReady = false;
    if (mCandidateTick == 0) {
        AccumulateMicros encoding(mProfile.corpusEncodingMicros);
        begin_learning_episode(mCurrentEpisode);
    }
    MilestoneObservation observation;
    ControllerObservation controller;
    GameplayCollisionCorrectionObservation collision;
    GameplayTraceSample gameplayTrace;
    {
        AccumulateMicros capture(mProfile.observationCaptureMicros);
        ++mProfile.observationCaptureSamples;
        observation = capture_milestone_observation(mEpisodeMilestoneStorage);
        controller = capture_controller_observation(mEpisodeControllerStorage);
        collision = capture_gameplay_collision_correction();
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
    }
    GameplayCollisionPlanesObservation collisionPlanes;
    GameplayPlayerFormObservation playerForm;
    {
        AccumulateMicros capture(mProfile.observationCaptureMicros);
        collisionPlanes = capture_gameplay_collision_planes();
        playerForm = capture_gameplay_player_form();
    }
    RawPadState previousInput{};
    if (mCandidateTick != 0) {
        previousInput = mConsumedPads.back();
    } else if (mSource.pad.active[0]) {
        previousInput = raw_pad_state_from_pad_status(mSource.pad.status[0]);
    }
    if (mDefinition.candidates[mCandidateIndex].frozenPolicy) {
        const NativePolicyFeatureInput policyInput{
            .playerPresent = observation.playerPresent,
            .playerIsLink = observation.playerIsLink,
            .playerPosition = {observation.playerPositionX, observation.playerPositionY,
                observation.playerPositionZ},
            .playerVelocity = {observation.playerVelocityX, observation.playerVelocityY,
                observation.playerVelocityZ},
            .playerForwardSpeed = observation.playerForwardSpeed,
            .playerCurrentYaw = observation.playerCurrentAngleY,
            .playerShapeYaw = observation.playerShapeAngleY,
            .playerGroundContact = observation.playerGroundContact,
            .playerWallContact = observation.playerWallContact,
            .playerRoofContact = observation.playerRoofContact,
            .playerWaterContact = observation.playerWaterContact,
            .playerWaterIn = observation.playerWaterIn,
            .playerGroundHeightPresent = observation.playerGroundHeightPresent,
            .playerGroundHeight = observation.playerGroundHeight,
            .playerRoofHeightPresent = observation.playerRoofHeightPresent,
            .playerRoofHeight = observation.playerRoofHeight,
            .eventRunning = observation.eventRunning,
            .eventMode = observation.eventMode,
            .eventStatus = observation.eventStatus,
            .eventMapToolId = observation.eventMapToolId,
            .nextStageEnabled = observation.nextStageEnabled,
            .cameraPresent = controller.cameraPresent,
            .cameraYawRadians = controller.cameraYawRadians,
            .collisionCorrectionPresent = collision.present,
            .collisionCorrectionX = collision.x,
            .collisionCorrectionZ = collision.z,
            .remainingTicks = static_cast<std::uint32_t>(
                mDefinition.maximumTicks - mCandidateTick),
            .previousInput = previousInput,
            .playerDamageWaitTimer = observation.playerDamageWaitTimer,
            .playerIceDamageWaitTimer = observation.playerIceDamageWaitTimer,
            .playerSwordChangeWaitTimer = observation.playerSwordChangeWaitTimer,
            .playerDoStatus = observation.playerDoStatus,
            .stageName = controller.stageName,
            .room = observation.room,
            .layer = observation.layer,
            .point = observation.point,
            .playerProcedure = observation.playerProcId,
            .playerModeFlags = observation.playerModeFlags,
        };
        if (!encode_native_policy_features(policyInput, mPolicyFeatureRow, error))
            return false;
        mPolicyFeatureRowReady = true;
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
    {
        AccumulateMicros encoding(mProfile.corpusEncodingMicros);
        if (!append_learning_observation(mCurrentEpisode, observation, context, error))
            return false;
    }
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
        if (mDefinition.checkpointValidation == SuffixCheckpointValidation::RecordedReplayWindow) {
            mValidationFreshDigests.clear();
            mValidationFreshDigests.reserve(mDefinition.validationTicks);
            mValidationRestoredDigestMaterial.clear();
            mValidationRestoredDigestMaterial.reserve(mDefinition.validationTicks * 32);
            mValidationTick = 0;
            mPhase = Phase::ValidateFresh;
        } else {
            mValidationVerified = true;
            if (!beginEpisodeShard(error)) {
                fail(error);
                return false;
            }
            mPhase = Phase::Candidate;
        }
    } else if (mPhase == Phase::RestoreValidation) {
        if (!restoreSource(simulationTick, tapeFrame, preparedInputFrame, tapeFrameApplied, error))
        {
            fail(error);
            return false;
        }
        mValidationTick = 0;
        mPhase = Phase::ValidateRestored;
    } else if (mPhase == Phase::RestoreNext) {
        if (!restoreSource(simulationTick, tapeFrame, preparedInputFrame, tapeFrameApplied, error)) {
            fail(error);
            return false;
        }
        if (!mEpisodeShard.active() && !beginEpisodeShard(error)) {
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
    const auto& candidate = mDefinition.candidates[mCandidateIndex];
    if (candidate.tapePassthrough) {
        AccumulateNanos policy(mProfile.policyApplicationNanos);
        ++mProfile.policyApplicationSamples;
        return;
    }
    RawPadState chosen{};
    if (candidate.frozenPolicy) {
        if (!mPolicyFeatureRowReady || !mDefinition.frozenPolicy.has_value() ||
            !mFrozenPolicyModel.loaded())
        {
            fail("frozen policy lacks its phase-correct pre-input feature row or model");
            return;
        }
        std::string inferenceError;
        {
            AccumulateNanos inference(mProfile.policyInferenceNanos);
            ++mProfile.policyInferenceSamples;
            if (!mFrozenPolicyModel.infer(mPolicyFeatureRow, mPolicyOutput, inferenceError)) {
                fail("frozen model inference failed at the native input boundary: " +
                     inferenceError);
                return;
            }
        }
        FactorizedPadPolicyDecision decision;
        {
            AccumulateNanos decode(mProfile.policyHeadDecodeNanos);
            ++mProfile.policyHeadDecodeSamples;
            if (!decode_factorized_pad_policy(mDefinition.frozenPolicy->policyHead,
                    mPolicyOutput, decision, inferenceError))
            {
                fail("frozen model PAD output failed at the native input boundary: " +
                     inferenceError);
                return;
            }
        }
        chosen = decision.pad;
    } else {
        chosen = candidate.pads[mCandidateTick];
    }
    if (candidate.factorizedPolicy) {
        const std::uint32_t outputIndex = candidate.policyOutputIndexByTick[mCandidateTick];
        FactorizedPadPolicyDecision decision;
        std::string decodeError;
        {
            AccumulateNanos inference(mProfile.policyHeadDecodeNanos);
            ++mProfile.policyHeadDecodeSamples;
            if (outputIndex >= candidate.policyOutputs.size() ||
                !decode_factorized_pad_policy(candidate.policyHead,
                    candidate.policyOutputs[outputIndex], decision, decodeError))
            {
                fail("factorized PAD policy output failed at the native input boundary: " +
                     decodeError);
                return;
            }
        }
        chosen = decision.pad;
        if (chosen != candidate.pads[mCandidateTick]) {
            fail("factorized PAD policy output disagrees with its validated expected PAD");
            return;
        }
    }
    AccumulateNanos policy(mProfile.policyApplicationNanos);
    ++mProfile.policyApplicationSamples;
    mCandidateChosenPad = chosen;
    mCandidateChosenPadReady = true;
    const PADStatus status = raw_pad_state_to_pad_status(chosen);
    PADSetAutomationStatus(0, &status);
}

void SuffixBatchRunner::beginSimulationProfile() {
    if (!mProfile.active || mProfile.complete || mProfile.simulationActive ||
        mPhase != Phase::Candidate)
        return;
    mProfile.simulationStart = ProfileClock::now();
    mProfile.simulationActive = true;
}

void SuffixBatchRunner::finishSimulationProfile() {
    if (!mProfile.simulationActive) return;
    mProfile.simulationMicros += elapsed_micros(mProfile.simulationStart);
    ++mProfile.simulationSamples;
    mProfile.simulationActive = false;
}

void SuffixBatchRunner::beginCpuDrawTraversalProfile() {
    if (!mProfile.active || mProfile.complete || mProfile.cpuDrawActive ||
        mPhase != Phase::Candidate)
        return;
    mProfile.cpuDrawStart = ProfileClock::now();
    mProfile.cpuDrawActive = true;
}

void SuffixBatchRunner::endCpuDrawTraversalProfile() {
    if (!mProfile.cpuDrawActive) return;
    mProfile.cpuDrawTraversalMicros += elapsed_micros(mProfile.cpuDrawStart);
    ++mProfile.cpuDrawTraversalSamples;
    mProfile.cpuDrawActive = false;
}

void SuffixBatchRunner::beginCpuRendererSubmissionProfile() {
    if (!mProfile.active || mProfile.complete || mProfile.cpuRendererActive ||
        mPhase != Phase::Candidate)
        return;
    mProfile.cpuRendererStart = ProfileClock::now();
    mProfile.cpuRendererActive = true;
}

void SuffixBatchRunner::endCpuRendererSubmissionProfile() {
    if (!mProfile.cpuRendererActive) return;
    mProfile.cpuRendererSubmissionMicros += elapsed_micros(mProfile.cpuRendererStart);
    ++mProfile.cpuRendererSubmissionSamples;
    mProfile.cpuRendererActive = false;
}

void SuffixBatchRunner::resetBatchProfile(const bool sourceCheckpointReused) {
    mProfile = {};
    const AuroraStats* renderer = aurora_get_stats();
    mProfile.submittedCommandBuffersAtStart = renderer->submittedCommandBufferCount;
    mProfile.discardedGpuFramesAtStart = renderer->discardedGpuFrameCount;
    mProfile.batchStart = ProfileClock::now();
    mProfile.active = true;
    mProfile.sourceCheckpointReused = sourceCheckpointReused;
}

void SuffixBatchRunner::finishBatchProfile() {
    finishSimulationProfile();
    endCpuDrawTraversalProfile();
    endCpuRendererSubmissionProfile();
    if (!mProfile.active || mProfile.complete) return;
    mProfile.batchWallMicros = elapsed_micros(mProfile.batchStart);
    mProfile.complete = true;
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
    {
        AccumulateMicros encoding(mProfile.corpusEncodingMicros);
        append_learning_action(mCurrentEpisode, chosenPad, mConsumedPads.back());
    }
    ControllerObservation controller;
    GameplayCollisionCorrectionObservation collision;
    GameplayTraceSample gameplayTrace;
    {
        AccumulateMicros capture(mProfile.observationCaptureMicros);
        controller = capture_controller_observation(mEpisodeControllerStorage);
        collision = capture_gameplay_collision_correction();
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
    }
    GameplayCollisionPlanesObservation collisionPlanes;
    GameplayPlayerFormObservation playerForm;
    {
        AccumulateMicros capture(mProfile.observationCaptureMicros);
        collisionPlanes = capture_gameplay_collision_planes();
        playerForm = capture_gameplay_player_form();
    }
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
    {
        AccumulateMicros encoding(mProfile.corpusEncodingMicros);
        if (!append_learning_observation(mCurrentEpisode, observation, context, error))
            return false;
    }
    mEpisodePreInputCaptured = false;
    if (!terminal)
        return true;
    AccumulateMicros encoding(mProfile.corpusEncodingMicros);
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
    if (mDefinition.verifyStateHashes) {
        result.stateSequenceDigest = xxh3_128_hex(mStateDigestMaterial);
        result.stateTickDigests = mStateTickDigests;
    }
    result.terminalBoundaryFingerprint =
        compute_milestone_boundary_fingerprint(observation, input_tape_player().tape().boot);
    if (result.terminalBoundaryFingerprint.empty()) {
        error = "suffix candidate terminal boundary fingerprint is unavailable";
        return false;
    }
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
    {
        AccumulateMicros encoding(mProfile.corpusEncodingMicros);
        if (!mEpisodeShard.append(episode, mCurrentEpisode, error))
            return false;
    }

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
    const std::uint64_t tapeFrame, const std::uint64_t preparedInputFrame,
    const bool tapeFrameApplied, std::string& error) {
    if (!mEnabled || !ownsPostSimulation() || mCompleted || mFailed)
        return false;
    if (mPhase == Phase::ValidateFresh || mPhase == Phase::ValidateRestored) {
        std::string digest;
        if (!captureValidationTickDigest(
                simulationTick, tapeFrame, preparedInputFrame, tapeFrameApplied, digest, error))
        {
            fail(error);
            return true;
        }
        if (mPhase == Phase::ValidateFresh) {
            mValidationFreshDigests.push_back(digest);
        } else if (mValidationTick >= mValidationFreshDigests.size() ||
                   digest != mValidationFreshDigests[mValidationTick])
        {
            mValidationFirstDivergence = mValidationTick;
            mValidationExpectedDigest = mValidationTick < mValidationFreshDigests.size() ?
                                            mValidationFreshDigests[mValidationTick] :
                                            "<missing>";
            mValidationActualDigest = std::move(digest);
            error = "suffix checkpoint replay validation diverged at tick " +
                    std::to_string(mValidationTick) + "; expected " + mValidationExpectedDigest +
                    ", observed " + mValidationActualDigest;
            fail(error);
            return true;
        } else {
            mValidationRestoredDigestMaterial += digest;
        }
        ++mValidationTick;
        if (mValidationTick != mDefinition.validationTicks)
            return false;
        if (mPhase == Phase::ValidateFresh) {
            std::string sequence;
            sequence.reserve(mValidationFreshDigests.size() * 32);
            for (const std::string& tickDigest : mValidationFreshDigests)
                sequence += tickDigest;
            mValidationFreshSequenceDigest = xxh3_128_hex(sequence);
            mPhase = Phase::RestoreValidation;
        } else {
            mValidationRestoredSequenceDigest = xxh3_128_hex(mValidationRestoredDigestMaterial);
            mValidationVerified = true;
            mPhase = Phase::RestoreNext;
        }
        return false;
    }
    finishSimulationProfile();
    const auto& candidate = mDefinition.candidates[mCandidateIndex];
    RawPadState expectedPad{};
    if (candidate.tapePassthrough) {
        expectedPad =
            input_tape_player().tape().frames[mDefinition.sourceFrame + mCandidateTick].pads[0];
    } else if (candidate.frozenPolicy) {
        if (!mCandidateChosenPadReady) {
            error = "frozen policy did not produce a PAD for the current tick";
            fail(error);
            return true;
        }
        expectedPad = mCandidateChosenPad;
    } else {
        expectedPad = candidate.pads[mCandidateTick];
    }
    if (mConsumedCaptureFailed || mConsumedPads.size() != mCandidateTick + 1 ||
        mConsumedPads.back() != expectedPad)
    {
        error = "candidate PAD state was not consumed exactly at the input boundary";
        fail(error);
        return true;
    }
    if (mDefinition.verifyStateHashes) {
        AccumulateMicros validation(mProfile.stateValidationMicros);
        ++mProfile.stateValidationSamples;
        std::string digest;
        const StateCheckpointError checkpointError = mCheckpoint.currentSemanticDigest(digest);
        if (checkpointError != StateCheckpointError::None) {
            error = state_checkpoint_error_message(checkpointError);
            fail(error);
            return true;
        }
        mStateDigestMaterial += digest;
        mStateTickDigests.push_back(std::move(digest));
    }

    MilestoneObservation observation;
    {
        AccumulateMicros capture(mProfile.observationCaptureMicros);
        ++mProfile.observationCaptureSamples;
        observation = capture_milestone_observation(mMilestoneStorage);
        mGoalTracker.observe(observation, simulationTick, tapeFrame);
    }
    const bool success = mGoalTracker.goalReached();
    const bool exhausted = mCandidateTick + 1 == mDefinition.maximumTicks;
    if (!appendEpisodePostSimulation(
            observation, expectedPad, simulationTick, success || exhausted, error)) {
        fail(error);
        return true;
    }
    mPolicyFeatureRowReady = false;
    mCandidateChosenPadReady = false;
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
        {
            AccumulateMicros encoding(mProfile.corpusEncodingMicros);
            if (!mEpisodeShard.finish(error)) {
                fail(error);
                return true;
            }
        }
        // Preserve the final candidate's ordinary milestone result for the
        // standard artifact writer. Every candidate retains its own complete
        // predicate evidence in the aggregate suffix result and episode shard.
        milestone_tracker() = mGoalTracker;
        mPhase = Phase::Complete;
        mCompleted = true;
        finishBatchProfile();
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

bool SuffixBatchRunner::writeArtifacts(std::string& error) {
    if (!mEnabled) return true;
    if (mArtifactsWritten) return true;
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
            {"state_tick_digests", result.stateTickDigests.empty()
                    ? nlohmann::json(nullptr) : nlohmann::json(result.stateTickDigests)},
            {"terminal_boundary_fingerprint", result.terminalBoundaryFingerprint},
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
    const std::uint64_t candidateTicks = std::accumulate(mResults.begin(), mResults.end(),
        std::uint64_t{0}, [](const std::uint64_t total, const CandidateResult& candidate) {
            return total + candidate.ticksExecuted;
        });
    std::uint64_t expectedPolicyHeadDecodeSamples = 0;
    std::uint64_t expectedPolicyInferenceSamples = 0;
    for (std::size_t index = 0; index < mResults.size(); ++index) {
        if (mDefinition.candidates[index].factorizedPolicy ||
            mDefinition.candidates[index].frozenPolicy)
            expectedPolicyHeadDecodeSamples += mResults[index].ticksExecuted;
        if (mDefinition.candidates[index].frozenPolicy)
            expectedPolicyInferenceSamples += mResults[index].ticksExecuted;
    }
    const std::size_t expectedRestores =
        mDefinition.candidates.size() - 1 + (mProfile.sourceCheckpointReused ? 1 : 0) +
        (!mProfile.sourceCheckpointReused && mDefinition.checkpointValidation ==
                                                 SuffixCheckpointValidation::RecordedReplayWindow ?
                2 :
                0);
    const bool profileVerified =
        mProfile.complete && mProfile.policyApplicationSamples == candidateTicks &&
        mProfile.policyHeadDecodeSamples == expectedPolicyHeadDecodeSamples &&
        mProfile.policyInferenceSamples == expectedPolicyInferenceSamples &&
        mProfile.simulationSamples == candidateTicks &&
        mProfile.observationCaptureSamples == candidateTicks * 2 &&
        mProfile.cpuDrawTraversalSamples == candidateTicks &&
        mProfile.cpuRendererSubmissionSamples == candidateTicks &&
        mRestoreMicros.size() == expectedRestores &&
        mEpisodeShard.episodeCount() == mResults.size() && mValidationVerified &&
        (mDefinition.checkpointValidation != SuffixCheckpointValidation::RecordedReplayWindow ||
            mValidationSamples == mDefinition.validationTicks * 2) &&
        (!mDefinition.verifyStateHashes || mProfile.stateValidationSamples == candidateTicks);
    if (mCompleted && !profileVerified) {
        error = "completed suffix batch has incomplete phase-profile sample coverage";
        return false;
    }
    const auto measured = [](const std::uint64_t micros, const std::uint64_t samples) {
        return nlohmann::json{{"status", "measured"}, {"micros", micros},
            {"samples", samples}};
    };
    const AuroraStats* renderer = aurora_get_stats();
    const std::uint64_t submittedCommandBuffers =
        renderer->submittedCommandBufferCount - mProfile.submittedCommandBuffersAtStart;
    const std::uint64_t discardedGpuFrames =
        renderer->discardedGpuFrameCount - mProfile.discardedGpuFramesAtStart;
    const bool gpuFramesDiscarded = submittedCommandBuffers == 0 &&
                                    discardedGpuFrames >= candidateTicks;
    const bool gpuCodeCreationAbsent = renderer->createdShaderModuleCount == 0 &&
                                       renderer->createdRenderPipelineCount == 0 &&
                                       renderer->createdComputePipelineCount == 0;
    const bool gpuQueueOperationsAbsent = renderer->submittedCommandBufferCount == 0 &&
                                          renderer->directQueueWriteCount == 0;
    const nlohmann::json timing{
        {"schema", mDefinition.frozenPolicy.has_value() ?
                       "dusklight-suffix-batch-timing/v2" :
                       "dusklight-suffix-batch-timing/v1"},
        {"batch_wall_micros", mProfile.complete ? nlohmann::json(mProfile.batchWallMicros) :
                                                  nlohmann::json(nullptr)},
        {"candidate_ticks", candidateTicks},
        {"verified", profileVerified},
        {"accounting", {
            {"batch_boundary", "source-capture-or-rearm through sealed episode shard"},
            {"simulation_boundary", "after PADRead through deterministic clock commit"},
            {"nested_simulation_phases", {"cpu_draw_traversal",
                "cpu_renderer_submission"}},
        }},
        {"phases", {
            {"checkpoint_capture", {
                {"status", mProfile.sourceCheckpointReused ? "reused" : "measured"},
                {"micros", mProfile.sourceCheckpointReused ? nlohmann::json(nullptr) :
                                                             nlohmann::json(mCaptureMicros)},
                {"session_capture_micros", mCaptureMicros},
            }},
            {"checkpoint_restore", {
                {"status", "measured"},
                {"micros", std::accumulate(mRestoreMicros.begin(), mRestoreMicros.end(),
                    std::uint64_t{0})},
                {"samples", mRestoreMicros.size()},
            }},
            {"checkpoint_validation", {
                {"status", mDefinition.checkpointValidation ==
                        SuffixCheckpointValidation::GameplayReadyFSp103
                    ? "fixed_predicate" :
                      mProfile.sourceCheckpointReused ? "reused" : "measured"},
                {"micros", mProfile.sourceCheckpointReused ||
                        mDefinition.checkpointValidation ==
                            SuffixCheckpointValidation::GameplayReadyFSp103
                    ? nlohmann::json(nullptr) : nlohmann::json(mValidationMicros)},
                {"session_micros", mValidationMicros},
                {"samples", mValidationSamples},
            }},
            {"policy_inference", expectedPolicyInferenceSamples == 0
                ? nlohmann::json{{"status", "not_present"}, {"micros", nullptr}}
                : nlohmann::json{{"status", "measured"},
                      {"schema", "dusklight-frozen-inference/v1"},
                      {"feature_schema_sha256", kNativePolicyFeatureSchemaSha256},
                      {"micros", mProfile.policyInferenceNanos / 1'000},
                      {"nanoseconds", mProfile.policyInferenceNanos},
                      {"samples", mProfile.policyInferenceSamples},
                      {"placement", "native_pre_input"}}},
            {"policy_head_decode", expectedPolicyHeadDecodeSamples == 0
                ? nlohmann::json{{"status", "not_present"}, {"micros", nullptr}}
                : nlohmann::json{{"status", "measured"},
                      {"schema", kFactorizedPadPolicyHeadSchema},
                      {"micros", mProfile.policyHeadDecodeNanos / 1'000},
                      {"nanoseconds", mProfile.policyHeadDecodeNanos},
                      {"samples", mProfile.policyHeadDecodeSamples},
                      {"input", expectedPolicyInferenceSamples == 0 ?
                          "precomputed continuous policy-output row" :
                          "frozen model output at phase-correct pre-input boundary"}}},
            {"policy_application", {
                {"status", "measured"},
                {"micros", mProfile.policyApplicationNanos / 1'000},
                {"nanoseconds", mProfile.policyApplicationNanos},
                {"samples", mProfile.policyApplicationSamples},
            }},
            {"simulation", measured(mProfile.simulationMicros, mProfile.simulationSamples)},
            {"observation_capture", measured(mProfile.observationCaptureMicros,
                                               mProfile.observationCaptureSamples)},
            {"state_validation", mDefinition.verifyStateHashes
                ? measured(mProfile.stateValidationMicros, mProfile.stateValidationSamples)
                : nlohmann::json{{"status", "disabled"}, {"micros", nullptr}}},
            {"corpus_encoding", {
                {"status", "measured"},
                {"micros", mProfile.corpusEncodingMicros},
                {"episodes", mEpisodeShard.episodeCount()},
                {"uncompressed_bytes", mEpisodeShard.uncompressedBytes()},
                {"compressed_bytes", mEpisodeShard.compressedBytes()},
            }},
            {"cpu_draw_traversal", measured(mProfile.cpuDrawTraversalMicros,
                                              mProfile.cpuDrawTraversalSamples)},
            {"cpu_renderer_submission", measured(mProfile.cpuRendererSubmissionMicros,
                                                   mProfile.cpuRendererSubmissionSamples)},
            {"gpu_work", {
                {"status", gpuFramesDiscarded ? "discarded" : "timestamp_unavailable"},
                {"micros", nullptr},
                {"submitted_command_buffers", submittedCommandBuffers},
                {"submitted_command_buffers_process_total", renderer->submittedCommandBufferCount},
                {"discarded_frames", discardedGpuFrames},
                {"direct_queue_writes_process_total", renderer->directQueueWriteCount},
                {"gpu_queue_operations_absent", gpuQueueOperationsAbsent},
                {"created_shader_modules_process_total", renderer->createdShaderModuleCount},
                {"created_render_pipelines_process_total", renderer->createdRenderPipelineCount},
                {"created_compute_pipelines_process_total", renderer->createdComputePipelineCount},
                {"gpu_code_creation_absent", gpuCodeCreationAbsent},
                {"reason", gpuFramesDiscarded
                    ? (gpuCodeCreationAbsent && gpuQueueOperationsAbsent
                          ? "simulation-only render sink created no shader or pipeline objects, issued no queue operations, and discarded every candidate frame before GPU encoding"
                          : "simulation-only render sink discarded every candidate frame before GPU encoding but GPU setup or queue operations were observed")
                    : "Aurora exposes submission counts but no authenticated GPU timestamps"},
            }},
        }},
    };
    nlohmann::json result{
        {"schema", "dusklight-suffix-batch-result/v6"},
        {"status", mCompleted ? "passed" :
                   mFailed    ? "failed" :
                                "incomplete"},
        {"source_frame", mDefinition.sourceFrame},
        {"policy_model", mDefinition.frozenPolicy.has_value()
            ? nlohmann::json{{"schema", FrozenPolicySchema},
                  {"model_xxh3_128", mDefinition.frozenPolicy->modelXxh3_128},
                  {"feature_schema_sha256", digest_hex(
                      mFrozenPolicyModel.featureSchemaSha256())},
                  {"action_schema_sha256", digest_hex(
                      mFrozenPolicyModel.actionSchemaSha256())},
                  {"objective_sha256", digest_hex(mFrozenPolicyModel.objectiveSha256())},
                  {"parameter_count", mFrozenPolicyModel.parameterCount()}}
            : nlohmann::json(nullptr)},
        {"source_boundary",
            {
                {"milestone", mDefinition.checkpointValidation ==
                                      SuffixCheckpointValidation::GameplayReadyFSp103 ?
                                  nlohmann::json(milestone_name(MilestoneId::GameplayReadyFSp103)) :
                                  nlohmann::json(nullptr)},
                {"expected_fingerprint", mDefinition.sourceBoundaryFingerprint},
                {"actual_fingerprint", mActualSourceBoundaryFingerprint.empty() ?
                                           nlohmann::json(nullptr) :
                                           nlohmann::json(mActualSourceBoundaryFingerprint)},
                {"fingerprint_verified",
                    !mActualSourceBoundaryFingerprint.empty() &&
                        mActualSourceBoundaryFingerprint == mDefinition.sourceBoundaryFingerprint},
                {"verified",
                    !mActualSourceBoundaryFingerprint.empty() &&
                        mActualSourceBoundaryFingerprint == mDefinition.sourceBoundaryFingerprint &&
                        mValidationVerified},
            }},
        {"checkpoint_validation",
            {
                {"kind", mDefinition.checkpointValidation ==
                                 SuffixCheckpointValidation::RecordedReplayWindow ?
                             "recorded_replay_window" :
                             "gameplay_ready_f_sp103"},
                {"ticks", mDefinition.validationTicks},
                {"verified", mValidationVerified},
                {"source_semantic_digest", mSourceSemanticDigest.empty() ?
                                               nlohmann::json(nullptr) :
                                               nlohmann::json(mSourceSemanticDigest)},
                {"fresh_sequence_digest", mValidationFreshSequenceDigest.empty() ?
                                              nlohmann::json(nullptr) :
                                              nlohmann::json(mValidationFreshSequenceDigest)},
                {"restored_sequence_digest", mValidationRestoredSequenceDigest.empty() ?
                                                 nlohmann::json(nullptr) :
                                                 nlohmann::json(mValidationRestoredSequenceDigest)},
                {"first_divergence_tick", mValidationFirstDivergence.has_value() ?
                                              nlohmann::json(*mValidationFirstDivergence) :
                                              nlohmann::json(nullptr)},
                {"expected_tick_digest", mValidationExpectedDigest.empty() ?
                                             nlohmann::json(nullptr) :
                                             nlohmann::json(mValidationExpectedDigest)},
                {"actual_tick_digest", mValidationActualDigest.empty() ?
                                           nlohmann::json(nullptr) :
                                           nlohmann::json(mValidationActualDigest)},
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
        {"timing", timing},
        {"audio_callback_quiesced", mAudioCallbackQuiesced},
        {"episode_shard", {
            {"schema", mDefinition.frozenPolicy.has_value() ?
                LearningEpisodePolicyShardSchema : LearningEpisodeShardSchema},
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

    if (mWinnerTapePath.empty() || !mWinnerResultIndex.has_value()) {
        mArtifactsWritten = true;
        return true;
    }
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
    if (!write_atomic(mWinnerTapePath,
            std::string_view(reinterpret_cast<const char*>(bytes.data()), bytes.size()), error))
        return false;
    mArtifactsWritten = true;
    return true;
}

SuffixBatchRunner& suffix_batch_runner() {
    static SuffixBatchRunner runner;
    return runner;
}

}  // namespace dusk::automation
