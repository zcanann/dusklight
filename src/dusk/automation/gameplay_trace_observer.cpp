#include "dusk/automation/gameplay_trace_observer.hpp"

#include "dusk/automation/gameplay_trace.hpp"

#if DUSK_ENABLE_AUTOMATION_OBSERVERS

#include "JSystem/JUtility/JUTGamePad.h"
#include "d/actor/d_a_alink.h"
#include "d/actor/d_a_scene_exit.h"
#include "d/actor/d_a_scene_exit2.h"
#include "d/d_com_inf_game.h"
#include "f_op/f_op_actor_iter.h"
#include "f_op/f_op_actor_mng.h"
#include "f_op/f_op_camera_mng.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstring>
#include <limits>
#include <tuple>

#endif

namespace dusk::automation {

#if DUSK_ENABLE_AUTOMATION_OBSERVERS

struct GameplayTraceCollisionReadAdapter {
    static u32 flags(const dBgS_Acch& value) { return value.m_flags; }
    static u32 wallFlags(const dBgS_AcchCir& value) { return value.m_flags; }
    static s16 wallAngleY(const dBgS_AcchCir& value) { return value.m_wall_angle_y; }
    static float waterHeight(const dBgS_SplGrpChk& value) { return value.m_height; }
    static const cM3dGPla& groundPlane(const dBgS_Acch& value) { return value.field_0xa0; }
    static const cXyz* oldPosition(const dBgS_Acch& value) { return value.pm_old_pos; }
};

namespace {

bool finite_xyz(const cXyz& value) {
    return std::isfinite(value.x) && std::isfinite(value.y) && std::isfinite(value.z);
}

bool copy_plane(const cM3dGPla& plane, std::array<float, 4>& output) {
    if (!finite_xyz(plane.mNormal) || !std::isfinite(plane.GetD()))
        return false;
    output = {plane.mNormal.x, plane.mNormal.y, plane.mNormal.z, plane.GetD()};
    return true;
}

bool copy_poly_identity(const cBgS_PolyInfo& poly, std::uint16_t& bgIndex, std::uint16_t& polyIndex,
    std::uint32_t& ownerSessionProcessId) {
    if (!poly.ChkSetInfo())
        return false;
    const int bg = poly.GetBgIndex();
    const int polygon = poly.GetPolyIndex();
    if (bg < 0 || bg >= 256 || polygon < 0 || polygon >= 0xffff)
        return false;
    bgIndex = static_cast<std::uint16_t>(bg);
    polyIndex = static_cast<std::uint16_t>(polygon);

    const dBgS& background = dComIfG_Bgsp();
    const cBgS_ChkElm& element = background.m_chk_element[bg];
    if (!element.ChkUsed() || background.GetBgWBasePointer(poly) == nullptr) {
        return true;
    }
    if (!poly.ChkSafe(element.m_bgw_base_ptr, element.m_actor_id))
        return true;
    const fopAc_ac_c* owner = background.cBgS::GetActorPointer(bg);
    if (owner == nullptr)
        return true;
    ownerSessionProcessId = static_cast<std::uint32_t>(fopAcM_GetID(owner));
    return true;
}

const stage_scls_info_dummy_class* loaded_scls_for_room(const std::int8_t room) {
    if (room == -1) {
        const dComIfG_play_c& play = g_dComIfG_gameInfo.play;
        return play.mStageData.getSclsInfo();
    }
    if (room < 0 || room >= 64)
        return nullptr;
    return dStage_roomControl_c::mStatus[room].mRoomDt.getSclsInfo();
}

void copy_destination(GameplayTraceSceneExitSample& output, const std::int8_t sclsRoom) {
    const stage_scls_info_dummy_class* table = loaded_scls_for_room(sclsRoom);
    if (table == nullptr || table->m_entries == nullptr || table->num <= 0 || table->num > 256 ||
        output.exitId >= table->num)
        return;
    const stage_scls_info_class& destination = table->m_entries[output.exitId];
    std::memcpy(output.destinationStage.data(), destination.mStage, output.destinationStage.size());
    output.destinationRoom = destination.mRoom;
    output.destinationLayer = static_cast<std::int8_t>(destination.field_0xb & 0x0f);
    if (output.destinationLayer >= 15)
        output.destinationLayer = -1;
    output.destinationPoint = destination.mStart;
    output.destinationWipe = destination.mWipe == 15 ? 0 : destination.mWipe;
    output.destinationWipeTime = (destination.field_0xb >> 5) & 7;
    output.destinationTimeHour = static_cast<std::int8_t>(
        ((destination.field_0xa >> 4) & 0x0f) | (destination.field_0xb & 0x10));
    if (output.destinationTimeHour >= 31)
        output.destinationTimeHour = -1;
    output.flags |= GameplayTraceSceneExitDestinationValid;
}

float box_signed_distance(const cXyz& local, const cXyz& extent, bool& inside) {
    inside = local.y >= 0.0f && local.y <= extent.y && std::fabs(local.x) <= extent.x &&
             std::fabs(local.z) <= extent.z;
    if (inside) {
        return -std::min({extent.x - std::fabs(local.x), local.y, extent.y - local.y,
            extent.z - std::fabs(local.z)});
    }
    const float dx = std::max(std::fabs(local.x) - extent.x, 0.0f);
    const float dy = std::max(std::max(-local.y, local.y - extent.y), 0.0f);
    const float dz = std::max(std::fabs(local.z) - extent.z, 0.0f);
    return std::sqrt(dx * dx + dy * dy + dz * dz);
}

}  // namespace

#endif

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

