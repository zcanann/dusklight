#include "dusk/automation/engine_session.hpp"

#include <array>

namespace dusk::automation {
namespace {

constexpr std::array Blockers{
    EngineSessionReuseBlocker{
        "automation_state_reset",
        "automation",
        "all per-run paths, flags, players, recorders, observers, and timers return to declared defaults",
    },
    EngineSessionReuseBlocker{
        "dolphin_thread_join",
        "dolphin_threads",
        "DVD, memory-card, audio, and OS worker threads reach joined quiescence and can be recreated",
    },
    EngineSessionReuseBlocker{
        "game_global_reconstruction",
        "game_state",
        "game context, process lists, reset data, and static managers reconstruct from a clean origin",
    },
    EngineSessionReuseBlocker{
        "heap_recreation",
        "game_heaps",
        "JFW and JKR heaps plus ARAM are destroyed without live references and recreated from a valid arena",
    },
    EngineSessionReuseBlocker{
        "mod_lifecycle",
        "mods",
        "native mod hooks and loader registrations survive or repeat initialization without duplication",
    },
    EngineSessionReuseBlocker{
        "process_run_lifecycle_partition",
        "lifecycle",
        "Aurora, DVD hosting, logging, and process services outlive only the game-run state",
    },
};

}  // namespace

std::span<const EngineSessionReuseBlocker> engine_session_reuse_blockers() {
    return Blockers;
}

}  // namespace dusk::automation
