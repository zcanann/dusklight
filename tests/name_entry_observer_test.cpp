#include "dusk/automation/name_entry_observer.hpp"

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
    REQUIRE(observer.fidelityProfile() == NameEntryFidelityProfile::ObserveOnly);
    observer.beginSession();
    observer.updateVisualCursor(7);

    std::array<NameEntryCharacterObservation, NameEntryOriginalLayout::CharacterCount> characters{};
    characters[7].character = 'Z';
    observer.observe(8, 7, 8, 0, 12, 4, 2, characters);
    observer.noteCursorMove(8, 9, 1, true);

    const NameEntryObservation& snapshot = observer.latest();
    REQUIRE(snapshot.active == 1);
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
}

void testOriginalLayoutWritesStayInShadowMemory() {
    using namespace dusk::automation;

    NameEntryObserver observer;
    observer.beginSession();

    // Observe-only mode reports the attempt but cannot mutate the model.
    REQUIRE(!observer.noteCharacterWrite(9, 2, 3, 4, 0x11223344, false, true));
    for (std::uint8_t byte : observer.latest().modeledNeighborBytes) {
        REQUIRE(byte == 0);
    }

    observer.setFidelityProfile(NameEntryFidelityProfile::CursorBreakoutShadow);
    REQUIRE(observer.noteCharacterWrite(9, 2, 3, 4, 0x11223344, true, true));
    const auto& shadow = observer.latest().modeledNeighborBytes;
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