    const bool wantsPlayer = wants(Channel::PlayerMotion) || wants(Channel::PlayerAction) ||
                             wants(Channel::SceneExit) || wants(Channel::PlayerBackgroundCollision);
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
        if (player != nullptr && link == nullptr) {
            sample.sceneExitStatus = Status::Unavailable;
        } else if (link != nullptr) {
            struct SceneExitSelection {
                const daAlink_c* link;
                GameplayTraceSceneExitSample selected{};
                bool hasSelection = false;
                bool latched = false;
                bool changeOk = false;
                bool inside = false;
                float signedDistance = std::numeric_limits<float>::max();
                std::uint32_t processId = 0xffffffffu;
                std::uint8_t observedCount = 0;
                bool observedCountSaturated = false;
            } selection{link};
            fopAcIt_Executor(
                [](void* candidate, void* capture) -> int {
                    const auto* actor = static_cast<const fopAc_ac_c*>(candidate);
                    auto* selection = static_cast<SceneExitSelection*>(capture);
                    const s16 actorName = fopAcM_GetName(actor);
                    if (actorName != fpcNm_SCENE_EXIT_e && actorName != fpcNm_SCENE_EXIT2_e) {
                        return 1;
                    }
                    if (selection->observedCount != 0xff)
                        ++selection->observedCount;
                    else
                        selection->observedCountSaturated = true;

                    GameplayTraceSceneExitSample observed;
                    observed.sessionProcessId = static_cast<std::uint32_t>(fopAcM_GetID(actor));
                    observed.rawParameters = fopAcM_GetParam(actor);
                    observed.actorName = actorName;
                    observed.setId = static_cast<std::uint16_t>(fopAcM_GetSetId(actor));
                    observed.exitId = observed.rawParameters & 0xff;
                    observed.homeRoom = actor->home.roomNo;
                    observed.shapeYaw = actor->shape_angle.y;
                    observed.homePosition = {
                        actor->home.pos.x, actor->home.pos.y, actor->home.pos.z};
                    bool latched = false;
                    bool changeOk = false;
                    bool inside = false;
                    bool valid = false;
                    std::int8_t sclsRoom = actor->current.roomNo;
                    if (actorName == fpcNm_SCENE_EXIT_e) {
                        const auto* exit = static_cast<const daScex_c*>(actor);
                        observed.kind = GameplayTraceSceneExitBox;
                        observed.argument1 = (observed.rawParameters >> 8) & 0xff;
                        observed.pathId = (observed.rawParameters >> 16) & 0xff;
                        observed.switchNo = (observed.rawParameters >> 24) & 0xff;
                        observed.volumeExtent = {actor->scale.x, actor->scale.y, actor->scale.z};
                        const cXyz& position = selection->link->current.pos;
                        cXyz local;
                        local.x = exit->mMatrix[0][0] * position.x +
                                  exit->mMatrix[0][1] * position.y +
                                  exit->mMatrix[0][2] * position.z + exit->mMatrix[0][3];
                        local.y = exit->mMatrix[1][0] * position.x +
                                  exit->mMatrix[1][1] * position.y +
                                  exit->mMatrix[1][2] * position.z + exit->mMatrix[1][3];
                        local.z = exit->mMatrix[2][0] * position.x +
                                  exit->mMatrix[2][1] * position.y +
                                  exit->mMatrix[2][2] * position.z + exit->mMatrix[2][3];
                        observed.playerLocalPosition = {local.x, local.y, local.z};
                        valid = finite_xyz(local) && finite_xyz(actor->scale) &&
                                finite_xyz(actor->home.pos) && actor->scale.x >= 0.0f &&
                                actor->scale.y >= 0.0f && actor->scale.z >= 0.0f;
                        if (valid) {
                            observed.signedDistanceToVolume =
                                box_signed_distance(local, actor->scale, inside);
                            valid = std::isfinite(observed.signedDistanceToVolume);
                        }
                        latched = selection->link->mpScnChg == exit;
                        changeOk = exit->mSceneChangeOK && latched;
                        sclsRoom = selection->link->current.roomNo;
                    } else {
                        const auto* exit = static_cast<const daScExit_c*>(actor);
                        observed.kind = GameplayTraceSceneExitRadialXz;
                        observed.actorAction = exit->mAction;
                        observed.volumeExtent = {exit->mRadius, 0.0f, exit->mRadius};
                        const cXyz local = selection->link->current.pos - actor->current.pos;
                        observed.playerLocalPosition = {local.x, local.y, local.z};
                        valid = finite_xyz(local) && finite_xyz(actor->home.pos) &&
                                std::isfinite(exit->mRadius) && exit->mRadius >= 0.0f;
                        if (valid) {
                            const float radialDistance =
                                std::sqrt(local.x * local.x + local.z * local.z);
                            observed.signedDistanceToVolume = radialDistance - exit->mRadius;
                            inside = radialDistance < exit->mRadius;
                            valid = std::isfinite(observed.signedDistanceToVolume);
                        }
                    }
                    if (!valid)
                        return 1;

                    observed.flags |= GameplayTraceSceneExitVolumeValid;
                    if (inside)
                        observed.flags |= GameplayTraceSceneExitPlayerInside;
                    if (latched) {
                        observed.flags |= GameplayTraceSceneExitPlayerLatched;
                        observed.linkExitDirection = selection->link->mExitDirection;
                        observed.linkExitId = selection->link->mExitID;
                        if (selection->link->checkSceneChangeAreaStart())
                            observed.flags |= GameplayTraceSceneExitChangeStarted;
                    }
                    if (changeOk)
                        observed.flags |= GameplayTraceSceneExitChangeOk;
                    copy_destination(observed, sclsRoom);

                    bool better = !selection->hasSelection;
                    if (selection->hasSelection) {
                        if (latched != selection->latched)
                            better = latched;
                        else if (changeOk != selection->changeOk)
                            better = changeOk;
                        else if (inside != selection->inside)
                            better = inside;
                        else if (observed.signedDistanceToVolume != selection->signedDistance) {
                            better = observed.signedDistanceToVolume < selection->signedDistance;
                        } else {
                            const auto stableKey = [](const GameplayTraceSceneExitSample& value) {
                                return std::tuple{value.kind, value.setId, value.rawParameters,
                                    value.homeRoom, value.shapeYaw, value.homePosition[0],
                                    value.homePosition[1], value.homePosition[2]};
                            };
                            const auto observedKey = stableKey(observed);
                            const auto selectedKey = stableKey(selection->selected);
                            better = observedKey != selectedKey ?
                                         observedKey < selectedKey :
                                         observed.sessionProcessId < selection->processId;
                        }
                    }
                    if (better) {
                        selection->selected = observed;
                        selection->hasSelection = true;
                        selection->latched = latched;
                        selection->changeOk = changeOk;
                        selection->inside = inside;
                        selection->signedDistance = observed.signedDistanceToVolume;
                        selection->processId = observed.sessionProcessId;
                    }
                    return 1;
                },
                &selection);
            if (selection.hasSelection) {
                sample.sceneExitStatus = Status::Present;
                sample.sceneExit = selection.selected;
                sample.sceneExit.observedCount = selection.observedCount;
                if (selection.observedCountSaturated) {
                    sample.sceneExit.flags |= GameplayTraceSceneExitObservedCountSaturated;
                }
            } else if (selection.observedCount != 0) {
                sample.sceneExitStatus = Status::Unavailable;
            }
        }
    }

    if (wants(Channel::PlayerBackgroundCollision)) {
        sample.playerBackgroundCollisionStatus = player == nullptr ? Status::Absent :
                                                 link == nullptr   ? Status::Unavailable :
                                                                     Status::Present;
        if (link != nullptr) {
            auto& output = sample.playerBackgroundCollision;
            const dBgS_Acch& acch = link->mLinkAcch;
            const u32 flags = GameplayTraceCollisionReadAdapter::flags(acch);
            const bool groundEnabled = (flags & dBgS_Acch::FLAG_GRND_NONE) == 0;
            const bool groundHeightValid = groundEnabled && std::isfinite(acch.GetGroundH()) &&
                                           acch.GetGroundH() != -G_CM3D_F_INF;
            const bool groundFind = (flags & dBgS_Acch::FLAG_GROUND_FIND) != 0;
            if (groundHeightValid) {
                output.flags |= GameplayTraceCollisionGroundProbeValid;
                output.groundHeight = acch.GetGroundH();
                if (copy_poly_identity(acch.m_gnd, output.groundBgIndex, output.groundPolyIndex,
                        output.groundOwnerSessionProcessId))
                {
                    output.flags |= GameplayTraceCollisionGroundIdentityPresent;
                }
                if (output.groundOwnerSessionProcessId != 0xffffffffu)
                    output.flags |= GameplayTraceCollisionGroundOwnerPresent;
            }
            if (groundHeightValid && (flags & dBgS_Acch::FLAG_GROUND_HIT) != 0)
                output.flags |= GameplayTraceCollisionGroundContact;
            if (groundHeightValid && (flags & dBgS_Acch::FLAG_GROUND_LANDING) != 0)
                output.flags |= GameplayTraceCollisionLanding;
            if ((flags & dBgS_Acch::FLAG_GROUND_AWAY) != 0)
                output.flags |= GameplayTraceCollisionAway;
            if (groundEnabled && groundFind &&
                (output.flags & GameplayTraceCollisionGroundContact) != 0)
            {
                if (copy_plane(
                        GameplayTraceCollisionReadAdapter::groundPlane(acch), output.groundPlane))
                {
                    output.flags |= GameplayTraceCollisionGroundPlaneValid;
                }
            }

            const bool wallEnabled = (flags & dBgS_Acch::FLAG_WALL_NONE) == 0;
            if (wallEnabled)
                output.flags |= GameplayTraceCollisionWallProbeEnabled;
            if (wallEnabled && (flags & dBgS_Acch::FLAG_WALL_HIT) != 0)
                output.flags |= GameplayTraceCollisionWallContact;
            for (std::size_t index = 0; wallEnabled && index < output.walls.size(); ++index) {
                const dBgS_AcchCir& realized = link->mAcchCir[index];
                auto& wall = output.walls[index];
                const u32 wallFlags = GameplayTraceCollisionReadAdapter::wallFlags(realized);
                if ((wallFlags & dBgS_AcchCir::FLAG_WALL_HIT) != 0) {
                    wall.flags |= GameplayTraceCollisionWallHit;
                    wall.angleY = GameplayTraceCollisionReadAdapter::wallAngleY(realized);
                    if (copy_poly_identity(
                            realized, wall.bgIndex, wall.polyIndex, wall.ownerSessionProcessId))
                    {
                        wall.flags |= GameplayTraceCollisionWallIdentityPresent;
                    }
                    if (wall.ownerSessionProcessId != 0xffffffffu)
                        wall.flags |= GameplayTraceCollisionWallOwnerPresent;
                }
            }

            const bool roofEnabled = (flags & dBgS_Acch::FLAG_ROOF_NONE) == 0;
            const bool roofValid = roofEnabled && std::isfinite(acch.GetRoofHeight()) &&
                                   acch.GetRoofHeight() != G_CM3D_F_INF;
            if (roofValid) {
                output.flags |= GameplayTraceCollisionRoofProbeValid;
                output.roofHeight = acch.GetRoofHeight();
                if (copy_poly_identity(acch.m_roof, output.roofBgIndex, output.roofPolyIndex,
                        output.roofOwnerSessionProcessId))
                {
                    output.flags |= GameplayTraceCollisionRoofIdentityPresent;
                }
                if (output.roofOwnerSessionProcessId != 0xffffffffu)
                    output.flags |= GameplayTraceCollisionRoofOwnerPresent;
                if ((flags & dBgS_Acch::FLAG_ROOF_HIT) != 0) {
                    output.flags |= GameplayTraceCollisionRoofContact;
                }
            }

            const bool waterEnabled = (flags & dBgS_Acch::FLAG_WATER_NONE) == 0;
            if (waterEnabled)
                output.flags |= GameplayTraceCollisionWaterProbeEnabled;
            if (waterEnabled && (flags & dBgS_Acch::FLAG_WATER_HIT) != 0) {
                const float waterHeight =
                    GameplayTraceCollisionReadAdapter::waterHeight(acch.m_wtr);
                if (std::isfinite(waterHeight) && waterHeight != -G_CM3D_F_INF) {
                    output.flags |= GameplayTraceCollisionWaterSurfaceFound;
                    output.waterHeight = waterHeight;
                    if (copy_poly_identity(acch.m_wtr, output.waterBgIndex, output.waterPolyIndex,
                            output.waterOwnerSessionProcessId))
                    {
                        output.flags |= GameplayTraceCollisionWaterIdentityPresent;
                    }
                    if (output.waterOwnerSessionProcessId != 0xffffffffu)
                        output.flags |= GameplayTraceCollisionWaterOwnerPresent;
                }
            }
            if ((output.flags & GameplayTraceCollisionWaterSurfaceFound) != 0 &&
                (flags & dBgS_Acch::FLAG_WATER_IN) != 0)
                output.flags |= GameplayTraceCollisionWaterIn;

            const cXyz finalPosition = link->current.pos;
            const cXyz* oldPosition = GameplayTraceCollisionReadAdapter::oldPosition(acch);
            if (oldPosition != nullptr && finite_xyz(*oldPosition) && finite_xyz(finalPosition)) {
                output.flags |= GameplayTraceCollisionTrajectoryValid;
                output.oldPosition = {oldPosition->x, oldPosition->y, oldPosition->z};
                output.finalPosition = {finalPosition.x, finalPosition.y, finalPosition.z};
                output.resolvedFrameDisplacement = {finalPosition.x - oldPosition->x,
                    finalPosition.y - oldPosition->y, finalPosition.z - oldPosition->z};
            }
        }
    }

    recorder.record(sample);
#else
    (void)context;
#endif
}

}  // namespace dusk::automation
