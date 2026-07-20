//! Learner-facing mechanism index derived from revision-bound Skybook metadata.
//!
//! This module intentionally never examines page bodies. It classifies the
//! corpus so observation, action, temporal-memory, and oracle requirements can
//! be audited without copying published setups into an agent or reward model.

use crate::manifest::SkybookManifest;
use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const SKYBOOK_REQUIREMENTS_INDEX_SCHEMA: &str = "dusklight-skybook-requirements-index/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkybookMechanism {
    PrecisionMovementCollision,
    ActorDisplacementInteraction,
    TargetingCamera,
    ItemAnimationConcurrency,
    EventTriggerLoading,
    RngTimers,
    ActorLifecycleMemory,
    OtherOrUnclassified,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookRequirementsEntry {
    pub slug: String,
    pub source_path: String,
    pub source_sha256: Digest,
    pub mechanisms: Vec<SkybookMechanism>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookMechanismCount {
    pub mechanism: SkybookMechanism,
    pub pages: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookRequirementsIndex {
    pub schema: String,
    pub content_sha256: Digest,
    pub source_manifest_content_sha256: Digest,
    pub source_repository_url: String,
    pub source_git_revision: String,
    pub classification_inputs: Vec<String>,
    pub mechanism_counts: Vec<SkybookMechanismCount>,
    pub entries: Vec<SkybookRequirementsEntry>,
}

#[derive(Debug)]
pub struct SkybookRequirementsError(String);

impl fmt::Display for SkybookRequirementsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SkybookRequirementsError {}

impl SkybookRequirementsIndex {
    pub fn build(manifest: &SkybookManifest) -> Result<Self, SkybookRequirementsError> {
        manifest
            .validate()
            .map_err(|error| requirements_error(format!("invalid Skybook manifest: {error}")))?;
        let entries = manifest
            .pages
            .iter()
            .map(|page| SkybookRequirementsEntry {
                slug: page.slug.clone(),
                source_path: page.source_path.clone(),
                source_sha256: page.source_sha256,
                mechanisms: classify_metadata(&page.title, &page.description, &page.canonical_tags),
            })
            .collect::<Vec<_>>();
        let mut index = Self {
            schema: SKYBOOK_REQUIREMENTS_INDEX_SCHEMA.into(),
            content_sha256: Digest::ZERO,
            source_manifest_content_sha256: manifest.content_sha256,
            source_repository_url: manifest.source.repository_url.clone(),
            source_git_revision: manifest.source.git_revision.clone(),
            classification_inputs: vec![
                "canonical_tags".into(),
                "title".into(),
                "description".into(),
            ],
            mechanism_counts: mechanism_counts(&entries),
            entries,
        };
        index.content_sha256 = index.compute_content_sha256()?;
        index.validate_against(manifest)?;
        Ok(index)
    }

    pub fn validate_against(
        &self,
        manifest: &SkybookManifest,
    ) -> Result<(), SkybookRequirementsError> {
        manifest
            .validate()
            .map_err(|error| requirements_error(format!("invalid Skybook manifest: {error}")))?;
        if self.schema != SKYBOOK_REQUIREMENTS_INDEX_SCHEMA
            || self.source_manifest_content_sha256 != manifest.content_sha256
            || self.source_repository_url != manifest.source.repository_url
            || self.source_git_revision != manifest.source.git_revision
            || self.classification_inputs != ["canonical_tags", "title", "description"]
        {
            return Err(requirements_error(
                "requirements index is detached from its Skybook manifest or classifier contract",
            ));
        }
        if self.entries.len() != manifest.pages.len()
            || !self
                .entries
                .windows(2)
                .all(|pair| pair[0].source_path < pair[1].source_path)
        {
            return Err(requirements_error(
                "requirements entries must cover the manifest in canonical source order",
            ));
        }
        for (entry, page) in self.entries.iter().zip(&manifest.pages) {
            if entry.slug != page.slug
                || entry.source_path != page.source_path
                || entry.source_sha256 != page.source_sha256
                || entry.mechanisms
                    != classify_metadata(&page.title, &page.description, &page.canonical_tags)
                || entry.mechanisms.is_empty()
                || !entry.mechanisms.windows(2).all(|pair| pair[0] < pair[1])
            {
                return Err(requirements_error(format!(
                    "requirements classification is invalid for Skybook page {}",
                    page.slug
                )));
            }
        }
        if self.mechanism_counts != mechanism_counts(&self.entries)
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(requirements_error(
                "requirements index summary or content identity is invalid",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, SkybookRequirementsError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|error| {
            requirements_error(format!("cannot encode requirements index: {error}"))
        })?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn compute_content_sha256(&self) -> Result<Digest, SkybookRequirementsError> {
        let encoded = serde_json::to_vec(&(
            &self.schema,
            self.source_manifest_content_sha256,
            &self.source_repository_url,
            &self.source_git_revision,
            &self.classification_inputs,
            &self.mechanism_counts,
            &self.entries,
        ))
        .map_err(|error| {
            requirements_error(format!("cannot encode requirements index: {error}"))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.skybook-requirements-index/v1\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn classify_metadata(
    title: &str,
    description: &str,
    canonical_tags: &[String],
) -> Vec<SkybookMechanism> {
    // Location, platform, and region tags identify applicability; they are not
    // evidence of a loading/event or hardware mechanism by themselves.
    let semantic_tags = canonical_tags
        .iter()
        .filter(|tag| {
            !tag.starts_with("map-") && !tag.starts_with("platform-") && !tag.starts_with("region-")
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut words = format!("{} {} {}", title, description, semantic_tags.join(" "));
    words.make_ascii_lowercase();
    let groups = [
        (
            SkybookMechanism::PrecisionMovementCollision,
            [
                "movement",
                "collision",
                "clip",
                "oob",
                "out of bounds",
                "wall",
                "floor",
                "ceiling",
                "seam",
                "ledge",
                "crawl",
                "roll",
                "jump",
                "speed",
                "position",
            ]
            .as_slice(),
        ),
        (
            SkybookMechanism::ActorDisplacementInteraction,
            [
                "actor",
                "enemy",
                "npc",
                "boss",
                "epona",
                "horse",
                "goron",
                "bokoblin",
                "push",
                "displacement",
                "hitbox",
                "damage",
                "combat",
            ]
            .as_slice(),
        ),
        (
            SkybookMechanism::TargetingCamera,
            ["target", "camera", "lock-on", "lock on", "z-target", "aim"].as_slice(),
        ),
        (
            SkybookMechanism::ItemAnimationConcurrency,
            [
                "item",
                "animation",
                "bomb",
                "arrow",
                "boomerang",
                "clawshot",
                "sword",
                "shield",
                "bottle",
                "fishing",
                "rupee",
                "pickup",
            ]
            .as_slice(),
        ),
        (
            SkybookMechanism::EventTriggerLoading,
            [
                "event",
                "trigger",
                "cutscene",
                "loading",
                "load",
                "warp",
                "door",
                "transition",
            ]
            .as_slice(),
        ),
        (
            SkybookMechanism::RngTimers,
            [
                "rng", "random", "timer", "timing", "frame", "cycle", "delay",
            ]
            .as_slice(),
        ),
        (
            SkybookMechanism::ActorLifecycleMemory,
            [
                "memory",
                "heap",
                "slot",
                "allocation",
                "corruption",
                "crash",
                "softlock",
                "spawn",
                "unload",
                "delete",
                "lifecycle",
            ]
            .as_slice(),
        ),
    ];
    let mut mechanisms = BTreeSet::new();
    for (mechanism, needles) in groups {
        if needles.iter().any(|needle| words.contains(needle)) {
            mechanisms.insert(mechanism);
        }
    }
    if mechanisms.is_empty() {
        mechanisms.insert(SkybookMechanism::OtherOrUnclassified);
    }
    mechanisms.into_iter().collect()
}

fn mechanism_counts(entries: &[SkybookRequirementsEntry]) -> Vec<SkybookMechanismCount> {
    let mut counts = BTreeMap::new();
    for entry in entries {
        for mechanism in &entry.mechanisms {
            *counts.entry(*mechanism).or_insert(0usize) += 1;
        }
    }
    counts
        .into_iter()
        .map(|(mechanism, pages)| SkybookMechanismCount { mechanism, pages })
        .collect()
}

fn requirements_error(message: impl Into<String>) -> SkybookRequirementsError {
    SkybookRequirementsError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn manifest() -> SkybookManifest {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "skybook-requirements-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("_posts")).unwrap();
        fs::write(
            root.join("_posts/alpha.md"),
            "---\ntitle: Wall Clip\ndescription: Uses an enemy and a loading boundary\ncategories: [Glitches]\ntags: [mechanic-movement, mechanic-collision]\n---\nA published setup that must not be indexed.\n",
        )
        .unwrap();
        fs::write(
            root.join("_posts/beta.md"),
            "---\ntitle: Miscellaneous finding\ndescription: No classified metadata\ncategories: [Glitches]\ntags: [platform-gcn]\n---\nThe body mentions a bomb, but bodies are excluded.\n",
        )
        .unwrap();
        let manifest = SkybookManifest::import_directory(
            &root,
            "https://github.com/example/skybook",
            &"ab".repeat(20),
        )
        .unwrap();
        fs::remove_dir_all(root).unwrap();
        manifest
    }

    #[test]
    fn indexes_every_page_without_reading_setup_bodies() {
        let manifest = manifest();
        let index = SkybookRequirementsIndex::build(&manifest).unwrap();
        index.validate_against(&manifest).unwrap();
        assert_eq!(index.entries.len(), 2);
        assert_eq!(
            index.entries[1].mechanisms,
            vec![SkybookMechanism::OtherOrUnclassified]
        );
        assert!(
            index.entries[0]
                .mechanisms
                .contains(&SkybookMechanism::PrecisionMovementCollision)
        );
        assert!(
            index.entries[0]
                .mechanisms
                .contains(&SkybookMechanism::ActorDisplacementInteraction)
        );
        assert!(
            index.entries[0]
                .mechanisms
                .contains(&SkybookMechanism::EventTriggerLoading)
        );
    }

    #[test]
    fn rejects_reclassification_or_reordering() {
        let manifest = manifest();
        let mut index = SkybookRequirementsIndex::build(&manifest).unwrap();
        index.entries.reverse();
        assert!(index.validate_against(&manifest).is_err());
    }
}
