//! Canonical catalogue of every exact retail/runtime context the planner supports.

use crate::artifact::Digest;
use crate::identity::ContentIdentity;
use crate::message_flow::bundled_gz2e01_english_message_flow_profile;
use crate::orig_discovery::{
    OrigInputScan, SupportedBuildRegistry, bundled_supported_build_registry,
};
use crate::{
    PlannerContractError, canonical_json, require_canonical_json_bytes, validate_stable_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::path::{Component, Path};

pub const SUPPORTED_CONTEXT_CATALOG_SCHEMA: &str =
    "dusklight.route-planner.supported-context-catalog/v2";
const BUNDLED_SUPPORTED_CONTEXT_CATALOG: &[u8] = include_bytes!("../data/supported-contexts.json");
const MAX_CONTEXTS: usize = 1_024;
const MAX_DISC_IMAGES: usize = 16;
const MAX_LANGUAGES: usize = 64;
const MAX_MESSAGE_ARCHIVES: usize = 1_024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedContextCatalog {
    pub schema: String,
    pub contexts: Vec<SupportedRuntimeContext>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedRuntimeContext {
    pub content: ContentIdentity,
    /// Digest of the canonical `orig-input-scan/v1` that reproduced the
    /// executable, normalized game-data, and resource-manifest fingerprint.
    pub orig_scan_sha256: Digest,
    /// Exact retail container identities when those bytes were available.
    /// An authenticated extracted tree remains a supported extraction input
    /// when this list is empty; no reconstructed image may be substituted.
    pub disc_images: Vec<DiscImageIdentity>,
    pub languages: Vec<SupportedLanguageBundle>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscImageIdentity {
    pub format: String,
    pub bytes: u64,
    pub sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedLanguageBundle {
    pub language: String,
    pub locale_bundle: String,
    pub message_import: MessageImportSupport,
    pub message_archives: Vec<SupportedResourceIdentity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessageImportSupport {
    /// Exact-content backing semantics and locale selection have both been
    /// audited, so this language can construct planner message facts.
    Supported {
        profile_id: String,
        profile_sha256: Digest,
    },
    /// Archive identities are exact, but backing/handler semantics have not
    /// been promoted from source-family evidence for this executable.
    ResourceIdentityOnly,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedResourceIdentity {
    pub relative_path: String,
    pub sha256: Digest,
}

impl SupportedContextCatalog {
    pub fn validate(&self, registry: &SupportedBuildRegistry) -> Result<(), PlannerContractError> {
        registry.validate()?;
        if self.schema != SUPPORTED_CONTEXT_CATALOG_SCHEMA {
            return Err(PlannerContractError::new(
                "supported_context_catalog.schema",
                "is unsupported",
            ));
        }
        if self.contexts.is_empty() || self.contexts.len() > MAX_CONTEXTS {
            return Err(PlannerContractError::new(
                "supported_context_catalog.contexts",
                format!("must contain between 1 and {MAX_CONTEXTS} contexts"),
            ));
        }
        if self.contexts.len() != registry.identities.len() {
            return Err(PlannerContractError::new(
                "supported_context_catalog.contexts",
                "must catalogue every and only registered supported build",
            ));
        }
        let mut previous = None;
        for context in &self.contexts {
            context.validate()?;
            if previous.is_some_and(|id: &str| id >= context.content.id.as_str()) {
                return Err(PlannerContractError::new(
                    "supported_context_catalog.contexts",
                    "must be unique and sorted by stable content ID",
                ));
            }
            let registered = registry
                .identities
                .iter()
                .find(|identity| identity.id == context.content.id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "supported_context_catalog.content",
                        "is absent from the supported-build registry",
                    )
                })?;
            if registered != &context.content {
                return Err(PlannerContractError::new(
                    "supported_context_catalog.content",
                    "differs from the exact supported-build registry identity",
                ));
            }
            previous = Some(context.content.id.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(
        &self,
        registry: &SupportedBuildRegistry,
    ) -> Result<Vec<u8>, PlannerContractError> {
        self.validate(registry)?;
        canonical_json(self)
    }

    pub fn digest(
        &self,
        registry: &SupportedBuildRegistry,
    ) -> Result<Digest, PlannerContractError> {
        Ok(Digest(
            Sha256::digest(self.canonical_bytes(registry)?).into(),
        ))
    }

    pub fn decode_canonical(
        bytes: &[u8],
        registry: &SupportedBuildRegistry,
    ) -> Result<Self, PlannerContractError> {
        let catalog: Self = serde_json::from_slice(bytes)?;
        catalog.validate(registry)?;
        require_canonical_json_bytes(
            "supported_context_catalog",
            bytes,
            &catalog.canonical_bytes(registry)?,
        )?;
        Ok(catalog)
    }
}

impl SupportedRuntimeContext {
    fn validate(&self) -> Result<(), PlannerContractError> {
        self.content.validate()?;
        if self.orig_scan_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "supported_context.orig_scan_sha256",
                "must bind the canonical reproduced input scan",
            ));
        }
        if self.disc_images.len() > MAX_DISC_IMAGES {
            return Err(PlannerContractError::new(
                "supported_context.disc_images",
                format!("must contain at most {MAX_DISC_IMAGES} exact images"),
            ));
        }
        if self.languages.is_empty() || self.languages.len() > MAX_LANGUAGES {
            return Err(PlannerContractError::new(
                "supported_context.languages",
                format!("must contain between 1 and {MAX_LANGUAGES} runtime languages"),
            ));
        }
        if self.disc_images.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(PlannerContractError::new(
                "supported_context.disc_images",
                "must be unique and sorted",
            ));
        }
        for image in &self.disc_images {
            validate_stable_id("supported_context.disc_images.format", &image.format)?;
            if image.bytes == 0 || image.sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "supported_context.disc_images",
                    "must bind nonempty bytes to a nonzero digest",
                ));
            }
        }
        let mut previous = None;
        for language in &self.languages {
            language.validate()?;
            if previous.is_some_and(|value: &str| value >= language.language.as_str()) {
                return Err(PlannerContractError::new(
                    "supported_context.languages",
                    "must be unique and sorted by runtime language",
                ));
            }
            previous = Some(language.language.as_str());
        }
        Ok(())
    }

    /// Reopens a canonical retail-tree scan and proves that this catalogue row
    /// names that exact build and every selected locale archive. This is kept
    /// separate from ordinary catalogue decoding because normal planner use
    /// must not require proprietary `orig/` inputs.
    pub fn validate_orig_scan(&self, scan: &OrigInputScan) -> Result<(), PlannerContractError> {
        scan.validate()?;
        if scan.fingerprint != self.content.fingerprint || scan.digest()? != self.orig_scan_sha256 {
            return Err(PlannerContractError::new(
                "supported_context.orig_scan_sha256",
                "does not reproduce the catalogued exact content identity",
            ));
        }
        for language in &self.languages {
            if language.message_archives.len() != 9 {
                return Err(PlannerContractError::new(
                    "supported_language.message_archives",
                    "must contain the selected base archive and groups 1 through 8",
                ));
            }
            let expected_paths =
                std::iter::once(format!(
                    "files/res/Msg{}/bmgres.arc",
                    language.locale_bundle
                ))
                .chain((1..=8).map(|group| {
                    format!("files/res/Msg{}/bmgres{group}.arc", language.locale_bundle)
                }))
                .collect::<Vec<_>>();
            if language
                .message_archives
                .iter()
                .map(|archive| archive.relative_path.as_str())
                .ne(expected_paths.iter().map(String::as_str))
            {
                return Err(PlannerContractError::new(
                    "supported_language.message_archives",
                    "does not contain the exact selected base/group path set",
                ));
            }
            for archive in &language.message_archives {
                let record = scan
                    .files
                    .binary_search_by(|record| {
                        record.relative_path.as_str().cmp(&archive.relative_path)
                    })
                    .ok()
                    .map(|index| &scan.files[index])
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "supported_language.message_archives",
                            "references an archive absent from the exact input scan",
                        )
                    })?;
                if record.sha256 != archive.sha256 {
                    return Err(PlannerContractError::new(
                        "supported_language.message_archives",
                        "digest differs from the exact input scan",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl SupportedLanguageBundle {
    fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("supported_language.language", &self.language)?;
        validate_stable_id("supported_language.locale_bundle", &self.locale_bundle)?;
        if let MessageImportSupport::Supported {
            profile_id,
            profile_sha256,
        } = &self.message_import
        {
            validate_stable_id("supported_language.message_import.profile_id", profile_id)?;
            if *profile_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "supported_language.message_import.profile_sha256",
                    "must be nonzero when semantic import is supported",
                ));
            }
        }
        if self.message_archives.is_empty() || self.message_archives.len() > MAX_MESSAGE_ARCHIVES {
            return Err(PlannerContractError::new(
                "supported_language",
                "must bind a bounded nonempty archive set",
            ));
        }
        if self
            .message_archives
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(PlannerContractError::new(
                "supported_language.message_archives",
                "must be unique and sorted by path and digest",
            ));
        }
        let expected_prefix = format!("files/res/Msg{}/", self.locale_bundle);
        for archive in &self.message_archives {
            let path = Path::new(&archive.relative_path);
            if archive.sha256 == Digest::ZERO
                || path.is_absolute()
                || path
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
                || !archive.relative_path.starts_with(&expected_prefix)
                || !archive.relative_path.ends_with(".arc")
            {
                return Err(PlannerContractError::new(
                    "supported_language.message_archives",
                    "contains an invalid, cross-bundle, or unsealed archive reference",
                ));
            }
        }
        Ok(())
    }
}

