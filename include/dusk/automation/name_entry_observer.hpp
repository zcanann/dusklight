#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <span>

namespace dusk::automation {

// Original 32-bit dName_c layout. These are logical offsets, never host
// pointers or native offsetof values.
struct NameEntryOriginalLayout {
    static constexpr std::uint16_t Cursor = 0x2B1;
    static constexpr std::uint16_t InputString = 0x2B5;
    static constexpr std::uint16_t InputStringSize = 23;
    static constexpr std::uint16_t CharacterInfo = 0x2CC;
    static constexpr std::uint16_t CharacterInfoSize = 8;
    static constexpr std::uint16_t CharacterCount = 8;
    static constexpr std::uint16_t NeighborWindow = 0x30C;
    static constexpr std::uint16_t MenuHistory = 0x30C;
    static constexpr std::uint16_t MenuHistorySize = 16;
    static constexpr std::uint16_t NextNameString = 0x31C;
    static constexpr std::uint16_t NextNameStringSize = 24;
    static constexpr std::uint16_t ObjectEnd = 0x334;
    // Eye Shredder's documented valid write positions extend through index
    // 249. Model that retail address range explicitly without touching native
    // memory beyond dName_c.
    static constexpr std::uint16_t EyeShredderLastIndex = 249;
    static constexpr std::uint16_t EyeShredderWindowEnd =
        CharacterInfo + (EyeShredderLastIndex + 1) * CharacterInfoSize;
    static constexpr std::size_t EyeShredderWindowSize =
        EyeShredderWindowEnd - NeighborWindow;
};

inline constexpr std::uint64_t NameEntryNoTick = std::numeric_limits<std::uint64_t>::max();

enum class NameEntryFidelityProfile : std::uint8_t {
    ObserveOnly = 0,
    // Reproduces the excluded cursor movement and original-layout writes in a
    // bounded shadow window. It never performs a native out-of-bounds access.
    CursorBreakoutShadow = 1,
};

enum class NameEntryCapability : std::uint32_t {
    None = 0,
    OriginalLayoutOffsets = 1 << 0,
    CursorBreakoutShadow = 1 << 1,
};

constexpr NameEntryCapability operator|(NameEntryCapability lhs, NameEntryCapability rhs) {
    return static_cast<NameEntryCapability>(static_cast<std::uint32_t>(lhs) |
                                            static_cast<std::uint32_t>(rhs));
}

enum class NameEntryEventKind : std::uint8_t {
    SessionStarted,
    SessionEnded,
    CursorMoveAttempt,
    CharacterWriteAttempt,
    CharacterReadBlocked,
};

enum NameEntryEventFlags : std::uint8_t {
    NameEntryEventAccepted = 1 << 0,
    NameEntryEventOutOfRange = 1 << 1,
    NameEntryEventShadowModeled = 1 << 2,
};

struct NameEntryCharacterObservation {
    std::uint8_t column = 0;
    std::uint8_t row = 0;
    std::uint8_t characterSet = 0;
    std::uint8_t active = 0;
    std::int32_t character = 0;

