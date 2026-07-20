#include "dusk/automation/native_state_region.hpp"

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstring>
#include <string_view>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <Windows.h>
#elif defined(__APPLE__)
#include <mach-o/getsect.h>
#include <mach-o/ldsyms.h>
#elif defined(__ELF__) && defined(__clang__)
extern "C" {
extern std::byte __start_dusk_game_data[];
extern std::byte __stop_dusk_game_data[];
extern std::byte __start_dusk_game_bss[];
extern std::byte __stop_dusk_game_bss[];
}
#endif

namespace dusk::automation {

NativeGameStateRegions native_game_state_regions() {
    NativeGameStateRegions result;
#if defined(_WIN32)
    const auto* base = reinterpret_cast<const std::byte*>(GetModuleHandleW(nullptr));
    if (base == nullptr) {
        return result;
    }
    const auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(base);
    if (dos->e_magic != IMAGE_DOS_SIGNATURE || dos->e_lfanew <= 0) {
        return result;
    }
    const auto* nt = reinterpret_cast<const IMAGE_NT_HEADERS*>(base + dos->e_lfanew);
    if (nt->Signature != IMAGE_NT_SIGNATURE) {
        return result;
    }
    constexpr std::array Names{std::string_view{".dskdat"}, std::string_view{".dskbss"}};
    const IMAGE_SECTION_HEADER* section = IMAGE_FIRST_SECTION(nt);
    for (std::uint16_t index = 0; index < nt->FileHeader.NumberOfSections; ++index, ++section) {
        char rawName[IMAGE_SIZEOF_SHORT_NAME + 1]{};
        std::memcpy(rawName, section->Name, IMAGE_SIZEOF_SHORT_NAME);
        const std::string_view name(rawName);
        if (std::ranges::find(Names, name) == Names.end() || section->Misc.VirtualSize == 0) {
            continue;
        }
        if (result.count == result.items.size()) {
            return {};
        }
        result.items[result.count++] = {
            const_cast<std::byte*>(base) + section->VirtualAddress,
            static_cast<std::size_t>(section->Misc.VirtualSize),
        };
    }
#elif defined(__APPLE__)
    const auto appendSection = [&result](const char* const sectionName) {
        unsigned long size = 0;
        auto* const bytes = getsectiondata(&_mh_execute_header, "__DATA", sectionName, &size);
        if (bytes == nullptr || size == 0 || result.count == result.items.size()) {
            return false;
        }
        result.items[result.count++] = {
            reinterpret_cast<std::byte*>(bytes), static_cast<std::size_t>(size)};
        return true;
    };
    if (!appendSection("dusk_game_data") || !appendSection("dusk_game_bss")) {
        return {};
    }
#elif defined(__ELF__) && defined(__clang__)
    result.items[result.count++] = {__start_dusk_game_data,
        static_cast<std::size_t>(__stop_dusk_game_data - __start_dusk_game_data)};
    result.items[result.count++] = {__start_dusk_game_bss,
        static_cast<std::size_t>(__stop_dusk_game_bss - __start_dusk_game_bss)};
#endif
    return result;
}

}  // namespace dusk::automation
