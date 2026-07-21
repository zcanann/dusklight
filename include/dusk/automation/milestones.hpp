#pragma once

#include <array>
#include <cstdint>
#include <filesystem>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "dusk/automation/input_tape.hpp"
#include "dusk/automation/milestone_program.hpp"
#include "dusk/automation/rng.hpp"

namespace dusk::automation {

inline constexpr std::uint32_t MilestoneResultSchemaVersion = 5;
inline constexpr std::uint32_t MilestoneBoundaryFingerprintVersion = 6;
inline constexpr std::uint32_t MilestoneObservationFingerprintVersion = 4;
inline constexpr std::uint64_t MilestoneNoTapeFrame = ~std::uint64_t{0};

enum class MilestoneId : std::uint8_t {
    GameplayReadyFSp103,
    ExitFSp103ToFSp104,
    EnteredFSp104,
};

struct MilestoneObservation {
    enum class ChannelStatus : std::uint8_t {
        NotSampled = 0,
        Present = 1,
        Absent = 2,
        Unavailable = 3,
    };

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

    // Read-only menu observations used to prove deterministic TAS boundaries.
    bool titlePresent = false;
    std::uint8_t titleProcedure = 0xff;
    bool titleLogoSkipReady = false;
    bool titleStartReady = false;
    bool nameEntryActive = false;
    bool nameEntryCharacterSelectReady = false;
    bool nameEntryInputReady = false;
    std::uint8_t nameEntrySelectionProcedure = 0xff;
    bool fileSelectNoSaveReady = false;
    bool fileSelectDataSelectReady = false;
    bool fileSelectKeyWaitReady = false;
    bool fileSelectYesNoReady = false;
    bool nameScenePresent = false;
    std::uint8_t nameSceneProcedure = 0xff;
    bool fileSelectPresent = false;
    std::uint8_t fileSelectProcedure = 0xff;
    std::uint8_t fileSelectCardCheckProcedure = 0xff;

    bool nextStageEnabled = false;
    const char* nextStageName = nullptr;
    std::int8_t nextRoom = -1;
    std::int8_t nextLayer = -1;
    std::int16_t nextPoint = -1;

    GameRngSnapshot rng;

    // Typed, read-only gameplay resources. This deliberately exposes semantic
    // save-state fields rather than copying dSv_player_c bytes or host padding.
    // The component is unavailable before an active player exists.
    struct PlayerResources {
        static constexpr std::size_t InventorySlotCount = 24;
        static constexpr std::size_t SelectItemCount = 4;
        static constexpr std::size_t EquipmentCount = 6;
        static constexpr std::size_t BombBagCount = 3;
        static constexpr std::size_t BottleCount = 4;
        static constexpr std::size_t AcquiredItemByteCount = 32;
        static constexpr std::size_t CollectItemByteCount = 8;

        std::uint16_t maximumLife = 0;
        std::uint16_t life = 0;
        std::uint16_t rupees = 0;
        std::uint16_t rupeeCapacity = 0;
        std::uint16_t maximumOil = 0;
        std::uint16_t oil = 0;
        std::uint8_t maximumMagic = 0;
        std::uint8_t magic = 0;
        std::uint8_t wallet = 0;
        std::uint8_t transformStatus = 0;
        float worldTime = 0.0F;
        std::uint16_t date = 0;
        std::uint8_t arrows = 0;
        std::uint8_t arrowCapacity = 0;
        std::uint8_t pachinko = 0;
        std::uint8_t poeSouls = 0;
        std::uint8_t smallKeys = 0;
        bool dungeonMap = false;
        bool dungeonCompass = false;
        bool dungeonBossKey = false;
        bool dungeonWarp = false;
        std::array<std::uint8_t, InventorySlotCount> inventory{};
        std::array<std::uint8_t, SelectItemCount> selectedItems{};
        std::array<std::uint8_t, SelectItemCount> mixedItems{};
        std::array<std::uint8_t, EquipmentCount> equipment{};
        std::array<std::uint8_t, BombBagCount> bombCounts{};
        std::array<std::uint8_t, BombBagCount> bombCapacities{};
        std::array<std::uint8_t, BottleCount> bottleQuantities{};
        std::array<std::uint8_t, AcquiredItemByteCount> acquiredItemBits{};
        std::array<std::uint8_t, CollectItemByteCount> collectItemBits{};
        std::uint8_t collectedCrystalBits = 0;
        std::uint8_t collectedMirrorBits = 0;
    };
    PlayerResources playerResources;
    bool playerResourcesPresent = false;

