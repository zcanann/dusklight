#include "dusk/automation/scenario_fixture_runtime.hpp"

#include "SSystem/SComponent/c_math_rng.h"
#include "d/d_com_inf_game.h"
#include "d/d_s_play.h"
#include "d/actor/d_a_player.h"
#include "dusk/config.hpp"
#include "m_Do/m_Do_machine.h"

#include <algorithm>
#include <bit>
#include <charconv>
#include <cmath>
#include <limits>
#include <optional>
#include <string>
#include <type_traits>

namespace dusk::automation {
namespace {

std::optional<ScenarioFixture> g_fixture;
std::int8_t g_bootRoom = -1;
std::string g_error;
bool g_startupApplied = false;
bool g_saveStateApplied = false;
bool g_stageFlagsApplied = false;
bool g_roomFlagsApplied = false;

bool fail(std::string message) {
    g_error = std::move(message);
    return false;
}

bool validates_native_limits(const ScenarioFixture& fixture, const std::int8_t bootRoom) {
    if (validate_scenario_fixture(fixture) != ScenarioFixtureError::None) {
        return fail("scenario fixture failed canonical validation");
    }
    for (const auto& entry : fixture.inventory) {
        if (entry.slot >= 24 || entry.item > std::numeric_limits<std::uint8_t>::max() ||
            entry.quantity > std::numeric_limits<std::uint8_t>::max()) {
            return fail("inventory slot, item, or quantity exceeds native save limits");
        }
        const bool hasQuantityStore = entry.slot == SLOT_4 ||
                                      (entry.slot >= SLOT_11 && entry.slot <= SLOT_17) ||
                                      entry.slot == SLOT_23;
        if (entry.quantity != 1 && !hasQuantityStore) {
            return fail("inventory quantity is only defined for bow, bottle, bomb-bag, and slingshot slots");
        }
    }
    for (const auto& entry : fixture.equipment) {
        if (entry.slot >= MAX_EQUIPMENT || entry.item > std::numeric_limits<std::uint8_t>::max()) {
            return fail("equipment slot or item exceeds native save limits");
        }
    }
    for (const auto& flag : fixture.flags) {
        switch (flag.domain) {
        case FixtureFlagDomain::Event:
            if (flag.room != -1 || flag.index >= 822) {
                return fail("event flags require room -1 and a label index below 822");
            }
            break;
        case FixtureFlagDomain::Temporary:
            if (flag.room != -1 || flag.index >= 185) {
                return fail("temporary flags require room -1 and a label index below 185");
            }
            break;
        case FixtureFlagDomain::Dungeon:
            if (flag.room != -1 || flag.index >= dSv_info_c::DAN_SWITCH) {
                return fail("dungeon flags require room -1 and a switch index below 64");
            }
            break;
        case FixtureFlagDomain::Switch:
            if (flag.room != bootRoom || flag.index >=
                    dSv_info_c::MEMORY_SWITCH + dSv_info_c::DAN_SWITCH +
                        dSv_info_c::ZONE_SWITCH + dSv_info_c::ONEZONE_SWITCH) {
                return fail("switch flags must target the boot room and use an index below 240");
            }
            break;
        }
    }
#if VERSION != VERSION_GCN_PAL
    if (fixture.videoMode == FixtureVideoMode::Pal50 ||
        fixture.videoMode == FixtureVideoMode::Pal60) {
        return fail("PAL fixture video modes require a PAL game build");
    }
#else
    if (fixture.videoMode == FixtureVideoMode::NtscInterlaced ||
        fixture.videoMode == FixtureVideoMode::NtscProgressive) {
        return fail("NTSC fixture video modes require an NTSC game build");
    }
#endif
    return true;
}

std::string setting_argument(const FixtureSettingValue& value) {
    return std::visit(
        [](const auto& typed) -> std::string {
            using T = std::decay_t<decltype(typed)>;
            if constexpr (std::is_same_v<T, bool>) {
                return typed ? "true" : "false";
            } else if constexpr (std::is_same_v<T, std::int64_t>) {
                return std::to_string(typed);
            } else if constexpr (std::is_same_v<T, FixtureFloat>) {
                const double decoded = std::bit_cast<double>(typed.bits);
                char buffer[64];
                const auto result = std::to_chars(buffer, buffer + sizeof(buffer), decoded,
                    std::chars_format::general, std::numeric_limits<double>::max_digits10);
                return result.ec == std::errc{} ? std::string(buffer, result.ptr) : std::string{};
            } else {
                return typed;
            }
        },
        value);
}

void apply_health_inventory_equipment(const ScenarioFixture& fixture) {
    if (fixture.form) {
        dComIfGs_setTransformStatus(*fixture.form == PlayerFixtureForm::Wolf ?
                                        TF_STATUS_WOLF : TF_STATUS_HUMAN);
    }
    if (fixture.health) {
        dComIfGs_getSaveInfo()->getPlayer().getPlayerStatusA().setMaxLife(
            fixture.health->maximum);
        dComIfGs_setLife(fixture.health->current);
    }
    for (const auto& entry : fixture.inventory) {
        dComIfGs_setItem(entry.slot, static_cast<std::uint8_t>(entry.item));
        if (entry.item != dItemNo_NONE_e) {
            dComIfGs_onItemFirstBit(static_cast<std::uint8_t>(entry.item));
        }
        if (entry.slot == SLOT_4) {
            dComIfGs_setArrowNum(static_cast<std::uint8_t>(entry.quantity));
        } else if (entry.slot >= SLOT_11 && entry.slot <= SLOT_14) {
            dComIfGs_setBottleNum(static_cast<std::uint8_t>(entry.slot - SLOT_11),
                static_cast<std::uint8_t>(entry.quantity));
        } else if (entry.slot >= SLOT_15 && entry.slot <= SLOT_17) {
            dComIfGs_setBombNum(static_cast<std::uint8_t>(entry.slot - SLOT_15),
                static_cast<std::uint8_t>(entry.quantity));
        } else if (entry.slot == SLOT_23) {
            dComIfGs_setPachinkoNum(static_cast<std::uint8_t>(entry.quantity));
        }
    }
    auto& status = dComIfGs_getSaveInfo()->getPlayer().getPlayerStatusA();
    for (const auto& entry : fixture.equipment) {
        const auto item = static_cast<std::uint8_t>(entry.item);
        switch (entry.slot) {
        case COLLECT_CLOTHING:
            dComIfGs_setSelectEquipClothes(item);
            break;
        case COLLECT_SWORD:
            dComIfGs_setSelectEquipSword(item);
            break;
        case COLLECT_SHIELD:
            dComIfGs_setSelectEquipShield(item);
            break;
        default:
            status.setSelectEquip(entry.slot, item);
            break;
        }
    }
}

void apply_global_flags(const ScenarioFixture& fixture) {
    for (const auto& flag : fixture.flags) {
        const auto set = flag.value;
        if (flag.domain == FixtureFlagDomain::Event) {
            const auto native = dSv_event_flag_c::saveBitLabels[flag.index];
            set ? dComIfGs_onEventBit(native) : dComIfGs_offEventBit(native);
        } else if (flag.domain == FixtureFlagDomain::Temporary) {
            const auto native = dSv_event_tmp_flag_c::tempBitLabels[flag.index];
            set ? dComIfGs_onTmpBit(native) : dComIfGs_offTmpBit(native);
        }
    }
}

void apply_dungeon_flags(const ScenarioFixture& fixture) {
    for (const auto& flag : fixture.flags) {
        if (flag.domain == FixtureFlagDomain::Dungeon) {
            flag.value ? dComIfGs_onSaveDunSwitch(flag.index) :
                         dComIfGs_offSaveDunSwitch(flag.index);
        }
    }
}

void apply_switch_flags(const ScenarioFixture& fixture) {
    for (const auto& flag : fixture.flags) {
        if (flag.domain == FixtureFlagDomain::Switch) {
            flag.value ? dComIfGs_onSwitch(flag.index, flag.room) :
                         dComIfGs_offSwitch(flag.index, flag.room);
        }
    }
}

std::uint8_t inventory_quantity(const InventoryFixture& entry) {
    if (entry.slot == SLOT_4) return dComIfGs_getArrowNum();
    if (entry.slot >= SLOT_11 && entry.slot <= SLOT_14) {
        return dComIfGs_getBottleNum(static_cast<std::uint8_t>(entry.slot - SLOT_11));
    }
    if (entry.slot >= SLOT_15 && entry.slot <= SLOT_17) {
        return dComIfGs_getBombNum(static_cast<std::uint8_t>(entry.slot - SLOT_15));
    }
    if (entry.slot == SLOT_23) return dComIfGs_getPachinkoNum();
    return 1;
}

bool verify_tick_zero(const ScenarioFixture& fixture) {
    if (fixture.form) {
        const bool requestedWolf = *fixture.form == PlayerFixtureForm::Wolf;
        if ((dComIfGs_getTransformStatus() == TF_STATUS_WOLF) != requestedWolf ||
            static_cast<bool>(daPy_py_c::checkNowWolf()) != requestedWolf) {
            return fail("player form did not survive stage actor initialization");
        }
    }
    if (fixture.health && (dComIfGs_getLife() != fixture.health->current ||
                              dComIfGs_getMaxLife() != fixture.health->maximum)) {
        return fail("health did not match the fixture at tick zero");
    }
    for (const auto& entry : fixture.inventory) {
        if (dComIfGs_getItem(entry.slot, false) != entry.item ||
            inventory_quantity(entry) != entry.quantity) {
            return fail("inventory did not match the fixture at tick zero");
        }
    }
    for (const auto& entry : fixture.equipment) {
        if (dComIfGs_getSaveInfo()->getPlayer().getPlayerStatusA().getSelectEquip(entry.slot) !=
            entry.item) {
            return fail("equipment did not match the fixture at tick zero");
        }
    }
    for (const auto& flag : fixture.flags) {
        bool value = false;
        switch (flag.domain) {
        case FixtureFlagDomain::Event:
            value = dComIfGs_isEventBit(dSv_event_flag_c::saveBitLabels[flag.index]);
            break;
        case FixtureFlagDomain::Temporary:
            value = dComIfGs_isTmpBit(dSv_event_tmp_flag_c::tempBitLabels[flag.index]);
            break;
        case FixtureFlagDomain::Dungeon:
            value = dComIfGs_isSaveDunSwitch(flag.index);
            break;
        case FixtureFlagDomain::Switch:
            value = dComIfGs_isSwitch(flag.index, flag.room);
            break;
        }
        if (value != flag.value) return fail("flag did not match the fixture at tick zero");
    }
    return true;
}

}  // namespace

StageBootReadinessObservation capture_stage_boot_readiness() {
    StageBootReadinessObservation observation;
    if (const char* stage = dComIfGp_getStartStageName(); stage != nullptr) {
        std::size_t length = 0;
        while (length < 9 && stage[length] != '\0') ++length;
        if (length > 0 && length <= 8) {
            std::copy_n(stage, length, observation.stage.begin());
            observation.stagePresent = true;
        }
    }
    observation.room = static_cast<std::int8_t>(dComIfGp_roomControl_getStayNo());
    observation.layer = static_cast<std::int8_t>(dComIfG_play_c::getLayerNo(0));
    observation.point = dComIfGp_getStartStagePoint();
    observation.playerReady = dComIfGp_getPlayer(0) != nullptr;
    return observation;
}

bool install_scenario_fixture_runtime(
    const std::optional<ScenarioFixture>& fixture, const std::int8_t bootRoom) {
    clear_scenario_fixture_runtime();
    if (!fixture) return true;
    if (!validates_native_limits(*fixture, bootRoom)) return false;
    g_fixture = fixture;
    g_bootRoom = bootRoom;
    return true;
}

void clear_scenario_fixture_runtime() {
    g_fixture.reset();
    g_bootRoom = -1;
    g_error.clear();
    g_startupApplied = false;
    g_saveStateApplied = false;
    g_stageFlagsApplied = false;
    g_roomFlagsApplied = false;
}

std::string_view scenario_fixture_runtime_error() { return g_error; }

bool apply_scenario_fixture_startup() {
    if (!g_fixture || g_startupApplied) return true;
    for (const auto& setting : g_fixture->settings) {
        auto* variable = dusk::config::GetConfigVar(setting.key);
        if (variable == nullptr) return fail("fixture setting names an unknown configuration variable: " + setting.key);
        const auto argument = setting_argument(setting.value);
        if (argument.empty() && !std::holds_alternative<std::string>(setting.value)) {
            return fail("fixture setting could not be represented as a launch override: " + setting.key);
        }
        try {
            variable->getImpl()->loadFromArg(*variable, argument);
        } catch (const std::exception& error) {
            return fail("fixture setting '" + setting.key + "' is invalid: " + error.what());
        }
    }
    if (g_fixture->videoMode) {
        switch (*g_fixture->videoMode) {
        case FixtureVideoMode::Automatic:
            break;
        case FixtureVideoMode::NtscInterlaced:
        case FixtureVideoMode::Pal50:
            mDoMch_render_c::setRenderModeObj(&g_ntscZeldaIntDf);
            break;
        case FixtureVideoMode::NtscProgressive:
        case FixtureVideoMode::Pal60:
            mDoMch_render_c::setRenderModeObj(&g_ntscZeldaProg);
            break;
        }
    }
    g_startupApplied = true;
    return true;
}

bool apply_scenario_fixture_save_state() {
    if (!g_fixture || g_saveStateApplied) return true;
    apply_health_inventory_equipment(*g_fixture);
    apply_global_flags(*g_fixture);
    g_saveStateApplied = true;
    return true;
}

bool apply_scenario_fixture_stage_flags() {
    if (!g_fixture || g_stageFlagsApplied) return true;
    apply_dungeon_flags(*g_fixture);
    g_stageFlagsApplied = true;
    return true;
}

bool apply_scenario_fixture_room_flags(const std::int8_t room) {
    if (!g_fixture || g_roomFlagsApplied || room != g_bootRoom) return true;
    apply_switch_flags(*g_fixture);
    g_roomFlagsApplied = true;
    return true;
}

bool establish_scenario_fixture_tick_zero() {
    if (!g_fixture) return true;
    if (!g_startupApplied || !g_saveStateApplied || !g_stageFlagsApplied || !g_roomFlagsApplied) {
        return fail("scenario fixture did not complete every native boot phase");
    }
    // Establish exact mutable save state at the replay boundary after all loading ticks.
    apply_health_inventory_equipment(*g_fixture);
    apply_global_flags(*g_fixture);
    apply_dungeon_flags(*g_fixture);
    apply_switch_flags(*g_fixture);
    for (const auto& entry : g_fixture->rng) {
        const cM_RndState state{
            .version = cM_RndStateVersion,
            .state0 = entry.state0,
            .state1 = entry.state1,
            .state2 = entry.state2,
            .callCount = entry.callCount,
        };
        const auto stream = entry.stream == FixtureRngStream::Primary ?
                                cM_RndStream::Primary : cM_RndStream::Secondary;
        if (!cM_setRndState(stream, state)) return fail("failed to restore fixture RNG stream");
    }
    if (!verify_tick_zero(*g_fixture)) return false;
    for (const auto& entry : g_fixture->rng) {
        cM_RndState observed;
        const auto stream = entry.stream == FixtureRngStream::Primary ?
                                cM_RndStream::Primary : cM_RndStream::Secondary;
        if (!cM_getRndState(stream, observed) || observed.state0 != entry.state0 ||
            observed.state1 != entry.state1 || observed.state2 != entry.state2 ||
            observed.callCount != entry.callCount) {
            return fail("RNG did not match the fixture at tick zero");
        }
    }
    return true;
}

}  // namespace dusk::automation
