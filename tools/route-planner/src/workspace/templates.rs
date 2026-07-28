//! Import and fork project templates, blank scenarios, and libraries.

use super::*;

impl WorkspaceStore {
    pub(super) fn import_project_template(
        &mut self,
        project: &PlannerWebProject,
        library_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let fragment = stable_fragment(&project.id);
        self.import_project_template_as(project, library_sha256, &fragment, None)
    }

    pub(super) fn fork_project_template(
        &mut self,
        project: &PlannerWebProject,
        library_sha256: Digest,
        namespace: &str,
    ) -> Result<(), WorkspaceError> {
        validate_stable_id("Library fork namespace", namespace)?;
        let fragment = stable_fragment(namespace);
        if fragment.is_empty() {
            return Err(WorkspaceError::new(
                "Library fork namespace must contain letters or digits",
            ));
        }
        self.import_project_template_as(project, library_sha256, &fragment, None)
    }

    pub(super) fn import_blank_scenario(
        &mut self,
        project: &PlannerWebProject,
        library_sha256: Digest,
        namespace: &str,
        label: &str,
        goal_id: &str,
    ) -> Result<(), WorkspaceError> {
        validate_stable_id("scenario namespace", namespace)?;
        validate_label("scenario label", label)?;
        validate_stable_id("scenario goal", goal_id)?;
        let fragment = stable_fragment(namespace);
        if fragment.is_empty() {
            return Err(WorkspaceError::new(
                "scenario namespace must contain letters or digits",
            ));
        }
        self.import_project_template_as(
            project,
            library_sha256,
            &fragment,
            Some(BlankScenarioConfiguration { label, goal_id }),
        )
    }

