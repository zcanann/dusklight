use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const OBSERVATION_SPEC_SCHEMA: &str = "dusklight-observation-spec/v1";
pub const MOVEMENT_STATE_V2_ID: &str = "movement-state/v2";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationSpec {
    pub schema: String,
    pub id: String,
    pub objective: ObjectiveSpec,
    pub phase: ObservationPhase,
    pub channels: Vec<ChannelRequirement>,
    pub features: Vec<FeatureSpec>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveSpec {
    pub id: String,
    pub source: LocationSelector,
    pub target: LocationSelector,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocationSelector {
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationPhase {
    PostSimulation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelRequirement {
    pub channel: String,
    pub version: u16,
    pub stride: u32,
    pub status: ChannelStatusPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelStatusPolicy {
    Present,
    PresentOrAbsent,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSpec {
    pub field_id: u32,
    pub name: String,
    pub value_type: FeatureValueType,
    pub units: FeatureUnits,
    pub coordinate_space: CoordinateSpace,
    pub transform: FeatureTransform,
    pub categorical: bool,
    pub missingness: MissingnessPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureValueType {
    Boolean,
    SignedInteger,
    UnsignedInteger,
    Float,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureUnits {
    None,
    GameUnits,
    GameUnitsPerTick,
    AngleS16,
    Ticks,
    Identifier,
    RawBits,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateSpace {
    None,
    World,
    PlayerLocal,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeatureTransform {
    Identity,
    Divide { divisor: f32 },
    AngleSin,
    AngleCos,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MissingnessPolicy {
    Required,
    MaskedBy { field_id: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationSpecError(String);

impl fmt::Display for ObservationSpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ObservationSpecError {}

impl ObservationSpec {
    pub fn validate(&self) -> Result<(), ObservationSpecError> {
        if self.schema != OBSERVATION_SPEC_SCHEMA {
            return Err(ObservationSpecError(format!(
                "expected observation schema {OBSERVATION_SPEC_SCHEMA:?}, received {:?}",
                self.schema
            )));
        }
        if self.id.is_empty() || self.objective.id.is_empty() {
            return Err(ObservationSpecError(
                "observation and objective IDs must be nonempty".into(),
            ));
        }
        for selector in [&self.objective.source, &self.objective.target] {
            if selector.stage.is_empty() || selector.stage.len() > 8 {
                return Err(ObservationSpecError(
                    "objective stage selectors must contain 1..=8 bytes".into(),
                ));
            }
        }
        if self.channels.is_empty() || self.features.is_empty() {
            return Err(ObservationSpecError(
                "observation spec must contain channels and features".into(),
            ));
        }
        let mut channel_names = BTreeSet::new();
        for channel in &self.channels {
            if channel.channel.is_empty()
                || channel.version == 0
                || channel.stride == 0
                || !channel_names.insert(channel.channel.as_str())
            {
                return Err(ObservationSpecError(
                    "channel requirements must have unique names and nonzero exact formats".into(),
                ));
            }
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut prior_id = 0;
        for feature in &self.features {
            if feature.field_id == 0
                || feature.field_id <= prior_id
                || feature.name.is_empty()
                || !ids.insert(feature.field_id)
                || !names.insert(feature.name.as_str())
            {
                return Err(ObservationSpecError(
                    "features must have unique names and strictly increasing nonzero field IDs"
                        .into(),
                ));
            }
            if let FeatureTransform::Divide { divisor } = feature.transform
                && (!divisor.is_finite() || divisor == 0.0)
            {
                return Err(ObservationSpecError(
                    "feature divisors must be finite and nonzero".into(),
                ));
            }
            prior_id = feature.field_id;
        }
        for feature in &self.features {
            if let MissingnessPolicy::MaskedBy { field_id } = feature.missingness {
                let Some(mask) = self
                    .features
                    .iter()
                    .find(|candidate| candidate.field_id == field_id)
                else {
                    return Err(ObservationSpecError(format!(
                        "feature {} references missing mask field {field_id}",
                        feature.name
                    )));
                };
                if mask.value_type != FeatureValueType::Boolean
                    || mask.missingness != MissingnessPolicy::Required
                {
                    return Err(ObservationSpecError(format!(
                        "feature {} mask must be a required boolean",
                        feature.name
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ObservationSpecError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| ObservationSpecError(error.to_string()))
    }

    pub fn digest(&self) -> Result<Digest, ObservationSpecError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn feature_count(&self) -> u32 {
        self.features.len() as u32
    }

    pub fn categorical_features(&self) -> Vec<usize> {
        self.features
            .iter()
            .enumerate()
            .filter_map(|(index, feature)| feature.categorical.then_some(index))
            .collect()
    }
}

fn channel(
    channel: &str,
    version: u16,
    stride: u32,
    status: ChannelStatusPolicy,
) -> ChannelRequirement {
    ChannelRequirement {
        channel: channel.into(),
        version,
        stride,
        status,
    }
}

pub fn movement_state_v2_spec() -> ObservationSpec {
    use ChannelStatusPolicy::{Present, PresentOrAbsent};
    use CoordinateSpace::{None as NoSpace, PlayerLocal, World};
    use FeatureTransform::{AngleCos, AngleSin, Divide, Identity};
    use FeatureUnits::{
        AngleS16, GameUnits, GameUnitsPerTick, Identifier, None as NoUnits, RawBits, Ticks,
    };
    use FeatureValueType::{Boolean, Float, SignedInteger, UnsignedInteger};
    use MissingnessPolicy::{MaskedBy, Required};

    let definitions = [
        (
            "stage.byte0",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "stage.byte1",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "stage.byte2",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "stage.byte3",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "stage.byte4",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "stage.byte5",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "stage.byte6",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "stage.byte7",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "stage.room",
            SignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "stage.layer",
            SignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "stage.point",
            SignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "objective.target_stage",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "objective.target_location",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "stage.next_enabled",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "objective.next_target",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "player.present",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "player.is_link",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "player.procedure_present",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "player.procedure",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            MaskedBy { field_id: 18 },
        ),
        (
            "player.position_x",
            Float,
            GameUnits,
            World,
            Divide { divisor: 8192.0 },
            false,
            Required,
        ),
        (
            "player.position_y",
            Float,
            GameUnits,
            World,
            Divide { divisor: 8192.0 },
            false,
            Required,
        ),
        (
            "player.position_z",
            Float,
            GameUnits,
            World,
            Divide { divisor: 8192.0 },
            false,
            Required,
        ),
        (
            "player.velocity_x",
            Float,
            GameUnitsPerTick,
            World,
            Divide { divisor: 64.0 },
            false,
            Required,
        ),
        (
            "player.velocity_y",
            Float,
            GameUnitsPerTick,
            World,
            Divide { divisor: 64.0 },
            false,
            Required,
        ),
        (
            "player.velocity_z",
            Float,
            GameUnitsPerTick,
            World,
            Divide { divisor: 64.0 },
            false,
            Required,
        ),
        (
            "player.forward_speed",
            Float,
            GameUnitsPerTick,
            PlayerLocal,
            Divide { divisor: 64.0 },
            false,
            Required,
        ),
        (
            "player.current_yaw_sin",
            Float,
            AngleS16,
            World,
            AngleSin,
            false,
            Required,
        ),
        (
            "player.current_yaw_cos",
            Float,
            AngleS16,
            World,
            AngleCos,
            false,
            Required,
        ),
        (
            "player.shape_yaw_sin",
            Float,
            AngleS16,
            World,
            AngleSin,
            false,
            Required,
        ),
        (
            "player.shape_yaw_cos",
            Float,
            AngleS16,
            World,
            AngleCos,
            false,
            Required,
        ),
        (
            "player.yaw_delta_sin",
            Float,
            AngleS16,
            PlayerLocal,
            AngleSin,
            false,
            Required,
        ),
        (
            "player.yaw_delta_cos",
            Float,
            AngleS16,
            PlayerLocal,
            AngleCos,
            false,
            Required,
        ),
        (
            "input.buttons_lo",
            UnsignedInteger,
            RawBits,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "input.buttons_hi",
            UnsignedInteger,
            RawBits,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "input.stick_x",
            SignedInteger,
            NoUnits,
            PlayerLocal,
            Divide { divisor: 127.0 },
            false,
            Required,
        ),
        (
            "input.stick_y",
            SignedInteger,
            NoUnits,
            PlayerLocal,
            Divide { divisor: 127.0 },
            false,
            Required,
        ),
        (
            "input.pad_error",
            SignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "event.running",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "event.name_hash_present",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "event.name_hash_lo",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 65535.0 },
            true,
            MaskedBy { field_id: 39 },
        ),
        (
            "event.name_hash_hi",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 65535.0 },
            true,
            MaskedBy { field_id: 39 },
        ),
        (
            "event.id",
            SignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "event.mode",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "event.status",
            UnsignedInteger,
            RawBits,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "event.map_tool",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Divide { divisor: 255.0 },
            true,
            Required,
        ),
        (
            "scene_exit.present",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "scene_exit.target_match",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            MaskedBy { field_id: 46 },
        ),
        (
            "scene_exit.signed_distance",
            Float,
            GameUnits,
            PlayerLocal,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 46 },
        ),
        (
            "scene_exit.local_x",
            Float,
            GameUnits,
            PlayerLocal,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 46 },
        ),
        (
            "scene_exit.local_y",
            Float,
            GameUnits,
            PlayerLocal,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 46 },
        ),
        (
            "scene_exit.local_z",
            Float,
            GameUnits,
            PlayerLocal,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 46 },
        ),
        (
            "scene_exit.extent_x",
            Float,
            GameUnits,
            PlayerLocal,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 46 },
        ),
        (
            "scene_exit.extent_y",
            Float,
            GameUnits,
            PlayerLocal,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 46 },
        ),
        (
            "scene_exit.extent_z",
            Float,
            GameUnits,
            PlayerLocal,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 46 },
        ),
        (
            "collision.ground_contact",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "collision.ground_height",
            Float,
            GameUnits,
            World,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 55 },
        ),
        (
            "collision.ground_plane_valid",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "collision.ground_normal_x",
            Float,
            NoUnits,
            World,
            Identity,
            false,
            MaskedBy { field_id: 57 },
        ),
        (
            "collision.ground_normal_y",
            Float,
            NoUnits,
            World,
            Identity,
            false,
            MaskedBy { field_id: 57 },
        ),
        (
            "collision.ground_normal_z",
            Float,
            NoUnits,
            World,
            Identity,
            false,
            MaskedBy { field_id: 57 },
        ),
        (
            "collision.ground_plane_d",
            Float,
            GameUnits,
            World,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 57 },
        ),
        (
            "collision.trajectory_valid",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "collision.displacement_x",
            Float,
            GameUnits,
            World,
            Divide { divisor: 64.0 },
            false,
            MaskedBy { field_id: 62 },
        ),
        (
            "collision.displacement_y",
            Float,
            GameUnits,
            World,
            Divide { divisor: 64.0 },
            false,
            MaskedBy { field_id: 62 },
        ),
        (
            "collision.displacement_z",
            Float,
            GameUnits,
            World,
            Divide { divisor: 64.0 },
            false,
            MaskedBy { field_id: 62 },
        ),
        (
            "surface.ground_identity",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "surface.ground_backing",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "surface.ground_destination",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "surface.ground_target_match",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            MaskedBy { field_id: 68 },
        ),
        (
            "surface.ground_bg",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            MaskedBy { field_id: 66 },
        ),
        (
            "surface.ground_polygon",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            MaskedBy { field_id: 66 },
        ),
        (
            "surface.ground_material",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            MaskedBy { field_id: 67 },
        ),
        (
            "surface.ground_exit",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            MaskedBy { field_id: 67 },
        ),
        (
            "surface.kcl_height_present",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "surface.kcl_height",
            Float,
            GameUnits,
            NoSpace,
            Divide { divisor: 8192.0 },
            false,
            MaskedBy { field_id: 74 },
        ),
        (
            "surface.link_exit_present",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "surface.link_exit",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            MaskedBy { field_id: 76 },
        ),
        (
            "surface.pending_match",
            Boolean,
            NoUnits,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "rng.primary_calls",
            UnsignedInteger,
            Ticks,
            NoSpace,
            Divide { divisor: 1048576.0 },
            false,
            Required,
        ),
        (
            "rng.primary_state0",
            SignedInteger,
            RawBits,
            NoSpace,
            Divide {
                divisor: 2147483648.0,
            },
            false,
            Required,
        ),
        (
            "rng.primary_state1",
            SignedInteger,
            RawBits,
            NoSpace,
            Divide {
                divisor: 2147483648.0,
            },
            false,
            Required,
        ),
        (
            "rng.primary_state2",
            SignedInteger,
            RawBits,
            NoSpace,
            Divide {
                divisor: 2147483648.0,
            },
            false,
            Required,
        ),
        (
            "rng.secondary_calls",
            UnsignedInteger,
            Ticks,
            NoSpace,
            Divide { divisor: 1048576.0 },
            false,
            Required,
        ),
        (
            "rng.secondary_state0",
            SignedInteger,
            RawBits,
            NoSpace,
            Divide {
                divisor: 2147483648.0,
            },
            false,
            Required,
        ),
        (
            "rng.secondary_state1",
            SignedInteger,
            RawBits,
            NoSpace,
            Divide {
                divisor: 2147483648.0,
            },
            false,
            Required,
        ),
        (
            "rng.secondary_state2",
            SignedInteger,
            RawBits,
            NoSpace,
            Divide {
                divisor: 2147483648.0,
            },
            false,
            Required,
        ),
        (
            "camera.view_yaw_sin",
            Float,
            AngleS16,
            World,
            AngleSin,
            false,
            Required,
        ),
        (
            "camera.view_yaw_cos",
            Float,
            AngleS16,
            World,
            AngleCos,
            false,
            Required,
        ),
        (
            "camera.eye_x",
            Float,
            GameUnits,
            World,
            Divide { divisor: 8192.0 },
            false,
            Required,
        ),
        (
            "camera.eye_y",
            Float,
            GameUnits,
            World,
            Divide { divisor: 8192.0 },
            false,
            Required,
        ),
        (
            "camera.eye_z",
            Float,
            GameUnits,
            World,
            Divide { divisor: 8192.0 },
            false,
            Required,
        ),
        (
            "action.procedure",
            UnsignedInteger,
            Identifier,
            NoSpace,
            Identity,
            true,
            Required,
        ),
        (
            "action.mode_flags",
            UnsignedInteger,
            RawBits,
            NoSpace,
            Divide {
                divisor: 4294967295.0,
            },
            false,
            Required,
        ),
        (
            "action.damage_wait",
            SignedInteger,
            Ticks,
            NoSpace,
            Identity,
            false,
            Required,
        ),
        (
            "action.sword_at_up",
            UnsignedInteger,
            Ticks,
            NoSpace,
            Identity,
            false,
            Required,
        ),
        (
            "action.ice_damage_wait",
            SignedInteger,
            Ticks,
            NoSpace,
            Identity,
            false,
            Required,
        ),
        (
            "window.elapsed",
            UnsignedInteger,
            Ticks,
            NoSpace,
            Divide { divisor: 1024.0 },
            false,
            Required,
        ),
        (
            "window.remaining",
            UnsignedInteger,
            Ticks,
            NoSpace,
            Divide { divisor: 1024.0 },
            false,
            Required,
        ),
    ];

    let features = definitions
        .into_iter()
        .enumerate()
        .map(
            |(
                index,
                (name, value_type, units, coordinate_space, transform, categorical, missingness),
            )| {
                FeatureSpec {
                    field_id: index as u32 + 1,
                    name: name.into(),
                    value_type,
                    units,
                    coordinate_space,
                    transform,
                    categorical,
                    missingness,
                }
            },
        )
        .collect();

    ObservationSpec {
        schema: OBSERVATION_SPEC_SCHEMA.into(),
        id: MOVEMENT_STATE_V2_ID.into(),
        objective: ObjectiveSpec {
            id: "intro.f_sp103_to_f_sp104.load-zone/v1".into(),
            source: LocationSelector {
                stage: "F_SP103".into(),
                room: 1,
                layer: 3,
                point: 1,
            },
            target: LocationSelector {
                stage: "F_SP104".into(),
                room: 1,
                layer: -1,
                point: 0,
            },
        },
        phase: ObservationPhase::PostSimulation,
        channels: vec![
            channel("core", 1, 32, Present),
            channel("stage", 1, 32, Present),
            channel("applied_pads", 1, 52, Present),
            channel("player_motion", 1, 52, Present),
            channel("event", 1, 16, Present),
            channel("scene_exit", 2, 88, PresentOrAbsent),
            channel("rng", 1, 64, Present),
            channel("camera", 1, 48, Present),
            channel("player_action", 3, 160, Present),
            channel("player_background_collision", 1, 128, Present),
            channel("player_collision_surfaces", 1, 496, Present),
        ],
        features,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_spec_is_valid_and_canonical() {
        let spec = movement_state_v2_spec();
        spec.validate().unwrap();
        assert_eq!(spec.feature_count(), 98);
        assert_eq!(
            spec.canonical_bytes().unwrap(),
            spec.canonical_bytes().unwrap()
        );
        assert_eq!(
            spec.digest().unwrap().to_string(),
            "4bc4cc410fbe8747bca7564661759239884a49ee6658dd1e7d332f6302069218"
        );
        assert_eq!(spec.features.first().unwrap().name, "stage.byte0");
        assert_eq!(spec.features.last().unwrap().name, "window.remaining");
    }

    #[test]
    fn objective_target_changes_authenticated_identity() {
        let first = movement_state_v2_spec();
        let mut second = first.clone();
        second.objective.target.point = 1;
        assert_ne!(first.digest().unwrap(), second.digest().unwrap());
    }

    #[test]
    fn rejects_bad_formats_duplicate_fields_and_invalid_masks() {
        let mut bad_format = movement_state_v2_spec();
        bad_format.channels[0].stride = 0;
        assert!(bad_format.validate().is_err());

        let mut duplicate = movement_state_v2_spec();
        duplicate.features[1].field_id = duplicate.features[0].field_id;
        assert!(duplicate.validate().is_err());

        let mut bad_mask = movement_state_v2_spec();
        bad_mask.features[18].missingness = MissingnessPolicy::MaskedBy { field_id: 9999 };
        assert!(bad_mask.validate().is_err());
    }
}
