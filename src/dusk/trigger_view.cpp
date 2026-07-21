#include "dusk/trigger_view.hpp"

#include "d/d_bg_w.h"
#include "d/d_bg_w_kcol.h"
#include "d/d_com_inf_game.h"
#include "d/d_debug_viewer.h"
#include "dusk/automation/game_state_observer.hpp"
#include "dusk/main.h"
#include "dusk/settings.h"
#include "f_op/f_op_actor_iter.h"
#include "f_op/f_op_actor_mng.h"
#include "m_Do/m_Do_mtx.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>

namespace dusk {

struct TriggerViewReadAdapter {
    static std::size_t kclPrismCount(const dBgWKCol& collision) {
        const KC_Header* header = collision.m_pkc_head;
        if (header == nullptr || header->m_prism_data == nullptr || header->m_block_data == nullptr)
            return 0;
        const KC_PrismData* prisms = header->m_prism_data;
        const BE(u32)* blocks = header->m_block_data;
        const auto prismAddress = reinterpret_cast<std::uintptr_t>(prisms);
        const auto blockAddress = reinterpret_cast<std::uintptr_t>(blocks);
        if (blockAddress <= prismAddress ||
            (blockAddress - prismAddress) % sizeof(KC_PrismData) != 0)
        {
            return 0;
        }
        const std::size_t count = (blockAddress - prismAddress) / sizeof(KC_PrismData);
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

GXColor event_area_color(const bool enabled, const float opacity, const bool wireframe) {
    const u8 alpha = alpha_from_percent(opacity * (wireframe ? 1.0f : 0.5f));
    return enabled ? GXColor{0xff, 0x00, 0xc8, alpha} : GXColor{0x80, 0x00, 0x64, alpha};
}

void draw_triangle(cXyz* points, const GXColor& color, const bool wireframe) {
    if (!wireframe) {
        dDbVw_drawTriangleOpa(points, color, TRUE);
        return;
    }
    dDbVw_drawLineXlu(points[0], points[1], color, TRUE, 2);
    dDbVw_drawLineXlu(points[1], points[2], color, TRUE, 2);
    dDbVw_drawLineXlu(points[2], points[0], color, TRUE, 2);
}

void draw_box(cXyz center, const cXyz& half_extent, const s16 angle, const GXColor& color,
    const bool wireframe) {
    if (!wireframe) {
        csXyz rotation(0, angle, 0);
        cXyz size = half_extent;
        dDbVw_drawCubeXlu(center, size, rotation, color);
        return;
    }

    Mtx transform;
    cMtx_trans(transform, center.x, center.y, center.z);
    cMtx_YrotM(transform, angle);
    const float x = std::abs(half_extent.x);
    const float y = std::abs(half_extent.y);
    const float z = std::abs(half_extent.z);
    cXyz local[8] = {
        cXyz(-x, -y, -z),
        cXyz(x, -y, -z),
        cXyz(x, -y, z),
        cXyz(-x, -y, z),
        cXyz(-x, y, -z),
        cXyz(x, y, -z),
        cXyz(x, y, z),
        cXyz(-x, y, z),
    };
    cXyz points[8];
    for (int point = 0; point < 8; ++point) {
        cMtx_multVec(transform, &local[point], &points[point]);
    }
    constexpr int edges[12][2] = {
        {0, 1},
        {1, 2},
        {2, 3},
        {3, 0},
        {4, 5},
        {5, 6},
        {6, 7},
        {7, 4},
        {0, 4},
        {1, 5},
        {2, 6},
        {3, 7},
    };
    for (const auto& edge : edges) {
        dDbVw_drawLineXlu(points[edge[0]], points[edge[1]], color, TRUE, 2);
    }
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
        return TriggerViewReadAdapter::kclPrismCount(*value);
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
            draw_triangle(points, enabled ? enabled_color : disabled_color, settings.wireframeOnly);
        }
    }
}

struct ActorTriggerDrawContext {
    const cXyz& player;
    const TriggerViewSettings& settings;
    GXColor enabledColor;
    GXColor disabledColor;
};

void draw_elliptic_cylinder(
    cXyz base, const cXyz& size, const s16 angle, const GXColor& color, const bool wireframe) {
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

        if (wireframe) {
            dDbVw_drawLineXlu(bottom[0], bottom[1], color, TRUE, 2);
            dDbVw_drawLineXlu(top[0], top[1], color, TRUE, 2);
            dDbVw_drawLineXlu(bottom[0], top[0], color, TRUE, 2);
        } else {
            cXyz side[4] = {bottom[0], bottom[1], top[1], top[0]};
            cXyz bottom_cap[3] = {bottom_center, bottom[1], bottom[0]};
            cXyz top_cap[3] = {top_center, top[0], top[1]};
            dDbVw_drawQuadXlu(side, color, TRUE);
            dDbVw_drawTriangleXlu(bottom_cap, color, TRUE);
            dDbVw_drawTriangleXlu(top_cap, color, TRUE);
        }
    }
}

int draw_actor_trigger(void* candidate, void* raw_context) {
    const auto* actor = static_cast<const fopAc_ac_c*>(candidate);
    auto& context = *static_cast<ActorTriggerDrawContext*>(raw_context);
    using Actor = automation::MilestoneObservation::Actor;
    Actor::TriggerVolumeComponent trigger;
    if (!automation::capture_actor_trigger_volume(*actor, trigger))
        return 1;

    const bool sceneExit = trigger.kind == Actor::TriggerVolumeKind::SceneExit ||
                           trigger.kind == Actor::TriggerVolumeKind::SceneExitCylinder;
    if ((sceneExit && !context.settings.enableSceneExitView) ||
        (!sceneExit && !context.settings.enableEventAreaView))
    {
        return 1;
    }

    const cXyz center(trigger.centerX, trigger.centerY, trigger.centerZ);
    const cXyz halfExtent(trigger.halfExtentX, trigger.halfExtentY, trigger.halfExtentZ);
    if (!finite(center) || !finite(halfExtent))
        return 1;
    const float extent = std::max({halfExtent.x, halfExtent.y, halfExtent.z});
    const cXyz delta = center - context.player;
    const float distanceSquared =
        trigger.verticalUnbounded ? cXyz(delta.x, 0.0f, delta.z).abs2() : delta.abs2();
    const float visibleRange = context.settings.drawRange + extent;
    if (distanceSquared > visibleRange * visibleRange)
        return 1;

    const GXColor color = sceneExit ?
                              (trigger.enabled ? context.enabledColor : context.disabledColor) :
                              event_area_color(trigger.enabled, context.settings.opacity,
                                  context.settings.wireframeOnly);
    if (trigger.shape == Actor::TriggerVolumeShape::Box) {
        draw_box(center, halfExtent, trigger.yaw, color, context.settings.wireframeOnly);
        return 1;
    }

    cXyz base = center;
    cXyz size(halfExtent.x, halfExtent.y * 2.0f, halfExtent.z);
    if (trigger.verticalUnbounded) {
        base.y = context.player.y - context.settings.drawRange;
        size.y = context.settings.drawRange * 2.0f;
    } else {
        base.y -= halfExtent.y;
    }
    draw_elliptic_cylinder(base, size, trigger.yaw, color, context.settings.wireframeOnly);
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
