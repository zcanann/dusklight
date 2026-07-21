#include "dusk/automation/actor_catalog.hpp"

#if DUSK_ENABLE_AUTOMATION_OBSERVERS

#include "dusk/automation/build_identity.hpp"
#include "dusk/automation/game_state_observer.hpp"
#include "dusk/automation/learning_episode.hpp"
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
    std::uint32_t condition = 0;
    std::int32_t actorType = 0;
    std::int32_t processSubtype = 0;
    std::int16_t actorName = 0;
    std::int16_t profileName = 0;
    std::uint16_t setId = 0xffff;
    std::int16_t health = 0;
    std::int8_t homeRoom = -1;
    std::int8_t oldRoom = -1;
    std::int8_t currentRoom = -1;
    std::uint8_t group = 0;
    std::int8_t argument = 0;
    std::uint8_t pauseFlag = 0;
    std::int8_t processInitState = 0;
    std::uint8_t processCreatePhase = 0;
    std::uint8_t cullType = 0;
    std::uint8_t demoActorId = 0;
    std::uint8_t carryType = 0;
    bool heapPresent = false;
    bool modelPresent = false;
    bool jointCollisionPresent = false;
    std::array<char, 32> symbolicName{};
    cXyz homePosition{};
    cXyz oldPosition{};
    cXyz currentPosition{};
    cXyz scale{};
    cXyz eyePosition{};
    csXyz homeAngle{};
    csXyz oldAngle{};
    float gravity = 0.0F;
    float maxFallSpeed = 0.0F;
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
        .condition = actor->actor_condition,
        .actorType = actor->actor_type,
        .processSubtype = actor->subtype,
        .actorName = fopAcM_GetName(actor),
        .profileName = fopAcM_GetProfName(actor),
        .setId = actor->setID,
        .health = actor->health,
        .homeRoom = actor->home.roomNo,
        .oldRoom = actor->old.roomNo,
        .currentRoom = actor->current.roomNo,
        .group = actor->group,
        .argument = actor->argument,
        .pauseFlag = actor->pause_flag,
        .processInitState = actor->state.init_state,
        .processCreatePhase = actor->state.create_phase,
        .cullType = actor->cullType,
        .demoActorId = actor->demoActorID,
        .carryType = actor->carryType,
        .heapPresent = actor->heap != nullptr,
        .modelPresent = actor->model != nullptr,
        .jointCollisionPresent = actor->jntCol != nullptr,
        .homePosition = actor->home.pos,
        .oldPosition = actor->old.pos,
        .currentPosition = actor->current.pos,
        .scale = actor->scale,
        .eyePosition = actor->eyePos,
        .homeAngle = actor->home.angle,
        .oldAngle = actor->old.angle,
        .gravity = actor->gravity,
        .maxFallSpeed = actor->maxFallSpeed,
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

json position_json(const float x, const float y, const float z) {
    return json::array({x, y, z});
}

json angle_json(const std::int16_t x, const std::int16_t y, const std::int16_t z) {
    return json::array({x, y, z});
}

