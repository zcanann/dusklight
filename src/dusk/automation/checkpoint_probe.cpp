#include "dusk/automation/checkpoint_probe.hpp"
#include "dusk/automation/io_mode.hpp"
#include "dusk/audio/DuskAudioSystem.h"

#include <algorithm>
#include <chrono>
#include <fstream>
#include <string_view>
#include <system_error>

#include <nlohmann/json.hpp>
#include <xxhash.h>

#include <aurora/dvd.h>
#include <dolphin/os.h>
#include <JSystem/JFramework/JFWDisplay.h>
#include <JSystem/JKernel/JKRExpHeap.h>
#include <JSystem/JUtility/JUTDbPrint.h>
#include <JSystem/JUtility/JUTDirectPrint.h>
#include <JSystem/JUtility/JUTProcBar.h>

namespace dusk::automation {
namespace {

std::string hex_digest(const std::string_view value) {
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

std::string hex_context(const std::vector<std::byte>& bytes, const std::size_t offset) {
    constexpr char Hex[] = "0123456789abcdef";
    const std::size_t end = std::min(bytes.size(), offset + 16);
    std::string output;
    output.reserve((end - offset) * 2);
    for (std::size_t index = offset; index < end; ++index) {
        const std::uint8_t byte = std::to_integer<std::uint8_t>(bytes[index]);
        output.push_back(Hex[byte >> 4]);
        output.push_back(Hex[byte & 0xf]);
    }
    return output;
}

std::string phase_name(const CheckpointProbe::Phase phase) {
    switch (phase) {
    case CheckpointProbe::Phase::WaitingForSource: return "waiting_for_source";
    case CheckpointProbe::Phase::A1: return "a1";
    case CheckpointProbe::Phase::RestoreB: return "restore_b";
    case CheckpointProbe::Phase::B: return "b";
    case CheckpointProbe::Phase::RestoreA2: return "restore_a2";
    case CheckpointProbe::Phase::A2: return "a2";
    case CheckpointProbe::Phase::Complete: return "complete";
    case CheckpointProbe::Phase::Failed: return "failed";
    }
    return "unknown";
}

}  // namespace

bool CheckpointProbe::configure(const std::size_t sourceFrame, const std::size_t suffixTicks,
    std::filesystem::path resultPath, std::string& error) {
    if (mEnabled) {
        error = "checkpoint probe is already configured";
        return false;
    }
    if (suffixTicks == 0 || resultPath.empty()) {
        error = "checkpoint probe requires a nonzero suffix and result path";
        return false;
    }
    mEnabled = true;
    mSourceFrame = sourceFrame;
    mSuffixTicks = suffixTicks;
    mResultPath = std::move(resultPath);
    mA1Digests.reserve(suffixTicks);
    mA1EntryDigests.reserve(suffixTicks);
    mRestoreMicros.reserve(2);
    return true;
}

bool CheckpointProbe::captureSource(const std::uint64_t simulationTick,
    const std::uint64_t tapeFrame, const std::uint64_t preparedInputFrame,
    const bool tapeFrameApplied, std::string& error) {
    if (!synchronous_io_enabled() || !aurora_dvd_is_synchronous()) {
        error = "checkpoint capture requires synchronous simulation-thread I/O";
        return false;
    }
    mAudioCallbackQuiesced = dusk::audio::QuiesceForStateCheckpoint();
    if (!mAudioCallbackQuiesced) {
        error = "could not quiesce host audio callback for checkpoint capture";
        return false;
    }
    StateCheckpointError checkpointError = register_emulated_machine_checkpoint(mCheckpoint);
    if (checkpointError != StateCheckpointError::None) {
        error = state_checkpoint_error_message(checkpointError);
        return false;
    }
    auto& player = input_tape_player();
    mSource.tapePlayer = player.captureState();
    mSource.milestones = milestone_tracker();
    if (!PADCaptureAutomationState(&mSource.pad)) {
        error = "could not capture automation PAD state";
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
    return true;
}

bool CheckpointProbe::restoreSource(std::uint64_t& simulationTick, std::uint64_t& tapeFrame,
    std::uint64_t& preparedInputFrame, bool& tapeFrameApplied, std::string& error) {
    const auto start = std::chrono::steady_clock::now();
    const StateCheckpointError checkpointError = mCheckpoint.restore(mImage);
    const auto end = std::chrono::steady_clock::now();
    mRestoreMicros.push_back(static_cast<std::uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(end - start).count()));
    if (checkpointError != StateCheckpointError::None) {
        error = state_checkpoint_error_message(checkpointError);
        return false;
    }
    std::string restoredDigest;
    if (mCheckpoint.currentDigest(restoredDigest) != StateCheckpointError::None ||
        restoredDigest != mImage.digest)
    {
        error = "restored checkpoint bytes do not match the captured image";
        return false;
    }
    if (!input_tape_player().restoreState(mSource.tapePlayer)) {
        error = "input tape player rejected its checkpoint state";
        return false;
    }
    milestone_tracker() = mSource.milestones;
    if (!PADRestoreAutomationState(&mSource.pad)) {
        error = "automation PAD state restore failed";
        return false;
    }
    simulationTick = mSource.simulationTick;
    tapeFrame = mSource.tapeFrame;
    preparedInputFrame = mSource.preparedInputFrame;
    tapeFrameApplied = mSource.tapeFrameApplied;
    mEpisodeTick = 0;
    return true;
}

bool CheckpointProbe::preInput(std::uint64_t& simulationTick, std::uint64_t& tapeFrame,
    std::uint64_t& preparedInputFrame, bool& tapeFrameApplied, std::string& error) {
    if (!mEnabled || mCompleted || mFailed) return true;
    if (mPhase == Phase::WaitingForSource) {
        if (input_tape_player().nextFrameIndex() < mSourceFrame) return true;
        if (input_tape_player().nextFrameIndex() != mSourceFrame) {
            error = "input tape passed checkpoint source before capture";
            fail(error);
            return false;
        }
        if (!captureSource(simulationTick, tapeFrame, preparedInputFrame, tapeFrameApplied, error)) {
            fail(error);
            return false;
        }
        mPhase = Phase::A1;
        return true;
    }
    if (mPhase == Phase::RestoreB) {
        if (!restoreSource(simulationTick, tapeFrame, preparedInputFrame, tapeFrameApplied, error)) {
            fail(error);
            return false;
        }
        mPhase = Phase::B;
    } else if (mPhase == Phase::RestoreA2) {
        if (!restoreSource(simulationTick, tapeFrame, preparedInputFrame, tapeFrameApplied, error)) {
            fail(error);
            return false;
        }
        mPhase = Phase::A2;
    }
    return true;
}

void CheckpointProbe::overrideInputForAlternate() {
    if (!mEnabled || mPhase != Phase::B || mEpisodeTick != 0) return;
    const auto& player = input_tape_player();
    if (mSourceFrame >= player.tape().frames.size()) return;
    const InputFrame& source = player.tape().frames[mSourceFrame];
    const PADStatus neutral{};
    for (std::uint32_t port = 0; port < kInputPortCount; ++port) {
        if ((source.ownedPorts & (1u << port)) != 0) {
            PADSetAutomationStatus(port, &neutral);
        }
    }
}

bool CheckpointProbe::captureTickDigest(const std::uint64_t simulationTick,
    const std::uint64_t tapeFrame, const std::uint64_t preparedInputFrame,
    const bool tapeFrameApplied, std::string& output,
    std::vector<StateCheckpointEntryDigest>* const entryDigests, std::string& error) {
    std::string machine;
    const StateCheckpointError checkpointError =
        mCheckpoint.currentDigest(machine, entryDigests);
    if (checkpointError != StateCheckpointError::None) {
        error = state_checkpoint_error_message(checkpointError);
        return false;
    }
    const InputTapePlayerState player = input_tape_player().captureState();
    PADAutomationState pad{};
    if (!PADCaptureAutomationState(&pad)) {
        error = "could not capture automation PAD state for digest";
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
    const std::string host = nlohmann::json{
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
        {"milestones", serialize_milestone_result(milestone_tracker())},
    }.dump();
    output = hex_digest(host);
    return true;
}

bool CheckpointProbe::postSimulation(const std::uint64_t simulationTick,
    const std::uint64_t tapeFrame, const std::uint64_t preparedInputFrame,
    const bool tapeFrameApplied, std::string& error) {
    if (!mEnabled || mPhase == Phase::WaitingForSource || mCompleted || mFailed) return false;
    std::string digest;
    std::vector<StateCheckpointEntryDigest> entryDigests;
    std::vector<StateCheckpointEntryDigest>* const entryOutput =
        mPhase == Phase::A1 || mPhase == Phase::A2 ? &entryDigests : nullptr;
    if (!captureTickDigest(simulationTick, tapeFrame, preparedInputFrame, tapeFrameApplied,
            digest, entryOutput, error)) {
        fail(error);
        return true;
    }
    if (mPhase == Phase::A1) {
        mA1Digests.push_back(digest);
        mA1EntryDigests.push_back(std::move(entryDigests));
        if (mEpisodeTick == 0) {
            const StateCheckpointError captureError = mCheckpoint.capture(mA1FirstTickImage);
            if (captureError != StateCheckpointError::None) {
                error = state_checkpoint_error_message(captureError);
                fail(error);
                return true;
            }
        }
    } else if (mPhase == Phase::B) {
        if (!mBDiffered && digest != mA1Digests[mEpisodeTick]) {
            mBDiffered = true;
            mFirstBDifference = mEpisodeTick;
        }
    } else if (mPhase == Phase::A2 && digest != mA1Digests[mEpisodeTick]) {
        mFirstDivergence = mEpisodeTick;
        mExpectedDivergence = mA1Digests[mEpisodeTick];
        mActualDivergence = digest;
        const auto& expectedEntries = mA1EntryDigests[mEpisodeTick];
        if (entryDigests.size() == expectedEntries.size()) {
            for (std::size_t index = 0; index < entryDigests.size(); ++index) {
                if (entryDigests[index].name != expectedEntries[index].name ||
                    entryDigests[index].digest != expectedEntries[index].digest)
                {
                    mDivergentEntries.push_back(entryDigests[index].name);
                }
            }
        } else {
            mDivergentEntries.push_back("checkpoint_manifest");
        }
        if (mEpisodeTick == 0) {
            StateCheckpointImage actualImage;
            if (mCheckpoint.capture(actualImage) == StateCheckpointError::None &&
                actualImage.entries.size() == mA1FirstTickImage.entries.size())
            {
                constexpr std::size_t MaxDifferencesPerEntry = 8;
                for (std::size_t entryIndex = 0;
                     entryIndex < actualImage.entries.size(); ++entryIndex)
                {
                    const auto& expectedEntry = mA1FirstTickImage.entries[entryIndex];
                    const auto& actualEntry = actualImage.entries[entryIndex];
                    if (std::find(mDivergentEntries.begin(), mDivergentEntries.end(),
                            expectedEntry.name) == mDivergentEntries.end())
                    {
                        continue;
                    }
                    if (expectedEntry.name != actualEntry.name ||
                        expectedEntry.bytes.size() != actualEntry.bytes.size())
                    {
                        continue;
                    }
                    std::size_t entryDifferences = 0;
                    for (std::size_t offset = 0; offset < expectedEntry.bytes.size(); ++offset) {
                        if (expectedEntry.bytes[offset] == actualEntry.bytes[offset]) continue;
                        const std::size_t contextOffset = offset & ~std::size_t{7};
                        mByteDifferences.push_back({
                            .entry = expectedEntry.name,
                            .offset = offset,
                            .expected = std::to_integer<std::uint8_t>(expectedEntry.bytes[offset]),
                            .actual = std::to_integer<std::uint8_t>(actualEntry.bytes[offset]),
                            .contextOffset = contextOffset,
                            .expectedContext = hex_context(expectedEntry.bytes, contextOffset),
                            .actualContext = hex_context(actualEntry.bytes, contextOffset),
                        });
                        ByteDifference& difference = mByteDifferences.back();
                        if (expectedEntry.name == "mem1") {
                            auto* const address = static_cast<std::byte*>(
                                AuroraGetMEM1StorageAddress()) + offset;
                            JKRHeap* const heap = JKRHeap::findFromRoot(address);
                            if (heap != nullptr) {
                                difference.heapName = heap->getName();
                                difference.heapOffset = static_cast<std::size_t>(
                                    address - static_cast<std::byte*>(heap->getStartAddr()));
                                if (heap->getHeapType() == 'EXPH') {
                                    auto* const expHeap = static_cast<JKRExpHeap*>(heap);
                                    for (auto* block = expHeap->getUsedFirst(); block != nullptr;
                                         block = block->getNextBlock())
                                    {
                                        auto* const content = static_cast<std::byte*>(block->getContent());
                                        if (content <= address && address < content + block->getSize()) {
                                            difference.allocationOffset =
                                                static_cast<std::size_t>(address - content);
                                            difference.allocationSize = block->getSize();
                                            break;
                                        }
                                    }
                                }
                            }
                            const auto identifyObject = [&difference, address](
                                                            const char* const name,
                                                            const auto* const object) {
                                if (object == nullptr) return;
                                const auto* const begin = reinterpret_cast<const std::byte*>(object);
                                const auto* const end = begin + sizeof(*object);
                                if (begin <= address && address < end) {
                                    difference.objectName = name;
                                    difference.objectOffset =
                                        static_cast<std::size_t>(address - begin);
                                }
                            };
                            identifyObject("JFWDisplay", JFWDisplay::getManager());
                            identifyObject("JUTXfb", JUTXfb::getManager());
                            identifyObject("JUTProcBar", JUTProcBar::getManager());
                            identifyObject("JUTDbPrint", JUTDbPrint::getManager());
                            identifyObject("JUTDirectPrint", JUTDirectPrint::getManager());
                            if (JFWDisplay::getManager() != nullptr) {
                                identifyObject("JUTFader", JFWDisplay::getManager()->getFader());
                            }
                        }
                        if (++entryDifferences == MaxDifferencesPerEntry) break;
                    }
                }
            }
        }
        error = "A2 diverged from A1 after checkpoint restore";
        fail(error);
        return true;
    }

    ++mEpisodeTick;
    if (mEpisodeTick < mSuffixTicks) return false;
    if (mPhase == Phase::A1) {
        mPhase = Phase::RestoreB;
    } else if (mPhase == Phase::B) {
        if (!mBDiffered) {
            error = "alternate B input did not produce a distinct state";
            fail(error);
            return true;
        }
        mPhase = Phase::RestoreA2;
    } else if (mPhase == Phase::A2) {
        mPhase = Phase::Complete;
        mCompleted = true;
        return true;
    }
    return false;
}

void CheckpointProbe::fail(std::string message) {
    mFailed = true;
    mPhase = Phase::Failed;
    mError = std::move(message);
}

bool CheckpointProbe::writeResult(std::string& error) const {
    if (!mEnabled) return true;
    std::string sequence;
    for (const std::string& digest : mA1Digests) sequence += digest;
    nlohmann::json byteDifferences = nlohmann::json::array();
    for (const ByteDifference& difference : mByteDifferences) {
        byteDifferences.push_back({
            {"entry", difference.entry},
            {"offset", difference.offset},
            {"expected", difference.expected},
            {"actual", difference.actual},
            {"context_offset", difference.contextOffset},
            {"expected_context", difference.expectedContext},
            {"actual_context", difference.actualContext},
            {"heap_name", difference.heapName.empty() ? nlohmann::json(nullptr)
                                                          : nlohmann::json(difference.heapName)},
            {"heap_offset", difference.heapOffset},
            {"allocation_offset", difference.allocationOffset},
            {"allocation_size", difference.allocationSize},
            {"object_name", difference.objectName.empty() ? nlohmann::json(nullptr)
                                                              : nlohmann::json(difference.objectName)},
            {"object_offset", difference.objectOffset},
        });
    }
    nlohmann::json result{
        {"schema", "dusklight-checkpoint-probe/v1"},
        {"status", mCompleted ? "passed" : mFailed ? "failed" : "incomplete"},
        {"phase", phase_name(mPhase)},
        {"source_frame", mSourceFrame},
        {"suffix_ticks", mSuffixTicks},
        {"checkpoint_bytes", mCheckpoint.byteCount()},
        {"checkpoint_digest", mImage.digest},
        {"capture_micros", mCaptureMicros},
        {"restore_micros", mRestoreMicros},
        {"audio_callback_quiesced", mAudioCallbackQuiesced},
        {"a_sequence_digest", hex_digest(sequence)},
        {"b_differed", mBDiffered},
        {"first_b_difference_tick", mBDiffered ? nlohmann::json(mFirstBDifference) : nullptr},
        {"first_divergence_tick", mFailed && !mExpectedDivergence.empty()
                                      ? nlohmann::json(mFirstDivergence) : nullptr},
        {"expected_digest", mExpectedDivergence.empty() ? nlohmann::json(nullptr)
                                                          : nlohmann::json(mExpectedDivergence)},
        {"actual_digest", mActualDivergence.empty() ? nlohmann::json(nullptr)
                                                      : nlohmann::json(mActualDivergence)},
        {"divergent_entries", mDivergentEntries},
        {"first_byte_differences", std::move(byteDifferences)},
        {"error", mError.empty() ? nlohmann::json(nullptr) : nlohmann::json(mError)},
    };
    std::error_code filesystemError;
    const std::filesystem::path parent = mResultPath.parent_path();
    if (!parent.empty()) {
        std::filesystem::create_directories(parent, filesystemError);
        if (filesystemError) {
            error = "could not create checkpoint probe result directory: " +
                    filesystemError.message();
            return false;
        }
    }
    const std::filesystem::path temporary = mResultPath.string() + ".tmp";
    {
        std::ofstream stream(temporary, std::ios::binary | std::ios::trunc);
        if (!stream || !(stream << result.dump(2) << '\n')) {
            error = "could not write checkpoint probe temporary result";
            return false;
        }
    }
    std::filesystem::remove(mResultPath, filesystemError);
    filesystemError.clear();
    std::filesystem::rename(temporary, mResultPath, filesystemError);
    if (filesystemError) {
        error = "could not publish checkpoint probe result: " + filesystemError.message();
        return false;
    }
    return true;
}

CheckpointProbe& checkpoint_probe() {
    static CheckpointProbe probe;
    return probe;
}

}  // namespace dusk::automation
