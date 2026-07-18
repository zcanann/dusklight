#include "dusk/automation/build_identity.hpp"

#include "version.h"

namespace dusk::automation {

BuildIdentity current_build_identity() noexcept {
    constexpr std::string_view describe = DUSK_WC_DESCRIBE;
    constexpr std::string_view dirtyDigest = DUSK_DIRTY_DIGEST;

    return {
        .version = DUSK_VERSION_STRING,
        .describe = describe,
        .revision = DUSK_WC_REVISION,
        .dirtyDigest = dirtyDigest,
        .branch = DUSK_WC_BRANCH,
        .sourceDate = DUSK_WC_DATE,
        .auroraRevision = DUSK_AURORA_REVISION,
        .compiler = DUSK_COMPILER_ID,
        .compilerTarget = DUSK_COMPILER_TARGET,
        .buildType = DUSK_BUILD_TYPE,
        .featureSwitches = DUSK_FEATURE_SWITCHES,
        .featureDigest = DUSK_FEATURE_DIGEST,
        .platform = DUSK_PLATFORM_NAME,
        .architecture = DUSK_ARCH,
        .pointerBits = static_cast<std::uint32_t>(sizeof(void*) * 8),
        .dirty = !dirtyDigest.empty(),
    };
}

}  // namespace dusk::automation
