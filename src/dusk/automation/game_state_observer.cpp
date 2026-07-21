#include "dusk/automation/game_state_observer.hpp"

#if DUSK_ENABLE_AUTOMATION_OBSERVERS

#include "d/actor/d_a_alink.h"
#include "d/actor/d_a_kytag14.h"
#include "d/actor/d_a_npc.h"
#include "d/actor/d_a_npc4.h"
#include "d/actor/d_a_scene_exit.h"
#include "d/actor/d_a_scene_exit2.h"
#include "d/actor/d_a_tag_event.h"
#include "d/actor/d_a_tag_evt.h"
#include "d/actor/d_a_tag_evtarea.h"
#include "d/actor/d_a_title.h"
#include "d/d_camera.h"
#include "d/d_com_inf_game.h"
#include "d/d_demo.h"
#include "d/d_msg_object.h"
#include "d/d_s_name.h"
#include "d/d_s_play.h"
#include "d/d_s_room.h"
#include "dusk/automation/file_select_observer.hpp"
#include "dusk/automation/menu_state_observer.hpp"
#include "dusk/automation/name_entry_observer.hpp"
#include "dusk/automation/rng.hpp"
#include "f_op/f_op_actor_iter.h"
#include "f_op/f_op_actor_mng.h"
#include "f_op/f_op_overlap_mng.h"
#include "f_op/f_op_scene_mng.h"
#include "f_pc/f_pc_create_req.h"
#include "f_pc/f_pc_create_tag.h"
#include "f_pc/f_pc_delete_tag.h"
#include "SSystem/SComponent/c_counter.h"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>

namespace dusk::automation {

namespace {

MilestoneObservation::ProcessLifecycleState::ProcessState process_state(
    const base_process_class& process) {
    return {
        .runtimeGeneration = process.id,
        .processName = process.name,
        .profileName = process.profname,
        .processType = process.type,
        .processSubtype = process.subtype,
        .parameters = process.parameters,
        .initState = process.state.init_state,
        .createPhase = process.state.create_phase,
    };
}

bool capture_process_lifecycle(MilestoneObservation::ProcessLifecycleState& lifecycle,
    const std::uint32_t activeActorCount) {
    using Status = MilestoneObservation::ChannelStatus;
    if (g_fpcCtTg_Queue.mSize < 0 || g_fpcDtTg_Queue.mSize < 0)
        return false;

    lifecycle.pendingCreates.clear();
    lifecycle.pendingDeletes.clear();
    lifecycle.pendingCreates.reserve(static_cast<std::size_t>(g_fpcCtTg_Queue.mSize));
    lifecycle.pendingDeletes.reserve(static_cast<std::size_t>(g_fpcDtTg_Queue.mSize));

    for (const node_class* node = g_fpcCtTg_Queue.mpHead; node != nullptr;
         node = node->mpNextNode) {
        const auto* tag = reinterpret_cast<const create_tag_class*>(node);
        const auto* request = static_cast<const create_request*>(tag->mpTagData);
        if (request == nullptr)
            return false;
        auto& retained = lifecycle.pendingCreates.emplace_back();
        retained.runtimeGeneration = request->id;
        retained.doing = request->is_doing != 0;
        retained.cancelled = request->is_cancel != 0;
        if (request->process != nullptr) {
            retained.processStatus = Status::Present;
            retained.process = process_state(*request->process);
            if (retained.process.runtimeGeneration != retained.runtimeGeneration)
                return false;
        } else {
            retained.processStatus = Status::Absent;
        }
    }

    for (const node_class* node = g_fpcDtTg_Queue.mpHead; node != nullptr;
         node = node->mpNextNode) {
        const auto* tag = reinterpret_cast<const delete_tag_class*>(node);
        const auto* process = static_cast<const base_process_class*>(tag->base.mpTagData);
        if (process == nullptr)
            return false;
        lifecycle.pendingDeletes.push_back({
            .process = process_state(*process),
            .timer = tag->timer,
        });
    }

    if (lifecycle.pendingCreates.size() != static_cast<std::size_t>(g_fpcCtTg_Queue.mSize) ||
        lifecycle.pendingDeletes.size() != static_cast<std::size_t>(g_fpcDtTg_Queue.mSize))
        return false;

    lifecycle.status = Status::Present;
    lifecycle.activeActorCount = activeActorCount;
    lifecycle.pendingCreateCount = static_cast<std::uint32_t>(lifecycle.pendingCreates.size());
    lifecycle.pendingDeleteCount = static_cast<std::uint32_t>(lifecycle.pendingDeletes.size());
    return true;
}

}  // namespace

struct MilestoneCollisionReadAdapter {
    static constexpr u32 KnownFlags =
        dBgS_Acch::FLAG_GRND_NONE | dBgS_Acch::FLAG_WALL_NONE | dBgS_Acch::FLAG_ROOF_NONE |
        dBgS_Acch::FLAG_WALL_HIT | dBgS_Acch::FLAG_GROUND_HIT | dBgS_Acch::FLAG_GROUND_FIND |
        dBgS_Acch::FLAG_GROUND_LANDING | dBgS_Acch::FLAG_GROUND_AWAY | dBgS_Acch::FLAG_ROOF_HIT |
        dBgS_Acch::FLAG_WATER_NONE | dBgS_Acch::FLAG_WATER_HIT | dBgS_Acch::FLAG_WATER_IN |
        dBgS_Acch::FLAG_LINE_CHECK | dBgS_Acch::FLAG_LINE_CHECK_NONE | dBgS_Acch::FLAG_CLR_SPEED_Y |
        dBgS_Acch::FLAG_LINE_CHECK_HIT | dBgS_Acch::FLAG_MOVE_BG_ONLY |
        dBgS_Acch::FLAG_GND_THIN_CELLING_OFF | dBgS_Acch::FLAG_WALL_SORT |
        dBgS_Acch::FLAG_LINE_DOWN;

    static u32 flags(const dBgS_Acch& value) { return value.m_flags & KnownFlags; }
    static int wallTableSize(const dBgS_Acch& value) { return value.m_tbl_size; }
    static u8 waterMode(const dBgS_Acch& value) { return value.m_wtr_mode; }
    static const cM3dGLin& line(const dBgS_Acch& value) { return value.m_lin; }
    static const cM3dGCyl& wallCylinder(const dBgS_Acch& value) { return value.m_wall_cyl; }
    static float groundCheckOffset(const dBgS_Acch& value) { return value.m_gnd_chk_offset; }
    static float roofCorrectionHeight(const dBgS_Acch& value) { return value.m_roof_crr_height; }
    static float waterCheckOffset(const dBgS_Acch& value) { return value.m_wtr_chk_offset; }

    static u32 wallFlags(const dBgS_AcchCir& value) { return value.m_flags & 0x6u; }
    static s16 wallAngleY(const dBgS_AcchCir& value) { return value.m_wall_angle_y; }
    static float wallRadiusSquared(const dBgS_AcchCir& value) { return value.m_wall_rr; }
    static float wallHeight(const dBgS_AcchCir& value) { return value.m_wall_h; }
    static float wallRadius(const dBgS_AcchCir& value) { return value.m_wall_r; }
    static float directWallHeight(const dBgS_AcchCir& value) { return value.m_wall_h_direct; }
    static const cM3dGCir& realizedCircle(const dBgS_AcchCir& value) { return value.m_cir; }
};

struct MilestoneEventManagerReadAdapter {
    struct State {
        bool dataLoaded = false;
        int cameraPlay = 0;
        s16 currentEventId = -1;
        int currentEventType = 0;
        int room = -1;
        cXyz goal{};
    };

    static State state(const dEvent_manager_c& manager) {
        return {
            .dataLoaded = manager.mDataLoaded,
            .cameraPlay = manager.mCameraPlay,
            .currentEventId = manager.mCurrentEvId,
            .currentEventType = manager.mCurrentEvType,
            .room = manager.mRoomNo,
            .goal = manager.mGoal,
        };
    }

