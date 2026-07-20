#include "dusk/automation/milestones.hpp"

#include "dusk/automation/card_fixture.hpp"

#include "dusk/automation/typed_facts.hpp"

#include <algorithm>
#include <array>
#include <bit>
#include <cstring>
#include <fstream>
#include <limits>
#include <system_error>
#include <tuple>
#include <type_traits>
#include <utility>
#include <vector>

#include <nlohmann/json.hpp>
#include <xxhash.h>

namespace dusk::automation {
namespace {

using nlohmann::json;

template <typename T>
void append_integer(std::vector<std::uint8_t>& output, const T value) {
    using U = std::make_unsigned_t<T>;
    const U bits = static_cast<U>(value);
    for (std::size_t byte = 0; byte < sizeof(T); ++byte) {
        output.push_back(static_cast<std::uint8_t>((bits >> (byte * 8)) & 0xffu));
    }
}

void append_float(std::vector<std::uint8_t>& output, const float value) {
    append_integer(output, std::bit_cast<std::uint32_t>(value));
}

void append_fixed_string(std::vector<std::uint8_t>& output, const std::string_view value) {
    constexpr std::size_t Width = 8;
    for (std::size_t index = 0; index < Width; ++index) {
        output.push_back(index < value.size() ? static_cast<std::uint8_t>(value[index]) : 0);
    }
}

std::string xxh3_128_hex(const std::span<const std::uint8_t> bytes) {
    const XXH128_hash_t hash = XXH3_128bits(bytes.data(), bytes.size());
    XXH128_canonical_t digest;
    XXH128_canonicalFromHash(&digest, hash);
    constexpr char Hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(sizeof(digest.digest) * 2);
    for (const unsigned char byte : digest.digest) {
        output.push_back(Hex[byte >> 4]);
        output.push_back(Hex[byte & 0x0f]);
    }
    return output;
}

std::string fixed_stage_string(const std::array<char, 8>& stage) {
    const auto end = std::ranges::find(stage, '\0');
    return std::string(stage.begin(), end);
}

bool stage_is(const char* actual, const char* expected) {
    return actual != nullptr && std::strcmp(actual, expected) == 0;
}

bool gameplay_ready_f_sp103(const MilestoneObservation& observation) {
    return stage_is(observation.stageName, "F_SP103") && observation.room == 1 &&
           observation.point == 1 && observation.playerPresent && observation.playerIsLink &&
           !observation.eventRunning && observation.eventId == -1;
}

bool exit_f_sp103_to_f_sp104(const MilestoneObservation& observation) {
    return stage_is(observation.stageName, "F_SP103") && observation.room == 1 &&
           observation.point == 1 && observation.nextStageEnabled &&
           stage_is(observation.nextStageName, "F_SP104") && observation.nextRoom == 1 &&
           observation.nextPoint == 0;
}

bool entered_f_sp104(const MilestoneObservation& observation) {
    return stage_is(observation.stageName, "F_SP104") && observation.room == 1 &&
           observation.point == 0;
}

constexpr std::array<MilestoneDefinition, 3> Definitions{{
    {MilestoneId::GameplayReadyFSp103, "gameplay-ready-f-sp103",
        "Link is controllable after the opening in F_SP103 room 1 point 1", gameplay_ready_f_sp103},
    {MilestoneId::ExitFSp103ToFSp104, "exit-f-sp103-to-f-sp104",
        "F_SP103 has committed the scene transition to F_SP104 room 1 point 0",
        exit_f_sp103_to_f_sp104},
    {MilestoneId::EnteredFSp104, "entered-f-sp104", "The live stage is F_SP104 room 1 point 0",
        entered_f_sp104},
}};

MilestoneEvidence capture_evidence(
    const MilestoneObservation& observation, const TapeBoot& boot) {
    return {
        .boot = boot,
        .cardFixtureIdentity = std::string(active_automation_card_fixture_identity()),
        .stageName = observation.stageName == nullptr ? "" : observation.stageName,
        .room = observation.room,
        .layer = observation.layer,
        .point = observation.point,
        .playerPresent = observation.playerPresent,
        .playerIsLink = observation.playerIsLink,
        .playerProcessId = observation.playerProcessId,
        .playerActorName = observation.playerActorName,
        .playerProcId = observation.playerProcId,
        .playerPositionX = observation.playerPositionX,
        .playerPositionY = observation.playerPositionY,
        .playerPositionZ = observation.playerPositionZ,
        .playerVelocityX = observation.playerVelocityX,
        .playerVelocityY = observation.playerVelocityY,
        .playerVelocityZ = observation.playerVelocityZ,
        .playerForwardSpeed = observation.playerForwardSpeed,
        .playerCurrentAngleX = observation.playerCurrentAngleX,
        .playerCurrentAngleY = observation.playerCurrentAngleY,
        .playerCurrentAngleZ = observation.playerCurrentAngleZ,
        .playerShapeAngleX = observation.playerShapeAngleX,
        .playerShapeAngleY = observation.playerShapeAngleY,
        .playerShapeAngleZ = observation.playerShapeAngleZ,
        .eventRunning = observation.eventRunning,
        .eventId = observation.eventId,
        .eventMode = observation.eventMode,
        .eventStatus = observation.eventStatus,
        .eventMapToolId = observation.eventMapToolId,
        .eventNameHashPresent = observation.eventNameHashPresent,
        .eventNameHash = observation.eventNameHash,
        .titlePresent = observation.titlePresent,
        .titleProcedure = observation.titleProcedure,
        .titleLogoSkipReady = observation.titleLogoSkipReady,
        .titleStartReady = observation.titleStartReady,
        .nameEntryActive = observation.nameEntryActive,
        .nameEntryCharacterSelectReady = observation.nameEntryCharacterSelectReady,
        .nameEntryInputReady = observation.nameEntryInputReady,
        .nameEntrySelectionProcedure = observation.nameEntrySelectionProcedure,
        .fileSelectNoSaveReady = observation.fileSelectNoSaveReady,
        .fileSelectDataSelectReady = observation.fileSelectDataSelectReady,
        .fileSelectKeyWaitReady = observation.fileSelectKeyWaitReady,
        .fileSelectYesNoReady = observation.fileSelectYesNoReady,
        .nameScenePresent = observation.nameScenePresent,
        .nameSceneProcedure = observation.nameSceneProcedure,
        .fileSelectPresent = observation.fileSelectPresent,
        .fileSelectProcedure = observation.fileSelectProcedure,
        .fileSelectCardCheckProcedure = observation.fileSelectCardCheckProcedure,
        .nextStageEnabled = observation.nextStageEnabled,
        .nextStageName = observation.nextStageName == nullptr ? "" : observation.nextStageName,
        .nextRoom = observation.nextRoom,
        .nextLayer = observation.nextLayer,
        .nextPoint = observation.nextPoint,
        .rng = observation.rng,
    };
}

json rng_stream_json(const GameRngStreamSnapshot& stream) {
    return {
        {"id", stream.id == GameRngStreamId::Primary ? "primary" : "secondary"},
        {"algorithm_version", stream.algorithmVersion},
        {"state", {stream.state0, stream.state1, stream.state2}},
        {"call_count", stream.callCount},
    };
}

json scenario_fixture_json(const ScenarioFixture& fixture) {
    json document{
        {"schema", kScenarioFixtureSchema},
        {"name", fixture.name},
    };
    if (fixture.form) {
        document["form"] = *fixture.form == PlayerFixtureForm::Human ? "human" : "wolf";
    }
    if (fixture.health) {
        document["health"] = {
            {"current", fixture.health->current}, {"maximum", fixture.health->maximum}};
    }
    if (!fixture.rng.empty()) {
        document["rng"] = json::array();
        for (const RngFixture& rng : fixture.rng) {
            document["rng"].push_back({
                {"stream", rng.stream == FixtureRngStream::Primary ? "primary" : "secondary"},
                {"state0", rng.state0},
                {"state1", rng.state1},
                {"state2", rng.state2},
                {"call_count", rng.callCount},
            });
        }
    }
    if (fixture.videoMode) {
        constexpr std::array<const char*, 5> Names{
            "automatic", "ntsc_interlaced", "ntsc_progressive", "pal50", "pal60"};
        document["video_mode"] = Names[static_cast<std::size_t>(*fixture.videoMode)];
    }
    if (!fixture.inventory.empty()) {
        document["inventory"] = json::array();
        for (const InventoryFixture& item : fixture.inventory) {
            document["inventory"].push_back(
                {{"slot", item.slot}, {"item", item.item}, {"quantity", item.quantity}});
        }
    }
    if (!fixture.equipment.empty()) {
        document["equipment"] = json::array();
        for (const EquipmentFixture& item : fixture.equipment) {
            document["equipment"].push_back({{"slot", item.slot}, {"item", item.item}});
        }
    }
    if (!fixture.flags.empty()) {
        constexpr std::array<const char*, 4> Domains{"event", "temporary", "dungeon", "switch"};
        document["flags"] = json::array();
        for (const FlagFixture& flag : fixture.flags) {
            document["flags"].push_back({
                {"domain", Domains[static_cast<std::size_t>(flag.domain)]},
                {"room", flag.room},
                {"index", flag.index},
                {"value", flag.value},
            });
        }
    }
    if (!fixture.settings.empty()) {
        document["settings"] = json::array();
        for (const SettingFixture& setting : fixture.settings) {
            json value;
            if (const auto* boolean = std::get_if<bool>(&setting.value)) {
                value = {{"type", "boolean"}, {"value", *boolean}};
            } else if (const auto* integer = std::get_if<std::int64_t>(&setting.value)) {
                value = {{"type", "integer"}, {"value", *integer}};
            } else if (const auto* floating = std::get_if<FixtureFloat>(&setting.value)) {
                value = {{"type", "float_bits"}, {"bits", floating->bits}};
            } else {
                value = {{"type", "string"}, {"value", std::get<std::string>(setting.value)}};
            }
            document["settings"].push_back({{"key", setting.key}, {"value", std::move(value)}});
        }
    }
    return document;
}

json boot_json(const TapeBoot& boot) {
    if (boot.kind == TapeBootKind::Process) {
        return {{"kind", "process"}};
    }
    json document{
        {"kind", "stage"},
        {"stage", boot.stage},
        {"room", boot.room},
        {"point", boot.point},
        {"layer", boot.layer},
        {"save_slot", boot.saveSlot == 0 ? json(nullptr) : json(boot.saveSlot)},
    };
    if (boot.fixture) {
        document["fixture"] = scenario_fixture_json(*boot.fixture);
    }
    return document;
}

std::vector<AuthoredMilestoneHit::Projection> capture_value_projections(
    const MilestoneProgramDefinition& definition, const MilestoneObservation& observation) {
    std::vector<AuthoredMilestoneHit::Projection> output;
    output.reserve(definition.valueProjections().size());
    for (const MilestoneValueProjection& spec : definition.valueProjections()) {
        AuthoredMilestoneHit::Projection projection{
            .name = spec.name,
            .identity = spec.identity,
            .available = true,
        };
        projection.items.reserve(spec.items.size());
        for (const MilestoneValueProjectionItem& specItem : spec.items) {
            AuthoredMilestoneHit::ProjectionItem item{
                .kind = specItem.kind,
                .selector = specItem.selector,
                .stage = fixed_stage_string(specItem.stage),
                .room = specItem.room,
                .index = specItem.index,
                .available = true,
            };
            if (specItem.kind == MilestoneValueProjectionKind::Rng) {
                item.available = observation.rng.version == kGameRngSnapshotVersion &&
                                 observation.rng.streamCount == kGameRngStreamCount;
                if (item.available) {
                    const GameRngStreamId id = static_cast<GameRngStreamId>(specItem.selector);
                    const auto found = std::ranges::find(observation.rng.streams, id,
                        &GameRngStreamSnapshot::id);
                    item.available = found != observation.rng.streams.end();
                    if (item.available) item.rng = *found;
                }
            } else if (specItem.kind == MilestoneValueProjectionKind::ActorPopulation) {
                item.available = !observation.actorsTruncated && observation.stageName != nullptr &&
                                 item.stage == observation.stageName;
                if (item.available) {
                    for (const MilestoneObservation::Actor& actor : observation.actors) {
                        if (actor.homeRoom != item.room) continue;
                        item.actors.push_back({
                            .actorName = actor.actorName,
                            .setId = actor.setId,
                            .homeRoom = actor.homeRoom,
                            .currentRoom = actor.currentRoom,
                            .positionXBits = std::bit_cast<std::uint32_t>(actor.positionX),
                            .positionYBits = std::bit_cast<std::uint32_t>(actor.positionY),
                            .positionZBits = std::bit_cast<std::uint32_t>(actor.positionZ),
                            .health = actor.health,
                            .status = actor.status,
                        });
                    }
                    std::ranges::sort(item.actors, {}, [](const auto& actor) {
                        return std::tuple{actor.actorName, actor.setId, actor.homeRoom,
                            actor.currentRoom, actor.positionXBits, actor.positionYBits,
                            actor.positionZBits, actor.health, actor.status};
                    });
                }
            } else {
                const std::span<const std::uint8_t>* flags = nullptr;
                if (observation.flagsPresent) {
                    switch (specItem.selector) {
                    case 0: flags = &observation.eventFlags; break;
                    case 1: flags = &observation.temporaryFlags; break;
                    case 2: flags = &observation.dungeonFlags; break;
                    case 3:
                        if (observation.switchFlagRoom == specItem.room)
                            flags = &observation.switchFlags;
                        break;
                    }
                }
                item.available = flags != nullptr && specItem.index < flags->size();
                if (item.available) item.flagValue = (*flags)[specItem.index] != 0;
            }
            projection.available = projection.available && item.available;
            projection.items.push_back(std::move(item));
        }
        if (projection.available) {
            std::vector<std::uint8_t> canonical;
            constexpr std::string_view Domain = "dusklight.value-projection.value/v1\0";
            canonical.insert(canonical.end(), Domain.begin(), Domain.end());
            append_integer<std::uint16_t>(canonical,
                static_cast<std::uint16_t>(projection.identity.size()));
            canonical.insert(canonical.end(), projection.identity.begin(), projection.identity.end());
            append_integer<std::uint8_t>(canonical,
                static_cast<std::uint8_t>(projection.items.size()));
            for (const auto& item : projection.items) {
                append_integer(canonical, static_cast<std::uint8_t>(item.kind));
                append_integer(canonical, item.selector);
                append_fixed_string(canonical, item.stage);
                append_integer(canonical, item.room);
                append_integer(canonical, item.index);
                if (item.kind == MilestoneValueProjectionKind::Rng) {
                    append_integer(canonical, item.rng.algorithmVersion);
                    append_integer(canonical, item.rng.state0);
                    append_integer(canonical, item.rng.state1);
                    append_integer(canonical, item.rng.state2);
                    append_integer(canonical, item.rng.callCount);
                } else if (item.kind == MilestoneValueProjectionKind::ActorPopulation) {
                    append_integer<std::uint16_t>(canonical,
                        static_cast<std::uint16_t>(item.actors.size()));
                    for (const auto& actor : item.actors) {
                        append_integer(canonical, actor.actorName);
                        append_integer(canonical, actor.setId);
                        append_integer(canonical, actor.homeRoom);
                        append_integer(canonical, actor.currentRoom);
                        append_integer(canonical, actor.positionXBits);
                        append_integer(canonical, actor.positionYBits);
                        append_integer(canonical, actor.positionZBits);
                        append_integer(canonical, actor.health);
                        append_integer(canonical, actor.status);
                    }
                } else {
                    append_integer<std::uint8_t>(canonical, item.flagValue ? 1 : 0);
                }
            }
            projection.valueDigest = xxh3_128_hex(canonical);
        }
        output.push_back(std::move(projection));
    }
    return output;
}

json value_projections_json(
    const std::vector<AuthoredMilestoneHit::Projection>& projections) {
    constexpr std::array<const char*, 4> FlagDomains{"event", "temporary", "dungeon", "switch"};
    json output = json::array();
    for (const auto& projection : projections) {
        json values = json::array();
        for (const auto& item : projection.items) {
            json value{{"available", item.available}};
            if (item.kind == MilestoneValueProjectionKind::Rng) {
                value["kind"] = "rng";
                value["stream"] = item.selector == 0 ? "primary" : "secondary";
                value["value"] = item.available ? json(rng_stream_json(item.rng)) : json(nullptr);
            } else if (item.kind == MilestoneValueProjectionKind::ActorPopulation) {
                value["kind"] = "actor_population";
                value["stage"] = item.stage;
                value["room"] = item.room;
                value["value"] = item.available ? json::array() : json(nullptr);
                if (item.available) {
                    for (const auto& actor : item.actors) {
                        value["value"].push_back({
                            {"actor_name", actor.actorName}, {"set_id", actor.setId},
                            {"home_room", actor.homeRoom}, {"current_room", actor.currentRoom},
                            {"position_bits", {actor.positionXBits, actor.positionYBits,
                                                  actor.positionZBits}},
                            {"health", actor.health}, {"status", actor.status},
                        });
                    }
                }
            } else {
                value["kind"] = "flag";
                value["domain"] = FlagDomains[item.selector];
                value["room"] = item.room;
                value["index"] = item.index;
                value["value"] = item.available ? json(item.flagValue) : json(nullptr);
            }
            values.push_back(std::move(value));
        }
        output.push_back({
            {"name", projection.name},
            {"identity", projection.identity},
            {"available", projection.available},
            {"value_fingerprint", projection.available
                    ? json{{"schema", "dusklight.value-projection/v1"},
                          {"algorithm", "xxh3-128"},
                          {"canonical_encoding", "little-endian-exact-v1"},
                          {"digest", projection.valueDigest}}
                    : json(nullptr)},
            {"values", std::move(values)},
        });
    }
    return output;
}

json evidence_json(const MilestoneEvidence& evidence) {
    return {
        {"boot", boot_json(evidence.boot)},
        {"card_fixture_identity",
            evidence.cardFixtureIdentity.empty() ? json(nullptr) : json(evidence.cardFixtureIdentity)},
        {"stage",
            {
                {"name", evidence.stageName},
                {"room", evidence.room},
                {"layer", evidence.layer},
                {"point", evidence.point},
            }},
        {"player",
            {
                {"present", evidence.playerPresent},
                {"is_link", evidence.playerIsLink},
                {"process_id", evidence.playerProcessId},
                {"actor_name", evidence.playerActorName},
                {"procedure_id", evidence.playerProcId},
                {"position",
                    {evidence.playerPositionX, evidence.playerPositionY, evidence.playerPositionZ}},
                {"velocity",
                    {evidence.playerVelocityX, evidence.playerVelocityY, evidence.playerVelocityZ}},
                {"forward_speed", evidence.playerForwardSpeed},
                {"current_angle", {evidence.playerCurrentAngleX, evidence.playerCurrentAngleY,
                                      evidence.playerCurrentAngleZ}},
                {"shape_angle", {evidence.playerShapeAngleX, evidence.playerShapeAngleY,
                                    evidence.playerShapeAngleZ}},
            }},
        {"event",
            {
                {"running", evidence.eventRunning},
                {"id", evidence.eventId},
                {"mode", evidence.eventMode},
                {"status", evidence.eventStatus},
                {"map_tool_id", evidence.eventMapToolId},
                {"name_fnv1a_present", evidence.eventNameHashPresent},
                {"name_fnv1a",
                    evidence.eventNameHashPresent ? json(evidence.eventNameHash) : json(nullptr)},
            }},
        // Retained for additive compatibility with v1 consumers.
        {"event_running", evidence.eventRunning},
        {"menu",
            {
                {"title",
                    {
                        {"present", evidence.titlePresent},
                        {"procedure", evidence.titleProcedure},
                        {"logo_skip_ready", evidence.titleLogoSkipReady},
                        {"start_ready", evidence.titleStartReady},
                    }},
                {"name_entry",
                    {
                        {"active", evidence.nameEntryActive},
                        {"character_select_ready", evidence.nameEntryCharacterSelectReady},
                        {"input_ready", evidence.nameEntryInputReady},
                        {"selection_procedure", evidence.nameEntrySelectionProcedure},
                    }},
                {"name_scene",
                    {
                        {"present", evidence.nameScenePresent},
                        {"procedure", evidence.nameSceneProcedure},
                    }},
                {"file_select",
                    {
                        {"present", evidence.fileSelectPresent},
                        {"procedure", evidence.fileSelectProcedure},
                        {"card_check_procedure", evidence.fileSelectCardCheckProcedure},
                        {"no_save_ready", evidence.fileSelectNoSaveReady},
                        {"data_select_ready", evidence.fileSelectDataSelectReady},
                        {"key_wait_ready", evidence.fileSelectKeyWaitReady},
                        {"yes_no_ready", evidence.fileSelectYesNoReady},
                    }},
            }},
        {"next_stage",
            {
                {"enabled", evidence.nextStageEnabled},
                {"name", evidence.nextStageName},
                {"room", evidence.nextRoom},
                {"layer", evidence.nextLayer},
                {"point", evidence.nextPoint},
            }},
        {"rng",
            {
                {"snapshot_version", evidence.rng.version},
                {"stream_count", evidence.rng.streamCount},
                {"streams", {rng_stream_json(evidence.rng.streams[0]),
                                rng_stream_json(evidence.rng.streams[1])}},
            }},
        {"boundary_fingerprint",
            {
                {"schema", "dusklight.milestone-boundary/v6"},
                {"algorithm", "xxh3-128"},
                {"canonical_encoding", "little-endian-fixed-v6"},
                {"digest", evidence.boundaryFingerprint},
            }},
    };
}

}  // namespace

std::span<const MilestoneDefinition> milestone_definitions() {
    return Definitions;
}

const MilestoneDefinition* find_milestone(const MilestoneId id) {
    const auto found = std::ranges::find(Definitions, id, &MilestoneDefinition::id);
    return found == Definitions.end() ? nullptr : &*found;
}

const MilestoneDefinition* find_milestone(const std::string_view name) {
    const auto found = std::ranges::find(Definitions, name, &MilestoneDefinition::name);
    return found == Definitions.end() ? nullptr : &*found;
}

std::string_view milestone_name(const MilestoneId id) {
    const MilestoneDefinition* definition = find_milestone(id);
    return definition == nullptr ? "unknown" : definition->name;
}

std::string compute_milestone_boundary_fingerprint(const MilestoneEvidence& evidence) {
    std::vector<std::uint8_t> canonical;
    canonical.reserve(256);
    append_integer(canonical, MilestoneBoundaryFingerprintVersion);
    append_integer(canonical, static_cast<std::uint8_t>(evidence.boot.kind));
    if (evidence.cardFixtureIdentity.size() > std::numeric_limits<std::uint16_t>::max())
        return {};
    append_integer(
        canonical, static_cast<std::uint16_t>(evidence.cardFixtureIdentity.size()));
    canonical.insert(canonical.end(), evidence.cardFixtureIdentity.begin(),
        evidence.cardFixtureIdentity.end());
    append_fixed_string(canonical, evidence.boot.stage);
    append_integer(canonical, evidence.boot.room);
    append_integer(canonical, evidence.boot.layer);
    append_integer(canonical, evidence.boot.point);
    append_integer(canonical, evidence.boot.saveSlot);
    std::vector<std::uint8_t> fixtureBytes;
    if (evidence.boot.fixture) {
        const ScenarioFixtureError error =
            encode_scenario_fixture(*evidence.boot.fixture, fixtureBytes);
        if (error != ScenarioFixtureError::None) {
            return {};
        }
    }
    append_integer<std::uint32_t>(canonical, static_cast<std::uint32_t>(fixtureBytes.size()));
    canonical.insert(canonical.end(), fixtureBytes.begin(), fixtureBytes.end());
    append_fixed_string(canonical, evidence.stageName);
    append_integer(canonical, evidence.room);
    append_integer(canonical, evidence.layer);
    append_integer(canonical, evidence.point);
    append_integer<std::uint8_t>(canonical, evidence.playerPresent ? 1 : 0);
    append_integer<std::uint8_t>(canonical, evidence.playerIsLink ? 1 : 0);
    append_integer(canonical, evidence.playerProcessId);
    append_integer(canonical, evidence.playerActorName);
    append_integer(canonical, evidence.playerProcId);
    append_float(canonical, evidence.playerPositionX);
    append_float(canonical, evidence.playerPositionY);
    append_float(canonical, evidence.playerPositionZ);
    append_float(canonical, evidence.playerVelocityX);
    append_float(canonical, evidence.playerVelocityY);
    append_float(canonical, evidence.playerVelocityZ);
    append_float(canonical, evidence.playerForwardSpeed);
    append_integer(canonical, evidence.playerCurrentAngleX);
    append_integer(canonical, evidence.playerCurrentAngleY);
    append_integer(canonical, evidence.playerCurrentAngleZ);
    append_integer(canonical, evidence.playerShapeAngleX);
    append_integer(canonical, evidence.playerShapeAngleY);
    append_integer(canonical, evidence.playerShapeAngleZ);
    append_integer<std::uint8_t>(canonical, evidence.eventRunning ? 1 : 0);
    append_integer(canonical, evidence.eventId);
    append_integer(canonical, evidence.eventMode);
    append_integer(canonical, evidence.eventStatus);
    append_integer(canonical, evidence.eventMapToolId);
    append_integer<std::uint8_t>(canonical, evidence.eventNameHashPresent ? 1 : 0);
    if (evidence.eventNameHashPresent) {
        append_integer(canonical, evidence.eventNameHash);
    }
    append_integer<std::uint8_t>(canonical, evidence.titlePresent ? 1 : 0);
    append_integer(canonical, evidence.titleProcedure);
    append_integer<std::uint8_t>(canonical, evidence.titleLogoSkipReady ? 1 : 0);
    append_integer<std::uint8_t>(canonical, evidence.titleStartReady ? 1 : 0);
    append_integer<std::uint8_t>(canonical, evidence.nameEntryActive ? 1 : 0);
    append_integer<std::uint8_t>(canonical,
        evidence.nameEntryCharacterSelectReady ? 1 : 0);
    append_integer<std::uint8_t>(canonical, evidence.nameEntryInputReady ? 1 : 0);
    append_integer(canonical, evidence.nameEntrySelectionProcedure);
    append_integer<std::uint8_t>(canonical, evidence.fileSelectNoSaveReady ? 1 : 0);
    append_integer<std::uint8_t>(canonical, evidence.fileSelectDataSelectReady ? 1 : 0);
    append_integer<std::uint8_t>(canonical, evidence.fileSelectKeyWaitReady ? 1 : 0);
    append_integer<std::uint8_t>(canonical, evidence.fileSelectYesNoReady ? 1 : 0);
    append_integer<std::uint8_t>(canonical, evidence.nameScenePresent ? 1 : 0);
    append_integer(canonical, evidence.nameSceneProcedure);
    append_integer<std::uint8_t>(canonical, evidence.fileSelectPresent ? 1 : 0);
    append_integer(canonical, evidence.fileSelectProcedure);
    append_integer(canonical, evidence.fileSelectCardCheckProcedure);
    append_integer<std::uint8_t>(canonical, evidence.nextStageEnabled ? 1 : 0);
    append_fixed_string(canonical, evidence.nextStageName);
    append_integer(canonical, evidence.nextRoom);
    append_integer(canonical, evidence.nextLayer);
    append_integer(canonical, evidence.nextPoint);
    append_integer(canonical, evidence.rng.version);
    append_integer(canonical, evidence.rng.streamCount);
    for (const GameRngStreamSnapshot& stream : evidence.rng.streams) {
        append_integer(canonical, static_cast<std::uint8_t>(stream.id));
        append_integer(canonical, stream.algorithmVersion);
        append_integer(canonical, stream.state0);
        append_integer(canonical, stream.state1);
        append_integer(canonical, stream.state2);
        append_integer(canonical, stream.callCount);
    }

    const XXH128_hash_t hash = XXH3_128bits(canonical.data(), canonical.size());
    XXH128_canonical_t digest;
    XXH128_canonicalFromHash(&digest, hash);
    constexpr char Hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(sizeof(digest.digest) * 2);
    for (const unsigned char byte : digest.digest) {
        output.push_back(Hex[byte >> 4]);
        output.push_back(Hex[byte & 0x0f]);
    }
    return output;
}

std::string compute_milestone_boundary_fingerprint(
    const MilestoneObservation& observation, const TapeBoot& boot) {
    return compute_milestone_boundary_fingerprint(capture_evidence(observation, boot));
}

std::string compute_milestone_observation_fingerprint(
    const MilestoneObservation& observation, const TapeBoot& boot) {
    const std::string boundary =
        compute_milestone_boundary_fingerprint(capture_evidence(observation, boot));
    if (boundary.empty()) return {};

    std::vector<std::uint8_t> canonical;
    canonical.reserve(2048);
    append_integer(canonical, MilestoneObservationFingerprintVersion);
    canonical.insert(canonical.end(), boundary.begin(), boundary.end());
    append_integer(canonical, observation.playerModeFlags);
    append_integer(canonical, observation.playerDamageWaitTimer);
    append_integer(canonical, observation.playerIceDamageWaitTimer);
    append_integer(canonical, observation.playerSwordChangeWaitTimer);
    append_integer(canonical, observation.playerDoStatus);

    const auto appendActorIdentity = [&canonical](const MilestoneObservation::ActorIdentity& actor) {
        append_integer<std::uint8_t>(canonical, actor.present ? 1 : 0);
        append_integer(canonical, actor.runtimeGeneration);
        append_integer(canonical, actor.actorName);
        append_integer(canonical, actor.setId);
        append_integer(canonical, actor.homeRoom);
        append_integer(canonical, actor.currentRoom);
        append_integer<std::uint8_t>(canonical, actor.homePositionPresent ? 1 : 0);
        append_float(canonical, actor.homePositionX);
        append_float(canonical, actor.homePositionY);
        append_float(canonical, actor.homePositionZ);
    };
    appendActorIdentity(observation.talkPartner);
    appendActorIdentity(observation.grabbedActor);
    append_integer<std::uint8_t>(canonical, observation.playerGroundContact ? 1 : 0);
    append_integer<std::uint8_t>(canonical, observation.playerWallContact ? 1 : 0);
    append_integer<std::uint8_t>(canonical, observation.playerRoofContact ? 1 : 0);
    append_integer<std::uint8_t>(canonical, observation.playerWaterContact ? 1 : 0);
    append_integer<std::uint8_t>(canonical, observation.playerWaterIn ? 1 : 0);
    append_integer<std::uint8_t>(canonical, observation.playerGroundHeightPresent ? 1 : 0);
    append_integer<std::uint8_t>(canonical, observation.playerRoofHeightPresent ? 1 : 0);
    append_float(canonical, observation.playerGroundHeight);
    append_float(canonical, observation.playerRoofHeight);

    std::vector<MilestoneObservation::Actor> actors(
        observation.actors.begin(), observation.actors.end());
    std::ranges::sort(actors, [](const auto& left, const auto& right) {
        return std::tie(left.runtimeGeneration, left.actorName, left.setId, left.homeRoom,
                   left.currentRoom) <
               std::tie(right.runtimeGeneration, right.actorName, right.setId, right.homeRoom,
                   right.currentRoom);
    });
    append_integer<std::uint64_t>(canonical, actors.size());
    for (const MilestoneObservation::Actor& actor : actors) {
        append_integer(canonical, actor.runtimeGeneration);
        append_integer(canonical, actor.parentRuntimeGeneration);
        append_integer(canonical, actor.parameters);
        append_integer(canonical, actor.status);
        append_integer(canonical, actor.actorName);
        append_integer(canonical, actor.profileName);
        append_integer(canonical, actor.setId);
        append_integer(canonical, actor.homeRoom);
        append_integer(canonical, actor.currentRoom);
        append_integer(canonical, actor.group);
        append_integer(canonical, actor.argument);
        append_integer(canonical, actor.health);
        append_float(canonical, actor.positionX);
        append_float(canonical, actor.positionY);
        append_float(canonical, actor.positionZ);
        append_float(canonical, actor.homePositionX);
        append_float(canonical, actor.homePositionY);
        append_float(canonical, actor.homePositionZ);
        append_float(canonical, actor.velocityX);
        append_float(canonical, actor.velocityY);
        append_float(canonical, actor.velocityZ);
        append_float(canonical, actor.forwardSpeed);
        append_integer(canonical, actor.currentAngleX);
        append_integer(canonical, actor.currentAngleY);
        append_integer(canonical, actor.currentAngleZ);
        append_integer(canonical, actor.shapeAngleX);
        append_integer(canonical, actor.shapeAngleY);
        append_integer(canonical, actor.shapeAngleZ);
    }
    append_integer(canonical, observation.actorObservedCount);
    append_integer<std::uint8_t>(canonical, observation.actorsTruncated ? 1 : 0);

    const auto appendBytes = [&canonical](const std::span<const std::uint8_t> bytes) {
        append_integer<std::uint64_t>(canonical, bytes.size());
        canonical.insert(canonical.end(), bytes.begin(), bytes.end());
    };
    appendBytes(observation.eventFlags);
    appendBytes(observation.temporaryFlags);
    appendBytes(observation.dungeonFlags);
    appendBytes(observation.switchFlags);
    append_integer(canonical, observation.switchFlagRoom);
    append_integer<std::uint8_t>(canonical, observation.flagsPresent ? 1 : 0);
    return xxh3_128_hex(canonical);
}

bool parse_milestone_list(
    const std::string_view text, std::vector<MilestoneId>& output, std::string& error) {
    output.clear();
    if (text.empty()) {
        error = "milestone list cannot be empty";
        return false;
    }

    std::size_t begin = 0;
    while (begin <= text.size()) {
        const std::size_t end = text.find(',', begin);
        const std::string_view name =
            text.substr(begin, end == std::string_view::npos ? text.size() - begin : end - begin);
        const MilestoneDefinition* definition = find_milestone(name);
        if (definition == nullptr) {
            error = "unknown milestone '" + std::string(name) + "'";
            output.clear();
            return false;
        }
        if (std::ranges::find(output, definition->id) == output.end()) {
            output.push_back(definition->id);
        }
        if (end == std::string_view::npos) {
            break;
        }
        begin = end + 1;
    }
    return true;
}

bool parse_milestone_name_list(
    const std::string_view text, std::vector<std::string>& output, std::string& error) {
    output.clear();
    if (text.empty()) {
        error = "milestone list cannot be empty";
        return false;
    }
    std::size_t begin = 0;
    while (begin <= text.size()) {
        const std::size_t end = text.find(',', begin);
        const std::string_view name =
            text.substr(begin, end == std::string_view::npos ? text.size() - begin : end - begin);
        if (name.empty()) {
            error = "milestone names cannot be empty";
            output.clear();
            return false;
        }
        if (std::ranges::find(output, name) == output.end())
            output.emplace_back(name);
        if (end == std::string_view::npos)
            break;
        begin = end + 1;
    }
    return true;
}

bool MilestoneTracker::configure(const std::span<const MilestoneId> requested,
    const std::optional<MilestoneId> goal, std::string& error) {
    mHits.clear();
    mAuthoredHits.clear();
    mGoal.reset();
    mGoalName.reset();
    mProgram = nullptr;
    mProgramDigest.clear();
    if (requested.empty()) {
        error = "at least one milestone must be requested";
        return false;
    }
    for (const MilestoneId id : requested) {
        if (find_milestone(id) == nullptr) {
            error = "requested milestone is not registered";
            mHits.clear();
            return false;
        }
        if (std::ranges::find(mHits, id, &MilestoneHit::id) == mHits.end()) {
            mHits.push_back({.id = id});
        }
    }
    if (goal.has_value() && std::ranges::find(mHits, *goal, &MilestoneHit::id) == mHits.end()) {
        error = "goal '" + std::string(milestone_name(*goal)) + "' was not requested";
        mHits.clear();
        return false;
    }
    mGoal = goal;
    if (goal.has_value())
        mGoalName = std::string(milestone_name(*goal));
    return true;
}

bool MilestoneTracker::configureNames(const std::span<const std::string> requested,
    const std::optional<std::string> goal, const MilestoneProgram& program, std::string& error) {
    mHits.clear();
    mAuthoredHits.clear();
    mGoal.reset();
    mGoalName.reset();
    mProgram = &program;
    mProgramDigest = std::string(program.digest());
    if (requested.empty()) {
        error = "at least one milestone must be requested";
        return false;
    }
    for (const std::string& name : requested) {
        if (const MilestoneDefinition* builtin = find_milestone(name); builtin != nullptr) {
            mHits.push_back({.id = builtin->id});
        } else if (const MilestoneProgramDefinition* authored = program.find(name);
            authored != nullptr)
        {
            mAuthoredHits.push_back({
                .id = authored->id,
                .phase = authored->phase,
                .stableTicks = authored->stableTicks,
                .sequenceSteps = static_cast<std::uint8_t>(authored->sequenceStepCount()),
                .sequenceWithinTicks = authored->sequenceWithinTicks(),
                .definitionDigest = authored->definitionDigest,
                .programDigest = std::string(program.digest()),
            });
        } else {
            error = "unknown milestone '" + name + "'";
            mHits.clear();
            mAuthoredHits.clear();
            return false;
        }
    }
    if (goal.has_value() && std::ranges::find(requested, *goal) == requested.end()) {
        error = "goal '" + *goal + "' was not requested";
        mHits.clear();
        mAuthoredHits.clear();
        return false;
    }
    if (goal.has_value()) {
        mGoalName = goal;
        if (const MilestoneDefinition* builtin = find_milestone(*goal); builtin != nullptr)
            mGoal = builtin->id;
    }
    return true;
}

void MilestoneTracker::reset() {
    for (MilestoneHit& hit : mHits) {
        hit.hit = false;
        hit.simulationTick = 0;
        hit.tapeFrame = MilestoneNoTapeFrame;
        hit.evidence = {};
    }
    for (AuthoredMilestoneHit& hit : mAuthoredHits) {
        hit.consecutiveTicks = 0;
        hit.sequenceNextStep = 0;
        hit.sequenceElapsedTicks = 0;
        hit.hit = false;
        hit.boundaryIndex = 0;
        hit.simulationTick = 0;
        hit.tapeFrame = MilestoneNoTapeFrame;
        hit.evidence = {};
        hit.projections.clear();
    }
}

void MilestoneTracker::setBootOrigin(TapeBoot boot) {
    mBootOrigin = std::move(boot);
    mBootOriginEstablished = mBootOrigin.kind == TapeBootKind::Process;
}

void MilestoneTracker::observe(const MilestoneObservation& observation,
    const std::uint64_t simulationTick, const std::uint64_t tapeFrame) {
    observeBoundary(observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick,
        simulationTick + 1, simulationTick, tapeFrame);
}

void MilestoneTracker::observeBoundary(const MilestoneObservation& observation,
    const MilestoneProgramPhase phase, const MilestoneBoundaryKind boundaryKind,
    const std::uint64_t boundaryIndex, const std::uint64_t simulationTick,
    const std::uint64_t tapeFrame) {
    if (phase == MilestoneProgramPhase::PostSim) {
        for (MilestoneHit& hit : mHits) {
            if (hit.hit) {
                continue;
            }
            const MilestoneDefinition* definition = find_milestone(hit.id);
            if (definition != nullptr && definition->predicate(observation)) {
                hit.hit = true;
                hit.simulationTick = simulationTick;
                hit.tapeFrame = tapeFrame;
                hit.evidence = capture_evidence(observation, mBootOrigin);
                hit.evidence.boundaryFingerprint =
                    compute_milestone_boundary_fingerprint(hit.evidence);
            }
        }
    }
    if (mProgram == nullptr)
        return;
    for (AuthoredMilestoneHit& hit : mAuthoredHits) {
        if (hit.hit || hit.phase != phase)
            continue;
        const MilestoneProgramDefinition* definition = mProgram->find(hit.id);
        if (definition == nullptr) continue;
        const auto facts = build_typed_fact_response(observation,
            phase == MilestoneProgramPhase::PreInput ? TypedFactPhase::PreInput :
                                                       TypedFactPhase::PostSimulation,
            simulationTick,
            tapeFrame == MilestoneNoTapeFrame ? std::nullopt :
                                                std::optional<std::uint64_t>(tapeFrame));
        const MilestoneProgramContext context{
            .observation = observation,
            .facts = &facts,
            .phase = phase,
            .boundaryKind = boundaryKind,
            .boundaryIndex = boundaryIndex,
            .tapeFrame = tapeFrame == MilestoneNoTapeFrame ?
                             std::nullopt :
                             std::optional<std::uint64_t>(tapeFrame),
        };
        if (hit.sequenceSteps != 0) {
            if (hit.sequenceNextStep == 0) {
                if (definition->evaluateSequenceStep(0, context)) {
                    hit.sequenceNextStep = 1;
                    hit.sequenceElapsedTicks = 0;
                }
                continue;
            }
            const std::uint32_t nextElapsed =
                static_cast<std::uint32_t>(hit.sequenceElapsedTicks) + 1;
            if (nextElapsed > hit.sequenceWithinTicks) {
                hit.sequenceNextStep = 0;
                hit.sequenceElapsedTicks = 0;
                if (definition->evaluateSequenceStep(0, context)) {
                    hit.sequenceNextStep = 1;
                }
                continue;
            }
            hit.sequenceElapsedTicks = static_cast<std::uint16_t>(nextElapsed);
            if (definition->evaluateSequenceStep(hit.sequenceNextStep, context)) {
                ++hit.sequenceNextStep;
            } else if (definition->evaluateSequenceStep(0, context)) {
                // Deterministic overlap: a fresh first step restarts the window
                // only when the currently expected step did not match.
                hit.sequenceNextStep = 1;
                hit.sequenceElapsedTicks = 0;
            }
            if (hit.sequenceNextStep != hit.sequenceSteps) continue;
            hit.hit = true;
            hit.boundaryIndex = boundaryIndex;
            hit.simulationTick = simulationTick;
            hit.tapeFrame = tapeFrame;
            hit.evidence = capture_evidence(observation, mBootOrigin);
            hit.evidence.boundaryFingerprint =
                compute_milestone_boundary_fingerprint(hit.evidence);
            hit.projections = capture_value_projections(*definition, observation);
            continue;
        }
        const bool matches = definition->evaluate(context);
        if (!matches) {
            hit.consecutiveTicks = 0;
            continue;
        }
        if (hit.consecutiveTicks < hit.stableTicks)
            ++hit.consecutiveTicks;
        if (hit.consecutiveTicks != hit.stableTicks)
            continue;
        hit.hit = true;
        hit.boundaryIndex = boundaryIndex;
        hit.simulationTick = simulationTick;
        hit.tapeFrame = tapeFrame;
        hit.evidence = capture_evidence(observation, mBootOrigin);
        hit.evidence.boundaryFingerprint = compute_milestone_boundary_fingerprint(hit.evidence);
        hit.projections = capture_value_projections(*definition, observation);
    }
}

bool MilestoneTracker::goalReached() const {
    if (!mGoalName.has_value()) {
        return false;
    }
    if (mGoal.has_value()) {
        const auto found = std::ranges::find(mHits, *mGoal, &MilestoneHit::id);
        return found != mHits.end() && found->hit;
    }
    const auto found = std::ranges::find(mAuthoredHits, *mGoalName, &AuthoredMilestoneHit::id);
    return found != mAuthoredHits.end() && found->hit;
}

std::optional<std::string_view> MilestoneTracker::goalName() const {
    if (!mGoalName.has_value())
        return std::nullopt;
    return *mGoalName;
}

MilestoneTracker& milestone_tracker() {
    static MilestoneTracker tracker;
    return tracker;
}

std::string serialize_milestone_result(const MilestoneTracker& tracker) {
    json milestones = json::array();
    for (const MilestoneHit& hit : tracker.hits()) {
        json item{
            {"id", milestone_name(hit.id)},
            {"hit", hit.hit},
        };
        if (hit.hit) {
            item["sim_tick"] = hit.simulationTick;
            item["tape_frame"] =
                hit.tapeFrame == MilestoneNoTapeFrame ? json(nullptr) : json(hit.tapeFrame);
            item["evidence"] = evidence_json(hit.evidence);
        } else {
            item["sim_tick"] = nullptr;
            item["tape_frame"] = nullptr;
            item["evidence"] = nullptr;
        }
        milestones.push_back(std::move(item));
    }
    for (const AuthoredMilestoneHit& hit : tracker.authoredHits()) {
        json item{
            {"id", hit.id},
            {"hit", hit.hit},
            {"phase", hit.phase == MilestoneProgramPhase::PreInput ? "pre_input" : "post_sim"},
            {"stable_ticks", hit.stableTicks},
            {"definition_digest", hit.definitionDigest},
            {"program_digest", hit.programDigest},
        };
        if (hit.hit) {
            item["boundary_index"] = hit.boundaryIndex;
            item["sim_tick"] = hit.simulationTick;
            item["tape_frame"] =
                hit.tapeFrame == MilestoneNoTapeFrame ? json(nullptr) : json(hit.tapeFrame);
            item["evidence"] = evidence_json(hit.evidence);
            item["projections"] = value_projections_json(hit.projections);
        } else {
            item["boundary_index"] = nullptr;
            item["sim_tick"] = nullptr;
            item["tape_frame"] = nullptr;
            item["evidence"] = nullptr;
            item["projections"] = nullptr;
        }
        milestones.push_back(std::move(item));
    }

    return json{
        {"schema",
            {
                {"name", "dusklight.automation.milestones"},
                {"version", MilestoneResultSchemaVersion},
            }},
        {"goal", tracker.goalName().has_value() ? json(*tracker.goalName()) : json(nullptr)},
        {"goal_reached", tracker.goalReached()},
        {"boot", boot_json(tracker.bootOrigin())},
        {"boot_origin_established", tracker.bootOriginEstablished()},
        {"program_digest",
            tracker.programDigest().empty() ? json(nullptr) : json(tracker.programDigest())},
        {"milestones", std::move(milestones)},
    }
        .dump(2);
}

bool write_milestone_result(
    const std::filesystem::path& path, const MilestoneTracker& tracker, std::string& error) {
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
        error = "could not open milestone result for writing";
        return false;
    }
    stream << serialize_milestone_result(tracker) << '\n';
    if (!stream) {
        error = "failed while writing milestone result";
        return false;
    }
    return true;
}

}  // namespace dusk::automation
