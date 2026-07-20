//! Exact game-content identity and mutable runtime configuration.

use crate::artifact::Digest;
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const CONTENT_IDENTITY_SCHEMA: &str = "dusklight.route-planner.content-identity/v1";
pub const RUNTIME_CONFIGURATION_SCHEMA: &str = "dusklight.route-planner.runtime-configuration/v1";
pub const EQUIVALENCE_SET_SCHEMA: &str = "dusklight.route-planner.equivalence-set/v1";
pub const CONTENT_GROUP_SCHEMA: &str = "dusklight.route-planner.content-group/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GamePlatform {
    GameCube,
    Wii,
    WiiU,
    Shield,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GameRegion {
    Usa,
    Pal,
    Japan,
    Korea,
    China,
}

/// Values detected from original content. Friendly names are deliberately absent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentFingerprint {
    pub platform: GamePlatform,
    pub region: GameRegion,
    pub revision: String,
    pub product_id: String,
    pub executable_sha256: Digest,
    pub game_data_sha256: Digest,
    pub resource_manifest_sha256: Digest,
}

/// A friendly stable ID bound to one verified content fingerprint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentIdentity {
    pub schema: String,
    pub id: String,
    pub fingerprint: ContentFingerprint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ConfigurationValue {
    Boolean(bool),
    Integer(i64),
    Text(String),
}

/// Runtime-selectable state is separate from immutable disc/data identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfiguration {
    pub schema: String,
    pub content_sha256: Digest,
    pub language: String,
    pub settings: BTreeMap<String, ConfigurationValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactContext {
    pub content_sha256: Digest,
    pub runtime_configuration_sha256: Digest,
}

/// A friendly family expands only to enumerated exact content identities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentGroup {
    pub schema: String,
    pub id: String,
    pub members: Vec<Digest>,
}

/// Query selectors are either exact or backed by an evidenced equivalence set.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextSelector {
    Exact { context: ExactContext },
    Equivalent { equivalence_set_id: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EquivalenceEvidenceKind {
    StaticDiff,
    SourceAudit,
    TraceComparison,
    CommunityVerification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EquivalenceEvidence {
    pub kind: EquivalenceEvidenceKind,
    pub source_id: String,
    pub source_sha256: Digest,
}

/// Declares semantic equivalence over exact contexts. It cannot contain wildcards.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EquivalenceSet {
    pub schema: String,
    pub id: String,
    pub semantic_scope: String,
    pub contexts: Vec<ExactContext>,
    pub evidence: Vec<EquivalenceEvidence>,
}

impl ContentFingerprint {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        validate_label("fingerprint.revision", &self.revision)?;
        if self.product_id.is_empty()
            || self.product_id.len() > 32
            || !self
                .product_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(PlannerContractError::new(
                "fingerprint.product_id",
                "must be 1-32 ASCII letters, digits, '-' or '_'",
            ));
        }
        for (field, digest) in [
            ("fingerprint.executable_sha256", self.executable_sha256),
            ("fingerprint.game_data_sha256", self.game_data_sha256),
            (
                "fingerprint.resource_manifest_sha256",
                self.resource_manifest_sha256,
            ),
        ] {
            require_digest(field, digest)?;
        }
        Ok(())
    }
}