    static const char* runningEventName(const dEvent_manager_c& manager) {
        if (manager.mCurrentEvId == -1)
            return nullptr;
        const int eventType = manager.mCurrentEvId >> 8;
        const int eventIndex = manager.mCurrentEvId & 0xff;
        if (eventType <= dEvent_manager_c::BASE_NULL || eventType >= dEvent_manager_c::BASE_MAX)
            return nullptr;
        const dEvDtBase_c& base = manager.mEventList[eventType];
        if (base.mHeaderP == nullptr || base.mEventP == nullptr || eventIndex < 0 ||
            eventIndex >= base.mHeaderP->eventNum)
        {
            return nullptr;
        }
        const dEvDtEvent_c& event = base.mEventP[eventIndex];
        return event.mEventState == dEvDt_State_START_e ? event.mName : nullptr;
    }
};

struct MilestoneNpcFlowReadAdapter {
    static const dMsgFlow_c& flow(const daNpcF_c& npc) { return npc.mFlow; }
};

struct MilestoneMessageFlowReadAdapter {
    static bool active(const dMsgFlow_c& flow) {
        return flow.mFlowNodeTBL != nullptr && flow.mNodeIdx != 0xffff;
    }
    static std::uint16_t flowId(const dMsgFlow_c& flow) { return flow.mFlow; }
    static std::uint16_t nodeIndex(const dMsgFlow_c& flow) { return flow.mNodeIdx; }
};

struct MilestoneTriggerReadAdapter {
    using Actor = MilestoneObservation::Actor;
    using Component = Actor::TriggerVolumeComponent;
    using Kind = Actor::TriggerVolumeKind;
    using Shape = Actor::TriggerVolumeShape;

    static int eventAreaType(const daTag_EvtArea_c& area) {
        const u16 type = area.shape_angle.z & 0xff;
        return type == 0xff ? 0 : type;
    }

    static bool eventAreaEnabled(const daTag_EvtArea_c& area) {
        if (area.field_0x56c != 0)
            return false;
        const u32 parameters = fopAcM_GetParam(&area);
        const u32 onEventBit = parameters & 0xfff;
        const u8 onSwitch = area.home.angle.x & 0xff;
        return (onEventBit != 0xfff && daNpcT_chkEvtBit(onEventBit)) ||
               (onSwitch != 0xff && dComIfGs_isSwitch(onSwitch, fopAcM_GetRoomNo(&area))) ||
               (onEventBit == 0xfff && onSwitch == 0xff);
    }

    static bool sceneExitEnabled(const daScex_c& exit) {
        const u32 parameters = fopAcM_GetParam(&exit);
        const u8 argument = (parameters >> 8) & 0xff;
        const u8 switchNumber = parameters >> 24;
        if (argument == 0xff || argument == 0 || argument == 3) {
            if (fopAcM_isSwitch(&exit, switchNumber))
                return false;
        } else if ((argument == 1 || argument == 2 || argument == 4) && switchNumber != 0xff &&
                   !fopAcM_isSwitch(&exit, switchNumber))
        {
            return false;
        }
        const u16 offEventBit = exit.home.angle.z & 0x0fff;
        if (offEventBit != 0x0fff &&
            dComIfGs_isEventBit(dSv_event_flag_c::saveBitLabels[offEventBit]))
        {
            return false;
        }
        const u16 onEventBit = exit.home.angle.x & 0x0fff;
        return onEventBit == 0x0fff ||
               dComIfGs_isEventBit(dSv_event_flag_c::saveBitLabels[onEventBit]);
    }

    static bool scriptedEventDeleted(const daTag_Evt_c& tag) {
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
            return dComIfGs_isSwitch(tag.field_0x5DE, fopAcM_GetRoomNo(&tag));
        }
        return false;
    }

    static bool mappedEventEnabled(const daTag_Event_c& tag) {
        if (tag.mAction != daTag_Event_c::ACTION_ARRIVAL &&
            tag.mAction != daTag_Event_c::ACTION_HUNT)
        {
            return false;
        }
        const u32 parameters = fopAcM_GetParam(&tag);
        const u8 switchNumber = (parameters >> 8) & 0xff;
        if (switchNumber != 0xff && dComIfGs_isSwitch(switchNumber, fopAcM_GetRoomNo(&tag)))
            return false;
        const u8 requiredSwitch = (parameters >> 16) & 0xff;
        if (requiredSwitch != 0xff && !dComIfGs_isSwitch(requiredSwitch, fopAcM_GetRoomNo(&tag))) {
            return false;
        }
        const u16 invalidEvent = tag.home.angle.x & 0x7fff;
        if (invalidEvent != 0x7fff && invalidEvent != 0 && daNpcT_chkEvtBit(invalidEvent))
            return false;
        const u16 requiredEvent = tag.home.angle.z;
        return requiredEvent == 0xffff || requiredEvent == 0 || daNpcT_chkEvtBit(requiredEvent);
    }

    static bool capture(const fopAc_ac_c& actor, Component& output) {
        const s16 name = fopAcM_GetName(&actor);
        if (name == fpcNm_SCENE_EXIT_e) {
            const auto& exit = static_cast<const daScex_c&>(actor);
            output.kind = Kind::SceneExit;
            output.shape = Shape::Box;
            output.enabled = sceneExitEnabled(exit);
            output.behavior = (fopAcM_GetParam(&exit) >> 8) & 0xff;
            output.centerX = exit.current.pos.x;
            output.centerY = exit.current.pos.y + exit.scale.y * 0.5f;
            output.centerZ = exit.current.pos.z;
            output.halfExtentX = std::abs(exit.scale.x);
            output.halfExtentY = std::abs(exit.scale.y) * 0.5f;
            output.halfExtentZ = std::abs(exit.scale.z);
            output.yaw = exit.shape_angle.y;
            return true;
        }
        if (name == fpcNm_SCENE_EXIT2_e) {
            const auto& exit = static_cast<const daScExit_c&>(actor);
            output.kind = Kind::SceneExitCylinder;
            output.shape = Shape::EllipticCylinder;
            output.enabled = exit.mAction == daScExit_c::ACTION_WAIT_e;
            output.verticalUnbounded = true;
            output.behavior = exit.mAction;
            output.centerX = exit.current.pos.x;
            output.centerY = exit.current.pos.y;
            output.centerZ = exit.current.pos.z;
            output.halfExtentX = std::abs(exit.mRadius);
            output.halfExtentZ = std::abs(exit.mRadius);
            return true;
        }
        if (name == fpcNm_TAG_EVTAREA_e) {
            const auto& area = static_cast<const daTag_EvtArea_c&>(actor);
            const int type = eventAreaType(area);
            output.kind = Kind::EventArea;
            output.shape = type == 15 || type == 16 ? Shape::Box : Shape::EllipticCylinder;
            output.enabled = eventAreaEnabled(area);
            output.verticalUnbounded = type == 21;
            output.behavior = static_cast<std::uint16_t>(type);
            output.centerX = type == 15 || type == 16 ? area.home.pos.x : area.current.pos.x;
            output.centerY = type == 15 || type == 16 ?
                                 area.current.pos.y + area.scale.y * 0.5f :
                                 area.current.pos.y - 10.0f + std::abs(area.scale.y) * 0.5f;
            output.centerZ = type == 15 || type == 16 ? area.home.pos.z : area.current.pos.z;
            output.halfExtentX = std::abs(area.scale.x);
            output.halfExtentY = output.verticalUnbounded ? 0.0f : std::abs(area.scale.y) * 0.5f;
            output.halfExtentZ = std::abs(area.scale.z);
            output.yaw = type == 15 || type == 16 ? area.current.angle.y : area.shape_angle.y;
            return true;
        }
        if (name == fpcNm_TAG_EVT_e) {
            const auto& tag = static_cast<const daTag_Evt_c&>(actor);
            output.kind = Kind::ScriptedEvent;
            output.shape = Shape::EllipticCylinder;
            output.enabled = (tag.field_0x5E4 == 0 || tag.field_0x5E4 == 1) &&
                             tag.field_0x5D0 == 0 && !scriptedEventDeleted(tag);
            output.behavior = static_cast<std::uint8_t>(tag.field_0x5E4);
            output.centerX = tag.current.pos.x;
            output.centerY = tag.current.pos.y;
            output.centerZ = tag.current.pos.z;
            output.halfExtentX = std::abs(tag.scale.x);
            output.halfExtentY = std::abs(tag.scale.y);
            output.halfExtentZ = std::abs(tag.scale.x);
            return true;
        }
        if (name == fpcNm_TAG_EVENT_e) {
            const auto& tag = static_cast<const daTag_Event_c&>(actor);
            const bool box = (tag.home.angle.x & 0x8000) != 0;
            output.kind = Kind::MappedEvent;
            output.shape = box ? Shape::Box : Shape::EllipticCylinder;
            output.enabled = mappedEventEnabled(tag);
            output.behavior = tag.mAction;
            output.centerX = tag.current.pos.x;
            output.centerY = box ? tag.current.pos.y + tag.scale.y * 0.5f : tag.current.pos.y;
            output.centerZ = tag.current.pos.z;
            output.halfExtentX = std::abs(tag.scale.x) * (box ? 0.5f : 1.0f);
            output.halfExtentY = std::abs(tag.scale.y) * (box ? 0.5f : 1.0f);
            output.halfExtentZ = std::abs(box ? tag.scale.z * 0.5f : tag.scale.x);
            return true;
        }
        return false;
    }
};

