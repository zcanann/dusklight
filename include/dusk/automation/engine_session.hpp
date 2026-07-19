#ifndef DUSK_AUTOMATION_ENGINE_SESSION_HPP
#define DUSK_AUTOMATION_ENGINE_SESSION_HPP

#include <span>
#include <string_view>

namespace dusk::automation {

inline constexpr std::string_view EngineSessionReuseAuditSchema =
    "dusklight-engine-session-reuse-audit/v1";

struct EngineSessionReuseBlocker {
    std::string_view code;
    std::string_view subsystem;
    std::string_view requiredGuarantee;
};

[[nodiscard]] std::span<const EngineSessionReuseBlocker> engine_session_reuse_blockers();

}  // namespace dusk::automation

#endif  // DUSK_AUTOMATION_ENGINE_SESSION_HPP
