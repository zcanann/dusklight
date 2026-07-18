#include "dusk/automation/game_state_observer.hpp"

#if DUSK_ENABLE_AUTOMATION_OBSERVERS

#include "d/actor/d_a_alink.h"
#include "d/d_camera.h"
#include "d/d_com_inf_game.h"
#include "d/d_s_play.h"
#include "dusk/automation/rng.hpp"
#include "f_op/f_op_actor_iter.h"
#include "f_op/f_op_actor_mng.h"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>

namespace dusk::automation {
namespace {

int capture_controller_actor(void* candidate, void* context) {
    // fopAcIt's original callback ABI is mutable. The candidate is immediately
    // narrowed to const and only already-realized identity/transform POD is
    // copied. No actor method is called.
    const auto* actor = static_cast<const fopAc_ac_c*>(candidate);
    auto* storage = static_cast<ControllerObservationStorage*>(context);
    const std::uint64_t stableId = static_cast<std::uint32_t>(fopAcM_GetID(actor));
    const ControllerActor snapshot{
        .actorName = static_cast<std::int16_t>(fopAcM_GetName(actor)),
        .stableId = stableId,
        .setId = actor->setID,
        .homeRoom = actor->home.roomNo,
        .x = actor->current.pos.x,
        .y = actor->current.pos.y,
        .z = actor->current.pos.z,
    };

    if (storage->count < storage->actors.size()) {
        storage->actors[storage->count++] = snapshot;
        return 1;
    }

    storage->truncated = true;
    // Retention is independent of actor iteration order: keep the lowest
    // process IDs under the fixed observation budget.
    auto largest = std::max_element(storage->actors.begin(), storage->actors.end(),
        [](const auto& left, const auto& right) { return left.stableId < right.stableId; });
    if (stableId < largest->stableId) {
        *largest = snapshot;
    }
    return 1;
}

}  // namespace

bool game_state_observers_enabled() {
    return true;
}

ControllerObservation capture_controller_observation(ControllerObservationStorage& storage) {
    storage = {};
    ControllerObservation observation;
    if (const char* stageName = dComIfGp_getStartStageName(); stageName != nullptr) {
        const std::size_t length = std::min(std::strlen(stageName), observation.stageName.size());
        std::copy_n(stageName, length, observation.stageName.begin());
    }
    if (const fopAc_ac_c* player = dComIfGp_getPlayer(0); player != nullptr) {
        observation.playerPresent = true;
        observation.playerX = player->current.pos.x;
        observation.playerY = player->current.pos.y;
        observation.playerZ = player->current.pos.z;
    }
    if (const camera_process_class* camera = dComIfGp_getCamera(0); camera != nullptr) {
        constexpr float AngleToRadians = 3.14159265358979323846F / 32768.0F;
        const float yaw =
            static_cast<float>(static_cast<std::int16_t>(camera->mCamera.U2())) * AngleToRadians;
        if (std::isfinite(yaw)) {
            observation.cameraPresent = true;
            observation.cameraYawRadians = yaw;
        }
    }

    fopAcIt_Executor(capture_controller_actor, &storage);
    std::sort(storage.actors.begin(), storage.actors.begin() + storage.count,
        [](const auto& left, const auto& right) { return left.stableId < right.stableId; });
    observation.actors = std::span<const ControllerActor>(storage.actors.data(), storage.count);
    return observation;
}

MilestoneObservation capture_milestone_observation() {
    const fopAc_ac_c* player = dComIfGp_getPlayer(0);
    const bool playerIsLink = player != nullptr && fopAcM_GetName(player) == fpcNm_ALINK_e;
    const auto* link = playerIsLink ? static_cast<const daAlink_c*>(player) : nullptr;
    const dEvt_control_c* event = dComIfGp_getEvent();
    return {
        .stageName = dComIfGp_getStartStageName(),
        .room = static_cast<std::int8_t>(dComIfGp_roomControl_getStayNo()),
        .layer = static_cast<std::int8_t>(dComIfG_play_c::getLayerNo(0)),
        .point = dComIfGp_getStartStagePoint(),
        .playerPresent = player != nullptr,
        .playerIsLink = playerIsLink,
        .playerProcessId = player == nullptr ? fpcM_ERROR_PROCESS_ID_e : fopAcM_GetID(player),
        .playerActorName =
            static_cast<std::int16_t>(player == nullptr ? -1 : fopAcM_GetName(player)),
        .playerProcId = static_cast<std::uint16_t>(link == nullptr ? 0xffff : link->mProcID),
        .playerPositionX = player == nullptr ? 0.0F : player->current.pos.x,
        .playerPositionY = player == nullptr ? 0.0F : player->current.pos.y,
        .playerPositionZ = player == nullptr ? 0.0F : player->current.pos.z,
        .playerVelocityX = player == nullptr ? 0.0F : player->speed.x,
        .playerVelocityY = player == nullptr ? 0.0F : player->speed.y,
        .playerVelocityZ = player == nullptr ? 0.0F : player->speed.z,
        .playerForwardSpeed = player == nullptr ? 0.0F : player->speedF,
        .playerCurrentAngleX =
            static_cast<std::int16_t>(player == nullptr ? 0 : player->current.angle.x),
        .playerCurrentAngleY =
            static_cast<std::int16_t>(player == nullptr ? 0 : player->current.angle.y),
        .playerCurrentAngleZ =
            static_cast<std::int16_t>(player == nullptr ? 0 : player->current.angle.z),
        .playerShapeAngleX =
            static_cast<std::int16_t>(player == nullptr ? 0 : player->shape_angle.x),
        .playerShapeAngleY =
            static_cast<std::int16_t>(player == nullptr ? 0 : player->shape_angle.y),
        .playerShapeAngleZ =
            static_cast<std::int16_t>(player == nullptr ? 0 : player->shape_angle.z),
        .eventRunning = dComIfGp_event_runCheck() != 0,
        .eventId = static_cast<std::int16_t>(event == nullptr ? -1 : event->mEventId),
        .eventMode = static_cast<std::uint8_t>(event == nullptr ? 0 : event->getMode()),
        .eventStatus = static_cast<std::uint8_t>(event == nullptr ? 0 : event->mEventStatus),
        .eventMapToolId =
            static_cast<std::uint8_t>(event == nullptr ? 0xff : event->getMapToolId()),
        // Event-name identity is deliberately unavailable. The only existing
        // accessor is non-const and traverses private manager state.
        .eventNameHashPresent = false,
        .eventNameHash = 0,
        .nextStageEnabled = dComIfGp_isEnableNextStage() != 0,
        .nextStageName = dComIfGp_getNextStageName(),
        .nextRoom = static_cast<std::int8_t>(dComIfGp_getNextStageRoomNo()),
        .nextLayer = static_cast<std::int8_t>(dComIfGp_getNextStageLayer()),
        .nextPoint = dComIfGp_getNextStagePoint(),
        .rng = capture_game_rng_snapshot(),
    };
}

EyeShredderGameplayTelemetry capture_eye_shredder_gameplay_telemetry() {
    const fopAc_ac_c* player = dComIfGp_getPlayer(0);
    return {
        .stageName = dComIfGp_getStartStageName(),
        .room = dComIfGp_getStartStageRoomNo(),
        .point = dComIfGp_getStartStagePoint(),
        .layer = dComIfGp_getStartStageLayer(),
        .playerActorName = player == nullptr ? -1 : fopAcM_GetName(player),
        .playerActorPresent = player != nullptr,
        .playerIsLink = player != nullptr && fopAcM_GetName(player) == fpcNm_ALINK_e,
        .eventRunning = dComIfGp_event_runCheck() != 0,
    };
}

}  // namespace dusk::automation

#else

namespace dusk::automation {

bool game_state_observers_enabled() {
    return false;
}

ControllerObservation capture_controller_observation(ControllerObservationStorage& storage) {
    storage = {};
    return {};
}

MilestoneObservation capture_milestone_observation() {
    return {};
}

EyeShredderGameplayTelemetry capture_eye_shredder_gameplay_telemetry() {
    return {};
}

}  // namespace dusk::automation

#endif
