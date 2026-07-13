#pragma once

namespace dusk::game_clock {

enum class MainLoopMode {
    Realtime,
    FixedStep,
};

void ensure_initialized();
void reset_frame_timer();
void set_main_loop_mode(MainLoopMode mode);
MainLoopMode main_loop_mode();

constexpr float sim_pace() { return 1.0f / 30.0f; }
constexpr float period_for_original_frames(float frame_count) { return frame_count * sim_pace(); }
constexpr float ui_maximum_dt() { return 0.05f; }
constexpr float ui_initial_dt() { return 1.0f / 60.0f; }

struct MainLoopPacer {
    float presentation_dt_seconds;
    bool is_interpolating;
    int sim_ticks_to_run;
    float sim_pace;
};

constexpr MainLoopPacer fixed_step_pacer() {
    return {
        .presentation_dt_seconds = sim_pace(),
        .is_interpolating = false,
        .sim_ticks_to_run = 1,
        .sim_pace = sim_pace(),
    };
}

MainLoopPacer advance_main_loop();
void commit_sim_tick();
float sample_interpolation_step();

float consume_interval(const void* consumer);

} // namespace dusk::game_clock
