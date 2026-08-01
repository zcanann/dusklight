#include "dusk/epona_blocker_view.hpp"

#include "d/actor/d_a_tag_hstop.h"
#include "d/d_bg_w.h"
#include "d/d_bg_w_kcol.h"
#include "d/d_com_inf_game.h"
#include "d/d_debug_viewer.h"
#include "dusk/collision_epona_surface.h"
#include "dusk/main.h"
#include "dusk/settings.h"
#include "m_Do/m_Do_mtx.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>

namespace dusk {

struct EponaBlockerViewReadAdapter {
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

constexpr GXColor kActiveStopVolumeColor = {0xff, 0x00, 0xff, 0xff};
constexpr GXColor kInactiveStopVolumeColor = {0x70, 0x30, 0x70, 0xff};

bool finite(const cXyz& value) {
    return std::isfinite(value.x) && std::isfinite(value.y) && std::isfinite(value.z);
}

u8 alpha_from_percent(const float opacity) {
    const float bounded = std::clamp(opacity, 0.0f, 100.0f);
    return static_cast<u8>(std::lround(bounded * 2.55f));
}

GXColor with_opacity(GXColor color, const float opacity) {
    color.a = alpha_from_percent(opacity);
    return color;
}

GXColor collision_color(const EponaCollisionTint& tint, const float opacity) {
    return GXColor{tint.red, tint.green, tint.blue, alpha_from_percent(opacity)};
}

bool triangle_in_range(const cXyz& player, const cXyz* points, const float range) {
    const cXyz minimum(std::min({points[0].x, points[1].x, points[2].x}),
        std::min({points[0].y, points[1].y, points[2].y}),
        std::min({points[0].z, points[1].z, points[2].z}));
    const cXyz maximum(std::max({points[0].x, points[1].x, points[2].x}),
        std::max({points[0].y, points[1].y, points[2].y}),
        std::max({points[0].z, points[1].z, points[2].z}));
    const float dx = std::max({minimum.x - player.x, 0.0f, player.x - maximum.x});
    const float dy = std::max({minimum.y - player.y, 0.0f, player.y - maximum.y});
    const float dz = std::max({minimum.z - player.z, 0.0f, player.z - maximum.z});
    return dx * dx + dy * dy + dz * dz <= range * range;
}

void draw_triangle(cXyz* points, const GXColor& color, const bool wireframe) {
    if (!wireframe) {
        dDbVw_drawTriangleXlu(points, color, TRUE);
        return;
    }
    dDbVw_drawLineXlu(points[0], points[1], color, TRUE, 2);
    dDbVw_drawLineXlu(points[1], points[2], color, TRUE, 2);
    dDbVw_drawLineXlu(points[2], points[0], color, TRUE, 2);
}

void draw_box(const cXyz& center, const cXyz& halfExtent, const s16 yaw, const GXColor& color,
    const bool wireframe) {
    if (!wireframe) {
        cXyz position = center;
        cXyz size = halfExtent;
        csXyz rotation(0, yaw, 0);
        dDbVw_drawCubeXlu(position, size, rotation, color);
        return;
    }

    Mtx transform;
    cMtx_trans(transform, center.x, center.y, center.z);
    cMtx_YrotM(transform, yaw);
    const float x = std::abs(halfExtent.x);
    const float y = std::abs(halfExtent.y);
    const float z = std::abs(halfExtent.z);
    const cXyz local[8] = {
        cXyz(-x, -y, -z), cXyz(x, -y, -z), cXyz(x, -y, z), cXyz(-x, -y, z),
        cXyz(-x, y, -z), cXyz(x, y, -z), cXyz(x, y, z), cXyz(-x, y, z),
    };
    cXyz points[8];
    for (int point = 0; point < 8; ++point)
        cMtx_multVec(transform, &local[point], &points[point]);

    constexpr int edges[12][2] = {
        {0, 1}, {1, 2}, {2, 3}, {3, 0}, {4, 5}, {5, 6},
        {6, 7}, {7, 4}, {0, 4}, {1, 5}, {2, 6}, {3, 7},
    };
    for (const auto& edge : edges)
        dDbVw_drawLineXlu(points[edge[0]], points[edge[1]], color, TRUE, 2);
}

std::size_t collision_polygon_count(dBgW_Base& collision, bool& kcl) {
    kcl = false;
    if (const auto* value = dynamic_cast<const dBgWKCol*>(&collision); value != nullptr) {
        kcl = true;
        return EponaBlockerViewReadAdapter::kclPrismCount(*value);
    }
    if (const auto* value = dynamic_cast<const cBgW*>(&collision); value != nullptr) {
        const cBgD_t* data = value->GetBgd();
        if (data == nullptr || static_cast<int>(data->m_t_num) < 0)
            return 0;
        return static_cast<std::size_t>(static_cast<int>(data->m_t_num));
    }
    return 0;
}

void draw_collision_blockers(const cXyz& player, const EponaBlockerViewSettings& settings) {
    dBgS& sceneCollision = dComIfG_Bgsp();
    for (int bgIndex = 0; bgIndex < 256; ++bgIndex) {
        cBgS_ChkElm& element = sceneCollision.m_chk_element[bgIndex];
        if (!element.ChkUsed() || element.m_bgw_base_ptr == nullptr)
            continue;

        dBgW_Base& collision = *element.m_bgw_base_ptr;
        bool kcl = false;
        const std::size_t polygonCount = collision_polygon_count(collision, kcl);
        const std::size_t firstPolygon = kcl ? 1 : 0;
        for (std::size_t polygon = firstPolygon; polygon < polygonCount; ++polygon) {
            cBgS_PolyInfo info;
            info.SetActorInfo(bgIndex, &collision, element.m_actor_id);
            info.SetPolyIndex(static_cast<int>(polygon));
            const EponaCollisionTint tint = epona_collision_tint(
                collision.GetHorseNoEntry(info) != 0, collision.GetWallCode(info),
                settings.showCollisionPolygons, settings.showHorseWallPolygons);
            if (!tint.active)
                continue;

            cXyz points[3];
            if (!collision.GetTriPnt(info, &points[0], &points[1], &points[2]) ||
                !finite(points[0]) || !finite(points[1]) || !finite(points[2]) ||
                !triangle_in_range(player, points, settings.drawRange))
            {
                continue;
            }

            const cM3dGPla plane = collision.GetTriPla(info);
            if (finite(plane.mNormal)) {
                const cXyz offset = plane.mNormal * 3.0f;
                points[0] += offset;
                points[1] += offset;
                points[2] += offset;
            }
            draw_triangle(points, collision_color(tint, settings.opacity), settings.wireframeOnly);
        }
    }
}

void draw_stop_volumes(const cXyz& player, const EponaBlockerViewSettings& settings) {
    for (daTagHstop_c* stop = daTagHstop_c::getTop(); stop != nullptr; stop = stop->getNext()) {
        const bool active = stop->getActiveFlg() != 0;
        if (!active && !settings.showInactiveStopVolumes)
            continue;

        // Mounted Link tests the widest vertical interval: local Y -200 through
        // scale.y + 600. Epona's look-ahead test is contained within this box.
        const cXyz halfExtent(std::abs(stop->scale.x), 400.0f + std::abs(stop->scale.y) * 0.5f,
            std::abs(stop->scale.z));
        cXyz center = stop->current.pos;
        center.y += 200.0f + std::abs(stop->scale.y) * 0.5f;
        const cXyz delta = center - player;
        const float visibleRange = settings.drawRange +
                                   std::max({halfExtent.x, halfExtent.y, halfExtent.z});
        if (!finite(center) || !finite(halfExtent) || delta.abs2() > visibleRange * visibleRange)
            continue;

        const GXColor baseColor = active ? kActiveStopVolumeColor : kInactiveStopVolumeColor;
        draw_box(center, halfExtent, stop->shape_angle.y,
            with_opacity(baseColor, settings.opacity), settings.wireframeOnly);
    }
}

}  // namespace

void draw_epona_blocker_view() {
    const EponaBlockerViewSettings& settings = getTransientSettings().eponaBlockerView;
    if (!IsGameLaunched ||
        (!settings.showCollisionPolygons && !settings.showHorseWallPolygons &&
            !settings.showStopVolumes) ||
        settings.drawRange <= 0.0f)
    {
        return;
    }

    const fopAc_ac_c* player = dComIfGp_getPlayer(0);
    if (player == nullptr || !finite(player->current.pos))
        return;

    if (settings.showCollisionPolygons || settings.showHorseWallPolygons)
        draw_collision_blockers(player->current.pos, settings);
    if (settings.showStopVolumes)
        draw_stop_volumes(player->current.pos, settings);
}

}  // namespace dusk