    // Pointer-free semantic relationships rooted at Link. Native pointers are
    // resolved immediately to the same stable actor identity used by the
    // complete learner actor population and never cross this boundary.
    struct PlayerRelationships {
        ActorIdentity targetedActor;
        ActorIdentity rideActor;
        ActorIdentity heldItemActor;
        ActorIdentity grabbedActor;
        ActorIdentity thrownBoomerangActor;
        ActorIdentity copyRodActor;
        ActorIdentity hookshotRoofWaitActor;
        ActorIdentity chainGrabActor;
        ActorIdentity attentionHintActor;
        ActorIdentity attentionCatchActor;
        ActorIdentity attentionLookActor;
    };
    PlayerRelationships playerRelationships;
    bool playerRelationshipsPresent = false;

    // Cached Link background-collision solver state. This is copied after the
    // game has run its own solver; observation never invokes a collision query.
    struct PlayerCollisionSolver {
        static constexpr std::size_t WallCircleCount = 3;

        struct WallCircle {
            std::uint32_t flags = 0;
            std::int16_t angleY = 0;
            float wallRadiusSquared = 0.0F;
            float wallHeight = 0.0F;
            float wallRadius = 0.0F;
            float directWallHeight = 0.0F;
            std::array<float, 3> realizedCenter{};
            float realizedRadius = 0.0F;
        };

        std::uint32_t flags = 0;
        std::int32_t wallTableSize = 0;
        std::uint8_t waterMode = 0;
        std::array<float, 3> lineStart{};
        std::array<float, 3> lineEnd{};
        std::array<float, 3> wallCylinderCenter{};
        float wallCylinderRadius = 0.0F;
        float wallCylinderHeight = 0.0F;
        float groundCheckOffset = 0.0F;
        float roofCorrectionHeight = 0.0F;
        float waterCheckOffset = 0.0F;
        std::array<WallCircle, WallCircleCount> wallCircles{};
    };
    PlayerCollisionSolver playerCollisionSolver;
    bool playerCollisionSolverPresent = false;

    // Planner-facing save/runtime state is deliberately kept separate from
    // PlayerResources. mDataNum identifies the selected zero-based card slot,
    // while mNoFile is a distinct legacy marker whose nonzero meanings differ
    // between supported ports. Preserve both raw values and only claim an
    // attachment when the live engine state makes it unambiguous.
    struct PhysicalSlot {
        std::uint8_t number = 0;
        ChannelStatus contentStatus = ChannelStatus::NotSampled;
        bool attachedToRuntime = false;
    };
    struct RuntimeFileState {
        ChannelStatus status = ChannelStatus::NotSampled;
        ChannelStatus backingAttachmentStatus = ChannelStatus::NotSampled;
        std::uint8_t noFileRaw = 0;
        std::uint8_t dataNumRaw = 0;
        std::int8_t attachedPhysicalSlot = -1;
        std::array<PhysicalSlot, 3> physicalSlots{};
    };
    RuntimeFileState runtimeFile;

    struct ReturnPlaceState {
        ChannelStatus status = ChannelStatus::NotSampled;
        std::array<char, 8> stage{};
        std::int8_t room = -1;
        std::uint8_t playerStatus = 0;
    };
    ReturnPlaceState returnPlace;

    struct RestartState {
        ChannelStatus status = ChannelStatus::NotSampled;
        std::int8_t room = -1;
        std::int16_t startPoint = -1;
        std::int16_t angleY = 0;
        float positionX = 0.0F;
        float positionY = 0.0F;
        float positionZ = 0.0F;
        std::uint32_t roomParam = 0;
        float lastSpeed = 0.0F;
        std::uint32_t lastMode = 0;
        std::int16_t lastAngleY = 0;
    };
    RestartState restart;

    struct EventHandoffState {
        ChannelStatus status = ChannelStatus::NotSampled;
        std::uint8_t preItemNo = 0;
        std::uint8_t getItemNo = 0;
        std::uint16_t eventFlags = 0;
        std::uint16_t secondaryFlags = 0;
        std::uint16_t hindFlags = 0;
        std::uint8_t talkXyType = 0;
        std::uint8_t compulsory = 0;
        bool roomInfoSet = false;
        std::int32_t skipTimer = 0;
        std::int32_t skipParameter = 0;
        ActorIdentity itemPartner;
        ChannelStatus eventNameStatus = ChannelStatus::NotSampled;
        std::array<char, 64> eventName{};
        ChannelStatus messageFlowStatus = ChannelStatus::Unavailable;
        std::uint16_t messageFlowId = 0;
        std::uint16_t messageNodeIndex = 0;
        ChannelStatus messageCutStatus = ChannelStatus::NotSampled;
        std::uint32_t messageCutHash = 0;
        ChannelStatus pendingCleanupStatus = ChannelStatus::Unavailable;
        std::uint32_t pendingCleanupFlags = 0;
        ChannelStatus playerControlStatus = ChannelStatus::NotSampled;
        std::uint32_t playerControlModeFlags = 0;
        std::uint8_t playerControlDoStatus = 0;
        ChannelStatus noTelopStatus = ChannelStatus::NotSampled;
        bool noTelop = false;
    };
    EventHandoffState eventHandoff;

