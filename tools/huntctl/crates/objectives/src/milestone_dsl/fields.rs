use super::*;

impl Field {
    pub fn path(self) -> &'static str {
        match self {
            Self::BoundaryKind => "boundary.kind",
            Self::BoundaryIndex => "boundary.index",
            Self::TapeFrame => "tape.frame",
            Self::StageName => "stage.name",
            Self::StageRoom => "stage.room",
            Self::StageLayer => "stage.layer",
            Self::StageSpawn => "stage.spawn",
            Self::PlayerExists => "player.exists",
            Self::PlayerPositionX => "player.position.x",
            Self::PlayerPositionY => "player.position.y",
            Self::PlayerPositionZ => "player.position.z",
            Self::PlayerSpeed => "player.speed",
            Self::PlayerProcedure => "player.procedure",
            Self::EventRunning => "event.running",
            Self::EventId => "event.id",
            Self::NextStageName => "next_stage.name",
            Self::NextStageRoom => "next_stage.room",
            Self::NextStageLayer => "next_stage.layer",
            Self::NextStageSpawn => "next_stage.spawn",
            Self::BoundaryReached => "boundary.reached",
            Self::PlayerIsLink => "player.is_link",
            Self::NextStageEnabled => "next_stage.enabled",
            Self::PlayerProcessId => "player.process_id",
            Self::PlayerActorName => "player.actor_name",
            Self::PlayerVelocityX => "player.velocity.x",
            Self::PlayerVelocityY => "player.velocity.y",
            Self::PlayerVelocityZ => "player.velocity.z",
            Self::PlayerCurrentAngleX => "player.current_angle.x",
            Self::PlayerCurrentAngleY => "player.current_angle.y",
            Self::PlayerCurrentAngleZ => "player.current_angle.z",
            Self::PlayerShapeAngleX => "player.shape_angle.x",
            Self::PlayerShapeAngleY => "player.shape_angle.y",
            Self::PlayerShapeAngleZ => "player.shape_angle.z",
            Self::PlayerModeFlags => "player.mode_flags",
            Self::PlayerDamageWaitTimer => "player.timer.damage_wait",
            Self::PlayerIceDamageWaitTimer => "player.timer.ice_damage_wait",
            Self::PlayerSwordChangeWaitTimer => "player.timer.sword_change_wait",
            Self::EventMode => "event.mode",
            Self::EventStatus => "event.status",
            Self::EventMapToolId => "event.map_tool_id",
            Self::EventNameHashPresent => "event.name_hash.present",
            Self::EventNameHash => "event.name_hash.fnv1a32",
            Self::RngPrimaryState0 => "rng.primary.state0",
            Self::RngPrimaryState1 => "rng.primary.state1",
            Self::RngPrimaryState2 => "rng.primary.state2",
            Self::RngPrimaryCalls => "rng.primary.calls",
            Self::RngSecondaryState0 => "rng.secondary.state0",
            Self::RngSecondaryState1 => "rng.secondary.state1",
            Self::RngSecondaryState2 => "rng.secondary.state2",
            Self::RngSecondaryCalls => "rng.secondary.calls",
            Self::CollisionGroundContact => "collision.ground.contact",
            Self::CollisionWallContact => "collision.wall.contact",
            Self::CollisionRoofContact => "collision.roof.contact",
            Self::CollisionWaterContact => "collision.water.contact",
            Self::CollisionWaterIn => "collision.water.in",
            Self::CollisionGroundHeight => "collision.ground.height",
            Self::CollisionRoofHeight => "collision.roof.height",
            Self::CollisionGroundClearance => "collision.ground.clearance",
            Self::PlayerDoStatus => "player.interaction.do_status",
            Self::TalkPartnerExists => "player.interaction.talk_partner.exists",
            Self::TalkPartnerActorName => "player.interaction.talk_partner.actor_name",
            Self::TalkPartnerSetId => "player.interaction.talk_partner.set_id",
            Self::TalkPartnerHomeRoom => "player.interaction.talk_partner.home_room",
            Self::TalkPartnerCurrentRoom => "player.interaction.talk_partner.current_room",
            Self::GrabbedActorExists => "player.interaction.grabbed_actor.exists",
            Self::GrabbedActorActorName => "player.interaction.grabbed_actor.actor_name",
            Self::GrabbedActorSetId => "player.interaction.grabbed_actor.set_id",
            Self::GrabbedActorHomeRoom => "player.interaction.grabbed_actor.home_room",
            Self::GrabbedActorCurrentRoom => "player.interaction.grabbed_actor.current_room",
            Self::TalkPartnerHomePositionX => "player.interaction.talk_partner.home_position.x",
            Self::TalkPartnerHomePositionY => "player.interaction.talk_partner.home_position.y",
            Self::TalkPartnerHomePositionZ => "player.interaction.talk_partner.home_position.z",
            Self::GrabbedActorHomePositionX => "player.interaction.grabbed_actor.home_position.x",
            Self::GrabbedActorHomePositionY => "player.interaction.grabbed_actor.home_position.y",
            Self::GrabbedActorHomePositionZ => "player.interaction.grabbed_actor.home_position.z",
            Self::TitleLogoSkipReady => "menu.title.logo_skip_ready",
            Self::TitleStartReady => "menu.title.start_ready",
            Self::NameEntryActive => "menu.name_entry.active",
            Self::NameEntryCharacterSelectReady => "menu.name_entry.character_select_ready",
            Self::NameEntryInputReady => "menu.name_entry.input_ready",
            Self::NameEntrySelectionProcedure => "menu.name_entry.selection_procedure",
            Self::FileSelectNoSaveReady => "menu.file_select.no_save_ready",
            Self::FileSelectDataSelectReady => "menu.file_select.data_select_ready",
            Self::FileSelectKeyWaitReady => "menu.file_select.key_wait_ready",
            Self::FileSelectYesNoReady => "menu.file_select.yes_no_ready",
            Self::TitlePresent => "menu.title.present",
            Self::TitleProcedure => "menu.title.procedure",
            Self::NameScenePresent => "menu.name_scene.present",
            Self::NameSceneProcedure => "menu.name_scene.procedure",
            Self::FileSelectPresent => "menu.file_select.present",
            Self::FileSelectProcedure => "menu.file_select.procedure",
            Self::FileSelectCardCheckProcedure => "menu.file_select.card_check_procedure",
        }
    }

    pub(super) fn field_type(self) -> FieldType {
        match self {
            Self::BoundaryKind => FieldType::Enum,
            Self::BoundaryIndex
            | Self::TapeFrame
            | Self::RngPrimaryCalls
            | Self::RngSecondaryCalls => FieldType::U64,
            Self::PlayerProcessId
            | Self::PlayerModeFlags
            | Self::PlayerSwordChangeWaitTimer
            | Self::EventMode
            | Self::EventStatus
            | Self::EventMapToolId
            | Self::EventNameHash => FieldType::U32,
            Self::PlayerDoStatus | Self::TalkPartnerSetId | Self::GrabbedActorSetId => {
                FieldType::U32
            }
            Self::NameEntrySelectionProcedure
            | Self::TitleProcedure
            | Self::NameSceneProcedure
            | Self::FileSelectProcedure
            | Self::FileSelectCardCheckProcedure => FieldType::U32,
            Self::StageName | Self::NextStageName => FieldType::Symbol,
            Self::StageRoom
            | Self::StageLayer
            | Self::StageSpawn
            | Self::NextStageRoom
            | Self::NextStageLayer
            | Self::NextStageSpawn => FieldType::I32,
            Self::PlayerActorName
            | Self::PlayerCurrentAngleX
            | Self::PlayerCurrentAngleY
            | Self::PlayerCurrentAngleZ
            | Self::PlayerShapeAngleX
            | Self::PlayerShapeAngleY
            | Self::PlayerShapeAngleZ
            | Self::PlayerDamageWaitTimer
            | Self::PlayerIceDamageWaitTimer
            | Self::RngPrimaryState0
            | Self::RngPrimaryState1
            | Self::RngPrimaryState2
            | Self::RngSecondaryState0
            | Self::RngSecondaryState1
            | Self::RngSecondaryState2 => FieldType::I32,
            Self::TalkPartnerActorName
            | Self::TalkPartnerHomeRoom
            | Self::TalkPartnerCurrentRoom
            | Self::GrabbedActorActorName
            | Self::GrabbedActorHomeRoom
            | Self::GrabbedActorCurrentRoom => FieldType::I32,
            Self::PlayerExists
            | Self::EventRunning
            | Self::BoundaryReached
            | Self::PlayerIsLink
            | Self::NextStageEnabled => FieldType::Bool,
            Self::TalkPartnerExists | Self::GrabbedActorExists => FieldType::Bool,
            Self::TitleLogoSkipReady
            | Self::TitleStartReady
            | Self::NameEntryActive
            | Self::NameEntryCharacterSelectReady
            | Self::NameEntryInputReady
            | Self::FileSelectNoSaveReady
            | Self::FileSelectDataSelectReady
            | Self::FileSelectKeyWaitReady
            | Self::FileSelectYesNoReady
            | Self::TitlePresent
            | Self::NameScenePresent
            | Self::FileSelectPresent => FieldType::Bool,
            Self::EventNameHashPresent
            | Self::CollisionGroundContact
            | Self::CollisionWallContact
            | Self::CollisionRoofContact
            | Self::CollisionWaterContact
            | Self::CollisionWaterIn => FieldType::Bool,
            Self::PlayerPositionX
            | Self::PlayerPositionY
            | Self::PlayerPositionZ
            | Self::PlayerSpeed => FieldType::F32,
            Self::PlayerVelocityX
            | Self::PlayerVelocityY
            | Self::PlayerVelocityZ
            | Self::CollisionGroundHeight
            | Self::CollisionRoofHeight
            | Self::CollisionGroundClearance => FieldType::F32,
            Self::TalkPartnerHomePositionX
            | Self::TalkPartnerHomePositionY
            | Self::TalkPartnerHomePositionZ
            | Self::GrabbedActorHomePositionX
            | Self::GrabbedActorHomePositionY
            | Self::GrabbedActorHomePositionZ => FieldType::F32,
            Self::PlayerProcedure => FieldType::Procedure,
            Self::EventId => FieldType::I32,
        }
    }

    pub(super) fn parse(path: &str) -> Option<Self> {
        (1..=92).find_map(|id| {
            let field = Self::from_id(id)?;
            (field.path() == path).then_some(field)
        })
    }

    pub(super) fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            1 => Self::BoundaryKind,
            2 => Self::BoundaryIndex,
            3 => Self::TapeFrame,
            4 => Self::StageName,
            5 => Self::StageRoom,
            6 => Self::StageLayer,
            7 => Self::StageSpawn,
            8 => Self::PlayerExists,
            9 => Self::PlayerPositionX,
            10 => Self::PlayerPositionY,
            11 => Self::PlayerPositionZ,
            12 => Self::PlayerSpeed,
            13 => Self::PlayerProcedure,
            14 => Self::EventRunning,
            15 => Self::EventId,
            16 => Self::NextStageName,
            17 => Self::NextStageRoom,
            18 => Self::NextStageLayer,
            19 => Self::NextStageSpawn,
            20 => Self::BoundaryReached,
            21 => Self::PlayerIsLink,
            22 => Self::NextStageEnabled,
            23 => Self::PlayerProcessId,
            24 => Self::PlayerActorName,
            25 => Self::PlayerVelocityX,
            26 => Self::PlayerVelocityY,
            27 => Self::PlayerVelocityZ,
            28 => Self::PlayerCurrentAngleX,
            29 => Self::PlayerCurrentAngleY,
            30 => Self::PlayerCurrentAngleZ,
            31 => Self::PlayerShapeAngleX,
            32 => Self::PlayerShapeAngleY,
            33 => Self::PlayerShapeAngleZ,
            34 => Self::PlayerModeFlags,
            35 => Self::PlayerDamageWaitTimer,
            36 => Self::PlayerIceDamageWaitTimer,
            37 => Self::PlayerSwordChangeWaitTimer,
            38 => Self::EventMode,
            39 => Self::EventStatus,
            40 => Self::EventMapToolId,
            41 => Self::EventNameHashPresent,
            42 => Self::EventNameHash,
            43 => Self::RngPrimaryState0,
            44 => Self::RngPrimaryState1,
            45 => Self::RngPrimaryState2,
            46 => Self::RngPrimaryCalls,
            47 => Self::RngSecondaryState0,
            48 => Self::RngSecondaryState1,
            49 => Self::RngSecondaryState2,
            50 => Self::RngSecondaryCalls,
            51 => Self::CollisionGroundContact,
            52 => Self::CollisionWallContact,
            53 => Self::CollisionRoofContact,
            54 => Self::CollisionWaterContact,
            55 => Self::CollisionWaterIn,
            56 => Self::CollisionGroundHeight,
            57 => Self::CollisionRoofHeight,
            58 => Self::CollisionGroundClearance,
            59 => Self::PlayerDoStatus,
            60 => Self::TalkPartnerExists,
            61 => Self::TalkPartnerActorName,
            62 => Self::TalkPartnerSetId,
            63 => Self::TalkPartnerHomeRoom,
            64 => Self::TalkPartnerCurrentRoom,
            65 => Self::GrabbedActorExists,
            66 => Self::GrabbedActorActorName,
            67 => Self::GrabbedActorSetId,
            68 => Self::GrabbedActorHomeRoom,
            69 => Self::GrabbedActorCurrentRoom,
            70 => Self::TalkPartnerHomePositionX,
            71 => Self::TalkPartnerHomePositionY,
            72 => Self::TalkPartnerHomePositionZ,
            73 => Self::GrabbedActorHomePositionX,
            74 => Self::GrabbedActorHomePositionY,
            75 => Self::GrabbedActorHomePositionZ,
            76 => Self::TitleLogoSkipReady,
            77 => Self::TitleStartReady,
            78 => Self::NameEntryActive,
            79 => Self::NameEntryCharacterSelectReady,
            80 => Self::NameEntryInputReady,
            81 => Self::NameEntrySelectionProcedure,
            82 => Self::FileSelectNoSaveReady,
            83 => Self::FileSelectDataSelectReady,
            84 => Self::FileSelectKeyWaitReady,
            85 => Self::FileSelectYesNoReady,
            86 => Self::TitlePresent,
            87 => Self::TitleProcedure,
            88 => Self::NameScenePresent,
            89 => Self::NameSceneProcedure,
            90 => Self::FileSelectPresent,
            91 => Self::FileSelectProcedure,
            92 => Self::FileSelectCardCheckProcedure,
            _ => return None,
        })
    }
}
