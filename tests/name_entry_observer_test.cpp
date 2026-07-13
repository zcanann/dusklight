#include "dusk/automation/name_entry_observer.hpp"
#include "dusk/automation/eye_shredder_oracle.hpp"

#include <array>
#include <cstdlib>
#include <iostream>

namespace {

void require(bool condition, const char* expression, int line) {
    if (!condition) {
        std::cerr << "name_entry_observer_test.cpp:" << line << ": check failed: "
                  << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

void testSnapshotAndOutOfRangeEvents() {
    using namespace dusk::automation;

    NameEntryObserver observer;
    observer.setTickContext(42, 17);
    REQUIRE(observer.fidelityProfile() == NameEntryFidelityProfile::ObserveOnly);
    observer.beginSession();
    REQUIRE(!observer.inputProcessed());
    observer.markInputProcessed();
    REQUIRE(observer.inputProcessed());
    observer.updateVisualCursor(7);

    std::array<NameEntryCharacterObservation, NameEntryOriginalLayout::CharacterCount> characters{};
    characters[7].character = 'Z';
    observer.observe(8, 7, 8, 0, 12, 4, 2, characters);
    observer.noteCursorMove(8, 9, 1, true);

    const NameEntryObservation& snapshot = observer.latest();
    REQUIRE(snapshot.active == 1);
    REQUIRE(snapshot.simTick == 42);
    REQUIRE(snapshot.tapeFrame == 17);
    REQUIRE(snapshot.logicalCursor == 8);
    REQUIRE(snapshot.visualCursor == 7);
    REQUIRE(snapshot.characters[7].character == 'Z');
    REQUIRE(snapshot.outOfRangeMoveAttempts == 1);

    std::array<NameEntryEvent, 4> events{};
    REQUIRE(observer.drainEvents(events) == 2);
    REQUIRE(events[1].kind == NameEntryEventKind::CursorMoveAttempt);
    REQUIRE((events[1].flags & NameEntryEventAccepted) != 0);
    REQUIRE((events[1].flags & NameEntryEventOutOfRange) != 0);
    REQUIRE(events[1].cursorBefore == 8);
    REQUIRE(events[1].cursorRequested == 9);
    REQUIRE(events[1].simTick == 42);
    REQUIRE(events[1].tapeFrame == 17);
    observer.endSession();
    REQUIRE(!observer.inputProcessed());
}

void testOriginalLayoutWritesStayInShadowMemory() {
    using namespace dusk::automation;

    NameEntryObserver observer;
    observer.beginSession();

    // Observe-only mode reports the attempt but cannot mutate the model.
    REQUIRE(!observer.noteCharacterWrite(9, 2, 3, 4, 0x11223344, false, true));
    for (std::uint8_t byte : observer.latest().modeledRetailBytes) {
        REQUIRE(byte == 0);
    }

    observer.setFidelityProfile(NameEntryFidelityProfile::CursorBreakoutShadow);
    REQUIRE(observer.noteCharacterWrite(9, 2, 3, 4, 0x11223344, true, true));
    const auto& shadow = observer.latest().modeledRetailBytes;
    // ChrInfo[9] is 0x314, eight bytes into the 0x30C neighbor window.
    REQUIRE(shadow[8] == 2);
    REQUIRE(shadow[9] == 3);
    REQUIRE(shadow[10] == 4);
    REQUIRE(shadow[11] == 1);
    REQUIRE(shadow[12] == 0x11);
    REQUIRE(shadow[13] == 0x22);
    REQUIRE(shadow[14] == 0x33);
    REQUIRE(shadow[15] == 0x44);

    std::array<NameEntryEvent, 4> events{};
    REQUIRE(observer.drainEvents(events) == 3);
    REQUIRE(events[2].originalOffset == 0x314);
    REQUIRE((events[2].flags & NameEntryEventOutOfRange) != 0);
    REQUIRE((events[2].flags & NameEntryEventShadowModeled) != 0);
    REQUIRE(observer.latest().outOfRangeWriteAttempts == 2);

    observer.setTickContext(100, 99);
    REQUIRE(observer.noteCharacterWrite(113, 12, 0, 2, 'M', true, true));
    constexpr std::size_t eyeShredderOffset =
        EyeShredderExpectedWrite::OriginalOffset - NameEntryOriginalLayout::NeighborWindow;
    const auto& eyeShredderShadow = observer.latest().modeledRetailBytes;
    REQUIRE(eyeShredderShadow[eyeShredderOffset] == 0x0C);
    REQUIRE(eyeShredderShadow[eyeShredderOffset + 1] == 0x00);
    REQUIRE(eyeShredderShadow[eyeShredderOffset + 2] == 0x02);
    REQUIRE(eyeShredderShadow[eyeShredderOffset + 3] == 0x01);
    REQUIRE(eyeShredderShadow[eyeShredderOffset + 7] == 0x4D);
    REQUIRE(observer.latest().lastWrite.characterIndex == 113);
    REQUIRE(observer.latest().lastWrite.originalOffset == 0x654);
    REQUIRE(observer.latest().lastWrite.simTick == 100);
    REQUIRE(observer.latest().lastWrite.tapeFrame == 99);
}

void testEventRingIsBounded() {
    using namespace dusk::automation;

    NameEntryObserver observer;
    for (std::size_t i = 0; i < NameEntryObserver::EventCapacity + 5; ++i) {
        observer.noteCursorMove(0, 1, 1, true);
    }
    REQUIRE(observer.eventCount() == NameEntryObserver::EventCapacity);
    REQUIRE(observer.droppedEventCount() == 5);
}

} // namespace

int main() {
    testSnapshotAndOutOfRangeEvents();
    testOriginalLayoutWritesStayInShadowMemory();
    testEventRingIsBounded();
    std::cout << "name entry observer tests passed\n";
    return 0;
}
