#pragma once

#include "dusk/automation/input_tape.hpp"
#include "dusk/automation/rng.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>
#include <vector>

namespace dusk::automation {

inline constexpr std::uint16_t GameplayTraceVersion = 5;
inline constexpr std::uint16_t GameplayTraceHeaderSize = 128;
inline constexpr std::uint16_t GameplayTraceV2HeaderSize = 64;
inline constexpr std::uint16_t GameplayTraceDirectoryEntrySize = 64;
inline constexpr std::uint16_t GameplayTraceV1HeaderSize = 36;
inline constexpr std::uint16_t GameplayTraceV1RecordSize = 102;
inline constexpr std::uint64_t GameplayTraceNoTick = ~std::uint64_t{0};
inline constexpr std::uint64_t GameplayTraceNoTapeFrame = ~std::uint64_t{0};
inline constexpr std::size_t GameplayTraceMaximumSamples = 131072;

enum class GameplayTraceChannel : std::uint16_t {
    Core = 0,
    Stage = 1,
    AppliedPads = 2,
    PlayerMotion = 3,
    Event = 4,
    SceneExit = 5,
    Rng = 6,
    Camera = 7,
    PlayerAction = 8,
    PlayerBackgroundCollision = 9,
    PlayerCollisionSurfaces = 10,
    GoalProgress = 11,
    SelectedActors = 12,
};

constexpr std::uint64_t gameplay_trace_channel_bit(const GameplayTraceChannel channel) {
    return std::uint64_t{1} << static_cast<std::uint16_t>(channel);
}

inline constexpr std::uint64_t GameplayTraceKnownChannels =
    gameplay_trace_channel_bit(GameplayTraceChannel::Core) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Stage) |
    gameplay_trace_channel_bit(GameplayTraceChannel::AppliedPads) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerMotion) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Event) |
    gameplay_trace_channel_bit(GameplayTraceChannel::SceneExit) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Rng) |
    gameplay_trace_channel_bit(GameplayTraceChannel::Camera) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerAction) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerBackgroundCollision) |
    gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces) |
    gameplay_trace_channel_bit(GameplayTraceChannel::GoalProgress) |
    gameplay_trace_channel_bit(GameplayTraceChannel::SelectedActors);
inline constexpr std::uint64_t GameplayTraceDefaultChannels =
    GameplayTraceKnownChannels &
    ~(gameplay_trace_channel_bit(GameplayTraceChannel::PlayerCollisionSurfaces) |
        gameplay_trace_channel_bit(GameplayTraceChannel::SelectedActors));

enum class GameplayTraceChannelStatus : std::uint8_t {
    NotSampled = 0,
    Present = 1,
    Absent = 2,
    Unavailable = 3,
    Truncated = 4,
};

enum class GameplayTracePhase : std::uint8_t {
    PreInput = 1,
    PostSimulation = 2,
};

enum class GameplayTraceBoundaryKind : std::uint8_t {
    Boot = 0,
    Tick = 1,
};

enum GameplayTraceInputSource : std::uint8_t {
    GameplayTraceInputNone = 0,
    GameplayTraceInputTape = 1u << 0,
    GameplayTraceInputController = 1u << 1,
    GameplayTraceInputLive = 1u << 2,
};

enum GameplayTraceCoreFlags : std::uint32_t {
    GameplayTraceSimulationTickValid = 1u << 0,
    GameplayTraceTapeFrameValid = 1u << 1,
};

enum GameplayTraceStageFlags : std::uint32_t {
    GameplayTraceNextStageEnabled = 1u << 0,
};

enum GameplayTracePlayerFlags : std::uint32_t {
    GameplayTracePlayerIsLink = 1u << 0,
};

enum GameplayTraceEventFlags : std::uint32_t {
    GameplayTraceEventRunning = 1u << 0,
    GameplayTraceEventNameHashPresent = 1u << 1,
};

