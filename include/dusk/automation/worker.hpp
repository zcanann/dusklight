#ifndef DUSK_AUTOMATION_WORKER_HPP
#define DUSK_AUTOMATION_WORKER_HPP

#include <optional>

namespace dusk::automation {

inline constexpr unsigned WorkerProtocolVersion = 1;

/**
 * Handles automation-only command-line modes before any game, SDL, renderer,
 * settings, or logging initialization occurs.
 *
 * Returns nullopt for a normal Dusklight launch. Otherwise, returns the exit
 * code for the completed automation mode.
 */
[[nodiscard]] std::optional<int> run_from_command_line(int argc, char* argv[]);

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_WORKER_HPP
