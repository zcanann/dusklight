#ifndef D_D_DOOR_PARAM2_H
#define D_D_DOOR_PARAM2_H

#include <types.h>

class fopAc_ac_c;

class door_param2_c {
public:
    static int getKind(const fopAc_ac_c* i_actor);
    static u32 getDoorModel(const fopAc_ac_c* i_actor);
    static u8 getFrontOption(const fopAc_ac_c* i_actor);
    static u8 getBackOption(const fopAc_ac_c* i_actor);
    static u8 getFRoomNo(const fopAc_ac_c* i_actor);
    static u8 getBRoomNo(const fopAc_ac_c* i_actor);
    static u8 getSwbit(const fopAc_ac_c* i_actor);
    static u8 getSwbit2(const fopAc_ac_c* i_actor);
    static u8 getSwbit3(const fopAc_ac_c* i_actor);
    static int isMsgDoor(const fopAc_ac_c* i_actor);
    static u8 getEventNo(const fopAc_ac_c* i_actor);
    static u8 getEventNo2(const fopAc_ac_c* i_actor);
    static u16 getMsgNo(const fopAc_ac_c* i_actor);
    static u8 getExitNo(const fopAc_ac_c* i_actor);
    static u32 getFLightInf(const fopAc_ac_c* i_actor);
    static u32 getBLightInf(const fopAc_ac_c* i_actor);
    static u32 getMFLightInf(const fopAc_ac_c* i_actor);
    static u32 getMBLightInf(const fopAc_ac_c* i_actor);
};

#endif /* D_D_DOOR_PARAM2_H */