enum GameplayTraceSceneExitFlags : std::uint32_t {
    GameplayTraceSceneExitVolumeValid = 1u << 0,
    GameplayTraceSceneExitPlayerInside = 1u << 1,
    GameplayTraceSceneExitPlayerLatched = 1u << 2,
    GameplayTraceSceneExitChangeOk = 1u << 3,
    GameplayTraceSceneExitChangeStarted = 1u << 4,
    GameplayTraceSceneExitDestinationValid = 1u << 5,
    GameplayTraceSceneExitObservedCountSaturated = 1u << 6,
};

enum GameplayTraceSceneExitKind : std::uint8_t {
    GameplayTraceSceneExitBox = 1,
    GameplayTraceSceneExitRadialXz = 2,
};

enum GameplayTracePlayerCollisionFlags : std::uint32_t {
    GameplayTraceCollisionGroundProbeValid = 1u << 0,
    GameplayTraceCollisionGroundContact = 1u << 1,
    GameplayTraceCollisionLanding = 1u << 2,
    GameplayTraceCollisionAway = 1u << 3,
    GameplayTraceCollisionGroundPlaneValid = 1u << 4,
    GameplayTraceCollisionGroundOwnerPresent = 1u << 5,
    GameplayTraceCollisionWallContact = 1u << 6,
    GameplayTraceCollisionRoofProbeValid = 1u << 7,
    GameplayTraceCollisionRoofContact = 1u << 8,
    GameplayTraceCollisionRoofOwnerPresent = 1u << 9,
    GameplayTraceCollisionWaterProbeEnabled = 1u << 10,
    GameplayTraceCollisionWaterSurfaceFound = 1u << 11,
    GameplayTraceCollisionWaterIn = 1u << 12,
    GameplayTraceCollisionWaterOwnerPresent = 1u << 13,
    GameplayTraceCollisionWallProbeEnabled = 1u << 14,
    GameplayTraceCollisionTrajectoryValid = 1u << 15,
    GameplayTraceCollisionGroundIdentityPresent = 1u << 16,
    GameplayTraceCollisionRoofIdentityPresent = 1u << 17,
    GameplayTraceCollisionWaterIdentityPresent = 1u << 18,
};

enum GameplayTraceCollisionWallFlags : std::uint16_t {
    GameplayTraceCollisionWallHit = 1u << 0,
    GameplayTraceCollisionWallOwnerPresent = 1u << 1,
    GameplayTraceCollisionWallIdentityPresent = 1u << 2,
};

enum GameplayTraceCollisionSurfaceSetFlags : std::uint32_t {
    GameplayTraceCollisionSurfaceCurrentRoomValid = 1u << 0,
    GameplayTraceCollisionSurfaceExplicitLinkExitPresent = 1u << 1,
    GameplayTraceCollisionSurfaceNextStagePending = 1u << 2,
};

enum GameplayTraceCollisionSurfaceFlags : std::uint32_t {
    GameplayTraceCollisionSurfaceIdentityPresent = 1u << 0,
    GameplayTraceCollisionSurfaceOwnerPresent = 1u << 1,
    GameplayTraceCollisionSurfaceBackingResolved = 1u << 2,
    GameplayTraceCollisionSurfaceRawCodesPresent = 1u << 3,
    GameplayTraceCollisionSurfaceMaterialPresent = 1u << 4,
    GameplayTraceCollisionSurfaceGroupPresent = 1u << 5,
    GameplayTraceCollisionSurfaceSourceRoomPresent = 1u << 6,
    GameplayTraceCollisionSurfaceSourceRoomExact = 1u << 7,
    GameplayTraceCollisionSurfaceSclsSourcePresent = 1u << 8,
    GameplayTraceCollisionSurfaceDestinationPresent = 1u << 9,
    GameplayTraceCollisionSurfaceDestinationMatchesPending = 1u << 10,
    GameplayTraceCollisionSurfaceGeometryPresent = 1u << 11,
    GameplayTraceCollisionSurfaceKclHeightPresent = 1u << 12,
};

