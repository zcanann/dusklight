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

}  // namespace dusk::automation
