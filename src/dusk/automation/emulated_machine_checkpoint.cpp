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
#include "JSystem/JKernel/JKRAramHeap.h"
#include "JSystem/JKernel/JKRArchive.h"
#include "JSystem/JKernel/JKRDvdFile.h"
#include "JSystem/JKernel/JKRFileLoader.h"
#include "JSystem/JKernel/JKRHeap.h"
#include "Z2AudioLib/Z2Audience.h"
#include "Z2AudioLib/Z2SoundHandles.h"
#include "d/d_meter_HIO.h"

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
static_assert(std::is_trivially_copyable_v<JUTGamePadState>);

struct JKRResourceRegistryCheckpointState {
    JKRFileLoader* currentVolume;
    JSUPtrLink* volumeHead;
    JSUPtrLink* volumeTail;
    JSUPtrLink* aramHead;
    JSUPtrLink* aramTail;
    JSUPtrLink* dvdHead;
    JSUPtrLink* dvdTail;
    u32 volumeLength;
    u32 aramLength;
    u32 dvdLength;
    u32 currentDirectoryId;
};

static_assert(std::is_trivially_copyable_v<JKRResourceRegistryCheckpointState>);

std::vector<StateCheckpointIgnoredRange> native_semantic_padding(
    const std::span<std::byte> region) {
    std::vector<StateCheckpointIgnoredRange> result;
    const auto regionBegin = reinterpret_cast<std::uintptr_t>(region.data());
    const auto regionEnd = regionBegin + region.size();
    const auto appendPadding = [&](const void* const afterField,
                                   const void* const nextField) {
        const auto begin = reinterpret_cast<std::uintptr_t>(afterField);
        const auto end = reinterpret_cast<std::uintptr_t>(nextField);
        if (regionBegin <= begin && begin < end && end <= regionEnd) {
            result.push_back({
                .offset = static_cast<std::size_t>(begin - regionBegin),
                .size = static_cast<std::size_t>(end - begin),
            });
        }
    };
    appendPadding(reinterpret_cast<const std::byte*>(&g_drawHIO.field_0x4) +
            sizeof(g_drawHIO.field_0x4),
        &g_drawHIO.mLifeTopPosX);
    appendPadding(reinterpret_cast<const std::byte*>(&g_drawHIO.mButtonATextDebug) +
            sizeof(g_drawHIO.mButtonATextDebug),
        &g_drawHIO.mButtonBFontScale);
    appendPadding(reinterpret_cast<const std::byte*>(&g_drawHIO.mMidnaIconFlashRate) +
            sizeof(g_drawHIO.mMidnaIconFlashRate),
        &g_drawHIO.field_0x3f0);
    appendPadding(reinterpret_cast<const std::byte*>(&g_drawHIO.mScrollArrowDisplayAll) +
            sizeof(g_drawHIO.mScrollArrowDisplayAll),
        &g_drawHIO.mWiiLockArrowScaleX);
    appendPadding(reinterpret_cast<const std::byte*>(&g_drawHIO.mWiiLockArrowDisplayAll) +
            sizeof(g_drawHIO.mWiiLockArrowDisplayAll),
        &g_drawHIO.mFloatingMessagePosX);
    appendPadding(reinterpret_cast<const std::byte*>(&g_drawHIO.mItemScalePercent) +
            sizeof(g_drawHIO.mItemScalePercent),
        &g_drawHIO.mTouchAreaUnselectScale[0]);
    appendPadding(reinterpret_cast<const std::byte*>(&g_drawHIO.field_0x60a) +
            sizeof(g_drawHIO.field_0x60a),
        &g_drawHIO.mLanternIconMeterPosX);
    appendPadding(reinterpret_cast<const std::byte*>(&g_drawHIO.mLanternIconMeterSize) +
            sizeof(g_drawHIO.mLanternIconMeterSize),
        &g_drawHIO.mCollectScreen);
    return result;
}

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

bool capture_jkr_resource_registries(void*, const std::span<std::byte> output) {
    if (output.size() != sizeof(JKRResourceRegistryCheckpointState)) {
        return false;
    }
    const JSUPtrListCheckpointState volumes =
        JKRFileLoader::getVolumeList().captureCheckpointState();
    const JSUPtrListCheckpointState aram = JKRAramHeap::sAramList.captureCheckpointState();
    const JSUPtrListCheckpointState dvds = JKRDvdFile::getDvdList().captureCheckpointState();
    const JKRResourceRegistryCheckpointState state{
        .currentVolume = JKRFileLoader::getCurrentVolume(),
        .volumeHead = volumes.head,
        .volumeTail = volumes.tail,
        .aramHead = aram.head,
        .aramTail = aram.tail,
        .dvdHead = dvds.head,
        .dvdTail = dvds.tail,
        .volumeLength = volumes.length,
        .aramLength = aram.length,
        .dvdLength = dvds.length,
        .currentDirectoryId = JKRArchive::getCurrentDirID(),
    };
    std::memcpy(output.data(), &state, sizeof(state));
    return true;
}

bool restore_jkr_resource_registries(void*, const std::span<const std::byte> input) {
    if (input.size() != sizeof(JKRResourceRegistryCheckpointState)) {
        return false;
    }
    JKRResourceRegistryCheckpointState state{};
    std::memcpy(&state, input.data(), sizeof(state));
    if (!JKRFileLoader::getVolumeList().restoreCheckpointState(
            {state.volumeHead, state.volumeTail, state.volumeLength}) ||
        !JKRAramHeap::sAramList.restoreCheckpointState(
            {state.aramHead, state.aramTail, state.aramLength}) ||
        !JKRDvdFile::getDvdList().restoreCheckpointState(
            {state.dvdHead, state.dvdTail, state.dvdLength}))
    {
        return false;
    }
    JKRFileLoader::setCurrentVolume(state.currentVolume);
    JKRArchive::setCurrentDirID(state.currentDirectoryId);
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
    // Value initialization does not guarantee deterministic padding bytes.
    // Canonicalize the complete trivially-copyable representation before
    // assigning every logical field so checkpoint digests never learn stack
    // residue as gameplay state.
    JUTGamePadState state;
    std::memset(&state, 0, sizeof(state));
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
        const std::vector<StateCheckpointIgnoredRange> ignored =
            native_semantic_padding(region);
        error = checkpoint.addMemoryRegion(name, region.data(), region.size(), ignored);
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
    error = checkpoint.addComponent("jkr_resource_registries",
        sizeof(JKRResourceRegistryCheckpointState), nullptr, capture_jkr_resource_registries,
        restore_jkr_resource_registries);
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
