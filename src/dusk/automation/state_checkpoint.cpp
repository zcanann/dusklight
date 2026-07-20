#include "dusk/automation/state_checkpoint.hpp"

#include <algorithm>
#include <array>
#include <cstring>
#include <limits>
#include <new>

#include <xxhash.h>

namespace dusk::automation {
namespace {

void hash_u64(XXH3_state_t* state, const std::uint64_t value) {
    std::array<std::uint8_t, 8> bytes{};
    for (std::size_t index = 0; index < bytes.size(); ++index) {
        bytes[index] = static_cast<std::uint8_t>(value >> (index * 8));
    }
    XXH3_128bits_update(state, bytes.data(), bytes.size());
}

std::string checkpoint_digest(const StateCheckpointImage& image) {
    XXH3_state_t* state = XXH3_createState();
    if (state == nullptr) {
        return {};
    }
    XXH3_128bits_reset(state);
    constexpr std::string_view Domain = "dusklight-state-checkpoint/v1";
    XXH3_128bits_update(state, Domain.data(), Domain.size());
    hash_u64(state, image.entries.size());
    for (const StateCheckpointImageEntry& entry : image.entries) {
        const std::uint8_t kind = static_cast<std::uint8_t>(entry.kind);
        XXH3_128bits_update(state, &kind, sizeof(kind));
        hash_u64(state, entry.name.size());
        XXH3_128bits_update(state, entry.name.data(), entry.name.size());
        hash_u64(state, entry.bytes.size());
        XXH3_128bits_update(state, entry.bytes.data(), entry.bytes.size());
    }
    const XXH128_hash_t hash = XXH3_128bits_digest(state);
    XXH3_freeState(state);

    XXH128_canonical_t canonical{};
    XXH128_canonicalFromHash(&canonical, hash);
    constexpr char Hex[] = "0123456789abcdef";
    std::string result;
    result.reserve(sizeof(canonical.digest) * 2);
    for (const unsigned char byte : canonical.digest) {
        result.push_back(Hex[byte >> 4]);
        result.push_back(Hex[byte & 0xf]);
    }
    return result;
}

std::string finish_digest(XXH3_state_t* state) {
    const XXH128_hash_t hash = XXH3_128bits_digest(state);
    XXH128_canonical_t canonical{};
    XXH128_canonicalFromHash(&canonical, hash);
    constexpr char Hex[] = "0123456789abcdef";
    std::string result;
    result.reserve(sizeof(canonical.digest) * 2);
    for (const unsigned char byte : canonical.digest) {
        result.push_back(Hex[byte >> 4]);
        result.push_back(Hex[byte & 0xf]);
    }
    return result;
}

std::string bytes_digest(const void* const bytes, const std::size_t size) {
    const XXH128_hash_t hash = XXH3_128bits(bytes, size);
    XXH128_canonical_t canonical{};
    XXH128_canonicalFromHash(&canonical, hash);
    constexpr char Hex[] = "0123456789abcdef";
    std::string result;
    result.reserve(sizeof(canonical.digest) * 2);
    for (const unsigned char byte : canonical.digest) {
        result.push_back(Hex[byte >> 4]);
        result.push_back(Hex[byte & 0xf]);
    }
    return result;
}

}  // namespace

StateCheckpointError StateCheckpoint::validateNameAndSize(
    const std::string_view name, const std::size_t size) const {
    if (name.empty()) {
        return StateCheckpointError::InvalidName;
    }
    if (size == 0) {
        return StateCheckpointError::EmptyState;
    }
    if (std::ranges::any_of(mEntries,
            [name](const Entry& entry) { return entry.name == name; })) {
        return StateCheckpointError::DuplicateName;
    }
    return StateCheckpointError::None;
}

StateCheckpointError StateCheckpoint::addMemoryRegion(
    const std::string_view name, void* const address, const std::size_t size) {
    const StateCheckpointError common = validateNameAndSize(name, size);
    if (common != StateCheckpointError::None) {
        return common;
    }
    if (address == nullptr) {
        return StateCheckpointError::NullAddress;
    }
    const auto start = reinterpret_cast<std::uintptr_t>(address);
    if (size > std::numeric_limits<std::uintptr_t>::max() - start) {
        return StateCheckpointError::AddressOverflow;
    }
    const std::uintptr_t end = start + size;
    for (const Entry& entry : mEntries) {
        if (entry.kind != StateCheckpointEntryKind::MemoryRegion) {
            continue;
        }
        const auto otherStart = reinterpret_cast<std::uintptr_t>(entry.address);
        const std::uintptr_t otherEnd = otherStart + entry.size;
        if (start < otherEnd && otherStart < end) {
            return StateCheckpointError::OverlappingRegion;
        }
    }
    try {
        mEntries.push_back(Entry{
            .name = std::string(name),
            .kind = StateCheckpointEntryKind::MemoryRegion,
            .address = static_cast<std::byte*>(address),
            .size = size,
        });
    } catch (const std::bad_alloc&) {
        return StateCheckpointError::AllocationFailed;
    }
    return StateCheckpointError::None;
}

StateCheckpointError StateCheckpoint::addComponent(const std::string_view name,
    const std::size_t size, void* const context, const StateCheckpointCaptureCallback capture,
    const StateCheckpointRestoreCallback restore) {
    const StateCheckpointError common = validateNameAndSize(name, size);
    if (common != StateCheckpointError::None) {
        return common;
    }
    if (capture == nullptr || restore == nullptr) {
        return StateCheckpointError::MissingCallback;
    }
    try {
        mEntries.push_back(Entry{
            .name = std::string(name),
            .kind = StateCheckpointEntryKind::Component,
            .size = size,
            .context = context,
            .capture = capture,
            .restore = restore,
        });
    } catch (const std::bad_alloc&) {
        return StateCheckpointError::AllocationFailed;
    }
    return StateCheckpointError::None;
}

StateCheckpointError StateCheckpoint::capture(StateCheckpointImage& image) const {
    StateCheckpointImage captured;
    try {
        captured.entries.reserve(mEntries.size());
        for (const Entry& entry : mEntries) {
            StateCheckpointImageEntry output{
                .name = entry.name,
                .kind = entry.kind,
                .bytes = std::vector<std::byte>(entry.size),
            };
            if (entry.kind == StateCheckpointEntryKind::MemoryRegion) {
                std::memcpy(output.bytes.data(), entry.address, entry.size);
            } else if (!entry.capture(entry.context, output.bytes)) {
                return StateCheckpointError::CaptureFailed;
            }
            captured.entries.push_back(std::move(output));
        }
        captured.digest = checkpoint_digest(captured);
    } catch (const std::bad_alloc&) {
        return StateCheckpointError::AllocationFailed;
    }
    if (captured.digest.empty()) {
        return StateCheckpointError::AllocationFailed;
    }
    image = std::move(captured);
    return StateCheckpointError::None;
}

StateCheckpointError StateCheckpoint::restore(const StateCheckpointImage& image) const {
    if (image.entries.size() != mEntries.size()) {
        return StateCheckpointError::ManifestMismatch;
    }
    for (std::size_t index = 0; index < mEntries.size(); ++index) {
        const Entry& expected = mEntries[index];
        const StateCheckpointImageEntry& actual = image.entries[index];
        if (actual.name != expected.name || actual.kind != expected.kind ||
            actual.bytes.size() != expected.size) {
            return StateCheckpointError::ManifestMismatch;
        }
    }
    if (image.digest.empty() || checkpoint_digest(image) != image.digest) {
        return StateCheckpointError::DigestMismatch;
    }
    for (std::size_t index = 0; index < mEntries.size(); ++index) {
        const Entry& destination = mEntries[index];
        const auto& source = image.entries[index].bytes;
        if (destination.kind == StateCheckpointEntryKind::MemoryRegion) {
            std::memcpy(destination.address, source.data(), destination.size);
        } else if (!destination.restore(destination.context, source)) {
            return StateCheckpointError::RestoreFailed;
        }
    }
    return StateCheckpointError::None;
}

StateCheckpointError StateCheckpoint::currentDigest(std::string& digest,
    std::vector<StateCheckpointEntryDigest>* const entryDigests) const {
    XXH3_state_t* state = XXH3_createState();
    if (state == nullptr) {
        return StateCheckpointError::AllocationFailed;
    }
    XXH3_128bits_reset(state);
    constexpr std::string_view Domain = "dusklight-state-checkpoint/v1";
    XXH3_128bits_update(state, Domain.data(), Domain.size());
    hash_u64(state, mEntries.size());
    std::vector<std::byte> componentBytes;
    try {
        std::vector<StateCheckpointEntryDigest> capturedEntryDigests;
        if (entryDigests != nullptr) {
            capturedEntryDigests.reserve(mEntries.size());
        }
        for (const Entry& entry : mEntries) {
            const std::uint8_t kind = static_cast<std::uint8_t>(entry.kind);
            XXH3_128bits_update(state, &kind, sizeof(kind));
            hash_u64(state, entry.name.size());
            XXH3_128bits_update(state, entry.name.data(), entry.name.size());
            hash_u64(state, entry.size);
            const void* entryBytes = nullptr;
            if (entry.kind == StateCheckpointEntryKind::MemoryRegion) {
                XXH3_128bits_update(state, entry.address, entry.size);
                entryBytes = entry.address;
            } else {
                componentBytes.resize(entry.size);
                if (!entry.capture(entry.context, componentBytes)) {
                    XXH3_freeState(state);
                    return StateCheckpointError::CaptureFailed;
                }
                XXH3_128bits_update(state, componentBytes.data(), componentBytes.size());
                entryBytes = componentBytes.data();
            }
            if (entryDigests != nullptr) {
                capturedEntryDigests.push_back({
                    .name = entry.name,
                    .kind = entry.kind,
                    .size = entry.size,
                    .digest = bytes_digest(entryBytes, entry.size),
                });
            }
        }
        std::string finished = finish_digest(state);
        XXH3_freeState(state);
        state = nullptr;
        digest = std::move(finished);
        if (entryDigests != nullptr) {
            *entryDigests = std::move(capturedEntryDigests);
        }
    } catch (const std::bad_alloc&) {
        if (state != nullptr) {
            XXH3_freeState(state);
        }
        return StateCheckpointError::AllocationFailed;
    }
    return digest.empty() ? StateCheckpointError::AllocationFailed : StateCheckpointError::None;
}

std::size_t StateCheckpoint::byteCount() const {
    std::size_t total = 0;
    for (const Entry& entry : mEntries) {
        if (entry.size > std::numeric_limits<std::size_t>::max() - total) {
            return std::numeric_limits<std::size_t>::max();
        }
        total += entry.size;
    }
    return total;
}

const char* state_checkpoint_error_message(const StateCheckpointError error) {
    switch (error) {
    case StateCheckpointError::None: return "none";
    case StateCheckpointError::InvalidName: return "checkpoint entry name is empty";
    case StateCheckpointError::EmptyState: return "checkpoint entry has zero bytes";
    case StateCheckpointError::NullAddress: return "checkpoint memory region has a null address";
    case StateCheckpointError::AddressOverflow: return "checkpoint memory region overflows the address space";
    case StateCheckpointError::DuplicateName: return "checkpoint entry name is duplicated";
    case StateCheckpointError::OverlappingRegion: return "checkpoint memory regions overlap";
    case StateCheckpointError::MissingCallback: return "checkpoint component callback is missing";
    case StateCheckpointError::CaptureFailed: return "checkpoint component capture failed";
    case StateCheckpointError::RestoreFailed: return "checkpoint component restore failed";
    case StateCheckpointError::ManifestMismatch: return "checkpoint image manifest does not match the registered state";
    case StateCheckpointError::DigestMismatch: return "checkpoint image digest does not match its contents";
    case StateCheckpointError::AllocationFailed: return "checkpoint allocation failed";
    }
    return "unknown checkpoint error";
}

}  // namespace dusk::automation
