#include "dusk/automation/state_checkpoint.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iostream>

using namespace dusk::automation;

namespace {

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "state_checkpoint_test.cpp:" << line
                  << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

struct Component {
    std::uint32_t value = 0;
    bool rejectCapture = false;
    bool rejectRestore = false;
};

bool capture_component(void* context, const std::span<std::byte> output) {
    auto& component = *static_cast<Component*>(context);
    if (component.rejectCapture || output.size() != sizeof(component.value)) {
        return false;
    }
    std::memcpy(output.data(), &component.value, sizeof(component.value));
    return true;
}

bool restore_component(void* context, const std::span<const std::byte> input) {
    auto& component = *static_cast<Component*>(context);
    if (component.rejectRestore || input.size() != sizeof(component.value)) {
        return false;
    }
    std::memcpy(&component.value, input.data(), sizeof(component.value));
    return true;
}

void test_round_trip_and_integrity() {
    std::array<std::byte, 16> memory{};
    memory[3] = std::byte{0x7a};
    Component component{.value = 0x12345678};

    StateCheckpoint checkpoint;
    REQUIRE(checkpoint.addMemoryRegion("memory", memory.data(), memory.size()) ==
           StateCheckpointError::None);
    REQUIRE(checkpoint.addComponent("component", sizeof(component.value), &component,
               capture_component, restore_component) == StateCheckpointError::None);
    REQUIRE(checkpoint.entryCount() == 2);
    REQUIRE(checkpoint.byteCount() == memory.size() + sizeof(component.value));

    StateCheckpointImage image;
    REQUIRE(checkpoint.capture(image) == StateCheckpointError::None);
    REQUIRE(!image.digest.empty());
    std::string liveDigest;
    std::vector<StateCheckpointEntryDigest> entryDigests;
    REQUIRE(checkpoint.currentDigest(liveDigest, &entryDigests) == StateCheckpointError::None);
    REQUIRE(liveDigest == image.digest);
    REQUIRE(entryDigests.size() == 2);
    REQUIRE(entryDigests[0].name == "memory");
    REQUIRE(!entryDigests[0].digest.empty());

    memory.fill(std::byte{0xff});
    component.value = 9;
    REQUIRE(checkpoint.restore(image) == StateCheckpointError::None);
    REQUIRE(memory[3] == std::byte{0x7a});
    REQUIRE(memory[4] == std::byte{0});
    REQUIRE(component.value == 0x12345678);

    memory.fill(std::byte{0xee});
    component.value = 7;
    REQUIRE(checkpoint.restoreTrusted(image) == StateCheckpointError::None);
    REQUIRE(memory[3] == std::byte{0x7a});
    REQUIRE(component.value == 0x12345678);

    image.entries[0].bytes[0] = std::byte{1};
    REQUIRE(checkpoint.restore(image) == StateCheckpointError::DigestMismatch);
}

void test_registration_rejections() {
    std::array<std::byte, 16> memory{};
    Component component{};
    StateCheckpoint checkpoint;
    REQUIRE(checkpoint.addMemoryRegion("", memory.data(), memory.size()) ==
           StateCheckpointError::InvalidName);
    REQUIRE(checkpoint.addMemoryRegion("zero", memory.data(), 0) ==
           StateCheckpointError::EmptyState);
    REQUIRE(checkpoint.addMemoryRegion("null", nullptr, 1) ==
           StateCheckpointError::NullAddress);
    REQUIRE(checkpoint.addMemoryRegion("a", memory.data(), 8) == StateCheckpointError::None);
    REQUIRE(checkpoint.addMemoryRegion("b", memory.data() + 7, 4) ==
           StateCheckpointError::OverlappingRegion);
    REQUIRE(checkpoint.addComponent("a", sizeof(component.value), &component, capture_component,
               restore_component) == StateCheckpointError::DuplicateName);
    REQUIRE(checkpoint.addComponent("component", sizeof(component.value), &component, nullptr,
               restore_component) == StateCheckpointError::MissingCallback);
}

void test_component_fail_closed() {
    Component component{.value = 42, .rejectCapture = true};
    StateCheckpoint checkpoint;
    REQUIRE(checkpoint.addComponent("component", sizeof(component.value), &component,
               capture_component, restore_component) == StateCheckpointError::None);
    StateCheckpointImage image;
    REQUIRE(checkpoint.capture(image) == StateCheckpointError::CaptureFailed);

    component.rejectCapture = false;
    REQUIRE(checkpoint.capture(image) == StateCheckpointError::None);
    component.rejectRestore = true;
    REQUIRE(checkpoint.restore(image) == StateCheckpointError::RestoreFailed);

    StateCheckpoint other;
    std::array<std::byte, 4> bytes{};
    REQUIRE(other.addMemoryRegion("other", bytes.data(), bytes.size()) ==
           StateCheckpointError::None);
    REQUIRE(other.restore(image) == StateCheckpointError::ManifestMismatch);
}

void test_semantic_padding_is_explicit_and_raw_integrity_stays_exact() {
    std::array<std::byte, 16> memory{};
    memory[3] = std::byte{0x11};
    memory[5] = std::byte{0x22};
    const std::array ignored{
        StateCheckpointIgnoredRange{.offset = 4, .size = 3},
    };
    StateCheckpoint checkpoint;
    REQUIRE(checkpoint.addMemoryRegion("memory", memory.data(), memory.size(), ignored) ==
            StateCheckpointError::None);

    StateCheckpointImage image;
    REQUIRE(checkpoint.capture(image) == StateCheckpointError::None);
    std::string rawBefore;
    std::string semanticBefore;
    REQUIRE(checkpoint.currentDigest(rawBefore) == StateCheckpointError::None);
    REQUIRE(checkpoint.currentSemanticDigest(semanticBefore) == StateCheckpointError::None);
    REQUIRE(rawBefore == image.digest);
    REQUIRE(rawBefore != semanticBefore);

    memory[5] = std::byte{0x99};
    std::string rawAfterPadding;
    std::string semanticAfterPadding;
    REQUIRE(checkpoint.currentDigest(rawAfterPadding) == StateCheckpointError::None);
    REQUIRE(checkpoint.currentSemanticDigest(semanticAfterPadding) ==
            StateCheckpointError::None);
    REQUIRE(rawAfterPadding != rawBefore);
    REQUIRE(semanticAfterPadding == semanticBefore);

    memory[3] = std::byte{0x44};
    std::string semanticAfterState;
    REQUIRE(checkpoint.currentSemanticDigest(semanticAfterState) ==
            StateCheckpointError::None);
    REQUIRE(semanticAfterState != semanticBefore);
    REQUIRE(checkpoint.restore(image) == StateCheckpointError::None);
    REQUIRE(memory[3] == std::byte{0x11});
    REQUIRE(memory[5] == std::byte{0x22});

    StateCheckpoint invalid;
    const std::array zeroRange{
        StateCheckpointIgnoredRange{.offset = 2, .size = 0},
    };
    REQUIRE(invalid.addMemoryRegion("zero", memory.data(), memory.size(), zeroRange) ==
            StateCheckpointError::InvalidIgnoredRange);
    const std::array overlapping{
        StateCheckpointIgnoredRange{.offset = 2, .size = 4},
        StateCheckpointIgnoredRange{.offset = 5, .size = 2},
    };
    REQUIRE(invalid.addMemoryRegion("overlap", memory.data(), memory.size(), overlapping) ==
            StateCheckpointError::InvalidIgnoredRange);
}

}  // namespace

int main() {
    test_round_trip_and_integrity();
    test_registration_rejections();
    test_component_fail_closed();
    test_semantic_padding_is_explicit_and_raw_integrity_stays_exact();
    std::cout << "state checkpoint tests passed\n";
    return 0;
}