json learning_actor_json(const MilestoneObservation::Actor& actor) {
    json attention = nullptr;
    if (actor.attentionPresent) {
        attention = {
            {"flags", actor.attention.flags},
            {"position", position_json(actor.attention.positionX, actor.attention.positionY,
                             actor.attention.positionZ)},
            {"distance_indices", actor.attention.distanceIndices},
            {"auxiliary", actor.attention.auxiliary},
        };
    }
    json eventParticipation = nullptr;
    if (actor.eventParticipationPresent) {
        eventParticipation = {
            {"command", actor.eventParticipation.command},
            {"condition", actor.eventParticipation.condition},
            {"event_id", actor.eventParticipation.eventId},
            {"map_tool_id", actor.eventParticipation.mapToolId},
            {"index", actor.eventParticipation.index},
        };
    }
    json returnPlaceWriter = nullptr;
    if (actor.returnPlaceWriterPresent) {
        returnPlaceWriter = {
            {"save_room", actor.returnPlaceWriter.saveRoom},
            {"save_point", actor.returnPlaceWriter.savePoint},
            {"switch_room", actor.returnPlaceWriter.switchRoom},
            {"required_event_set", actor.returnPlaceWriter.requiredEventSet},
            {"required_event_unset", actor.returnPlaceWriter.requiredEventUnset},
            {"required_switch_set", actor.returnPlaceWriter.requiredSwitchSet},
            {"required_switch_unset", actor.returnPlaceWriter.requiredSwitchUnset},
            {"no_telop_clear", actor.returnPlaceWriter.noTelopClear},
            {"event_set_satisfied", actor.returnPlaceWriter.eventSetSatisfied},
            {"event_unset_satisfied", actor.returnPlaceWriter.eventUnsetSatisfied},
            {"switch_set_satisfied", actor.returnPlaceWriter.switchSetSatisfied},
            {"switch_unset_satisfied", actor.returnPlaceWriter.switchUnsetSatisfied},
            {"eligible", actor.returnPlaceWriter.eligible},
        };
    }
    return {
        {"runtime_generation", actor.runtimeGeneration},
        {"actor_type", actor.actorType},
        {"process_subtype", actor.processSubtype},
        {"parent_runtime_generation", actor.parentRuntimeGeneration},
        {"actor_name", actor.actorName},
        {"profile_name", actor.profileName},
        {"group", actor.group},
        {"set_id", actor.setId},
        {"parameters", actor.parameters},
        {"condition", actor.condition},
        {"argument", actor.argument},
        {"home_room", actor.homeRoom},
        {"old_room", actor.oldRoom},
        {"current_room", actor.currentRoom},
        {"pause_flag", actor.pauseFlag},
        {"process_init_state", actor.processInitState},
        {"process_create_phase", actor.processCreatePhase},
        {"cull_type", actor.cullType},
        {"demo_actor_id", actor.demoActorId},
        {"carry_type", actor.carryType},
        {"heap_present", actor.heapPresent},
        {"model_present", actor.modelPresent},
        {"joint_collision_present", actor.jointCollisionPresent},
        {"home_position",
            position_json(actor.homePositionX, actor.homePositionY, actor.homePositionZ)},
        {"old_position", position_json(actor.oldPositionX, actor.oldPositionY, actor.oldPositionZ)},
        {"current_position", position_json(actor.positionX, actor.positionY, actor.positionZ)},
        {"velocity", position_json(actor.velocityX, actor.velocityY, actor.velocityZ)},
        {"forward_speed", actor.forwardSpeed},
        {"scale", position_json(actor.scaleX, actor.scaleY, actor.scaleZ)},
        {"gravity", actor.gravity},
        {"max_fall_speed", actor.maxFallSpeed},
        {"eye_position", position_json(actor.eyePositionX, actor.eyePositionY, actor.eyePositionZ)},
        {"home_angle", angle_json(actor.homeAngleX, actor.homeAngleY, actor.homeAngleZ)},
        {"old_angle", angle_json(actor.oldAngleX, actor.oldAngleY, actor.oldAngleZ)},
        {"current_angle",
            angle_json(actor.currentAngleX, actor.currentAngleY, actor.currentAngleZ)},
        {"shape_angle", angle_json(actor.shapeAngleX, actor.shapeAngleY, actor.shapeAngleZ)},
        {"health", actor.health},
        {"status", actor.status},
        {"attention", std::move(attention)},
        {"event_participation", std::move(eventParticipation)},
        {"return_place_writer", std::move(returnPlaceWriter)},
    };
}

json learning_collider_json(const MilestoneObservation::DynamicCollider& collider) {
    const char* shape =
        collider.shape == MilestoneObservation::DynamicColliderShape::Sphere   ? "sphere" :
        collider.shape == MilestoneObservation::DynamicColliderShape::Cylinder ? "cylinder" :
                                                                                 "unknown";
    const auto optionalGeneration = [](const bool present, const std::uint32_t generation) {
        return present ? json(generation) : json(nullptr);
    };
    return {
        {"registration_index", collider.registrationIndex},
        {"owner_runtime_generation",
            optionalGeneration(collider.ownerPresent, collider.ownerRuntimeGeneration)},
        {"attack_hit_owner_runtime_generation", optionalGeneration(collider.attackHitOwnerPresent,
                                                    collider.attackHitOwnerRuntimeGeneration)},
        {"target_hit_owner_runtime_generation", optionalGeneration(collider.targetHitOwnerPresent,
                                                    collider.targetHitOwnerRuntimeGeneration)},
        {"correction_hit_owner_runtime_generation",
            optionalGeneration(
                collider.correctionHitOwnerPresent, collider.correctionHitOwnerRuntimeGeneration)},
        {"status_present", collider.statusPresent},
        {"shape_present", collider.shapePresent},
        {"shape", shape},
        {"attack_set", collider.attackSet},
        {"target_set", collider.targetSet},
        {"correction_set", collider.correctionSet},
        {"attack_hit", collider.attackHit},
        {"target_hit", collider.targetHit},
        {"correction_hit", collider.correctionHit},
        {"attack_type", collider.attackType},
        {"target_type", collider.targetType},
        {"attack_source_parameters", collider.attackSourceParameters},
        {"attack_result_parameters", collider.attackResultParameters},
        {"target_source_parameters", collider.targetSourceParameters},
        {"target_result_parameters", collider.targetResultParameters},
        {"correction_source_parameters", collider.correctionSourceParameters},
        {"correction_result_parameters", collider.correctionResultParameters},
        {"attack_power", collider.attackPower},
        {"weight", collider.weight},
        {"damage", collider.damage},
        {"center", position_json(collider.centerX, collider.centerY, collider.centerZ)},
        {"radius", collider.radius},
        {"height", collider.height},
        {"aabb_min", position_json(collider.aabbMinX, collider.aabbMinY, collider.aabbMinZ)},
        {"aabb_max", position_json(collider.aabbMaxX, collider.aabbMaxY, collider.aabbMaxZ)},
        {"correction",
            position_json(collider.correctionX, collider.correctionY, collider.correctionZ)},
    };
}

