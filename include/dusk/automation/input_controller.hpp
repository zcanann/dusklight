#pragma once

#include "dusk/automation/input_tape.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>

#include "dusk/automation/typed_facts.hpp"

namespace dusk::automation {

inline constexpr std::array<std::uint8_t, 8> kInputControllerMagic{
    'D',
    'U',
    'S',
    'K',
    'C',
    'T',
    'R',
    'L',
};
inline constexpr std::uint16_t kInputControllerMajorVersion = 1;
inline constexpr std::uint16_t kInputControllerMinorVersion = 4;
inline constexpr std::size_t kInputControllerHeaderSize = 32;
inline constexpr std::size_t kInputControllerRecordSize = 64;
inline constexpr std::uint32_t kInputControllerMaximumDuration = 1'000'000;
inline constexpr std::size_t kInputControllerMaximumLayers = 32;
inline constexpr std::size_t kInputControllerMaximumActors = 256;

enum class InputControllerError {
    None,
    Truncated,
    BadMagic,
    UnsupportedVersion,
    InvalidHeaderSize,
    InvalidRecordSize,
    InvalidDuration,
    TooManyLayers,
    InvalidReservedData,
    InvalidPayloadLength,
    TrailingData,
    InvalidLayerKind,
    InvalidLayerBlend,
    InvalidLayerPort,
    InvalidLayerRange,
    InvalidFloat,
    InvalidStopRadius,
    InvalidMagnitude,
    InvalidButtonMask,
    InvalidActorSelector,
    InvalidProcessId,
    InvalidSetId,
    InvalidStageName,
    InvalidCoordinateFrame,
    InvalidPlaneNormal,
    InvalidResolvedTarget,
    InvalidMotionControl,
    InvalidHeading,
    InvalidDistance,
    InvalidUnusedData,
    OverlappingReplaceLayers,
    OverlappingSafetyClamps,
};

enum class InputControllerLayerKind : std::uint8_t {
    Bezier = 1,
    SeekPoint = 2,
    SeekActor = 3,
    Buttons = 4,
    SeekCoordinate = 5,
    SeekPlane = 6,
    SeekResolved = 7,
    Neutral = 8,
    Turn = 9,
    Brake = 10,
    Heading = 11,
    MaintainDistance = 12,
    Camera = 13,
    SafetyClamp = 14,
};

enum class InputControllerBlend : std::uint8_t {
    Replace = 0,
    Add = 1,
    Or = 2,
};

enum class InputControllerActorSelector : std::uint8_t {
    Nearest = 0,
    Process = 1,
    Placed = 2,
};

enum class InputControllerCoordinateFrame : std::uint8_t {
    World = 0,
    Player = 1,
    Camera = 2,
};

enum class InputControllerResolvedTarget : std::uint8_t {
    PathPoint = 0,
    Opening = 1,
};

enum class InputControllerTurnDirection : std::uint8_t {
    Left = 0,
    Right = 1,
};

enum class InputControllerHeadingMode : std::uint8_t {
    Align = 0,
    Maintain = 1,
};

struct ControllerActor {
    std::int16_t actorName = 0;
    // Stable IDs must be unique within an observation. Pointer values are not
    // suitable IDs because they change between processes and restored states.
    std::uint64_t stableId = 0;
    std::uint16_t setId = 0;
    std::int8_t homeRoom = 0;
    float x = 0.0F;
    float y = 0.0F;
    float z = 0.0F;
};

struct ControllerObservation {
    bool playerPresent = false;
    bool playerIsLink = false;
    float playerX = 0.0F;
    float playerY = 0.0F;
    float playerZ = 0.0F;
    bool playerYawPresent = false;
    float playerYawRadians = 0.0F;
    bool playerVelocityPresent = false;
    float playerVelocityX = 0.0F;
    float playerVelocityZ = 0.0F;
    bool cameraPresent = false;
    float cameraYawRadians = 0.0F;
    // Exact placed-actor selectors compare all eight canonical fixed-string
    // bytes. Short names are zero-padded; pointer identity is never involved.
    std::array<char, 8> stageName{};
    // Evaluation inspects at most kInputControllerMaximumActors entries.
    std::span<const ControllerActor> actors{};
    bool actorsTruncated = false;
};

enum class InputControllerTerminalReason : std::uint8_t {
    None = 0,
    TargetLost = 1,
};

struct InputControllerEvaluation {
    RawPadState input;
    InputControllerTerminalReason terminalReason = InputControllerTerminalReason::None;
    std::uint16_t terminalLayer = 0xffff;
};

inline constexpr std::uint16_t kInputControllerStepMajorVersion = 1;
inline constexpr std::uint16_t kInputControllerStepMinorVersion = 0;

enum class InputControllerObservationPhase : std::uint8_t {
    PreInput = 1,
};

enum class InputControllerStepError : std::uint8_t {
    None = 0,
    UnsupportedVersion = 1,
    InvalidPhase = 2,
    InvalidFrame = 3,
    InvalidFacts = 4,
};

// One immutable observation offered immediately before the named input frame.
// The copied ControllerObservation contains no writable game pointer.
struct InputControllerStepRequest {
    std::uint16_t majorVersion = kInputControllerStepMajorVersion;
    std::uint16_t minorVersion = kInputControllerStepMinorVersion;
    InputControllerObservationPhase phase = InputControllerObservationPhase::PreInput;
    std::uint64_t simulationTick = 0;
    std::uint64_t inputFrame = 0;
    std::uint32_t controllerFrame = 0;
    TypedFactResponse facts;
    ControllerObservation observation;
};

// A synchronous controller must return exactly one fixed-size raw PAD action
// for the request it was given. Echoed counters prevent a stale response from
// being consumed at another logical boundary.
struct InputControllerStepResponse {
    std::uint16_t majorVersion = kInputControllerStepMajorVersion;
    std::uint16_t minorVersion = kInputControllerStepMinorVersion;
    std::uint64_t simulationTick = 0;
    std::uint64_t inputFrame = 0;
    std::uint32_t controllerFrame = 0;
    InputControllerStepError error = InputControllerStepError::None;
    InputControllerEvaluation evaluation;
};

struct InputControllerLayer {
    InputControllerLayerKind kind = InputControllerLayerKind::Bezier;
    InputControllerBlend blend = InputControllerBlend::Replace;
    std::uint32_t start = 0;
    std::uint32_t duration = 0;

