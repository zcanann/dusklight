//! Read-only, revision-bound import of the Skybook Markdown requirements corpus.

use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

pub const SKYBOOK_MANIFEST_SCHEMA: &str = "dusklight-skybook-manifest/v2";
const MAX_POSTS: usize = 4_096;
const MAX_POST_BYTES: usize = 4 * 1_024 * 1_024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookSourceIdentity {
    pub repository_url: String,
    pub git_revision: String,
    pub post_count: usize,
    pub categorized_glitch_count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookManifest {
    pub schema: String,
    pub content_sha256: Digest,
    pub source: SkybookSourceIdentity,
    pub alias_rules: Vec<SkybookAliasRule>,
    pub pages: Vec<SkybookPage>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookAliasRule {
    pub original: String,
    pub canonical: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookPage {
    pub slug: String,
    pub source_path: String,
    pub source_url: String,
    pub source_sha256: Digest,
    pub body_sha256: Digest,
    pub title: String,
    pub description: String,
    pub authors: Vec<String>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub canonical_tags: Vec<String>,
    pub resolved_aliases: Vec<SkybookAliasRule>,
    pub platforms: Vec<String>,
    pub maps: Vec<String>,
    pub canonical_platforms: Vec<String>,
    pub canonical_maps: Vec<String>,
    pub canonical_regions: Vec<String>,
    pub date: Option<String>,
    pub front_matter: BTreeMap<String, Vec<String>>,
    pub body_markdown: String,
    pub internal_links: Vec<SkybookInternalLink>,
    pub source_links: Vec<String>,
    pub images: Vec<SkybookImageEvidence>,
    pub videos: Vec<SkybookVideoEvidence>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookInternalLink {
    pub label: String,
    pub target: String,
    pub target_slug: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookImageEvidence {
    pub alt: String,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookVideoEvidence {
    pub provider: String,
    pub video_id: String,
    pub source: String,
}

#[derive(Debug)]
pub struct SkybookImportError(String);

impl fmt::Display for SkybookImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SkybookImportError {}

impl SkybookManifest {
    pub fn import_directory(
        root: &Path,
        repository_url: &str,
        git_revision: &str,
    ) -> Result<Self, SkybookImportError> {
        validate_source_identity(repository_url, git_revision)?;
        let posts = root.join("_posts");
        let mut paths = fs::read_dir(&posts)
            .map_err(|error| import_error(format!("cannot read {}: {error}", posts.display())))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| import_error(format!("cannot enumerate Skybook posts: {error}")))?;
        paths.retain(|path| path.extension().is_some_and(|extension| extension == "md"));
        paths.sort();
        if paths.is_empty() || paths.len() > MAX_POSTS {
            return Err(import_error(
                "Skybook post count is empty or exceeds its bound",
            ));
        }

        let mut pages = Vec::with_capacity(paths.len());
        for path in paths {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| import_error("Skybook post escaped its source root"))?;
            let source_path = portable_path(relative)?;
            let bytes = fs::read(&path).map_err(|error| {
                import_error(format!("cannot read Skybook post {source_path}: {error}"))
            })?;
            pages.push(parse_page(
                &bytes,
                &source_path,
                repository_url,
                git_revision,
            )?);
        }
        pages.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        let categorized_glitch_count = pages
            .iter()
            .filter(|page| {
                page.categories
                    .iter()
                    .any(|category| category.eq_ignore_ascii_case("glitches"))
            })
            .count();
        let mut manifest = Self {
            schema: SKYBOOK_MANIFEST_SCHEMA.into(),
            content_sha256: Digest::ZERO,
            source: SkybookSourceIdentity {
                repository_url: repository_url.trim_end_matches('/').into(),
                git_revision: git_revision.into(),
                post_count: pages.len(),
                categorized_glitch_count,
            },
            alias_rules: alias_rules(),
            pages,
        };
        manifest.content_sha256 = manifest.compute_content_sha256()?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), SkybookImportError> {
        if self.schema != SKYBOOK_MANIFEST_SCHEMA {
            return Err(import_error("unknown Skybook manifest schema"));
        }
        validate_source_identity(&self.source.repository_url, &self.source.git_revision)?;
        if self.alias_rules != alias_rules() {
            return Err(import_error("Skybook alias catalog is invalid"));
        }
        if self.pages.is_empty()
            || self.pages.len() > MAX_POSTS
            || self.source.post_count != self.pages.len()
        {
            return Err(import_error("Skybook manifest post count is invalid"));
        }
        let mut previous_path = None;
        let mut slugs = BTreeSet::new();
        for page in &self.pages {
            if page.slug.is_empty()
                || page.source_path.is_empty()
                || page.title.is_empty()
                || page.source_sha256 == Digest::ZERO
                || page.body_sha256 == Digest::ZERO
                || !slugs.insert(page.slug.as_str())
                || previous_path.is_some_and(|previous| previous >= page.source_path.as_str())
            {
                return Err(import_error("Skybook page identity or ordering is invalid"));
            }
            let expected_url = format!(
                "{}/blob/{}/{}",
                self.source.repository_url, self.source.git_revision, page.source_path
            );
            if page.source_url != expected_url
                || page.body_sha256 != sha256(page.body_markdown.as_bytes())
                || page.canonical_tags != canonicalize_tags(&page.tags).0
                || page.resolved_aliases != canonicalize_tags(&page.tags).1
                || page.canonical_platforms != prefixed_values(&page.canonical_tags, "platform-")
                || page.canonical_maps != prefixed_values(&page.canonical_tags, "map-")
                || page.canonical_regions != prefixed_values(&page.canonical_tags, "region-")
            {
                return Err(import_error("Skybook page source binding is invalid"));
            }
            previous_path = Some(page.source_path.as_str());
        }
        let glitches = self
            .pages
            .iter()
            .filter(|page| {
                page.categories
                    .iter()
                    .any(|category| category.eq_ignore_ascii_case("glitches"))
            })
            .count();
        if self.source.categorized_glitch_count != glitches
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(import_error("Skybook manifest content identity is invalid"));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, SkybookImportError> {
        self.validate()?;
        let mut encoded = serde_json::to_vec_pretty(self)
            .map_err(|error| import_error(format!("cannot encode Skybook manifest: {error}")))?;
        encoded.push(b'\n');
        Ok(encoded)
    }

    fn compute_content_sha256(&self) -> Result<Digest, SkybookImportError> {
        let encoded =
            serde_json::to_vec(&(&self.schema, &self.source, &self.alias_rules, &self.pages))
                .map_err(|error| {
                    import_error(format!("cannot encode Skybook manifest: {error}"))
                })?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.skybook-manifest/v2\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn parse_page(
    bytes: &[u8],
    source_path: &str,
    repository_url: &str,
    git_revision: &str,
) -> Result<SkybookPage, SkybookImportError> {
    if bytes.is_empty() || bytes.len() > MAX_POST_BYTES {
        return Err(import_error(format!(
            "Skybook post {source_path} is empty or exceeds its bound"
        )));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| import_error(format!("Skybook post {source_path} is not UTF-8")))?;
    let normalized = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut lines = normalized.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(import_error(format!(
            "Skybook post {source_path} has no front matter"
        )));
    }
    let mut front_matter_text = String::new();
    let mut body_markdown = String::new();
    let mut closed = false;
    for line in lines {
        if !closed && line.trim_end() == "---" {
            closed = true;
            continue;
        }
        if closed {
            body_markdown.push_str(line);
        } else {
            front_matter_text.push_str(line);
        }
    }
    if !closed {
        return Err(import_error(format!(
            "Skybook post {source_path} has unterminated front matter"
        )));
    }
    let front_matter = parse_front_matter(&front_matter_text, source_path)?;
    let title = required_scalar(&front_matter, "title", source_path)?;
    let description = required_scalar(&front_matter, "description", source_path)?;
    let categories = values(&front_matter, "categories");
    let tags = values(&front_matter, "tags");
    let (canonical_tags, resolved_aliases) = canonicalize_tags(&tags);
    let mut authors = values(&front_matter, "authors");
    authors.extend(values(&front_matter, "author"));
    sort_dedup(&mut authors);
    let platforms = prefixed_values(&tags, "platform-");
    let maps = prefixed_values(&tags, "map-");
    let canonical_platforms = prefixed_values(&canonical_tags, "platform-");
    let canonical_maps = prefixed_values(&canonical_tags, "map-");
    let canonical_regions = prefixed_values(&canonical_tags, "region-");
    let date = front_matter
        .get("date")
        .and_then(|values| values.first())
        .cloned();
    let slug = source_path
        .strip_prefix("_posts/")
        .and_then(|path| path.strip_suffix(".md"))
        .ok_or_else(|| import_error(format!("invalid Skybook post path {source_path}")))?
        .into();
    let (internal_links, source_links, images, videos) = extract_evidence(&body_markdown);
    Ok(SkybookPage {
        slug,
        source_path: source_path.into(),
        source_url: format!(
            "{}/blob/{git_revision}/{source_path}",
            repository_url.trim_end_matches('/')
        ),
        source_sha256: sha256(bytes),
        body_sha256: sha256(body_markdown.as_bytes()),
        title,
        description,
        authors,
        categories,
        tags,
        canonical_tags,
        resolved_aliases,
        platforms,
        maps,
        canonical_platforms,
        canonical_maps,
        canonical_regions,
        date,
        front_matter,
        body_markdown,
        internal_links,
        source_links,
        images,
        videos,
    })
}

fn parse_front_matter(
    source: &str,
    source_path: &str,
) -> Result<BTreeMap<String, Vec<String>>, SkybookImportError> {
    let mut output = BTreeMap::new();
    for (index, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once(':').ok_or_else(|| {
            import_error(format!(
                "Skybook post {source_path} front matter line {} is invalid",
                index + 1
            ))
        })?;
        let key = key.trim();
        if key.is_empty() || output.contains_key(key) {
            return Err(import_error(format!(
                "Skybook post {source_path} has an invalid or duplicate front matter key"
            )));
        }
        output.insert(key.into(), parse_front_matter_value(value.trim())?);
    }
    Ok(output)
}

fn parse_front_matter_value(value: &str) -> Result<Vec<String>, SkybookImportError> {
    if value.starts_with('[') {
        let inner = value
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .ok_or_else(|| import_error("unterminated front matter sequence"))?;
        if inner.trim().is_empty() {
            return Ok(Vec::new());
        }
        return inner
            .split(',')
            .map(|item| parse_scalar(item.trim()))
            .collect();
    }
    Ok(vec![parse_scalar(value)?])
}

fn parse_scalar(value: &str) -> Result<String, SkybookImportError> {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return Ok(value[1..value.len() - 1].into());
        }
    }
    Ok(value.into())
}

fn extract_evidence(
    body: &str,
) -> (
    Vec<SkybookInternalLink>,
    Vec<String>,
    Vec<SkybookImageEvidence>,
    Vec<SkybookVideoEvidence>,
) {
    let prose = markdown_prose(body);
    let mut internal_links = BTreeSet::new();
    let mut external_links = BTreeSet::new();
    let mut images = BTreeSet::new();
    let mut videos = BTreeSet::new();
    for (is_image, label, target) in markdown_links(&prose) {
        let target = target.trim().to_string();
        let image_target = is_image || is_image_target(&target);
        if image_target {
            images.insert(SkybookImageEvidence {
                alt: label.clone(),
                source: target.clone(),
            });
        }
        if let Some(video) = video_evidence(&target) {
            videos.insert(video);
        } else if !image_target && (target.starts_with("http://") || target.starts_with("https://"))
        {
            external_links.insert(target.clone());
        } else if !image_target {
            internal_links.insert(SkybookInternalLink {
                label,
                target: target.clone(),
                target_slug: internal_target_slug(&target),
            });
        }
    }
    for url in bare_urls(&prose) {
        if let Some(video) = video_evidence(&url) {
            videos.insert(video);
        } else {
            external_links.insert(url);
        }
    }
    for video_id in liquid_youtube_ids(&prose) {
        videos.insert(SkybookVideoEvidence {
            provider: "youtube".into(),
            source: format!("{{% youtube {video_id} %}}"),
            video_id,
        });
    }
    (
        internal_links.into_iter().collect(),
        external_links.into_iter().collect(),
        images.into_iter().collect(),
        videos.into_iter().collect(),
    )
}

fn markdown_prose(body: &str) -> String {
    let mut output = String::with_capacity(body.len());
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            output.push_str(line);
            output.push('\n');
        }
    }
    output
}

fn markdown_links(body: &str) -> Vec<(bool, String, String)> {
    let mut output = Vec::new();
    for line in body.lines() {
        let mut cursor = 0;
        while let Some(marker_relative) = line[cursor..].find("](") {
            let marker = cursor + marker_relative;
            let Some(open) = line[..marker].rfind('[') else {
                cursor = marker + 2;
                continue;
            };
            let target_start = marker + 2;
            let Some(target_end_relative) = line[target_start..].find(')') else {
                break;
            };
            let target_end = target_start + target_end_relative;
            let is_image = open > 0 && line.as_bytes()[open - 1] == b'!';
            output.push((
                is_image,
                line[open + 1..marker].to_string(),
                line[target_start..target_end].to_string(),
            ));
            cursor = target_end + 1;
        }
    }
    output
}

fn bare_urls(body: &str) -> Vec<String> {
    let mut urls = BTreeSet::new();
    for prefix in ["https://", "http://"] {
        let mut cursor = 0;
        while let Some(relative) = body[cursor..].find(prefix) {
            let start = cursor + relative;
            let end = body[start..]
                .find(|character: char| {
                    character.is_whitespace() || matches!(character, ')' | ']' | '>' | '"' | '\'')
                })
                .map_or(body.len(), |relative| start + relative);
            let url = body[start..end].trim_end_matches(['.', ',', ';', ':']);
            if !url.is_empty() {
                urls.insert(url.to_string());
            }
            cursor = end.max(start + prefix.len());
        }
    }
    urls.into_iter().collect()
}

fn liquid_youtube_ids(body: &str) -> Vec<String> {
    let mut output = BTreeSet::new();
    let mut cursor = 0;
    while let Some(relative) = body[cursor..].find("{% youtube ") {
        let start = cursor + relative + "{% youtube ".len();
        let Some(end_relative) = body[start..].find(" %}") else {
            break;
        };
        let video_id = body[start..start + end_relative].trim();
        if valid_video_id(video_id) {
            output.insert(video_id.to_string());
        }
        cursor = start + end_relative + 3;
    }
    output.into_iter().collect()
}

fn video_evidence(url: &str) -> Option<SkybookVideoEvidence> {
    if let Some(youtube) = youtube_evidence(url) {
        return Some(youtube);
    }
    for (needle, provider) in [
        ("clips.twitch.tv/", "twitch"),
        ("twitch.tv/videos/", "twitch"),
        ("streamable.com/", "streamable"),
        ("vimeo.com/", "vimeo"),
    ] {
        let Some((_, tail)) = url.split_once(needle) else {
            continue;
        };
        let video_id = tail.split(['?', '#', '/', '&']).next().unwrap_or_default();
        if valid_video_id(video_id) {
            return Some(SkybookVideoEvidence {
                provider: provider.into(),
                video_id: video_id.into(),
                source: url.into(),
            });
        }
    }
    None
}

fn youtube_evidence(url: &str) -> Option<SkybookVideoEvidence> {
    let without_query = url.split('&').next().unwrap_or(url);
    let id = if let Some((_, tail)) = without_query.split_once("youtu.be/") {
        tail.split(['?', '#', '/']).next()
    } else if let Some((_, tail)) = without_query.split_once("youtube.com/watch?v=") {
        tail.split(['?', '#', '&']).next()
    } else if let Some((_, tail)) = without_query.split_once("youtube.com/embed/") {
        tail.split(['?', '#', '/']).next()
    } else {
        None
    }?;
    valid_video_id(id).then(|| SkybookVideoEvidence {
        provider: "youtube".into(),
        video_id: id.into(),
        source: url.into(),
    })
}

fn valid_video_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn internal_target_slug(target: &str) -> Option<String> {
    let path = target.split(['?', '#']).next().unwrap_or(target);
    if let Some(slug) = path.strip_prefix("/posts/") {
        return Some(slug.trim_end_matches('/').into());
    }
    path.strip_prefix("_posts/")
        .or_else(|| path.strip_prefix("../_posts/"))
        .and_then(|path| path.strip_suffix(".md"))
        .map(str::to_string)
}

fn is_image_target(target: &str) -> bool {
    let path = target.split(['?', '#']).next().unwrap_or(target);
    ["png", "jpg", "jpeg", "gif", "webp", "svg"]
        .iter()
        .any(|extension| {
            path.to_ascii_lowercase()
                .ends_with(&format!(".{extension}"))
        })
}

const TAG_ALIASES: [(&str, &str); 7] = [
    ("castle-town-sewers", "map-castle-town-sewers"),
    ("map-snowpeak-mountain", "map-snowpeak-mountains"),
    ("map-zora-river", "map-zoras-river"),
    ("platform-gcn", "platform-gamecube"),
    ("platform-hd", "platform-wii-u-hd"),
    ("platform-pal", "region-pal"),
    ("reference", "type-reference"),
];

fn alias_rules() -> Vec<SkybookAliasRule> {
    TAG_ALIASES
        .iter()
        .map(|(original, canonical)| SkybookAliasRule {
            original: (*original).into(),
            canonical: (*canonical).into(),
        })
        .collect()
}

fn canonicalize_tags(tags: &[String]) -> (Vec<String>, Vec<SkybookAliasRule>) {
    let mut canonical_tags = Vec::with_capacity(tags.len());
    let mut resolved_aliases = Vec::new();
    for tag in tags {
        if let Some((_, canonical)) = TAG_ALIASES.iter().find(|(original, _)| tag == original) {
            canonical_tags.push((*canonical).into());
            resolved_aliases.push(SkybookAliasRule {
                original: tag.clone(),
                canonical: (*canonical).into(),
            });
        } else {
            canonical_tags.push(tag.clone());
        }
    }
    sort_dedup(&mut canonical_tags);
    resolved_aliases.sort();
    resolved_aliases.dedup();
    (canonical_tags, resolved_aliases)
}

fn prefixed_values(tags: &[String], prefix: &str) -> Vec<String> {
    let mut output = tags
        .iter()
        .filter_map(|tag| tag.strip_prefix(prefix).map(str::to_string))
        .collect::<Vec<_>>();
    sort_dedup(&mut output);
    output
}

fn required_scalar(
    values: &BTreeMap<String, Vec<String>>,
    key: &str,
    source_path: &str,
) -> Result<String, SkybookImportError> {
    values
        .get(key)
        .and_then(|values| values.first())
        .cloned()
        .ok_or_else(|| import_error(format!("Skybook post {source_path} has no {key}")))
}

fn values(values: &BTreeMap<String, Vec<String>>, key: &str) -> Vec<String> {
    values.get(key).cloned().unwrap_or_default()
}

fn sort_dedup(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

fn portable_path(path: &Path) -> Result<String, SkybookImportError> {
    let components = path
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| import_error("Skybook path is not UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(components.join("/"))
}

fn validate_source_identity(
    repository_url: &str,
    git_revision: &str,
) -> Result<(), SkybookImportError> {
    if !repository_url.starts_with("https://")
        || repository_url.trim_end_matches('/').is_empty()
        || git_revision.len() != 40
        || !git_revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(import_error(
            "Skybook repository or Git revision is invalid",
        ));
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn import_error(message: impl Into<String>) -> SkybookImportError {
    SkybookImportError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dusklight-skybook-import-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("_posts")).unwrap();
        root
    }

    #[test]
    fn imports_bom_crlf_front_matter_and_typed_evidence() {
        let root = root();
        let post = concat!(
            "\u{feff}---\r\n",
            "layout: post\r\n",
            "title: Example Clip\r\n",
            "description:\r\n",
            "authors: [alice, bob]\r\n",
            "categories: [Glitches]\r\n",
            "tags: [type-glitch, platform-gcn, platform-pal, map-snowpeak-mountain, reference]\r\n",
            "date: 2026-01-01 00:00:00\r\n",
            "---\r\n\r\n",
            "See [Step Clip](/posts/step-clip) and [notes](https://example.com/source).\r\n",
            "![cue](/assets/cue.png)\r\n",
            "{% youtube abc_DEF-123 %}\r\n",
            "```c++\r\nif (values[0] != 0) { fake(); }\r\n```\r\n",
            "https://clips.twitch.tv/Clip_123\r\n"
        );
        fs::write(root.join("_posts/example-clip.md"), post).unwrap();
        let manifest = SkybookManifest::import_directory(
            &root,
            "https://github.com/example/skybook",
            &"ab".repeat(20),
        )
        .unwrap();
        assert_eq!(manifest.source.post_count, 1);
        assert_eq!(manifest.source.categorized_glitch_count, 1);
        let page = &manifest.pages[0];
        assert_eq!(page.platforms, ["gcn", "pal"]);
        assert_eq!(page.maps, ["snowpeak-mountain"]);
        assert!(page.tags.contains(&"platform-gcn".into()));
        assert_eq!(page.source_path, "_posts/example-clip.md");
        assert_eq!(page.canonical_platforms, ["gamecube"]);
        assert_eq!(page.canonical_maps, ["snowpeak-mountains"]);
        assert_eq!(page.canonical_regions, ["pal"]);
        assert!(page.canonical_tags.contains(&"type-reference".into()));
        assert_eq!(page.resolved_aliases.len(), 4);
        assert_eq!(manifest.alias_rules.len(), TAG_ALIASES.len());
        assert_eq!(page.authors, ["alice", "bob"]);
        assert!(page.description.is_empty());
        assert_eq!(
            page.internal_links[0].target_slug.as_deref(),
            Some("step-clip")
        );
        assert_eq!(page.source_links, ["https://example.com/source"]);
        assert_eq!(page.images[0].source, "/assets/cue.png");
        assert_eq!(page.videos.len(), 2);
        assert!(
            page.videos
                .iter()
                .any(|video| video.video_id == "abc_DEF-123")
        );
        assert!(page.videos.iter().any(|video| video.video_id == "Clip_123"));
        assert!(
            page.internal_links
                .iter()
                .all(|link| !link.label.contains("fake"))
        );
        manifest.validate().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn content_identity_is_deterministic_and_tamper_evident() {
        let root = root();
        fs::write(
            root.join("_posts/example.md"),
            "---\ntitle: Example\ndescription: Example page.\ncategories: [Reference]\ntags: [type-reference]\n---\nBody.\n",
        )
        .unwrap();
        let revision = "12".repeat(20);
        let first = SkybookManifest::import_directory(
            &root,
            "https://github.com/example/skybook",
            &revision,
        )
        .unwrap();
        let second = SkybookManifest::import_directory(
            &root,
            "https://github.com/example/skybook",
            &revision,
        )
        .unwrap();
        assert_eq!(first, second);
        let mut tampered = first;
        tampered.pages[0].title.push_str(" changed");
        assert!(tampered.validate().is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_revision_or_front_matter_is_rejected() {
        let root = root();
        fs::write(root.join("_posts/example.md"), "no front matter").unwrap();
        assert!(
            SkybookManifest::import_directory(
                &root,
                "https://github.com/example/skybook",
                &"ab".repeat(20),
            )
            .is_err()
        );
        assert!(
            SkybookManifest::import_directory(&root, "https://github.com/example/skybook", "main",)
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
