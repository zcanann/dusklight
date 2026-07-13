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
inline constexpr std::uint16_t kInputTapeMajorVersion = 1;
inline constexpr std::uint16_t kInputTapeMinorVersion = 2;
inline constexpr std::size_t kInputTapeHeaderSize = 32;
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

struct InputFrame {
    // Bit N means automation owns controller port N for this tick.
    std::uint8_t ownedPorts = 0;
    // A conditioned frame owns the declared ports with neutral input until the
    // condition is true, then advances to the next frame in the same tick.
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
    TrailingData,
    TooManyFrames,
};

const char* input_tape_error_message(InputTapeError error);
InputTapeError validate_input_tape(const InputTape& tape);
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
    void tick();

    bool isPlaying() const { return mPlaying; }
    bool hasFailed() const { return mPlaybackError != InputTapePlaybackError::None; }
    InputTapePlaybackError playbackError() const { return mPlaybackError; }
    std::size_t failedFrameIndex() const { return mFailedFrame; }
    InputFrameCondition failedCondition() const { return mFailedCondition; }
    std::size_t nextFrameIndex() const { return mNextFrame; }
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
 * Records the post-PADRead, post-clamp state used by JUTGamePad.
 *
 * start() performs the only reservation. recordTick(), stop(), and take() do
 * not allocate; when the configured frame capacity is reached, recording
 * stops and reports CapacityExhausted.
 */
class InputTapeRecorder {
public:
    InputTapeError start(std::uint8_t ownedPorts, std::size_t frameCapacity,
                         std::uint32_t tickRateNumerator = 30, std::uint32_t tickRateDenominator = 1);
    InputRecordResult recordTick(std::span<const PADStatus, kInputPortCount> statuses);
    void stop();
    InputTape take();

    bool isRecording() const { return mRecording; }
    bool capacityExhausted() const { return mCapacityExhausted; }
    std::size_t frameCount() const { return mTape.frames.size(); }
    std::size_t frameCapacity() const { return mFrameCapacity; }
    std::uint8_t ownedPorts() const { return mOwnedPorts; }

private:
    InputTape mTape;
    std::size_t mFrameCapacity = 0;
    std::uint8_t mOwnedPorts = 0;
    bool mRecording = false;
    bool mCapacityExhausted = false;
};

// The main game input read samples this process-wide recorder once per tick.
InputTapeRecorder& input_tape_recorder();

} // namespace dusk::automation
