#include "dusk/automation/actor_profile_catalog.hpp"

#include "f_op/f_op_actor.h"
#include "f_pc/f_pc_leaf.h"
#include "f_pc/f_pc_name.h"
#include "f_pc/f_pc_profile_lst.h"

#include <array>
#include <cstdint>
#include <fstream>
#include <string>
#include <system_error>
#include <type_traits>
#include <vector>

#include <nlohmann/json.hpp>
#include <xxhash.h>

namespace dusk::automation {
namespace {

constexpr std::string_view Schema = "dusklight-actor-profile-catalog/v1";

struct ProfileEntry {
    std::uint32_t slot = 0;
    bool present = false;
    std::uint32_t layerId = 0;
    std::uint16_t listId = 0;
    std::uint16_t listPriority = 0;
    std::int16_t profileName = 0;
    std::uint32_t processSize = 0;
    std::uint32_t auxiliarySize = 0;
    std::uint32_t parameters = 0;
    bool leaf = false;
    std::int16_t drawPriority = 0;
    bool actor = false;
    std::uint32_t status = 0;
    std::uint8_t group = 0;
    std::uint8_t cullType = 0;
};

template <typename T>
void append_integer(std::vector<std::uint8_t>& output, const T value) {
    using U = std::make_unsigned_t<T>;
    U bits = static_cast<U>(value);
    for (std::size_t index = 0; index < sizeof(T); ++index) {
        output.push_back(static_cast<std::uint8_t>(bits & 0xff));
        bits >>= 8;
    }
}

std::vector<ProfileEntry> capture_entries() {
    std::vector<ProfileEntry> entries;
    entries.reserve(fpcNm_MAX_NUM);
    for (std::uint32_t slot = 0; slot < static_cast<std::uint32_t>(fpcNm_MAX_NUM); ++slot) {
        const process_profile_definition* profile = g_fpcPfLst_ProfileList[slot];
        ProfileEntry entry{.slot = slot, .present = profile != nullptr};
        if (profile != nullptr) {
            entry.layerId = profile->layer_id;
            entry.listId = profile->list_id;
            entry.listPriority = profile->list_priority;
            entry.profileName = profile->name;
            entry.processSize = profile->process_size;
            entry.auxiliarySize = profile->unk_size;
            entry.parameters = profile->parameters;

            // Actor profiles are leaf profiles whose leaf method table is the
            // common actor method table. Pointer values only classify the
            // static record; they are never retained or hashed.
            entry.leaf = profile->methods == &g_fpcLf_Method.base;
            const auto* leaf = entry.leaf
                ? reinterpret_cast<const leaf_process_profile_definition*>(profile)
                : nullptr;
            entry.drawPriority = leaf == nullptr ? 0 : leaf->priority;
            entry.actor = leaf != nullptr && leaf->sub_method == &g_fopAc_Method.base;
            if (entry.actor) {
                const auto* actor =
                    reinterpret_cast<const actor_process_profile_definition*>(profile);
                entry.status = actor->status;
                entry.group = actor->group;
                entry.cullType = actor->cullType;
            }
        }
        entries.push_back(entry);
    }
    return entries;
}

std::string build_identity(const std::vector<ProfileEntry>& entries) {
    std::vector<std::uint8_t> canonical;
    canonical.reserve(64 + entries.size() * 48);
    canonical.insert(canonical.end(), Schema.begin(), Schema.end());
    canonical.push_back(0);
    append_integer(canonical, static_cast<std::uint32_t>(entries.size()));

    for (const ProfileEntry& entry : entries) {
        append_integer(canonical, entry.slot);
        canonical.push_back(entry.present ? 1 : 0);
        if (!entry.present)
            continue;
        append_integer(canonical, entry.layerId);
        append_integer(canonical, entry.listId);
        append_integer(canonical, entry.listPriority);
        append_integer(canonical, entry.profileName);
        append_integer(canonical, entry.processSize);
        append_integer(canonical, entry.auxiliarySize);
        append_integer(canonical, entry.parameters);
        canonical.push_back(entry.leaf ? 1 : 0);
        append_integer(canonical, entry.drawPriority);
        canonical.push_back(entry.actor ? 1 : 0);
        if (entry.actor) {
            append_integer(canonical, entry.status);
            canonical.push_back(entry.group);
            canonical.push_back(entry.cullType);
        }
    }

    const XXH128_hash_t hash = XXH3_128bits(canonical.data(), canonical.size());
    XXH128_canonical_t digest{};
    XXH128_canonicalFromHash(&digest, hash);
    constexpr std::array Hex{'0', '1', '2', '3', '4', '5', '6', '7',
        '8', '9', 'a', 'b', 'c', 'd', 'e', 'f'};
    std::string identity = "actor-profile-catalog:xxh3-128:";
    identity.reserve(identity.size() + sizeof(digest.digest) * 2);
    for (const std::uint8_t byte : digest.digest) {
        identity.push_back(Hex[byte >> 4]);
        identity.push_back(Hex[byte & 0x0f]);
    }
    return identity;
}

}  // namespace

std::string_view actor_profile_catalog_identity() {
    static const std::string Identity = build_identity(capture_entries());
    return Identity;
}

bool write_actor_profile_catalog(const std::filesystem::path& path, std::string& error) {
    error.clear();
    if (path.empty()) {
        error = "actor profile catalog path is empty";
        return false;
    }
    const std::vector<ProfileEntry> entries = capture_entries();
    nlohmann::ordered_json profiles = nlohmann::ordered_json::array();
    for (const ProfileEntry& entry : entries) {
        profiles.push_back({
            {"slot", entry.slot},
            {"present", entry.present},
            {"layer_id", entry.present ? nlohmann::ordered_json(entry.layerId) : nullptr},
            {"list_id", entry.present ? nlohmann::ordered_json(entry.listId) : nullptr},
            {"list_priority",
                entry.present ? nlohmann::ordered_json(entry.listPriority) : nullptr},
            {"profile_name",
                entry.present ? nlohmann::ordered_json(entry.profileName) : nullptr},
            {"process_size",
                entry.present ? nlohmann::ordered_json(entry.processSize) : nullptr},
            {"auxiliary_size",
                entry.present ? nlohmann::ordered_json(entry.auxiliarySize) : nullptr},
            {"parameters",
                entry.present ? nlohmann::ordered_json(entry.parameters) : nullptr},
            {"is_leaf", entry.present ? nlohmann::ordered_json(entry.leaf) : nullptr},
            {"draw_priority",
                entry.present && entry.leaf
                    ? nlohmann::ordered_json(entry.drawPriority) : nullptr},
            {"is_actor", entry.present ? nlohmann::ordered_json(entry.actor) : nullptr},
            {"status", entry.actor ? nlohmann::ordered_json(entry.status) : nullptr},
            {"group", entry.actor ? nlohmann::ordered_json(entry.group) : nullptr},
            {"cull_type", entry.actor ? nlohmann::ordered_json(entry.cullType) : nullptr},
        });
    }
    const nlohmann::ordered_json document{
        {"schema", Schema},
        {"identity", build_identity(entries)},
        {"profiles", std::move(profiles)},
    };
    const std::string bytes = document.dump() + '\n';
    std::error_code filesystemError;
    if (const auto parent = path.parent_path(); !parent.empty()) {
        std::filesystem::create_directories(parent, filesystemError);
        if (filesystemError) {
            error = filesystemError.message();
            return false;
        }
    }
    const std::filesystem::path temporary = path.string() + ".tmp";
    std::filesystem::remove(temporary, filesystemError);
    filesystemError.clear();
    {
        std::ofstream stream(temporary, std::ios::binary | std::ios::trunc);
        if (!stream || !stream.write(bytes.data(), static_cast<std::streamsize>(bytes.size()))) {
            error = "could not write actor profile catalog temporary artifact";
            return false;
        }
    }
    std::filesystem::rename(temporary, path, filesystemError);
    if (filesystemError) {
        error = filesystemError.message();
        return false;
    }
    return true;
}

}  // namespace dusk::automation
