#include "dusk/automation/state_checkpoint.hpp"
#include "dusk/automation/native_state_region.hpp"
#include "dusk/automation/jut_gamepad_state.hpp"
#include "dusk/automation/vi_state.hpp"

#include "JSystem/JAudio2/JAISe.h"
#include "JSystem/JAudio2/JAISeq.h"
#include "JSystem/JAudio2/JAISoundChild.h"
#include "JSystem/JAudio2/JAIStream.h"
#include "JSystem/JAudio2/JASChannel.h"
#include "JSystem/JAudio2/JASTrack.h"
#include "JSystem/JFramework/JFWDisplay.h"
#include "JSystem/JKernel/JKRHeap.h"
#include "Z2AudioLib/Z2Audience.h"
#include "Z2AudioLib/Z2SoundHandles.h"

#include <cstring>
#include <cstdio>
#include <type_traits>

#include <dolphin/ar.h>
#include <dolphin/gx/GXAurora.h>
#include <dolphin/os.h>
#include <dolphin/os/OSTime.h>

namespace dusk::automation {
namespace {

static_assert(std::is_trivially_copyable_v<JASGenericMemPoolState>);
static_assert(std::is_trivially_copyable_v<JFWDisplayCheckpointState>);

template <typename T>
bool capture_audio_pool(void*, const std::span<std::byte> output) {
    if (output.size() != sizeof(JASGenericMemPoolState)) {
        return false;
    }
    const JASGenericMemPoolState state = T::captureCheckpointState();
    std::memcpy(output.data(), &state, sizeof(state));
    return true;
}

template <typename T>
bool restore_audio_pool(void*, const std::span<const std::byte> input) {
    if (input.size() != sizeof(JASGenericMemPoolState)) {
        return false;
    }
    JASGenericMemPoolState state{};
    std::memcpy(&state, input.data(), sizeof(state));
    T::restoreCheckpointState(state);
    return true;
}

template <typename T>
StateCheckpointError add_audio_pool(StateCheckpoint& checkpoint, const char* const name) {
    return checkpoint.addComponent(name, sizeof(JASGenericMemPoolState), nullptr,
        capture_audio_pool<T>, restore_audio_pool<T>);
}

bool capture_jfw_display(void*, const std::span<std::byte> output) {
    if (output.size() != sizeof(JFWDisplayCheckpointState)) {
        return false;
    }
    const JFWDisplayCheckpointState state = JFWDisplay::captureCheckpointState();
    std::memcpy(output.data(), &state, sizeof(state));
    return true;
}

bool restore_jfw_display(void*, const std::span<const std::byte> input) {
    if (input.size() != sizeof(JFWDisplayCheckpointState)) {
        return false;
    }
    JFWDisplayCheckpointState state{};
    std::memcpy(&state, input.data(), sizeof(state));
    JFWDisplay::restoreCheckpointState(state);
    return true;
}

struct JKRHeapSelectionState {
    JKRHeap* currentHeap = nullptr;
};

bool capture_jkr_heap_selection(void*, const std::span<std::byte> output) {
    if (output.size() != sizeof(JKRHeapSelectionState)) {
        return false;
    }
    const JKRHeapSelectionState state{.currentHeap = JKRHeap::getCurrentHeap()};
    std::memcpy(output.data(), &state, sizeof(state));
    return true;
}

bool restore_jkr_heap_selection(void*, const std::span<const std::byte> input) {
    if (input.size() != sizeof(JKRHeapSelectionState)) {
        return false;
    }
    JKRHeapSelectionState state{};
    std::memcpy(&state, input.data(), sizeof(state));
    JKRHeap::setCurrentHeap(state.currentHeap);
    return true;
}

bool capture_gx_identity(void*, const std::span<std::byte> output) {
    if (output.size() != sizeof(AuroraGXObjectIdentityState)) {
        return false;
    }
    AuroraGXObjectIdentityState state{};
    if (!AuroraCaptureGXObjectIdentityState(&state)) {
        return false;
    }
    std::memcpy(output.data(), &state, sizeof(state));
    return true;
}

bool restore_gx_identity(void*, const std::span<const std::byte> input) {
    if (input.size() != sizeof(AuroraGXObjectIdentityState)) {
        return false;
    }
    AuroraGXObjectIdentityState state{};
    std::memcpy(&state, input.data(), sizeof(state));
    return AuroraRestoreGXObjectIdentityState(&state) != FALSE;
}

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
    error = checkpoint.addComponent("gx_object_identity",
        sizeof(AuroraGXObjectIdentityState), nullptr,
        capture_gx_identity, restore_gx_identity);
    if (error != StateCheckpointError::None) {
        return error;
    }
    error = checkpoint.addComponent("jkr_current_heap", sizeof(JKRHeapSelectionState), nullptr,
        capture_jkr_heap_selection, restore_jkr_heap_selection);
    if (error != StateCheckpointError::None) {
        return error;
    }
    error = checkpoint.addComponent("jfw_display", sizeof(JFWDisplayCheckpointState), nullptr,
        capture_jfw_display, restore_jfw_display);
    if (error != StateCheckpointError::None) {
        return error;
    }
    error = add_audio_pool<Z2Audible>(checkpoint, "audio_pool_z2_audible");
    if (error != StateCheckpointError::None) return error;
    error = add_audio_pool<Z2SoundHandlePool>(checkpoint, "audio_pool_z2_sound_handle");
    if (error != StateCheckpointError::None) return error;
    error = add_audio_pool<JAISe>(checkpoint, "audio_pool_jai_se");
    if (error != StateCheckpointError::None) return error;
    error = add_audio_pool<JAISeq>(checkpoint, "audio_pool_jai_seq");
    if (error != StateCheckpointError::None) return error;
    error = add_audio_pool<JAIStream>(checkpoint, "audio_pool_jai_stream");
    if (error != StateCheckpointError::None) return error;
    error = add_audio_pool<JAISoundChild>(checkpoint, "audio_pool_jai_sound_child");
    if (error != StateCheckpointError::None) return error;
    error = add_audio_pool<JASTrack>(checkpoint, "audio_pool_jas_track");
    if (error != StateCheckpointError::None) return error;
    error = add_audio_pool<JASTrack::TChannelMgr>(checkpoint, "audio_pool_jas_channel_mgr");
    if (error != StateCheckpointError::None) return error;
    error = add_audio_pool<JASChannel>(checkpoint, "audio_pool_jas_channel");
    if (error != StateCheckpointError::None) return error;
    return checkpoint.addComponent("jut_gamepad", sizeof(JUTGamePadState), nullptr,
        capture_jut_gamepad, restore_jut_gamepad);
}

}  // namespace dusk::automation
