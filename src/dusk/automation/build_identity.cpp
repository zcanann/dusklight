#include "dusk/automation/build_identity.hpp"

#include "version.h"

namespace dusk::automation {

BuildIdentity current_build_identity() noexcept {
    constexpr std::string_view describe = DUSK_WC_DESCRIBE;

    return {
        .version = DUSK_VERSION_STRING,
        .describe = describe,
        .revision = DUSK_WC_REVISION,
        .branch = DUSK_WC_BRANCH,
        .sourceDate = DUSK_WC_DATE,
        .buildType = DUSK_BUILD_TYPE,
        .platform = DUSK_PLATFORM_NAME,
        .architecture = DUSK_ARCH,
        .pointerBits = static_cast<std::uint32_t>(sizeof(void*) * 8),
        .dirty = describe.ends_with("-dirty"),
    };
}

}  // namespace dusk::automation