json learning_player_resources_json(const MilestoneObservation::PlayerResources& resources) {
    return {
        {"maximum_life", resources.maximumLife},
        {"life", resources.life},
        {"rupees", resources.rupees},
        {"rupee_capacity", resources.rupeeCapacity},
        {"maximum_oil", resources.maximumOil},
        {"oil", resources.oil},
        {"maximum_magic", resources.maximumMagic},
        {"magic", resources.magic},
        {"wallet", resources.wallet},
        {"transform_status", resources.transformStatus},
        {"world_time", resources.worldTime},
        {"date", resources.date},
        {"arrows", resources.arrows},
        {"arrow_capacity", resources.arrowCapacity},
        {"pachinko", resources.pachinko},
        {"poe_souls", resources.poeSouls},
        {"small_keys", resources.smallKeys},
        {"dungeon_map", resources.dungeonMap},
        {"dungeon_compass", resources.dungeonCompass},
        {"dungeon_boss_key", resources.dungeonBossKey},
        {"dungeon_warp", resources.dungeonWarp},
        {"inventory", resources.inventory},
        {"selected_items", resources.selectedItems},
        {"mixed_items", resources.mixedItems},
        {"equipment", resources.equipment},
        {"bomb_counts", resources.bombCounts},
        {"bomb_capacities", resources.bombCapacities},
        {"bottle_quantities", resources.bottleQuantities},
        {"acquired_item_bits", resources.acquiredItemBits},
        {"collect_item_bits", resources.collectItemBits},
        {"collected_crystal_bits", resources.collectedCrystalBits},
        {"collected_mirror_bits", resources.collectedMirrorBits},
    };
}

json learning_actor_identity_json(const MilestoneObservation::ActorIdentity& identity) {
    if (!identity.present)
        return nullptr;
    return {
        {"runtime_generation", identity.runtimeGeneration},
        {"actor_name", identity.actorName},
        {"set_id", identity.setId},
        {"home_room", identity.homeRoom},
        {"current_room", identity.currentRoom},
        {"home_position", identity.homePositionPresent ?
                              position_json(identity.homePositionX, identity.homePositionY,
                                  identity.homePositionZ) :
                              json(nullptr)},
    };
}

json learning_player_relationships_json(
    const MilestoneObservation::PlayerRelationships& relationships) {
    return {
        {"targeted_actor", learning_actor_identity_json(relationships.targetedActor)},
        {"ride_actor", learning_actor_identity_json(relationships.rideActor)},
        {"held_item_actor", learning_actor_identity_json(relationships.heldItemActor)},
        {"grabbed_actor", learning_actor_identity_json(relationships.grabbedActor)},
        {"thrown_boomerang_actor",
            learning_actor_identity_json(relationships.thrownBoomerangActor)},
        {"copy_rod_actor", learning_actor_identity_json(relationships.copyRodActor)},
        {"hookshot_roof_wait_actor",
            learning_actor_identity_json(relationships.hookshotRoofWaitActor)},
        {"chain_grab_actor", learning_actor_identity_json(relationships.chainGrabActor)},
        {"attention_hint_actor",
            learning_actor_identity_json(relationships.attentionHintActor)},
        {"attention_catch_actor",
            learning_actor_identity_json(relationships.attentionCatchActor)},
        {"attention_look_actor",
            learning_actor_identity_json(relationships.attentionLookActor)},
    };
}