enum GameplayTraceCollisionSurfaceKind : std::uint8_t {
    GameplayTraceCollisionSurfaceGround = 1,
    GameplayTraceCollisionSurfaceRoof = 2,
    GameplayTraceCollisionSurfaceWater = 3,
    GameplayTraceCollisionSurfaceWall = 4,
};

enum GameplayTraceCollisionBackingFormat : std::uint8_t {
    GameplayTraceCollisionBackingNone = 0,
    GameplayTraceCollisionBackingDzb = 1,
    GameplayTraceCollisionBackingKcl = 2,
};

enum GameplayTraceGoalFlags : std::uint32_t {
    GameplayTraceGoalConfigured = 1u << 0,
    GameplayTraceGoalReached = 1u << 1,
    GameplayTraceGoalAuthored = 1u << 2,
    GameplayTraceGoalFirstHitTickPresent = 1u << 3,
};

enum GameplayTraceSelectedActorFlags : std::uint32_t {
    GameplayTraceSelectedActorsTruncated = 1u << 0,
};

enum GameplayTracePlayerActionFlags : std::uint32_t {
    GameplayTraceTalkPartnerPresent = 1u << 0,
    GameplayTraceGrabbedActorPresent = 1u << 1,
};

enum GameplayTraceRetentionTrigger : std::uint32_t {
    GameplayTraceTriggerCrash = 1u << 0,
    GameplayTraceTriggerNovelContact = 1u << 1,
    GameplayTraceTriggerFlagChange = 1u << 2,
    GameplayTraceTriggerPredicateHit = 1u << 3,
};

inline constexpr std::uint32_t GameplayTraceKnownRetentionTriggers =
    GameplayTraceTriggerCrash | GameplayTraceTriggerNovelContact |
    GameplayTraceTriggerFlagChange | GameplayTraceTriggerPredicateHit;

struct GameplayTraceRetentionConfig {
    std::size_t preTriggerTicks = 0;
    std::size_t postTriggerTicks = 0;
    std::uint32_t triggers = 0;

    bool enabled() const { return triggers != 0; }
};

inline constexpr std::size_t GameplayTraceSelectedActorCapacity = 16;

struct GameplayTraceCoreSample {
    std::uint64_t boundaryIndex = 0;
    std::uint64_t simulationTick = GameplayTraceNoTick;
    std::uint64_t tapeFrame = GameplayTraceNoTapeFrame;
    std::uint32_t flags = 0;
    GameplayTracePhase phase = GameplayTracePhase::PostSimulation;
    GameplayTraceBoundaryKind boundaryKind = GameplayTraceBoundaryKind::Tick;
    std::uint8_t inputSource = GameplayTraceInputNone;
};

struct GameplayTraceStageSample {
    std::array<char, 8> stageName{};
    std::int8_t room = 0;
    std::int8_t layer = -1;
    std::int16_t point = 0;
    std::array<char, 8> nextStageName{};
    std::int8_t nextRoom = 0;
    std::int8_t nextLayer = -1;
    std::int16_t nextPoint = 0;
    std::uint32_t flags = 0;
};

struct GameplayTraceAppliedPadsSample {
    std::uint8_t validPorts = 0;
    std::uint8_t ownedPorts = 0;
    std::array<RawPadState, kInputPortCount> pads{};
};

struct GameplayTracePlayerMotionSample {
    std::uint32_t sessionProcessId = 0;
    std::int16_t actorName = -1;
    std::uint16_t procedureId = 0xffff;
    std::array<std::int16_t, 3> currentAngle{};
    std::array<std::int16_t, 3> shapeAngle{};
    std::array<float, 3> position{};
    std::array<float, 3> velocity{};
    float forwardSpeed = 0.0f;
    std::uint32_t flags = 0;
};

struct GameplayTraceEventSample {
    std::uint32_t flags = 0;
    std::int16_t eventId = -1;
    std::uint8_t mode = 0;
    std::uint8_t status = 0;
    std::uint8_t mapToolId = 0xff;
    std::uint32_t nameHash = 0;
};

