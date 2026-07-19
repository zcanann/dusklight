#ifndef DUSK_AUTOMATION_PHASE_TIMING_HPP
#define DUSK_AUTOMATION_PHASE_TIMING_HPP

#include <cstdint>
#include <filesystem>
#include <string>

namespace dusk::automation {

enum class NativeLifecyclePhase : std::uint8_t {
    ProcessEntry,
    CliConfigured,
    AuroraInitialized,
    EngineReady,
    StageReady,
    FirstSimulationTick,
    LastSimulationTick,
    ProofArtifactsWritten,
    EngineShutdown,
    ExitReady,
    Count,
};

void begin_native_lifecycle_timing();
void mark_native_lifecycle_phase(NativeLifecyclePhase phase);
void update_native_lifecycle_phase(NativeLifecyclePhase phase);

[[nodiscard]] bool write_native_lifecycle_timing(
    const std::filesystem::path& path, std::string& error);

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_PHASE_TIMING_HPP
