#include "dusk/automation/gameplay_trace_observer.hpp"

#include "dusk/automation/gameplay_trace.hpp"

#if DUSK_ENABLE_AUTOMATION_OBSERVERS

#include "JSystem/JUtility/JUTGamePad.h"
#include "d/actor/d_a_alink.h"
#include "d/d_com_inf_game.h"
#include "f_op/f_op_actor_iter.h"
#include "f_op/f_op_actor_mng.h"
#include "f_op/f_op_camera_mng.h"

#include <array>
#include <cmath>
#include <cstring>
#include <limits>

#endif

namespace dusk::automation {

bool gameplay_trace_observer_enabled() {
#if DUSK_ENABLE_AUTOMATION_OBSERVERS
    return true;
#else
    return false;
#endif
}

void record_gameplay_trace_post_simulation(const GameplayTracePostSimulationContext& context) {
#if DUSK_ENABLE_AUTOMATION_OBSERVERS
    auto& recorder = gameplay_trace_recorder();
    if (!recorder.active())
        return;

    using Channel = GameplayTraceChannel;
    using Status = GameplayTraceChannelStatus;
    const std::uint64_t requested = recorder.requestedChannels();
    const auto wants = [requested](const Channel channel) {
        return (requested & gameplay_trace_channel_bit(channel)) != 0;
    };
    const auto copyName = [](std::array<char, 8>& output, const char* input) {
        if (input != nullptr)
            std::strncpy(output.data(), input, output.size());
    };

    GameplayTraceSample sample;
    sample.core.boundaryIndex = context.simulationTick + 1;
    sample.core.simulationTick = context.simulationTick;
    sample.core.tapeFrame = context.tapeFrame;
    sample.core.flags = GameplayTraceSimulationTickValid;
    if (context.tapeFrame != GameplayTraceNoTapeFrame) {
        sample.core.flags |= GameplayTraceTapeFrameValid;
    }
    sample.core.phase = GameplayTracePhase::PostSimulation;
    sample.core.boundaryKind = GameplayTraceBoundaryKind::Tick;
    if (context.tapeFrameApplied)
        sample.core.inputSource |= GameplayTraceInputTape;
    if (context.controllerFrameApplied)
        sample.core.inputSource |= GameplayTraceInputController;

    if (wants(Channel::Stage)) {
        sample.stageStatus = Status::Present;
        copyName(sample.stage.stageName, dComIfGp_getStartStageName());
        sample.stage.room = static_cast<std::int8_t>(dComIfGp_roomControl_getStayNo());
        sample.stage.layer = static_cast<std::int8_t>(dComIfG_play_c::getLayerNo(0));
        sample.stage.point = dComIfGp_getStartStagePoint();
        copyName(sample.stage.nextStageName, dComIfGp_getNextStageName());
        sample.stage.nextRoom = static_cast<std::int8_t>(dComIfGp_getNextStageRoomNo());
        sample.stage.nextLayer = static_cast<std::int8_t>(dComIfGp_getNextStageLayer());
        sample.stage.nextPoint = dComIfGp_getNextStagePoint();
        if (dComIfGp_isEnableNextStage() != 0) {
            sample.stage.flags |= GameplayTraceNextStageEnabled;
        }
    }

    if (wants(Channel::AppliedPads)) {
        sample.appliedPadsStatus = Status::Present;
        sample.appliedPads.ownedPorts = context.tapeFrameApplied       ? context.tapeOwnedPorts :
                                        context.controllerFrameApplied ? 1 :
                                                                         0;
        for (std::size_t port = 0; port < kInputPortCount; ++port) {
            auto& pad = sample.appliedPads.pads[port];
            pad = raw_pad_state_from_pad_status(JUTGamePad::mPadStatus[port]);
            if (has_flag(pad.flags, RawPadFlags::Connected)) {
                sample.appliedPads.validPorts |= static_cast<std::uint8_t>(1u << port);
            }
        }
    }

    if (wants(Channel::Event)) {
        sample.eventStatus = Status::Present;
        if (dComIfGp_event_runCheck() != 0)
            sample.event.flags |= GameplayTraceEventRunning;
        const dEvt_control_c* event = dComIfGp_getEvent();
        sample.event.eventId = event->mEventId;
        sample.event.mode = event->getMode();
        sample.event.status = event->mEventStatus;
        sample.event.mapToolId = event->getMapToolId();
        // getRunEventName() is logically read-only today but has a non-const
        // gameplay API and walks private manager state. The observer contract
        // deliberately does not call it. Event-name identity remains explicitly
        // absent until an audited const data path exists.
    }

    if (wants(Channel::Rng)) {
        sample.rngStatus = Status::Present;
        sample.rng = capture_game_rng_snapshot();
    }

    if (wants(Channel::Camera)) {
        const auto* camera = static_cast<const camera_process_class*>(dComIfGp_getCamera(0));
        sample.cameraStatus = camera == nullptr ? Status::Absent : Status::Unavailable;
        if (camera != nullptr) {
            const auto finite = [](const cXyz& value) {
                return std::isfinite(value.x) && std::isfinite(value.y) && std::isfinite(value.z);
            };
            const cXyz eyeToCenter = camera->view.lookat.center - camera->view.lookat.eye;
            const bool realizedView =
                finite(camera->view.lookat.eye) && finite(camera->view.lookat.center) &&
                finite(camera->view.lookat.up) && std::isfinite(camera->view.fovy) &&
                camera->view.fovy >= 1.0f && camera->view.fovy <= 179.0f &&
                eyeToCenter.abs2() > 0.001f && camera->view.lookat.up.abs2() > 0.001f;
            if (realizedView) {
                sample.cameraStatus = Status::Present;
                sample.camera.viewYaw = camera->angle.y;
                sample.camera.controlledYaw = static_cast<std::int16_t>(camera->mCamera.U2());
                sample.camera.bank = camera->view.bank;
                sample.camera.eye = {camera->view.lookat.eye.x, camera->view.lookat.eye.y,
                    camera->view.lookat.eye.z};
                sample.camera.center = {camera->view.lookat.center.x, camera->view.lookat.center.y,
                    camera->view.lookat.center.z};
                sample.camera.up = {
                    camera->view.lookat.up.x, camera->view.lookat.up.y, camera->view.lookat.up.z};
                sample.camera.fovy = camera->view.fovy;
            }
        }
    }

    const bool wantsPlayer =
        wants(Channel::PlayerMotion) || wants(Channel::PlayerAction) || wants(Channel::SceneExit);
    const fopAc_ac_c* player = wantsPlayer ? dComIfGp_getPlayer(0) : nullptr;
    const bool playerIsLink = player != nullptr && fopAcM_GetName(player) == fpcNm_ALINK_e;
    const auto* link = playerIsLink ? static_cast<const daAlink_c*>(player) : nullptr;

    if (wants(Channel::PlayerMotion)) {
        sample.playerMotionStatus = player == nullptr ? Status::Absent : Status::Present;
        if (player != nullptr) {
            sample.playerMotion.sessionProcessId = static_cast<std::uint32_t>(fopAcM_GetID(player));
            sample.playerMotion.actorName = fopAcM_GetName(player);
            sample.playerMotion.procedureId = link == nullptr ? 0xffff : link->mProcID;
            sample.playerMotion.currentAngle = {
                player->current.angle.x, player->current.angle.y, player->current.angle.z};
            sample.playerMotion.shapeAngle = {
                player->shape_angle.x, player->shape_angle.y, player->shape_angle.z};
            sample.playerMotion.position = {
                player->current.pos.x, player->current.pos.y, player->current.pos.z};
            sample.playerMotion.velocity = {player->speed.x, player->speed.y, player->speed.z};
            sample.playerMotion.forwardSpeed = player->speedF;
            if (playerIsLink)
                sample.playerMotion.flags |= GameplayTracePlayerIsLink;
        }
    }

    if (wants(Channel::PlayerAction)) {
        sample.playerActionStatus = link == nullptr ? Status::Absent : Status::Present;
        if (link != nullptr) {
            auto& action = sample.playerAction;
            action.procedureId = link->mProcID;
            action.modeFlags = link->mModeFlg;
            action.procedureContextRaw = {
                link->mProcVar0.field_0x3008,
                link->mProcVar1.field_0x300a,
                link->mProcVar2.field_0x300c,
                link->mProcVar3.field_0x300e,
                link->mProcVar4.field_0x3010,
                link->mProcVar5.field_0x3012,
            };
            action.damageWaitTimer = link->getDamageWaitTimer();
            action.swordAtUpTime = link->getSwordAtUpTime();
            action.iceDamageWaitTimer = link->mIceDamageWaitTimer;
            action.swordChangeWaitTimer = link->getSwordChangeWaitTimer();
            for (std::size_t index = 0; index < 3; ++index) {
                action.underAnimations[index] = {
                    .resourceId = link->mUnderAnmHeap[index].getIdx(),
                    .frame = link->mUnderFrameCtrl[index].getFrame(),
                    .rate = link->mUnderFrameCtrl[index].getRate(),
                };
                action.upperAnimations[index] = {
                    .resourceId = link->mUpperAnmHeap[index].getIdx(),
                    .frame = link->mUpperFrameCtrl[index].getFrame(),
                    .rate = link->mUpperFrameCtrl[index].getRate(),
                };
            }
        }
    }

    if (wants(Channel::SceneExit)) {
        sample.sceneExitStatus = Status::Absent;
        if (player != nullptr) {
            struct NearestSceneExit {
                const cXyz* playerPosition;
                const fopAc_ac_c* actor;
                float distanceSquared;
            } nearest{&player->current.pos, nullptr, std::numeric_limits<float>::max()};
            fopAcIt_Executor(
                [](void* candidate, void* capture) -> int {
                    const auto* actor = static_cast<const fopAc_ac_c*>(candidate);
                    auto* nearest = static_cast<NearestSceneExit*>(capture);
                    const s16 actorName = fopAcM_GetName(actor);
                    if (actorName != fpcNm_SCENE_EXIT_e && actorName != fpcNm_SCENE_EXIT2_e) {
                        return 1;
                    }
                    const float distance = actor->current.pos.abs2(*nearest->playerPosition);
                    if (distance < nearest->distanceSquared) {
                        nearest->actor = actor;
                        nearest->distanceSquared = distance;
                    }
                    return 1;
                },
                &nearest);
            if (nearest.actor != nullptr) {
                sample.sceneExitStatus = Status::Present;
                sample.sceneExit.sessionProcessId =
                    static_cast<std::uint32_t>(fopAcM_GetID(nearest.actor));
                sample.sceneExit.actorName = fopAcM_GetName(nearest.actor);
                sample.sceneExit.position = {nearest.actor->current.pos.x,
                    nearest.actor->current.pos.y, nearest.actor->current.pos.z};
                sample.sceneExit.distance = std::sqrt(nearest.distanceSquared);
            }
        }
    }

    recorder.record(sample);
#else
    (void)context;
#endif
}

}  // namespace dusk::automation
