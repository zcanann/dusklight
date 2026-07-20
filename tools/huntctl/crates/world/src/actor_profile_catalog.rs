//! Canonical pointer-free metadata for the executable's process/actor profiles.

use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use xxhash_rust::xxh3::xxh3_128;

pub const ACTOR_PROFILE_CATALOG_SCHEMA: &str = "dusklight-actor-profile-catalog/v1";
pub const MAX_ACTOR_PROFILE_CATALOG_ENTRIES: usize = 65_536;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorProfileEntry {
    pub slot: u32,
    pub present: bool,
    pub layer_id: Option<u32>,
    pub list_id: Option<u16>,
    pub list_priority: Option<u16>,
    pub profile_name: Option<i16>,
    pub process_size: Option<u32>,
    pub auxiliary_size: Option<u32>,
    pub parameters: Option<u32>,
    pub is_leaf: Option<bool>,
    pub draw_priority: Option<i16>,
    pub is_actor: Option<bool>,
    pub status: Option<u32>,
    pub group: Option<u8>,
    pub cull_type: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorProfileCatalog {
    pub schema: String,
    pub identity: String,
    pub profiles: Vec<ActorProfileEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorProfileCatalogError(String);

impl ActorProfileCatalogError {
    fn invalid(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ActorProfileCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ActorProfileCatalogError {}

impl From<std::io::Error> for ActorProfileCatalogError {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}

impl From<serde_json::Error> for ActorProfileCatalogError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}

impl ActorProfileCatalog {
    pub fn read_canonical(path: &Path) -> Result<Self, ActorProfileCatalogError> {
        Self::decode_canonical(&fs::read(path)?)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, ActorProfileCatalogError> {
        let catalog: Self = serde_json::from_slice(bytes)?;
        catalog.validate()?;
        if catalog.canonical_bytes()? != bytes {
            return Err(ActorProfileCatalogError::invalid(
                "actor profile catalog is not canonical JSON",
            ));
        }
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), ActorProfileCatalogError> {
        if self.schema != ACTOR_PROFILE_CATALOG_SCHEMA
            || self.profiles.is_empty()
            || self.profiles.len() > MAX_ACTOR_PROFILE_CATALOG_ENTRIES
        {
            return Err(ActorProfileCatalogError::invalid(
                "invalid actor profile catalog envelope",
            ));
        }
        for (index, profile) in self.profiles.iter().enumerate() {
            if profile.slot as usize != index {
                return Err(ActorProfileCatalogError::invalid(
                    "actor profile slots must be complete and ordered",
                ));
            }
            let base_present = profile.layer_id.is_some()
                && profile.list_id.is_some()
                && profile.list_priority.is_some()
                && profile.profile_name.is_some()
                && profile.process_size.is_some()
                && profile.auxiliary_size.is_some()
                && profile.parameters.is_some()
                && profile.is_leaf.is_some()
                && profile.is_actor.is_some();
            let base_absent = profile.layer_id.is_none()
                && profile.list_id.is_none()
                && profile.list_priority.is_none()
                && profile.profile_name.is_none()
                && profile.process_size.is_none()
                && profile.auxiliary_size.is_none()
                && profile.parameters.is_none()
                && profile.is_leaf.is_none()
                && profile.is_actor.is_none();
            if (profile.present && !base_present)
                || (!profile.present && !base_absent)
                || (profile.is_leaf == Some(true)) != profile.draw_priority.is_some()
                || (profile.is_actor == Some(true))
                    != (profile.status.is_some()
                        && profile.group.is_some()
                        && profile.cull_type.is_some())
                || (profile.is_actor == Some(true) && profile.is_leaf != Some(true))
                || (profile.is_actor != Some(true)
                    && (profile.status.is_some()
                        || profile.group.is_some()
                        || profile.cull_type.is_some()))
            {
                return Err(ActorProfileCatalogError::invalid(
                    "actor profile optional fields are inconsistent",
                ));
            }
        }
        if self.identity != self.computed_identity()? {
            return Err(ActorProfileCatalogError::invalid(
                "actor profile catalog identity does not match its contents",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ActorProfileCatalogError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn digest(&self) -> Result<Digest, ActorProfileCatalogError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn computed_identity(&self) -> Result<String, ActorProfileCatalogError> {
        if self.profiles.len() > u32::MAX as usize {
            return Err(ActorProfileCatalogError::invalid(
                "actor profile count overflowed",
            ));
        }
        let mut bytes = Vec::with_capacity(64 + self.profiles.len() * 48);
        bytes.extend_from_slice(ACTOR_PROFILE_CATALOG_SCHEMA.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&(self.profiles.len() as u32).to_le_bytes());
        for profile in &self.profiles {
            bytes.extend_from_slice(&profile.slot.to_le_bytes());
            bytes.push(u8::from(profile.present));
            if !profile.present {
                continue;
            }
            bytes.extend_from_slice(&profile.layer_id.unwrap().to_le_bytes());
            bytes.extend_from_slice(&profile.list_id.unwrap().to_le_bytes());
            bytes.extend_from_slice(&profile.list_priority.unwrap().to_le_bytes());
            bytes.extend_from_slice(&profile.profile_name.unwrap().to_le_bytes());
            bytes.extend_from_slice(&profile.process_size.unwrap().to_le_bytes());
            bytes.extend_from_slice(&profile.auxiliary_size.unwrap().to_le_bytes());
            bytes.extend_from_slice(&profile.parameters.unwrap().to_le_bytes());
            bytes.push(u8::from(profile.is_leaf.unwrap()));
            bytes.extend_from_slice(&profile.draw_priority.unwrap_or(0).to_le_bytes());
            bytes.push(u8::from(profile.is_actor.unwrap()));
            if profile.is_actor == Some(true) {
                bytes.extend_from_slice(&profile.status.unwrap().to_le_bytes());
                bytes.push(profile.group.unwrap());
                bytes.push(profile.cull_type.unwrap());
            }
        }
        Ok(format!(
            "actor-profile-catalog:xxh3-128:{:032x}",
            xxh3_128(&bytes)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> ActorProfileCatalog {
        let mut catalog = ActorProfileCatalog {
            schema: ACTOR_PROFILE_CATALOG_SCHEMA.into(),
            identity: String::new(),
            profiles: vec![ActorProfileEntry {
                slot: 0,
                present: true,
                layer_id: Some(u32::MAX - 1),
                list_id: Some(7),
                list_priority: Some(3),
                profile_name: Some(0),
                process_size: Some(512),
                auxiliary_size: Some(0),
                parameters: Some(9),
                is_leaf: Some(true),
                draw_priority: Some(11),
                is_actor: Some(true),
                status: Some(0x1234),
                group: Some(2),
                cull_type: Some(4),
            }],
        };
        catalog.identity = catalog.computed_identity().unwrap();
        catalog
    }

    #[test]
    fn canonical_catalog_round_trips_and_seals_every_scalar() {
        let catalog = catalog();
        let bytes = catalog.canonical_bytes().unwrap();
        assert_eq!(
            ActorProfileCatalog::decode_canonical(&bytes).unwrap(),
            catalog
        );
        assert_ne!(catalog.digest().unwrap(), Digest::ZERO);
        let mut tampered = catalog.clone();
        tampered.profiles[0].group = Some(4);
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn catalog_rejects_sparse_slots_and_inconsistent_optional_fields() {
        let mut sparse = catalog();
        sparse.profiles[0].slot = 1;
        assert!(sparse.validate().is_err());
        let mut inconsistent = catalog();
        inconsistent.profiles[0].draw_priority = None;
        assert!(inconsistent.validate().is_err());
    }

    #[test]
    fn catalog_accepts_zero_sized_processes_and_non_actor_leaves() {
        let mut catalog = catalog();
        catalog.profiles[0].process_size = Some(0);
        catalog.profiles[0].is_actor = Some(false);
        catalog.profiles[0].status = None;
        catalog.profiles[0].group = None;
        catalog.profiles[0].cull_type = None;
        catalog.identity = catalog.computed_identity().unwrap();
        catalog.validate().unwrap();
    }
}