pub fn bundled_supported_context_catalog() -> Result<SupportedContextCatalog, PlannerContractError>
{
    let registry = bundled_supported_build_registry()?;
    let catalog =
        SupportedContextCatalog::decode_canonical(BUNDLED_SUPPORTED_CONTEXT_CATALOG, &registry)?;
    let profile = bundled_gz2e01_english_message_flow_profile()?;
    let language = catalog
        .contexts
        .iter()
        .find(|context| context.content.id == "gcn-us-1.0-gz2e01")
        .and_then(|context| {
            context
                .languages
                .iter()
                .find(|language| language.language == "en")
        })
        .ok_or_else(|| {
            PlannerContractError::new(
                "supported_context_catalog",
                "omits the bundled GZ2E01 English runtime",
            )
        })?;
    let MessageImportSupport::Supported {
        profile_id,
        profile_sha256,
    } = &language.message_import
    else {
        return Err(PlannerContractError::new(
            "supported_context_catalog.languages",
            "marks the bundled GZ2E01 English runtime as resource-only",
        ));
    };
    if profile_id != &profile.id
        || *profile_sha256 != profile.digest()?
        || profile.language_bundles.get("en").map(String::as_str)
            != Some(language.locale_bundle.as_str())
    {
        return Err(PlannerContractError::new(
            "supported_context_catalog.languages",
            "drifts from the bundled GZ2E01 English import profile",
        ));
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orig_discovery::scan_orig_tree;
    use std::path::Path;

    #[test]
    fn bundled_catalog_binds_exact_disc_build_and_language_resources() {
        let registry = bundled_supported_build_registry().unwrap();
        let catalog = bundled_supported_context_catalog().unwrap();
        assert_eq!(catalog.contexts.len(), 3);
        let context = catalog
            .contexts
            .iter()
            .find(|context| context.content.id == "gcn-us-1.0-gz2e01")
            .unwrap();
        assert_eq!(context.content.id, "gcn-us-1.0-gz2e01");
        assert_eq!(context.disc_images.len(), 1);
        assert_eq!(context.disc_images[0].bytes, 1_459_978_240);
        assert_eq!(context.languages.len(), 1);
        assert_eq!(context.languages[0].language, "en");
        assert_eq!(context.languages[0].message_archives.len(), 9);
        assert_ne!(catalog.digest(&registry).unwrap(), Digest::ZERO);
    }

    #[test]
    fn catalog_rejects_nearby_build_language_and_archive_drift() {
        let registry = bundled_supported_build_registry().unwrap();
        let catalog = bundled_supported_context_catalog().unwrap();

        let mut changed = catalog.clone();
        let context = changed
            .contexts
            .iter_mut()
            .find(|context| context.content.id == "gcn-us-1.0-gz2e01")
            .unwrap();
        context.content.fingerprint.revision = "1.1".into();
        assert!(changed.validate(&registry).is_err());

        let mut changed = catalog.clone();
        let context = changed
            .contexts
            .iter_mut()
            .find(|context| context.content.id == "gcn-us-1.0-gz2e01")
            .unwrap();
        context.languages[0].language = "fr".into();
        assert!(changed.validate(&registry).is_ok());
        assert_ne!(
            changed.digest(&registry).unwrap(),
            catalog.digest(&registry).unwrap()
        );

        let mut changed = catalog.clone();
        let context = changed
            .contexts
            .iter_mut()
            .find(|context| context.content.id == "gcn-us-1.0-gz2e01")
            .unwrap();
        context.languages[0].message_archives[0].sha256 = Digest::ZERO;
        assert!(changed.validate(&registry).is_err());
    }

    #[test]
    fn resource_only_languages_cannot_smuggle_a_semantic_profile() {
        let registry = bundled_supported_build_registry().unwrap();
        let catalog = bundled_supported_context_catalog().unwrap();
        let pal = catalog
            .contexts
            .iter()
            .find(|context| context.content.id == "gcn-pal-1.0-gz2p01")
            .unwrap();
        assert!(pal.disc_images.is_empty());
        assert_eq!(pal.languages.len(), 5);
        assert!(pal.languages.iter().all(|language| matches!(
            language.message_import,
            MessageImportSupport::ResourceIdentityOnly
        )));

        let mut changed = catalog.clone();
        let pal = changed
            .contexts
            .iter_mut()
            .find(|context| context.content.id == "gcn-pal-1.0-gz2p01")
            .unwrap();
        pal.orig_scan_sha256 = Digest::ZERO;
        assert!(changed.validate(&registry).is_err());
    }

    #[test]
    fn bundled_catalog_bytes_are_canonical_lf_json() {
        let registry = bundled_supported_build_registry().unwrap();
        let catalog = bundled_supported_context_catalog().unwrap();
        assert_eq!(
            catalog.canonical_bytes(&registry).unwrap(),
            BUNDLED_SUPPORTED_CONTEXT_CATALOG
        );
        let mut crlf = BUNDLED_SUPPORTED_CONTEXT_CATALOG.to_vec();
        crlf.extend_from_slice(b"\r\n");
        assert!(SupportedContextCatalog::decode_canonical(&crlf, &registry).is_err());
    }

    #[test]
    fn bundled_catalog_reopens_every_available_exact_orig_scan() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(4)
            .unwrap();
        let cases = [
            ("gcn-pal-1.0-gz2p01", "orig/GZ2P01"),
            ("gcn-us-1.0-gz2e01", "orig/GZ2E01"),
            ("wii-us-1.2-rzde01", "orig/RZDE01_02/DATA"),
        ];
        let catalog = bundled_supported_context_catalog().unwrap();
        for (content_id, relative_root) in cases {
            let orig = root.join(relative_root);
            if !orig.is_dir() {
                continue;
            }
            let context = catalog
                .contexts
                .iter()
                .find(|context| context.content.id == content_id)
                .unwrap();
            let scan =
                scan_orig_tree(&orig, Some(&context.content.fingerprint.product_id)).unwrap();
            context.validate_orig_scan(&scan).unwrap();
        }
    }
}
