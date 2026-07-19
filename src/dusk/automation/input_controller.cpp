#include "dusk/automation/input_controller.hpp"

#include <algorithm>
#include <bit>
#include <cmath>
#include <cstdint>
#include <limits>

namespace dusk::automation {
namespace {

std::uint16_t read_u16(const std::uint8_t* input) {
    return static_cast<std::uint16_t>(input[0]) |
           static_cast<std::uint16_t>(static_cast<std::uint16_t>(input[1]) << 8);
}

std::int16_t read_i16(const std::uint8_t* input) {
    return std::bit_cast<std::int16_t>(read_u16(input));
}

std::uint32_t read_u32(const std::uint8_t* input) {
    return static_cast<std::uint32_t>(input[0]) | (static_cast<std::uint32_t>(input[1]) << 8) |
           (static_cast<std::uint32_t>(input[2]) << 16) |
           (static_cast<std::uint32_t>(input[3]) << 24);
}

std::uint64_t read_u64(const std::uint8_t* input) {
    return static_cast<std::uint64_t>(read_u32(input)) |
           (static_cast<std::uint64_t>(read_u32(input + 4)) << 32);
}

float read_f32(const std::uint8_t* input) {
    return std::bit_cast<float>(read_u32(input));
}

bool all_zero(const std::uint8_t* begin, const std::uint8_t* end) {
    return std::all_of(begin, end, [](const std::uint8_t value) { return value == 0; });
}

bool finite(const float value) {
    return std::isfinite(value);
}

bool canonical_nonempty_fixed_string(const std::uint8_t* begin, const std::size_t size) {
    if (size == 0 || begin[0] == 0) {
        return false;
    }
    bool sawZero = false;
    for (std::size_t index = 0; index < size; ++index) {
        if (begin[index] > 0x7f) {
            return false;
        }
        if (begin[index] == 0) {
            sawZero = true;
        } else if (sawZero) {
            return false;
        }
    }
    return true;
}

bool valid_seek(const InputControllerLayer& layer) {
    return finite(layer.offsetX) && finite(layer.offsetY) && finite(layer.offsetZ) &&
           finite(layer.stopRadius) && layer.stopRadius >= 0.0F && layer.magnitude >= 1 &&
           layer.magnitude <= 127;
}

InputControllerError validate_seek(const InputControllerLayer& layer) {
    if (!finite(layer.offsetX) || !finite(layer.offsetY) || !finite(layer.offsetZ) ||
        !finite(layer.stopRadius))
    {
        return InputControllerError::InvalidFloat;
    }
    if (layer.stopRadius < 0.0F) {
        return InputControllerError::InvalidStopRadius;
    }
    if (layer.magnitude < 1 || layer.magnitude > 127) {
        return InputControllerError::InvalidMagnitude;
    }
    return InputControllerError::None;
}

bool ranges_overlap(const InputControllerLayer& left, const InputControllerLayer& right) {
    const std::uint64_t leftEnd = static_cast<std::uint64_t>(left.start) + left.duration;
    const std::uint64_t rightEnd = static_cast<std::uint64_t>(right.start) + right.duration;
    return static_cast<std::uint64_t>(left.start) < rightEnd &&
           static_cast<std::uint64_t>(right.start) < leftEnd;
}

// A tiny allocation-free unsigned i128 implementation. Only multiplication by
// a 16-bit value, addition, subtraction, and comparison are needed here. This
// avoids compiler-runtime i128 division helpers that clang-cl does not provide.
struct WideMagnitude {
    std::uint64_t high = 0;
    std::uint64_t low = 0;
};

WideMagnitude multiply_wide(const std::uint64_t value, const std::uint32_t scalar) {
    constexpr std::uint64_t LowMask = 0xffff'ffffULL;
    const std::uint64_t lowProduct = (value & LowMask) * scalar;
    const std::uint64_t highProduct = (value >> 32) * scalar;
    const std::uint64_t shiftedHigh = highProduct << 32;
    const std::uint64_t low = lowProduct + shiftedHigh;
    return {
        .high = (highProduct >> 32) + (low < lowProduct ? 1ULL : 0ULL),
        .low = low,
    };
}

WideMagnitude add_wide(const WideMagnitude left, const WideMagnitude right) {
    const std::uint64_t low = left.low + right.low;
    return {
        .high = left.high + right.high + (low < left.low ? 1ULL : 0ULL),
        .low = low,
    };
}

int compare_wide(const WideMagnitude left, const WideMagnitude right) {
    if (left.high != right.high) {
        return left.high < right.high ? -1 : 1;
    }
    if (left.low != right.low) {
        return left.low < right.low ? -1 : 1;
    }
    return 0;
}

WideMagnitude subtract_wide(const WideMagnitude left, const WideMagnitude right) {
    return {
        .high = left.high - right.high - (left.low < right.low ? 1ULL : 0ULL),
        .low = left.low - right.low,
    };
}

void add_signed_term(WideMagnitude& positive, WideMagnitude& negative,
    const std::uint64_t coefficient, const std::int16_t value) {
    const std::int32_t widened = value;
    const std::uint32_t magnitude = static_cast<std::uint32_t>(widened < 0 ? -widened : widened);
    if (widened < 0) {
        negative = add_wide(negative, multiply_wide(coefficient, magnitude));
    } else {
        positive = add_wide(positive, multiply_wide(coefficient, magnitude));
    }
}

std::int64_t round_wide_ratio_ties_away_from_zero(
    const WideMagnitude positive, const WideMagnitude negative, const std::uint64_t denominator) {
    const int signComparison = compare_wide(positive, negative);
    if (signComparison == 0) {
        return 0;
    }
    const bool negativeResult = signComparison < 0;
    const WideMagnitude magnitude =
        negativeResult ? subtract_wide(negative, positive) : subtract_wide(positive, negative);

    // A cubic Bezier lies within the convex hull of its i16 control points,
    // so the magnitude of the quotient is at most 32768.
    std::uint32_t low = 0;
    std::uint32_t high = 32'769;
    while (low + 1 < high) {
        const std::uint32_t middle = low + (high - low) / 2;
        if (compare_wide(multiply_wide(denominator, middle), magnitude) <= 0) {
            low = middle;
        } else {
            high = middle;
        }
    }
    const WideMagnitude remainder = subtract_wide(magnitude, multiply_wide(denominator, low));
    const bool roundAway = remainder.high != 0 || remainder.low * 2 >= denominator;
    const std::int64_t rounded = static_cast<std::int64_t>(low + (roundAway ? 1U : 0U));
    return negativeResult ? -rounded : rounded;
}

std::int64_t evaluate_bezier_axis(
    const InputControllerLayer& layer, const std::uint32_t localFrame, const std::size_t axis) {
    if (layer.duration == 1) {
        return layer.bezier[axis];
    }

    const std::uint64_t denominatorRoot = layer.duration - 1;
    const std::uint64_t u = localFrame;
    const std::uint64_t v = denominatorRoot - u;
    const std::uint64_t denominator = denominatorRoot * denominatorRoot * denominatorRoot;
    const std::array<std::uint64_t, 4> coefficients{
        v * v * v,
        3 * v * v * u,
        3 * v * u * u,
        u * u * u,
    };
    WideMagnitude positive;
    WideMagnitude negative;
    for (std::size_t point = 0; point < coefficients.size(); ++point) {
        add_signed_term(positive, negative, coefficients[point], layer.bezier[point * 2 + axis]);
    }
    return round_wide_ratio_ties_away_from_zero(positive, negative, denominator);
}

struct StickValue {
    std::int64_t x = 0;
    std::int64_t y = 0;
};

std::int64_t rounded_stick_component(const double component, const std::uint8_t magnitude) {
    return std::lround(component * static_cast<double>(magnitude));
}

StickValue seek(const InputControllerLayer& layer, const ControllerObservation& observation,
    const float targetX, const float targetZ) {
    if (!observation.playerPresent || !observation.cameraPresent || !finite(observation.playerX) ||
        !finite(observation.playerZ) || !finite(observation.cameraYawRadians) || !finite(targetX) ||
        !finite(targetZ))
    {
        return {};
    }

    const double deltaX = static_cast<double>(targetX) - observation.playerX;
    const double deltaZ = static_cast<double>(targetZ) - observation.playerZ;
    const double distance = std::hypot(deltaX, deltaZ);
    if (distance <= static_cast<double>(layer.stopRadius)) {
        return {};
    }

    const double worldAngle = std::atan2(deltaX, deltaZ);
    const double relativeAngle = worldAngle - observation.cameraYawRadians;
    return {
        // The native PAD X axis is opposite the world/camera-space right axis:
        // negative raw X steers toward world +X when camera yaw is zero.
        .x = rounded_stick_component(-std::sin(relativeAngle), layer.magnitude),
        .y = rounded_stick_component(std::cos(relativeAngle), layer.magnitude),
    };
}

StickValue world_heading_stick(const InputControllerLayer& layer,
    const ControllerObservation& observation, const double worldHeading) {
    if (!observation.cameraPresent || !finite(observation.cameraYawRadians) ||
        !std::isfinite(worldHeading))
    {
        return {};
    }
    const double relativeAngle = worldHeading - observation.cameraYawRadians;
    return {
        .x = rounded_stick_component(-std::sin(relativeAngle), layer.magnitude),
        .y = rounded_stick_component(std::cos(relativeAngle), layer.magnitude),
    };
}

double wrap_angle(double value) {
    constexpr double Pi = 3.14159265358979323846;
    constexpr double Tau = 2.0 * Pi;
    value = std::fmod(value + Pi, Tau);
    if (value < 0.0) {
        value += Tau;
    }
    return value - Pi;
}

bool resolve_heading(const InputControllerLayer& layer, const ControllerObservation& observation,
    double& output) {
    switch (layer.coordinateFrame) {
    case InputControllerCoordinateFrame::World:
        output = layer.headingRadians;
        return true;
    case InputControllerCoordinateFrame::Player:
        if (!observation.playerYawPresent || !finite(observation.playerYawRadians)) {
            return false;
        }
        output = static_cast<double>(observation.playerYawRadians) + layer.headingRadians;
        return std::isfinite(output);
    case InputControllerCoordinateFrame::Camera:
        if (!observation.cameraPresent || !finite(observation.cameraYawRadians)) {
            return false;
        }
        output = static_cast<double>(observation.cameraYawRadians) + layer.headingRadians;
        return std::isfinite(output);
    }
    return false;
}

struct WorldPoint {
    float x = 0.0F;
    float y = 0.0F;
    float z = 0.0F;
};

bool frame_yaw(const InputControllerCoordinateFrame frame, const ControllerObservation& observation,
    float& yaw) {
    switch (frame) {
    case InputControllerCoordinateFrame::World:
        yaw = 0.0F;
        return true;
    case InputControllerCoordinateFrame::Player:
        yaw = observation.playerYawRadians;
        return observation.playerYawPresent && finite(yaw);
    case InputControllerCoordinateFrame::Camera:
        yaw = observation.cameraYawRadians;
        return observation.cameraPresent && finite(yaw);
    }
    return false;
}

bool resolve_point(const InputControllerCoordinateFrame frame,
    const ControllerObservation& observation, const float x, const float y, const float z,
    WorldPoint& output) {
    if (!observation.playerPresent || !finite(observation.playerX) ||
        !finite(observation.playerY) || !finite(observation.playerZ) || !finite(x) || !finite(y) ||
        !finite(z))
    {
        return false;
    }
    if (frame == InputControllerCoordinateFrame::World) {
        output = {.x = x, .y = y, .z = z};
        return true;
    }
    float yaw = 0.0F;
    if (!frame_yaw(frame, observation, yaw)) {
        return false;
    }
    const double sine = std::sin(static_cast<double>(yaw));
    const double cosine = std::cos(static_cast<double>(yaw));
    output = {
        .x = static_cast<float>(observation.playerX + cosine * x + sine * z),
        .y = observation.playerY + y,
        .z = static_cast<float>(observation.playerZ - sine * x + cosine * z),
    };
    return finite(output.x) && finite(output.y) && finite(output.z);
}

bool resolve_vector(const InputControllerCoordinateFrame frame,
    const ControllerObservation& observation, const float x, const float y, const float z,
    WorldPoint& output) {
    if (!finite(x) || !finite(y) || !finite(z)) {
        return false;
    }
    float yaw = 0.0F;
    if (!frame_yaw(frame, observation, yaw)) {
        return false;
    }
    const double sine = std::sin(static_cast<double>(yaw));
    const double cosine = std::cos(static_cast<double>(yaw));
    output = {
        .x = static_cast<float>(cosine * x + sine * z),
        .y = y,
        .z = static_cast<float>(-sine * x + cosine * z),
    };
    return finite(output.x) && finite(output.y) && finite(output.z);
}

StickValue evaluate_stick_layer(const InputControllerLayer& layer, const std::uint32_t localFrame,
    const ControllerObservation& observation, bool& exactTargetLost) {
    exactTargetLost = false;
    switch (layer.kind) {
    case InputControllerLayerKind::Bezier:
        return {
            .x = evaluate_bezier_axis(layer, localFrame, 0),
            .y = evaluate_bezier_axis(layer, localFrame, 1),
        };
    case InputControllerLayerKind::SeekPoint:
        return seek(
            layer, observation, layer.targetX + layer.offsetX, layer.targetZ + layer.offsetZ);
    case InputControllerLayerKind::SeekActor: {
        if (!observation.playerPresent || !finite(observation.playerX) ||
            !finite(observation.playerZ))
        {
            return {};
        }

        const ControllerActor* selected = nullptr;
        double selectedDistanceSquared = std::numeric_limits<double>::infinity();
        const std::span actors = observation.actors.first(
            std::min(observation.actors.size(), kInputControllerMaximumActors));
        for (const ControllerActor& actor : actors) {
            if (actor.actorName != layer.actorName || !finite(actor.x) || !finite(actor.z)) {
                continue;
            }

            if (layer.actorSelector == InputControllerActorSelector::Process) {
                if (actor.stableId == layer.processId) {
                    selected = &actor;
                    break;
                }
                continue;
            }
            if (layer.actorSelector == InputControllerActorSelector::Placed) {
                if (observation.stageName == layer.placedStageName && actor.setId == layer.setId &&
                    actor.homeRoom == layer.homeRoom &&
                    (selected == nullptr || actor.stableId < selected->stableId))
                {
                    selected = &actor;
                }
                continue;
            }

            const double deltaX = static_cast<double>(actor.x) - observation.playerX;
            const double deltaZ = static_cast<double>(actor.z) - observation.playerZ;
            const double distanceSquared = deltaX * deltaX + deltaZ * deltaZ;
            if (selected == nullptr || distanceSquared < selectedDistanceSquared ||
                (distanceSquared == selectedDistanceSquared && actor.stableId < selected->stableId))
            {
                selected = &actor;
                selectedDistanceSquared = distanceSquared;
            }
        }
        if (selected == nullptr) {
            exactTargetLost = layer.actorSelector != InputControllerActorSelector::Nearest &&
                              !observation.actorsTruncated;
            return {};
        }
        return seek(layer, observation, selected->x + layer.offsetX, selected->z + layer.offsetZ);
    }
    case InputControllerLayerKind::SeekCoordinate: {
        WorldPoint target;
        WorldPoint offset;
        if (!resolve_point(layer.coordinateFrame, observation, layer.targetX, layer.targetY,
                layer.targetZ, target) ||
            !resolve_vector(layer.coordinateFrame, observation, layer.offsetX, layer.offsetY,
                layer.offsetZ, offset))
        {
            return {};
        }
        return seek(layer, observation, target.x + offset.x, target.z + offset.z);
    }
    case InputControllerLayerKind::SeekPlane: {
        WorldPoint point;
        WorldPoint normal;
        if (!resolve_point(layer.coordinateFrame, observation, layer.targetX, layer.targetY,
                layer.targetZ, point) ||
            !resolve_vector(layer.coordinateFrame, observation, layer.offsetX, layer.offsetY,
                layer.offsetZ, normal))
        {
            return {};
        }
        const double normalSquared = static_cast<double>(normal.x) * normal.x +
                                     static_cast<double>(normal.z) * normal.z;
        if (!(normalSquared > 0.0) || !std::isfinite(normalSquared)) {
            return {};
        }
        const double signedScale =
            ((static_cast<double>(observation.playerX) - point.x) * normal.x +
                (static_cast<double>(observation.playerZ) - point.z) * normal.z) /
            normalSquared;
        const float targetX =
            static_cast<float>(observation.playerX - signedScale * normal.x);
        const float targetZ =
            static_cast<float>(observation.playerZ - signedScale * normal.z);
        return seek(layer, observation, targetX, targetZ);
    }
    case InputControllerLayerKind::SeekResolved:
        return seek(
            layer, observation, layer.targetX + layer.offsetX, layer.targetZ + layer.offsetZ);
    case InputControllerLayerKind::Neutral:
        return {};
    case InputControllerLayerKind::Turn:
        return {
            .x = layer.turnDirection == InputControllerTurnDirection::Left ? -layer.magnitude :
                                                                                layer.magnitude,
            .y = 0,
        };
    case InputControllerLayerKind::Brake: {
        if (!observation.playerVelocityPresent || !finite(observation.playerVelocityX) ||
            !finite(observation.playerVelocityZ))
        {
            return {};
        }
        const double speed = std::hypot(
            static_cast<double>(observation.playerVelocityX), observation.playerVelocityZ);
        if (speed <= layer.stopRadius) {
            return {};
        }
        const double oppositeHeading =
            std::atan2(-observation.playerVelocityX, -observation.playerVelocityZ);
        return world_heading_stick(layer, observation, oppositeHeading);
    }
    case InputControllerLayerKind::Heading: {
        double heading = 0.0;
        if (!resolve_heading(layer, observation, heading)) {
            return {};
        }
        if (layer.headingMode == InputControllerHeadingMode::Align) {
            if (!observation.playerYawPresent || !finite(observation.playerYawRadians) ||
                std::fabs(wrap_angle(heading - observation.playerYawRadians)) <= layer.tolerance)
            {
                return {};
            }
        }
        return world_heading_stick(layer, observation, heading);
    }
    case InputControllerLayerKind::MaintainDistance: {
        WorldPoint target;
        if (!resolve_point(layer.coordinateFrame, observation, layer.targetX, layer.targetY,
                layer.targetZ, target) || !observation.playerPresent)
        {
            return {};
        }
        const double deltaX = static_cast<double>(target.x) - observation.playerX;
        const double deltaZ = static_cast<double>(target.z) - observation.playerZ;
        const double actualDistance = std::hypot(deltaX, deltaZ);
        if (actualDistance > static_cast<double>(layer.distance + layer.tolerance)) {
            return seek(layer, observation, target.x, target.z);
        }
        if (actualDistance < static_cast<double>(layer.distance - layer.tolerance) &&
            actualDistance > 0.0)
        {
            const double awayHeading = std::atan2(-deltaX, -deltaZ);
            return world_heading_stick(layer, observation, awayHeading);
        }
        return {};
    }
    case InputControllerLayerKind::Camera:
    case InputControllerLayerKind::SafetyClamp:
    case InputControllerLayerKind::Buttons:
        break;
    }
    return {};
}

std::int8_t clamp_stick(const std::int64_t value) {
    return static_cast<std::int8_t>(std::clamp<std::int64_t>(
        value, std::numeric_limits<std::int8_t>::min(), std::numeric_limits<std::int8_t>::max()));
}

}  // namespace

InputControllerError decode_input_controller(
    const std::span<const std::uint8_t> bytes, InputControllerProgram& output) {
    if (bytes.size() < kInputControllerHeaderSize) {
        return InputControllerError::Truncated;
    }
    if (!std::equal(kInputControllerMagic.begin(), kInputControllerMagic.end(), bytes.begin())) {
        return InputControllerError::BadMagic;
    }
    const std::uint16_t minorVersion = read_u16(bytes.data() + 10);
    if (read_u16(bytes.data() + 8) != kInputControllerMajorVersion ||
        minorVersion > kInputControllerMinorVersion)
    {
        return InputControllerError::UnsupportedVersion;
    }
    if (read_u16(bytes.data() + 12) != kInputControllerHeaderSize) {
        return InputControllerError::InvalidHeaderSize;
    }
    if (read_u16(bytes.data() + 14) != kInputControllerRecordSize) {
        return InputControllerError::InvalidRecordSize;
    }

    const std::uint32_t duration = read_u32(bytes.data() + 16);
    const std::uint16_t layerCount = read_u16(bytes.data() + 20);
    const std::uint32_t payloadLength = read_u32(bytes.data() + 24);
    if (duration == 0 || duration > kInputControllerMaximumDuration) {
        return InputControllerError::InvalidDuration;
    }
    if (layerCount > kInputControllerMaximumLayers) {
        return InputControllerError::TooManyLayers;
    }
    if (read_u16(bytes.data() + 22) != 0 || read_u32(bytes.data() + 28) != 0) {
        return InputControllerError::InvalidReservedData;
    }
    const std::uint32_t expectedPayloadLength =
        static_cast<std::uint32_t>(layerCount) * kInputControllerRecordSize;
    if (payloadLength != expectedPayloadLength) {
        return InputControllerError::InvalidPayloadLength;
    }
    const std::size_t expectedSize = kInputControllerHeaderSize + expectedPayloadLength;
    if (bytes.size() < expectedSize) {
        return InputControllerError::Truncated;
    }
    if (bytes.size() > expectedSize) {
        return InputControllerError::TrailingData;
    }

    InputControllerProgram candidate;
    candidate.mDuration = duration;
    candidate.mLayerCount = layerCount;
    for (std::uint16_t index = 0; index < layerCount; ++index) {
        const std::uint8_t* record =
            bytes.data() + kInputControllerHeaderSize + index * kInputControllerRecordSize;
        InputControllerLayer& layer = candidate.mLayers[index];

        if (record[0] < static_cast<std::uint8_t>(InputControllerLayerKind::Bezier) ||
            record[0] > static_cast<std::uint8_t>(InputControllerLayerKind::SafetyClamp) ||
            (minorVersion < 2 &&
                record[0] > static_cast<std::uint8_t>(InputControllerLayerKind::Buttons)) ||
            (minorVersion < 3 &&
                record[0] > static_cast<std::uint8_t>(InputControllerLayerKind::SeekResolved)) ||
            (minorVersion < 4 &&
                record[0] > static_cast<std::uint8_t>(InputControllerLayerKind::MaintainDistance)))
        {
            return InputControllerError::InvalidLayerKind;
        }
        layer.kind = static_cast<InputControllerLayerKind>(record[0]);
        if (record[1] > static_cast<std::uint8_t>(InputControllerBlend::Or)) {
            return InputControllerError::InvalidLayerBlend;
        }
        layer.blend = static_cast<InputControllerBlend>(record[1]);
        if (record[2] != 0) {
            return InputControllerError::InvalidLayerPort;
        }
        if (record[3] != 0) {
            return InputControllerError::InvalidReservedData;
        }
        layer.start = read_u32(record + 4);
        layer.duration = read_u32(record + 8);
        if (layer.duration == 0 ||
            static_cast<std::uint64_t>(layer.start) + layer.duration > duration)
        {
            return InputControllerError::InvalidLayerRange;
        }

        if (layer.kind == InputControllerLayerKind::Buttons) {
            if (layer.blend != InputControllerBlend::Or) {
                return InputControllerError::InvalidLayerBlend;
            }
            layer.buttons = read_u16(record + 12);
            if (layer.buttons == 0) {
                return InputControllerError::InvalidButtonMask;
            }
            if (!all_zero(record + 14, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }
        if (layer.blend == InputControllerBlend::Or) {
            return InputControllerError::InvalidLayerBlend;
        }

        if (layer.kind == InputControllerLayerKind::Camera) {
            layer.cameraX = read_i16(record + 12);
            layer.cameraY = read_i16(record + 14);
            if (layer.cameraX < -128 || layer.cameraX > 127 || layer.cameraY < -128 ||
                layer.cameraY > 127)
            {
                return InputControllerError::InvalidMagnitude;
            }
            if (!all_zero(record + 16, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::SafetyClamp) {
            if (layer.blend != InputControllerBlend::Replace) {
                return InputControllerError::InvalidLayerBlend;
            }
            layer.mainLimit = record[12];
            layer.substickLimit = record[13];
            if (layer.mainLimit > 127 || layer.substickLimit > 127) {
                return InputControllerError::InvalidMagnitude;
            }
            if (!all_zero(record + 14, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::Bezier) {
            for (std::size_t point = 0; point < layer.bezier.size(); ++point) {
                layer.bezier[point] = read_i16(record + 12 + point * 2);
            }
            if (!all_zero(record + 28, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::SeekPoint) {
            layer.targetX = read_f32(record + 12);
            layer.targetY = read_f32(record + 16);
            layer.targetZ = read_f32(record + 20);
            layer.offsetX = read_f32(record + 24);
            layer.offsetY = read_f32(record + 28);
            layer.offsetZ = read_f32(record + 32);
            layer.stopRadius = read_f32(record + 36);
            layer.magnitude = record[40];
            if (!finite(layer.targetX) || !finite(layer.targetY) || !finite(layer.targetZ) ||
                !valid_seek(layer))
            {
                return !finite(layer.stopRadius) || !finite(layer.targetX) ||
                               !finite(layer.targetY) || !finite(layer.targetZ) ||
                               !finite(layer.offsetX) || !finite(layer.offsetY) ||
                               !finite(layer.offsetZ) ?
                           InputControllerError::InvalidFloat :
                       layer.stopRadius < 0.0F ? InputControllerError::InvalidStopRadius :
                                                 InputControllerError::InvalidMagnitude;
            }
            if (!all_zero(record + 41, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::SeekCoordinate ||
            layer.kind == InputControllerLayerKind::SeekPlane)
        {
            if (record[12] > static_cast<std::uint8_t>(InputControllerCoordinateFrame::Camera)) {
                return InputControllerError::InvalidCoordinateFrame;
            }
            if (!all_zero(record + 13, record + 16)) {
                return InputControllerError::InvalidReservedData;
            }
            layer.coordinateFrame = static_cast<InputControllerCoordinateFrame>(record[12]);
            layer.targetX = read_f32(record + 16);
            layer.targetY = read_f32(record + 20);
            layer.targetZ = read_f32(record + 24);
            layer.offsetX = read_f32(record + 28);
            layer.offsetY = read_f32(record + 32);
            layer.offsetZ = read_f32(record + 36);
            layer.stopRadius = read_f32(record + 40);
            layer.magnitude = record[44];
            if (!finite(layer.targetX) || !finite(layer.targetY) || !finite(layer.targetZ)) {
                return InputControllerError::InvalidFloat;
            }
            if (const InputControllerError error = validate_seek(layer);
                error != InputControllerError::None)
            {
                return error;
            }
            if (layer.kind == InputControllerLayerKind::SeekPlane) {
                const double horizontalNormalSquared =
                    static_cast<double>(layer.offsetX) * layer.offsetX +
                    static_cast<double>(layer.offsetZ) * layer.offsetZ;
                if (!(horizontalNormalSquared > std::numeric_limits<double>::epsilon())) {
                    return InputControllerError::InvalidPlaneNormal;
                }
            }
            if (!all_zero(record + 45, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::SeekResolved) {
            if (record[12] > static_cast<std::uint8_t>(InputControllerResolvedTarget::Opening) ||
                !all_zero(record + 13, record + 16))
            {
                return InputControllerError::InvalidResolvedTarget;
            }
            layer.resolvedTarget = static_cast<InputControllerResolvedTarget>(record[12]);
            layer.targetIdentity = read_u64(record + 16);
            layer.targetSubIndex = read_u32(record + 24);
            layer.targetX = read_f32(record + 28);
            layer.targetY = read_f32(record + 32);
            layer.targetZ = read_f32(record + 36);
            layer.offsetX = read_f32(record + 40);
            layer.offsetY = read_f32(record + 44);
            layer.offsetZ = read_f32(record + 48);
            layer.stopRadius = read_f32(record + 52);
            layer.magnitude = record[56];
            if (layer.targetIdentity == 0 ||
                (layer.resolvedTarget == InputControllerResolvedTarget::Opening &&
                    layer.targetSubIndex != 0))
            {
                return InputControllerError::InvalidResolvedTarget;
            }
            if (!finite(layer.targetX) || !finite(layer.targetY) || !finite(layer.targetZ)) {
                return InputControllerError::InvalidFloat;
            }
            if (const InputControllerError error = validate_seek(layer);
                error != InputControllerError::None)
            {
                return error;
            }
            if (!all_zero(record + 57, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::Neutral) {
            if (layer.blend != InputControllerBlend::Replace) {
                return InputControllerError::InvalidLayerBlend;
            }
            if (!all_zero(record + 12, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::Turn) {
            if (record[12] > static_cast<std::uint8_t>(InputControllerTurnDirection::Right)) {
                return InputControllerError::InvalidMotionControl;
            }
            layer.turnDirection = static_cast<InputControllerTurnDirection>(record[12]);
            layer.magnitude = record[13];
            if (layer.magnitude < 1 || layer.magnitude > 127) {
                return InputControllerError::InvalidMagnitude;
            }
            if (!all_zero(record + 14, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::Brake) {
            layer.magnitude = record[12];
            layer.stopRadius = read_f32(record + 16);
            if (!all_zero(record + 13, record + 16)) {
                return InputControllerError::InvalidReservedData;
            }
            if (!finite(layer.stopRadius)) {
                return InputControllerError::InvalidFloat;
            }
            if (layer.stopRadius < 0.0F) {
                return InputControllerError::InvalidMotionControl;
            }
            if (layer.magnitude < 1 || layer.magnitude > 127) {
                return InputControllerError::InvalidMagnitude;
            }
            if (!all_zero(record + 20, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::Heading) {
            if (record[12] > static_cast<std::uint8_t>(InputControllerHeadingMode::Maintain) ||
                record[13] > static_cast<std::uint8_t>(InputControllerCoordinateFrame::Camera) ||
                record[15] != 0)
            {
                return InputControllerError::InvalidMotionControl;
            }
            layer.headingMode = static_cast<InputControllerHeadingMode>(record[12]);
            layer.coordinateFrame = static_cast<InputControllerCoordinateFrame>(record[13]);
            layer.magnitude = record[14];
            layer.headingRadians = read_f32(record + 16);
            layer.tolerance = read_f32(record + 20);
            constexpr float Pi = 3.14159265358979323846F;
            if (!finite(layer.headingRadians) || layer.headingRadians < -Pi ||
                layer.headingRadians > Pi || !finite(layer.tolerance) || layer.tolerance < 0.0F ||
                layer.tolerance > Pi)
            {
                return InputControllerError::InvalidHeading;
            }
            if (layer.headingMode == InputControllerHeadingMode::Maintain &&
                std::bit_cast<std::uint32_t>(layer.tolerance) != 0)
            {
                return InputControllerError::InvalidHeading;
            }
            if (layer.magnitude < 1 || layer.magnitude > 127) {
                return InputControllerError::InvalidMagnitude;
            }
            if (!all_zero(record + 24, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        if (layer.kind == InputControllerLayerKind::MaintainDistance) {
            if (record[12] > static_cast<std::uint8_t>(InputControllerCoordinateFrame::Camera) ||
                record[14] != 0 || record[15] != 0)
            {
                return InputControllerError::InvalidMotionControl;
            }
            layer.coordinateFrame = static_cast<InputControllerCoordinateFrame>(record[12]);
            layer.magnitude = record[13];
            layer.targetX = read_f32(record + 16);
            layer.targetY = read_f32(record + 20);
            layer.targetZ = read_f32(record + 24);
            layer.distance = read_f32(record + 28);
            layer.tolerance = read_f32(record + 32);
            if (!finite(layer.targetX) || !finite(layer.targetY) || !finite(layer.targetZ) ||
                !finite(layer.distance) || !finite(layer.tolerance))
            {
                return InputControllerError::InvalidFloat;
            }
            if (layer.distance < 0.0F || layer.tolerance < 0.0F ||
                layer.tolerance > layer.distance)
            {
                return InputControllerError::InvalidDistance;
            }
            if (layer.magnitude < 1 || layer.magnitude > 127) {
                return InputControllerError::InvalidMagnitude;
            }
            if (!all_zero(record + 36, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
            continue;
        }

        layer.actorName = read_i16(record + 12);
        if (minorVersion == 0 && (record[14] != 0 || record[15] != 0)) {
            return InputControllerError::InvalidReservedData;
        }
        if (minorVersion >= 1) {
            if (record[14] > static_cast<std::uint8_t>(InputControllerActorSelector::Placed)) {
                return InputControllerError::InvalidActorSelector;
            }
            layer.actorSelector = static_cast<InputControllerActorSelector>(record[14]);
        }
        layer.offsetX = read_f32(record + 16);
        layer.offsetY = read_f32(record + 20);
        layer.offsetZ = read_f32(record + 24);
        layer.stopRadius = read_f32(record + 28);
        layer.magnitude = record[32];
        if (!valid_seek(layer)) {
            return !finite(layer.stopRadius) || !finite(layer.offsetX) || !finite(layer.offsetY) ||
                           !finite(layer.offsetZ) ?
                       InputControllerError::InvalidFloat :
                   layer.stopRadius < 0.0F ? InputControllerError::InvalidStopRadius :
                                             InputControllerError::InvalidMagnitude;
        }
        if (minorVersion == 0 || layer.actorSelector == InputControllerActorSelector::Nearest) {
            if (record[15] != 0 || !all_zero(record + 33, record + 39)) {
                return InputControllerError::InvalidUnusedData;
            }
        } else if (layer.actorSelector == InputControllerActorSelector::Process) {
            if (record[15] != 0 || !all_zero(record + 37, record + 39)) {
                return InputControllerError::InvalidUnusedData;
            }
            layer.processId = read_u32(record + 33);
            if (layer.processId == 0 ||
                layer.processId == std::numeric_limits<std::uint32_t>::max())
            {
                return InputControllerError::InvalidProcessId;
            }
        } else {
            if (!all_zero(record + 33, record + 37)) {
                return InputControllerError::InvalidUnusedData;
            }
            layer.homeRoom = std::bit_cast<std::int8_t>(record[15]);
            layer.setId = read_u16(record + 37);
            if (layer.setId == std::numeric_limits<std::uint16_t>::max()) {
                return InputControllerError::InvalidSetId;
            }
            if (!canonical_nonempty_fixed_string(record + 39, layer.placedStageName.size())) {
                return InputControllerError::InvalidStageName;
            }
            std::copy_n(record + 39, layer.placedStageName.size(), layer.placedStageName.begin());
        }
        if (layer.actorSelector == InputControllerActorSelector::Placed && minorVersion >= 1) {
            if (!all_zero(record + 47, record + kInputControllerRecordSize)) {
                return InputControllerError::InvalidUnusedData;
            }
        } else if (!all_zero(record + 39, record + kInputControllerRecordSize)) {
            return InputControllerError::InvalidUnusedData;
        }
    }

    const auto sameReplacementSurface = [](const InputControllerLayer& left,
                                            const InputControllerLayer& right) {
        const bool leftCamera = left.kind == InputControllerLayerKind::Camera;
        const bool rightCamera = right.kind == InputControllerLayerKind::Camera;
        const bool leftMain = left.kind != InputControllerLayerKind::Buttons && !leftCamera &&
                              left.kind != InputControllerLayerKind::SafetyClamp;
        const bool rightMain = right.kind != InputControllerLayerKind::Buttons && !rightCamera &&
                               right.kind != InputControllerLayerKind::SafetyClamp;
        return (leftCamera && rightCamera) || (leftMain && rightMain);
    };
    for (std::size_t left = 0; left < layerCount; ++left) {
        const InputControllerLayer& leftLayer = candidate.mLayers[left];
        if (leftLayer.kind == InputControllerLayerKind::Buttons ||
            leftLayer.kind == InputControllerLayerKind::SafetyClamp ||
            leftLayer.blend != InputControllerBlend::Replace)
        {
            continue;
        }
        for (std::size_t right = left + 1; right < layerCount; ++right) {
            const InputControllerLayer& rightLayer = candidate.mLayers[right];
            if (rightLayer.kind != InputControllerLayerKind::Buttons &&
                rightLayer.kind != InputControllerLayerKind::SafetyClamp &&
                rightLayer.blend == InputControllerBlend::Replace &&
                sameReplacementSurface(leftLayer, rightLayer) &&
                ranges_overlap(leftLayer, rightLayer))
            {
                return InputControllerError::OverlappingReplaceLayers;
            }
        }
    }
    for (std::size_t left = 0; left < layerCount; ++left) {
        const InputControllerLayer& leftLayer = candidate.mLayers[left];
        if (leftLayer.kind != InputControllerLayerKind::SafetyClamp) {
            continue;
        }
        for (std::size_t right = left + 1; right < layerCount; ++right) {
            const InputControllerLayer& rightLayer = candidate.mLayers[right];
            if (rightLayer.kind == InputControllerLayerKind::SafetyClamp &&
                ranges_overlap(leftLayer, rightLayer))
            {
                return InputControllerError::OverlappingSafetyClamps;
            }
        }
    }

    output = candidate;
    return InputControllerError::None;
}

RawPadState InputControllerProgram::evaluate(
    const std::uint32_t frame, const ControllerObservation& observation) const {
    return evaluateDetailed(frame, observation).input;
}

InputControllerEvaluation InputControllerProgram::evaluateDetailed(
    const std::uint32_t frame, const ControllerObservation& observation) const {
    InputControllerEvaluation result;
    RawPadState& output = result.input;
    if (finished(frame)) {
        return result;
    }

    StickValue replacement;
    StickValue additions;
    StickValue cameraReplacement;
    StickValue cameraAdditions;
    const InputControllerLayer* safetyClamp = nullptr;
    const std::span activeLayers = layers();
    for (std::size_t layerIndex = 0; layerIndex < activeLayers.size(); ++layerIndex) {
        const InputControllerLayer& layer = activeLayers[layerIndex];
        if (frame < layer.start || frame - layer.start >= layer.duration) {
            continue;
        }
        if (layer.kind == InputControllerLayerKind::Buttons) {
            output.buttons = static_cast<std::uint16_t>(output.buttons | layer.buttons);
            continue;
        }
        if (layer.kind == InputControllerLayerKind::SafetyClamp) {
            safetyClamp = &layer;
            continue;
        }
        if (layer.kind == InputControllerLayerKind::Camera) {
            StickValue& target = layer.blend == InputControllerBlend::Replace ?
                                     cameraReplacement :
                                     cameraAdditions;
            target.x += layer.cameraX;
            target.y += layer.cameraY;
            continue;
        }

        bool exactTargetLost = false;
        const StickValue value =
            evaluate_stick_layer(layer, frame - layer.start, observation, exactTargetLost);
        if (exactTargetLost) {
            result = {};
            result.terminalReason = InputControllerTerminalReason::TargetLost;
            result.terminalLayer = static_cast<std::uint16_t>(layerIndex);
            return result;
        }
        if (layer.blend == InputControllerBlend::Replace) {
            replacement = value;
        } else {
            additions.x += value.x;
            additions.y += value.y;
        }
    }
    output.stickX = clamp_stick(replacement.x + additions.x);
    output.stickY = clamp_stick(replacement.y + additions.y);
    output.substickX = clamp_stick(cameraReplacement.x + cameraAdditions.x);
    output.substickY = clamp_stick(cameraReplacement.y + cameraAdditions.y);
    if (safetyClamp != nullptr) {
        const auto clampLimit = [](const std::int8_t value, const std::uint8_t limit) {
            return static_cast<std::int8_t>(std::clamp<int>(value, -static_cast<int>(limit),
                static_cast<int>(limit)));
        };
        output.stickX = clampLimit(output.stickX, safetyClamp->mainLimit);
        output.stickY = clampLimit(output.stickY, safetyClamp->mainLimit);
        output.substickX = clampLimit(output.substickX, safetyClamp->substickLimit);
        output.substickY = clampLimit(output.substickY, safetyClamp->substickLimit);
    }
    return result;
}

InputControllerStepResponse InputControllerProgram::respond(
    const InputControllerStepRequest& request) const {
    InputControllerStepResponse response{
        .majorVersion = kInputControllerStepMajorVersion,
        .minorVersion = kInputControllerStepMinorVersion,
        .simulationTick = request.simulationTick,
        .inputFrame = request.inputFrame,
        .controllerFrame = request.controllerFrame,
    };
    if (request.majorVersion != kInputControllerStepMajorVersion ||
        request.minorVersion != kInputControllerStepMinorVersion)
    {
        response.error = InputControllerStepError::UnsupportedVersion;
        return response;
    }
    if (request.phase != InputControllerObservationPhase::PreInput) {
        response.error = InputControllerStepError::InvalidPhase;
        return response;
    }
    if (!validate_typed_fact_response(request.facts) ||
        request.facts.phase != TypedFactPhase::PreInput ||
        request.facts.simulationTick != request.simulationTick ||
        request.facts.tapeFrame != request.inputFrame) {
        response.error = InputControllerStepError::InvalidFacts;
        return response;
    }
    if (finished(request.controllerFrame)) {
        response.error = InputControllerStepError::InvalidFrame;
        return response;
    }
    ControllerObservation observation = request.observation;
    const auto* stage = request.facts.find(TypedFactId::StageName);
    observation.stageName = {};
    if (stage != nullptr && stage->status == TypedFactStatus::Present &&
        stage->type == TypedFactValueType::StageCode) {
        observation.stageName = stage->value.stageCode;
    }
    const auto* playerExists = request.facts.find(TypedFactId::PlayerExists);
    observation.playerPresent = playerExists != nullptr &&
                                playerExists->status == TypedFactStatus::Present &&
                                playerExists->type == TypedFactValueType::Boolean &&
                                playerExists->value.boolean;
    const auto* playerPosition = request.facts.find(TypedFactId::PlayerPosition);
    if (observation.playerPresent && playerPosition != nullptr &&
        playerPosition->status == TypedFactStatus::Present &&
        playerPosition->type == TypedFactValueType::Vec3F32) {
        observation.playerX = playerPosition->value.vec3[0];
        observation.playerY = playerPosition->value.vec3[1];
        observation.playerZ = playerPosition->value.vec3[2];
    } else {
        observation.playerPresent = false;
    }
    response.evaluation = evaluateDetailed(request.controllerFrame, observation);
    return response;
}

const char* input_controller_error_message(const InputControllerError error) {
    switch (error) {
    case InputControllerError::None:
        return "no error";
    case InputControllerError::Truncated:
        return "controller is truncated";
    case InputControllerError::BadMagic:
        return "controller has invalid magic";
    case InputControllerError::UnsupportedVersion:
        return "controller version is unsupported";
    case InputControllerError::InvalidHeaderSize:
        return "controller header size is invalid";
    case InputControllerError::InvalidRecordSize:
        return "controller record size is invalid";
    case InputControllerError::InvalidDuration:
        return "controller duration is invalid";
    case InputControllerError::TooManyLayers:
        return "controller has too many layers";
    case InputControllerError::InvalidReservedData:
        return "controller reserved data is nonzero";
    case InputControllerError::InvalidPayloadLength:
        return "controller payload length is invalid";
    case InputControllerError::TrailingData:
        return "controller has trailing data";
    case InputControllerError::InvalidLayerKind:
        return "controller layer kind is invalid";
    case InputControllerError::InvalidLayerBlend:
        return "controller layer blend is invalid";
    case InputControllerError::InvalidLayerPort:
        return "controller layer port is invalid";
    case InputControllerError::InvalidLayerRange:
        return "controller layer range is invalid";
    case InputControllerError::InvalidFloat:
        return "controller layer contains a non-finite float";
    case InputControllerError::InvalidStopRadius:
        return "controller seek stop radius is invalid";
    case InputControllerError::InvalidMagnitude:
        return "controller seek magnitude is invalid";
    case InputControllerError::InvalidButtonMask:
        return "controller button mask is empty";
    case InputControllerError::InvalidActorSelector:
        return "controller actor selector is invalid";
    case InputControllerError::InvalidProcessId:
        return "controller actor process ID is invalid";
    case InputControllerError::InvalidSetId:
        return "controller actor set ID is invalid";
    case InputControllerError::InvalidStageName:
        return "controller placed-actor stage name is invalid";
    case InputControllerError::InvalidCoordinateFrame:
        return "controller coordinate frame is invalid";
    case InputControllerError::InvalidPlaneNormal:
        return "controller plane normal has no horizontal component";
    case InputControllerError::InvalidResolvedTarget:
        return "controller resolved target identity is invalid";
    case InputControllerError::InvalidMotionControl:
        return "controller motion control is invalid";
    case InputControllerError::InvalidHeading:
        return "controller heading or angular tolerance is invalid";
    case InputControllerError::InvalidDistance:
        return "controller distance or distance tolerance is invalid";
    case InputControllerError::InvalidUnusedData:
        return "controller layer unused data is nonzero";
    case InputControllerError::OverlappingReplaceLayers:
        return "controller replace layers overlap";
    case InputControllerError::OverlappingSafetyClamps:
        return "controller safety-clamp layers overlap";
    }
    return "unknown controller error";
}

}  // namespace dusk::automation
