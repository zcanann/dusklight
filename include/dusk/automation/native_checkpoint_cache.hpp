#pragma once

#include "dusk/automation/state_checkpoint.hpp"

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace dusk::automation {

struct NativeCheckpointCacheStats {
    std::size_t capacityBytes = 0;
    std::size_t capacityEntries = 0;
    std::size_t residentBytes = 0;
    std::size_t residentCheckpointBytes = 0;
    std::size_t residentMetadataBytes = 0;
    std::size_t residentEntries = 0;
    std::uint64_t insertions = 0;
    std::uint64_t replacements = 0;
    std::uint64_t evictions = 0;
    std::uint64_t hits = 0;
    std::uint64_t misses = 0;
};

[[nodiscard]] inline std::size_t state_checkpoint_image_payload_bytes(
    const StateCheckpointImage& image) noexcept
{
    std::size_t total = 0;
    for (const StateCheckpointImageEntry& entry : image.entries) {
        if (entry.bytes.size() > std::numeric_limits<std::size_t>::max() - total)
            return std::numeric_limits<std::size_t>::max();
        total += entry.bytes.size();
    }
    return total;
}

/**
 * Bounded process-local storage for genuine native checkpoint images.
 *
 * `Metadata` carries the host-side state that must accompany an emulated
 * machine image. Callers provide its accounted payload size explicitly so
 * reports distinguish machine bytes from host snapshot bytes. Allocation
 * overhead is intentionally not guessed.
 *
 * Eviction is deterministic least-recently-used. Equal access ordinals are
 * broken by the canonical checkpoint identity.
 */
template <typename Metadata>
class NativeCheckpointCache {
public:
    struct Entry {
        std::string identity;
        std::string semanticDigest;
        StateCheckpointImage image;
        Metadata metadata;
        std::size_t checkpointBytes = 0;
        std::size_t metadataBytes = 0;
        std::uint64_t lastUseOrdinal = 0;

        [[nodiscard]] std::size_t accountedBytes() const noexcept {
            return checkpointBytes + metadataBytes;
        }
    };

    NativeCheckpointCache(const std::size_t capacityBytes,
        const std::size_t capacityEntries) noexcept
        : mCapacityBytes(capacityBytes), mCapacityEntries(capacityEntries)
    {}

    [[nodiscard]] bool insert(std::string identity, std::string semanticDigest,
        StateCheckpointImage image, Metadata metadata, const std::size_t metadataBytes,
        std::vector<std::string>* const evictedIdentities = nullptr)
    {
        const std::size_t checkpointBytes = state_checkpoint_image_payload_bytes(image);
        if (identity.empty() || semanticDigest.empty() || image.digest != identity ||
            checkpointBytes == 0 ||
            checkpointBytes > std::numeric_limits<std::size_t>::max() - metadataBytes)
            return false;
        const std::size_t accountedBytes = checkpointBytes + metadataBytes;
        if (mCapacityEntries == 0 || accountedBytes > mCapacityBytes)
            return false;

        const auto existing = findWithoutAccounting(identity);
        if (existing != mEntries.end()) {
            subtractResident(*existing);
            mEntries.erase(existing);
            ++mReplacements;
        }

        while (!mEntries.empty() &&
               (mEntries.size() >= mCapacityEntries ||
                   accountedBytes > mCapacityBytes - mResidentBytes))
        {
            const auto victim = std::min_element(mEntries.begin(), mEntries.end(),
                [](const Entry& left, const Entry& right) {
                    if (left.lastUseOrdinal != right.lastUseOrdinal)
                        return left.lastUseOrdinal < right.lastUseOrdinal;
                    return left.identity < right.identity;
                });
            if (evictedIdentities != nullptr)
                evictedIdentities->push_back(victim->identity);
            subtractResident(*victim);
            mEntries.erase(victim);
            ++mEvictions;
        }

        Entry entry{
            .identity = std::move(identity),
            .semanticDigest = std::move(semanticDigest),
            .image = std::move(image),
            .metadata = std::move(metadata),
            .checkpointBytes = checkpointBytes,
            .metadataBytes = metadataBytes,
            .lastUseOrdinal = nextOrdinal(),
        };
        mResidentBytes += accountedBytes;
        mResidentCheckpointBytes += checkpointBytes;
        mResidentMetadataBytes += metadataBytes;
        mEntries.push_back(std::move(entry));
        ++mInsertions;
        return true;
    }

    [[nodiscard]] const Entry* find(const std::string_view identity) noexcept {
        const auto found = findWithoutAccounting(identity);
        if (found == mEntries.end()) {
            ++mMisses;
            return nullptr;
        }
        found->lastUseOrdinal = nextOrdinal();
        ++mHits;
        return &*found;
    }

    [[nodiscard]] const Entry* peek(const std::string_view identity) const noexcept {
        const auto found = std::ranges::find(mEntries, identity, &Entry::identity);
        return found == mEntries.end() ? nullptr : &*found;
    }

    [[nodiscard]] NativeCheckpointCacheStats stats() const noexcept {
        return {
            .capacityBytes = mCapacityBytes,
            .capacityEntries = mCapacityEntries,
            .residentBytes = mResidentBytes,
            .residentCheckpointBytes = mResidentCheckpointBytes,
            .residentMetadataBytes = mResidentMetadataBytes,
            .residentEntries = mEntries.size(),
            .insertions = mInsertions,
            .replacements = mReplacements,
            .evictions = mEvictions,
            .hits = mHits,
            .misses = mMisses,
        };
    }

private:
    using Iterator = typename std::vector<Entry>::iterator;

    [[nodiscard]] Iterator findWithoutAccounting(const std::string_view identity) noexcept {
        return std::ranges::find(mEntries, identity, &Entry::identity);
    }

    [[nodiscard]] std::uint64_t nextOrdinal() noexcept {
        if (mNextOrdinal != std::numeric_limits<std::uint64_t>::max())
            return ++mNextOrdinal;
        std::ranges::sort(mEntries, [](const Entry& left, const Entry& right) {
            if (left.lastUseOrdinal != right.lastUseOrdinal)
                return left.lastUseOrdinal < right.lastUseOrdinal;
            return left.identity < right.identity;
        });
        for (std::size_t index = 0; index < mEntries.size(); ++index)
            mEntries[index].lastUseOrdinal = index + 1;
        mNextOrdinal = mEntries.size();
        return ++mNextOrdinal;
    }

    void subtractResident(const Entry& entry) noexcept {
        mResidentBytes -= entry.accountedBytes();
        mResidentCheckpointBytes -= entry.checkpointBytes;
        mResidentMetadataBytes -= entry.metadataBytes;
    }

    std::size_t mCapacityBytes = 0;
    std::size_t mCapacityEntries = 0;
    std::size_t mResidentBytes = 0;
    std::size_t mResidentCheckpointBytes = 0;
    std::size_t mResidentMetadataBytes = 0;
    std::uint64_t mNextOrdinal = 0;
    std::uint64_t mInsertions = 0;
    std::uint64_t mReplacements = 0;
    std::uint64_t mEvictions = 0;
    std::uint64_t mHits = 0;
    std::uint64_t mMisses = 0;
    std::vector<Entry> mEntries;
};

}  // namespace dusk::automation
