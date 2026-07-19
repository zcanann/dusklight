#pragma once

#include <cstdint>
#include <filesystem>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "dusk/automation/milestone_program.hpp"
#include "dusk/automation/input_tape.hpp"
#include "dusk/automation/rng.hpp"

namespace dusk::automation {

inline constexpr std::uint32_t MilestoneResultSchemaVersion = 5;
inline constexpr std::uint32_t MilestoneBoundaryFingerprintVersion = 4;
inline constexpr std::uint64_t MilestoneNoTapeFrame = ~std::uint64_t{0};

enum class MilestoneId : std::uint8_t {
    GameplayReadyFSp103,
    ExitFSp103ToFSp104,
    EnteredFSp104,
};

struct MilestoneObservation {
    struct ActorIdentity {
        bool present = false;
        std::uint32_t runtimeGeneration = 0xffffffff;
        std::int16_t actorName = -1;
        std::uint16_t setId = 0xffff;
        std::int8_t homeRoom = -1;
        std::int8_t currentRoom = -1;
        bool homePositionPresent = false;
        float homePositionX = 0.0f;
        float homePositionY = 0.0f;
        float homePositionZ = 0.0f;
    };

    const char* stageName = nullptr;
    std::int8_t room = -1;
    std::int8_t layer = -1;
    std::int16_t point = -1;
    bool playerPresent = false;
    bool playerIsLink = false;
    std::uint32_t playerProcessId = 0xffffffff;
    std::int16_t playerActorName = -1;
    std::uint16_t playerProcId = 0xffff;
    float playerPositionX = 0.0f;
    float playerPositionY = 0.0f;
    float playerPositionZ = 0.0f;
    float playerVelocityX = 0.0f;
    float playerVelocityY = 0.0f;
    float playerVelocityZ = 0.0f;
    float playerForwardSpeed = 0.0f;
    std::int16_t playerCurrentAngleX = 0;
    std::int16_t playerCurrentAngleY = 0;
    std::int16_t playerCurrentAngleZ = 0;
    std::int16_t playerShapeAngleX = 0;
    std::int16_t playerShapeAngleY = 0;
    std::int16_t playerShapeAngleZ = 0;
    std::uint32_t playerModeFlags = 0;
    std::int16_t playerDamageWaitTimer = 0;
    std::int16_t playerIceDamageWaitTimer = 0;
    std::uint8_t playerSwordChangeWaitTimer = 0;
    std::uint8_t playerDoStatus = 0;
    ActorIdentity talkPartner;
    ActorIdentity grabbedActor;
    bool playerGroundContact = false;
    bool playerWallContact = false;
    bool playerRoofContact = false;
    bool playerWaterContact = false;
    bool playerWaterIn = false;
    bool playerGroundHeightPresent = false;
    bool playerRoofHeightPresent = false;
    float playerGroundHeight = 0.0f;
    float playerRoofHeight = 0.0f;

    bool eventRunning = false;
    std::int16_t eventId = -1;
    std::uint8_t eventMode = 0;
    std::uint8_t eventStatus = 0;
    std::uint8_t eventMapToolId = 0xff;
    bool eventNameHashPresent = false;
    std::uint32_t eventNameHash = 0;

    bool nextStageEnabled = false;
    const char* nextStageName = nullptr;
    std::int8_t nextRoom = -1;
    std::int8_t nextLayer = -1;
    std::int16_t nextPoint = -1;

    GameRngSnapshot rng;

    struct Actor {
        std::uint64_t runtimeGeneration = 0;
        std::int16_t actorName = -1;
        std::uint16_t setId = 0xffff;
        std::int8_t homeRoom = -1;
        std::int8_t currentRoom = -1;
        float positionX = 0.0f;
        float positionY = 0.0f;
        float positionZ = 0.0f;
        std::int16_t health = 0;
        std::uint32_t status = 0;
    };
    std::span<const Actor> actors;
    bool actorsTruncated = false;

    // Indexed flag snapshots are immutable copies captured at the same phase
    // as the scalar observation. Switches cover exactly switchFlagRoom; an
    // off-room query evaluates as unavailable rather than reading live state.
    std::span<const std::uint8_t> eventFlags;
    std::span<const std::uint8_t> temporaryFlags;
    std::span<const std::uint8_t> dungeonFlags;
    std::span<const std::uint8_t> switchFlags;
    std::int8_t switchFlagRoom = -1;
    bool flagsPresent = false;
};

struct MilestoneDefinition {
    MilestoneId id;
    std::string_view name;
    std::string_view description;
    bool (*predicate)(const MilestoneObservation&);
};

struct MilestoneEvidence {
    TapeBoot boot;
    std::string stageName;
    std::int8_t room = -1;
    std::int8_t layer = -1;
    std::int16_t point = -1;
    bool playerPresent = false;
    bool playerIsLink = false;
    std::uint32_t playerProcessId = 0xffffffff;
    std::int16_t playerActorName = -1;
    std::uint16_t playerProcId = 0xffff;
    float playerPositionX = 0.0f;
    float playerPositionY = 0.0f;
    float playerPositionZ = 0.0f;
    float playerVelocityX = 0.0f;
    float playerVelocityY = 0.0f;
    float playerVelocityZ = 0.0f;
    float playerForwardSpeed = 0.0f;
    std::int16_t playerCurrentAngleX = 0;
    std::int16_t playerCurrentAngleY = 0;
    std::int16_t playerCurrentAngleZ = 0;
    std::int16_t playerShapeAngleX = 0;
    std::int16_t playerShapeAngleY = 0;
    std::int16_t playerShapeAngleZ = 0;