TitleMenuObservation MenuStateObserver::captureTitle() {
    const auto* title = static_cast<const daTitle_c*>(fopAcM_SearchByName(fpcNm_TITLE_e));
    if (title == nullptr)
        return {};
    return {
        .present = true,
        .procedure = title->mProcID,
        .logoSkipReady = title->mProcID == 1,
        .startReady = title->mProcID == 3,
    };
}

NameSceneMenuObservation MenuStateObserver::captureNameScene() {
    const auto* scene = reinterpret_cast<const dScnName_c*>(fpcM_SearchByName(fpcNm_NAME_SCENE_e));
    if (scene == nullptr)
        return {};
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
    const std::uint64_t runtimeGeneration = static_cast<std::uint32_t>(fopAcM_GetID(actor));
    static_assert(sizeof(actor->attention_info.distances) ==
                  MilestoneObservation::Actor::AttentionDistanceCount);
    const bool attentionPresent = actor->attention_info.flags != 0;
    const bool eventParticipationPresent =
        actor->eventInfo.mCommand != dEvtCmd_NONE_e ||
        actor->eventInfo.mCondition != dEvtCnd_CANDEMO_e || actor->eventInfo.mEventId != -1 ||
        actor->eventInfo.mMapToolId != 0xff || actor->eventInfo.mIndex != 0;
    const bool returnPlaceWriterPresent = fopAcM_GetName(actor) == fpcNm_KYTAG14_e;
    const bool enemyBasePresent = actor->group == fopAc_ENEMY_e;
    MilestoneObservation::Actor snapshot{
        .runtimeGeneration = runtimeGeneration,
        .actorType = actor->actor_type,
        .processSubtype = actor->subtype,
        .actorName = static_cast<std::int16_t>(fopAcM_GetName(actor)),
        .setId = actor->setID,
        .homeRoom = actor->home.roomNo,
        .oldRoom = actor->old.roomNo,
        .currentRoom = actor->current.roomNo,
        .positionX = actor->current.pos.x,
        .positionY = actor->current.pos.y,
        .positionZ = actor->current.pos.z,
        .health = actor->health,
        .status = actor->actor_status,
        .condition = actor->actor_condition,
        .parentRuntimeGeneration = static_cast<std::uint32_t>(actor->parentActorID),
        .parameters = fopAcM_GetParam(actor),
        .profileName = static_cast<std::int16_t>(fopAcM_GetProfName(actor)),
        .group = actor->group,
        .argument = actor->argument,
        .pauseFlag = actor->pause_flag,
        .processInitState = actor->state.init_state,
        .processCreatePhase = actor->state.create_phase,
        .cullType = actor->cullType,
        .demoActorId = actor->demoActorID,
        .carryType = actor->carryType,
        .heapPresent = actor->heap != nullptr,
        .modelPresent = actor->model != nullptr,
        .jointCollisionPresent = actor->jntCol != nullptr,
        .homePositionX = actor->home.pos.x,
        .homePositionY = actor->home.pos.y,
        .homePositionZ = actor->home.pos.z,
        .oldPositionX = actor->old.pos.x,
        .oldPositionY = actor->old.pos.y,
        .oldPositionZ = actor->old.pos.z,
        .velocityX = actor->speed.x,
        .velocityY = actor->speed.y,
        .velocityZ = actor->speed.z,
        .forwardSpeed = actor->speedF,
        .scaleX = actor->scale.x,
        .scaleY = actor->scale.y,
        .scaleZ = actor->scale.z,
        .gravity = actor->gravity,
        .maxFallSpeed = actor->maxFallSpeed,
        .eyePositionX = actor->eyePos.x,
        .eyePositionY = actor->eyePos.y,
        .eyePositionZ = actor->eyePos.z,
        .homeAngleX = actor->home.angle.x,
        .homeAngleY = actor->home.angle.y,
        .homeAngleZ = actor->home.angle.z,
        .oldAngleX = actor->old.angle.x,
        .oldAngleY = actor->old.angle.y,
        .oldAngleZ = actor->old.angle.z,
        .currentAngleX = actor->current.angle.x,
        .currentAngleY = actor->current.angle.y,
        .currentAngleZ = actor->current.angle.z,
        .shapeAngleX = actor->shape_angle.x,
        .shapeAngleY = actor->shape_angle.y,
        .shapeAngleZ = actor->shape_angle.z,
        .attentionPresent = attentionPresent,
        .eventParticipationPresent = eventParticipationPresent,
        .returnPlaceWriterPresent = returnPlaceWriterPresent,
        .enemyBasePresent = enemyBasePresent,
    };
    if (attentionPresent) {
        snapshot.attention.flags = actor->attention_info.flags;
        snapshot.attention.positionX = actor->attention_info.position.x;
        snapshot.attention.positionY = actor->attention_info.position.y;
        snapshot.attention.positionZ = actor->attention_info.position.z;
        std::copy(std::begin(actor->attention_info.distances),
            std::end(actor->attention_info.distances), snapshot.attention.distanceIndices.begin());
        snapshot.attention.auxiliary = actor->attention_info.field_0xa;
    }
    if (eventParticipationPresent) {
        snapshot.eventParticipation.command = actor->eventInfo.mCommand;
        snapshot.eventParticipation.condition = actor->eventInfo.mCondition;
        snapshot.eventParticipation.eventId = actor->eventInfo.mEventId;
        snapshot.eventParticipation.mapToolId = actor->eventInfo.mMapToolId;
        snapshot.eventParticipation.index = actor->eventInfo.mIndex;
    }
    if (returnPlaceWriterPresent) {
        const auto* writer = static_cast<const kytag14_class*>(actor);
        auto& component = snapshot.returnPlaceWriter;
        component.saveRoom = writer->mSaveRoomNo;
        component.savePoint = writer->mSavePoint;
        component.switchRoom = dComIfGp_roomControl_getStayNo();
        component.requiredEventSet = writer->mEventID1;
        component.requiredEventUnset = writer->mEventID2;
        component.requiredSwitchSet = writer->mSwitchNo1;
        component.requiredSwitchUnset = writer->mSwitchNo2;
        component.noTelopClear = !dComIfGs_isTmpBit(dSv_event_tmp_flag_c::NO_TELOP);
        component.eventSetSatisfied =
            writer->mEventID1 == 0xffff ||
            dComIfGs_isEventBit(dSv_event_flag_c::saveBitLabels[writer->mEventID1]);
        component.eventUnsetSatisfied =
            writer->mEventID2 == 0xffff ||
            !dComIfGs_isEventBit(dSv_event_flag_c::saveBitLabels[writer->mEventID2]);
        component.switchSetSatisfied = writer->mSwitchNo1 == 0xff ||
                                       dComIfGs_isSwitch(writer->mSwitchNo1, component.switchRoom);
        component.switchUnsetSatisfied =
            writer->mSwitchNo2 == 0xff ||
            !dComIfGs_isSwitch(writer->mSwitchNo2, component.switchRoom);
        component.eligible = component.noTelopClear && component.eventSetSatisfied &&
                             component.eventUnsetSatisfied && component.switchSetSatisfied &&
                             component.switchUnsetSatisfied;
    }
    if (enemyBasePresent) {
        const auto* enemy = static_cast<const fopEn_enemy_c*>(actor);
        auto& component = snapshot.enemyBase;
        component.flags = enemy->mFlags;
        component.throwMode = enemy->mThrowMode;
        component.downPositionX = enemy->mDownPos.x;
        component.downPositionY = enemy->mDownPos.y;
        component.downPositionZ = enemy->mDownPos.z;
        component.headLockPositionX = enemy->mHeadLockPos.x;
        component.headLockPositionY = enemy->mHeadLockPos.y;
        component.headLockPositionZ = enemy->mHeadLockPos.z;
    }
    snapshot.triggerVolumePresent = capture_actor_trigger_volume(*actor, snapshot.triggerVolume);
    storage->actorPointers.push_back(actor);
    storage->actors.push_back(snapshot);
    return 1;
}

