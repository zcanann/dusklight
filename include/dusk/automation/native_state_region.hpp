#ifndef DUSK_AUTOMATION_NATIVE_STATE_REGION_HPP
#define DUSK_AUTOMATION_NATIVE_STATE_REGION_HPP

#include <cstddef>
#include <array>
#include <span>

namespace dusk::automation {

struct NativeGameStateRegions {
    static constexpr std::size_t Capacity = 8;
    std::array<std::span<std::byte>, Capacity> items{};
    std::size_t count = 0;
};

/** Writable native globals isolated by the build from host/automation state. */
[[nodiscard]] NativeGameStateRegions native_game_state_regions();

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_NATIVE_STATE_REGION_HPP
