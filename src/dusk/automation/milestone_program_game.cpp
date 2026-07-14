#define MAGIC_ENUM_RANGE_MIN 0
#define MAGIC_ENUM_RANGE_MAX 400
#if defined(__clang__)
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wenum-constexpr-conversion"
#endif
#include <magic_enum.hpp>
#if defined(__clang__)
#pragma clang diagnostic pop
#endif

#include "d/actor/d_a_alink.h"
#include "dusk/automation/milestone_program.hpp"

namespace dusk::automation {

bool resolve_game_milestone_symbol(const MilestoneProgramSymbolKind kind,
    const std::string_view symbol, std::uint32_t& value) {
    if (kind != MilestoneProgramSymbolKind::PlayerProcedure) return false;
    if (symbol == "PROC_PREACTION_UNEQUIP") {
        value = static_cast<std::uint32_t>(daAlink_c::PROC_PREACTION_UNEQUIP);
        return true;
    }
    const auto procedure = magic_enum::enum_cast<daAlink_c::daAlink_PROC>(symbol);
    if (!procedure.has_value()) return false;
    value = static_cast<std::uint32_t>(*procedure);
    return true;
}

}  // namespace dusk::automation
