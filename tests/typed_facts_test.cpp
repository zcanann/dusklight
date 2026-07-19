#include "dusk/automation/typed_facts.hpp"

#include <cstdlib>
#include <iostream>

namespace {

#define REQUIRE(condition)                                                                    \
    do {                                                                                      \
        if (!(condition)) {                                                                   \
            std::cerr << "Requirement failed at line " << __LINE__ << ": " #condition "\n"; \
            std::abort();                                                                     \
        }                                                                                     \
    } while (false)

using namespace dusk::automation;

void test_exact_interaction_snapshot() {
    MilestoneObservation observation{};
    observation.stageName = "F_SP104";
    observation.room = 1;
    observation.point = 0;
    observation.playerPresent = true;
    observation.playerIsLink = true;
    observation.playerPositionX = 1.0F;
    observation.playerPositionY = 2.0F;
    observation.playerPositionZ = 3.0F;
    observation.playerDoStatus = 21;
    observation.eventRunning = true;
    observation.eventId = 7;
    observation.talkPartner = {
        .present = true,
        .runtimeGeneration = 99,
        .actorName = 42,
        .setId = 8,
        .homeRoom = 1,
        .currentRoom = 2,
    };

    const auto response = build_typed_fact_response(
        observation, TypedFactPhase::PreInput, 12, 11);
    REQUIRE(response.majorVersion == 1);
    REQUIRE(response.minorVersion == 0);
    REQUIRE(response.phase == TypedFactPhase::PreInput);
    REQUIRE(response.simulationTick == 12);
    REQUIRE(response.tapeFrame == 11);
    REQUIRE(response.count == 11);
    const auto* position = response.find(TypedFactId::PlayerPosition);
    REQUIRE(position != nullptr);
    REQUIRE(position->status == TypedFactStatus::Present);
    REQUIRE(position->type == TypedFactValueType::Vec3F32);
    REQUIRE(position->value.vec3[2] == 3.0F);
    const auto* talk = response.find(TypedFactId::TalkPartner);
    REQUIRE(talk != nullptr);
    REQUIRE(talk->status == TypedFactStatus::Present);
    REQUIRE(talk->value.actor.actorName == 42);
    REQUIRE(talk->value.actor.setId == 8);
    REQUIRE(talk->value.actor.homeRoom == 1);
    REQUIRE(talk->value.actor.currentRoom == 2);
    REQUIRE(talk->value.actor.runtimeGeneration == 99);
    REQUIRE(response.find(TypedFactId::GrabbedActor)->status == TypedFactStatus::Absent);
}

void test_missingness_is_not_false() {
    MilestoneObservation observation{};
    TypedFactSourceStatus sources{};
    sources.event = TypedFactStatus::Unavailable;
    sources.interaction = TypedFactStatus::Truncated;
    const auto response = build_typed_fact_response(
        observation, TypedFactPhase::PostSimulation, 4, std::nullopt, sources);
    REQUIRE(response.tapeFrame == kTypedFactNoTapeFrame);
    REQUIRE(response.find(TypedFactId::PlayerExists)->status == TypedFactStatus::Present);
    REQUIRE(!response.find(TypedFactId::PlayerExists)->value.boolean);
    REQUIRE(response.find(TypedFactId::PlayerPosition)->status == TypedFactStatus::Absent);
    REQUIRE(response.find(TypedFactId::EventRunning)->status == TypedFactStatus::Unavailable);
    REQUIRE(response.find(TypedFactId::TalkPartner)->status == TypedFactStatus::Truncated);
    REQUIRE(response.find(TypedFactId::GrabbedActor)->status == TypedFactStatus::Truncated);
    REQUIRE(response.find(TypedFactId::StageName)->status == TypedFactStatus::Absent);
    REQUIRE(response.find(TypedFactId::StageRoom)->status == TypedFactStatus::Absent);
    REQUIRE(response.find(TypedFactId::StageSpawn)->status == TypedFactStatus::Absent);
    REQUIRE(validate_typed_fact_response(response));
}

}  // namespace

int main() {
    test_exact_interaction_snapshot();
    test_missingness_is_not_false();
    return 0;
}