struct GameplayTraceSceneExitSample {
    std::uint32_t sessionProcessId = 0;
    std::uint32_t rawParameters = 0;
    std::uint32_t flags = 0;
    float signedDistanceToVolume = 0.0f;
    std::int16_t actorName = -1;
    std::uint16_t setId = 0;
    std::uint8_t exitId = 0xff;
    std::uint8_t pathId = 0xff;
    std::uint8_t argument1 = 0xff;
    std::uint8_t switchNo = 0xff;
    std::uint8_t kind = 0;
    std::uint8_t observedCount = 0;
    std::int8_t homeRoom = -1;
    std::uint8_t linkExitDirection = 0xff;
    std::uint16_t linkExitId = 0xffff;
    std::int16_t shapeYaw = 0;
    std::uint8_t actorAction = 0xff;
    std::array<float, 3> playerLocalPosition{};
    std::array<float, 3> volumeExtent{};
    std::array<float, 3> homePosition{};
    std::array<char, 8> destinationStage{};
    std::int8_t destinationRoom = -1;
    std::int8_t destinationLayer = -1;
    std::int16_t destinationPoint = -1;
    std::uint8_t destinationWipe = 0xff;
    std::uint8_t destinationWipeTime = 0xff;
    std::int8_t destinationTimeHour = -1;
};

struct GameplayTraceCollisionWallSample {
    std::uint16_t bgIndex = 0xffff;
    std::uint16_t polyIndex = 0xffff;
    std::uint32_t ownerSessionProcessId = 0xffffffffu;
    std::int16_t angleY = 0;
    std::uint16_t flags = 0;
};

struct GameplayTracePlayerBackgroundCollisionSample {
    std::uint32_t flags = 0;
    float groundHeight = -1000000000.0f;
    float roofHeight = 1000000000.0f;
    float waterHeight = -1000000000.0f;
    std::uint16_t groundBgIndex = 0xffff;
    std::uint16_t groundPolyIndex = 0xffff;
    std::uint32_t groundOwnerSessionProcessId = 0xffffffffu;
    std::array<float, 4> groundPlane{};
    std::uint16_t roofBgIndex = 0xffff;
    std::uint16_t roofPolyIndex = 0xffff;
    std::uint32_t roofOwnerSessionProcessId = 0xffffffffu;
    std::uint16_t waterBgIndex = 0xffff;
    std::uint16_t waterPolyIndex = 0xffff;
    std::uint32_t waterOwnerSessionProcessId = 0xffffffffu;
    std::array<GameplayTraceCollisionWallSample, 3> walls{};
    std::array<float, 3> oldPosition{};
    std::array<float, 3> resolvedFrameDisplacement{};
    std::array<float, 3> finalPosition{};
};

struct GameplayTraceCollisionSurfaceSample {
    std::uint32_t flags = 0;
    std::uint8_t kind = 0;
    std::uint8_t wallSlot = 0;
    std::uint8_t backingFormat = GameplayTraceCollisionBackingNone;
    std::uint8_t rawCodePresenceMask = 0;
    std::uint16_t bgIndex = 0xffff;
    std::uint16_t polyIndex = 0xffff;
    std::uint32_t ownerSessionProcessId = 0xffffffffu;
    std::uint16_t materialIndex = 0xffff;
    std::uint16_t groupIndex = 0xffff;
    std::array<std::uint32_t, 5> rawCodes{};
    std::uint8_t rawExitId = 0xff;
    std::int8_t sourceRoom = -128;
    std::int8_t sclsSourceRoom = -128;
    std::int8_t destinationRoom = -128;
    std::int8_t destinationLayer = -128;
    std::uint8_t destinationWipe = 0xff;
    std::uint8_t destinationWipeTime = 0xff;
    std::int8_t destinationTimeHour = -128;
    std::int16_t destinationPoint = -32768;
    std::uint8_t sourceGeometryIndexCount = 0;
    std::array<std::uint16_t, 6> sourceGeometryIndices{
        0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff};
    float kclPrismHeight = 0.0f;
    std::array<char, 8> destinationStage{};
};

