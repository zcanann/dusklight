//! Canonical identity for the immutable static worlds required by a run.

use crate::artifact::Digest;
use crate::world_inventory::WorldInventory;
use crate::world_spatial::WorldSpatialIndex;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const WORLD_CONTEXT_SCHEMA: &str = "dusklight-world-context/v1";
pub const MAX_WORLD_CONTEXT_STAGES: usize = 256;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldContextStage {
    pub stage: String,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldContext {
    pub schema: String,
    pub game_data_sha256: Digest,
    pub stages: Vec<WorldContextStage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldContextError(String);

impl WorldContextError {
    fn invalid(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for WorldContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for WorldContextError {}

impl From<serde_json::Error> for WorldContextError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}

impl WorldContext {
    pub fn build(
        game_data_sha256: Digest,
        inventories: &[WorldInventory],
    ) -> Result<Self, WorldContextError> {
        if game_data_sha256 == Digest::ZERO {
            return Err(WorldContextError::invalid(
                "world context game-data digest must be nonzero",
            ));
        }
        if inventories.is_empty() || inventories.len() > MAX_WORLD_CONTEXT_STAGES {
            return Err(WorldContextError::invalid(
                "world context must contain between 1 and 256 stages",
            ));
        }
        let mut stages = Vec::with_capacity(inventories.len());
        for inventory in inventories {
            inventory
                .validate()
                .map_err(|error| WorldContextError::invalid(error.to_string()))?;
            let spatial = WorldSpatialIndex::build(inventory)
                .map_err(|error| WorldContextError::invalid(error.to_string()))?;
            stages.push(WorldContextStage {
                stage: inventory.stage.clone(),
                inventory_sha256: inventory
                    .digest()
                    .map_err(|error| WorldContextError::invalid(error.to_string()))?,
                spatial_index_sha256: spatial
                    .artifact_digest()
                    .map_err(|error| WorldContextError::invalid(error.to_string()))?,
            });
        }
        stages.sort_by(|left, right| left.stage.cmp(&right.stage));
        let context = Self {
            schema: WORLD_CONTEXT_SCHEMA.into(),
            game_data_sha256,
            stages,
        };
        context.validate()?;
        Ok(context)
    }

    pub fn validate(&self) -> Result<(), WorldContextError> {
        if self.schema != WORLD_CONTEXT_SCHEMA || self.game_data_sha256 == Digest::ZERO {
            return Err(WorldContextError::invalid("invalid world context identity"));
        }
        if self.stages.is_empty() || self.stages.len() > MAX_WORLD_CONTEXT_STAGES {
            return Err(WorldContextError::invalid(
                "world context must contain between 1 and 256 stages",
            ));
        }
        let mut names = BTreeSet::new();
        let mut previous = None;
        for stage in &self.stages {
            if stage.stage.is_empty()
                || stage.stage.len() > 8
                || !stage
                    .stage
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || stage.inventory_sha256 == Digest::ZERO
                || stage.spatial_index_sha256 == Digest::ZERO
                || !names.insert(stage.stage.as_str())
                || previous.is_some_and(|prior: &str| prior >= stage.stage.as_str())
            {
                return Err(WorldContextError::invalid(
                    "world context stages are invalid, duplicate, or noncanonical",
                ));
            }
            previous = Some(stage.stage.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorldContextError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, WorldContextError> {
        let context: Self = serde_json::from_slice(bytes)?;
        context.validate()?;
        if context.canonical_bytes()? != bytes {
            return Err(WorldContextError::invalid(
                "world context is not canonical JSON",
            ));
        }
        Ok(context)
    }

    pub fn digest(&self) -> Result<Digest, WorldContextError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_inventory::{
        SourceKind, SourceScope, WORLD_INVENTORY_SCHEMA, WorldInventory, WorldSource,
    };

    fn empty_inventory(stage: &str) -> WorldInventory {
        WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: stage.into(),
            sources: vec![WorldSource {
                scope: SourceScope {
                    kind: SourceKind::Stage,
                    room: None,
                },
                archive_sha256: Digest([1; 32]),
                stage_data_path: "stage.dzs".into(),
                stage_data_sha256: Digest([2; 32]),
                kcl_path: None,
                kcl_sha256: None,
                plc_path: None,
                plc_sha256: None,
                addressable_prisms: 0,
            }],
            chunks: Vec::new(),
            placements: Vec::new(),
            player_spawns: Vec::new(),
            exits: Vec::new(),
            collisions: Vec::new(),
            load_triggers: Vec::new(),
        }
    }

    #[test]
    fn context_sorts_and_seals_multiple_static_worlds() {
        let context = WorldContext::build(
            Digest([7; 32]),
            &[empty_inventory("F_SP104"), empty_inventory("F_SP103")],
        )
        .unwrap();
        assert_eq!(context.stages[0].stage, "F_SP103");
        assert_eq!(context.stages[1].stage, "F_SP104");
        assert_ne!(context.digest().unwrap(), Digest::ZERO);
        assert_eq!(
            WorldContext::decode_canonical(&context.canonical_bytes().unwrap()).unwrap(),
            context
        );
    }

    #[test]
    fn context_rejects_duplicates_and_alternate_json() {
        assert!(
            WorldContext::build(
                Digest([7; 32]),
                &[empty_inventory("F_SP103"), empty_inventory("F_SP103")],
            )
            .is_err()
        );
        let context = WorldContext::build(Digest([7; 32]), &[empty_inventory("F_SP103")]).unwrap();
        let pretty = serde_json::to_vec_pretty(&context).unwrap();
        assert!(WorldContext::decode_canonical(&pretty).is_err());
    }
}
