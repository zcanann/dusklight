#include "dusk/automation/input_recording.hpp"

#include "dusk/automation/milestone_program.hpp"
#include "dusk/automation/milestones.hpp"

#include <algorithm>
#include <cassert>

namespace dusk::automation {

BootRecordingError validate_boot_recording_cli(const BootRecordingCliRequest& request) {
    if (!request.enabled)
        return BootRecordingError::None;
    if (!request.hasOutputTape)
        return BootRecordingError::MissingOutputTape;
    if (request.hasAutomationInput)
        return BootRecordingError::HasAutomationInput;
    if (request.headless)
        return BootRecordingError::Headless;
    if (request.unpaced)
        return BootRecordingError::Unpaced;
    if (!request.hasMilestoneProgram)
        return BootRecordingError::MissingMilestoneProgram;
    if (!request.hasMilestoneSelection)
        return BootRecordingError::MissingMilestoneSelection;
    if (!request.hasMilestoneResult)
        return BootRecordingError::MissingMilestoneResult;
    if (request.hasMilestoneGoal)
        return BootRecordingError::HasMilestoneGoal;
    if (!request.hasStartMilestone)
        return BootRecordingError::MissingStartMilestone;
    return BootRecordingError::None;
}

BootRecordingError validate_authored_boot_definition(
    const MilestoneProgram& program, const std::string_view milestone) {
    const MilestoneProgramDefinition* definition = program.find(milestone);
    if (definition == nullptr)
        return BootRecordingError::UnknownStartMilestone;
    if (definition->phase != MilestoneProgramPhase::PreInput)
        return BootRecordingError::StartMilestoneNotPreInput;
    // Boundary zero is observed exactly once. A stability count above one can never prove it.
    if (definition->stableTicks != 1)
        return BootRecordingError::StartMilestoneNotSingleTick;
    return BootRecordingError::None;
}

BootRecordingError begin_authored_boot_recording(const MilestoneTracker& tracker,
    const MilestoneProgram& program, const std::string_view milestone,
    const std::string_view expectedFingerprint, const BootRecordingBegin begin,
    const BootRecordingReleaseInput releaseInput, void* const context,
    BootRecordingBinding& binding) {
    binding = {};
    if (const BootRecordingError error = validate_authored_boot_definition(program, milestone);
        error != BootRecordingError::None)
        return error;
    const auto hit =
        std::ranges::find(tracker.authoredHits(), milestone, &AuthoredMilestoneHit::id);
    if (hit == tracker.authoredHits().end())
        return BootRecordingError::StartMilestoneNotRequested;
    if (!hit->hit)
        return BootRecordingError::StartMilestoneNotHit;
    if (hit->phase != MilestoneProgramPhase::PreInput || hit->boundaryIndex != 0 ||
        hit->simulationTick != 0 || hit->tapeFrame != MilestoneNoTapeFrame)
        return BootRecordingError::InvalidBootBoundary;
    const MilestoneProgramDefinition* definition = program.find(milestone);
    if (hit->programDigest != program.digest() ||
        hit->definitionDigest != definition->definitionDigest)
        return BootRecordingError::StaleProgram;
    if (hit->evidence.boundaryFingerprint.empty() ||
        (!expectedFingerprint.empty() && expectedFingerprint != hit->evidence.boundaryFingerprint))
        return BootRecordingError::FingerprintMismatch;
    if (begin == nullptr || !begin(context))
        return BootRecordingError::RecorderBeginFailed;

    binding = {
        .milestone = hit->id,
        .boundaryFingerprint = hit->evidence.boundaryFingerprint,
        .programDigest = hit->programDigest,
        .definitionDigest = hit->definitionDigest,
        .boundaryIndex = hit->boundaryIndex,
    };
    if (releaseInput != nullptr)
        releaseInput(context);
    return BootRecordingError::None;
}

const char* boot_recording_error_message(const BootRecordingError error) {
    switch (error) {
    case BootRecordingError::None:
        return "no error";
    case BootRecordingError::MissingOutputTape:
        return "--record-input-from-boot requires --record-input-tape PATH";
    case BootRecordingError::HasAutomationInput:
        return "Boot recording cannot be combined with --input-tape or --input-controller";
    case BootRecordingError::Headless:
        return "Boot recording requires a headful window";
    case BootRecordingError::Unpaced:
        return "Boot recording cannot use --unpaced; --fixed-step is supported";
    case BootRecordingError::MissingMilestoneProgram:
        return "Boot recording requires --milestone-program PATH";
    case BootRecordingError::MissingMilestoneSelection:
        return "Boot recording requires --milestones LIST";
    case BootRecordingError::MissingMilestoneResult:
        return "Boot recording requires --milestone-result PATH";
    case BootRecordingError::HasMilestoneGoal:
        return "Boot recording cannot use --milestone-goal; the Boot hit starts recording";
    case BootRecordingError::MissingStartMilestone:
        return "Boot recording requires --record-input-start-milestone ID";
    case BootRecordingError::UnknownStartMilestone:
        return "Boot recording start milestone is not authored by the loaded program";
    case BootRecordingError::StartMilestoneNotPreInput:
        return "Boot recording start milestone must use phase pre_input";
    case BootRecordingError::StartMilestoneNotSingleTick:
        return "Boot recording start milestone must use stable 1";
    case BootRecordingError::StartMilestoneNotRequested:
        return "Boot recording start milestone was not selected";
    case BootRecordingError::StartMilestoneNotHit:
        return "Boot recording start milestone did not hit before the first PAD read";
    case BootRecordingError::InvalidBootBoundary:
        return "Boot recording start hit is not exact pre-input boundary zero";
    case BootRecordingError::StaleProgram:
        return "Boot recording start hit was produced by stale predicate bytecode";
    case BootRecordingError::FingerprintMismatch:
        return "Boot recording boundary fingerprint mismatch";
    case BootRecordingError::RecorderBeginFailed:
        return "armed Boot input recorder failed to begin";
    }
    return "unknown Boot recording error";
}

RecordingStartError bind_recording_start(const MilestoneTracker& tracker,
    const MilestoneProgram& program, const std::string_view milestone,
    const std::string_view expectedFingerprint, const std::uint64_t expectedTapeFrame,
    RecordingStartBinding& binding) {
    binding = {};
    if (const MilestoneDefinition* definition = find_milestone(milestone); definition != nullptr) {
        const auto hit = std::ranges::find(tracker.hits(), definition->id, &MilestoneHit::id);
        if (hit == tracker.hits().end())
            return RecordingStartError::MilestoneNotRequested;
        if (!hit->hit)
            return RecordingStartError::MilestoneNotHit;
        if (hit->tapeFrame != expectedTapeFrame)
            return RecordingStartError::WrongTapeFrame;
        if (hit->evidence.boundaryFingerprint != expectedFingerprint)
            return RecordingStartError::FingerprintMismatch;
        binding = {
            .milestone = std::string(definition->name),
            .boundaryFingerprint = hit->evidence.boundaryFingerprint,
            .boundaryIndex = expectedTapeFrame + 1,
            .tapeFrame = hit->tapeFrame,
        };
        return RecordingStartError::None;
    }

    const MilestoneProgramDefinition* definition = program.find(milestone);
    if (definition == nullptr)
        return RecordingStartError::UnknownMilestone;
    const auto hit =
        std::ranges::find(tracker.authoredHits(), milestone, &AuthoredMilestoneHit::id);
    if (hit == tracker.authoredHits().end())
        return RecordingStartError::MilestoneNotRequested;
    if (!hit->hit)
        return RecordingStartError::MilestoneNotHit;
    if (hit->tapeFrame != expectedTapeFrame)
        return RecordingStartError::WrongTapeFrame;
    if (hit->evidence.boundaryFingerprint != expectedFingerprint)
        return RecordingStartError::FingerprintMismatch;
    if (hit->programDigest != program.digest() ||
        hit->definitionDigest != definition->definitionDigest)
        return RecordingStartError::StaleProgram;
    binding = {
        .milestone = hit->id,
        .boundaryFingerprint = hit->evidence.boundaryFingerprint,
        .programDigest = hit->programDigest,
        .definitionDigest = hit->definitionDigest,
        .boundaryIndex = hit->boundaryIndex,
        .tapeFrame = hit->tapeFrame,
    };
    return RecordingStartError::None;
}

const char* recording_start_error_message(const RecordingStartError error) {
    switch (error) {
    case RecordingStartError::None:
        return "no error";
    case RecordingStartError::UnknownMilestone:
        return "start milestone is not registered";
    case RecordingStartError::MilestoneNotRequested:
        return "start milestone was not selected";
    case RecordingStartError::MilestoneNotHit:
        return "start milestone was not hit";
    case RecordingStartError::WrongTapeFrame:
        return "start milestone first hit is not the final parent tape frame";
    case RecordingStartError::FingerprintMismatch:
        return "start milestone fingerprint mismatch";
    case RecordingStartError::StaleProgram:
        return "start milestone was produced by stale predicate bytecode";
    }
    return "unknown recording start error";
}

FastForwardBoundaryError validate_fast_forward_boundary(const std::uint64_t requestedFrames,
    const std::uint64_t tapeFrames, const bool recording,
    const bool tapeEndReleasesInput) {
    if (requestedFrames > tapeFrames) return FastForwardBoundaryError::PastTapeEnd;
    if (requestedFrames < tapeFrames) return FastForwardBoundaryError::None;
    if (!recording) return FastForwardBoundaryError::TapeEndRequiresRecording;
    if (!tapeEndReleasesInput) return FastForwardBoundaryError::TapeEndRequiresRelease;
    return FastForwardBoundaryError::None;
}

bool accelerated_recording_reveal_ready(
    const bool exactTapeEndHandoff, const bool boundaryVerified) {
    return !exactTapeEndHandoff || boundaryVerified;
}

std::uint8_t recording_handoff_countdown_display_seconds(
    const std::uint64_t remainingMilliseconds) {
    if (remainingMilliseconds == 0)
        return 0;
    const std::uint64_t roundedUp = (remainingMilliseconds + 999) / 1000;
    return static_cast<std::uint8_t>(
        std::min<std::uint64_t>(roundedUp, RecordingHandoffCountdownMaximumSeconds));
}

ParentRecordingBoundary exact_parent_recording_boundary(const std::uint64_t completedFrames) {
    assert(completedFrames > 0);
    return {
        .boundaryIndex = completedFrames,
        .tapeFrame = completedFrames - 1,
    };
}

const char* fast_forward_boundary_error_message(const FastForwardBoundaryError error) {
    switch (error) {
    case FastForwardBoundaryError::None: return "no error";
    case FastForwardBoundaryError::PastTapeEnd:
        return "fast-forward frame count exceeds tape frame count";
    case FastForwardBoundaryError::TapeEndRequiresRecording:
        return "fast-forward to tape end requires --record-input-tape PATH with no controller continuation";
    case FastForwardBoundaryError::TapeEndRequiresRelease:
        return "fast-forward to recording handoff requires --input-tape-end release";
    }
    return "unknown fast-forward boundary error";
}

}  // namespace dusk::automation
