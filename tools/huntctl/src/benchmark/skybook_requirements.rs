//! Deterministic capability requirements derived from a revision-pinned Skybook manifest.

use super::skybook::{SkybookManifest, SkybookPage};
use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const SKYBOOK_REQUIREMENTS_SCHEMA: &str = "dusklight-skybook-requirements/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookRequirementsManifest {
    pub schema: String,
    pub content_sha256: Digest,
    pub source_manifest_content_sha256: Digest,
    pub source_repository_url: String,
    pub source_git_revision: String,
    pub coverage: SkybookRequirementsCoverage,
    pub pages: Vec<SkybookPageRequirements>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookRequirementsCoverage {
    pub page_count: usize,
    pub scenario_requirement_count: usize,
    pub action_requirement_count: usize,
    pub observation_requirement_count: usize,
    pub oracle_requirement_count: usize,
    pub fidelity_requirement_count: usize,
    pub pages_with_known_setup_steps: usize,
    pub known_setup_step_count: usize,
    pub pages_with_scenario_unknowns: usize,
    pub pages_with_action_unknowns: usize,
    pub pages_with_observation_unknowns: usize,
    pub pages_with_oracle_unknowns: usize,
    pub pages_with_fidelity_unknowns: usize,
    pub pages_with_setup_unknowns: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookPageRequirements {
    pub slug: String,
    pub source_path: String,
    pub source_url: String,
    pub scenarios: SkybookRequirementProjection,
    pub actions: SkybookRequirementProjection,
    pub observations: SkybookRequirementProjection,
    pub oracles: SkybookRequirementProjection,
    pub fidelity: SkybookRequirementProjection,
    pub setup: SkybookSetupKnowledge,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookRequirementProjection {
    pub required: Vec<SkybookRequirement>,
    pub unknown: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookRequirement {
    pub id: String,
    pub source_tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookSetupKnowledge {
    pub known_steps: Vec<String>,
    pub unknown: Vec<String>,
}

#[derive(Debug)]
pub struct SkybookRequirementsError(String);

impl fmt::Display for SkybookRequirementsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SkybookRequirementsError {}

impl SkybookRequirementsManifest {
    pub fn derive(source: &SkybookManifest) -> Result<Self, SkybookRequirementsError> {
        source
            .validate()
            .map_err(|error| requirement_error(format!("invalid Skybook source: {error}")))?;
        let pages = source.pages.iter().map(derive_page).collect::<Vec<_>>();
        let coverage = coverage(&pages);
        let mut manifest = Self {
            schema: SKYBOOK_REQUIREMENTS_SCHEMA.into(),
            content_sha256: Digest::ZERO,
            source_manifest_content_sha256: source.content_sha256,
            source_repository_url: source.source.repository_url.clone(),
            source_git_revision: source.source.git_revision.clone(),
            coverage,
            pages,
        };
        manifest.content_sha256 = manifest.compute_content_sha256()?;
        manifest.validate_against(source)?;
        Ok(manifest)
    }

    pub fn validate_against(
        &self,
        source: &SkybookManifest,
    ) -> Result<(), SkybookRequirementsError> {
        source
            .validate()
            .map_err(|error| requirement_error(format!("invalid Skybook source: {error}")))?;
        if self.schema != SKYBOOK_REQUIREMENTS_SCHEMA
            || self.source_manifest_content_sha256 != source.content_sha256
            || self.source_repository_url != source.source.repository_url
            || self.source_git_revision != source.source.git_revision
            || self.pages.len() != source.pages.len()
        {
            return Err(requirement_error(
                "requirements manifest is not bound to its Skybook source",
            ));
        }
        for (page, source_page) in self.pages.iter().zip(&source.pages) {
            if page != &derive_page(source_page) {
                return Err(requirement_error(format!(
                    "requirements do not match Skybook page {}",
                    source_page.source_path
                )));
            }
            validate_projection(&page.scenarios)?;
            validate_projection(&page.actions)?;
            validate_projection(&page.observations)?;
            validate_projection(&page.oracles)?;
            validate_projection(&page.fidelity)?;
            if !is_sorted_unique(&page.setup.unknown) {
                return Err(requirement_error(
                    "setup unknowns are not sorted and unique",
                ));
            }
        }
        if self.coverage != coverage(&self.pages)
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(requirement_error(
                "requirements coverage or content identity is invalid",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, SkybookRequirementsError> {
        let mut encoded = serde_json::to_vec_pretty(self).map_err(|error| {
            requirement_error(format!("cannot encode Skybook requirements: {error}"))
        })?;
        encoded.push(b'\n');
        Ok(encoded)
    }

    fn compute_content_sha256(&self) -> Result<Digest, SkybookRequirementsError> {
        let encoded = serde_json::to_vec(&(
            &self.schema,
            self.source_manifest_content_sha256,
            &self.source_repository_url,
            &self.source_git_revision,
            &self.coverage,
            &self.pages,
        ))
        .map_err(|error| {
            requirement_error(format!("cannot encode Skybook requirements: {error}"))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.skybook-requirements/v1\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn derive_page(page: &SkybookPage) -> SkybookPageRequirements {
    let mechanics = page
        .canonical_tags
        .iter()
        .filter(|tag| tag.starts_with("mechanic-"))
        .cloned()
        .collect::<Vec<_>>();

    let mut scenarios = Vec::new();
    add_requirement(
        &mut scenarios,
        "scenario:page-specific-fixture",
        std::iter::empty::<String>(),
    );
    for tag in page
        .canonical_tags
        .iter()
        .filter(|tag| tag.starts_with("map-") || tag.starts_with("room-"))
    {
        add_requirement(&mut scenarios, format!("scenario:{tag}"), [tag.clone()]);
    }
    let mut scenario_unknown = vec![
        "exact-save-or-new-game-state".into(),
        "inventory-and-capacity-state".into(),
        "progression-and-event-flags".into(),
        "rng-and-actor-initial-state".into(),
    ];
    if scenarios.is_empty() {
        scenario_unknown.push("exact-map-room-and-spawn".into());
    }

    let mut actions = Vec::new();
    add_requirement(
        &mut actions,
        "action:source-described-controller-input",
        std::iter::empty::<String>(),
    );
    for tag in &mechanics {
        for id in action_ids(tag) {
            add_requirement(&mut actions, *id, [tag.clone()]);
        }
    }
    let action_unknown = if page.tags.iter().any(|tag| tag == "meta-needs-source") {
        vec![
            "source-input-sequence".into(),
            "input-timing-and-holds".into(),
        ]
    } else {
        vec!["input-timing-and-holds".into()]
    };

    let mut observations = Vec::new();
    add_requirement(
        &mut observations,
        "observation:run-outcome",
        std::iter::empty::<String>(),
    );
    add_requirement(
        &mut observations,
        "observation:stage-room-position",
        page.canonical_maps.iter().map(|map| format!("map-{map}")),
    );
    for tag in &mechanics {
        for id in observation_ids(tag) {
            add_requirement(&mut observations, *id, [tag.clone()]);
        }
    }
    let observation_unknown = vec!["page-specific-state-transition-fields".into()];

    let mut oracles = Vec::new();
    add_requirement(
        &mut oracles,
        "oracle:page-specific-semantic-success",
        std::iter::empty::<String>(),
    );
    add_requirement(
        &mut oracles,
        "oracle:failure-softlock-crash-classification",
        mechanics.iter().cloned(),
    );
    let oracle_unknown = vec!["exact-machine-checkable-success-predicate".into()];

    let mut fidelity = Vec::new();
    add_requirement(
        &mut fidelity,
        "fidelity:deterministic-replay-identity",
        std::iter::empty::<String>(),
    );
    for tag in page
        .canonical_tags
        .iter()
        .filter(|tag| tag.starts_with("platform-") || tag.starts_with("region-"))
    {
        add_requirement(&mut fidelity, format!("fidelity:{tag}"), [tag.clone()]);
    }
    for tag in &mechanics {
        if matches!(
            tag.as_str(),
            "mechanic-memory" | "mechanic-actor-corruption" | "mechanic-crash"
        ) {
            add_requirement(
                &mut fidelity,
                "fidelity:original-memory-and-process-behavior",
                [tag.clone()],
            );
        }
    }
    if !page.images.is_empty() || !page.videos.is_empty() {
        add_requirement(
            &mut fidelity,
            "fidelity:headful-visual-evidence",
            std::iter::empty::<String>(),
        );
    }
    let fidelity_unknown = if page.canonical_platforms.is_empty() {
        vec!["required-platform-version-and-region".into()]
    } else {
        vec!["game-version-and-runtime-fidelity-profile".into()]
    };

    let known_steps = extract_known_setup_steps(&page.body_markdown);
    let mut setup_unknown = vec![
        "exact-fixture-definition".into(),
        "exact-controller-tape".into(),
        "setup-completeness-confirmation".into(),
    ];
    if known_steps.is_empty() {
        setup_unknown.push("structured-source-setup-steps".into());
    }

    SkybookPageRequirements {
        slug: page.slug.clone(),
        source_path: page.source_path.clone(),
        source_url: page.source_url.clone(),
        scenarios: projection(scenarios, scenario_unknown),
        actions: projection(actions, action_unknown),
        observations: projection(observations, observation_unknown),
        oracles: projection(oracles, oracle_unknown),
        fidelity: projection(fidelity, fidelity_unknown),
        setup: SkybookSetupKnowledge {
            known_steps,
            unknown: sorted_unique(setup_unknown),
        },
    }
}

fn action_ids(mechanic: &str) -> &'static [&'static str] {
    match mechanic {
        "mechanic-movement" => &["action:movement", "action:camera-and-facing"],
        "mechanic-collision" | "mechanic-oob" | "mechanic-step-clip" => {
            &["action:collision-approach", "action:precise-positioning"]
        }
        "mechanic-combat" => &["action:combat", "action:targeting-and-damage"],
        "mechanic-cutscene" | "mechanic-storage" => &["action:event-and-ui-timing"],
        "mechanic-warp" | "mechanic-back-in-time" => &["action:transition-timing"],
        "mechanic-memory" | "mechanic-actor-corruption" => &["action:state-manipulation"],
        "mechanic-rng" => &["action:rng-manipulation"],
        "mechanic-long-jump-attack" => &["action:long-jump-attack"],
        _ => &[],
    }
}

fn observation_ids(mechanic: &str) -> &'static [&'static str] {
    match mechanic {
        "mechanic-movement" => &["observation:motion-and-velocity"],
        "mechanic-collision" | "mechanic-oob" | "mechanic-step-clip" => &[
            "observation:contacts-and-collision",
            "observation:oob-state",
        ],
        "mechanic-combat" => &["observation:combat-damage-and-actors"],
        "mechanic-cutscene" | "mechanic-storage" => &["observation:event-ui-and-control-ownership"],
        "mechanic-warp" | "mechanic-back-in-time" => {
            &["observation:transition-and-entrance-resolution"]
        }
        "mechanic-memory" | "mechanic-actor-corruption" => {
            &["observation:actor-resource-and-memory-facts"]
        }
        "mechanic-softlock" => &["observation:liveness-and-control"],
        "mechanic-crash" => &["observation:process-termination"],
        "mechanic-rng" => &["observation:rng-sensitive-state"],
        _ => &[],
    }
}

fn extract_known_setup_steps(markdown: &str) -> Vec<String> {
    let mut in_fence = false;
    let mut in_setup_section = false;
    let mut steps = Vec::new();
    for raw in markdown.lines() {
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(heading) = line.strip_prefix("## ") {
            let lower = heading.to_ascii_lowercase();
            in_setup_section = [
                "requirement",
                "setup",
                "method",
                "instruction",
                "procedure",
                "step",
                "execution",
            ]
            .iter()
            .any(|word| lower.contains(word));
            continue;
        }
        if !in_setup_section || line.starts_with('#') {
            continue;
        }
        let item = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .or_else(|| numbered_list_item(line));
        if let Some(item) = item {
            let clean = item.trim().to_owned();
            if !clean.is_empty() && !steps.contains(&clean) {
                steps.push(clean);
            }
        }
    }
    steps
}

fn numbered_list_item(line: &str) -> Option<&str> {
    let dot = line.find('.')?;
    if dot == 0 || !line[..dot].bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    line[dot + 1..].strip_prefix(' ')
}

fn add_requirement(
    requirements: &mut Vec<SkybookRequirement>,
    id: impl Into<String>,
    source_tags: impl IntoIterator<Item = String>,
) {
    let id = id.into();
    let tags = sorted_unique(source_tags.into_iter().collect());
    if let Some(existing) = requirements.iter_mut().find(|item| item.id == id) {
        existing.source_tags.extend(tags);
        existing.source_tags.sort();
        existing.source_tags.dedup();
    } else {
        requirements.push(SkybookRequirement {
            id,
            source_tags: tags,
        });
    }
}

fn projection(
    mut required: Vec<SkybookRequirement>,
    unknown: Vec<String>,
) -> SkybookRequirementProjection {
    required.sort();
    required.dedup();
    SkybookRequirementProjection {
        required,
        unknown: sorted_unique(unknown),
    }
}

fn validate_projection(
    projection: &SkybookRequirementProjection,
) -> Result<(), SkybookRequirementsError> {
    if projection.required.is_empty()
        || !projection.required.windows(2).all(|pair| pair[0] < pair[1])
        || !is_sorted_unique(&projection.unknown)
        || projection.required.iter().any(|requirement| {
            requirement.id.is_empty() || !is_sorted_unique(&requirement.source_tags)
        })
    {
        return Err(requirement_error(
            "requirement projection is empty or non-canonical",
        ));
    }
    Ok(())
}

fn coverage(pages: &[SkybookPageRequirements]) -> SkybookRequirementsCoverage {
    SkybookRequirementsCoverage {
        page_count: pages.len(),
        scenario_requirement_count: pages.iter().map(|p| p.scenarios.required.len()).sum(),
        action_requirement_count: pages.iter().map(|p| p.actions.required.len()).sum(),
        observation_requirement_count: pages.iter().map(|p| p.observations.required.len()).sum(),
        oracle_requirement_count: pages.iter().map(|p| p.oracles.required.len()).sum(),
        fidelity_requirement_count: pages.iter().map(|p| p.fidelity.required.len()).sum(),
        pages_with_known_setup_steps: pages
            .iter()
            .filter(|p| !p.setup.known_steps.is_empty())
            .count(),
        known_setup_step_count: pages.iter().map(|p| p.setup.known_steps.len()).sum(),
        pages_with_scenario_unknowns: pages
            .iter()
            .filter(|p| !p.scenarios.unknown.is_empty())
            .count(),
        pages_with_action_unknowns: pages
            .iter()
            .filter(|p| !p.actions.unknown.is_empty())
            .count(),
        pages_with_observation_unknowns: pages
            .iter()
            .filter(|p| !p.observations.unknown.is_empty())
            .count(),
        pages_with_oracle_unknowns: pages
            .iter()
            .filter(|p| !p.oracles.unknown.is_empty())
            .count(),
        pages_with_fidelity_unknowns: pages
            .iter()
            .filter(|p| !p.fidelity.unknown.is_empty())
            .count(),
        pages_with_setup_unknowns: pages.iter().filter(|p| !p.setup.unknown.is_empty()).count(),
    }
}

fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn is_sorted_unique(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn requirement_error(message: impl Into<String>) -> SkybookRequirementsError {
    SkybookRequirementsError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn page() -> SkybookPage {
        SkybookPage {
            slug: "gate-clip".into(),
            source_path: "_posts/gate-clip.md".into(),
            source_url: "https://example.invalid/source".into(),
            source_sha256: Digest([1; 32]),
            body_sha256: Digest([2; 32]),
            title: "Gate Clip".into(),
            description: "Clip a gate.".into(),
            authors: vec![],
            categories: vec!["Glitches".into()],
            tags: vec![],
            canonical_tags: vec![
                "map-arbiters-grounds".into(),
                "mechanic-collision".into(),
                "mechanic-oob".into(),
                "platform-wii".into(),
            ],
            resolved_aliases: vec![],
            platforms: vec!["wii".into()],
            maps: vec!["arbiters-grounds".into()],
            canonical_platforms: vec!["wii".into()],
            canonical_maps: vec!["arbiters-grounds".into()],
            canonical_regions: vec![],
            date: None,
            front_matter: BTreeMap::new(),
            body_markdown: "## Requirements\n- Iron Boots\n\n## Setup\n1. Face the fence\n2. Roll into the seam\n\n## Notes\n- Not a setup step\n".into(),
            internal_links: vec![],
            source_links: vec![],
            images: vec![],
            videos: vec![],
        }
    }

    #[test]
    fn page_mapping_preserves_provenance_and_unknowns() {
        let requirements = derive_page(&page());
        assert_eq!(
            requirements.setup.known_steps,
            ["Iron Boots", "Face the fence", "Roll into the seam"]
        );
        assert!(requirements.scenarios.required.iter().any(|requirement| {
            requirement.id == "scenario:map-arbiters-grounds"
                && requirement.source_tags == ["map-arbiters-grounds"]
        }));
        assert!(
            requirements
                .observations
                .required
                .iter()
                .any(|requirement| requirement.id == "observation:contacts-and-collision")
        );
        assert!(
            requirements
                .oracles
                .unknown
                .contains(&"exact-machine-checkable-success-predicate".into())
        );
    }

    #[test]
    fn setup_extraction_ignores_fences_and_unrelated_sections() {
        let markdown = "## Setup\n- First\n```\n- Fake\n```\n2. Second\n## Notes\n- Third\n";
        assert_eq!(extract_known_setup_steps(markdown), ["First", "Second"]);
    }
}
