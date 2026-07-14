#pragma once

#include "dusk/automation/input_tape.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>

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
inline constexpr std::uint16_t kInputControllerMinorVersion = 1;
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
    InvalidUnusedData,
    OverlappingReplaceLayers,
};

enum class InputControllerLayerKind : std::uint8_t {
    Bezier = 1,
    SeekPoint = 2,
    SeekActor = 3,
    Buttons = 4,
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
    float playerX = 0.0F;
    float playerY = 0.0F;
    float playerZ = 0.0F;
    bool cameraPresent = false;
    float cameraYawRadians = 0.0F;
    // Exact placed-actor selectors compare all eight canonical fixed-string
    // bytes. Short names are zero-padded; pointer identity is never involved.
    std::array<char, 8> stageName{};
    // Evaluation inspects at most kInputControllerMaximumActors entries.
    std::span<const ControllerActor> actors{};
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
