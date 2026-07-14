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

float read_f32(const std::uint8_t* input) {
    return std::bit_cast<float>(read_u32(input));
}

bool all_zero(const std::uint8_t* begin, const std::uint8_t* end) {
    return std::all_of(begin, end, [](const std::uint8_t value) { return value == 0; });
}

bool finite(const float value) {
    return std::isfinite(value);
}

bool valid_seek(const InputControllerLayer& layer) {
    return finite(layer.offsetX) && finite(layer.offsetY) && finite(layer.offsetZ) &&
           finite(layer.stopRadius) && layer.stopRadius >= 0.0F && layer.magnitude >= 1 &&
           layer.magnitude <= 127;
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
        .x = rounded_stick_component(std::sin(relativeAngle), layer.magnitude),
        .y = rounded_stick_component(std::cos(relativeAngle), layer.magnitude),
    };
}

StickValue evaluate_stick_layer(const InputControllerLayer& layer, const std::uint32_t localFrame,
    const ControllerObservation& observation) {
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
            return {};
        }
        return seek(layer, observation, selected->x + layer.offsetX, selected->z + layer.offsetZ);
    }
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
    if (read_u16(bytes.data() + 8) != kInputControllerMajorVersion ||
        read_u16(bytes.data() + 10) != kInputControllerMinorVersion)
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
            record[0] > static_cast<std::uint8_t>(InputControllerLayerKind::Buttons))
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

        layer.actorName = read_i16(record + 12);
        if (read_u16(record + 14) != 0) {
            return InputControllerError::InvalidReservedData;
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
        if (!all_zero(record + 33, record + kInputControllerRecordSize)) {
            return InputControllerError::InvalidUnusedData;
        }
    }

    for (std::size_t left = 0; left < layerCount; ++left) {
        const InputControllerLayer& leftLayer = candidate.mLayers[left];
        if (leftLayer.kind == InputControllerLayerKind::Buttons ||
            leftLayer.blend != InputControllerBlend::Replace)
        {
            continue;
        }
        for (std::size_t right = left + 1; right < layerCount; ++right) {
            const InputControllerLayer& rightLayer = candidate.mLayers[right];
            if (rightLayer.kind != InputControllerLayerKind::Buttons &&
                rightLayer.blend == InputControllerBlend::Replace &&
                ranges_overlap(leftLayer, rightLayer))
            {
                return InputControllerError::OverlappingReplaceLayers;
            }
        }
    }

    output = candidate;
    return InputControllerError::None;
}

RawPadState InputControllerProgram::evaluate(
    const std::uint32_t frame, const ControllerObservation& observation) const {
    RawPadState output;
    if (finished(frame)) {
        return output;
    }

    StickValue replacement;
    StickValue additions;
    for (const InputControllerLayer& layer : layers()) {
        if (frame < layer.start || frame - layer.start >= layer.duration) {
            continue;
        }
        if (layer.kind == InputControllerLayerKind::Buttons) {
            output.buttons = static_cast<std::uint16_t>(output.buttons | layer.buttons);
            continue;
        }

        const StickValue value = evaluate_stick_layer(layer, frame - layer.start, observation);
        if (layer.blend == InputControllerBlend::Replace) {
            replacement = value;
        } else {
            additions.x += value.x;
            additions.y += value.y;
        }
    }
    output.stickX = clamp_stick(replacement.x + additions.x);
    output.stickY = clamp_stick(replacement.y + additions.y);
    return output;
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
    case InputControllerError::InvalidUnusedData:
        return "controller layer unused data is nonzero";
    case InputControllerError::OverlappingReplaceLayers:
        return "controller replace layers overlap";
    }
    return "unknown controller error";
}

}  // namespace dusk::automation
