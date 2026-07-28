/**
 * d_door_param2.cpp
 *
 */

#include "d/dolzel.h" // IWYU pragma: keep

#include "d/d_door_param2.h"
#include "f_op/f_op_actor_mng.h"

int door_param2_c::getKind(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 0, 5);
}

u32 door_param2_c::getDoorModel(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 5, 3);
}

u8 door_param2_c::getFrontOption(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 8, 2);
}

u8 door_param2_c::getBackOption(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 10, 3);
}

u8 door_param2_c::getFRoomNo(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 13, 6);
}

u8 door_param2_c::getBRoomNo(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 19, 6);
}

u8 door_param2_c::getSwbit(const fopAc_ac_c* i_actor) {
    return i_actor->home.angle.z & 0xFF;
}

u8 door_param2_c::getSwbit2(const fopAc_ac_c* i_actor) {
    return (i_actor->home.angle.z >> 8) & 0xFF;
}

u8 door_param2_c::getSwbit3(const fopAc_ac_c* i_actor) {
    return (i_actor->home.angle.x >> 8) & 0xFF;
}

int door_param2_c::isMsgDoor(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 31, 1);
}

u8 door_param2_c::getEventNo(const fopAc_ac_c* i_actor) {
    return i_actor->home.angle.x & 0xFF;
}

u8 door_param2_c::getEventNo2(const fopAc_ac_c* i_actor) {
    return (i_actor->home.angle.x >> 8) & 0xFF;
}

u16 door_param2_c::getMsgNo(const fopAc_ac_c* i_actor) {
    return i_actor->home.angle.x & 0xFFFF;
}

u8 door_param2_c::getExitNo(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 25, 6);
}

u32 door_param2_c::getFLightInf(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 25, 3) & 0xFF;
}

u32 door_param2_c::getBLightInf(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 28, 3) & 0xFF;
}

u32 door_param2_c::getMFLightInf(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 19, 3) & 0xFF;
}

u32 door_param2_c::getMBLightInf(const fopAc_ac_c* i_actor) {
    return fopAcM_GetParamBit(i_actor, 22, 3) & 0xFF;
}
