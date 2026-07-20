#include "dusk/automation/state_checkpoint.hpp"
#include "dusk/automation/native_state_region.hpp"
#include "dusk/automation/jut_gamepad_state.hpp"
#include "dusk/automation/vi_state.hpp"

#include <cstring>
#include <cstdio>

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

bool capture_vi(void*, const std::span<std::byte> output) {
    if (output.size() != sizeof(VIState)) {
        return false;
    }
    VIState state{};
    if (!capture_vi_state(state)) {
        return false;
    }
    std::memcpy(output.data(), &state, sizeof(state));
    return true;
}

bool restore_vi(void*, const std::span<const std::byte> input) {
    if (input.size() != sizeof(VIState)) {
        return false;
    }
    VIState state{};
    std::memcpy(&state, input.data(), sizeof(state));
    return restore_vi_state(state);
}

bool capture_jut_gamepad(void*, const std::span<std::byte> output) {
    if (output.size() != sizeof(JUTGamePadState)) {
        return false;
    }
    JUTGamePadState state{};
    if (!capture_jut_gamepad_state(state)) {
        return false;
    }
    std::memcpy(output.data(), &state, sizeof(state));
    return true;
}

bool restore_jut_gamepad(void*, const std::span<const std::byte> input) {
    if (input.size() != sizeof(JUTGamePadState)) {
        return false;
    }
    JUTGamePadState state{};
    std::memcpy(&state, input.data(), sizeof(state));
    return restore_jut_gamepad_state(state);
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
    const NativeGameStateRegions nativeState = native_game_state_regions();
    if (nativeState.count == 0) {
        return StateCheckpointError::EmptyState;
    }
    for (std::size_t index = 0; index < nativeState.count; ++index) {
        char name[32]{};
        std::snprintf(name, sizeof(name), "native_game_state_%zu", index);
        const std::span<std::byte> region = nativeState.items[index];
        error = checkpoint.addMemoryRegion(name, region.data(), region.size());
        if (error != StateCheckpointError::None) {
            return error;
        }
    }
    error = checkpoint.addComponent("aram_allocator", sizeof(AuroraARState), nullptr,
        capture_aram_allocator, restore_aram_allocator);
    if (error != StateCheckpointError::None) {
        return error;
    }
    error = checkpoint.addComponent("deterministic_os_time",
        sizeof(AuroraDeterministicTimeState), nullptr, capture_clock, restore_clock);
    if (error != StateCheckpointError::None) {
        return error;
    }
    error = checkpoint.addComponent("emulated_vi", sizeof(VIState), nullptr,
        capture_vi, restore_vi);
    if (error != StateCheckpointError::None) {
        return error;
    }
    return checkpoint.addComponent("jut_gamepad", sizeof(JUTGamePadState), nullptr,
        capture_jut_gamepad, restore_jut_gamepad);
}

}  // namespace dusk::automation
