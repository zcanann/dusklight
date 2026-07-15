#ifndef C_MATH_RNG_H
#define C_MATH_RNG_H

#include <cstdint>

enum class cM_RndStream : std::uint8_t {
    Primary = 0,
    Secondary = 1,
};

inline constexpr std::uint32_t cM_RndStateVersion = 1;

struct cM_RndState {
    std::uint32_t version = cM_RndStateVersion;
    std::int32_t state0 = 0;
    std::int32_t state1 = 0;
    std::int32_t state2 = 0;
    std::uint64_t callCount = 0;

    bool operator==(const cM_RndState&) const = default;
};

void cM_initRnd(int, int, int);
float cM_rnd();
float cM_rndF(float);
float cM_rndFX(float);
void cM_initRnd2(int, int, int);
float cM_rnd2();
float cM_rndF2(float);
float cM_rndFX2(float);

bool cM_getRndState(cM_RndStream stream, cM_RndState& output);

#endif /* C_MATH_RNG_H */