    // Global message-system state, observed through public read accessors. This
    // is deliberately independent of any NPC class or authored dialogue flow:
    // a learner sees the currently realized message session without the
    // observer advancing it or encoding how it was reached.
    struct MessageSessionState {
        enum Flag : std::uint16_t {
            TalkNow = 1u << 0,
            TalkMessage = 1u << 1,
            AutoMessage = 1u << 2,
            KillPending = 1u << 3,
            CameraCancel = 1u << 4,
            Send = 1u << 5,
            SendControl = 1u << 6,
        };

        ChannelStatus status = ChannelStatus::NotSampled;
        std::uint16_t procedure = 0;
        std::uint32_t messageId = 0;
        std::int32_t messageIndex = 0;
        std::uint16_t nodeIndex = 0;
        std::int16_t flowId = 0;
        std::uint8_t selectionCount = 0;
        std::uint8_t selectionCursor = 0;
        std::uint8_t selectionPush = 0;
        std::uint8_t outputType = 0;
        std::uint16_t flags = 0;
        ActorIdentity talkActor;
    };
    MessageSessionState messageSession;

    // Pending event requests and the actors participating in the active event.
    // The game stores native pointers in this subsystem; the observer resolves
    // them immediately to the same stable actor identities used by the complete
    // learner population. Queue order is semantic priority order, not the
    // incidental backing-array order.
    struct EventQueueState {
        static constexpr std::size_t MaximumPendingOrders = 8;

        struct ActorReference {
            ChannelStatus status = ChannelStatus::NotSampled;
            ActorIdentity identity;
        };

        struct PendingOrder {
            std::uint16_t type = 0;
            std::uint16_t flags = 0;
            std::uint16_t hindFlags = 0;
            std::int16_t eventId = -1;
            std::uint16_t priority = 0;
            std::uint8_t mapToolId = 0xff;
            ActorReference requestActor;
            ActorReference targetActor;
        };

        ChannelStatus status = ChannelStatus::NotSampled;
        std::uint8_t pendingCount = 0;
        std::array<PendingOrder, MaximumPendingOrders> pendingOrders{};
        ActorReference activeRequestActor;
        ActorReference activeTargetActor;
        ActorReference activeTalkActor;
        ActorReference activeItemActor;
        ActorReference activeDoorActor;
        ActorReference changeActor;
        bool skipRegistered = false;
        ActorReference skipActor;
    };
    EventQueueState eventQueue;

    struct Actor {
        // The port preserves the GameCube actor layout: nine attention lanes.
        static constexpr std::size_t AttentionDistanceCount = 9;

        struct AttentionComponent {
            std::uint32_t flags = 0;
            float positionX = 0.0f;
            float positionY = 0.0f;
            float positionZ = 0.0f;
            std::array<std::uint8_t, AttentionDistanceCount> distanceIndices{};
            std::int16_t auxiliary = 0;
        };

        struct EventParticipationComponent {
            std::uint16_t command = 0;
            std::uint16_t condition = 0;
            std::int16_t eventId = -1;
            std::uint8_t mapToolId = 0xff;
            std::uint8_t index = 0;
        };

        // Decoded configuration and boundary-local guard evaluation for the
        // SavMem (KYTAG14) actor. The target is authored in actor parameters,
        // while the predicates read shared save/temporary backing stores. Keep
        // both so a planner can derive reachability instead of hard-coding a
        // "save location changed" effect.
        struct ReturnPlaceWriterComponent {
            std::int8_t saveRoom = -1;
            std::uint8_t savePoint = 0;
            std::int8_t switchRoom = -1;
            std::uint16_t requiredEventSet = 0xffff;
            std::uint16_t requiredEventUnset = 0xffff;
            std::uint8_t requiredSwitchSet = 0xff;
            std::uint8_t requiredSwitchUnset = 0xff;
            bool noTelopClear = false;
            bool eventSetSatisfied = false;
            bool eventUnsetSatisfied = false;
            bool switchSetSatisfied = false;
            bool switchUnsetSatisfied = false;
            bool eligible = false;
        };