    pub(super) fn import_project_template_as(
        &mut self,
        project: &PlannerWebProject,
        library_sha256: Digest,
        fragment: &str,
        blank: Option<BlankScenarioConfiguration<'_>>,
    ) -> Result<(), WorkspaceError> {
        project
            .validate()
            .map_err(|error| WorkspaceError::new(error.to_string()))?;
        if library_sha256 == Digest::ZERO {
            return Err(WorkspaceError::new("library digest must be nonzero"));
        }
        let state = project
            .start_state
            .clone()
            .ok_or_else(|| WorkspaceError::new("Library template has no grounded state seed"))?;
        let exact_context = state
            .snapshot
            .environment
            .runtime_configuration
            .exact_context()?;
        let scenario_id = format!("scenario.{fragment}");
        let graph_id = format!("route-graph.{fragment}");
        let state_id = format!("state-seed.{fragment}");
        let route_book_id = format!("route-book.{fragment}");
        let layout_id = format!("layout.{fragment}");
        let scenario_label = blank
            .as_ref()
            .map_or(project.label.as_str(), |configuration| configuration.label);
        let origin = |source_asset_id: String| {
            Some(WorkspaceAssetOrigin {
                library_id: project.id.clone(),
                library_version: BUILTIN_LIBRARY_VERSION.into(),
                library_sha256,
                source_asset_id,
            })
        };
        let route_book = match (&blank, &project.route_book) {
            (None, Some(route_book)) => route_book.clone(),
            _ => {
                let scope = ContextScope {
                    selectors: vec![ContextSelector::Exact {
                        context: exact_context.clone(),
                    }],
                };
                let selected_goal = match &blank {
                    Some(configuration) => project
                        .catalog
                        .mechanics
                        .goals
                        .iter()
                        .find(|goal| goal.id == configuration.goal_id)
                        .ok_or_else(|| {
                            WorkspaceError::new(format!(
                                "goal {} is not defined by exact Library {}",
                                configuration.goal_id, project.id
                            ))
                        })?,
                    None => project.catalog.mechanics.goals.first().ok_or_else(|| {
                        WorkspaceError::new(
                            "Library template needs a goal before authoring a route",
                        )
                    })?,
                };
                RouteBook {
                    schema: ROUTE_BOOK_SCHEMA.into(),
                    manifest: RouteBookManifest {
                        id: route_book_id.clone(),
                        version: "1.0.0".into(),
                        label: format!("{scenario_label} authored route"),
                        author: "Route Planner".into(),
                        source: "Workspace-authored exact transition sequence".into(),
                        scope: scope.clone(),
                        refinement_stack_sha256: Some(project.catalog.refinement_stack.digest()?),
                    },
                    goal_ids: vec![selected_goal.id.clone()],
                    constraints: Vec::new(),
                    directives: Vec::new(),
                    steps: Vec::new(),
                    methods: Vec::new(),
                    regions: Vec::new(),
                    annotations: Vec::new(),
                }
            }
        };
        route_book.validate_against_composed(&project.catalog)?;
        let graph = PlannerGraph::project_composed_with_route_book(&project.catalog, &route_book)?;
        let state_asset = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: state_id.clone(),
                label: format!("{scenario_label} start state"),
                kind: WorkspaceAssetKind::StateSeed,
                version: 1,
                origin: origin(format!("{}:start-state", project.id)),
            },
            references: Vec::new(),
            payload: WorkspaceAssetPayload::StateSeed { state },
        };
        let graph_references = vec![WorkspaceAssetReference {
            asset_id: route_book_id.clone(),
            kind: WorkspaceAssetKind::RouteBook,
        }];
        let graph_asset = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: graph_id.clone(),
                label: scenario_label.into(),
                kind: WorkspaceAssetKind::RouteGraph,
                version: 1,
                origin: origin(format!("{}:graph", project.id)),
            },
            references: graph_references,
            payload: WorkspaceAssetPayload::RouteGraph { graph },
        };
        let layout_asset = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: layout_id.clone(),
                label: format!("{scenario_label} layout"),
                kind: WorkspaceAssetKind::Layout,
                version: 1,
                origin: origin(format!("{}:presentation", project.id)),
            },
            references: vec![WorkspaceAssetReference {
                asset_id: graph_id.clone(),
                kind: WorkspaceAssetKind::RouteGraph,
            }],
            payload: WorkspaceAssetPayload::Layout(LayoutAsset {
                semantic_asset_id: graph_id.clone(),
                positions: project
                    .presentation
                    .positions
                    .iter()
                    .map(|(id, position)| {
                        (
                            id.clone(),
                            LayoutPoint {
                                x: position.x,
                                y: position.y,
                            },
                        )
                    })
                    .collect(),
                viewport: None,
            }),
        };
        let mut scenario_references = vec![
            WorkspaceAssetReference {
                asset_id: graph_id.clone(),
                kind: WorkspaceAssetKind::RouteGraph,
            },
            WorkspaceAssetReference {
                asset_id: state_id.clone(),
                kind: WorkspaceAssetKind::StateSeed,
            },
        ];
        scenario_references.push(WorkspaceAssetReference {
            asset_id: route_book_id.clone(),
            kind: WorkspaceAssetKind::RouteBook,
        });
        scenario_references.sort();
        let scenario_asset = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: scenario_id,
                label: scenario_label.into(),
                kind: WorkspaceAssetKind::Scenario,
                version: 1,
                origin: origin(format!("{}:scenario", project.id)),
            },
            references: scenario_references,
            payload: WorkspaceAssetPayload::Scenario(ScenarioAsset {
                exact_context: exact_context.clone(),
                anchor: ScenarioAnchor::StateSeed {
                    state_seed_id: state_id.clone(),
                },
                route_graph_id: graph_id,
                state_seed_id: Some(state_id),
                route_book_id: Some(route_book_id.clone()),
            }),
        };
        let mut mutations = vec![
            WorkspaceMutation::Put {
                relative_path: Path::new("scenarios").join(format!("{fragment}.json")),
                expected_revision_sha256: None,
                asset: scenario_asset,
            },
            WorkspaceMutation::Put {
                relative_path: Path::new("route-graphs").join(format!("{fragment}.json")),
                expected_revision_sha256: None,
                asset: graph_asset,
            },
            WorkspaceMutation::Put {
                relative_path: Path::new("state-seeds").join(format!("{fragment}.json")),
                expected_revision_sha256: None,
                asset: state_asset,
            },
            WorkspaceMutation::Put {
                relative_path: Path::new("layouts").join(format!("{fragment}.json")),
                expected_revision_sha256: None,
                asset: layout_asset,
            },
        ];
        mutations.push(WorkspaceMutation::Put {
            relative_path: Path::new("route-books").join(format!("{fragment}.json")),
            expected_revision_sha256: None,
            asset: WorkspaceAsset {
                schema: WORKSPACE_ASSET_SCHEMA.into(),
                header: WorkspaceAssetHeader {
                    id: route_book_id,
                    label: route_book.manifest.label.clone(),
                    kind: WorkspaceAssetKind::RouteBook,
                    version: 1,
                    origin: origin(format!("{}:route-book", project.id)),
                },
                references: Vec::new(),
                payload: WorkspaceAssetPayload::RouteBook { route_book },
            },
        });
        self.transact(&mutations)?;
        self.mount_library(
            MountedLibrary {
                id: project.id.clone(),
                version: BUILTIN_LIBRARY_VERSION.into(),
                sha256: library_sha256,
                source: format!("builtin:{}", project.id),
            },
            exact_context,
        )
    }

    pub(super) fn mount_library(
        &mut self,
        library: MountedLibrary,
        exact_context: ExactContext,
    ) -> Result<(), WorkspaceError> {
        match self
            .manifest
            .mounted_libraries
            .iter()
            .find(|mounted| mounted.id == library.id && mounted.version == library.version)
        {
            Some(mounted) if mounted.sha256 == library.sha256 => {}
            Some(mounted) => {
                return Err(WorkspaceError::new(format!(
                    "mounted Library {} changed from {} to {}; explicit upgrade is required",
                    library.id, mounted.sha256, library.sha256
                )));
            }
            None => self.manifest.mounted_libraries.push(library),
        }
        self.manifest.mounted_libraries.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.version.cmp(&right.version))
        });
        if !self
            .manifest
            .exact_context_defaults
            .contains(&exact_context)
        {
            self.manifest.exact_context_defaults.push(exact_context);
            self.manifest.exact_context_defaults.sort();
        }
        self.manifest.validate()?;
        write_atomically(
            &self.root.join(MANIFEST_FILE),
            &self.manifest.canonical_bytes()?,
        )
    }
}
