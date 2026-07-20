#include "dusk/automation/game_state_observer.hpp"

#if DUSK_ENABLE_AUTOMATION_OBSERVERS

#include "d/actor/d_a_alink.h"
#include "d/actor/d_a_title.h"
#include "d/d_camera.h"
#include "d/d_com_inf_game.h"
#include "d/d_s_play.h"
#include "d/d_s_name.h"
#include "dusk/automation/file_select_observer.hpp"
#include "dusk/automation/menu_state_observer.hpp"
#include "dusk/automation/name_entry_observer.hpp"
#include "dusk/automation/rng.hpp"
#include "f_op/f_op_actor_iter.h"
#include "f_op/f_op_actor_mng.h"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>

namespace dusk::automation {

TitleMenuObservation MenuStateObserver::captureTitle() {
    const auto* title = static_cast<const daTitle_c*>(fopAcM_SearchByName(fpcNm_TITLE_e));
    if (title == nullptr) return {};
    return {
        .present = true,
        .procedure = title->mProcID,
        .logoSkipReady = title->mProcID == 1,
        .startReady = title->mProcID == 3,
    };
}

NameSceneMenuObservation MenuStateObserver::captureNameScene() {
    const auto* scene = reinterpret_cast<const dScnName_c*>(
        fpcM_SearchByName(fpcNm_NAME_SCENE_e));
    if (scene == nullptr) return {};
    const dFile_select_c* fileSelect = scene->dFs_c;
    return {
        .present = true,
        .procedure = scene->mProc,
        .fileSelectPresent = fileSelect != nullptr,
        .fileSelectProcedure =
            static_cast<std::uint8_t>(fileSelect == nullptr ? 0xff : fileSelect->mDataSelProc),
        .cardCheckProcedure =
            static_cast<std::uint8_t>(fileSelect == nullptr ? 0xff : fileSelect->mCardCheckProc),
    };
}

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

int capture_milestone_actor(void* candidate, void* context) {
    const auto* actor = static_cast<const fopAc_ac_c*>(candidate);
    auto* storage = static_cast<MilestoneObservationStorage*>(context);
    if (storage->actorObservedCount != std::numeric_limits<std::uint32_t>::max())
        ++storage->actorObservedCount;
    const std::uint64_t runtimeGeneration =
        static_cast<std::uint32_t>(fopAcM_GetID(actor));
    const MilestoneObservation::Actor snapshot{
        .runtimeGeneration = runtimeGeneration,
        .actorName = static_cast<std::int16_t>(fopAcM_GetName(actor)),
        .setId = actor->setID,
        .homeRoom = actor->home.roomNo,
        .currentRoom = actor->current.roomNo,
        .positionX = actor->current.pos.x,
        .positionY = actor->current.pos.y,
        .positionZ = actor->current.pos.z,
        .health = actor->health,
        .status = actor->actor_status,
        .parentRuntimeGeneration = static_cast<std::uint32_t>(actor->parentActorID),
        .parameters = fopAcM_GetParam(actor),
        .profileName = static_cast<std::int16_t>(fopAcM_GetProfName(actor)),
        .group = actor->group,
        .argument = actor->argument,
        .homePositionX = actor->home.pos.x,
        .homePositionY = actor->home.pos.y,
        .homePositionZ = actor->home.pos.z,
        .velocityX = actor->speed.x,
        .velocityY = actor->speed.y,
        .velocityZ = actor->speed.z,
        .forwardSpeed = actor->speedF,
        .currentAngleX = actor->current.angle.x,
        .currentAngleY = actor->current.angle.y,
        .currentAngleZ = actor->current.angle.z,
        .shapeAngleX = actor->shape_angle.x,
        .shapeAngleY = actor->shape_angle.y,
        .shapeAngleZ = actor->shape_angle.z,
    };
    storage->actors.push_back(snapshot);
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
        observation.playerIsLink = fopAcM_GetName(player) == fpcNm_ALINK_e;
        observation.playerX = player->current.pos.x;
        observation.playerY = player->current.pos.y;
        observation.playerZ = player->current.pos.z;
        constexpr float AngleToRadians = 3.14159265358979323846F / 32768.0F;
        const float yaw = static_cast<float>(static_cast<std::int16_t>(player->current.angle.y)) *
                          AngleToRadians;
        if (std::isfinite(yaw)) {
            observation.playerYawPresent = true;
            observation.playerYawRadians = yaw;
        }
        if (std::isfinite(player->speed.x) && std::isfinite(player->speed.z)) {
            observation.playerVelocityPresent = true;
            observation.playerVelocityX = player->speed.x;
            observation.playerVelocityZ = player->speed.z;
        }
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
    observation.actorsTruncated = storage.truncated;
    return observation;
}

MilestoneObservation capture_milestone_observation(MilestoneObservationStorage& storage) {
    // Preserve vector capacity across per-frame captures. This keeps complete
    // actor capture allocation-free after the population high-water mark.
    storage.actors.clear();
    storage.actorObservedCount = 0;
    const fopAc_ac_c* player = dComIfGp_getPlayer(0);
    const bool playerIsLink = player != nullptr && fopAcM_GetName(player) == fpcNm_ALINK_e;
    const auto* link = playerIsLink ? static_cast<const daAlink_c*>(player) : nullptr;
    const dEvt_control_c* event = dComIfGp_getEvent();
    const TitleMenuObservation titleMenu = MenuStateObserver::captureTitle();
    const NameSceneMenuObservation nameScene = MenuStateObserver::captureNameScene();
    const NameEntryObservation& nameEntry = name_entry_observer().latest();
    const bool nameEntryActive = nameEntry.active != 0;
    const bool nameEntryInputReady = nameEntryActive && name_entry_observer().inputProcessed();
    const FileSelectObserver& fileSelect = file_select_observer();
    const auto actorIdentity = [](const fopAc_ac_c* actor) {
        MilestoneObservation::ActorIdentity identity;
        if (actor != nullptr) {
            identity.present = true;
            identity.runtimeGeneration = static_cast<std::uint32_t>(fopAcM_GetID(actor));
            identity.actorName = static_cast<std::int16_t>(fopAcM_GetName(actor));
            identity.setId = static_cast<std::uint16_t>(fopAcM_GetSetId(actor));
            identity.homeRoom = actor->home.roomNo;
            identity.currentRoom = actor->current.roomNo;
            identity.homePositionPresent = true;
            identity.homePositionX = actor->home.pos.x;
            identity.homePositionY = actor->home.pos.y;
            identity.homePositionZ = actor->home.pos.z;
        }
        return identity;
    };
    const MilestoneObservation::ActorIdentity talkPartner = actorIdentity(
        link == nullptr ? nullptr : fopAcM_getTalkEventPartner(link));
    const fpc_ProcID grabbedId =
        link == nullptr ? fpcM_ERROR_PROCESS_ID_e : link->getGrabActorID();
    const MilestoneObservation::ActorIdentity grabbedActor = actorIdentity(
        grabbedId == fpcM_ERROR_PROCESS_ID_e ? nullptr : fopAcM_SearchByID(grabbedId));
    MilestoneObservation observation{
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
        .playerModeFlags = link == nullptr ? 0 : link->mModeFlg,
        .playerDamageWaitTimer = static_cast<std::int16_t>(
            link == nullptr ? 0 : link->getDamageWaitTimer()),
        .playerIceDamageWaitTimer = static_cast<std::int16_t>(
            link == nullptr ? 0 : link->getIceDamageWaitTimer()),
        .playerSwordChangeWaitTimer = static_cast<std::uint8_t>(
            link == nullptr ? 0 : link->getSwordChangeWaitTimer()),
        .playerDoStatus = static_cast<std::uint8_t>(
            link == nullptr ? 0 : dComIfGp_getDoStatus()),
        .talkPartner = talkPartner,
        .grabbedActor = grabbedActor,
        .playerGroundContact = link != nullptr && link->mLinkAcch.ChkGroundHit(),
        .playerWallContact = link != nullptr && link->mLinkAcch.ChkWallHit() != 0,
        .playerRoofContact = link != nullptr && link->mLinkAcch.ChkRoofHit(),
        .playerWaterContact = link != nullptr && link->mLinkAcch.ChkWaterHit(),
        .playerWaterIn = link != nullptr && link->mLinkAcch.ChkWaterIn(),
        .playerGroundHeightPresent = link != nullptr &&
            std::isfinite(link->mLinkAcch.GetGroundH()) && link->mLinkAcch.GetGroundH() != -G_CM3D_F_INF,
        .playerRoofHeightPresent = link != nullptr &&
            std::isfinite(link->mLinkAcch.GetRoofHeight()) && link->mLinkAcch.GetRoofHeight() != G_CM3D_F_INF,
        .playerGroundHeight = link == nullptr ? 0.0F : link->mLinkAcch.GetGroundH(),
        .playerRoofHeight = link == nullptr ? 0.0F : link->mLinkAcch.GetRoofHeight(),
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
        .titlePresent = titleMenu.present,
        .titleProcedure = titleMenu.procedure,
        .titleLogoSkipReady = titleMenu.logoSkipReady,
        .titleStartReady = titleMenu.startReady,
        .nameEntryActive = nameEntryActive,
        .nameEntryCharacterSelectReady =
            nameEntryInputReady && nameEntry.selectionProcedure == 0,
        .nameEntryInputReady = nameEntryInputReady,
        .nameEntrySelectionProcedure = nameEntry.selectionProcedure,
        .fileSelectNoSaveReady = fileSelect.noSavePromptReady(),
        .fileSelectDataSelectReady = fileSelect.dataSelectReady(),
        .fileSelectKeyWaitReady = fileSelect.keyWaitReady(),
        .fileSelectYesNoReady = fileSelect.yesNoSelectReady(),
        .nameScenePresent = nameScene.present,
        .nameSceneProcedure = nameScene.procedure,
        .fileSelectPresent = nameScene.fileSelectPresent,
        .fileSelectProcedure = nameScene.fileSelectProcedure,
        .fileSelectCardCheckProcedure = nameScene.cardCheckProcedure,
        .nextStageEnabled = dComIfGp_isEnableNextStage() != 0,
        .nextStageName = dComIfGp_getNextStageName(),
        .nextRoom = static_cast<std::int8_t>(dComIfGp_getNextStageRoomNo()),
        .nextLayer = static_cast<std::int8_t>(dComIfGp_getNextStageLayer()),
        .nextPoint = dComIfGp_getNextStagePoint(),
        .rng = capture_game_rng_snapshot(),
    };

    fopAcIt_Executor(capture_milestone_actor, &storage);
    std::sort(storage.actors.begin(), storage.actors.end(),
        [](const auto& left, const auto& right) {
            return left.runtimeGeneration < right.runtimeGeneration;
        });
    observation.actors = storage.actors;
    observation.actorObservedCount = storage.actorObservedCount;
    observation.actorsTruncated = false;

    for (std::size_t index = 0; index < storage.eventFlags.size(); ++index) {
        storage.eventFlags[index] = static_cast<std::uint8_t>(
            dComIfGs_isEventBit(dSv_event_flag_c::saveBitLabels[index]) != 0);
    }
    for (std::size_t index = 0; index < storage.temporaryFlags.size(); ++index) {
        storage.temporaryFlags[index] = static_cast<std::uint8_t>(
            dComIfGs_isTmpBit(dSv_event_tmp_flag_c::tempBitLabels[index]) != 0);
    }
    // Structured read-only copy of the console-backed temporary event bank.
    // In the GCN layout this is dSv_info_c::mTmp at offset 0xDD8; preserving
    // bytes avoids collapsing register-style labels (low mask 0xff) to bools.
    std::copy(std::begin(g_dComIfG_gameInfo.info.mTmp.mEvent),
        std::end(g_dComIfG_gameInfo.info.mTmp.mEvent), storage.temporaryEventBytes.begin());
    for (std::size_t index = 0; index < storage.dungeonFlags.size(); ++index) {
        storage.dungeonFlags[index] =
            static_cast<std::uint8_t>(dComIfGs_isSaveDunSwitch(index) != 0);
    }
    observation.switchFlagRoom = observation.room;
    for (std::size_t index = 0; index < storage.switchFlags.size(); ++index) {
        storage.switchFlags[index] = static_cast<std::uint8_t>(
            dComIfGs_isSwitch(index, observation.switchFlagRoom) != 0);
    }
    observation.eventFlags = storage.eventFlags;
    observation.temporaryFlags = storage.temporaryFlags;
    observation.temporaryEventBytes = storage.temporaryEventBytes;
    observation.dungeonFlags = storage.dungeonFlags;
    observation.switchFlags = storage.switchFlags;
    observation.flagsPresent = true;
    return observation;
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

MilestoneObservation capture_milestone_observation(MilestoneObservationStorage& storage) {
    storage = {};
    return {};
}

EyeShredderGameplayTelemetry capture_eye_shredder_gameplay_telemetry() {
    return {};
}

}  // namespace dusk::automation

#endif
