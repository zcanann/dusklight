#ifndef DUSK_AUTOMATION_WORKER_HPP
#define DUSK_AUTOMATION_WORKER_HPP

#include <cstdint>
#include <filesystem>
#include <optional>
#include <string>
#include <string_view>

namespace dusk::automation {

inline constexpr unsigned WorkerProtocolVersion = 2;

enum class EngineWorkerCommandKind {
    RunBatch,
    Shutdown,
};

struct EngineWorkerCommand {
    EngineWorkerCommandKind kind = EngineWorkerCommandKind::Shutdown;
    std::uint64_t requestId = 0;
    std::filesystem::path batchPath;
    std::filesystem::path resultPath;
    std::filesystem::path winnerTapePath;
};

/**
 * Handles automation-only command-line modes before any game, SDL, renderer,
 * settings, or logging initialization occurs.
 *
 * Returns nullopt for a normal Dusklight launch. Otherwise, returns the exit
 * code for the completed automation mode.
 */
[[nodiscard]] std::optional<int> run_from_command_line(int argc, char* argv[]);

/** True only for the game-hosting persistent suffix worker mode. */
[[nodiscard]] bool engine_worker_enabled();
/** Blocks only between completed batches; simulation never polls stdin. */
[[nodiscard]] bool wait_for_engine_worker_command(
    EngineWorkerCommand& command, std::string& error);
void publish_engine_worker_batch_complete(std::uint64_t requestId,
    const std::filesystem::path& resultPath, const std::filesystem::path& episodeShardPath);
void reject_engine_worker_batch(std::uint64_t requestId, std::string_view message);

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_WORKER_HPP