        // Shared fopEn_enemy_c state. This is a typed optional component gated
        // by the actor's enemy group, never a guessed profile offset.
        struct EnemyBaseComponent {
            std::uint16_t flags = 0;
            std::uint8_t throwMode = 0;
            float downPositionX = 0.0f;
            float downPositionY = 0.0f;
            float downPositionZ = 0.0f;
            float headLockPositionX = 0.0f;
            float headLockPositionY = 0.0f;
            float headLockPositionZ = 0.0f;
        };

        // Profile-bound trigger geometry already interpreted by the native
        // actor. This is a read-only description of the active volume and its
        // current gate state; it does not invoke the trigger or nominate one
        // as an objective.
        enum class TriggerVolumeKind : std::uint8_t {
            SceneExit = 1,
            SceneExitCylinder = 2,
            EventArea = 3,
            ScriptedEvent = 4,
            MappedEvent = 5,
        };

        enum class TriggerVolumeShape : std::uint8_t {
            Box = 1,
            EllipticCylinder = 2,
        };

        struct TriggerVolumeComponent {
            TriggerVolumeKind kind = TriggerVolumeKind::SceneExit;
            TriggerVolumeShape shape = TriggerVolumeShape::Box;
            bool enabled = false;
            bool verticalUnbounded = false;
            std::uint16_t behavior = 0;
            float centerX = 0.0f;
            float centerY = 0.0f;
            float centerZ = 0.0f;
            float halfExtentX = 0.0f;
            float halfExtentY = 0.0f;
            float halfExtentZ = 0.0f;
            std::int16_t yaw = 0;
        };

        std::uint64_t runtimeGeneration = 0;
        std::int32_t actorType = 0;
        std::int32_t processSubtype = 0;
        std::int16_t actorName = -1;
        std::uint16_t setId = 0xffff;
        std::int8_t homeRoom = -1;
        std::int8_t oldRoom = -1;
        std::int8_t currentRoom = -1;
        float positionX = 0.0f;
        float positionY = 0.0f;
        float positionZ = 0.0f;
        std::int16_t health = 0;
        std::uint32_t status = 0;
        std::uint32_t condition = 0;
        std::uint32_t parentRuntimeGeneration = 0xffffffff;
        std::uint32_t parameters = 0;
        std::int16_t profileName = -1;
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
        float homePositionX = 0.0f;
        float homePositionY = 0.0f;
        float homePositionZ = 0.0f;
        float oldPositionX = 0.0f;
        float oldPositionY = 0.0f;
        float oldPositionZ = 0.0f;
        float velocityX = 0.0f;
        float velocityY = 0.0f;
        float velocityZ = 0.0f;
        float forwardSpeed = 0.0f;
        float scaleX = 1.0f;
        float scaleY = 1.0f;
        float scaleZ = 1.0f;
        float gravity = 0.0f;
        float maxFallSpeed = 0.0f;
        float eyePositionX = 0.0f;
        float eyePositionY = 0.0f;
        float eyePositionZ = 0.0f;
        std::int16_t homeAngleX = 0;
        std::int16_t homeAngleY = 0;
        std::int16_t homeAngleZ = 0;
        std::int16_t oldAngleX = 0;
        std::int16_t oldAngleY = 0;
        std::int16_t oldAngleZ = 0;
        std::int16_t currentAngleX = 0;
        std::int16_t currentAngleY = 0;
        std::int16_t currentAngleZ = 0;
        std::int16_t shapeAngleX = 0;
        std::int16_t shapeAngleY = 0;
        std::int16_t shapeAngleZ = 0;
        // These are semantic optional components. The underlying legacy actor
        // storage exists for every actor, but its payload is only retained when
        // the corresponding gameplay facility is active. This prevents default
        // constructor bytes from masquerading as universally meaningful state.
        bool attentionPresent = false;
        AttentionComponent attention;
        bool eventParticipationPresent = false;
        EventParticipationComponent eventParticipation;
        bool returnPlaceWriterPresent = false;
        ReturnPlaceWriterComponent returnPlaceWriter;
        bool enemyBasePresent = false;
        EnemyBaseComponent enemyBase;
        bool triggerVolumePresent = false;
        TriggerVolumeComponent triggerVolume;
    };
    std::span<const Actor> actors;
    // Total actor population visited by the observer. Current native learning
    // capture requires this to equal actors.size(); the truncation marker is
    // retained only so older or explicitly bounded observations fail closed.
    std::uint32_t actorObservedCount = 0;
    bool actorsTruncated = false;

