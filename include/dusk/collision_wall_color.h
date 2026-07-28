#pragma once

#include <cstdint>

namespace dusk {

struct CollisionWallTint {
    bool active = false;
    std::uint8_t red = 0;
    std::uint8_t green = 0;
    std::uint8_t blue = 0;
};

// Collision plane normals are normalized. For an upward-facing surface,
// normal.y is sin(90 degrees - surface angle), so sin(0.5 degrees) bounds the
// requested 89.5-to-90-degree band without an inverse trig call per triangle.
inline constexpr float kVerticalWallNormalEpsilon = 0.00001f;
inline constexpr float kNearVerticalWallNormalYMaximum = 0.008726535f;

constexpr std::uint8_t collision_wall_color_lerp(
    const std::uint8_t from, const std::uint8_t to, const float amount) noexcept {
    return static_cast<std::uint8_t>(static_cast<float>(from) +
                                     (static_cast<float>(to) - static_cast<float>(from)) * amount +
                                     0.5f);
}

constexpr CollisionWallTint collision_wall_tint(const float normalY) noexcept {
    if (normalY >= -kVerticalWallNormalEpsilon && normalY <= kVerticalWallNormalEpsilon) {
        return {
            .active = true,
            .red = 128,
            .green = 0,
            .blue = 128,
        };
    }

    if (!(normalY > kVerticalWallNormalEpsilon && normalY <= kNearVerticalWallNormalYMaximum)) {
        return {};
    }

    // Exact vertical is a separate purple class. The first nonvertical value
    // starts at magenta, then blends to pink at exactly 89.5 degrees.
    const float amount = (normalY - kVerticalWallNormalEpsilon) /
                         (kNearVerticalWallNormalYMaximum - kVerticalWallNormalEpsilon);
    return {
        .active = true,
        .red = collision_wall_color_lerp(255, 255, amount),
        .green = collision_wall_color_lerp(0, 192, amount),
        .blue = collision_wall_color_lerp(255, 203, amount),
    };
}

static_assert(collision_wall_tint(0.0f).active);
static_assert(collision_wall_tint(0.0f).red == 128);
static_assert(collision_wall_tint(0.0f).blue == 128);
static_assert(collision_wall_tint(kVerticalWallNormalEpsilon * 2.0f).red == 255);
static_assert(collision_wall_tint(kVerticalWallNormalEpsilon * 2.0f).green == 0);
static_assert(collision_wall_tint(kVerticalWallNormalEpsilon * 2.0f).blue == 255);
static_assert(collision_wall_tint(kNearVerticalWallNormalYMaximum).green == 192);
static_assert(collision_wall_tint(kNearVerticalWallNormalYMaximum).blue == 203);
static_assert(!collision_wall_tint(-0.001f).active);
static_assert(!collision_wall_tint(kNearVerticalWallNormalYMaximum + 0.001f).active);

}  // namespace dusk
