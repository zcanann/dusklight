//! Validate workspace manifests, assets, and logical folders.

use super::*;

impl WorkspaceManifest {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Result<Self, WorkspaceError> {
        let manifest = Self {
            schema: WORKSPACE_MANIFEST_SCHEMA.into(),
            format_version: WORKSPACE_FORMAT_VERSION,
            id: id.into(),
            label: label.into(),
            mounted_libraries: Vec::new(),
            exact_context_defaults: Vec::new(),
            asset_roots: WorkspaceAssetKind::ALL
                .into_iter()
                .map(|kind| (kind, kind.root_name().into()))
                .collect(),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.schema != WORKSPACE_MANIFEST_SCHEMA
            || self.format_version != WORKSPACE_FORMAT_VERSION
        {
            return Err(WorkspaceError::new(format!(
                "workspace format is unsupported: schema {}, version {}; migrate it with this application before opening",
                self.schema, self.format_version
            )));
        }
        validate_stable_id("workspace id", &self.id)?;
        validate_label("workspace label", &self.label)?;
        if self.asset_roots.len() != WorkspaceAssetKind::ALL.len() {
            return Err(WorkspaceError::new(
                "workspace manifest must define every fixed asset root exactly once",
            ));
        }
        let mut roots = BTreeSet::new();
        for kind in WorkspaceAssetKind::ALL {
            let root = self
                .asset_roots
                .get(&kind)
                .ok_or_else(|| WorkspaceError::new(format!("missing {kind:?} asset root")))?;
            validate_relative_path("asset root", Path::new(root))?;
            if root != kind.root_name() {
                return Err(WorkspaceError::new(format!(
                    "{kind:?} asset root is fixed at {}; found {root}",
                    kind.root_name()
                )));
            }
            if !roots.insert(root) {
                return Err(WorkspaceError::new("asset roots must be unique"));
            }
        }
        let mut libraries = BTreeSet::new();
        for library in &self.mounted_libraries {
            validate_stable_id("library id", &library.id)?;
            validate_label("library version", &library.version)?;
            validate_label("library source", &library.source)?;
            if library.sha256 == Digest::ZERO {
                return Err(WorkspaceError::new("library digest must be nonzero"));
            }
            if !libraries.insert((&library.id, &library.version)) {
                return Err(WorkspaceError::new(format!(
                    "library {} version {} is mounted more than once",
                    library.id, library.version
                )));
            }
        }
        if !self.exact_context_defaults.iter().all(|context| {
            context.content_sha256 != Digest::ZERO
                && context.runtime_configuration_sha256 != Digest::ZERO
        }) {
            return Err(WorkspaceError::new(
                "exact-context defaults must use nonzero content and runtime digests",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorkspaceError> {
        self.validate()?;
        canonical_json(self)
    }
}

impl WorkspaceAsset {
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.schema != WORKSPACE_ASSET_SCHEMA {
            return Err(WorkspaceError::new(format!(
                "asset {} uses unsupported schema {}; migrate it with this application before opening",
                self.header.id, self.schema
            )));
        }
        validate_stable_id("asset id", &self.header.id)?;
        validate_label("asset label", &self.header.label)?;
        if self.header.version == 0 {
            return Err(WorkspaceError::new("asset version must be positive"));
        }
        if let Some(origin) = &self.header.origin {
            validate_stable_id("asset origin library id", &origin.library_id)?;
            validate_label("asset origin library version", &origin.library_version)?;
            validate_stable_id("asset origin source asset id", &origin.source_asset_id)?;
            if origin.library_sha256 == Digest::ZERO {
                return Err(WorkspaceError::new(
                    "asset origin library digest must be nonzero",
                ));
            }
        }
        if self.header.kind != self.payload.kind() {
            return Err(WorkspaceError::new(format!(
                "asset {} header kind does not match its typed payload",
                self.header.id
            )));
        }
        let mut previous = None;
        for reference in &self.references {
            validate_stable_id("asset reference", &reference.asset_id)?;
            if previous.is_some_and(|value| value >= reference) {
                return Err(WorkspaceError::new(
                    "asset references must be unique and sorted",
                ));
            }
            previous = Some(reference);
        }
        match &self.payload {
            WorkspaceAssetPayload::Scenario(scenario) => {
                validate_stable_id("scenario route graph id", &scenario.route_graph_id)?;
                if let Some(id) = &scenario.state_seed_id {
                    validate_stable_id("scenario state seed id", id)?;
                }
                if let Some(id) = &scenario.route_book_id {
                    validate_stable_id("scenario route book id", id)?;
                }
                if let ScenarioAnchor::StateSeed { state_seed_id } = &scenario.anchor {
                    validate_stable_id("scenario anchor state seed id", state_seed_id)?;
                    if scenario.state_seed_id.as_ref() != Some(state_seed_id) {
                        return Err(WorkspaceError::new(
                            "state-seed scenario anchor must match scenario state_seed_id",
                        ));
                    }
                }
                if let ScenarioAnchor::AuthenticatedCheckpoint { checkpoint_sha256 } =
                    scenario.anchor
                    && checkpoint_sha256 == Digest::ZERO
                {
                    return Err(WorkspaceError::new(
                        "authenticated checkpoint digest must be nonzero",
                    ));
                }
                if let ScenarioAnchor::EntryContract { predicate } = &scenario.anchor {
                    predicate.validate()?;
                }
            }
            WorkspaceAssetPayload::RouteGraph { graph }
            | WorkspaceAssetPayload::ReusableSubgraph { graph } => graph.validate()?,
            WorkspaceAssetPayload::CustomNodeDefinition(node) => {
                node.guard.validate()?;
                validate_pins("custom node inputs", &node.inputs)?;
                validate_pins("custom node outputs", &node.outputs)?;
                let mut evidence_ids = BTreeSet::new();
                for evidence in &node.evidence {
                    validate_stable_id("custom node evidence id", &evidence.id)?;
                    validate_label("custom node evidence source", &evidence.source)?;
                    validate_label("custom node evidence note", &evidence.note)?;
                    if !evidence_ids.insert(&evidence.id) {
                        return Err(WorkspaceError::new(format!(
                            "custom node evidence contains duplicate {}",
                            evidence.id
                        )));
                    }
                }
                if node.evidence_status == CustomNodeEvidenceStatus::Established
                    && node.evidence.is_empty()
                {
                    return Err(WorkspaceError::new(
                        "an established custom node requires explicit evidence",
                    ));
                }
            }
            WorkspaceAssetPayload::StateSeed { state } => state.validate()?,
            WorkspaceAssetPayload::QueryGoal(goal) => goal.predicate.validate()?,
            WorkspaceAssetPayload::RouteBook { route_book } => route_book.validate()?,
            WorkspaceAssetPayload::Layout(layout) => {
                validate_stable_id("layout semantic asset id", &layout.semantic_asset_id)?;
                if layout
                    .positions
                    .values()
                    .any(|point| !point.x.is_finite() || !point.y.is_finite())
                {
                    return Err(WorkspaceError::new("layout positions must be finite"));
                }
                if let Some(viewport) = layout.viewport
                    && (!viewport.x.is_finite()
                        || !viewport.y.is_finite()
                        || !viewport.zoom.is_finite()
                        || viewport.zoom <= 0.0)
                {
                    return Err(WorkspaceError::new(
                        "layout viewport must be finite with positive zoom",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorkspaceError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, WorkspaceError> {
        let digest = Sha256::digest(self.canonical_bytes()?);
        Ok(Digest(digest.into()))
    }
}

impl WorkspaceFolder {
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.schema != WORKSPACE_FOLDER_SCHEMA {
            return Err(WorkspaceError::new(format!(
                "folder {} uses unsupported schema {}",
                self.id, self.schema
            )));
        }
        validate_stable_id("folder id", &self.id)?;
        validate_label("folder label", &self.label)?;
        if self.parent_id.as_ref() == Some(&self.id) {
            return Err(WorkspaceError::new("folder cannot be its own parent"));
        }
        if let Some(parent_id) = &self.parent_id {
            validate_stable_id("folder parent id", parent_id)?;
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorkspaceError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, WorkspaceError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}
