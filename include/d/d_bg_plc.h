#ifndef D_BG_D_BG_PLC_H
#define D_BG_D_BG_PLC_H

#include "d/d_bg_pc.h"

#if DUSK_ENABLE_AUTOMATION_OBSERVERS
// DUSKLIGHT OBSERVATION-ONLY APERTURE: declaration only; adapter body lives in automation.
namespace dusk::automation {
struct GameplayTraceCollisionReadAdapter;
}
#endif

struct sBgPlc {
    /* 0x0 */ BE(u32) magic;        // "SPLC"
    /* 0x4 */ BE(u16) m_code_size;  // Should normally always be 0x14
    /* 0x6 */ BE(u16) m_num;        // Number of sBgPc entries to follow
    /* 0x8 */ sBgPc m_code[0];  // m_num size array
};

class dBgPlc {
public:
    dBgPlc();
    ~dBgPlc();
    void setBase(void*);
    sBgPc* getCode(int, sBgPc**) const;
    u32 getGrpCode(int) const;

    static const int ZELDA_CODE_SIZE = sizeof(sBgPc);

private:
#if DUSK_ENABLE_AUTOMATION_OBSERVERS
    // DUSKLIGHT OBSERVATION-ONLY APERTURE: const backing-store reads only.
    friend struct dusk::automation::GameplayTraceCollisionReadAdapter;
#endif
    /* 0x00 */ sBgPlc* m_base;
};

#endif /* D_BG_D_BG_PLC_H */
