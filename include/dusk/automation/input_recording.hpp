#pragma once

#include <cstdint>
#include <string>
#include <string_view>

namespace dusk::automation {

class MilestoneProgram;
class MilestoneTracker;

enum class BootRecordingError {
    None,
    MissingOutputTape,
    HasAutomationInput,
    Headless,
    Unpaced,
    MissingMilestoneProgram,
    MissingMilestoneSelection,
    MissingMilestoneResult,
    HasMilestoneGoal,
    MissingStartMilestone,
    UnknownStartMilestone,
    StartMilestoneNotPreInput,
    StartMilestoneNotSingleTick,
    StartMilestoneNotRequested,
    StartMilestoneNotHit,
    InvalidBootBoundary,
    StaleProgram,
    FingerprintMismatch,
    RecorderBeginFailed,
};

enum class FastForwardBoundaryError {
    None,
    PastTapeEnd,
    TapeEndRequiresRecording,
    TapeEndRequiresRelease,
};

struct BootRecordingCliRequest {
    bool enabled = false;
    bool hasOutputTape = false;
    bool hasAutomationInput = false;
    bool headless = false;
    bool unpaced = false;
    bool hasMilestoneProgram = false;
    bool hasMilestoneSelection = false;
    bool hasMilestoneResult = false;
    bool hasMilestoneGoal = false;
    bool hasStartMilestone = false;
};

struct BootRecordingBinding {
    std::string milestone;
    std::string boundaryFingerprint;
    std::string programDigest;
    std::string definitionDigest;
    std::uint64_t boundaryIndex = 0;
};

struct ParentRecordingBoundary {
    std::uint64_t boundaryIndex = 0;
    std::uint64_t tapeFrame = 0;
};

using BootRecordingBegin = bool (*)(void* context);
using BootRecordingReleaseInput = void (*)(void* context);

BootRecordingError validate_boot_recording_cli(const BootRecordingCliRequest& request);
BootRecordingError validate_authored_boot_definition(
    const MilestoneProgram& program, std::string_view milestone);

/**
 * Validates the exact authored boundary, begins the already-armed recorder, and only then releases
 * live input. The release callback must perform any PAD automation cleanup before dropping host
 * input quarantine. No PAD read occurs inside this function.
 */
BootRecordingError begin_authored_boot_recording(const MilestoneTracker& tracker,
    const MilestoneProgram& program, std::string_view milestone,
    std::string_view expectedFingerprint, BootRecordingBegin begin,
    BootRecordingReleaseInput releaseInput, void* context, BootRecordingBinding& binding);

const char* boot_recording_error_message(BootRecordingError error);

/**
 * Ordinary playback must retain at least one visible tape frame. An exact tape-end reveal is only
 * meaningful for recording handoff, where the next PAD read is the first live child frame.
 */
FastForwardBoundaryError validate_fast_forward_boundary(std::uint64_t requestedFrames,
    std::uint64_t tapeFrames, bool recording, bool tapeEndReleasesInput);

/** Guard used immediately before revealing an exact tape-end recording handoff. */
bool accelerated_recording_reveal_ready(
    bool exactTapeEndHandoff, bool handoffReached, bool recorderIsRecording);

/** Maps an exact N-frame absolute prefix to its post-simulation handoff boundary. */
ParentRecordingBoundary exact_parent_recording_boundary(std::uint64_t completedFrames);

const char* fast_forward_boundary_error_message(FastForwardBoundaryError error);

}  // namespace dusk::automation