json learning_player_collision_solver_json(
    const MilestoneObservation::PlayerCollisionSolver& solver) {
    json wallCircles = json::array();
    for (const auto& wall : solver.wallCircles) {
        wallCircles.push_back({
            {"flags", wall.flags},
            {"angle_y", wall.angleY},
            {"wall_radius_squared", wall.wallRadiusSquared},
            {"wall_height", wall.wallHeight},
            {"wall_radius", wall.wallRadius},
            {"direct_wall_height", wall.directWallHeight},
            {"realized_center", wall.realizedCenter},
            {"realized_radius", wall.realizedRadius},
        });
    }
    return {
        {"flags", solver.flags},
        {"wall_table_size", solver.wallTableSize},
        {"water_mode", solver.waterMode},
        {"line_start", solver.lineStart},
        {"line_end", solver.lineEnd},
        {"wall_cylinder_center", solver.wallCylinderCenter},
        {"wall_cylinder_radius", solver.wallCylinderRadius},
        {"wall_cylinder_height", solver.wallCylinderHeight},
        {"ground_check_offset", solver.groundCheckOffset},
        {"roof_correction_height", solver.roofCorrectionHeight},
        {"water_check_offset", solver.waterCheckOffset},
        {"wall_circles", std::move(wallCircles)},
    };
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
            {"actor_type", actor.actorType},
            {"process_subtype", actor.processSubtype},
            {"actor_name", actor.actorName},
            {"profile_name", actor.profileName},
            {"symbolic_name", actor.symbolicName.data()},
            {"group", actor.group},
            {"is_enemy", actor.group == fopAc_ENEMY_e},
            {"set_id", actor.setId},
            {"parameters", actor.parameters},
            {"argument", actor.argument},
            {"home_room", actor.homeRoom},
            {"old_room", actor.oldRoom},
            {"current_room", actor.currentRoom},
            {"pause_flag", actor.pauseFlag},
            {"process_init_state", actor.processInitState},
            {"process_create_phase", actor.processCreatePhase},
            {"cull_type", actor.cullType},
            {"demo_actor_id", actor.demoActorId},
            {"carry_type", actor.carryType},
            {"heap_present", actor.heapPresent},
            {"model_present", actor.modelPresent},
            {"joint_collision_present", actor.jointCollisionPresent},
            {"home_position", position_json(actor.homePosition)},
            {"old_position", position_json(actor.oldPosition)},
            {"current_position", position_json(actor.currentPosition)},
            {"scale", position_json(actor.scale)},
            {"gravity", actor.gravity},
            {"max_fall_speed", actor.maxFallSpeed},
            {"eye_position", position_json(actor.eyePosition)},
            {"home_angle", angle_json(actor.homeAngle.x, actor.homeAngle.y, actor.homeAngle.z)},
            {"old_angle", angle_json(actor.oldAngle.x, actor.oldAngle.y, actor.oldAngle.z)},
            {"health", actor.health},
            {"status", actor.status},
            {"condition", actor.condition},
        });
    }

    // Capture the independent, complete actor vector used by native learning
    // episodes at this exact no-simulation boundary. Keeping both walks in one
    // artifact lets the host prove that the learner did not silently inherit
    // the controller's bounded selection rule. Both adapters are read-only.
    MilestoneObservationStorage learningStorage;
    const MilestoneObservation learningObservation = capture_milestone_observation(learningStorage);
    json learningActors = json::array();
    for (const MilestoneObservation::Actor& actor : learningObservation.actors)
        learningActors.push_back(learning_actor_json(actor));
    json learningColliders = json::array();
    for (const MilestoneObservation::DynamicCollider& collider :
        learningObservation.dynamicColliders)
        learningColliders.push_back(learning_collider_json(collider));

    const auto& nameEntryObserver = name_entry_observer();
    const BuildIdentity build = current_build_identity(
        nameEntryObserver.cursorBreakoutShadowEnabled() ? "cursor_breakout_shadow" :
                                                          "observe_only");
    json document{
        {"schema", "dusklight.actor-catalog.v7"},
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
        {"learning_actor_population",
            {
                {"source_schema", LearningObservationSchema},
                {"observed_actor_count", learningObservation.actorObservedCount},
                {"retained_actor_count", learningObservation.actors.size()},
                {"truncated", learningObservation.actorsTruncated},
                {"actors", std::move(learningActors)},
            }},
        {"learning_dynamic_collision_population",
            {
                {"source_schema", LearningObservationSchema},
                {"present", learningObservation.dynamicCollidersPresent},
                {"retained_collider_count", learningObservation.dynamicColliders.size()},
                {"truncated", learningObservation.dynamicCollidersTruncated},
                {"colliders", std::move(learningColliders)},
            }},
        {"learning_player_resources",
            {
                {"source_schema", LearningObservationSchema},
                {"present", learningObservation.playerResourcesPresent},
                {"value", learningObservation.playerResourcesPresent ?
                              learning_player_resources_json(learningObservation.playerResources) :
                              json(nullptr)},
            }},
        {"learning_player_relationships",
            {
                {"source_schema", LearningObservationSchema},
                {"present", learningObservation.playerRelationshipsPresent},
                {"value", learningObservation.playerRelationshipsPresent ?
                              learning_player_relationships_json(
                                  learningObservation.playerRelationships) :
                              json(nullptr)},
            }},
        {"learning_player_collision_solver",
            {
                {"source_schema", LearningObservationSchema},
                {"present", learningObservation.playerCollisionSolverPresent},
                {"value", learningObservation.playerCollisionSolverPresent ?
                              learning_player_collision_solver_json(
                                  learningObservation.playerCollisionSolver) :
                              json(nullptr)},
            }},
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
