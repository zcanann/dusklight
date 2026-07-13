#include "SSystem/SComponent/c_math_rng.h"

#include <cmath>

namespace {

struct RngStream {
    std::int32_t state0 = 0;
    std::int32_t state1 = 0;
    std::int32_t state2 = 0;
    std::uint64_t callCount = 0;
};

RngStream g_primary;
RngStream g_secondary;

RngStream* stream(const cM_RndStream id) {
    switch (id) {
    case cM_RndStream::Primary:
        return &g_primary;
    case cM_RndStream::Secondary:
        return &g_secondary;
    }
    return nullptr;
}

void initialize(RngStream& rng, const int state0, const int state1, const int state2) {
    rng.state0 = state0;
    rng.state1 = state1;
    rng.state2 = state2;
    rng.callCount = 0;
}

float next(RngStream& rng) {
    rng.state0 = (rng.state0 * 171) % 30269;
    rng.state1 = (rng.state1 * 172) % 30307;
    rng.state2 = (rng.state2 * 170) % 30323;
    ++rng.callCount;

    const float sum = rng.state0 / 30269.0f + rng.state1 / 30307.0f + rng.state2 / 30323.0f;
    return std::fabs(std::fmod(sum, 1.0f));
}

} // namespace

void cM_initRnd(const int state0, const int state1, const int state2) {
    initialize(g_primary, state0, state1, state2);
}

float cM_rnd() {
    return next(g_primary);
}

float cM_rndF(const float max) {
    return cM_rnd() * max;
}

float cM_rndFX(const float max) {
    return max * (cM_rnd() - 0.5f) * 2.0f;
}

void cM_initRnd2(const int state0, const int state1, const int state2) {
    initialize(g_secondary, state0, state1, state2);
}

float cM_rnd2() {
    return next(g_secondary);
}

float cM_rndF2(const float max) {
    return cM_rnd2() * max;
}

float cM_rndFX2(const float max) {
    return max * (cM_rnd2() - 0.5f) * 2.0f;
}

bool cM_getRndState(const cM_RndStream id, cM_RndState& output) {
    const RngStream* rng = stream(id);
    if (rng == nullptr) {
        return false;
    }
    output.version = cM_RndStateVersion;
    output.state0 = rng->state0;
    output.state1 = rng->state1;
    output.state2 = rng->state2;
    output.callCount = rng->callCount;
    return true;
}

bool cM_setRndState(const cM_RndStream id, const cM_RndState& state) {
    RngStream* rng = stream(id);
    if (rng == nullptr || state.version != cM_RndStateVersion) {
        return false;
    }
    rng->state0 = state.state0;
    rng->state1 = state.state1;
    rng->state2 = state.state2;
    rng->callCount = state.callCount;
    return true;
}
