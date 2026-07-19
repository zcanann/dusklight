#pragma once

#include "dusk/automation/milestones.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>

namespace dusk::automation {

struct ControllerObservation;

inline constexpr std::uint16_t kTypedFactResponseMajorVersion = 1;
inline constexpr std::uint16_t kTypedFactResponseMinorVersion = 1;
inline constexpr std::size_t kTypedFactMaximumEntries = 16;
inline constexpr std::uint64_t kTypedFactNoTapeFrame = ~std::uint64_t{0};

enum class TypedFactPhase : std::uint8_t {
    PreInput = 1,
    PostSimulation = 2,
};

enum class TypedFactStatus : std::uint8_t {
    Present = 1,
    Absent = 2,
    Unavailable = 3,
    Truncated = 4,
    Invalid = 5,
};

enum class TypedFactValueType : std::uint8_t {
    Boolean = 1,
    I32 = 2,
    U32 = 3,
    Vec3F32 = 4,
    StageCode = 5,
    ActorIdentity = 6,
};

enum class TypedFactId : std::uint16_t {
    StageName = 1,
    StageRoom = 2,
    StageSpawn = 3,
    PlayerExists = 4,
    PlayerIsLink = 5,
    PlayerPosition = 6,
    EventRunning = 7,
    EventId = 8,
    PlayerDoStatus = 9,
    TalkPartner = 10,
    GrabbedActor = 11,
};

struct TypedFactActorIdentity {
    std::uint32_t runtimeGeneration = 0xffffffff;
    std::int16_t actorName = -1;
    std::uint16_t setId = 0xffff;
    std::int8_t homeRoom = -1;
    std::int8_t currentRoom = -1;
    bool homePositionPresent = false;
    std::array<float, 3> homePosition{};
};

struct TypedFactValue {
    bool boolean = false;
    std::int32_t i32 = 0;
    std::uint32_t u32 = 0;
    std::array<float, 3> vec3{};
    std::array<char, 8> stageCode{};
    TypedFactActorIdentity actor{};
};

struct TypedFactEntry {
    TypedFactId id = TypedFactId::StageName;
    TypedFactStatus status = TypedFactStatus::Unavailable;
    TypedFactValueType type = TypedFactValueType::Boolean;
    TypedFactValue value{};
};

struct TypedFactSourceStatus {
    TypedFactStatus stage = TypedFactStatus::Present;
    TypedFactStatus player = TypedFactStatus::Present;
    TypedFactStatus event = TypedFactStatus::Present;
    TypedFactStatus interaction = TypedFactStatus::Present;
};

struct TypedFactResponse {
    std::uint16_t majorVersion = kTypedFactResponseMajorVersion;
    std::uint16_t minorVersion = kTypedFactResponseMinorVersion;
    TypedFactPhase phase = TypedFactPhase::PostSimulation;
    std::uint64_t simulationTick = 0;
    std::uint64_t tapeFrame = kTypedFactNoTapeFrame;
    std::uint16_t count = 0;
    std::array<TypedFactEntry, kTypedFactMaximumEntries> entries{};

    [[nodiscard]] const TypedFactEntry* find(TypedFactId id) const;
};

[[nodiscard]] TypedFactResponse build_typed_fact_response(
    const MilestoneObservation& observation,
    TypedFactPhase phase,
    std::uint64_t simulationTick,
    std::optional<std::uint64_t> tapeFrame,
    TypedFactSourceStatus sources = {});

[[nodiscard]] TypedFactResponse build_typed_fact_response(
    const ControllerObservation& observation,
    TypedFactPhase phase,
    std::uint64_t simulationTick,
    std::optional<std::uint64_t> tapeFrame);

[[nodiscard]] bool validate_typed_fact_response(const TypedFactResponse& response);

}  // namespace dusk::automation