std::pair<bool, std::uint32_t> collider_owner(cCcD_Obj* collider) {
    fopAc_ac_c* actor = collider == nullptr ? nullptr : collider->GetAc();
    return actor == nullptr ? std::pair{false, std::uint32_t{0xffffffff}} :
                              std::pair{true, static_cast<std::uint32_t>(fopAcM_GetID(actor))};
}

void capture_dynamic_colliders(MilestoneObservationStorage& storage, const bool gameplayPresent) {
    storage.dynamicColliders.clear();
    if (!gameplayPresent)
        return;

    // cCcS::Move retains the processed object pointers and their count in
    // field_0x2812 after clearing the registration counters. Reading that
    // retained set gives both observation phases the most recently completed
    // collision pass without invoking collision code or mutating game state.
    cCcS* collision = dComIfG_Ccsp();
    const std::size_t count = collision->field_0x2812;
    if (count > std::size(collision->mpObj))
        return;
    storage.dynamicColliders.reserve(count);
    for (std::size_t index = 0; index < count; ++index) {
        cCcD_Obj* object = collision->mpObj[index];
        if (object == nullptr) {
            storage.dynamicColliders.clear();
            return;
        }

        const auto [ownerPresent, ownerRuntimeGeneration] = collider_owner(object);
        const bool attackHit = object->ChkAtHit() != 0;
        const bool targetHit = object->ChkTgHit() != 0;
        const bool correctionHit = object->ChkCoHit() != 0;
        const auto [attackHitOwnerPresent, attackHitOwnerRuntimeGeneration] =
            attackHit ? collider_owner(object->GetAtHitObj()) :
                        std::pair{false, std::uint32_t{0xffffffff}};
        const auto [targetHitOwnerPresent, targetHitOwnerRuntimeGeneration] =
            targetHit ? collider_owner(object->GetTgHitObj()) :
                        std::pair{false, std::uint32_t{0xffffffff}};
        const auto [correctionHitOwnerPresent, correctionHitOwnerRuntimeGeneration] =
            correctionHit ? collider_owner(object->GetCoHitObj()) :
                            std::pair{false, std::uint32_t{0xffffffff}};
        cCcD_Stts* status = object->GetStts();
        cCcD_ShapeAttr* shape = object->GetShapeAttr();
        cCcD_ShapeAttr::Shape shapeAccess{};
        if (shape != nullptr)
            shape->getShapeAccess(&shapeAccess);
        const auto shapeKind = shape == nullptr || shapeAccess._0 == 2 ?
                                   MilestoneObservation::DynamicColliderShape::Unknown :
                               shapeAccess._0 == 0 ?
                                   MilestoneObservation::DynamicColliderShape::Sphere :
                                   MilestoneObservation::DynamicColliderShape::Cylinder;
        const cXyz correction = status == nullptr ? cXyz::Zero : *status->GetCCMoveP();
        const cM3dGAab* aabb = shape == nullptr ? nullptr : &shape->GetWorkAab();
        storage.dynamicColliders.push_back({
            .registrationIndex = static_cast<std::uint16_t>(index),
            .ownerRuntimeGeneration = ownerRuntimeGeneration,
            .attackHitOwnerRuntimeGeneration = attackHitOwnerRuntimeGeneration,
            .targetHitOwnerRuntimeGeneration = targetHitOwnerRuntimeGeneration,
            .correctionHitOwnerRuntimeGeneration = correctionHitOwnerRuntimeGeneration,
            .ownerPresent = ownerPresent,
            .statusPresent = status != nullptr,
            .shapePresent = shape != nullptr,
            .attackSet = object->ChkAtSet() != 0,
            .targetSet = object->ChkTgSet() != 0,
            .correctionSet = object->ChkCoSet() != 0,
            .attackHit = attackHit,
            .targetHit = targetHit,
            .correctionHit = correctionHit,
            .attackHitOwnerPresent = attackHitOwnerPresent,
            .targetHitOwnerPresent = targetHitOwnerPresent,
            .correctionHitOwnerPresent = correctionHitOwnerPresent,
            .shape = shapeKind,
            .attackType = object->GetAtType(),
            .targetType = static_cast<std::uint32_t>(object->GetTgType()),
            .attackSourceParameters = static_cast<std::uint32_t>(object->GetObjAt().getSPrm()),
            .attackResultParameters = static_cast<std::uint32_t>(object->GetObjAt().getRPrm()),
            .targetSourceParameters = static_cast<std::uint32_t>(object->GetObjTg().getSPrm()),
            .targetResultParameters = static_cast<std::uint32_t>(object->GetObjTg().getRPrm()),
            .correctionSourceParameters = static_cast<std::uint32_t>(object->GetObjCo().getSPrm()),
            .correctionResultParameters = static_cast<std::uint32_t>(object->GetObjCo().getRPrm()),
            .attackPower = object->GetAtAtp(),
            .weight = status == nullptr ? std::uint8_t{0} : status->GetWeightUc(),
            .damage = status == nullptr ? std::uint8_t{0} : status->GetDmg(),
            .centerX = shape == nullptr ? 0.0F : shapeAccess._4.x,
            .centerY = shape == nullptr ? 0.0F : shapeAccess._4.y,
            .centerZ = shape == nullptr ? 0.0F : shapeAccess._4.z,
            .radius = shape == nullptr ? 0.0F : shapeAccess._10,
            .height = shape == nullptr ? 0.0F : shapeAccess._14,
            .aabbMinX = aabb == nullptr ? 0.0F : aabb->GetMinX(),
            .aabbMinY = aabb == nullptr ? 0.0F : aabb->GetMinY(),
            .aabbMinZ = aabb == nullptr ? 0.0F : aabb->GetMinZ(),
            .aabbMaxX = aabb == nullptr ? 0.0F : aabb->GetMaxX(),
            .aabbMaxY = aabb == nullptr ? 0.0F : aabb->GetMaxY(),
            .aabbMaxZ = aabb == nullptr ? 0.0F : aabb->GetMaxZ(),
            .correctionX = correction.x,
            .correctionY = correction.y,
            .correctionZ = correction.z,
        });
    }
}

}  // namespace

bool game_state_observers_enabled() {
    return true;
}