    // Bezier uses all eight values as four (x, y) control points.
    std::array<std::int16_t, 8> bezier{};

    // SeekPoint stores its target here. SeekActor uses the selector fields.
    std::int16_t actorName = 0;
    InputControllerActorSelector actorSelector = InputControllerActorSelector::Nearest;
    InputControllerCoordinateFrame coordinateFrame = InputControllerCoordinateFrame::World;
    InputControllerResolvedTarget resolvedTarget = InputControllerResolvedTarget::PathPoint;
    InputControllerTurnDirection turnDirection = InputControllerTurnDirection::Left;
    InputControllerHeadingMode headingMode = InputControllerHeadingMode::Align;
    std::uint32_t processId = 0;
    std::uint16_t setId = 0;
    std::int8_t homeRoom = 0;
    std::array<char, 8> placedStageName{};
    float targetX = 0.0F;
    float targetY = 0.0F;
    float targetZ = 0.0F;
    float offsetX = 0.0F;
    float offsetY = 0.0F;
    float offsetZ = 0.0F;
    float stopRadius = 0.0F;
    std::uint8_t magnitude = 0;
    std::uint64_t targetIdentity = 0;
    std::uint32_t targetSubIndex = 0;
    float headingRadians = 0.0F;
    float tolerance = 0.0F;
    float distance = 0.0F;
    std::int16_t cameraX = 0;
    std::int16_t cameraY = 0;
    std::uint8_t mainLimit = 127;
    std::uint8_t substickLimit = 127;

    std::uint16_t buttons = 0;
};

class InputControllerProgram {
public:
    [[nodiscard]] std::uint32_t duration() const { return mDuration; }
    [[nodiscard]] std::uint16_t layerCount() const { return mLayerCount; }
    [[nodiscard]] bool finished(std::uint32_t frame) const { return frame >= mDuration; }
    [[nodiscard]] std::span<const InputControllerLayer> layers() const {
        return std::span<const InputControllerLayer>(mLayers.data(), mLayerCount);
    }

    /** Evaluate one zero-based controller frame. This function does not allocate. */
    [[nodiscard]] RawPadState evaluate(
        std::uint32_t frame, const ControllerObservation& observation) const;
    [[nodiscard]] InputControllerEvaluation evaluateDetailed(
        std::uint32_t frame, const ControllerObservation& observation) const;
    /** Validate and answer one versioned pre-input request without allocating. */
    [[nodiscard]] InputControllerStepResponse respond(
        const InputControllerStepRequest& request) const;

private:
    std::uint32_t mDuration = 0;
    std::uint16_t mLayerCount = 0;
    std::array<InputControllerLayer, kInputControllerMaximumLayers> mLayers{};

    friend InputControllerError decode_input_controller(
        std::span<const std::uint8_t> bytes, InputControllerProgram& output);
};

[[nodiscard]] InputControllerError decode_input_controller(
    std::span<const std::uint8_t> bytes, InputControllerProgram& output);
[[nodiscard]] const char* input_controller_error_message(InputControllerError error);

}  // namespace dusk::automation
