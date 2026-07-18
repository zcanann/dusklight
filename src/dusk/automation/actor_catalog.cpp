#include "dusk/automation/actor_catalog.hpp"

#if DUSK_ENABLE_AUTOMATION_OBSERVERS

#include "dusk/automation/build_identity.hpp"
#include "dusk/automation/name_entry_observer.hpp"

#include "d/d_com_inf_game.h"
#include "d/d_s_play.h"
#include "f_op/f_op_actor_iter.h"
#include "f_op/f_op_actor_mng.h"

#include <algorithm>
#include <array>
#include <cstdio>
#include <fstream>
#include <system_error>

#include <nlohmann/json.hpp>

namespace dusk::automation {
namespace {

using json = nlohmann::ordered_json;

struct ActorCatalogEntry {
    std::uint32_t processId = 0;
    std::uint32_t parentProcessId = 0;
    std::uint32_t parameters = 0;
    std::uint32_t status = 0;
    std::int16_t actorName = 0;
    std::int16_t profileName = 0;
    std::uint16_t setId = 0xffff;
    std::int16_t health = 0;
    std::int8_t homeRoom = -1;
    std::int8_t currentRoom = -1;
    std::uint8_t group = 0;
    std::int8_t argument = 0;
    std::array<char, 32> symbolicName{};
    cXyz homePosition{};
    cXyz currentPosition{};
};

struct ActorCatalogCapture {
    std::array<ActorCatalogEntry, ActorCatalogMaximumEntries> entries{};
    std::size_t count = 0;
    std::size_t observed = 0;
};

int capture_actor(void* candidate, void* context) {
    const auto* actor = static_cast<const fopAc_ac_c*>(candidate);
    auto* capture = static_cast<ActorCatalogCapture*>(context);
    ++capture->observed;

    ActorCatalogEntry entry{
        .processId = static_cast<std::uint32_t>(fopAcM_GetID(actor)),
        .parentProcessId = static_cast<std::uint32_t>(actor->parentActorID),
        .parameters = fopAcM_GetParam(actor),
        .status = actor->actor_status,
        .actorName = fopAcM_GetName(actor),
        .profileName = fopAcM_GetProfName(actor),
        .setId = actor->setID,
        .health = actor->health,
        .homeRoom = actor->home.roomNo,
        .currentRoom = actor->current.roomNo,
        .group = actor->group,
        .argument = actor->argument,
        .homePosition = actor->home.pos,
        .currentPosition = actor->current.pos,
    };
    const char* symbolicName = fopAcM_getProcNameString(actor);
    if (symbolicName != nullptr) {
        std::snprintf(entry.symbolicName.data(), entry.symbolicName.size(), "%s", symbolicName);
    }

    if (capture->count < capture->entries.size()) {
        capture->entries[capture->count++] = entry;
        return 1;
    }

    // Keep a deterministic bounded set even if a malformed or unusual stage
    // creates more actors than the artifact format permits.
    auto largest = std::max_element(capture->entries.begin(), capture->entries.end(),
        [](const auto& left, const auto& right) { return left.processId < right.processId; });
    if (entry.processId < largest->processId) {
        *largest = entry;
    }
    return 1;
}

json position_json(const cXyz& position) {
    return json::array({position.x, position.y, position.z});
}

}  // namespace

bool write_actor_catalog(
    const std::filesystem::path& path, const std::uint64_t simulationTick, std::string& error) {
    ActorCatalogCapture capture;
    fopAcIt_Executor(capture_actor, &capture);
    std::sort(capture.entries.begin(), capture.entries.begin() + capture.count,
        [](const auto& left, const auto& right) { return left.processId < right.processId; });

    json actors = json::array();
    for (std::size_t index = 0; index < capture.count; ++index) {
        const ActorCatalogEntry& actor = capture.entries[index];
        actors.push_back({
            {"process_id", actor.processId},
            {"parent_process_id", actor.parentProcessId},
            {"actor_name", actor.actorName},
            {"profile_name", actor.profileName},
            {"symbolic_name", actor.symbolicName.data()},
            {"group", actor.group},
            {"is_enemy", actor.group == fopAc_ENEMY_e},
            {"set_id", actor.setId},
            {"parameters", actor.parameters},
            {"argument", actor.argument},
            {"home_room", actor.homeRoom},
            {"current_room", actor.currentRoom},
            {"home_position", position_json(actor.homePosition)},
            {"current_position", position_json(actor.currentPosition)},
            {"health", actor.health},
            {"status", actor.status},
        });
    }

    const auto& nameEntryObserver = name_entry_observer();
    const BuildIdentity build = current_build_identity(
        nameEntryObserver.cursorBreakoutShadowEnabled()
            ? "cursor_breakout_shadow"
            : "observe_only");
    json document{
        {"schema", "dusklight.actor-catalog.v1"},
        {"build",
            {
                {"version", build.version},
                {"describe", build.describe},
                {"revision", build.revision},
                {"dirty_digest", build.dirtyDigest},
                {"branch", build.branch},
                {"source_date", build.sourceDate},
                {"aurora_revision", build.auroraRevision},
                {"compiler", build.compiler},
                {"compiler_target", build.compilerTarget},
                {"build_type", build.buildType},
                {"feature_switches", build.featureSwitches},
                {"feature_digest", build.featureDigest},
                {"fidelity_profile", build.fidelityProfile},
                {"platform", build.platform},
                {"architecture", build.architecture},
                {"pointer_bits", build.pointerBits},
                {"dirty", build.dirty},
            }},
        {"simulation_tick", simulationTick},
        {"stage", dComIfGp_getStartStageName()},
        {"room", dComIfGp_roomControl_getStayNo()},
        {"layer", dComIfG_play_c::getLayerNo(0)},
        {"observed_actor_count", capture.observed},
        {"retained_actor_count", capture.count},
        {"truncated", capture.observed > capture.count},
        {"actors", std::move(actors)},
    };

    std::error_code filesystemError;
    if (const auto parent = path.parent_path(); !parent.empty()) {
        std::filesystem::create_directories(parent, filesystemError);
        if (filesystemError) {
            error = filesystemError.message();
            return false;
        }
    }
    std::ofstream stream(path, std::ios::binary | std::ios::trunc);
    if (!stream) {
        error = "could not open actor catalog for writing";
        return false;
    }
    stream << document.dump(2) << '\n';
    if (!stream) {
        error = "failed while writing actor catalog";
        return false;
    }
    return true;
}

}  // namespace dusk::automation

#else

namespace dusk::automation {

bool write_actor_catalog(const std::filesystem::path&, const std::uint64_t, std::string& error) {
    error = "fork-only automation observers are disabled in this build";
    return false;
}

}  // namespace dusk::automation

#endif
