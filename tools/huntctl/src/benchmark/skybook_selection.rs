//! Human-authored, revision-bound selection of a small Skybook benchmark pilot.

use super::skybook::SkybookManifest;
use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const SKYBOOK_SELECTION_SCHEMA: &str = "dusklight-skybook-selection/v1";
pub const MIN_SELECTED_PAGES: usize = 3;
pub const MAX_SELECTED_PAGES: usize = 5;
const MAX_SELECTION_ENTRIES: usize = 64;
const MAX_RATIONALE_BYTES: usize = 2_048;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SkybookSelectionDisposition {
    #[serde(rename = "selected")]
    Selected,
    #[serde(rename = "deferred")]
    Deferred,
    #[serde(rename = "won't-do")]
    WontDo,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookSelectionEntry {
    pub slug: String,
    pub source_path: String,
    pub source_sha256: Digest,
    pub disposition: SkybookSelectionDisposition,
    pub rationale: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookSelection {
    pub schema: String,
    pub content_sha256: Digest,
    pub source_manifest_content_sha256: Digest,
    pub source_repository_url: String,
    pub source_git_revision: String,
    pub approved_by: String,
    pub approval_reference: String,
    pub entries: Vec<SkybookSelectionEntry>,
}

#[derive(Debug)]
pub struct SkybookSelectionError(String);

impl fmt::Display for SkybookSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SkybookSelectionError {}

impl SkybookSelection {
    pub fn validate_against(
        &self,
        manifest: &SkybookManifest,
    ) -> Result<(), SkybookSelectionError> {
        manifest
            .validate()
            .map_err(|error| selection_error(format!("invalid Skybook manifest: {error}")))?;
        if self.schema != SKYBOOK_SELECTION_SCHEMA
            || self.source_manifest_content_sha256 != manifest.content_sha256
            || self.source_repository_url != manifest.source.repository_url
            || self.source_git_revision != manifest.source.git_revision
        {
            return Err(selection_error(
                "selection is detached from its Skybook manifest",
            ));
        }
        if self.approved_by.trim().is_empty()
            || self.approval_reference.trim().is_empty()
            || self.approved_by != self.approved_by.trim()
            || self.approval_reference != self.approval_reference.trim()
        {
            return Err(selection_error(
                "selection requires canonical human approval identity and reference",
            ));
        }
        if self.entries.is_empty()
            || self.entries.len() > MAX_SELECTION_ENTRIES
            || !self
                .entries
                .windows(2)
                .all(|pair| pair[0].slug < pair[1].slug)
        {
            return Err(selection_error(
                "selection entries must be nonempty, bounded, unique, and slug-sorted",
            ));
        }
        let selected = self
            .entries
            .iter()
            .filter(|entry| entry.disposition == SkybookSelectionDisposition::Selected)
            .count();
        if !(MIN_SELECTED_PAGES..=MAX_SELECTED_PAGES).contains(&selected) {
            return Err(selection_error(
                "selection must contain three to five selected pages",
            ));
        }
        let pages = manifest
            .pages
            .iter()
            .map(|page| (page.slug.as_str(), page))
            .collect::<BTreeMap<_, _>>();
        for entry in &self.entries {
            let Some(page) = pages.get(entry.slug.as_str()) else {
                return Err(selection_error(format!(
                    "selection references unknown Skybook page {}",
                    entry.slug
                )));
            };
            if entry.source_path != page.source_path || entry.source_sha256 != page.source_sha256 {
                return Err(selection_error(format!(
                    "selection identity is stale for Skybook page {}",
                    entry.slug
                )));
            }
            if entry.rationale.trim().is_empty()
                || entry.rationale != entry.rationale.trim()
                || entry.rationale.len() > MAX_RATIONALE_BYTES
            {
                return Err(selection_error(format!(
                    "selection rationale is invalid for Skybook page {}",
                    entry.slug
                )));
            }
        }
        if self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(selection_error("selection content identity is invalid"));
        }
        Ok(())
    }

    pub fn refresh_content_sha256(&mut self) -> Result<(), SkybookSelectionError> {
        self.content_sha256 = self.compute_content_sha256()?;
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, SkybookSelectionError> {
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| selection_error(format!("cannot encode selection: {error}")))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn compute_content_sha256(&self) -> Result<Digest, SkybookSelectionError> {
        let encoded = serde_json::to_vec(&(
            &self.schema,
            self.source_manifest_content_sha256,
            &self.source_repository_url,
            &self.source_git_revision,
            &self.approved_by,
            &self.approval_reference,
            &self.entries,
        ))
        .map_err(|error| selection_error(format!("cannot encode selection: {error}")))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.skybook-selection/v1\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn selection_error(message: impl Into<String>) -> SkybookSelectionError {
    SkybookSelectionError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn manifest() -> SkybookManifest {
        let root = std::env::temp_dir().join(format!(
            "skybook-selection-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("_posts")).unwrap();
        for slug in ["alpha", "beta", "delta", "gamma"] {
            fs::write(
                root.join(format!("_posts/{slug}.md")),
                format!(
                    "---\ntitle: {slug}\ndescription: Test\ncategories: [Glitches]\ntags: [mechanic-movement]\n---\nBody.\n"
                ),
            )
            .unwrap();
        }
        let manifest = SkybookManifest::import_directory(
            &root,
            "https://github.com/example/skybook",
            &"ab".repeat(20),
        )
        .unwrap();
        fs::remove_dir_all(root).unwrap();
        manifest
    }

    fn build_selection(manifest: &SkybookManifest) -> SkybookSelection {
        let mut selection = SkybookSelection {
            schema: SKYBOOK_SELECTION_SCHEMA.into(),
            content_sha256: Digest::ZERO,
            source_manifest_content_sha256: manifest.content_sha256,
            source_repository_url: manifest.source.repository_url.clone(),
            source_git_revision: manifest.source.git_revision.clone(),
            approved_by: "benchmark-owner".into(),
            approval_reference: "review:pilot-1".into(),
            entries: manifest
                .pages
                .iter()
                .map(|page| SkybookSelectionEntry {
                    slug: page.slug.clone(),
                    source_path: page.source_path.clone(),
                    source_sha256: page.source_sha256,
                    disposition: if page.slug == "delta" {
                        SkybookSelectionDisposition::Deferred
                    } else {
                        SkybookSelectionDisposition::Selected
                    },
                    rationale: format!("Pilot disposition for {}.", page.slug),
                })
                .collect(),
        };
        selection.refresh_content_sha256().unwrap();
        selection
    }

    #[test]
    fn validates_three_selected_pages_and_one_deferred_page() {
        let manifest = manifest();
        let selection = build_selection(&manifest);
        selection.validate_against(&manifest).unwrap();
        assert_eq!(
            selection
                .entries
                .iter()
                .filter(|entry| entry.disposition == SkybookSelectionDisposition::Selected)
                .count(),
            3
        );
    }

    #[test]
    fn rejects_too_few_selected_or_stale_page_identity() {
        let manifest = manifest();
        let mut selection = build_selection(&manifest);
        selection.entries[0].disposition = SkybookSelectionDisposition::Deferred;
        selection.refresh_content_sha256().unwrap();
        assert!(selection.validate_against(&manifest).is_err());

        let mut selection = build_selection(&manifest);
        selection.entries[0].source_sha256 = Digest([7; 32]);
        selection.refresh_content_sha256().unwrap();
        assert!(selection.validate_against(&manifest).is_err());
    }

    #[test]
    fn content_digest_detects_unapproved_edits() {
        let manifest = manifest();
        let mut selection = build_selection(&manifest);
        selection.entries[0].rationale.push_str(" Changed");
        assert!(selection.validate_against(&manifest).is_err());
    }
}
