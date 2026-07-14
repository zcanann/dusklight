#pragma once

#include <cstdint>
#include <filesystem>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace dusk::automation {

inline constexpr std::uint32_t MilestoneResultSchemaVersion = 1;
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
    bool eventRunning = false;

    bool nextStageEnabled = false;
    const char* nextStageName = nullptr;
    std::int8_t nextRoom = -1;
    std::int8_t nextLayer = -1;
    std::int16_t nextPoint = -1;
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
    bool eventRunning = false;

    bool nextStageEnabled = false;
    std::string nextStageName;
    std::int8_t nextRoom = -1;
    std::int8_t nextLayer = -1;
    std::int16_t nextPoint = -1;
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
