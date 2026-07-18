#ifndef DUSK_AUTOMATION_BUILD_IDENTITY_HPP
#define DUSK_AUTOMATION_BUILD_IDENTITY_HPP

#include <cstdint>
#include <string_view>

namespace dusk::automation {

/**
 * Build properties that must match before an automation artifact is replayed.
 *
 * The string views point at compile-time version strings and remain valid for
 * the lifetime of the process.
 */
struct BuildIdentity {
    std::string_view version;
    std::string_view describe;
    std::string_view revision;
    std::string_view dirtyDigest;
    std::string_view branch;
    std::string_view sourceDate;
    std::string_view auroraRevision;
    std::string_view compiler;
    std::string_view compilerTarget;
    std::string_view buildType;
    std::string_view featureSwitches;
    std::string_view featureDigest;
    std::string_view fidelityProfile;
    std::string_view platform;
    std::string_view architecture;
    std::uint32_t pointerBits;
    bool dirty;
};

[[nodiscard]] BuildIdentity current_build_identity(std::string_view fidelityProfile) noexcept;

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_BUILD_IDENTITY_HPP
