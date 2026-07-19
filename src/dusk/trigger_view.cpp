#include "dusk/trigger_view.hpp"

#include "d/actor/d_a_scene_exit.h"
#include "d/actor/d_a_scene_exit2.h"
#include "d/d_bg_w.h"
#include "d/d_bg_w_kcol.h"
#include "d/d_com_inf_game.h"
#include "d/d_debug_viewer.h"
#include "dusk/main.h"
#include "dusk/settings.h"
#include "f_op/f_op_actor_iter.h"
#include "f_op/f_op_actor_mng.h"
#include "f_pc/f_pc_name.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>

namespace dusk {

struct TriggerViewReadAdapter {
    static std::size_t kcl_prism_count(const dBgWKCol& collision) {
        const KC_Header* header = collision.m_pkc_head;
        if (header == nullptr)
            return 0;
        const KC_PrismData* prisms = header->m_prism_data;
        const BE(u32)* blocks = header->m_block_data;
        if (prisms == nullptr || blocks == nullptr)
            return 0;
        const auto prism_address = reinterpret_cast<std::uintptr_t>(prisms);
        const auto block_address = reinterpret_cast<std::uintptr_t>(blocks);
        if (block_address <= prism_address ||
            (block_address - prism_address) % sizeof(KC_PrismData) != 0)
        {
            return 0;
        }
        const std::size_t count = (block_address - prism_address) / sizeof(KC_PrismData);
        return count <= std::numeric_limits<u16>::max() ? count : 0;
    }
};

namespace {

constexpr u8 kNoExit = 0x3f;

bool finite(const cXyz& value) {
    return std::isfinite(value.x) && std::isfinite(value.y) && std::isfinite(value.z);
}

u8 alpha_from_percent(const float opacity) {
    const float bounded = std::clamp(opacity, 0.0f, 100.0f);
    return static_cast<u8>(std::lround(bounded * 2.55f));
}

GXColor trigger_color(const bool enabled, const float opacity) {
    const u8 alpha = alpha_from_percent(opacity);
    return enabled ? GXColor{0xff, 0xdc, 0x00, alpha} : GXColor{0xff, 0x80, 0x00, alpha};
}

const stage_scls_info_dummy_class* loaded_scls_for_room(const int room) {
    if (room == -1)
        return g_dComIfG_gameInfo.play.mStageData.getSclsInfo();
    if (room < 0 || room >= 64)
        return nullptr;
    return dStage_roomControl_c::mStatus[room].mRoomDt.getSclsInfo();
}

bool collision_exit_enabled(const int room, const int exit_id) {
    const stage_scls_info_dummy_class* table = loaded_scls_for_room(room);
    return table != nullptr && table->m_entries != nullptr && table->num > 0 && table->num <= 256 &&
           exit_id >= 0 && exit_id < table->num;
}

bool triangle_in_range(const cXyz& player, const cXyz& first, const cXyz& second, const cXyz& third,
    const float range) {
    const cXyz minimum(std::min({first.x, second.x, third.x}),
        std::min({first.y, second.y, third.y}), std::min({first.z, second.z, third.z}));
    const cXyz maximum(std::max({first.x, second.x, third.x}),
        std::max({first.y, second.y, third.y}), std::max({first.z, second.z, third.z}));
    const float dx = std::max({minimum.x - player.x, 0.0f, player.x - maximum.x});
    const float dy = std::max({minimum.y - player.y, 0.0f, player.y - maximum.y});
    const float dz = std::max({minimum.z - player.z, 0.0f, player.z - maximum.z});
    return dx * dx + dy * dy + dz * dz <= range * range;
}

std::size_t collision_polygon_count(dBgW_Base& collision, bool& kcl) {
    kcl = false;
    if (const auto* value = dynamic_cast<const dBgWKCol*>(&collision); value != nullptr) {
        kcl = true;
        return TriggerViewReadAdapter::kcl_prism_count(*value);
    }
    if (const auto* value = dynamic_cast<const cBgW*>(&collision); value != nullptr) {
        const cBgD_t* data = value->GetBgd();
        if (data == nullptr || static_cast<int>(data->m_t_num) < 0)
            return 0;
        return static_cast<std::size_t>(static_cast<int>(data->m_t_num));
    }
    return 0;
}

void draw_collision_exit_view(const cXyz& player, const TriggerViewSettings& settings,
    const GXColor& enabled_color, const GXColor& disabled_color) {
    dBgS& scene_collision = dComIfG_Bgsp();
    for (int bg_index = 0; bg_index < 256; ++bg_index) {
        cBgS_ChkElm& element = scene_collision.m_chk_element[bg_index];
        if (!element.ChkUsed() || element.m_bgw_base_ptr == nullptr)
            continue;

        dBgW_Base& collision = *element.m_bgw_base_ptr;
        bool kcl = false;
        const std::size_t polygon_count = collision_polygon_count(collision, kcl);
        const std::size_t first_polygon = kcl ? 1 : 0;
        for (std::size_t polygon = first_polygon; polygon < polygon_count; ++polygon) {
            cBgS_PolyInfo info;
            info.SetActorInfo(bg_index, &collision, element.m_actor_id);
            info.SetPolyIndex(static_cast<int>(polygon));
            const int exit_id = collision.GetExitId(info);
            if (exit_id < 0 || exit_id == kNoExit)
                continue;

            cXyz points[3];
            if (!collision.GetTriPnt(info, &points[0], &points[1], &points[2]) ||
                !finite(points[0]) || !finite(points[1]) || !finite(points[2]) ||
                !triangle_in_range(player, points[0], points[1], points[2], settings.drawRange))
            {
                continue;
            }

            const cM3dGPla plane = collision.GetTriPla(info);
            if (finite(plane.mNormal)) {
                const cXyz offset = plane.mNormal * 2.0f;
                points[0] += offset;
                points[1] += offset;
                points[2] += offset;
            }
            const bool enabled = collision_exit_enabled(collision.GetGrpRoomIndex(info), exit_id);
            dDbVw_drawTriangleOpa(points, enabled ? enabled_color : disabled_color, TRUE);
        }
    }
}

bool scene_exit_enabled(const daScex_c& exit) {
    const u32 parameters = fopAcM_GetParam(&exit);
    const u8 argument = (parameters >> 8) & 0xff;
    const u8 switch_number = parameters >> 24;
    if (argument == 0xff || argument == 0 || argument == 3) {
        if (fopAcM_isSwitch(&exit, switch_number))
            return false;
    } else if ((argument == 1 || argument == 2 || argument == 4) && switch_number != 0xff) {
        if (!fopAcM_isSwitch(&exit, switch_number))
            return false;
    }

    const u16 off_event_bit = exit.home.angle.z & 0x0fff;
    if (off_event_bit != 0x0fff &&
        dComIfGs_isEventBit(dSv_event_flag_c::saveBitLabels[off_event_bit]))
    {
        return false;
    }
    const u16 on_event_bit = exit.home.angle.x & 0x0fff;
    if (on_event_bit != 0x0fff &&
        !dComIfGs_isEventBit(dSv_event_flag_c::saveBitLabels[on_event_bit]))
    {
        return false;
    }
    return true;
}

struct ActorTriggerDrawContext {
    const cXyz& player;
    const TriggerViewSettings& settings;
    GXColor enabledColor;
    GXColor disabledColor;
};

int draw_actor_trigger(void* candidate, void* raw_context) {
    const auto* actor = static_cast<const fopAc_ac_c*>(candidate);
    auto& context = *static_cast<ActorTriggerDrawContext*>(raw_context);
    const s16 actor_name = fopAcM_GetName(actor);

    if (actor_name == fpcNm_SCENE_EXIT_e) {
        const auto& exit = *static_cast<const daScex_c*>(actor);
        if (!finite(exit.current.pos) || !finite(exit.scale))
            return 1;
        const float extent = std::max({exit.scale.x, exit.scale.y, exit.scale.z, 0.0f});
        if ((exit.current.pos - context.player).abs2() >
            (context.settings.drawRange + extent) * (context.settings.drawRange + extent))
        {
            return 1;
        }
        cXyz center = exit.current.pos;
        center.y += exit.scale.y * 0.5f;
        cXyz half_extent(exit.scale.x, exit.scale.y * 0.5f, exit.scale.z);
        csXyz angle(0, exit.shape_angle.y, 0);
        GXColor color = scene_exit_enabled(exit) ? context.enabledColor : context.disabledColor;
        dDbVw_drawCubeXlu(center, half_extent, angle, color);
    } else if (actor_name == fpcNm_SCENE_EXIT2_e) {
        const auto& exit = *static_cast<const daScExit_c*>(actor);
        if (!finite(exit.current.pos) || !std::isfinite(exit.mRadius) || exit.mRadius < 0.0f)
            return 1;
        const cXyz horizontal_delta(
            exit.current.pos.x - context.player.x, 0.0f, exit.current.pos.z - context.player.z);
        const float visible_radius = context.settings.drawRange + exit.mRadius;
        if (horizontal_delta.abs2() > visible_radius * visible_radius)
            return 1;
        cXyz base(
            exit.current.pos.x, context.player.y - context.settings.drawRange, exit.current.pos.z);
        const bool enabled = exit.mAction == daScExit_c::ACTION_WAIT_e;
        GXColor color = enabled ? context.enabledColor : context.disabledColor;
        dDbVw_drawCylinderXlu(base, exit.mRadius, context.settings.drawRange * 2.0f, color, TRUE);
    }
    return 1;
}

}  // namespace

void draw_trigger_view() {
    const TriggerViewSettings& settings = getTransientSettings().triggerView;
    if (!IsGameLaunched || !settings.enableSceneExitView || settings.drawRange <= 0.0f)
        return;
    const fopAc_ac_c* player = dComIfGp_getPlayer(0);
    if (player == nullptr || !finite(player->current.pos))
        return;

    const GXColor enabled_color = trigger_color(true, settings.opacity);
    const GXColor disabled_color = trigger_color(false, settings.opacity);
    draw_collision_exit_view(player->current.pos, settings, enabled_color, disabled_color);

    ActorTriggerDrawContext context{player->current.pos, settings, enabled_color, disabled_color};
    fopAcIt_Executor(draw_actor_trigger, &context);
}

}  // namespace dusk
