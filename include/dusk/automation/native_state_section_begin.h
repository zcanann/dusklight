#pragma once

// Build-system-only isolation for native globals owned by original game
// translation units. The source files themselves remain untouched.
#if defined(_MSC_VER)
#pragma section(".dskdat$m", read, write)
#pragma section(".dskbss$m", read, write)
#pragma data_seg(".dskdat$m")
#pragma bss_seg(".dskbss$m")
#elif defined(__clang__)
#pragma clang section bss="dusk_game_bss" data="dusk_game_data"
#endif
