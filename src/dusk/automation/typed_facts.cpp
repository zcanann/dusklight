#include "dusk/automation/typed_facts.hpp"

#include "dusk/automation/input_controller.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace dusk::automation {
namespace {

void append(TypedFactResponse& response, TypedFactEntry entry) {
    if (response.count < response.entries.size()) {
        response.entries[response.count++] = entry;
    }
}

TypedFactEntry fact(TypedFactId id, TypedFactValueType type, TypedFactStatus status) {
    return {.id = id, .status = status, .type = type};
}

TypedFactStatus dependent_status(TypedFactStatus source, bool present) {
    return source == TypedFactStatus::Present
               ? (present ? TypedFactStatus::Present : TypedFactStatus::Absent)
               : source;
}

float canonical_float(float value);

TypedFactActorIdentity actor_identity(const MilestoneObservation::ActorIdentity& actor) {
    return {
        .runtimeGeneration = actor.runtimeGeneration,
        .actorName = actor.actorName,
        .setId = actor.setId,
        .homeRoom = actor.homeRoom,
        .currentRoom = actor.currentRoom,
        .homePositionPresent = actor.homePositionPresent,
        .homePosition = {canonical_float(actor.homePositionX), canonical_float(actor.homePositionY),
            canonical_float(actor.homePositionZ)},
    };
}

float canonical_float(const float value) {
    return value == 0.0F ? 0.0F : value;
}

}  // namespace

const TypedFactEntry* TypedFactResponse::find(TypedFactId id) const {
    const auto end = entries.begin() + std::min<std::size_t>(count, entries.size());
    const auto found = std::find_if(entries.begin(), end,
        [id](const TypedFactEntry& entry) { return entry.id == id; });
    return found == end ? nullptr : &*found;
}

TypedFactResponse build_typed_fact_response(const MilestoneObservation& observation,
    TypedFactPhase phase, std::uint64_t simulationTick,
    std::optional<std::uint64_t> tapeFrame, TypedFactSourceStatus sources) {
    TypedFactResponse response{
        .phase = phase,
        .simulationTick = simulationTick,
        .tapeFrame = tapeFrame.value_or(kTypedFactNoTapeFrame),
    };

    auto stageName = fact(TypedFactId::StageName, TypedFactValueType::StageCode,
        dependent_status(sources.stage, observation.stageName != nullptr));
    if (stageName.status == TypedFactStatus::Present) {
        std::memcpy(stageName.value.stageCode.data(), observation.stageName,
            std::min<std::size_t>(stageName.value.stageCode.size(),
                std::strlen(observation.stageName)));
    }
    append(response, stageName);

    const TypedFactStatus stageStatus =
        dependent_status(sources.stage, observation.stageName != nullptr);
    auto stageRoom = fact(TypedFactId::StageRoom, TypedFactValueType::I32, stageStatus);
    stageRoom.value.i32 = observation.room;
    append(response, stageRoom);
    auto stageSpawn = fact(TypedFactId::StageSpawn, TypedFactValueType::I32, stageStatus);
    stageSpawn.value.i32 = observation.point;
    append(response, stageSpawn);

    auto playerExists = fact(
        TypedFactId::PlayerExists, TypedFactValueType::Boolean, sources.player);
    playerExists.value.boolean = observation.playerPresent;
    append(response, playerExists);
    const TypedFactStatus playerStatus = dependent_status(sources.player, observation.playerPresent);
    auto playerIsLink = fact(TypedFactId::PlayerIsLink, TypedFactValueType::Boolean, playerStatus);
    playerIsLink.value.boolean = observation.playerIsLink;
    append(response, playerIsLink);
    auto playerPosition = fact(TypedFactId::PlayerPosition, TypedFactValueType::Vec3F32, playerStatus);
    playerPosition.value.vec3 = {
        canonical_float(observation.playerPositionX),
        canonical_float(observation.playerPositionY),
        canonical_float(observation.playerPositionZ),
    };
    append(response, playerPosition);

    auto eventRunning = fact(TypedFactId::EventRunning, TypedFactValueType::Boolean, sources.event);
    eventRunning.value.boolean = observation.eventRunning;
    append(response, eventRunning);
    auto eventId = fact(TypedFactId::EventId, TypedFactValueType::I32, sources.event);
    eventId.value.i32 = observation.eventId;
    append(response, eventId);

    const bool linkPresent = observation.playerPresent && observation.playerIsLink;
    const TypedFactStatus interactionStatus = dependent_status(sources.interaction, linkPresent);
    auto doStatus = fact(TypedFactId::PlayerDoStatus, TypedFactValueType::U32, interactionStatus);
    doStatus.value.u32 = observation.playerDoStatus;
    append(response, doStatus);

    auto talk = fact(TypedFactId::TalkPartner, TypedFactValueType::ActorIdentity,
        dependent_status(interactionStatus, observation.talkPartner.present));
    if (talk.status == TypedFactStatus::Present) {
        talk.value.actor = actor_identity(observation.talkPartner);
    }
    append(response, talk);

    auto grabbed = fact(TypedFactId::GrabbedActor, TypedFactValueType::ActorIdentity,
        dependent_status(interactionStatus, observation.grabbedActor.present));
    if (grabbed.status == TypedFactStatus::Present) {
        grabbed.value.actor = actor_identity(observation.grabbedActor);
    }
    append(response, grabbed);
    return response;
}

