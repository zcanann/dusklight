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
    InvalidIgnoredRange,
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

struct StateCheckpointEntryDigest {
    std::string name;
    StateCheckpointEntryKind kind = StateCheckpointEntryKind::MemoryRegion;
    std::size_t size = 0;
    std::string digest;
};

/**
 * A byte range that is preserved by checkpoint capture/restore and raw
 * integrity hashing, but canonicalized for gameplay-state comparisons.
 *
 * This is intentionally explicit. It is for proven host-ABI padding only,
 * never for state that merely appears unimportant in one scenario.
 */
struct StateCheckpointIgnoredRange {
    std::size_t offset = 0;
    std::size_t size = 0;
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
        std::string_view name, void* address, std::size_t size,
        std::span<const StateCheckpointIgnoredRange> semanticIgnoredRanges = {});
    StateCheckpointError addComponent(std::string_view name, std::size_t size, void* context,
        StateCheckpointCaptureCallback capture, StateCheckpointRestoreCallback restore);

    [[nodiscard]] StateCheckpointError capture(StateCheckpointImage& image) const;
    [[nodiscard]] StateCheckpointError restore(const StateCheckpointImage& image) const;
    /**
     * Restores a previously validated image without rehashing its bytes.
     *
     * The caller must retain the image privately and keep it immutable after
     * capture or a successful checked restore. The manifest is still checked.
     */
    [[nodiscard]] StateCheckpointError restoreTrusted(
        const StateCheckpointImage& image) const;
    /** Hashes the registered live state without copying direct memory regions. */
    [[nodiscard]] StateCheckpointError currentDigest(std::string& digest,
        std::vector<StateCheckpointEntryDigest>* entryDigests = nullptr) const;
    /**
     * Hashes live state while canonicalizing only explicitly registered
     * host-ABI padding. Raw checkpoint integrity remains byte exact.
     */
    [[nodiscard]] StateCheckpointError currentSemanticDigest(std::string& digest,
        std::vector<StateCheckpointEntryDigest>* entryDigests = nullptr) const;
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
        std::vector<StateCheckpointIgnoredRange> semanticIgnoredRanges;
    };

    [[nodiscard]] StateCheckpointError validateNameAndSize(
        std::string_view name, std::size_t size) const;
    [[nodiscard]] StateCheckpointError restoreImpl(
        const StateCheckpointImage& image, bool validateDigest) const;
    [[nodiscard]] StateCheckpointError currentDigestImpl(std::string& digest,
        std::vector<StateCheckpointEntryDigest>* entryDigests, bool semantic) const;

    std::vector<Entry> mEntries;
};

[[nodiscard]] const char* state_checkpoint_error_message(StateCheckpointError error);

/** Registers MEM1, ARAM, their allocator state, and deterministic OS time. */
[[nodiscard]] StateCheckpointError register_emulated_machine_checkpoint(
    StateCheckpoint& checkpoint);

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_STATE_CHECKPOINT_HPP