struct GameplayTracePlayerCollisionSurfacesSample {
    GameplayTracePlayerCollisionSurfacesSample() {
        surfaces[0].kind = GameplayTraceCollisionSurfaceGround;
        surfaces[1].kind = GameplayTraceCollisionSurfaceRoof;
        surfaces[2].kind = GameplayTraceCollisionSurfaceWater;
        for (std::size_t index = 0; index < 3; ++index) {
            surfaces[index + 3].kind = GameplayTraceCollisionSurfaceWall;
            surfaces[index + 3].wallSlot = static_cast<std::uint8_t>(index);
        }
    }

    std::uint32_t flags = 0;
    std::int8_t currentRoom = -128;
    std::uint8_t identityCount = 0;
    std::uint8_t backingCodeCount = 0;
    std::uint8_t destinationCount = 0;
    std::uint16_t rawLinkExit = 0x003f;
    std::uint8_t pendingStageMatchMask = 0;
    std::array<GameplayTraceCollisionSurfaceSample, 6> surfaces{};
};

struct GameplayTraceCameraSample {
    std::int16_t viewYaw = 0;
    std::int16_t controlledYaw = 0;
    std::int16_t bank = 0;
    std::array<float, 3> eye{};
    std::array<float, 3> center{};
    std::array<float, 3> up{};
    float fovy = 0.0f;
};

struct GameplayTraceAnimationLane {
    std::uint16_t resourceId = 0xffff;
    float frame = 0.0f;
    float rate = 0.0f;
};

// Portable placement identity plus a diagnostic session-local process ID. The
// process ID is never sufficient evidence by itself; actorName/setId/homeRoom
// are the stable selector used by objectives and offline comparison.
struct GameplayTraceActorIdentitySample {
    std::uint32_t sessionProcessId = 0xffffffffu;
    std::int16_t actorName = -1;
    std::uint16_t setId = 0xffff;
    std::int8_t homeRoom = -1;
    std::int8_t currentRoom = -1;
};

struct GameplayTracePlayerActionSample {
    std::uint16_t procedureId = 0xffff;
    std::uint32_t modeFlags = 0;
    std::array<std::int16_t, 6> procedureContextRaw{};
    std::int16_t damageWaitTimer = 0;
    std::uint16_t swordAtUpTime = 0;
    std::int16_t iceDamageWaitTimer = 0;
    std::uint8_t swordChangeWaitTimer = 0;
    std::array<GameplayTraceAnimationLane, 3> underAnimations{};
    std::array<GameplayTraceAnimationLane, 3> upperAnimations{};
    std::uint32_t flags = 0;
    std::uint8_t doStatus = 0;
    GameplayTraceActorIdentitySample talkPartner{};
    GameplayTraceActorIdentitySample grabbedActor{};
};

struct GameplayTraceGoalProgressSample {
    std::uint32_t flags = 0;
    std::uint32_t goalNameHash = 0;
    std::uint16_t requestedCount = 0;
    std::uint16_t hitCount = 0;
    std::uint16_t stableTicks = 0;
    std::uint16_t consecutiveTicks = 0;
    std::uint8_t sequenceSteps = 0;
    std::uint8_t sequenceNextStep = 0;
    std::uint16_t sequenceWithinTicks = 0;
    std::uint16_t sequenceElapsedTicks = 0;
    std::uint64_t firstHitTick = GameplayTraceNoTick;
};

struct GameplayTraceSelectedActorSample {
    std::uint32_t sessionProcessId = 0xffffffffu;
    std::int16_t actorName = -1;
    std::uint16_t setId = 0xffff;
    std::int8_t homeRoom = -1;
    std::int8_t currentRoom = -1;
    std::int16_t health = 0;
    std::uint32_t status = 0;
    std::array<float, 3> position{};
    std::array<std::int16_t, 3> currentAngle{};
    std::array<std::int16_t, 3> shapeAngle{};
};

