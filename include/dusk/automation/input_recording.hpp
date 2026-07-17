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

enum class RecordingStartError {
    None,
    UnknownMilestone,
    MilestoneNotRequested,
    MilestoneNotHit,
    WrongTapeFrame,
    FingerprintMismatch,
    StaleProgram,
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

struct RecordingStartBinding {
    std::string milestone;
    std::string boundaryFingerprint;
    std::string programDigest;
    std::string definitionDigest;
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
 * Binds a parent-tape handoff to either a built-in or authored milestone first-hit. The caller
 * supplies the exact final parent frame, so a predicate hit earlier in the tape cannot authorize
 * live recording even if its game-state fingerprint happens to match again later.
 */
RecordingStartError bind_recording_start(const MilestoneTracker& tracker,
    const MilestoneProgram& program, std::string_view milestone,
    std::string_view expectedFingerprint, std::uint64_t expectedTapeFrame,
    RecordingStartBinding& binding);
const char* recording_start_error_message(RecordingStartError error);

/**
 * Ordinary playback must retain at least one visible tape frame. An exact tape-end reveal is only
 * meaningful for recording handoff, where the next PAD read is the first live child frame.
 */
FastForwardBoundaryError validate_fast_forward_boundary(std::uint64_t requestedFrames,
    std::uint64_t tapeFrames, bool recording, bool tapeEndReleasesInput);

/**
 * Guard used immediately before revealing an exact tape-end recording handoff. The boundary must
 * be fully verified before the parent frame becomes visible, but recorder activation may be
 * deliberately deferred by a host-only countdown.
 */
bool accelerated_recording_reveal_ready(bool exactTapeEndHandoff, bool boundaryVerified);

constexpr std::uint8_t RecordingHandoffCountdownMaximumSeconds = 10;

/** Converts a positive remaining duration to the human-facing 3, 2, 1 countdown label. */
std::uint8_t recording_handoff_countdown_display_seconds(std::uint64_t remainingMilliseconds);

/** Maps an exact N-frame absolute prefix to its post-simulation handoff boundary. */
ParentRecordingBoundary exact_parent_recording_boundary(std::uint64_t completedFrames);

const char* fast_forward_boundary_error_message(FastForwardBoundaryError error);

}  // namespace dusk::automation