bool capture_actor_trigger_volume(
    const fopAc_ac_c& actor, MilestoneObservation::Actor::TriggerVolumeComponent& output) {
    output = {};
    return MilestoneTriggerReadAdapter::capture(actor, output);
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
        const float yaw =
            static_cast<float>(static_cast<std::int16_t>(player->current.angle.y)) * AngleToRadians;
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
    storage.actorPointers.clear();
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
    fopAcIt_Executor(capture_milestone_actor, &storage);
    const auto liveActorIdentity =
        [&](const fopAc_ac_c* actor) -> std::optional<MilestoneObservation::ActorIdentity> {
        if (actor == nullptr)
            return MilestoneObservation::ActorIdentity{};
        const auto found = std::ranges::find(storage.actorPointers, actor);
        if (found == storage.actorPointers.end())
            return std::nullopt;
        const auto& source = storage.actors[std::distance(storage.actorPointers.begin(), found)];
        return MilestoneObservation::ActorIdentity{
            .present = true,
            .runtimeGeneration = static_cast<std::uint32_t>(source.runtimeGeneration),
            .actorName = source.actorName,
            .setId = source.setId,
            .homeRoom = source.homeRoom,
            .currentRoom = source.currentRoom,
            .homePositionPresent = true,
            .homePositionX = source.homePositionX,
            .homePositionY = source.homePositionY,
            .homePositionZ = source.homePositionZ,
        };
    };
    const auto actorIdentity = [&](const fopAc_ac_c* actor) {
        return liveActorIdentity(actor).value_or(MilestoneObservation::ActorIdentity{});
    };
    const auto eventActorReference = [&](const fopAc_ac_c* actor) {
        MilestoneObservation::EventQueueState::ActorReference reference;
        const auto identity = liveActorIdentity(actor);
        if (!identity.has_value()) {
            reference.status = MilestoneObservation::ChannelStatus::Unavailable;
        } else if (!identity->present) {
            reference.status = MilestoneObservation::ChannelStatus::Absent;
        } else {
            reference.status = MilestoneObservation::ChannelStatus::Present;
            reference.identity = *identity;
        }
        return reference;
    };
    const auto eventReferenceFromProcessId = [&](const fpc_ProcID processId) {
        if (processId == fpcM_ERROR_PROCESS_ID_e)
            return eventActorReference(nullptr);
        const fopAc_ac_c* actor = fopAcM_SearchByID(processId);
        if (actor == nullptr) {
            MilestoneObservation::EventQueueState::ActorReference reference;
            reference.status = MilestoneObservation::ChannelStatus::Unavailable;
            return reference;
        }
        return eventActorReference(actor);
    };
    const fopAc_ac_c* talkPartnerActor =
        link == nullptr ? nullptr : fopAcM_getTalkEventPartner(link);
    const MilestoneObservation::ActorIdentity talkPartner = actorIdentity(talkPartnerActor);
    const fpc_ProcID grabbedId = link == nullptr ? fpcM_ERROR_PROCESS_ID_e : link->getGrabActorID();
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
        .playerDamageWaitTimer =
            static_cast<std::int16_t>(link == nullptr ? 0 : link->getDamageWaitTimer()),
        .playerIceDamageWaitTimer =
            static_cast<std::int16_t>(link == nullptr ? 0 : link->getIceDamageWaitTimer()),
        .playerSwordChangeWaitTimer =
            static_cast<std::uint8_t>(link == nullptr ? 0 : link->getSwordChangeWaitTimer()),
        .playerDoStatus = static_cast<std::uint8_t>(link == nullptr ? 0 : dComIfGp_getDoStatus()),
        .talkPartner = talkPartner,
        .grabbedActor = grabbedActor,
        .playerGroundContact = link != nullptr && link->mLinkAcch.ChkGroundHit(),
        .playerWallContact = link != nullptr && link->mLinkAcch.ChkWallHit() != 0,
        .playerRoofContact = link != nullptr && link->mLinkAcch.ChkRoofHit(),
        .playerWaterContact = link != nullptr && link->mLinkAcch.ChkWaterHit(),
        .playerWaterIn = link != nullptr && link->mLinkAcch.ChkWaterIn(),
        .playerGroundHeightPresent = link != nullptr &&
                                     std::isfinite(link->mLinkAcch.GetGroundH()) &&
                                     link->mLinkAcch.GetGroundH() != -G_CM3D_F_INF,
        .playerRoofHeightPresent = link != nullptr &&
                                   std::isfinite(link->mLinkAcch.GetRoofHeight()) &&
                                   link->mLinkAcch.GetRoofHeight() != G_CM3D_F_INF,
        .playerGroundHeight = link == nullptr ? 0.0F : link->mLinkAcch.GetGroundH(),
        .playerRoofHeight = link == nullptr ? 0.0F : link->mLinkAcch.GetRoofHeight(),
        .eventRunning = dComIfGp_event_runCheck() != 0,
        .eventId = static_cast<std::int16_t>(event == nullptr ? -1 : event->mEventId),
        .eventMode = static_cast<std::uint8_t>(event == nullptr ? 0 : event->getMode()),
        .eventStatus = static_cast<std::uint8_t>(event == nullptr ? 0 : event->mEventStatus),
        .eventMapToolId =
            static_cast<std::uint8_t>(event == nullptr ? 0xff : event->getMapToolId()),
        // The legacy hash channel remains unavailable. Observation v10 copies
        // the exact bounded run-event name into the planner handoff channel
        // below, avoiding an unauditable hash or an escaping game pointer.
        .eventNameHashPresent = false,
        .eventNameHash = 0,
        .titlePresent = titleMenu.present,
        .titleProcedure = titleMenu.procedure,
        .titleLogoSkipReady = titleMenu.logoSkipReady,
        .titleStartReady = titleMenu.startReady,
        .nameEntryActive = nameEntryActive,
        .nameEntryCharacterSelectReady = nameEntryInputReady && nameEntry.selectionProcedure == 0,
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

    const auto copyFixedName = [](auto& destination, const char* source) {
        destination.fill('\0');
        if (source == nullptr)
            return;
        for (std::size_t index = 0; index < destination.size() && source[index] != '\0'; ++index)
            destination[index] = source[index];
    };

    auto& runtimeFile = observation.runtimeFile;
    runtimeFile.status = MilestoneObservation::ChannelStatus::Present;
    runtimeFile.noFileRaw = dComIfGs_getNoFile();
    runtimeFile.dataNumRaw = dComIfGs_getDataNum();
    for (std::size_t index = 0; index < runtimeFile.physicalSlots.size(); ++index) {
        runtimeFile.physicalSlots[index].number = static_cast<std::uint8_t>(index + 1);
        // The active game state does not retain a trustworthy copy of all
        // three card payloads. Their existence is explicit, but their contents
        // are not fabricated from the live runtime file.
        runtimeFile.physicalSlots[index].contentStatus =
            MilestoneObservation::ChannelStatus::NotSampled;
    }
    if (player != nullptr && runtimeFile.noFileRaw == 0 && runtimeFile.dataNumRaw < 3) {
        runtimeFile.backingAttachmentStatus = MilestoneObservation::ChannelStatus::Present;
        runtimeFile.attachedPhysicalSlot = static_cast<std::int8_t>(runtimeFile.dataNumRaw + 1);
        runtimeFile.physicalSlots[runtimeFile.dataNumRaw].attachedToRuntime = true;
    } else {
        // Nonzero mNoFile denotes a slotless runtime in the original flow, but
        // the PC command-line loader also writes 1..3 here. Title/menu defaults
        // can likewise resemble slot zero before an active player exists.
        // Preserve the raw values and require exact-build/lifecycle rules to
        // interpret either ambiguity.
        runtimeFile.backingAttachmentStatus = MilestoneObservation::ChannelStatus::Unavailable;
    }

    auto& returnPlace = observation.returnPlace;
    auto& savedReturnPlace = g_dComIfG_gameInfo.info.getPlayer().getPlayerReturnPlace();
    returnPlace.status = MilestoneObservation::ChannelStatus::Present;
    copyFixedName(returnPlace.stage, savedReturnPlace.getName());
    returnPlace.room = savedReturnPlace.getRoomNo();
    returnPlace.playerStatus = savedReturnPlace.getPlayerStatus();

    auto& restart = observation.restart;
    const cXyz& restartPosition = dComIfGs_getRestartRoomPos();
    restart.status = MilestoneObservation::ChannelStatus::Present;
    restart.room = dComIfGs_getRestartRoomNo();
    restart.startPoint = dComIfGs_getStartPoint();
    restart.angleY = dComIfGs_getRestartRoomAngleY();
    restart.positionX = restartPosition.x;
    restart.positionY = restartPosition.y;
    restart.positionZ = restartPosition.z;
    restart.roomParam = dComIfGs_getRestartRoomParam();
    restart.lastSpeed = dComIfGs_getLastSceneSpeedF();
    restart.lastMode = dComIfGs_getLastSceneMode();
    restart.lastAngleY = dComIfGs_getLastSceneAngleY();

    auto& handoff = observation.eventHandoff;
    handoff.noTelopStatus = MilestoneObservation::ChannelStatus::Present;
    handoff.noTelop = dComIfGs_isTmpBit(dSv_event_tmp_flag_c::NO_TELOP) != 0;
    handoff.playerControlStatus = link == nullptr ? MilestoneObservation::ChannelStatus::Absent :
                                                    MilestoneObservation::ChannelStatus::Present;
    handoff.playerControlModeFlags = link == nullptr ? 0 : link->mModeFlg;
    handoff.playerControlDoStatus =
        static_cast<std::uint8_t>(link == nullptr ? 0 : dComIfGp_getDoStatus());
    if (event != nullptr) {
        handoff.status = MilestoneObservation::ChannelStatus::Present;
        handoff.preItemNo = event->mPreItemNo;
        handoff.getItemNo = event->mGtItm;
        handoff.eventFlags = event->mEventFlag;
        handoff.secondaryFlags = event->mFlag2;
        handoff.hindFlags = event->mHindFlag;
        handoff.talkXyType = event->mTalkXyType;
        handoff.compulsory = event->mCompulsory;
        handoff.roomInfoSet = event->mRoomInfoSet;
        handoff.skipTimer = event->mSkipTimer;
        handoff.skipParameter = event->mSkipParameter;
        handoff.itemPartner = actorIdentity(dComIfGp_event_getItemPartner());
        if (observation.eventRunning) {
            const char* eventName =
                MilestoneEventManagerReadAdapter::runningEventName(dComIfGp_getEventManager());
            const std::size_t eventNameLength = eventName == nullptr ? 0 : std::strlen(eventName);
            if (eventName != nullptr && eventNameLength < handoff.eventName.size()) {
                copyFixedName(handoff.eventName, eventName);
                handoff.eventNameStatus = MilestoneObservation::ChannelStatus::Present;
            } else {
                // A truncated event name cannot serve as exact evidence.
                handoff.eventNameStatus = MilestoneObservation::ChannelStatus::Unavailable;
            }
        } else {
            handoff.eventNameStatus = MilestoneObservation::ChannelStatus::Absent;
        }
    } else {
        handoff.status = MilestoneObservation::ChannelStatus::Unavailable;
        handoff.eventNameStatus = MilestoneObservation::ChannelStatus::Unavailable;
    }

    auto& eventQueue = observation.eventQueue;
    if (event == nullptr) {
        eventQueue.status = MilestoneObservation::ChannelStatus::Unavailable;
    } else {
        // The scene's event Step runs before the later actor process lists.
        // Step consumes the prior queue; Link, tags, doors and exits then submit
        // requests that remain here for the next Step. Observation therefore
        // records a real pending decision boundary, not stale consumed orders.
        // Raw actor pointers are matched by address against the complete live
        // actor walk above and are never dereferenced here.
        using EventQueue = MilestoneObservation::EventQueueState;
        EventQueue captured;
        captured.status = MilestoneObservation::ChannelStatus::Present;
        captured.activeRequestActor = eventReferenceFromProcessId(event->mPt1);
        captured.activeTargetActor = eventReferenceFromProcessId(event->mPt2);
        captured.activeTalkActor = eventReferenceFromProcessId(event->mPtT);
        captured.activeItemActor = eventReferenceFromProcessId(event->mPtI);
        captured.activeDoorActor = eventReferenceFromProcessId(event->mPtd);
        captured.changeActor =
            eventActorReference(static_cast<const fopAc_ac_c*>(event->mChangeActor));
        captured.skipRegistered = event->mSkipFunc != nullptr;
        captured.skipActor = captured.skipRegistered ?
                                 eventReferenceFromProcessId(event->mSkipActorId) :
                                 eventActorReference(nullptr);

        const int count = event->mNum;
        bool queueValid = count >= 0 && count <= static_cast<int>(EventQueue::MaximumPendingOrders);
        int orderIndex = count == 0 ? -1 : event->mOrderIdx;
        std::array<bool, EventQueue::MaximumPendingOrders> visited{};
        for (int outputIndex = 0; queueValid && outputIndex < count; ++outputIndex) {
            if (orderIndex < 0 ||
                orderIndex >= static_cast<int>(EventQueue::MaximumPendingOrders) ||
                visited[orderIndex])
            {
                queueValid = false;
                break;
            }
            visited[orderIndex] = true;
            const dEvt_order_c& source = event->mOrder[orderIndex];
            auto& destination = captured.pendingOrders[outputIndex];
            destination.type = source.mEventType;
            destination.flags = source.mFlag;
            destination.hindFlags = source.mHindFlag;
            destination.eventId = source.mEventId;
            destination.priority = source.mPriority;
            destination.mapToolId = source.mMapToolId;
            destination.requestActor = eventActorReference(source.mpRequestActor);
            destination.targetActor = eventActorReference(source.mpTargetActor);
            orderIndex = source.mNextOrderIdx;
        }
        if (queueValid && orderIndex != -1)
            queueValid = false;
        if (queueValid) {
            captured.pendingCount = static_cast<std::uint8_t>(count);
            eventQueue = captured;
        } else {
            eventQueue.status = MilestoneObservation::ChannelStatus::Unavailable;
        }
    }

    auto& attentionCandidates = observation.attentionCandidates;
    if (player == nullptr) {
        attentionCandidates.status = MilestoneObservation::ChannelStatus::Absent;
    } else {
        using Attention = MilestoneObservation::AttentionCandidateState;
        dAttention_c* attention = dComIfGp_getAttention();
        Attention captured;
        captured.status = MilestoneObservation::ChannelStatus::Present;
        captured.playerAttentionFlags = attention->mPlayerAttentionFlags;
        captured.attentionStatus = attention->mAttnStatus;
        captured.attentionBlockTimer = attention->mAttnBlockTimer;
        const auto countAndOffsetValid = [](const int count, const int offset, const int capacity) {
            return count >= 0 && count <= capacity && offset >= 0 &&
                   ((count == 0 && offset == 0) || (count != 0 && offset < count));
        };
        bool valid =
            countAndOffsetValid(attention->mLockonCount, attention->mLockOnOffset,
                static_cast<int>(Attention::MaximumLockCandidates)) &&
            countAndOffsetValid(attention->mActionCount, attention->mActionOffset,
                static_cast<int>(Attention::MaximumActionCandidates)) &&
            countAndOffsetValid(attention->mCheckObjectCount, attention->mCheckObjectOffset,
                static_cast<int>(Attention::MaximumCheckCandidates));
        const auto captureList = [&](auto& destination, dAttList_c* source, const int count) {
            for (int index = 0; valid && index < count; ++index) {
                dAttList_c& candidate = source[index];
                auto& output = destination[static_cast<std::size_t>(index)];
                output.actor = eventReferenceFromProcessId(candidate.getPid());
                output.weight = candidate.mWeight;
                output.distance = candidate.mDistance;
                output.angle = candidate.mAngle.Val();
                output.type = candidate.mType;
                valid = output.actor.status == MilestoneObservation::ChannelStatus::Present &&
                        std::isfinite(output.weight) && std::isfinite(output.distance) &&
                        output.distance >= 0.0F && output.type < fopAc_attn_MAX_e;
            }
        };
        if (valid) {
            captured.lockCount = static_cast<std::uint8_t>(attention->mLockonCount);
            captured.lockOffset = static_cast<std::uint8_t>(attention->mLockOnOffset);
            captured.actionCount = static_cast<std::uint8_t>(attention->mActionCount);
            captured.actionOffset = static_cast<std::uint8_t>(attention->mActionOffset);
            captured.checkCount = static_cast<std::uint8_t>(attention->mCheckObjectCount);
            captured.checkOffset = static_cast<std::uint8_t>(attention->mCheckObjectOffset);
            captureList(captured.lockCandidates, attention->mLockOnList, attention->mLockonCount);
            captureList(captured.actionCandidates, attention->mActionList, attention->mActionCount);
            captureList(captured.checkCandidates, attention->mCheckObjectList,
                attention->mCheckObjectCount);
        }
        attentionCandidates = valid ? captured : Attention{};
        if (!valid)
            attentionCandidates.status = MilestoneObservation::ChannelStatus::Unavailable;
    }

    handoff.messageCutStatus = MilestoneObservation::ChannelStatus::Unavailable;
    if (talkPartnerActor == nullptr) {
        handoff.messageFlowStatus = MilestoneObservation::ChannelStatus::Absent;
    } else if (fopAcM_GetName(talkPartnerActor) == fpcNm_NPC_RAFREL_e ||
               fopAcM_GetName(talkPartnerActor) == fpcNm_NPC_GRC_e)
    {
        const auto& npc = *static_cast<const daNpcF_c*>(talkPartnerActor);
        const dMsgFlow_c& flow = MilestoneNpcFlowReadAdapter::flow(npc);
        if (MilestoneMessageFlowReadAdapter::active(flow)) {
            handoff.messageFlowStatus = MilestoneObservation::ChannelStatus::Present;
            handoff.messageFlowId = MilestoneMessageFlowReadAdapter::flowId(flow);
            handoff.messageNodeIndex = MilestoneMessageFlowReadAdapter::nodeIndex(flow);
        } else {
            handoff.messageFlowStatus = MilestoneObservation::ChannelStatus::Absent;
        }
    } else {
        handoff.messageFlowStatus = MilestoneObservation::ChannelStatus::Unavailable;
    }

    auto& message = observation.messageSession;
    dMsgObject_c* messageObject = dMsgObject_getMsgObjectClass();
    if (messageObject == nullptr) {
        message.status = MilestoneObservation::ChannelStatus::Absent;
    } else {
        using Message = MilestoneObservation::MessageSessionState;
        message.status = MilestoneObservation::ChannelStatus::Present;
        message.procedure = dMsgObject_c::getStatus();
        message.messageId = dMsgObject_c::getMessageID();
        message.messageIndex = dMsgObject_c::getIdx();
        message.nodeIndex = dMsgObject_c::getNodeIdx();
        message.flowId = dMsgObject_c::getNowTalkFlowNo();
        message.selectionCount = dMsgObject_c::getSelectWordFlag();
        message.selectionCursor = dMsgObject_c::getSelectCursorPos();
        message.selectionPush = messageObject->getSelectPushFlag();
        message.outputType = dMsgObject_c::getMsgOutputType();
        message.flags = static_cast<std::uint16_t>(
            (dMsgObject_isTalkNowCheck() ? Message::TalkNow : 0) |
            (messageObject->isTalkMessage() ? Message::TalkMessage : 0) |
            (messageObject->isAutoMessageFlag() ? Message::AutoMessage : 0) |
            (messageObject->isKillMessageFlagLocal() ? Message::KillPending : 0) |
            (messageObject->isCameraCancelFlagLocal() ? Message::CameraCancel : 0) |
            (messageObject->isMsgSendLocal() ? Message::Send : 0) |
            (messageObject->isMsgSendControlLocal() ? Message::SendControl : 0));
        message.talkActor = actorIdentity(dMsgObject_c::getpTalkActor());
    }

    auto& transition = observation.eventTransition;
    const auto eventManagerState =
        MilestoneEventManagerReadAdapter::state(dComIfGp_getEventManager());
    transition.status = MilestoneObservation::ChannelStatus::Present;
    transition.eventDataLoaded = eventManagerState.dataLoaded;
    transition.cameraPlay = eventManagerState.cameraPlay;
    if (eventManagerState.currentEventId == -1) {
        transition.currentEventStatus = MilestoneObservation::ChannelStatus::Absent;
        transition.eventGoalStatus = MilestoneObservation::ChannelStatus::Absent;
    } else {
        transition.currentEventStatus = MilestoneObservation::ChannelStatus::Present;
        transition.currentEventId = eventManagerState.currentEventId;
        transition.currentEventType = eventManagerState.currentEventType;
        transition.currentEventRoom = eventManagerState.room;
        transition.eventGoalStatus = MilestoneObservation::ChannelStatus::Present;
        transition.eventGoal = {
            eventManagerState.goal.x, eventManagerState.goal.y, eventManagerState.goal.z};
    }
    if (observation.nextStageEnabled) {
        transition.nextStageStatus = MilestoneObservation::ChannelStatus::Present;
        copyFixedName(transition.nextStage, observation.nextStageName);
        transition.nextRoom = observation.nextRoom;
        transition.nextLayer = observation.nextLayer;
        transition.nextPoint = observation.nextPoint;
        transition.nextWipe = dComIfGp_getNextStageWipe();
        transition.nextWipeSpeed = dComIfGp_getNextStageWipeSpeed();
    } else {
        transition.nextStageStatus = MilestoneObservation::ChannelStatus::Absent;
    }

    auto& clocks = observation.clockDomains;
    clocks.status = MilestoneObservation::ChannelStatus::Present;
    clocks.frameworkFrames = g_Counter.mCounter0;
    clocks.gameplayFrames = g_Counter.mTimer;
    clocks.globalPause = dComIfGp_isPauseFlag() != 0;
    clocks.scenePaused = dScnPly_c::isPause() != 0;
    clocks.scenePauseTimer = dScnPly_c::pauseTimer;
    clocks.sceneNextPauseTimer = dScnPly_c::nextPauseTimer;
    clocks.overlapRequestActive = fopOvlpM_IsDoingReq() != 0;
    clocks.overlapFadeoutPeek = fopOvlpM_IsPeek() != 0;
    if (dDemo_c::getMode() == 0) {
        clocks.demoStatus = MilestoneObservation::ChannelStatus::Absent;
    } else {
        clocks.demoStatus = MilestoneObservation::ChannelStatus::Present;
        clocks.demoMode = dDemo_c::getMode();
        clocks.demoFrame = dDemo_c::getFrame();
        clocks.demoFrameNoMessage = dDemo_c::getFrameNoMsg();
        clocks.demoFlags = dDemo_c::isStatus(~std::uint32_t{0});
    }
    const bool timerObjectPresent = dComIfG_getTimerPtr() != nullptr;
    const std::int32_t timerMode = dComIfG_getTimerMode();
    if (timerObjectPresent && timerMode >= 0) {
        clocks.timerStatus = MilestoneObservation::ChannelStatus::Present;
        clocks.timerMode = timerMode;
        clocks.timerNowMs = dComIfG_getTimerNowTimeMs();
        clocks.timerLimitMs = dComIfG_getTimerLimitTimeMs();
    } else if (!timerObjectPresent && timerMode == -1) {
        clocks.timerStatus = MilestoneObservation::ChannelStatus::Absent;
    } else {
        clocks.timerStatus = MilestoneObservation::ChannelStatus::Unavailable;
    }

    auto& roomLoad = observation.roomLoad;
    roomLoad.status = MilestoneObservation::ChannelStatus::Present;
    roomLoad.roomRead = dStage_roomControl_c::getRoomReadId();
    roomLoad.stayRoom = dStage_roomControl_c::getStayNo();
    roomLoad.oldStayRoom = dStage_roomControl_c::mOldStayNo;
    roomLoad.nextStayRoom = dStage_roomControl_c::getNextStayNo();
    roomLoad.noChangeRoom = dStage_roomControl_c::mNoChangeRoom != 0;
    roomLoad.timePass = dStage_roomControl_c::GetTimePass() != 0;
    for (std::size_t index = 0; index < roomLoad.rooms.size(); ++index) {
        const auto& source = dStage_roomControl_c::mStatus[index];
        auto& room = roomLoad.rooms[index];
        room.statusFlags = source.mFlag;
        room.draw = source.mDraw;
        room.zoneCount = source.mZoneCount;
        room.zone = source.mZoneNo;
        room.memoryBlock = source.mMemBlockID;
        room.region = source.mRegionNo;
        if (source.mFlag == 0)
            continue;
        scene_class* scene =
            fopScnM_SearchByID(dStage_roomControl_c::getStatusProcID(index));
        if (scene == nullptr) {
            room.sceneStatus = MilestoneObservation::ChannelStatus::Absent;
        } else if (fpcM_GetName(scene) != fpcNm_ROOM_SCENE_e) {
            room.sceneStatus = MilestoneObservation::ChannelStatus::Unavailable;
        } else {
            const auto& phase = static_cast<const room_of_scene_class*>(scene)->phase;
            room.sceneStatus = MilestoneObservation::ChannelStatus::Present;
            room.scenePhase = phase.id;
            room.scenePhaseActive = phase.mpHandlerTable != nullptr;
        }
    }

    if (player != nullptr) {
        auto& resources = observation.playerResources;
        auto& savedPlayer = g_dComIfG_gameInfo.info.getPlayer();
        auto& status = savedPlayer.getPlayerStatusA();
        auto& items = savedPlayer.getItem();
        auto& collect = savedPlayer.getCollect();
        resources.maximumLife = status.getMaxLife();
        resources.life = status.getLife();
        resources.rupees = status.getRupee();
        resources.rupeeCapacity = status.getRupeeMax();
        resources.maximumOil = status.getMaxOil();
        resources.oil = status.getOil();
        resources.maximumMagic = status.getMaxMagic();
        resources.magic = status.getMagic();
        resources.wallet = status.getWalletSize();
        resources.transformStatus = status.getTransformStatus();
        resources.worldTime = dComIfGs_getTime();
        resources.date = dComIfGs_getDate();
        resources.arrows = dComIfGs_getArrowNum();
        resources.arrowCapacity = dComIfGs_getArrowMax();
        resources.pachinko = dComIfGs_getPachinkoNum();
        resources.poeSouls = dComIfGs_getPohSpiritNum();
        resources.smallKeys = dComIfGs_getKeyNum();
        resources.dungeonMap = dComIfGs_isDungeonItemMap() != 0;
        resources.dungeonCompass = dComIfGs_isDungeonItemCompass() != 0;
        resources.dungeonBossKey = dComIfGs_isDungeonItemBossKey() != 0;
        resources.dungeonWarp = dComIfGs_isDungeonItemWarp() != 0;
        for (std::size_t index = 0; index < resources.inventory.size(); ++index)
            resources.inventory[index] = items.getItem(static_cast<int>(index), false);
        for (std::size_t index = 0; index < resources.selectedItems.size(); ++index) {
            resources.selectedItems[index] = status.getSelectItemIndex(static_cast<int>(index));
            resources.mixedItems[index] = status.getMixItemIndex(static_cast<int>(index));
        }
        for (std::size_t index = 0; index < resources.equipment.size(); ++index)
            resources.equipment[index] = status.getSelectEquip(static_cast<int>(index));
        for (std::size_t index = 0; index < resources.bombCounts.size(); ++index) {
            resources.bombCounts[index] = dComIfGs_getBombNum(static_cast<std::uint8_t>(index));
            resources.bombCapacities[index] =
                dComIfGs_getBombMax(resources.inventory[SLOT_15 + index]);
        }
        for (std::size_t index = 0; index < resources.bottleQuantities.size(); ++index)
            resources.bottleQuantities[index] =
                dComIfGs_getBottleNum(static_cast<std::uint8_t>(index));
        for (std::size_t item = 0; item < resources.acquiredItemBits.size() * 8; ++item) {
            if (dComIfGs_isItemFirstBit(static_cast<std::uint8_t>(item)) != 0)
                resources.acquiredItemBits[item / 8] |= static_cast<std::uint8_t>(1u << (item % 8));
        }
        std::copy(
            std::begin(collect.mItem), std::end(collect.mItem), resources.collectItemBits.begin());
        resources.collectedCrystalBits = collect.mCrystal;
        resources.collectedMirrorBits = collect.mMirror;
        observation.playerResourcesPresent = true;
    }

    if (link != nullptr) {
        auto& relationships = observation.playerRelationships;
        const fpc_ProcID itemId = link->getItemID();
        relationships.targetedActor = actorIdentity(link->mTargetedActor);
        relationships.rideActor = actorIdentity(link->mRideAcKeep.getActorConst());
        relationships.heldItemActor =
            actorIdentity(itemId == fpcM_ERROR_PROCESS_ID_e ? nullptr : fopAcM_SearchByID(itemId));
        relationships.grabbedActor = grabbedActor;
        relationships.thrownBoomerangActor =
            actorIdentity(link->mThrowBoomerangAcKeep.getActorConst());
        relationships.copyRodActor = actorIdentity(link->mCopyRodAcKeep.getActorConst());
        relationships.hookshotRoofWaitActor =
            actorIdentity(link->mCargoCarryAcKeep.getActorConst());
        relationships.chainGrabActor = actorIdentity(link->field_0x2844.getActorConst());
        relationships.attentionHintActor = actorIdentity(dComIfGp_att_getZHint());
        relationships.attentionCatchActor = actorIdentity(dComIfGp_att_getCatghTarget());
        relationships.attentionLookActor = actorIdentity(dComIfGp_att_getLookTarget());
        observation.playerRelationshipsPresent = true;

        const dBgS_Acch& collision = link->mLinkAcch;
        auto& solver = observation.playerCollisionSolver;
        solver.flags = MilestoneCollisionReadAdapter::flags(collision);
        solver.wallTableSize = MilestoneCollisionReadAdapter::wallTableSize(collision);
        solver.waterMode = MilestoneCollisionReadAdapter::waterMode(collision);
        const cM3dGLin& line = MilestoneCollisionReadAdapter::line(collision);
        solver.lineStart = {line.GetStart().x, line.GetStart().y, line.GetStart().z};
        solver.lineEnd = {line.GetEnd().x, line.GetEnd().y, line.GetEnd().z};
        const cM3dGCyl& cylinder = MilestoneCollisionReadAdapter::wallCylinder(collision);
        solver.wallCylinderCenter = {cylinder.GetC().x, cylinder.GetC().y, cylinder.GetC().z};
        solver.wallCylinderRadius = cylinder.GetR();
        solver.wallCylinderHeight = cylinder.GetH();
        solver.groundCheckOffset = MilestoneCollisionReadAdapter::groundCheckOffset(collision);
        solver.roofCorrectionHeight =
            MilestoneCollisionReadAdapter::roofCorrectionHeight(collision);
        solver.waterCheckOffset = MilestoneCollisionReadAdapter::waterCheckOffset(collision);
        for (std::size_t index = 0; index < solver.wallCircles.size(); ++index) {
            const dBgS_AcchCir& source = link->mAcchCir[index];
            auto& destination = solver.wallCircles[index];
            destination.flags = MilestoneCollisionReadAdapter::wallFlags(source);
            destination.angleY = MilestoneCollisionReadAdapter::wallAngleY(source);
            destination.wallRadiusSquared =
                MilestoneCollisionReadAdapter::wallRadiusSquared(source);
            destination.wallHeight = MilestoneCollisionReadAdapter::wallHeight(source);
            destination.wallRadius = MilestoneCollisionReadAdapter::wallRadius(source);
            destination.directWallHeight = MilestoneCollisionReadAdapter::directWallHeight(source);
            const cM3dGCir& circle = MilestoneCollisionReadAdapter::realizedCircle(source);
            destination.realizedCenter = {circle.GetCx(), circle.GetHeight(), circle.GetCy()};
            destination.realizedRadius = circle.GetR();
        }
        observation.playerCollisionSolverPresent = true;
    }

    std::sort(
        storage.actors.begin(), storage.actors.end(), [](const auto& left, const auto& right) {
            return left.runtimeGeneration < right.runtimeGeneration;
        });
    observation.actors = storage.actors;
    observation.actorObservedCount = storage.actorObservedCount;
    observation.actorsTruncated = false;

    auto& lifecycle = observation.processLifecycle;
    if (!capture_process_lifecycle(lifecycle, storage.actorObservedCount)) {
        lifecycle = {};
        lifecycle.status = MilestoneObservation::ChannelStatus::Unavailable;
    }

    capture_dynamic_colliders(storage, player != nullptr);
    observation.dynamicColliders = storage.dynamicColliders;
    observation.dynamicCollidersPresent =
        player != nullptr && storage.dynamicColliders.size() == dComIfG_Ccsp()->field_0x2812;
    observation.dynamicCollidersTruncated = false;

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
        storage.switchFlags[index] =
            static_cast<std::uint8_t>(dComIfGs_isSwitch(index, observation.switchFlagRoom) != 0);
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