struct GameplayTraceSelectedActorsSample {
    std::uint16_t count = 0;
    std::uint16_t capacity = GameplayTraceSelectedActorCapacity;
    std::uint32_t flags = 0;
    std::uint32_t observedCount = 0;
    std::array<GameplayTraceSelectedActorSample, GameplayTraceSelectedActorCapacity> actors{};
};

struct GameplayTraceSample {
    GameplayTraceCoreSample core{};
    GameplayTraceStageSample stage{};
    GameplayTraceAppliedPadsSample appliedPads{};
    GameplayTracePlayerMotionSample playerMotion{};
    GameplayTraceEventSample event{};
    GameplayTraceSceneExitSample sceneExit{};
    GameRngSnapshot rng{};
    GameplayTraceCameraSample camera{};
    GameplayTracePlayerActionSample playerAction{};
    GameplayTracePlayerBackgroundCollisionSample playerBackgroundCollision{};
    GameplayTracePlayerCollisionSurfacesSample playerCollisionSurfaces{};
    GameplayTraceGoalProgressSample goalProgress{};
    GameplayTraceSelectedActorsSample selectedActors{};

    GameplayTraceChannelStatus stageStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus appliedPadsStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus playerMotionStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus eventStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus sceneExitStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus rngStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus cameraStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus playerActionStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus playerBackgroundCollisionStatus =
        GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus playerCollisionSurfacesStatus =
        GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus goalProgressStatus = GameplayTraceChannelStatus::NotSampled;
    GameplayTraceChannelStatus selectedActorsStatus = GameplayTraceChannelStatus::NotSampled;
};

class GameplayTraceRecorder {
public:
    void start(
        std::size_t capacity, std::uint64_t requestedChannels = GameplayTraceDefaultChannels,
        TapeBoot boot = {}, GameplayTraceRetentionConfig retention = {});
    void record(const GameplayTraceSample& sample);
    void trigger(GameplayTraceRetentionTrigger trigger);
    void stop();

    bool active() const { return mActive; }
    bool capacityExhausted() const { return mCapacityExhausted; }
    std::uint64_t requestedChannels() const { return mRequestedChannels; }
    const std::vector<GameplayTraceSample>& samples() const { return mSamples; }
    const TapeBoot& bootOrigin() const { return mBootOrigin; }
    const GameplayTraceRetentionConfig& retention() const { return mRetention; }
    std::uint32_t observedTriggers() const { return mObservedTriggers; }
    std::uint32_t triggerCount() const { return mTriggerCount; }
    std::uint64_t observedSampleCount() const { return mObservedSampleCount; }

private:
    void retain(const GameplayTraceSample& sample);
    void flushPreTrigger();
    std::uint32_t detectTriggers(const GameplayTraceSample& sample) const;

    std::vector<GameplayTraceSample> mSamples;
    std::vector<GameplayTraceSample> mPreTrigger;
    std::uint64_t mRequestedChannels = GameplayTraceDefaultChannels;
    TapeBoot mBootOrigin;
    GameplayTraceRetentionConfig mRetention;
    GameplayTraceSample mPreviousSample{};
    std::size_t mPreTriggerHead = 0;
    std::size_t mPostTriggerRemaining = 0;
    std::uint32_t mObservedTriggers = 0;
    std::uint32_t mTriggerCount = 0;
    std::uint64_t mObservedSampleCount = 0;
    bool mPreviousSampleValid = false;
    bool mActive = false;
    bool mCapacityExhausted = false;
};

GameplayTraceRecorder& gameplay_trace_recorder();

bool parse_gameplay_trace_channels(
    std::string_view text, std::uint64_t& channels, std::string& error);
bool parse_gameplay_trace_retention_triggers(
    std::string_view text, std::uint32_t& triggers, std::string& error);
bool write_gameplay_trace(
    const std::filesystem::path& path, const GameplayTraceRecorder& recorder, std::string& error);

}  // namespace dusk::automation
