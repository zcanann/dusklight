#ifndef DUSK_AUTOMATION_STATE_CHECKPOINT_HPP
#define DUSK_AUTOMATION_STATE_CHECKPOINT_HPP

#include <cstddef>
#include <cstdint>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace dusk::automation {

enum class StateCheckpointEntryKind : std::uint8_t {
    MemoryRegion = 1,
    Component = 2,
};

enum class StateCheckpointError {
    None,
    InvalidName,
    EmptyState,
    NullAddress,
    AddressOverflow,
    DuplicateName,
    OverlappingRegion,
    MissingCallback,
    CaptureFailed,
    RestoreFailed,
    ManifestMismatch,
    DigestMismatch,
    AllocationFailed,
};

struct StateCheckpointImageEntry {
    std::string name;
    StateCheckpointEntryKind kind = StateCheckpointEntryKind::MemoryRegion;
    std::vector<std::byte> bytes;
};

struct StateCheckpointImage {
    std::vector<StateCheckpointImageEntry> entries;
    std::string digest;
};

using StateCheckpointCaptureCallback = bool (*)(void*, std::span<std::byte>);
using StateCheckpointRestoreCallback = bool (*)(void*, std::span<const std::byte>);

/**
 * An explicit same-process state snapshot.
 *
 * This class deliberately has no process-memory discovery. Every byte must be
 * registered as either a direct region or a typed component. That makes an
 * incomplete checkpoint visible in the inventory instead of silently relying
 * on whichever writable pages happened to be copied.
 */
class StateCheckpoint {
public:
    StateCheckpointError addMemoryRegion(
        std::string_view name, void* address, std::size_t size);
    StateCheckpointError addComponent(std::string_view name, std::size_t size, void* context,
        StateCheckpointCaptureCallback capture, StateCheckpointRestoreCallback restore);

    [[nodiscard]] StateCheckpointError capture(StateCheckpointImage& image) const;
    [[nodiscard]] StateCheckpointError restore(const StateCheckpointImage& image) const;
    [[nodiscard]] std::size_t entryCount() const { return mEntries.size(); }
    [[nodiscard]] std::size_t byteCount() const;

private:
    struct Entry {
        std::string name;
        StateCheckpointEntryKind kind = StateCheckpointEntryKind::MemoryRegion;
        std::byte* address = nullptr;
        std::size_t size = 0;
        void* context = nullptr;
        StateCheckpointCaptureCallback capture = nullptr;
        StateCheckpointRestoreCallback restore = nullptr;
    };

    [[nodiscard]] StateCheckpointError validateNameAndSize(
        std::string_view name, std::size_t size) const;

    std::vector<Entry> mEntries;
};

[[nodiscard]] const char* state_checkpoint_error_message(StateCheckpointError error);

/** Registers MEM1, ARAM, their allocator state, and deterministic OS time. */
[[nodiscard]] StateCheckpointError register_emulated_machine_checkpoint(
    StateCheckpoint& checkpoint);

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_STATE_CHECKPOINT_HPP
