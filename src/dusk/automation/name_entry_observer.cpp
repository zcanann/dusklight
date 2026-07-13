#include "dusk/automation/name_entry_observer.hpp"

#include <algorithm>

namespace dusk::automation {
namespace {

constexpr bool character_index_out_of_range(const std::uint8_t index) {
    return index >= NameEntryOriginalLayout::CharacterCount;
}

constexpr std::uint16_t character_offset(const std::uint8_t index) {
    return static_cast<std::uint16_t>(NameEntryOriginalLayout::CharacterInfo +
                                      index * NameEntryOriginalLayout::CharacterInfoSize);
}

} // namespace

NameEntryCapability NameEntryObserver::capabilities() const {
    return NameEntryCapability::OriginalLayoutOffsets | NameEntryCapability::CursorBreakoutShadow;
}

void NameEntryObserver::setFidelityProfile(const NameEntryFidelityProfile profile) {
    mProfile = profile;
}

void NameEntryObserver::beginSession() {
    mLatest = {};
    mLatest.active = 1;
    mLatest.revision = 1;
    NameEntryEvent event;
    event.kind = NameEntryEventKind::SessionStarted;
    push(event);
}

void NameEntryObserver::endSession() {
    if (mLatest.active == 0) {
        return;
    }
    mLatest.active = 0;
    ++mLatest.revision;
    NameEntryEvent event;
    event.kind = NameEntryEventKind::SessionEnded;
    event.visualCursor = mLatest.visualCursor;
    push(event);
}

void NameEntryObserver::updateVisualCursor(const std::uint8_t visualCursor) {
    mLatest.visualCursor = visualCursor;
}

void NameEntryObserver::observe(
    const std::uint8_t logicalCursor, const std::uint8_t lastLogicalCursor,
    const std::uint8_t nameLength, const std::uint8_t selectionProcedure,
    const std::uint8_t characterColumn, const std::uint8_t characterRow,
    const std::uint8_t characterSet,
    const std::span<const NameEntryCharacterObservation,
                    NameEntryOriginalLayout::CharacterCount> characters) {
    mLatest.active = 1;
    mLatest.logicalCursor = logicalCursor;
    mLatest.lastLogicalCursor = lastLogicalCursor;
    mLatest.nameLength = nameLength;
    mLatest.selectionProcedure = selectionProcedure;
    mLatest.characterColumn = characterColumn;
    mLatest.characterRow = characterRow;
    mLatest.characterSet = characterSet;
    std::copy(characters.begin(), characters.end(), mLatest.characters.begin());
    ++mLatest.revision;
}

void NameEntryObserver::noteCursorMove(const std::uint8_t before,
                                       const std::uint8_t requested,
                                       const std::int8_t direction,
                                       const bool accepted) {
    NameEntryEvent event;
    event.kind = NameEntryEventKind::CursorMoveAttempt;
    event.cursorBefore = before;
    event.cursorRequested = requested;
    event.visualCursor = mLatest.visualCursor;
    event.direction = direction;
    if (accepted) {
        event.flags |= NameEntryEventAccepted;
    }
    if (requested > NameEntryOriginalLayout::CharacterCount) {
        event.flags |= NameEntryEventOutOfRange;
        ++mLatest.outOfRangeMoveAttempts;
        ++mLatest.revision;
    }
    push(event);
}

bool NameEntryObserver::noteCharacterWrite(
    const std::uint8_t index, const std::uint8_t column, const std::uint8_t row,
    const std::uint8_t characterSet, const std::int32_t character,
    const bool accepted, const bool modelOriginalLayout) {
    NameEntryEvent event;
    event.kind = NameEntryEventKind::CharacterWriteAttempt;
    event.characterIndex = index;
    event.originalOffset = character_offset(index);
    event.accessSize = NameEntryOriginalLayout::CharacterInfoSize;
    event.character = character;
    event.visualCursor = mLatest.visualCursor;
    if (accepted) {
        event.flags |= NameEntryEventAccepted;
    }
    if (character_index_out_of_range(index)) {
        event.flags |= NameEntryEventOutOfRange;
        ++mLatest.outOfRangeWriteAttempts;
        ++mLatest.revision;
    }

    bool modeled = false;
    if (modelOriginalLayout && cursorBreakoutShadowEnabled() &&
        character_index_out_of_range(index)) {
        const std::uint16_t target = character_offset(index);
        const std::array<std::uint8_t, NameEntryOriginalLayout::CharacterInfoSize> bytes{
            column,
            row,
            characterSet,
            1,
            static_cast<std::uint8_t>(static_cast<std::uint32_t>(character) >> 24),
            static_cast<std::uint8_t>(static_cast<std::uint32_t>(character) >> 16),
            static_cast<std::uint8_t>(static_cast<std::uint32_t>(character) >> 8),
            static_cast<std::uint8_t>(character),
        };
        for (std::size_t i = 0; i < bytes.size(); ++i) {
            const std::uint32_t offset = static_cast<std::uint32_t>(target) + i;
            if (offset >= NameEntryOriginalLayout::NeighborWindow &&
                offset < NameEntryOriginalLayout::ObjectEnd) {
                mLatest.modeledNeighborBytes[offset - NameEntryOriginalLayout::NeighborWindow] = bytes[i];
                modeled = true;
            }
        }
    }
    if (modeled) {
        event.flags |= NameEntryEventShadowModeled;
        ++mLatest.revision;
    }
    push(event);
    return modeled;
}

void NameEntryObserver::noteCharacterReadBlocked(const std::uint8_t index) {
    NameEntryEvent event;
    event.kind = NameEntryEventKind::CharacterReadBlocked;
    event.characterIndex = index;
    event.originalOffset = character_offset(index);
    event.accessSize = NameEntryOriginalLayout::CharacterInfoSize;
    event.flags = NameEntryEventOutOfRange;
    event.visualCursor = mLatest.visualCursor;
    ++mLatest.blockedReadAttempts;
    ++mLatest.revision;
    push(event);
}

std::size_t NameEntryObserver::drainEvents(const std::span<NameEntryEvent> output) {
    const std::size_t count = std::min(output.size(), mEventCount);
    for (std::size_t i = 0; i < count; ++i) {
        output[i] = mEvents[mEventHead];
        mEventHead = (mEventHead + 1) % EventCapacity;
    }
    mEventCount -= count;
    return count;
}

void NameEntryObserver::clearEvents() {
    mEventHead = 0;
    mEventCount = 0;
    mDroppedEvents = 0;
}

void NameEntryObserver::push(NameEntryEvent event) {
    event.sequence = mNextSequence++;
    if (mEventCount == EventCapacity) {
        mEventHead = (mEventHead + 1) % EventCapacity;
        --mEventCount;
        ++mDroppedEvents;
    }
    const std::size_t tail = (mEventHead + mEventCount) % EventCapacity;
    mEvents[tail] = event;
    ++mEventCount;
}

NameEntryObserver& name_entry_observer() {
    static NameEntryObserver observer;
    return observer;
}

} // namespace dusk::automation