    enum class DynamicColliderShape : std::uint8_t {
        Unknown = 0,
        Sphere = 1,
        Cylinder = 2,
    };

    struct DynamicCollider {
        std::uint16_t registrationIndex = 0;
        std::uint32_t ownerRuntimeGeneration = 0xffffffff;
        std::uint32_t attackHitOwnerRuntimeGeneration = 0xffffffff;
        std::uint32_t targetHitOwnerRuntimeGeneration = 0xffffffff;
        std::uint32_t correctionHitOwnerRuntimeGeneration = 0xffffffff;
        bool ownerPresent = false;
        bool statusPresent = false;
        bool shapePresent = false;
        bool attackSet = false;
        bool targetSet = false;
        bool correctionSet = false;
        bool attackHit = false;
        bool targetHit = false;
        bool correctionHit = false;
        bool attackHitOwnerPresent = false;
        bool targetHitOwnerPresent = false;
        bool correctionHitOwnerPresent = false;
        DynamicColliderShape shape = DynamicColliderShape::Unknown;
        std::uint32_t attackType = 0;
        std::uint32_t targetType = 0;
        std::uint32_t attackSourceParameters = 0;
        std::uint32_t attackResultParameters = 0;
        std::uint32_t targetSourceParameters = 0;
        std::uint32_t targetResultParameters = 0;
        std::uint32_t correctionSourceParameters = 0;
        std::uint32_t correctionResultParameters = 0;
        std::uint8_t attackPower = 0;
        std::uint8_t weight = 0;
        std::uint8_t damage = 0;
        float centerX = 0.0f;
        float centerY = 0.0f;
        float centerZ = 0.0f;
        float radius = 0.0f;
        float height = 0.0f;
        float aabbMinX = 0.0f;
        float aabbMinY = 0.0f;
        float aabbMinZ = 0.0f;
        float aabbMaxX = 0.0f;
        float aabbMaxY = 0.0f;
        float aabbMaxZ = 0.0f;
        float correctionX = 0.0f;
        float correctionY = 0.0f;
        float correctionZ = 0.0f;
    };
    // Complete dynamic collision set processed by the immediately preceding
    // collision pass. At a pre-input boundary this is the prior tick; at a
    // post-simulation boundary it is the just-completed tick.
    std::span<const DynamicCollider> dynamicColliders;
    bool dynamicCollidersPresent = false;
    bool dynamicCollidersTruncated = false;

    // Indexed flag snapshots are immutable copies captured at the same phase
    // as the scalar observation. Switches cover exactly switchFlagRoom; an
    // off-room query evaluates as unavailable rather than reading live state.
    std::span<const std::uint8_t> eventFlags;
    std::span<const std::uint8_t> temporaryFlags;
    // Exact dSv_info_c::mTmp.mEvent register bank. Unlike temporaryFlags,
    // this preserves multi-bit temporary registers used by message/event flow.
    std::span<const std::uint8_t> temporaryEventBytes;
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
    std::string cardFixtureIdentity;
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

    bool titlePresent = false;
    std::uint8_t titleProcedure = 0xff;
    bool titleLogoSkipReady = false;
    bool titleStartReady = false;
    bool nameEntryActive = false;
    bool nameEntryCharacterSelectReady = false;
    bool nameEntryInputReady = false;
    std::uint8_t nameEntrySelectionProcedure = 0xff;
    bool fileSelectNoSaveReady = false;
    bool fileSelectDataSelectReady = false;
    bool fileSelectKeyWaitReady = false;
    bool fileSelectYesNoReady = false;
    bool nameScenePresent = false;
    std::uint8_t nameSceneProcedure = 0xff;
    bool fileSelectPresent = false;
    std::uint8_t fileSelectProcedure = 0xff;
    std::uint8_t fileSelectCardCheckProcedure = 0xff;

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

/** Captures the standard boundary evidence from an observation and fingerprints it. */
std::string compute_milestone_boundary_fingerprint(
    const MilestoneObservation& observation, const TapeBoot& boot);

/**
 * Computes a process-independent fingerprint of every copied gameplay field in an observation,
 * plus the tape boot identity. Unlike a checkpoint digest, this deliberately excludes host
 * addresses and allocator bytes so equivalent cold launches can be compared. It is an observable
 * replay-equivalence key, not a substitute for same-process full-state checkpoint verification.
 */
std::string compute_milestone_observation_fingerprint(
    const MilestoneObservation& observation, const TapeBoot& boot);

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
