#pragma once

#include <filesystem>
#include <string>
#include <string_view>

namespace dusk::automation {

/**
 * Content identity for the immutable process/actor profile table compiled into
 * this executable. Pointer-valued method tables are deliberately excluded;
 * every stable scalar profile field and the table slot are included.
 */
[[nodiscard]] std::string_view actor_profile_catalog_identity();

/** Writes the canonical pointer-free table without touching gameplay state. */
[[nodiscard]] bool write_actor_profile_catalog(
    const std::filesystem::path& path, std::string& error);

}  // namespace dusk::automation