    bool operator==(const NameEntryCharacterObservation&) const = default;
};

struct NameEntryWriteObservation {
    std::uint64_t attempt = 0;
    std::uint64_t simTick = NameEntryNoTick;
    std::uint64_t tapeFrame = NameEntryNoTick;
    std::uint8_t characterIndex = 0;
    std::uint16_t originalOffset = 0;
    std::uint8_t flags = 0;
    std::array<std::uint8_t, NameEntryOriginalLayout::CharacterInfoSize> bytes{};
};

struct NameEntryObservation {
    std::uint64_t revision = 0;
    std::uint64_t simTick = NameEntryNoTick;
    std::uint64_t tapeFrame = NameEntryNoTick;
    std::uint8_t active = 0;
    std::uint8_t logicalCursor = 0;
    std::uint8_t lastLogicalCursor = 0;
    std::uint8_t visualCursor = 0;
    std::uint8_t nameLength = 0;
    std::uint8_t selectionProcedure = 0;
    std::uint8_t characterColumn = 0;
    std::uint8_t characterRow = 0;
    std::uint8_t characterSet = 0;
    std::uint64_t outOfRangeMoveAttempts = 0;
    std::uint64_t outOfRangeWriteAttempts = 0;
    std::uint64_t blockedReadAttempts = 0;
    std::array<NameEntryCharacterObservation, NameEntryOriginalLayout::CharacterCount> characters{};
    NameEntryWriteObservation lastWrite{};
    // Retail-layout shadow state beginning at field_0x30c and extending
    // through Eye Shredder position 249. This is diagnostic state, not native
    // object memory.
    std::array<std::uint8_t, NameEntryOriginalLayout::EyeShredderWindowSize>
        modeledRetailBytes{};
};

struct NameEntryEvent {
    std::uint64_t sequence = 0;
    std::uint64_t simTick = NameEntryNoTick;
    std::uint64_t tapeFrame = NameEntryNoTick;
    NameEntryEventKind kind = NameEntryEventKind::SessionStarted;
    std::uint8_t flags = 0;
    std::uint8_t cursorBefore = 0;
    std::uint8_t cursorRequested = 0;
    std::uint8_t visualCursor = 0;
    std::uint8_t characterIndex = 0;
    std::uint16_t originalOffset = 0;
    std::uint8_t accessSize = 0;
    std::int8_t direction = 0;
    std::int32_t character = 0;
};

class NameEntryObserver {
public:
    static constexpr std::size_t EventCapacity = 256;

    NameEntryCapability capabilities() const;
    void setFidelityProfile(NameEntryFidelityProfile profile);
    NameEntryFidelityProfile fidelityProfile() const { return mProfile; }
    bool cursorBreakoutShadowEnabled() const {
        return mProfile == NameEntryFidelityProfile::CursorBreakoutShadow;
    }

    void beginSession();
    void endSession();
    void setTickContext(std::uint64_t simTick, std::uint64_t tapeFrame);
    void updateVisualCursor(std::uint8_t visualCursor);
    void observe(std::uint8_t logicalCursor, std::uint8_t lastLogicalCursor,
                 std::uint8_t nameLength, std::uint8_t selectionProcedure,
                 std::uint8_t characterColumn, std::uint8_t characterRow,
                 std::uint8_t characterSet,
                 std::span<const NameEntryCharacterObservation,
                           NameEntryOriginalLayout::CharacterCount> characters);
    void noteCursorMove(std::uint8_t before, std::uint8_t requested,
                        std::int8_t direction, bool accepted);
    bool noteCharacterWrite(std::uint8_t index, std::uint8_t column,
                            std::uint8_t row, std::uint8_t characterSet,
                            std::int32_t character, bool accepted,
                            bool modelOriginalLayout);
    void noteCharacterReadBlocked(std::uint8_t index);

    const NameEntryObservation& latest() const { return mLatest; }
    std::size_t eventCount() const { return mEventCount; }
    std::uint64_t droppedEventCount() const { return mDroppedEvents; }
    std::size_t drainEvents(std::span<NameEntryEvent> output);
    void clearEvents();

private:
    void push(NameEntryEvent event);

    NameEntryObservation mLatest{};
    std::array<NameEntryEvent, EventCapacity> mEvents{};
    std::size_t mEventHead = 0;
    std::size_t mEventCount = 0;
    std::uint64_t mNextSequence = 1;
    std::uint64_t mDroppedEvents = 0;
    std::uint64_t mCurrentSimTick = NameEntryNoTick;
    std::uint64_t mCurrentTapeFrame = NameEntryNoTick;
    std::uint64_t mWriteAttemptCount = 0;
    NameEntryFidelityProfile mProfile = NameEntryFidelityProfile::ObserveOnly;
};

NameEntryObserver& name_entry_observer();

} // namespace dusk::automation
