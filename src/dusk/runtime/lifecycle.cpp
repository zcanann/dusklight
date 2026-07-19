#include "dusk/runtime/lifecycle.hpp"

#include <aurora/main.h>
#include <dolphin/os.h>

#include <cstdio>
#include <cstdlib>

#include "d/actor/d_a_movie_player.h"
#include "dusk/config.hpp"
#include "dusk/crash_reporting.h"
#include "dusk/discord_presence.hpp"
#include "dusk/logging.h"
#include "dusk/mod_loader.hpp"
#include "dusk/texture_replacements.hpp"
#include "dusk/ui/ui.hpp"
#include "m_Do/m_Do_machine.h"

namespace dusk::runtime {
namespace {

LifecycleBoundary CurrentBoundary = LifecycleBoundary::PreGameRun;

void require_boundary(LifecycleBoundary expected) {
    if (CurrentBoundary != expected) {
        std::abort();
    }
}

}  // namespace

GameRunAdmission begin_game_run() noexcept {
    switch (CurrentBoundary) {
    case LifecycleBoundary::PreGameRun:
        CurrentBoundary = LifecycleBoundary::GameRunActive;
        return GameRunAdmission::Admitted;
    case LifecycleBoundary::GameRunActive:
        return GameRunAdmission::RefusedActiveRun;
    default:
        return GameRunAdmission::RefusedResetUnproved;
    }
}

LifecycleBoundary lifecycle_boundary() noexcept {
    return CurrentBoundary;
}

std::string_view lifecycle_boundary_name() noexcept {
    switch (CurrentBoundary) {
    case LifecycleBoundary::PreGameRun:
        return "pre_game_run";
    case LifecycleBoundary::GameRunActive:
        return "game_run_active";
    case LifecycleBoundary::PostAuthenticatedRun:
        return "post_authenticated_run";
    case LifecycleBoundary::DiagnosticsStopped:
        return "diagnostics_stopped";
    case LifecycleBoundary::EmulatedMachineDestroyed:
        return "emulated_machine_destroyed";
    case LifecycleBoundary::HostServicesStopped:
        return "host_services_stopped";
    }
    std::abort();
}

void finish_game_run() {
    require_boundary(LifecycleBoundary::GameRunActive);
    mods::ModLoader::instance().shutdown();
    ui::shutdown();
    MoviePlayerShutdown();
    CurrentBoundary = LifecycleBoundary::PostAuthenticatedRun;
}

void shutdown_diagnostics() {
    require_boundary(LifecycleBoundary::PostAuthenticatedRun);
    crash_reporting::shutdown();
    ShutdownFileLogging();
    std::fflush(stdout);
    std::fflush(stderr);
    CurrentBoundary = LifecycleBoundary::DiagnosticsStopped;
}

void destroy_emulated_machine() {
    require_boundary(LifecycleBoundary::DiagnosticsStopped);
    mDoMch_Destroy();

    // Notifies all condition variables and causes worker threads to exit. The
    // current host implementation cannot reverse this signal or join/recreate
    // every affected thread, so another game run must be refused after here.
    OSResetSystem(OS_RESET_SHUTDOWN, 0, 0);
    CurrentBoundary = LifecycleBoundary::EmulatedMachineDestroyed;
}

void shutdown_host_services() {
    require_boundary(LifecycleBoundary::EmulatedMachineDestroyed);
#ifdef DUSK_DISCORD
    discord::shutdown();
#endif
    ui::shutdown();
    texture_replacements::shutdown();
    config::shutdown();
    aurora_shutdown();
    CurrentBoundary = LifecycleBoundary::HostServicesStopped;
}

}  // namespace dusk::runtime
