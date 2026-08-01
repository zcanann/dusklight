#pragma once

#include <cstdint>

namespace dusk {

enum class EponaCollisionSurface : std::uint8_t {
    None,
    NoEntry,
    HorseOnlyWall,
    ConditionalHorseWall,
};

struct EponaCollisionTint {
    bool active = false;
    EponaCollisionSurface surface = EponaCollisionSurface::None;
    std::uint8_t red = 0;
    std::uint8_t green = 0;
    std::uint8_t blue = 0;
};

// These are the same static polygon attributes consulted by horse collision:
// code0 bit 21 is an explicit no-entry surface, while wall codes 8 and 9 are
// selectively solid for the horse. Code 9 remains conditional on Epona's
// runtime special-wall check, so the view deliberately gives it its own color.
constexpr EponaCollisionTint epona_collision_tint(const bool horseNoEntry, const int wallCode,
    const bool highlightNoEntry, const bool highlightHorseWalls) noexcept {
    if (highlightNoEntry && horseNoEntry) {
        return {
            .active = true,
            .surface = EponaCollisionSurface::NoEntry,
            .red = 255,
            .green = 128,
            .blue = 0,
        };
    }

    if (highlightHorseWalls && wallCode == 8) {
        return {
            .active = true,
            .surface = EponaCollisionSurface::HorseOnlyWall,
            .red = 0,
            .green = 255,
            .blue = 255,
        };
    }

    if (highlightHorseWalls && wallCode == 9) {
        return {
            .active = true,
            .surface = EponaCollisionSurface::ConditionalHorseWall,
            .red = 255,
            .green = 255,
            .blue = 0,
        };
    }

    return {};
}

static_assert(epona_collision_tint(true, 0, true, false).surface == EponaCollisionSurface::NoEntry);
static_assert(
    epona_collision_tint(false, 8, false, true).surface == EponaCollisionSurface::HorseOnlyWall);
static_assert(epona_collision_tint(false, 9, false, true).surface ==
              EponaCollisionSurface::ConditionalHorseWall);
static_assert(!epona_collision_tint(true, 8, false, false).active);
static_assert(
    epona_collision_tint(true, 8, false, true).surface == EponaCollisionSurface::HorseOnlyWall);

}  // namespace dusk
