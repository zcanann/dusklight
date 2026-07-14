#pragma once

#include <cstdint>
#include <filesystem>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "dusk/automation/rng.hpp"

namespace dusk::automation {

inline constexpr std::uint32_t MilestoneResultSchemaVersion = 1;
inline constexpr std::uint32_t MilestoneBoundaryFingerprintVersion = 1;
inline constexpr std::uint64_t MilestoneNoTapeFrame = ~std::uint64_t{0};

enum class MilestoneId : std::uint8_t {
    GameplayReadyFSp103,
    ExitFSp103ToFSp104,
    EnteredFSp104,
};

struct MilestoneObservation {
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

    bool eventRunning = false;
    std::int16_t eventId = -1;
    std::uint8_t eventMode = 0;
    std::uint8_t eventStatus = 0;
    std::uint8_t eventMapToolId = 0xff;
    std::uint32_t eventNameHash = 0;

    bool nextStageEnabled = false;
    const char* nextStageName = nullptr;
    std::int8_t nextRoom = -1;
    std::int8_t nextLayer = -1;
    std::int16_t nextPoint = -1;

    GameRngSnapshot rng;
};

struct MilestoneDefinition {
    MilestoneId id;
    std::string_view name;
    std::string_view description;
    bool (*predicate)(const MilestoneObservation&);
};

struct MilestoneEvidence {
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

class MilestoneTracker {
public:
    /**
     * Installs the requested predicates. A goal is optional, but when supplied it must also be in
     * requested. First-hit values are immutable until the next configure/reset.
     */
    bool configure(std::span<const MilestoneId> requested, std::optional<MilestoneId> goal,
        std::string& error);
    void reset();
    void observe(const MilestoneObservation& observation, std::uint64_t simulationTick,
        std::uint64_t tapeFrame);

    bool active() const { return !mHits.empty(); }
    bool goalReached() const;
    std::optional<MilestoneId> goal() const { return mGoal; }
    const std::vector<MilestoneHit>& hits() const { return mHits; }

private:
    std::vector<MilestoneHit> mHits;
    std::optional<MilestoneId> mGoal;
};

MilestoneTracker& milestone_tracker();

std::string serialize_milestone_result(const MilestoneTracker& tracker);
bool write_milestone_result(
    const std::filesystem::path& path, const MilestoneTracker& tracker, std::string& error);

}  // namespace dusk::automation