TypedFactResponse build_typed_fact_response(const ControllerObservation& observation,
    const TypedFactPhase phase, const std::uint64_t simulationTick,
    const std::optional<std::uint64_t> tapeFrame) {
    std::array<char, 9> stageName{};
    std::copy(observation.stageName.begin(), observation.stageName.end(), stageName.begin());
    MilestoneObservation shared{
        .stageName = stageName[0] == '\0' ? nullptr : stageName.data(),
        .playerPresent = observation.playerPresent,
        .playerIsLink = observation.playerIsLink,
        .playerPositionX = observation.playerX,
        .playerPositionY = observation.playerY,
        .playerPositionZ = observation.playerZ,
    };
    auto response = build_typed_fact_response(shared, phase, simulationTick, tapeFrame,
        {
            .stage = TypedFactStatus::Present,
            .player = TypedFactStatus::Present,
            .event = TypedFactStatus::Unavailable,
            .interaction = TypedFactStatus::Unavailable,
        });
    for (std::size_t index = 0; index < response.count; ++index) {
        auto& entry = response.entries[index];
        if (entry.id == TypedFactId::StageRoom || entry.id == TypedFactId::StageSpawn) {
            entry.status = TypedFactStatus::Unavailable;
        }
    }
    return response;
}

bool validate_typed_fact_response(const TypedFactResponse& response) {
    constexpr std::array expectedIds{
        TypedFactId::StageName,
        TypedFactId::StageRoom,
        TypedFactId::StageSpawn,
        TypedFactId::PlayerExists,
        TypedFactId::PlayerIsLink,
        TypedFactId::PlayerPosition,
        TypedFactId::EventRunning,
        TypedFactId::EventId,
        TypedFactId::PlayerDoStatus,
        TypedFactId::TalkPartner,
        TypedFactId::GrabbedActor,
    };
    constexpr std::array expectedTypes{
        TypedFactValueType::StageCode,
        TypedFactValueType::I32,
        TypedFactValueType::I32,
        TypedFactValueType::Boolean,
        TypedFactValueType::Boolean,
        TypedFactValueType::Vec3F32,
        TypedFactValueType::Boolean,
        TypedFactValueType::I32,
        TypedFactValueType::U32,
        TypedFactValueType::ActorIdentity,
        TypedFactValueType::ActorIdentity,
    };
    if (response.majorVersion != kTypedFactResponseMajorVersion ||
        response.minorVersion != kTypedFactResponseMinorVersion ||
        (response.phase != TypedFactPhase::PreInput &&
            response.phase != TypedFactPhase::PostSimulation) ||
        response.count != expectedIds.size()) {
        return false;
    }
    for (std::size_t index = 0; index < expectedIds.size(); ++index) {
        const auto& entry = response.entries[index];
        if (entry.id != expectedIds[index] || entry.type != expectedTypes[index] ||
            entry.status < TypedFactStatus::Present || entry.status > TypedFactStatus::Invalid) {
            return false;
        }
        if (entry.status != TypedFactStatus::Present) continue;
        if (entry.type == TypedFactValueType::Vec3F32 &&
            !std::ranges::all_of(entry.value.vec3, [](const float value) {
                return std::isfinite(value) && !(value == 0.0F && std::signbit(value));
            })) {
            return false;
        }
        if (entry.type == TypedFactValueType::StageCode) {
            bool sawCharacter = false;
            bool sawTerminator = false;
            for (const char value : entry.value.stageCode) {
                if (value == '\0') {
                    sawTerminator = true;
                } else if (sawTerminator ||
                           !(value == '_' || (value >= '0' && value <= '9') ||
                               (value >= 'A' && value <= 'Z') ||
                               (value >= 'a' && value <= 'z'))) {
                    return false;
                } else {
                    sawCharacter = true;
                }
            }
            if (!sawCharacter) return false;
        }
        if (entry.type == TypedFactValueType::ActorIdentity) {
            const bool canonicalHomePosition = std::ranges::all_of(
                entry.value.actor.homePosition, [](const float value) {
                    return std::isfinite(value) && !(value == 0.0F && std::signbit(value));
                });
            if (entry.value.actor.homePositionPresent ? !canonicalHomePosition :
                                                        entry.value.actor.homePosition !=
                                                            std::array<float, 3>{}) {
                return false;
            }
        }
    }
    return true;
}

}  // namespace dusk::automation
