#ifndef DUSK_RUNTIME_LIFECYCLE_HPP
#define DUSK_RUNTIME_LIFECYCLE_HPP

#include <string_view>

namespace dusk::runtime {

enum class LifecycleBoundary {
    PreGameRun,
    GameRunActive,
    PostAuthenticatedRun,
    DiagnosticsStopped,
    EmulatedMachineDestroyed,
    HostServicesStopped,
};

enum class GameRunAdmission {
    Admitted,
    RefusedActiveRun,
    RefusedResetUnproved,
};

// Admits the first game run only. A later call fails closed until a complete
// reset implementation explicitly restores the PreGameRun boundary.
[[nodiscard]] GameRunAdmission begin_game_run() noexcept;
[[nodiscard]] LifecycleBoundary lifecycle_boundary() noexcept;
[[nodiscard]] std::string_view lifecycle_boundary_name() noexcept;

// Completes state coupled to one game-loop execution. Host services are still
// alive when this returns, but this boundary is not safe for another run.
void finish_game_run();

// Stops diagnostics that may refer to the emulated machine before its heaps
// are destroyed.
void shutdown_diagnostics();

// Destroys the emulated machine and broadcasts the irreversible OS shutdown
// signal used by its worker threads.
void destroy_emulated_machine();

// Releases process-lifetime host services after the emulated machine is gone.
void shutdown_host_services();

}  // namespace dusk::runtime

#endif  // DUSK_RUNTIME_LIFECYCLE_HPP
