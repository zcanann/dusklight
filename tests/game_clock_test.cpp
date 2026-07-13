#include "dusk/game_clock.h"

using dusk::game_clock::fixed_step_pacer;
using dusk::game_clock::sim_pace;

static_assert(fixed_step_pacer().presentation_dt_seconds == sim_pace());
static_assert(!fixed_step_pacer().is_interpolating);
static_assert(fixed_step_pacer().sim_ticks_to_run == 1);
static_assert(fixed_step_pacer().sim_pace == sim_pace());

int main() {
    return 0;
}
