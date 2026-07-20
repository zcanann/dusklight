#include "dusk/automation/native_state_region.hpp"
#include "dusk/automation/state_checkpoint.hpp"

#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <string>

extern std::uint64_t gNativeCheckpointFixture;
extern std::uint64_t gNativeCheckpointZeroFixture;

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "native_state_region_test.cpp:" << line
                  << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

bool contains(const std::span<std::byte> region, const void* address, const std::size_t size) {
    const auto value = reinterpret_cast<std::uintptr_t>(address);
    const auto begin = reinterpret_cast<std::uintptr_t>(region.data());
    return value >= begin && value + size <= begin + region.size();
}

bool contains(const dusk::automation::NativeGameStateRegions& regions, const void* address,
    const std::size_t size) {
    for (std::size_t index = 0; index < regions.count; ++index) {
        if (contains(regions.items[index], address, size)) return true;
    }
    return false;
}

int main() {
    const auto regions = dusk::automation::native_game_state_regions();
    REQUIRE(regions.count >= 2);
    REQUIRE(contains(regions, &gNativeCheckpointFixture, sizeof(gNativeCheckpointFixture)));
    REQUIRE(contains(regions, &gNativeCheckpointZeroFixture,
        sizeof(gNativeCheckpointZeroFixture)));

    dusk::automation::StateCheckpoint checkpoint;
    std::size_t totalBytes = 0;
    for (std::size_t index = 0; index < regions.count; ++index) {
        const std::string name = "native_" + std::to_string(index);
        REQUIRE(checkpoint.addMemoryRegion(name, regions.items[index].data(),
                    regions.items[index].size()) ==
                dusk::automation::StateCheckpointError::None);
        totalBytes += regions.items[index].size();
    }
    dusk::automation::StateCheckpointImage image;
    REQUIRE(checkpoint.capture(image) == dusk::automation::StateCheckpointError::None);
    gNativeCheckpointFixture = 0;
    gNativeCheckpointZeroFixture = 99;
    REQUIRE(checkpoint.restore(image) == dusk::automation::StateCheckpointError::None);
    REQUIRE(gNativeCheckpointFixture == 0x123456789abcdef0ULL);
    REQUIRE(gNativeCheckpointZeroFixture == 0);
    std::cout << "native state region tests passed (" << totalBytes << " bytes)\n";
    return 0;
}
