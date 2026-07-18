#include "SSystem/SComponent/c_math_rng.h"
#include "dusk/automation/rng.hpp"

#include <array>
#include <bit>
#include <cstdlib>
#include <iostream>

namespace {

void require(bool condition, const char* expression, int line) {
    if (!condition) {
        std::cerr << "rng_test.cpp:" << line << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

struct KnownStep {
    std::int32_t state0;
    std::int32_t state1;
    std::int32_t state2;
    std::uint32_t outputBits;
};

void testKnownPrimarySequence() {
    constexpr std::array<KnownStep, 6> expected{{
        {17100, 17200, 17000, 0x3F316E64},
        {18276, 18621, 9315, 0x3F068008},
        {7489, 20577, 6754, 0x3E18AE38},
        {9321, 23632, 26229, 0x3F73E2D0},
        {19903, 3566, 1449, 0x3F52AF2E},
        {13285, 7212, 3746, 0x3F4CE6F8},
    }};

    cM_initRnd(100, 100, 100);
    for (std::size_t i = 0; i < expected.size(); ++i) {
        const float value = cM_rnd();
        cM_RndState state;
        REQUIRE(cM_getRndState(cM_RndStream::Primary, state));
        REQUIRE(state.state0 == expected[i].state0);
        REQUIRE(state.state1 == expected[i].state1);
        REQUIRE(state.state2 == expected[i].state2);
        REQUIRE(state.callCount == i + 1);
        REQUIRE(std::bit_cast<std::uint32_t>(value) == expected[i].outputBits);
    }
}

void testKnownSecondarySequenceAndIndependence() {
    constexpr std::array<KnownStep, 4> expected{{
        {2052, 21156, 11900, 0x3E2216D0},
        {17933, 1992, 21682, 0x3EBF163C},
        {9374, 9247, 16857, 0x3E2ED008},
        {28966, 14520, 15328, 0x3F7108D2},
    }};

    cM_initRnd(100, 100, 100);
    cM_initRnd2(12, 123, 70);
    cM_rnd();
    for (std::size_t i = 0; i < expected.size(); ++i) {
        const float value = cM_rnd2();
        cM_RndState state;
        REQUIRE(cM_getRndState(cM_RndStream::Secondary, state));
        REQUIRE(state.state0 == expected[i].state0);
        REQUIRE(state.state1 == expected[i].state1);
        REQUIRE(state.state2 == expected[i].state2);
        REQUIRE(state.callCount == i + 1);
        REQUIRE(std::bit_cast<std::uint32_t>(value) == expected[i].outputBits);
    }
    cM_RndState primary;
    REQUIRE(cM_getRndState(cM_RndStream::Primary, primary));
    REQUIRE(primary.callCount == 1);
}

void testInitializationAndHelperCallCounts() {
    cM_initRnd(7, 8, 9);
    cM_RndState state;
    REQUIRE(cM_getRndState(cM_RndStream::Primary, state));
    REQUIRE(state.state0 == 7 && state.state1 == 8 && state.state2 == 9);
    REQUIRE(state.callCount == 0);

    cM_rndF(4.0f);
    cM_rndFX(2.0f);
    REQUIRE(cM_getRndState(cM_RndStream::Primary, state));
    REQUIRE(state.callCount == 2);

    cM_initRnd(7, 8, 9);
    REQUIRE(cM_getRndState(cM_RndStream::Primary, state));
    REQUIRE(state.callCount == 0);
}

void testExactStateRestore() {
    const cM_RndState requested{
        .version = cM_RndStateVersion,
        .state0 = 101,
        .state1 = -202,
        .state2 = 303,
        .callCount = 0x1'0000'0007ULL,
    };
    REQUIRE(cM_setRndState(cM_RndStream::Primary, requested));

    cM_RndState observed;
    REQUIRE(cM_getRndState(cM_RndStream::Primary, observed));
    REQUIRE(observed == requested);

    auto unsupported = requested;
    unsupported.version = cM_RndStateVersion + 1;
    REQUIRE(!cM_setRndState(cM_RndStream::Primary, unsupported));
    REQUIRE(cM_getRndState(cM_RndStream::Primary, observed));
    REQUIRE(observed == requested);
}

void testSnapshotCaptureDoesNotAdvanceEitherStream() {
    using namespace dusk::automation;

    cM_initRnd(100, 100, 100);
    cM_initRnd2(12, 123, 70);
    cM_rnd();
    cM_rnd2();
    cM_rnd2();
    cM_RndState primaryBefore;
    cM_RndState secondaryBefore;
    REQUIRE(cM_getRndState(cM_RndStream::Primary, primaryBefore));
    REQUIRE(cM_getRndState(cM_RndStream::Secondary, secondaryBefore));

    const GameRngSnapshot snapshot = capture_game_rng_snapshot();

    cM_RndState primaryAfter;
    cM_RndState secondaryAfter;
    REQUIRE(cM_getRndState(cM_RndStream::Primary, primaryAfter));
    REQUIRE(cM_getRndState(cM_RndStream::Secondary, secondaryAfter));
    REQUIRE(primaryAfter == primaryBefore);
    REQUIRE(secondaryAfter == secondaryBefore);
    REQUIRE(snapshot.streams[0].state0 == primaryBefore.state0);
    REQUIRE(snapshot.streams[0].state1 == primaryBefore.state1);
    REQUIRE(snapshot.streams[0].state2 == primaryBefore.state2);
    REQUIRE(snapshot.streams[0].callCount == primaryBefore.callCount);
    REQUIRE(snapshot.streams[1].state0 == secondaryBefore.state0);
    REQUIRE(snapshot.streams[1].state1 == secondaryBefore.state1);
    REQUIRE(snapshot.streams[1].state2 == secondaryBefore.state2);
    REQUIRE(snapshot.streams[1].callCount == secondaryBefore.callCount);
}

} // namespace

int main() {
    testKnownPrimarySequence();
    testKnownSecondarySequenceAndIndependence();
    testInitializationAndHelperCallCounts();
    testExactStateRestore();
    testSnapshotCaptureDoesNotAdvanceEitherStream();
    std::cout << "RNG tests passed\n";
    return 0;
}
