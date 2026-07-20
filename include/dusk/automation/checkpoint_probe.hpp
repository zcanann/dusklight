#ifndef DUSK_AUTOMATION_CHECKPOINT_PROBE_HPP
#define DUSK_AUTOMATION_CHECKPOINT_PROBE_HPP

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

#include "dusk/automation/input_tape.hpp"
#include "dusk/automation/milestones.hpp"
#include "dusk/automation/state_checkpoint.hpp"

#include <dolphin/pad.h>

namespace dusk::automation {

class CheckpointProbe {
public:
    bool configure(std::size_t sourceFrame, std::size_t suffixTicks,
        std::filesystem::path resultPath, std::string& error);

    /** Called at the stable boundary before InputTapePlayer::tick(). */
    bool preInput(std::uint64_t& simulationTick, std::uint64_t& tapeFrame,
        std::uint64_t& preparedInputFrame, bool& tapeFrameApplied, std::string& error);
    /** Called after tape injection and before JUTGamePad consumes PAD state. */
    void overrideInputForAlternate();
    /** Called after game execution and deterministic clock advancement. */
    bool postSimulation(std::uint64_t simulationTick, std::uint64_t tapeFrame,
        std::uint64_t preparedInputFrame, bool tapeFrameApplied, std::string& error);

    [[nodiscard]] bool enabled() const { return mEnabled; }
    [[nodiscard]] bool completed() const { return mCompleted; }
    [[nodiscard]] bool failed() const { return mFailed; }
    [[nodiscard]] bool writeResult(std::string& error) const;

    enum class Phase {
        WaitingForSource,
        A1,
        RestoreB,
        B,
        RestoreA2,
        A2,
        Complete,
        Failed,
    };

private:

    struct HostSnapshot {
        InputTapePlayerState tapePlayer;
        MilestoneTracker milestones;
        PADAutomationState pad{};
        std::uint64_t simulationTick = 0;
        std::uint64_t tapeFrame = 0;
        std::uint64_t preparedInputFrame = 0;
        bool tapeFrameApplied = false;
    };

    bool captureSource(std::uint64_t simulationTick, std::uint64_t tapeFrame,
        std::uint64_t preparedInputFrame, bool tapeFrameApplied, std::string& error);
    bool restoreSource(std::uint64_t& simulationTick, std::uint64_t& tapeFrame,
        std::uint64_t& preparedInputFrame, bool& tapeFrameApplied, std::string& error);
    bool captureTickDigest(std::uint64_t simulationTick, std::uint64_t tapeFrame,
        std::uint64_t preparedInputFrame, bool tapeFrameApplied, std::string& output,
        std::vector<StateCheckpointEntryDigest>* entryDigests, std::string& error);
    void fail(std::string message);

    bool mEnabled = false;
    bool mCompleted = false;
    bool mFailed = false;
    Phase mPhase = Phase::WaitingForSource;
    std::size_t mSourceFrame = 0;
    std::size_t mSuffixTicks = 0;
    std::size_t mEpisodeTick = 0;
    std::filesystem::path mResultPath;
    StateCheckpoint mCheckpoint;
    StateCheckpointImage mImage;
    HostSnapshot mSource;
    std::vector<std::string> mA1Digests;
    std::vector<std::vector<StateCheckpointEntryDigest>> mA1EntryDigests;
    bool mBDiffered = false;
    std::size_t mFirstBDifference = 0;
    std::size_t mFirstDivergence = 0;
    std::string mExpectedDivergence;
    std::string mActualDivergence;
    std::vector<std::string> mDivergentEntries;
    std::string mError;
    std::uint64_t mCaptureMicros = 0;
    std::vector<std::uint64_t> mRestoreMicros;
};

CheckpointProbe& checkpoint_probe();

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_CHECKPOINT_PROBE_HPP