    bool eventRunning = false;
    std::int16_t eventId = -1;
    std::uint8_t eventMode = 0;
    std::uint8_t eventStatus = 0;
    std::uint8_t eventMapToolId = 0xff;
    bool eventNameHashPresent = false;
    std::uint32_t eventNameHash = 0;

    bool nextStageEnabled = false;
    std::string nextStageName;
    std::int8_t nextRoom = -1;
    std::int8_t nextLayer = -1;
    std::int16_t nextPoint = -1;

    GameRngSnapshot rng;
    std::string boundaryFingerprint;
};

struct MilestoneHit {
    MilestoneId id = MilestoneId::GameplayReadyFSp103;
    bool hit = false;
    std::uint64_t simulationTick = 0;
    std::uint64_t tapeFrame = MilestoneNoTapeFrame;
    MilestoneEvidence evidence;
};

struct AuthoredMilestoneHit {
    std::string id;
    MilestoneProgramPhase phase = MilestoneProgramPhase::PostSim;
    std::uint16_t stableTicks = 1;
    std::uint16_t consecutiveTicks = 0;
    std::uint8_t sequenceSteps = 0;
    std::uint8_t sequenceNextStep = 0;
    std::uint16_t sequenceWithinTicks = 0;
    std::uint16_t sequenceElapsedTicks = 0;
    std::string definitionDigest;
    std::string programDigest;
    bool hit = false;
    std::uint64_t boundaryIndex = 0;
    std::uint64_t simulationTick = 0;
    std::uint64_t tapeFrame = MilestoneNoTapeFrame;
    MilestoneEvidence evidence;

    struct ProjectedActor {
        std::int16_t actorName = -1;
        std::uint16_t setId = 0xffff;
        std::int8_t homeRoom = -1;
        std::int8_t currentRoom = -1;
        std::uint32_t positionXBits = 0;
        std::uint32_t positionYBits = 0;
        std::uint32_t positionZBits = 0;
        std::int16_t health = 0;
        std::uint32_t status = 0;
    };

    struct ProjectionItem {
        MilestoneValueProjectionKind kind = MilestoneValueProjectionKind::Rng;
        std::uint8_t selector = 0;
        std::string stage;
        std::int8_t room = -1;
        std::uint16_t index = 0;
        bool available = false;
        GameRngStreamSnapshot rng;
        std::vector<ProjectedActor> actors;
        bool flagValue = false;
    };

    struct Projection {
        std::string name;
        std::string identity;
        bool available = false;
        std::string valueDigest;
        std::vector<ProjectionItem> items;
    };

    std::vector<Projection> projections;
};

std::span<const MilestoneDefinition> milestone_definitions();
const MilestoneDefinition* find_milestone(MilestoneId id);
const MilestoneDefinition* find_milestone(std::string_view name);
std::string_view milestone_name(MilestoneId id);

/**
 * Computes XXH3-128 over a versioned, canonical little-endian encoding of all explicit evidence
 * fields. Tick counters, tape position, addresses, host clocks, renderer state, camera state,
 * collision internals, the non-player actor population, and save/event/switch flag arrays are not
 * included. The evidence JSON remains authoritative and inspectable; this digest is a fast equality
 * key, not a claim that every future-relevant game byte is covered.
 */
std::string compute_milestone_boundary_fingerprint(const MilestoneEvidence& evidence);

/** Parse a comma-separated list of stable milestone IDs. */
bool parse_milestone_list(
    std::string_view text, std::vector<MilestoneId>& output, std::string& error);
bool parse_milestone_name_list(
    std::string_view text, std::vector<std::string>& output, std::string& error);

class MilestoneTracker {
public:
    /**
     * Installs the requested predicates. A goal is optional, but when supplied it must also be in
     * requested. First-hit values are immutable until the next configure/reset.
     */
    bool configure(std::span<const MilestoneId> requested, std::optional<MilestoneId> goal,
        std::string& error);
    bool configureNames(std::span<const std::string> requested, std::optional<std::string> goal,
        const MilestoneProgram& program, std::string& error);
    void reset();
    void setBootOrigin(TapeBoot boot);
    void markBootOriginEstablished() { mBootOriginEstablished = true; }
    void observe(const MilestoneObservation& observation, std::uint64_t simulationTick,
        std::uint64_t tapeFrame);
    void observeBoundary(const MilestoneObservation& observation, MilestoneProgramPhase phase,
        MilestoneBoundaryKind boundaryKind, std::uint64_t boundaryIndex,
        std::uint64_t simulationTick, std::uint64_t tapeFrame);

    bool active() const { return !mHits.empty() || !mAuthoredHits.empty(); }
    bool goalReached() const;
    bool goalConfigured() const { return mGoalName.has_value(); }
    std::optional<std::string_view> goalName() const;
    std::optional<MilestoneId> goal() const { return mGoal; }
    const std::vector<MilestoneHit>& hits() const { return mHits; }
    const std::vector<AuthoredMilestoneHit>& authoredHits() const { return mAuthoredHits; }
    std::string_view programDigest() const { return mProgramDigest; }
    const TapeBoot& bootOrigin() const { return mBootOrigin; }
    bool bootOriginEstablished() const { return mBootOriginEstablished; }

private:
    std::vector<MilestoneHit> mHits;
    std::vector<AuthoredMilestoneHit> mAuthoredHits;
    std::optional<MilestoneId> mGoal;
    std::optional<std::string> mGoalName;
    const MilestoneProgram* mProgram = nullptr;
    std::string mProgramDigest;
    TapeBoot mBootOrigin;
    bool mBootOriginEstablished = true;
};

MilestoneTracker& milestone_tracker();

std::string serialize_milestone_result(const MilestoneTracker& tracker);
bool write_milestone_result(
    const std::filesystem::path& path, const MilestoneTracker& tracker, std::string& error);

}  // namespace dusk::automation
