#include "dusk/automation/state_checkpoint.hpp"

#include <array>
#include <cassert>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <iostream>

using namespace dusk::automation;

namespace {

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
    assert(checkpoint.addMemoryRegion("memory", memory.data(), memory.size()) ==
           StateCheckpointError::None);
    assert(checkpoint.addComponent("component", sizeof(component.value), &component,
               capture_component, restore_component) == StateCheckpointError::None);
    assert(checkpoint.entryCount() == 2);
    assert(checkpoint.byteCount() == memory.size() + sizeof(component.value));

    StateCheckpointImage image;
    assert(checkpoint.capture(image) == StateCheckpointError::None);
    assert(!image.digest.empty());

    memory.fill(std::byte{0xff});
    component.value = 9;
    assert(checkpoint.restore(image) == StateCheckpointError::None);
    assert(memory[3] == std::byte{0x7a});
    assert(memory[4] == std::byte{0});
    assert(component.value == 0x12345678);

    image.entries[0].bytes[0] = std::byte{1};
    assert(checkpoint.restore(image) == StateCheckpointError::DigestMismatch);
}

void test_registration_rejections() {
    std::array<std::byte, 16> memory{};
    Component component{};
    StateCheckpoint checkpoint;
    assert(checkpoint.addMemoryRegion("", memory.data(), memory.size()) ==
           StateCheckpointError::InvalidName);
    assert(checkpoint.addMemoryRegion("zero", memory.data(), 0) ==
           StateCheckpointError::EmptyState);
    assert(checkpoint.addMemoryRegion("null", nullptr, 1) ==
           StateCheckpointError::NullAddress);
    assert(checkpoint.addMemoryRegion("a", memory.data(), 8) == StateCheckpointError::None);
    assert(checkpoint.addMemoryRegion("b", memory.data() + 7, 4) ==
           StateCheckpointError::OverlappingRegion);
    assert(checkpoint.addComponent("a", sizeof(component.value), &component, capture_component,
               restore_component) == StateCheckpointError::DuplicateName);
    assert(checkpoint.addComponent("component", sizeof(component.value), &component, nullptr,
               restore_component) == StateCheckpointError::MissingCallback);
}

void test_component_fail_closed() {
    Component component{.value = 42, .rejectCapture = true};
    StateCheckpoint checkpoint;
    assert(checkpoint.addComponent("component", sizeof(component.value), &component,
               capture_component, restore_component) == StateCheckpointError::None);
    StateCheckpointImage image;
    assert(checkpoint.capture(image) == StateCheckpointError::CaptureFailed);

    component.rejectCapture = false;
    assert(checkpoint.capture(image) == StateCheckpointError::None);
    component.rejectRestore = true;
    assert(checkpoint.restore(image) == StateCheckpointError::RestoreFailed);

    StateCheckpoint other;
    std::array<std::byte, 4> bytes{};
    assert(other.addMemoryRegion("other", bytes.data(), bytes.size()) ==
           StateCheckpointError::None);
    assert(other.restore(image) == StateCheckpointError::ManifestMismatch);
}

}  // namespace

int main() {
    test_round_trip_and_integrity();
    test_registration_rejections();
    test_component_fail_closed();
    std::cout << "state checkpoint tests passed\n";
    return 0;
}
