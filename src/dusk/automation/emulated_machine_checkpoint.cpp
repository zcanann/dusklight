#include "dusk/automation/state_checkpoint.hpp"

#include <cstring>

#include <dolphin/ar.h>
#include <dolphin/os.h>
#include <dolphin/os/OSTime.h>

namespace dusk::automation {
namespace {

bool capture_clock(void*, const std::span<std::byte> output) {
    if (output.size() != sizeof(AuroraDeterministicTimeState)) {
        return false;
    }
    AuroraDeterministicTimeState state{};
    if (!AuroraCaptureDeterministicTimeState(&state)) {
        return false;
    }
    std::memcpy(output.data(), &state, sizeof(state));
    return true;
}

bool restore_clock(void*, const std::span<const std::byte> input) {
    if (input.size() != sizeof(AuroraDeterministicTimeState)) {
        return false;
    }
    AuroraDeterministicTimeState state{};
    std::memcpy(&state, input.data(), sizeof(state));
    return AuroraRestoreDeterministicTimeState(&state) != FALSE;
}

bool capture_aram_allocator(void*, const std::span<std::byte> output) {
    if (output.size() != sizeof(AuroraARState)) {
        return false;
    }
    AuroraARState state{};
    if (!ARCaptureState(&state)) {
        return false;
    }
    std::memcpy(output.data(), &state, sizeof(state));
    return true;
}

bool restore_aram_allocator(void*, const std::span<const std::byte> input) {
    if (input.size() != sizeof(AuroraARState)) {
        return false;
    }
    AuroraARState state{};
    std::memcpy(&state, input.data(), sizeof(state));
    return ARRestoreState(&state) != FALSE;
}

}  // namespace

StateCheckpointError register_emulated_machine_checkpoint(StateCheckpoint& checkpoint) {
    void* const mem1 = AuroraGetMEM1StorageAddress();
    const std::size_t mem1Size = AuroraGetMEM1StorageSize();
    StateCheckpointError error = checkpoint.addMemoryRegion("mem1", mem1, mem1Size);
    if (error != StateCheckpointError::None) {
        return error;
    }
    void* const aram = ARGetStorageAddress();
    const std::size_t aramSize = ARGetSize();
    error = checkpoint.addMemoryRegion("aram", aram, aramSize);
    if (error != StateCheckpointError::None) {
        return error;
    }
    error = checkpoint.addComponent("aram_allocator", sizeof(AuroraARState), nullptr,
        capture_aram_allocator, restore_aram_allocator);
    if (error != StateCheckpointError::None) {
        return error;
    }
    return checkpoint.addComponent("deterministic_os_time",
        sizeof(AuroraDeterministicTimeState), nullptr, capture_clock, restore_clock);
}

}  // namespace dusk::automation