impl ContentIdentity {
    pub fn new(
        id: impl Into<String>,
        fingerprint: ContentFingerprint,
    ) -> Result<Self, PlannerContractError> {
        let identity = Self {
            schema: CONTENT_IDENTITY_SCHEMA.into(),
            id: id.into(),
            fingerprint,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != CONTENT_IDENTITY_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        validate_stable_id("id", &self.id)?;
        self.fingerprint.validate()
    }

    /// A requested friendly identity may not override detected content.
    pub fn verify_detected(
        &self,
        detected: &ContentFingerprint,
    ) -> Result<(), PlannerContractError> {
        self.validate()?;
        detected.validate()?;
        if &self.fingerprint != detected {
            return Err(PlannerContractError::new(
                "fingerprint",
                "does not match the detected content",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let identity: Self = serde_json::from_slice(bytes)?;
        identity.validate()?;
        if identity.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "content_identity",
                "is not canonical JSON",
            ));
        }
        Ok(identity)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl RuntimeConfiguration {
    pub fn new(
        content: &ContentIdentity,
        language: impl Into<String>,
        settings: BTreeMap<String, ConfigurationValue>,
    ) -> Result<Self, PlannerContractError> {
        let configuration = Self {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: content.digest()?,
            language: language.into(),
            settings,
        };
        configuration.validate()?;
        Ok(configuration)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != RUNTIME_CONFIGURATION_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        require_digest("content_sha256", self.content_sha256)?;
        validate_language(&self.language)?;
        if self.settings.len() > 128 {
            return Err(PlannerContractError::new(
                "settings",
                "must contain at most 128 entries",
            ));
        }
        for (key, value) in &self.settings {
            validate_stable_id("settings key", key)?;
            if let ConfigurationValue::Text(value) = value {
                validate_label("settings text", value)?;
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn exact_context(&self) -> Result<ExactContext, PlannerContractError> {
        Ok(ExactContext {
            content_sha256: self.content_sha256,
            runtime_configuration_sha256: self.digest()?,
        })
    }
}

impl ContentGroup {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != CONTENT_GROUP_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        validate_stable_id("id", &self.id)?;
        if self.members.is_empty() || self.members.len() > 64 {
            return Err(PlannerContractError::new(
                "members",
                "must contain between 1 and 64 exact content digests",
            ));
        }
        let mut previous = None;
        for member in &self.members {
            require_digest("members", *member)?;
            if previous.is_some_and(|prior: Digest| prior >= *member) {
                return Err(PlannerContractError::new(
                    "members",
                    "must be unique and sorted by exact content digest",
                ));
            }
            previous = Some(*member);
        }
        Ok(())
    }
}

impl ContextSelector {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        match self {
            Self::Exact { context } => {
                require_digest("context.content_sha256", context.content_sha256)?;
                require_digest(
                    "context.runtime_configuration_sha256",
                    context.runtime_configuration_sha256,
                )
            }
            Self::Equivalent { equivalence_set_id } => {
                validate_stable_id("equivalence_set_id", equivalence_set_id)
            }
        }
    }
}

impl EquivalenceSet {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != EQUIVALENCE_SET_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        validate_stable_id("id", &self.id)?;
        validate_stable_id("semantic_scope", &self.semantic_scope)?;
        if self.contexts.len() < 2 || self.contexts.len() > 64 {
            return Err(PlannerContractError::new(
                "contexts",
                "must contain between 2 and 64 exact contexts",
            ));
        }
        let mut contexts = BTreeSet::new();
        let mut previous = None;
        for context in &self.contexts {
            require_digest("contexts.content_sha256", context.content_sha256)?;
            require_digest(
                "contexts.runtime_configuration_sha256",
                context.runtime_configuration_sha256,
            )?;
            if !contexts.insert(context)
                || previous.is_some_and(|prior: &ExactContext| prior >= context)
            {
                return Err(PlannerContractError::new(
                    "contexts",
                    "must be unique and sorted by exact identity",
                ));
            }
            previous = Some(context);
        }
        if self.evidence.is_empty() || self.evidence.len() > 64 {
            return Err(PlannerContractError::new(
                "evidence",
                "must contain between 1 and 64 records",
            ));
        }
        let mut sources = BTreeSet::new();
        for evidence in &self.evidence {
            validate_stable_id("evidence.source_id", &evidence.source_id)?;
            require_digest("evidence.source_sha256", evidence.source_sha256)?;
            if !sources.insert((evidence.source_id.as_str(), evidence.source_sha256)) {
                return Err(PlannerContractError::new(
                    "evidence",
                    "contains a duplicate source",
                ));
            }
        }
        Ok(())
    }

    pub fn proves(&self, context: &ExactContext) -> bool {
        self.contexts.binary_search(context).is_ok()
    }
}

fn validate_language(value: &str) -> Result<(), PlannerContractError> {
    if value.len() < 2
        || value.len() > 35
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(PlannerContractError::new(
            "language",
            "must be a lowercase language tag",
        ));
    }
    Ok(())
}

pub(crate) fn require_digest(field: &str, digest: Digest) -> Result<(), PlannerContractError> {
    if digest == Digest::ZERO {
        return Err(PlannerContractError::new(
            field,
            "must be a nonzero SHA-256 digest",
        ));
    }
    Ok(())
}
