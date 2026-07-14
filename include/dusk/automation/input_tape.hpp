#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <vector>

struct PADStatus;

namespace dusk::automation {

inline constexpr std::array<std::uint8_t, 8> kInputTapeMagic{
    'D', 'U', 'S', 'K', 'T', 'A', 'P', 'E',
};
// DUSKTAPE v2 stores the canonical 52-byte v1.2 frame stream in one zstd
// frame. The decoder continues to accept uncompressed v1.0-v1.2 tapes.
inline constexpr std::uint16_t kInputTapeMajorVersion = 2;
inline constexpr std::uint16_t kInputTapeMinorVersion = 0;
inline constexpr std::size_t kInputTapeHeaderSize = 40;
inline constexpr std::size_t kRawPadStateSize = 12;
inline constexpr std::size_t kInputFrameSize = 52;
inline constexpr std::size_t kInputPortCount = 4;

enum class RawPadFlags : std::uint8_t {
    None = 0,
    Connected = 1 << 0,
};

enum class InputFrameCondition : std::uint8_t {
    None = 0,
    NameEntryActive = 1,
    NameEntryCharacterSelect = 2,
    NameEntryInputReady = 3,
    FileSelectNoSaveReady = 4,
    FileSelectDataSelectReady = 5,
    FileSelectAcceptReady = 6,
};

const char* input_frame_condition_name(InputFrameCondition condition);

constexpr RawPadFlags operator|(RawPadFlags lhs, RawPadFlags rhs) {
    return static_cast<RawPadFlags>(static_cast<std::uint8_t>(lhs) | static_cast<std::uint8_t>(rhs));
}

constexpr bool has_flag(RawPadFlags value, RawPadFlags flag) {
    return (static_cast<std::uint8_t>(value) & static_cast<std::uint8_t>(flag)) != 0;
}

/**
 * Canonical controller state stored in a tape.
 *
 * This deliberately does not embed or serialize PADStatus. Its fields and
 * widths are part of the tape format and therefore remain stable when the
 * native platform ABI or Aurora's host-only input extensions change.
 */
struct RawPadState {
    std::uint16_t buttons = 0;
    std::int8_t stickX = 0;
    std::int8_t stickY = 0;
    std::int8_t substickX = 0;
    std::int8_t substickY = 0;
    std::uint8_t triggerLeft = 0;
    std::uint8_t triggerRight = 0;
    std::uint8_t analogA = 0;
    std::uint8_t analogB = 0;
    RawPadFlags flags = RawPadFlags::Connected;
    // Exact PAD error value. Minor version 0 tapes derive this from flags.
    std::int8_t error = 0;

    bool operator==(const RawPadState&) const = default;
};

// Converts the stable automation representation to Aurora's host pad ABI.
// Keeping this conversion shared prevents reactive controllers and tapes from
// drifting on error/analog field semantics.
PADStatus raw_pad_state_to_pad_status(const RawPadState& input);

struct InputFrame {
    // Bit N means automation owns controller port N for this tick.
    std::uint8_t ownedPorts = 0;
    // Until the condition is true, a conditioned frame alternates its declared
    // input with an owned-neutral tick. Neutral frames are ordinary waits;
    // non-neutral frames are trigger-safe pulses. A satisfied frame is skipped.
    InputFrameCondition condition = InputFrameCondition::None;
    std::uint16_t timeoutTicks = 0;
    std::array<RawPadState, kInputPortCount> pads{};

    bool operator==(const InputFrame&) const = default;
};

struct InputTape {
    std::uint32_t tickRateNumerator = 30;
    std::uint32_t tickRateDenominator = 1;
    std::vector<InputFrame> frames;

    bool operator==(const InputTape&) const = default;
};

enum class InputTapeError {
    None,
    Truncated,
    BadMagic,
    UnsupportedVersion,
    InvalidHeaderSize,
    InvalidFrameSize,
    InvalidTickRate,
    InvalidOwnedPorts,
    InvalidFrameCondition,
    InvalidPadFlags,
    InvalidCompressedPayload,
    TrailingData,
    TooManyFrames,
};

const char* input_tape_error_message(InputTapeError error);
InputTapeError validate_input_tape(const InputTape& tape);
bool input_tape_is_absolute(const InputTape& tape);
// Returns the maximum simulation ticks the tape can consume. Conditioned
// frames contribute their timeout; absolute frames contribute one tick.
bool input_tape_maximum_execution_ticks(const InputTape& tape, std::size_t& output);
InputTapeError decode_input_tape(std::span<const std::uint8_t> bytes, InputTape& output);
InputTapeError encode_input_tape(const InputTape& tape, std::vector<std::uint8_t>& output);

enum class TapeEndBehavior {
    Release,
    Hold,
    Loop,
};

enum class InputTapePlaybackError {
    None,
    ConditionTimedOut,
};

const char* input_tape_playback_error_message(InputTapePlaybackError error);

/**
 * Game-thread tape player. Loading may allocate; tick() never does.
 */
class InputTapePlayer {
public:
    void install(InputTape tape);
    InputTapeError install(std::span<const std::uint8_t> bytes);

