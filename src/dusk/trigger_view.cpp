#include "dusk/trigger_view.hpp"

#include "d/actor/d_a_scene_exit.h"
#include "d/actor/d_a_scene_exit2.h"
#include "d/actor/d_a_npc.h"
#include "d/actor/d_a_tag_event.h"
#include "d/actor/d_a_tag_evt.h"
#include "d/actor/d_a_tag_evtarea.h"
#include "d/d_bg_w.h"
#include "d/d_bg_w_kcol.h"
#include "d/d_com_inf_game.h"
#include "d/d_debug_viewer.h"
#include "dusk/main.h"
#include "dusk/settings.h"
#include "f_op/f_op_actor_iter.h"
#include "f_op/f_op_actor_mng.h"
#include "f_pc/f_pc_name.h"
#include "m_Do/m_Do_mtx.h"

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

    static int event_area_type(const daTag_EvtArea_c& area) {
        const u16 type = area.shape_angle.z & 0xff;
        return type == 0xff ? 0 : type;
    }

    static bool event_area_enabled(const daTag_EvtArea_c& area) {
        if (area.field_0x56c != 0)
            return false;

        const u32 parameters = fopAcM_GetParam(&area);
        const u32 on_event_bit = parameters & 0xfff;
        const u8 on_switch = area.home.angle.x & 0xff;
        return (on_event_bit != 0xfff && daNpcT_chkEvtBit(on_event_bit)) ||
               (on_switch != 0xff && dComIfGs_isSwitch(on_switch, fopAcM_GetRoomNo(&area))) ||
               (on_event_bit == 0xfff && on_switch == 0xff);
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

GXColor event_area_color(const bool enabled, const float opacity) {
    const u8 alpha = alpha_from_percent(opacity * 0.5f);
    return enabled ? GXColor{0xff, 0x00, 0xc8, alpha} : GXColor{0x80, 0x00, 0x64, alpha};
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

void draw_elliptic_cylinder(cXyz base, const cXyz& size, const s16 angle, const GXColor& color) {
    constexpr int kSegments = 16;
    constexpr float kFullTurn = 6.28318530717958647692f;
    Mtx transform;
    cMtx_trans(transform, base.x, base.y, base.z);
    cMtx_YrotM(transform, angle);

    cXyz bottom_center;
    cXyz top_center;
    cXyz local_bottom_center(0.0f, 0.0f, 0.0f);
    cXyz local_top_center(0.0f, std::abs(size.y), 0.0f);
    cMtx_multVec(transform, &local_bottom_center, &bottom_center);
    cMtx_multVec(transform, &local_top_center, &top_center);

    for (int segment = 0; segment < kSegments; ++segment) {
        const float first_angle = kFullTurn * static_cast<float>(segment) / kSegments;
        const float second_angle = kFullTurn * static_cast<float>(segment + 1) / kSegments;
        cXyz local_bottom[2] = {
            cXyz(std::cos(first_angle) * std::abs(size.x), 0.0f,
                std::sin(first_angle) * std::abs(size.z)),
            cXyz(std::cos(second_angle) * std::abs(size.x), 0.0f,
                std::sin(second_angle) * std::abs(size.z)),
        };
        cXyz bottom[2];
        cXyz top[2];
        for (int point = 0; point < 2; ++point) {
            cMtx_multVec(transform, &local_bottom[point], &bottom[point]);
            cXyz local_top = local_bottom[point];
            local_top.y = std::abs(size.y);
            cMtx_multVec(transform, &local_top, &top[point]);
        }

        cXyz side[4] = {bottom[0], bottom[1], top[1], top[0]};
        cXyz bottom_cap[3] = {bottom_center, bottom[1], bottom[0]};
        cXyz top_cap[3] = {top_center, top[0], top[1]};
        dDbVw_drawQuadXlu(side, color, TRUE);
        dDbVw_drawTriangleXlu(bottom_cap, color, TRUE);
        dDbVw_drawTriangleXlu(top_cap, color, TRUE);
    }
}

void draw_event_area(const daTag_EvtArea_c& area, const ActorTriggerDrawContext& context) {
    if (!finite(area.current.pos) || !finite(area.home.pos) || !finite(area.scale))
        return;
    const float extent =
        std::max({std::abs(area.scale.x), std::abs(area.scale.y), std::abs(area.scale.z)});
    if ((area.current.pos - context.player).abs2() >
        (context.settings.drawRange + extent) * (context.settings.drawRange + extent))
    {
        return;
    }

    const int type = TriggerViewReadAdapter::event_area_type(area);
    const GXColor color = event_area_color(
        TriggerViewReadAdapter::event_area_enabled(area), context.settings.opacity);
    if (type == 15 || type == 16) {
        cXyz center(area.home.pos.x, area.current.pos.y + area.scale.y * 0.5f, area.home.pos.z);
        cXyz half_extent(
            std::abs(area.scale.x), std::abs(area.scale.y) * 0.5f, std::abs(area.scale.z));
        csXyz angle(0, area.current.angle.y, 0);
        dDbVw_drawCubeXlu(center, half_extent, angle, color);
        return;
    }

    cXyz base = area.current.pos;
    cXyz size = area.scale;
    base.y -= 10.0f;
    if (type == 21) {
        base.y = context.player.y - context.settings.drawRange;
        size.y = context.settings.drawRange * 2.0f;
    }
    draw_elliptic_cylinder(base, size, area.shape_angle.y, color);
}

bool scripted_event_tag_deleted(const daTag_Evt_c& tag) {
    if (tag.field_0x5E0 != 0xfff || tag.field_0x5E2 != 0xfff) {
        if (tag.field_0x5E0 != 0xfff && !daNpcT_chkEvtBit(tag.field_0x5E0))
            return true;
        return tag.field_0x5E2 != 0xfff && daNpcT_chkEvtBit(tag.field_0x5E2);
    }
    if (tag.field_0x5DD != 0xff || tag.field_0x5DE != 0xff) {
        if (tag.field_0x5DD != 0xff &&
            !dComIfGs_isSwitch(tag.field_0x5DD, fopAcM_GetRoomNo(&tag)))
        {
            return true;
        }
        // Mirrors the native comparison bug: this u8 field is always compared
        // as a real switch number, including 0xff.
        return dComIfGs_isSwitch(tag.field_0x5DE, fopAcM_GetRoomNo(&tag));
    }
    return false;
}

void draw_scripted_event_tag(const daTag_Evt_c& tag, const ActorTriggerDrawContext& context) {
    if (!finite(tag.current.pos) || !finite(tag.scale))
        return;
    const float radius = std::abs(tag.scale.x);
    const float half_height = std::abs(tag.scale.y);
    const float extent = std::max(radius, half_height);
    if ((tag.current.pos - context.player).abs2() >
        (context.settings.drawRange + extent) * (context.settings.drawRange + extent))
    {
        return;
    }

    const bool enabled = (tag.field_0x5E4 == 0 || tag.field_0x5E4 == 1) &&
                         tag.field_0x5D0 == 0 && !scripted_event_tag_deleted(tag);
    const GXColor color = event_area_color(enabled, context.settings.opacity);
    cXyz base = tag.current.pos;
    base.y -= half_height;
    draw_elliptic_cylinder(
        base, cXyz(radius, half_height * 2.0f, radius), 0, color);
}

bool mapped_event_tag_enabled(const daTag_Event_c& tag) {
    if (tag.mAction != daTag_Event_c::ACTION_ARRIVAL && tag.mAction != daTag_Event_c::ACTION_HUNT)
        return false;

    const u32 parameters = fopAcM_GetParam(&tag);
    const u8 switch_number = (parameters >> 8) & 0xff;
    if (switch_number != 0xff && dComIfGs_isSwitch(switch_number, fopAcM_GetRoomNo(&tag)))
        return false;
    const u8 required_switch = (parameters >> 16) & 0xff;
    if (required_switch != 0xff &&
        !dComIfGs_isSwitch(required_switch, fopAcM_GetRoomNo(&tag)))
    {
        return false;
    }

    const u16 invalid_event = tag.home.angle.x & 0x7fff;
    if (invalid_event != 0x7fff && invalid_event != 0 && daNpcT_chkEvtBit(invalid_event))
        return false;
    const u16 required_event = tag.home.angle.z;
    return required_event == 0xffff || required_event == 0 || daNpcT_chkEvtBit(required_event);
}

void draw_mapped_event_tag(const daTag_Event_c& tag, const ActorTriggerDrawContext& context) {
    if (!finite(tag.current.pos) || !finite(tag.scale))
        return;
    const float extent =
        std::max({std::abs(tag.scale.x), std::abs(tag.scale.y), std::abs(tag.scale.z)});
    if ((tag.current.pos - context.player).abs2() >
        (context.settings.drawRange + extent) * (context.settings.drawRange + extent))
    {
        return;
    }

    const GXColor color =
        event_area_color(mapped_event_tag_enabled(tag), context.settings.opacity);
    if ((tag.home.angle.x & 0x8000) != 0) {
        cXyz center = tag.current.pos;
        center.y += tag.scale.y * 0.5f;
        cXyz half_extent(std::abs(tag.scale.x) * 0.5f, std::abs(tag.scale.y) * 0.5f,
            std::abs(tag.scale.z) * 0.5f);
        csXyz angle(0, 0, 0);
        dDbVw_drawCubeXlu(center, half_extent, angle, color);
        return;
    }

    cXyz base = tag.current.pos;
    base.y -= std::abs(tag.scale.y);
    draw_elliptic_cylinder(base,
        cXyz(std::abs(tag.scale.x), std::abs(tag.scale.y) * 2.0f,
            std::abs(tag.scale.x)),
        0, color);
}

int draw_actor_trigger(void* candidate, void* raw_context) {
    const auto* actor = static_cast<const fopAc_ac_c*>(candidate);
    auto& context = *static_cast<ActorTriggerDrawContext*>(raw_context);
    const s16 actor_name = fopAcM_GetName(actor);

    if (context.settings.enableSceneExitView && actor_name == fpcNm_SCENE_EXIT_e) {
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
    } else if (context.settings.enableSceneExitView && actor_name == fpcNm_SCENE_EXIT2_e) {
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
    } else if (context.settings.enableEventAreaView && actor_name == fpcNm_TAG_EVTAREA_e) {
        draw_event_area(*static_cast<const daTag_EvtArea_c*>(actor), context);
    } else if (context.settings.enableEventAreaView && actor_name == fpcNm_TAG_EVT_e) {
        draw_scripted_event_tag(*static_cast<const daTag_Evt_c*>(actor), context);
    } else if (context.settings.enableEventAreaView && actor_name == fpcNm_TAG_EVENT_e) {
        draw_mapped_event_tag(*static_cast<const daTag_Event_c*>(actor), context);
    }
    return 1;
}

}  // namespace

void draw_trigger_view() {
    const TriggerViewSettings& settings = getTransientSettings().triggerView;
    if (!IsGameLaunched || (!settings.enableSceneExitView && !settings.enableEventAreaView) ||
        settings.drawRange <= 0.0f)
        return;
    const fopAc_ac_c* player = dComIfGp_getPlayer(0);
    if (player == nullptr || !finite(player->current.pos))
        return;

    const GXColor enabled_color = trigger_color(true, settings.opacity);
    const GXColor disabled_color = trigger_color(false, settings.opacity);
    if (settings.enableSceneExitView) {
        draw_collision_exit_view(player->current.pos, settings, enabled_color, disabled_color);
    }

    ActorTriggerDrawContext context{player->current.pos, settings, enabled_color, disabled_color};
    fopAcIt_Executor(draw_actor_trigger, &context);
}

}  // namespace dusk