    bool start(TapeEndBehavior endBehavior = TapeEndBehavior::Release);
    void stop();
    void handoffToLiveInput();
    void tick();

    bool isPlaying() const { return mPlaying; }
    bool hasFailed() const { return mPlaybackError != InputTapePlaybackError::None; }
    InputTapePlaybackError playbackError() const { return mPlaybackError; }
    std::size_t failedFrameIndex() const { return mFailedFrame; }
    InputFrameCondition failedCondition() const { return mFailedCondition; }
    std::size_t nextFrameIndex() const { return mNextFrame; }
    // tick() increments this when it installs a frame, before the caller runs
    // that frame's simulation tick. At a post-simulation call site it is also
    // the number of fully completed tape frames.
    std::size_t consumedFrameCount() const { return mNextFrame; }
    std::size_t frameCount() const { return mTape.frames.size(); }
    const InputTape& tape() const { return mTape; }

private:
    void apply(const InputFrame& frame);
    void applyNeutral(std::uint8_t ownedPorts);
    bool conditionSatisfied(InputFrameCondition condition) const;
    void advanceFrame();
    void releaseOwnedPorts();

    InputTape mTape;
    std::size_t mNextFrame = 0;
    std::uint8_t mOwnedPorts = 0;
    TapeEndBehavior mEndBehavior = TapeEndBehavior::Release;
    bool mPlaying = false;
    bool mReleasePending = false;
    std::uint16_t mConditionWaitTicks = 0;
    bool mConditionPulseNeutral = false;
    InputTapePlaybackError mPlaybackError = InputTapePlaybackError::None;
    std::size_t mFailedFrame = 0;
    InputFrameCondition mFailedCondition = InputFrameCondition::None;
};

// The main game input read advances this process-wide player once per tick.
InputTapePlayer& input_tape_player();

enum class InputRecordResult {
    Recorded,
    Inactive,
    CapacityExhausted,
};

/**
 * Records the post-PADRead, pre-clamp state produced by host input mapping.
 * Playback injects at that same boundary, then JUTGamePad applies PADClamp
 * exactly once. Recording post-clamp would make analog replay inexact because
 * PADClamp is intentionally not idempotent.
 *
 * arm() performs the only reservation while an automation prefix still owns
 * input. begin() starts capture at the subsequent exact handoff. start() is a
 * convenience for immediate recording. recordTick(), stop(), and take() do
 * not allocate; when the configured frame capacity is reached, recording
 * stops and reports CapacityExhausted while the session remains armed so
 * unrecorded mouse/gyro side channels stay suppressed.
 */
class InputTapeRecorder {
public:
    InputTapeError arm(std::uint8_t ownedPorts, std::size_t frameCapacity,
                       std::uint32_t tickRateNumerator = 30,
                       std::uint32_t tickRateDenominator = 1);
    bool begin();
    InputTapeError start(std::uint8_t ownedPorts, std::size_t frameCapacity,
                         std::uint32_t tickRateNumerator = 30, std::uint32_t tickRateDenominator = 1);
    InputRecordResult recordTick(std::span<const PADStatus, kInputPortCount> statuses);
    void stop();
    InputTape take();

    bool isArmed() const { return mArmed; }
    bool isRecording() const { return mRecording; }
    bool capacityExhausted() const { return mCapacityExhausted; }
    std::size_t frameCount() const { return mTape.frames.size(); }
    std::size_t frameCapacity() const { return mFrameCapacity; }
    std::uint8_t ownedPorts() const { return mOwnedPorts; }

private:
    InputTape mTape;
    std::size_t mFrameCapacity = 0;
    std::uint8_t mOwnedPorts = 0;
    bool mArmed = false;
    bool mRecording = false;
    bool mCapacityExhausted = false;
};

// The main game input read samples this process-wide recorder once per tick.
InputTapeRecorder& input_tape_recorder();

} // namespace dusk::automation
