const SERVICE_SCHEMA = "dusklight.route-planner.service/v46";
const PROJECT_SCHEMA = "dusklight.route-planner.web-project/v3";
const LEGACY_PROJECT_SCHEMAS = new Set([
  "dusklight.route-planner.web-project/v1",
  "dusklight.route-planner.web-project/v2",
]);
const PROJECT_SAVE_SCHEMA = "dusklight.route-planner.web-project-save/v1";
const WORKSPACE_CREATE_SCHEMA = "dusklight.route-planner.workspace-create/v1";
const WORKSPACE_ASSET_SCHEMA = "dusklight.route-planner.workspace-asset/v1";
const WORKSPACE_ASSET_SAVE_SCHEMA = "dusklight.route-planner.workspace-asset-save/v1";
const WORKSPACE_ASSET_COMMAND_SCHEMA = "dusklight.route-planner.workspace-asset-command/v1";
const WORKSPACE_TRASH_COMMAND_SCHEMA = "dusklight.route-planner.workspace-trash-command/v1";
const WORKSPACE_LIBRARY_FORK_SCHEMA = "dusklight.route-planner.workspace-library-fork/v1";
const WORKSPACE_EXPORT_SCHEMA = "dusklight.route-planner.workspace-export/v1";
const LIBRARY_DRAG_TYPE = "application/x-dusklight-library";
const ROUTE_BOOK_EDIT_BATCH_SCHEMA = "dusklight.route-planner.route-book-edit-batch/v7";
const NODE_WIDTH = 176;
const NODE_HEIGHT = 52;

const elements = Object.fromEntries([
  "workspace-list", "new-workspace", "workspace-tab", "library-tab", "content-search",
  "content-browser-list", "new-workspace-dialog", "new-workspace-form",
  "new-workspace-label", "new-workspace-id", "new-workspace-error", "cancel-new-workspace",
  "new-asset", "new-asset-dialog", "new-asset-form", "new-asset-label", "new-asset-id",
  "new-asset-error", "cancel-new-asset", "import-asset", "workspace-asset-file",
  "import-workspace", "export-workspace", "workspace-file",
  "add-node-menu", "add-node-search", "add-node-results",
  "project-list", "new-project", "open-project", "save-project", "save-as-project",
  "export-project", "project-file", "project-name", "status", "search", "palette-list",
  "canvas-shell", "canvas", "viewport", "edges", "nodes", "empty-state", "zoom-in",
  "zoom-out", "fit", "detail-title", "detail-subtitle", "detail-json", "state-inspector",
  "contract-inspector", "workspace-asset-editor",
  "model-context", "model-context-body",
  "region-nav", "region-breadcrumbs", "region-children",
  "evaluate-transition", "solve-goal", "insert-transition", "suggest-transition-chain", "replace-step", "remove-step",
  "group-selection", "copy-region", "fork-region", "reference-region", "version-region",
  "replace-region", "region-usage", "pin-selection", "ban-selection", "prefer-selection", "select-method",
].map((id) => [id, document.getElementById(id)]));

const state = {
  workspace: null,
  workspaceSignature: null,
  workspacePollActive: false,
  workspaceList: [],
  trash: [],
  libraries: [],
  contentSource: "workspace",
  selectedWorkspaceAsset: null,
  project: null,
  graph: null,
  positions: new Map(),
  selected: null,
  transform: { x: 70, y: 60, scale: 1 },
  gesture: null,
  revision: null,
  readOnly: false,
  dirty: false,
  transitionEvaluation: null,
  replacementStep: null,
  transitionSearch: new Map(),
  activeRegionId: null,
  collapsedRegionIds: new Set(),
  knownRegionIds: new Set(),
  routeStepInspections: new Map(),
  executionStateInspections: new Map(),
  routeFrontier: null,
  selectedStateFeasibility: null,
  solveReport: null,
  groupSelection: new Set(),
};

elements["workspace-list"].addEventListener("change", () => {
  const id = elements["workspace-list"].value;
  if (id) loadWorkspace(id);
});
elements["new-workspace"].addEventListener("click", () => {
  elements["new-workspace-error"].textContent = "";
  elements["new-workspace-dialog"].showModal();
  elements["new-workspace-label"].focus();
});
elements["cancel-new-workspace"].addEventListener("click", () => {
  elements["new-workspace-dialog"].close();
});
elements["new-workspace-form"].addEventListener("submit", createWorkspace);
elements["import-workspace"].addEventListener("click", () => elements["workspace-file"].click());
elements["export-workspace"].addEventListener("click", exportWorkspace);
elements["workspace-file"].addEventListener("change", importWorkspace);
elements["new-asset"].addEventListener("click", () => {
  elements["new-asset-error"].textContent = "";
  elements["new-asset-dialog"].showModal();
  elements["new-asset-label"].focus();
});
elements["cancel-new-asset"].addEventListener("click", () => {
  elements["new-asset-dialog"].close();
});
elements["new-asset-form"].addEventListener("submit", createCustomNodeAsset);
elements["import-asset"].addEventListener("click", () => elements["workspace-asset-file"].click());
elements["workspace-asset-file"].addEventListener("change", importWorkspaceAsset);
elements["workspace-tab"].addEventListener("click", () => selectContentSource("workspace"));
elements["library-tab"].addEventListener("click", () => selectContentSource("library"));
elements["content-search"].addEventListener("input", renderContentBrowser);
elements["project-list"].addEventListener("change", () => {
  const id = elements["project-list"].value;
  if (id) loadStoredProject(id);
});
elements["new-project"].addEventListener("click", newProject);
elements["open-project"].addEventListener("click", () => elements["project-file"].click());
elements["project-file"].addEventListener("change", importProject);
elements["save-project"].addEventListener("click", saveProject);
elements["save-as-project"].addEventListener("click", saveProjectAs);
elements["export-project"].addEventListener("click", exportProject);
elements.search.addEventListener("input", () => renderPalette());
elements["zoom-in"].addEventListener("click", () => zoomAt(1.2));
elements["zoom-out"].addEventListener("click", () => zoomAt(1 / 1.2));
elements.fit.addEventListener("click", fitGraph);
elements["evaluate-transition"].addEventListener("click", evaluateSelectedTransition);
elements["solve-goal"].addEventListener("click", solveSelectedGoal);
elements["insert-transition"].addEventListener("click", insertSelectedTransition);
elements["suggest-transition-chain"].addEventListener("click", suggestOrInsertSelectedTransitionChain);
elements["replace-step"].addEventListener("click", replaceSelectedRouteStep);
elements["remove-step"].addEventListener("click", removeSelectedRouteStep);
elements["group-selection"].addEventListener("click", groupSelectedNodes);
elements["copy-region"].addEventListener("click", () => createRegionDerivative("copy"));
elements["fork-region"].addEventListener("click", () => createRegionDerivative("fork"));
elements["reference-region"].addEventListener("click", () => createRegionDerivative("reference"));
elements["version-region"].addEventListener("click", () => createRegionDerivative("version"));
elements["replace-region"].addEventListener("click", replaceRegionFromSelection);
elements["region-usage"].addEventListener("click", inspectSelectedRegionUsage);
elements["pin-selection"].addEventListener("click", () => editSelectedDirective("pin"));
elements["ban-selection"].addEventListener("click", () => editSelectedDirective("ban"));
elements["prefer-selection"].addEventListener("click", () => editSelectedDirective("prefer"));
elements["select-method"].addEventListener("click", toggleSelectedMethod);
elements.canvas.addEventListener("wheel", onWheel, { passive: false });
elements.canvas.addEventListener("pointerdown", beginPan);
elements.canvas.addEventListener("contextmenu", openAddNodeMenu);
elements.canvas.addEventListener("dragover", allowTransitionDrop);
elements.canvas.addEventListener("drop", dropTransitionAtRouteFrontier);
window.addEventListener("pointermove", moveGesture);
window.addEventListener("pointerup", endGesture);
window.addEventListener("pointerdown", (event) => {
  if (!elements["add-node-menu"].hidden && !elements["add-node-menu"].contains(event.target)) {
    closeAddNodeMenu();
  }
});
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeAddNodeMenu();
});
elements["add-node-search"].addEventListener("input", renderAddNodeMenu);
window.addEventListener("beforeunload", (event) => {
  if (!state.dirty) return;
  event.preventDefault();
  event.returnValue = "";
});

applyTransform();
start();
window.setInterval(pollWorkspaceChanges, 3000);

async function start() {
  try {
    const response = await fetch("/api/health", { cache: "no-store" });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    setStatus("Planner service ready", "good");
    await Promise.all([refreshWorkspaces(), refreshLibraries()]);
    await refreshProjects(false);
  } catch (error) {
    setStatus(`Planner service unavailable: ${error.message}`, "bad");
  }
}

async function refreshWorkspaces(selectedId = null) {
  const list = await projectApi("/api/workspaces");
  state.workspaceList = list.workspaces ?? [];
  elements["workspace-list"].replaceChildren(new Option("Workspaces", ""));
  for (const workspace of state.workspaceList) {
    const suffix = workspace.dependency_error ? " — needs libraries" : "";
    elements["workspace-list"].append(new Option(`${workspace.label}${suffix}`, workspace.id));
  }
  const activeId = selectedId ?? state.workspace?.manifest?.id;
  if (activeId && state.workspaceList.some((workspace) => workspace.id === activeId)) {
    elements["workspace-list"].value = activeId;
  }
  renderContentBrowser();
}

async function refreshLibraries() {
  const list = await projectApi("/api/libraries");
  state.libraries = list.libraries ?? [];
  renderContentBrowser();
}

async function createWorkspace(event) {
  event.preventDefault();
  const label = elements["new-workspace-label"].value.trim();
  const id = elements["new-workspace-id"].value.trim();
  elements["new-workspace-error"].textContent = "";
  try {
    const record = await projectApi("/api/workspaces", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ schema: WORKSPACE_CREATE_SCHEMA, id, label }),
    });
    elements["new-workspace-dialog"].close();
    await openWorkspaceRecord(record);
    await refreshWorkspaces(id);
    setStatus("Workspace created", "good");
  } catch (error) {
    elements["new-workspace-error"].textContent = error.message;
  }
}

async function exportWorkspace() {
  if (!state.workspace) return;
  try {
    const workspaceId = state.workspace.manifest.id;
    const bundle = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/export`,
    );
    downloadJson(bundle, `${slug(state.workspace.manifest.label)}.workspace.json`);
    setStatus("Workspace exported with all assets and exact Library pins", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function importWorkspace(event) {
  const [file] = event.target.files;
  event.target.value = "";
  if (!file) return;
  if (state.dirty && !confirm("Discard unsaved planner changes and import this workspace?")) return;
  try {
    const bundle = JSON.parse(await file.text());
    if (bundle?.schema !== WORKSPACE_EXPORT_SCHEMA || !bundle.manifest || !Array.isArray(bundle.assets)) {
      throw new Error("Choose a Dusklight workspace export");
    }
    const id = prompt("Imported workspace folder ID", `${bundle.manifest.id}-imported`);
    if (id == null) return;
    const label = prompt("Imported workspace name", `${bundle.manifest.label} imported`);
    if (label == null) return;
    bundle.manifest.id = id.trim();
    bundle.manifest.label = label.trim();
    const record = await projectApi("/api/workspaces/import", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(bundle),
    });
    await openWorkspaceRecord(record);
    await refreshWorkspaces(record.manifest.id);
    elements["workspace-list"].value = record.manifest.id;
    setStatus(`Imported ${record.assets.length} workspace assets`, "good");
  } catch (error) {
    setStatus(`Workspace import failed: ${error.message}`, "bad");
  }
}

async function createCustomNodeAsset(event) {
  event.preventDefault();
  if (!state.workspace) return;
  const workspaceId = state.workspace.manifest.id;
  const label = elements["new-asset-label"].value.trim();
  const id = elements["new-asset-id"].value.trim();
  const asset = {
    schema: WORKSPACE_ASSET_SCHEMA,
    header: {
      id,
      label,
      kind: "custom_node_definition",
      version: 1,
    },
    references: [],
    payload: {
      kind: "custom_node_definition",
      inputs: [],
      outputs: [],
      guard: { kind: "true" },
      effects: [],
      evidence_status: "hypothetical",
      evidence: [],
    },
  };
  elements["new-asset-error"].textContent = "";
  try {
    const record = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/assets/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          schema: WORKSPACE_ASSET_SAVE_SCHEMA,
          relative_path: `custom-nodes/${slug(id)}.json`,
          expected_revision_sha256: null,
          asset,
        }),
      },
    );
    elements["new-asset-dialog"].close();
    await loadWorkspace(workspaceId);
    inspectWorkspaceAsset({
      ...record.asset.header,
      kind: record.asset.header.kind,
      relative_path: record.relative_path,
      revision_sha256: record.revision_sha256,
    });
    setStatus("Hypothetical custom node created", "good");
  } catch (error) {
    elements["new-asset-error"].textContent = error.message;
  }
}

async function importWorkspaceAsset(event) {
  const [file] = event.target.files;
  event.target.value = "";
  if (!file || !state.workspace) return;
  try {
    const exported = JSON.parse(await file.text());
    const asset = exported?.asset ?? exported;
    if (asset?.schema !== WORKSPACE_ASSET_SCHEMA || !asset.header?.id || !asset.header?.kind) {
      throw new Error("Choose a Dusklight workspace asset export");
    }
    if (state.workspace.assets.some((listing) => listing.id === asset.header.id)) {
      throw new Error(
        `Stable asset ID ${asset.header.id} already exists; move or duplicate the existing asset first`,
      );
    }
    const suggestedPath = exported?.relative_path
      ?? `${workspaceAssetRoot(asset.header.kind)}/${slug(asset.header.id)}.json`;
    const relativePath = prompt("Workspace-relative destination", suggestedPath);
    if (relativePath == null) return;
    const workspaceId = state.workspace.manifest.id;
    const record = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/assets/${encodeURIComponent(asset.header.id)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          schema: WORKSPACE_ASSET_SAVE_SCHEMA,
          relative_path: relativePath.trim(),
          expected_revision_sha256: null,
          asset,
        }),
      },
    );
    await loadWorkspace(workspaceId);
    await inspectWorkspaceAsset({
      ...record.asset.header,
      relative_path: record.relative_path,
      revision_sha256: record.revision_sha256,
    });
    setStatus("Asset imported with its stable identity and references", "good");
  } catch (error) {
    setStatus(`Asset import failed: ${error.message}`, "bad");
  }
}

async function loadWorkspace(id) {
  if (state.dirty && !confirm("Discard unsaved planner changes?")) {
    elements["workspace-list"].value = state.workspace?.manifest?.id ?? "";
    return;
  }
  try {
    const record = await projectApi(`/api/workspaces/${encodeURIComponent(id)}`);
    await openWorkspaceRecord(record);
    elements["workspace-list"].value = id;
    setStatus(`${record.assets.length} workspace assets`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function openWorkspaceRecord(record) {
  state.workspace = record;
  state.workspaceSignature = workspaceSignature(record);
  await refreshTrash();
  state.project = null;
  state.graph = null;
  state.revision = null;
  state.readOnly = false;
  state.dirty = false;
  state.selected = null;
  state.selectedWorkspaceAsset = null;
  state.positions = new Map();
  state.contentSource = "workspace";
  elements.nodes.replaceChildren();
  elements.edges.replaceChildren();
  elements["region-nav"].hidden = true;
  elements["empty-state"].hidden = false;
  elements["empty-state"].querySelector("strong").textContent = record.assets.length
    ? "Open a route graph"
    : "Create a grounded scenario";
  elements["empty-state"].querySelector("span").textContent = record.assets.length
    ? "Choose an asset in the Content Browser."
    : "Mount an exact Library, then create a Scenario Root.";
  elements["detail-title"].textContent = "Nothing selected";
  elements["detail-subtitle"].textContent = "Choose an asset, node, or connection to inspect it.";
  elements["detail-json"].textContent = "{}";
  elements["workspace-asset-editor"].hidden = true;
  elements["workspace-asset-editor"].replaceChildren();
  updateProjectControls();
  selectContentSource("workspace");
}

async function refreshTrash() {
  if (!state.workspace) {
    state.trash = [];
    return;
  }
  state.trash = await projectApi(
    `/api/workspaces/${encodeURIComponent(state.workspace.manifest.id)}/trash`,
  );
}

function workspaceSignature(record) {
  return JSON.stringify((record?.assets ?? []).map((asset) => [
    asset.id,
    asset.relative_path,
    asset.revision_sha256,
  ]).sort(([left], [right]) => left.localeCompare(right)));
}

async function pollWorkspaceChanges() {
  if (!state.workspace || state.workspacePollActive) return;
  state.workspacePollActive = true;
  try {
    const workspaceId = state.workspace.manifest.id;
    const fresh = await projectApi(`/api/workspaces/${encodeURIComponent(workspaceId)}`);
    const signature = workspaceSignature(fresh);
    if (signature === state.workspaceSignature) return;
    const selectedId = state.selectedWorkspaceAsset?.asset?.header?.id;
    state.workspace = fresh;
    state.workspaceSignature = signature;
    await refreshTrash();
    renderContentBrowser();
    if (selectedId && fresh.assets.some((asset) => asset.id === selectedId)) {
      await inspectWorkspaceAsset(fresh.assets.find((asset) => asset.id === selectedId));
    } else if (selectedId) {
      state.selectedWorkspaceAsset = null;
      elements["workspace-asset-editor"].hidden = true;
      setStatus("The selected asset changed or was removed on disk", "bad");
      return;
    }
    setStatus("Workspace changes from disk were reloaded", "good");
  } catch (error) {
    setStatus(`Workspace refresh failed: ${error.message}`, "bad");
  } finally {
    state.workspacePollActive = false;
  }
}

function selectContentSource(source) {
  state.contentSource = source;
  for (const name of ["workspace", "library"]) {
    const active = source === name;
    elements[`${name}-tab`].classList.toggle("active", active);
    elements[`${name}-tab`].setAttribute("aria-selected", String(active));
  }
  elements["new-asset"].disabled = source !== "workspace" || !state.workspace;
  elements["import-asset"].disabled = source !== "workspace" || !state.workspace;
  renderContentBrowser();
}

function renderContentBrowser() {
  const list = elements["content-browser-list"];
  const query = elements["content-search"].value.trim().toLowerCase();
  list.replaceChildren();
  if (state.contentSource === "library") {
    const libraries = state.libraries.filter((library) =>
      `${library.label} ${library.id}`.toLowerCase().includes(query));
    if (!libraries.length) {
      list.append(contentMessage(query ? "No matching Library content." : "No libraries are installed."));
      return;
    }
    for (const library of libraries) {
      list.append(libraryContentItem(library));
    }
    return;
  }
  if (!state.workspace) {
    list.append(contentMessage("Create or open a workspace."));
    return;
  }
  const dependencyError = state.workspaceList.find(
    (workspace) => workspace.id === state.workspace.manifest.id,
  )?.dependency_error;
  if (dependencyError) {
    const error = document.createElement("p");
    error.className = "content-error";
    error.textContent = dependencyError;
    list.append(error);
    return;
  }
  const labels = {
    scenario: "Scenarios",
    route_graph: "Route graphs",
    reusable_subgraph: "Subgraphs",
    custom_node_definition: "Custom nodes",
    state_seed: "State seeds",
    query_goal: "Queries & goals",
    route_book: "Route books",
    layout: "Layouts",
  };
  for (const [kind, label] of Object.entries(labels)) {
    const assets = state.workspace.assets.filter((asset) =>
      asset.kind === kind && `${asset.label} ${asset.id}`.toLowerCase().includes(query));
    if (query && !assets.length) continue;
    const group = document.createElement("details");
    group.className = "content-group";
    group.open = assets.length > 0;
    const summary = document.createElement("summary");
    summary.textContent = `${label}  ${assets.length}`;
    group.append(summary);
    for (const asset of assets) {
      group.append(contentAssetItem(asset, kind === "layout" ? "LAY" : "AST"));
    }
    list.append(group);
  }
  if (!query || "trash".includes(query)) {
    const trashGroup = document.createElement("details");
    trashGroup.className = "content-group";
    const summary = document.createElement("summary");
    summary.textContent = `Trash  ${state.trash.length}`;
    trashGroup.append(summary);
    for (const asset of state.trash) trashGroup.append(trashAssetItem(asset));
    list.append(trashGroup);
  }
  if (query && !list.childElementCount) list.append(contentMessage("No matching workspace assets."));
}

function libraryContentItem(library) {
  const row = document.createElement("div");
  row.className = "content-asset-row";
  const libraryButton = contentItem("LIB", library.label, "Read-only verified example", true, () => {
    loadStoredProject(library.id);
  });
  libraryButton.draggable = true;
  libraryButton.addEventListener("dragstart", (event) => {
    event.dataTransfer.effectAllowed = "link";
    event.dataTransfer.setData(LIBRARY_DRAG_TYPE, library.id);
    setStatus(`Drop ${library.label} on the canvas to add an exact reference`);
  });
  row.append(libraryButton);
  const actions = document.createElement("details");
  actions.className = "asset-actions";
  const summary = document.createElement("summary");
  summary.textContent = "⋯";
  summary.setAttribute("aria-label", `Actions for ${library.label}`);
  actions.append(summary);
  for (const [label, command, requiresWorkspace] of [
    ["Open", () => loadStoredProject(library.id), false],
    ["Inspect", () => inspectLibrary(library), false],
    ["Add Reference", () => addLibraryReference(library), true],
    ["Create Scenario from Template", () => createScenarioFromLibrary(library), true],
    ["Fork to Workspace", () => forkLibraryToWorkspace(library), true],
  ]) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.disabled = requiresWorkspace && !state.workspace;
    button.addEventListener("click", () => {
      actions.open = false;
      command();
    });
    actions.append(button);
  }
  row.append(actions);
  return row;
}

async function inspectLibrary(library) {
  try {
    const record = await projectApi(`/api/projects/${encodeURIComponent(library.id)}`);
    const project = record.project;
    const runtime = project.start_state?.snapshot?.environment?.runtime_configuration;
    const mechanics = project.catalog?.mechanics ?? {};
    elements["detail-title"].textContent = project.label;
    elements["detail-subtitle"].textContent = "Read-only Library content";
    elements["workspace-asset-editor"].hidden = true;
    elements["workspace-asset-editor"].replaceChildren();
    elements["contract-inspector"].hidden = true;
    const inspector = elements["state-inspector"];
    inspector.hidden = false;
    inspector.replaceChildren();
    const heading = document.createElement("h3");
    heading.textContent = "Library summary";
    const card = document.createElement("section");
    card.className = "state-card";
    const title = document.createElement("h4");
    title.textContent = "Exact read-only source";
    const metrics = document.createElement("dl");
    for (const [name, value] of [
      ["Source", "Library"],
      ["Version", "1"],
      ["Content", runtime?.content_sha256 ?? "No exact context"],
      ["Language", runtime?.language ?? "Unspecified"],
      ["Mechanics", mechanics.transitions?.length ?? 0],
      ["Goals", mechanics.goals?.length ?? 0],
      ["Readers", mechanics.readers?.length ?? 0],
    ]) {
      const term = document.createElement("dt");
      term.textContent = name;
      const detail = document.createElement("dd");
      detail.textContent = String(value);
      detail.title = String(value);
      metrics.append(term, detail);
    }
    card.append(title, metrics);
    inspector.append(heading, card);
    elements["detail-json"].textContent = JSON.stringify(record, null, 2);
    setStatus(`Inspecting ${library.label}`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function addLibraryReference(library) {
  if (!state.workspace) {
    setStatus("Create or open a workspace first", "bad");
    return;
  }
  try {
    const workspaceId = state.workspace.manifest.id;
    state.workspace = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/library-references/${encodeURIComponent(library.id)}`,
      { method: "POST" },
    );
    state.workspaceSignature = workspaceSignature(state.workspace);
    await refreshWorkspaces(workspaceId);
    renderContentBrowser();
    setStatus(`Referenced exact Library ${library.label}`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function forkLibraryToWorkspace(library) {
  if (!state.workspace) {
    setStatus("Create or open a workspace first", "bad");
    return;
  }
  const namespace = prompt("Fork namespace", `${slug(library.id)}-fork`);
  if (namespace == null || !namespace.trim()) return;
  try {
    const workspaceId = state.workspace.manifest.id;
    state.workspace = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/library-forks/${encodeURIComponent(library.id)}`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          schema: WORKSPACE_LIBRARY_FORK_SCHEMA,
          namespace: namespace.trim(),
        }),
      },
    );
    state.workspaceSignature = workspaceSignature(state.workspace);
    await refreshTrash();
    await refreshWorkspaces(workspaceId);
    selectContentSource("workspace");
    const graphId = `route-graph.${slug(namespace)}`;
    const graph = state.workspace.assets.find((asset) => asset.id === graphId);
    if (graph) await inspectWorkspaceAsset(graph);
    setStatus("Library content forked with exact provenance", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function createScenarioFromLibrary(library) {
  if (!state.workspace) {
    setStatus("Create or open a workspace first", "bad");
    return;
  }
  try {
    const workspaceId = state.workspace.manifest.id;
    setStatus(`Creating grounded scenario from ${library.label}...`);
    state.workspace = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/library-scenarios/${encodeURIComponent(library.id)}`,
      { method: "POST" },
    );
    state.workspaceSignature = workspaceSignature(state.workspace);
    await refreshTrash();
    await refreshWorkspaces(workspaceId);
    selectContentSource("workspace");
    const graph = state.workspace.assets.find((asset) => asset.kind === "route_graph"
      && asset.id.endsWith(slug(library.id)));
    if (graph) await inspectWorkspaceAsset(graph);
    setStatus("Grounded scenario created from exact Library content", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

function contentAssetItem(asset, iconText) {
  const row = document.createElement("div");
  row.className = "content-asset-row";
  row.append(contentItem(
    iconText,
    asset.label,
    asset.relative_path,
    false,
    () => inspectWorkspaceAsset(asset),
  ));
  const actions = document.createElement("details");
  actions.className = "asset-actions";
  const summary = document.createElement("summary");
  summary.setAttribute("aria-label", `Actions for ${asset.label}`);
  summary.textContent = "⋯";
  actions.append(summary);
  for (const [label, command] of [
    ["Rename", () => renameWorkspaceAsset(asset)],
    ["Move", () => moveWorkspaceAsset(asset)],
    ["Duplicate", () => duplicateWorkspaceAsset(asset)],
    ["Export", () => exportWorkspaceAsset(asset)],
    ["Delete to Trash", () => trashWorkspaceAsset(asset)],
  ]) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.addEventListener("click", () => {
      actions.open = false;
      command();
    });
    actions.append(button);
  }
  row.append(actions);
  return row;
}

async function exportWorkspaceAsset(asset) {
  try {
    const workspaceId = state.workspace.manifest.id;
    const record = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/assets/${encodeURIComponent(asset.id)}`,
    );
    downloadJson(record, `${slug(asset.label)}.asset.json`);
    setStatus("Asset exported with its stable identity and references", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

function trashAssetItem(asset) {
  const row = document.createElement("div");
  row.className = "content-asset-row";
  row.append(contentItem("BIN", asset.label, asset.original_relative_path, false, () => {
    elements["detail-title"].textContent = asset.label;
    elements["detail-subtitle"].textContent = `Deleted ${asset.kind.replaceAll("_", " ")}`;
    elements["detail-json"].textContent = JSON.stringify(asset, null, 2);
  }));
  const actions = document.createElement("details");
  actions.className = "asset-actions";
  const summary = document.createElement("summary");
  summary.textContent = "⋯";
  summary.setAttribute("aria-label", `Trash actions for ${asset.label}`);
  actions.append(summary);
  for (const [label, command] of [
    ["Restore", "restore"],
    ["Delete permanently", "permanently_delete"],
  ]) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.addEventListener("click", () => runTrashCommand(asset, command));
    actions.append(button);
  }
  row.append(actions);
  return row;
}

async function workspaceAssetCommand(asset, command) {
  const workspaceId = state.workspace.manifest.id;
  const record = await projectApi(
    `/api/workspaces/${encodeURIComponent(workspaceId)}/assets/${encodeURIComponent(asset.id)}`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ schema: WORKSPACE_ASSET_COMMAND_SCHEMA, command }),
    },
  );
  state.workspace = record;
  state.workspaceSignature = workspaceSignature(record);
  await refreshTrash();
  await refreshWorkspaces(workspaceId);
  renderContentBrowser();
}

async function renameWorkspaceAsset(asset) {
  const label = prompt("Asset name", asset.label);
  if (label == null || !label.trim() || label.trim() === asset.label) return;
  try {
    await workspaceAssetCommand(asset, {
      kind: "rename",
      expected_revision_sha256: asset.revision_sha256,
      label: label.trim(),
    });
    setStatus("Asset renamed; stable references preserved", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function moveWorkspaceAsset(asset) {
  const relativePath = prompt("Workspace-relative path", asset.relative_path);
  if (relativePath == null || relativePath === asset.relative_path) return;
  try {
    await workspaceAssetCommand(asset, {
      kind: "move",
      expected_revision_sha256: asset.revision_sha256,
      relative_path: relativePath,
    });
    setStatus("Asset moved; stable references preserved", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function duplicateWorkspaceAsset(asset) {
  const newId = prompt("New stable ID", `${asset.id}-copy`);
  if (newId == null) return;
  const newLabel = prompt("Copy name", `${asset.label} copy`);
  if (newLabel == null) return;
  const folder = String(asset.relative_path).split(/[\\/]/).slice(0, -1).join("/");
  const relativePath = prompt("Workspace-relative path", `${folder}/${slug(newId)}.json`);
  if (relativePath == null) return;
  try {
    await workspaceAssetCommand(asset, {
      kind: "duplicate",
      new_id: newId.trim(),
      new_label: newLabel.trim(),
      relative_path: relativePath,
    });
    setStatus("Asset duplicated with a new stable identity", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function trashWorkspaceAsset(asset) {
  if (!confirm(`Move “${asset.label}” to Trash?`)) return;
  const command = {
    kind: "delete_to_trash",
    expected_revision_sha256: asset.revision_sha256,
    allow_broken_references: false,
  };
  try {
    await workspaceAssetCommand(asset, command);
    setStatus("Asset moved to Trash", "good");
  } catch (error) {
    if (!error.message.includes("is referenced by")
      || !confirm(`${error.message}\n\nDelete anyway and keep broken references visible?`)) {
      setStatus(error.message, "bad");
      return;
    }
    try {
      await workspaceAssetCommand(asset, { ...command, allow_broken_references: true });
      setStatus("Asset moved to Trash; inbound references remain visibly broken", "good");
    } catch (confirmedError) {
      setStatus(confirmedError.message, "bad");
    }
  }
}

async function runTrashCommand(asset, command) {
  if (command === "permanently_delete"
    && !confirm(`Permanently delete “${asset.label}”? This cannot be undone.`)) return;
  try {
    const workspaceId = state.workspace.manifest.id;
    state.workspace = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/trash/${encodeURIComponent(asset.id)}`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          schema: WORKSPACE_TRASH_COMMAND_SCHEMA,
          expected_revision_sha256: asset.revision_sha256,
          command,
        }),
      },
    );
    state.workspaceSignature = workspaceSignature(state.workspace);
    await refreshTrash();
    await refreshWorkspaces(workspaceId);
    renderContentBrowser();
    setStatus(command === "restore" ? "Asset restored" : "Asset permanently deleted", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

function contentItem(iconText, label, subtitle, readOnly, action) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = `content-item${readOnly ? " read-only" : ""}`;
  const icon = document.createElement("span");
  icon.className = "asset-icon";
  icon.textContent = iconText;
  const strong = document.createElement("strong");
  strong.textContent = label;
  const small = document.createElement("small");
  small.textContent = subtitle;
  button.append(icon, strong, small);
  button.addEventListener("click", action);
  return button;
}

function contentMessage(message) {
  const paragraph = document.createElement("p");
  paragraph.className = "context-empty";
  paragraph.textContent = message;
  return paragraph;
}

async function inspectWorkspaceAsset(asset) {
  elements["detail-title"].textContent = asset.label;
  elements["detail-subtitle"].textContent = `${asset.kind.replaceAll("_", " ")} · ${asset.relative_path}`;
  elements["detail-json"].textContent = JSON.stringify(asset, null, 2);
  try {
    const workspaceId = state.workspace.manifest.id;
    const record = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/assets/${encodeURIComponent(asset.id)}`,
    );
    state.selectedWorkspaceAsset = record;
    elements["detail-json"].textContent = JSON.stringify(record.asset, null, 2);
    renderWorkspaceAssetEditor(record);
    if (record.asset.header.kind === "route_graph") await openWorkspaceGraph(record);
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function openWorkspaceGraph(record) {
  state.project = null;
  state.graph = record.asset.payload.graph;
  state.selected = null;
  state.positions = new Map();
  for (const listing of state.workspace.assets.filter((asset) => asset.kind === "layout")) {
    const layout = await projectApi(
      `/api/workspaces/${encodeURIComponent(state.workspace.manifest.id)}/assets/${encodeURIComponent(listing.id)}`,
    );
    if (layout.asset.payload.semantic_asset_id === record.asset.header.id) {
      state.positions = new Map(Object.entries(layout.asset.payload.positions ?? {}));
      break;
    }
  }
  state.transitionSearch = new Map();
  state.activeRegionId = null;
  state.collapsedRegionIds = new Set();
  state.knownRegionIds = new Set();
  ensurePositions();
  elements["empty-state"].hidden = true;
  render();
  requestAnimationFrame(fitGraph);
  elements["project-name"].textContent = `${state.workspace.manifest.label} · ${record.asset.header.label}`;
}

function renderWorkspaceAssetEditor(record) {
  const form = elements["workspace-asset-editor"];
  form.replaceChildren();
  if (record.asset.header.kind !== "custom_node_definition") {
    form.hidden = true;
    return;
  }
  form.hidden = false;
  const node = record.asset.payload;
  const status = document.createElement("span");
  status.className = `evidence-badge ${node.evidence_status}`;
  status.textContent = node.evidence_status;
  const label = document.createElement("input");
  label.required = true;
  label.value = record.asset.header.label;
  form.append(status, labeledEditorField("Name", label));

  const guard = document.createElement("select");
  guard.append(new Option("Always applicable", "true"), new Option("Never applicable", "false"));
  if (!["true", "false"].includes(node.guard.kind)) {
    guard.prepend(new Option("Keep Library-backed predicate", "preserve"));
  }
  guard.value = ["true", "false"].includes(node.guard.kind) ? node.guard.kind : "preserve";
  form.append(labeledEditorField("Guard", guard));
  if (!["true", "false"].includes(node.guard.kind)) {
    const warning = contentMessage("This Library-backed guard is preserved until replaced.");
    warning.className = "editor-warning";
    form.append(warning);
  }

  const inputs = pinEditor("Inputs", node.inputs);
  const outputs = pinEditor("Outputs", node.outputs);
  form.append(inputs.fieldset, outputs.fieldset);

  const locationEffect = node.effects.find((effect) => effect.kind === "set_location");
  const effect = document.createElement("fieldset");
  effect.className = "effect-editor";
  const effectLegend = document.createElement("legend");
  effectLegend.textContent = "Effect";
  const enabled = document.createElement("input");
  enabled.type = "checkbox";
  enabled.checked = Boolean(locationEffect);
  const enableLabel = document.createElement("label");
  enableLabel.className = "inline-check";
  enableLabel.append(enabled, document.createTextNode(" Set location"));
  const stage = document.createElement("input");
  stage.value = locationEffect?.location?.stage ?? "";
  const room = numericEditor(locationEffect?.location?.room ?? 0);
  const layer = numericEditor(locationEffect?.location?.layer ?? 0);
  const spawn = numericEditor(locationEffect?.location?.spawn ?? 0);
  const locationFields = document.createElement("div");
  locationFields.className = "location-fields";
  locationFields.append(
    labeledEditorField("Stage", stage),
    labeledEditorField("Room", room),
    labeledEditorField("Layer", layer),
    labeledEditorField("Spawn", spawn),
  );
  const updateLocationEnabled = () => {
    locationFields.querySelectorAll("input").forEach((input) => {
      input.disabled = !enabled.checked;
    });
  };
  enabled.addEventListener("change", updateLocationEnabled);
  updateLocationEnabled();
  effect.append(effectLegend, enableLabel, locationFields);
  form.append(effect);

  const evidence = node.evidence?.[0];
  const evidenceSource = document.createElement("input");
  evidenceSource.placeholder = "Test, trace, citation, or research note";
  evidenceSource.value = evidence?.source ?? "";
  const evidenceNote = document.createElement("textarea");
  evidenceNote.rows = 3;
  evidenceNote.placeholder = "What was observed or hypothesized?";
  evidenceNote.value = evidence?.note ?? "";
  form.append(
    labeledEditorField("Evidence source (optional)", evidenceSource),
    labeledEditorField("Evidence note", evidenceNote),
  );

  const save = document.createElement("button");
  save.type = "submit";
  save.textContent = "Save custom node";
  form.append(save);
  form.onsubmit = async (event) => {
    event.preventDefault();
    const updated = structuredClone(record.asset);
    updated.header.label = label.value.trim();
    updated.payload.inputs = collectPins(inputs.rows);
    updated.payload.outputs = collectPins(outputs.rows);
    if (guard.value !== "preserve") updated.payload.guard = { kind: guard.value };
    const effects = updated.payload.effects.filter((item) => item.kind !== "set_location");
    if (enabled.checked) {
      effects.push({
        kind: "set_location",
        location: {
          stage: stage.value.trim(),
          room: Number(room.value),
          layer: Number(layer.value),
          spawn: Number(spawn.value),
        },
      });
    }
    updated.payload.effects = effects;
    const source = evidenceSource.value.trim();
    const note = evidenceNote.value.trim();
    if (Boolean(source) !== Boolean(note)) {
      setStatus("Evidence source and note must be supplied together", "bad");
      return;
    }
    updated.payload.evidence = source ? [{
      id: evidence?.id ?? `evidence.${slug(updated.header.id)}.1`,
      source,
      note,
    }, ...(updated.payload.evidence ?? []).slice(1)] : [];
    await saveCustomNodeAsset(record, updated);
  };
}

function labeledEditorField(labelText, control) {
  const label = document.createElement("label");
  const text = document.createElement("span");
  text.textContent = labelText;
  label.append(text, control);
  return label;
}

function numericEditor(value) {
  const input = document.createElement("input");
  input.type = "number";
  input.step = "1";
  input.value = String(value);
  return input;
}

function pinEditor(label, pins) {
  const fieldset = document.createElement("fieldset");
  const legend = document.createElement("legend");
  legend.textContent = label;
  const rows = document.createElement("div");
  rows.className = "pin-rows";
  for (const pin of pins) appendPinRow(rows, pin);
  const add = document.createElement("button");
  add.type = "button";
  add.textContent = `Add ${label.slice(0, -1).toLowerCase()}`;
  add.addEventListener("click", () => appendPinRow(rows, {
    id: `${label.toLowerCase().slice(0, -1)}.${rows.childElementCount + 1}`,
    label: "",
    value_type: "state.value",
  }));
  fieldset.append(legend, rows, add);
  return { fieldset, rows };
}

function appendPinRow(container, pin) {
  const row = document.createElement("div");
  row.className = "pin-row";
  for (const [field, placeholder] of [
    ["id", "Stable pin ID"],
    ["label", "Label"],
    ["value_type", "Type"],
  ]) {
    const input = document.createElement("input");
    input.dataset.field = field;
    input.placeholder = placeholder;
    input.value = pin[field];
    row.append(input);
  }
  const remove = document.createElement("button");
  remove.type = "button";
  remove.textContent = "×";
  remove.setAttribute("aria-label", `Remove ${pin.label || pin.id}`);
  remove.addEventListener("click", () => row.remove());
  row.append(remove);
  container.append(row);
}

function collectPins(container) {
  return [...container.querySelectorAll(".pin-row")].map((row) =>
    Object.fromEntries([...row.querySelectorAll("input[data-field]")].map((input) => [
      input.dataset.field,
      input.value.trim(),
    ])));
}

async function saveCustomNodeAsset(record, asset) {
  try {
    const workspaceId = state.workspace.manifest.id;
    const saved = await projectApi(
      `/api/workspaces/${encodeURIComponent(workspaceId)}/assets/${encodeURIComponent(asset.header.id)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          schema: WORKSPACE_ASSET_SAVE_SCHEMA,
          relative_path: record.relative_path,
          expected_revision_sha256: record.revision_sha256,
          asset,
        }),
      },
    );
    state.selectedWorkspaceAsset = saved;
    state.workspace = await projectApi(`/api/workspaces/${encodeURIComponent(workspaceId)}`);
    state.workspaceSignature = workspaceSignature(state.workspace);
    await refreshWorkspaces(workspaceId);
    renderContentBrowser();
    elements["detail-title"].textContent = saved.asset.header.label;
    elements["detail-json"].textContent = JSON.stringify(saved.asset, null, 2);
    renderWorkspaceAssetEditor(saved);
    setStatus("Custom node saved", "good");
  } catch (error) {
    if (error.message.includes("revision conflict")
      && confirm(`${error.message}\n\nReload the version currently on disk?`)) {
      const workspaceId = state.workspace.manifest.id;
      const fresh = await projectApi(`/api/workspaces/${encodeURIComponent(workspaceId)}`);
      state.workspace = fresh;
      state.workspaceSignature = workspaceSignature(fresh);
      const listing = fresh.assets.find((item) => item.id === asset.header.id);
      if (listing) await inspectWorkspaceAsset(listing);
      setStatus("Reloaded the current asset from disk; local edits were not applied", "good");
      return;
    }
    setStatus(error.message, "bad");
  }
}

async function refreshProjects(openFirst = false, selectedId = null) {
  const list = await projectApi("/api/projects");
  elements["project-list"].replaceChildren(new Option("Projects", ""));
  for (const project of list.projects) {
    const prefix = project.read_only ? "Demo: " : "";
    elements["project-list"].append(new Option(`${prefix}${project.label}`, project.id));
  }
  if (selectedId && list.projects.some((project) => project.id === selectedId)) {
    elements["project-list"].value = selectedId;
  } else if (openFirst && list.projects.length) {
    elements["project-list"].value = list.projects[0].id;
    await loadStoredProject(list.projects[0].id, false);
  }
}

async function loadStoredProject(id, confirmDiscard = true) {
  if (confirmDiscard && state.dirty && !confirm("Discard unsaved planner changes?")) {
    elements["project-list"].value = state.project?.id ?? "";
    return;
  }
  try {
    const record = await projectApi(`/api/projects/${encodeURIComponent(id)}`);
    await loadProject(record.project, {
      revision: record.revision_sha256,
      readOnly: record.read_only,
      dirty: false,
      fit: true,
    });
    elements["project-list"].value = id;
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function newProject() {
  if (state.dirty && !confirm("Discard unsaved planner changes?")) return;
  try {
    const record = await projectApi("/api/project-template");
    const id = prompt("Project ID", "new-route");
    if (id == null) return;
    const label = prompt("Project name", "New route");
    if (label == null) return;
    record.project.id = id.trim();
    record.project.label = label.trim();
    await loadProject(record.project, { revision: null, readOnly: false, dirty: true, fit: true });
    elements["project-list"].value = "";
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function importProject(event) {
  const file = event.target.files?.[0];
  event.target.value = "";
  if (!file) return;
  if (state.dirty && !confirm("Discard unsaved planner changes?")) return;
  try {
    const project = JSON.parse(await file.text());
    if (!project.id) project.id = slug(file.name.replace(/\.json$/i, ""));
    await loadProject(project, { revision: null, readOnly: false, dirty: true, fit: true });
    elements["project-list"].value = "";
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function loadProject(project, options) {
  validateProject(project);
  setStatus("Projecting graph...");
  const payload = await service({
    command: "project_graph",
    request_id: requestId("project"),
    catalog: project.catalog,
    route_book: project.route_book ?? null,
  });
  if (payload.kind !== "graph") throw new Error(`Unexpected response ${payload.kind}`);
  state.project = project;
  state.graph = payload.graph;
  state.positions = new Map(Object.entries(project.presentation?.positions ?? {}));
  state.revision = options.revision;
  state.readOnly = options.readOnly;
  state.dirty = options.dirty;
  state.selected = null;
  state.transitionEvaluation = null;
  state.replacementStep = null;
  state.routeStepInspections = new Map();
  state.executionStateInspections = new Map();
  state.routeFrontier = null;
  state.selectedStateFeasibility = null;
  state.solveReport = null;
  state.groupSelection = new Set();
  state.activeRegionId = null;
  state.collapsedRegionIds = new Set();
  state.knownRegionIds = new Set();
  state.transitionSearch = new Map(project.catalog.mechanics.transitions.map((transition) => [
    transition.id,
    transitionSearchText(transition),
  ]));
  await refreshAuthoredRouteInspections();
  ensurePositions();
  elements["empty-state"].hidden = true;
  updateProjectControls();
  render();
  if (options.fit) requestAnimationFrame(fitGraph);
  setStatus(`${state.graph.nodes.length} nodes / ${state.graph.edges.length} connections`, "good");
}

function validateProject(project) {
  if (LEGACY_PROJECT_SCHEMAS.has(project?.schema)) project.schema = PROJECT_SCHEMA;
  if (!project || project.schema !== PROJECT_SCHEMA) throw new Error(`Expected ${PROJECT_SCHEMA}`);
  if (!project.id || typeof project.id !== "string") throw new Error("Project has no id");
  if (!project.label || typeof project.label !== "string") throw new Error("Project has no label");
  if (!project.catalog || typeof project.catalog !== "object") throw new Error("Project has no catalog");
  project.theorycraft_base_catalog ??= null;
  project.theorycraft_overlays ??= [];
  if (!Array.isArray(project.theorycraft_overlays)) throw new Error("Project theorycraft_overlays is invalid");
  if (project.route_book != null && typeof project.route_book !== "object") throw new Error("Project route_book is invalid");
  project.evidence_mode ??= "established_only";
  if (!["established_only", "research"].includes(project.evidence_mode)) {
    throw new Error("Project evidence_mode must be established_only or research");
  }
}

function projectEvidenceMode() {
  return state.project?.evidence_mode ?? "established_only";
}

async function projectApi(path, options = {}) {
  const response = await fetch(path, { cache: "no-store", ...options });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(body.error ?? `Project service returned HTTP ${response.status}`);
  return body;
}

async function service(request) {
  const response = await fetch("/api/service", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ schema: SERVICE_SCHEMA, request }),
  });
  if (!response.ok) throw new Error(`Planner service returned HTTP ${response.status}`);
  const envelope = await response.json();
  if (envelope.outcome?.status !== "ok") {
    throw new Error(`${envelope.outcome?.field ?? "planner"}: ${envelope.outcome?.detail ?? "request failed"}`);
  }
  return envelope.outcome.payload;
}

function requestId(prefix) {
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function ensurePositions() {
  synchronizeRegions();
  const validNodeIds = new Set(state.graph.nodes.map((node) => node.id));
  state.groupSelection = new Set(
    [...state.groupSelection].filter((nodeId) => validNodeIds.has(nodeId)),
  );
  const columns = new Map();
  for (const node of state.graph.nodes) {
    if (state.positions.has(node.id)) continue;
    const kind = node.payload.kind;
    const column = kind === "goal" ? 4 : kind === "transition" ? 2 : kind === "obstruction" ? 3 : kind === "reference_step" ? 1 : 0;
    const row = columns.get(column) ?? 0;
    columns.set(column, row + 1);
    state.positions.set(node.id, { x: column * 260, y: row * 82 });
  }
}

function render() {
  renderModelContext();
  renderRegionNavigation();
  renderEdges();
  renderNodes();
  renderPalette();
  renderDetails();
}

function renderModelContext() {
  const container = elements["model-context-body"];
  container.replaceChildren();
  if (!state.project) {
    const empty = document.createElement("p");
    empty.className = "context-empty";
    empty.textContent = "Open a project to inspect its exact model context.";
    container.append(empty);
    return;
  }

  const catalog = state.project.catalog;
  const runtime = state.project.start_state?.snapshot?.environment?.runtime_configuration ?? null;
  const identityRows = runtime ? [
    ["Content", shortDigest(runtime.content_sha256)],
    ["Language", runtime.language],
    ["Settings", Object.keys(runtime.settings ?? {}).length
      ? Object.entries(runtime.settings).map(([key, value]) => `${key}=${String(value)}`).join(", ")
      : "none"],
  ] : [["Runtime", "no exact start state"]];
  container.append(contextMetrics("Exact runtime", identityRows));

  const policy = document.createElement("section");
  policy.className = "context-section";
  const policyTitle = document.createElement("h3");
  policyTitle.textContent = "Evidence policy";
  const policySelect = document.createElement("select");
  policySelect.setAttribute("aria-label", "Evidence policy");
  policySelect.append(
    new Option("Established only", "established_only"),
    new Option("Research", "research"),
  );
  policySelect.value = projectEvidenceMode();
  policySelect.disabled = state.readOnly;
  policySelect.addEventListener("change", () => changeEvidenceMode(policySelect.value));
  policy.append(policyTitle, policySelect);
  container.append(policy);

  container.append(contextMetrics("Catalog provenance", [
    ["Facts", shortDigest(catalog.base_fact_catalog_sha256)],
    ["Mechanics", shortDigest(catalog.base_mechanics_catalog_sha256)],
    ["Bindings", catalog.obstruction_bindings?.length ?? 0],
  ]));

  const stack = catalog.refinement_stack?.entries ?? [];
  const packs = document.createElement("section");
  packs.className = "context-section";
  const packsTitle = document.createElement("h3");
  packsTitle.textContent = `Active packs & overlays · ${stack.length}`;
  packs.append(packsTitle);
  if (!stack.length) {
    const empty = document.createElement("p");
    empty.className = "context-empty";
    empty.textContent = "Base catalogs only; no refinement layer is active.";
    packs.append(empty);
  } else {
    for (const entry of stack) {
      const pack = document.createElement("div");
      pack.className = "context-pack";
      const name = document.createElement("strong");
      name.textContent = entry.pack_id;
      const detail = document.createElement("small");
      detail.textContent = `${taggedValue(entry.layer)} · priority ${entry.precedence} · ${shortDigest(entry.pack_sha256)}`;
      detail.title = entry.pack_sha256;
      pack.append(name, detail);
      if (!state.readOnly && state.project.theorycraft_overlays.some((overlay) =>
        overlay.manifest.id === entry.pack_id)) {
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "context-pack-remove";
        remove.textContent = "Remove";
        remove.setAttribute("aria-label", `Remove theorycraft overlay ${entry.pack_id}`);
        remove.addEventListener("click", () => editTheorycraftOverlays({
          kind: "remove",
          pack_id: entry.pack_id,
        }));
        pack.append(remove);
      }
      packs.append(pack);
    }
  }
  container.append(packs);

  const theorycraft = document.createElement("section");
  theorycraft.className = "context-section theorycraft-controls";
  const theorycraftTitle = document.createElement("h3");
  theorycraftTitle.textContent = "Theorycraft sandbox";
  const theorycraftHelp = document.createElement("p");
  theorycraftHelp.className = "context-empty";
  theorycraftHelp.textContent = "Add exact-context hypothetical transforms; each edit remains a removable refinement pack.";
  const actions = document.createElement("div");
  actions.className = "context-actions";
  for (const [label, action] of [
    ["Rebind", () => addComponentTransfer("rebind")],
    ["Copy", () => addComponentTransfer("copy")],
    ["Bypass", addObstructionBypass],
  ]) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.disabled = state.readOnly || !state.project.start_state;
    button.addEventListener("click", action);
    actions.append(button);
  }
  if (state.project.theorycraft_overlays.length) {
    const clear = document.createElement("button");
    clear.type = "button";
    clear.textContent = "Clear all";
    clear.disabled = state.readOnly;
    clear.addEventListener("click", () => editTheorycraftOverlays({ kind: "clear" }));
    actions.append(clear);
  }
  theorycraft.append(theorycraftTitle, theorycraftHelp, actions);
  container.append(theorycraft);

  const coverage = catalogCoverage(catalog);
  container.append(contextMetrics("Coverage", [
    ["Facts", `${coverage.facts} records`],
    ["Mechanics", `${coverage.mechanics} records`],
    ["Families", `${coverage.populatedFamilies}/${coverage.familyCount}`],
    ["Unresolved", coverage.unresolved],
  ]));
  const confidence = catalogConfidence(catalog);
  container.append(contextMetrics("Confidence", [
    ["Established", confidence.established ?? 0],
    ["Contested", confidence.contested ?? 0],
    ["Hypothetical", confidence.hypothetical ?? 0],
    ["Unknown", confidence.unknown ?? 0],
  ]));
  const costs = catalogCostAxes(catalog);
  container.append(contextMetrics("Route-cost model", costs.length
    ? costs.map(([axis, range]) => [axis, range])
    : [["Axes", "none declared"]]));
}

function contextMetrics(titleText, rows) {
  const section = document.createElement("section");
  section.className = "context-section";
  const title = document.createElement("h3");
  title.textContent = titleText;
  const metrics = document.createElement("dl");
  for (const [name, value] of rows) {
    const term = document.createElement("dt");
    term.textContent = name;
    const detail = document.createElement("dd");
    detail.textContent = String(value);
    detail.title = String(value);
    metrics.append(term, detail);
  }
  section.append(title, metrics);
  return section;
}

async function addComponentTransfer(mode) {
  if (!state.project?.start_state || state.readOnly) return;
  const components = state.project.start_state.snapshot.environment.components ?? [];
  if (!components.length) {
    setStatus("The exact start state has no live components to transfer", "bad");
    return;
  }
  const sourceId = prompt(
    `Source component ID\n\nAvailable: ${components.map((component) => component.id).join(", ")}`,
    components[0].id,
  );
  if (sourceId == null) return;
  const source = components.find((component) => component.id === sourceId.trim());
  if (!source) {
    setStatus(`Component ${sourceId.trim()} is not in the exact start state`, "bad");
    return;
  }
  const binding = promptComponentBinding(source.binding);
  if (!binding) return;
  const defaultLabel = mode === "copy"
    ? `Copy ${source.id} into ${bindingSummary(binding)}`
    : `Rebind ${source.id} to ${bindingSummary(binding)}`;
  const label = prompt("Theorycraft assumption label", defaultLabel);
  if (label == null) return;
  const packId = prompt(
    "Refinement pack ID",
    `what-if.${slug(label).slice(0, 54)}-${Date.now().toString(36)}`,
  );
  if (packId == null) return;
  const destination = mode === "copy"
    ? (() => {
      const destinationId = prompt("New component ID", `${source.id}.what-if-copy`);
      return destinationId == null ? null : {
        kind: "copy",
        destination_component_id: destinationId.trim(),
        binding,
      };
    })()
    : { kind: "rebind", binding };
  if (!destination) return;
  const preview = mode === "copy"
    ? `${source.id} (${bindingSummary(source.binding)}) will be copied as ${destination.destination_component_id} (${bindingSummary(binding)}).`
    : `${source.id} will retain its payload and change binding from ${bindingSummary(source.binding)} to ${bindingSummary(binding)}.`;
  if (!confirm(`Preview hypothetical component transfer:\n\n${preview}\n\nEnable this exact-context assumption?`)) return;
  await editTheorycraftOverlays({
    kind: "add_component_transfer",
    pack_id: packId.trim(),
    label: label.trim(),
    source_component_id: source.id,
    destination,
  });
}

async function addObstructionBypass() {
  if (!state.project?.start_state || state.readOnly) return;
  const obstructions = state.project.catalog.mechanics.obstructions ?? [];
  if (!obstructions.length) {
    setStatus("The composed catalog has no obstruction to bypass", "bad");
    return;
  }
  const selected = state.selected?.type === "node" && state.selected.value.payload.kind === "obstruction"
    ? state.selected.value.payload.obstruction_id
    : obstructions[0].id;
  const obstructionId = prompt(
    `Obstruction ID\n\nAvailable: ${obstructions.map((record) => record.id).join(", ")}`,
    selected,
  );
  if (obstructionId == null) return;
  const obstruction = obstructions.find((record) => record.id === obstructionId.trim());
  if (!obstruction) {
    setStatus(`Obstruction ${obstructionId.trim()} is not in the composed catalog`, "bad");
    return;
  }
  const label = prompt("Theorycraft assumption label", `Assume ${obstruction.label} absent`);
  if (label == null) return;
  const packId = prompt(
    "Refinement pack ID",
    `what-if.${slug(label).slice(0, 54)}-${Date.now().toString(36)}`,
  );
  if (packId == null) return;
  if (!confirm(`Enable an exact-context hypothetical resolver that assumes ${obstruction.label} absent?`)) return;
  await editTheorycraftOverlays({
    kind: "add_obstruction_bypass",
    pack_id: packId.trim(),
    label: label.trim(),
    obstruction_id: obstruction.id,
  });
}

function promptComponentBinding(current) {
  const kind = prompt(
    "Destination binding kind: global, stage, room, zone, dungeon, runtime_file, actor, session, unbound, or custom",
    current?.kind === "unbound" ? "unbound" : current?.kind ?? "stage",
  );
  if (kind == null) return null;
  const value = (message, fallback = "") => {
    const entered = prompt(message, fallback);
    return entered == null ? null : entered.trim();
  };
  const integer = (message, fallback) => {
    const entered = value(message, String(fallback));
    if (entered == null) return null;
    const parsed = Number(entered);
    if (!Number.isInteger(parsed)) throw new Error(`${message} must be an integer`);
    return parsed;
  };
  try {
    switch (kind.trim()) {
      case "global": return { kind: "global" };
      case "stage": {
        const stage = value("Stage ID", current?.stage ?? state.project.start_state.snapshot.environment.location.stage);
        return stage == null ? null : { kind: "stage", stage };
      }
      case "room": {
        const stage = value("Stage ID", current?.stage ?? state.project.start_state.snapshot.environment.location.stage);
        if (stage == null) return null;
        const room = integer("Room", current?.room ?? state.project.start_state.snapshot.environment.location.room);
        return room == null ? null : { kind: "room", stage, room };
      }
      case "zone": {
        const stage = value("Stage ID", current?.stage ?? state.project.start_state.snapshot.environment.location.stage);
        if (stage == null) return null;
        const zone = integer("Zone", current?.zone ?? 0);
        return zone == null ? null : { kind: "zone", stage, zone };
      }
      case "dungeon": {
        const dungeon = value("Dungeon ID", current?.dungeon ?? "");
        return dungeon == null ? null : { kind: "dungeon", dungeon };
      }
      case "runtime_file": {
        const runtimeFileId = value("Runtime file ID", current?.runtime_file_id ?? state.project.start_state.snapshot.environment.active_runtime_file.id);
        return runtimeFileId == null ? null : { kind: "runtime_file", runtime_file_id: runtimeFileId };
      }
      case "actor": {
        const instanceId = value("Actor instance ID", current?.instance_id ?? "");
        return instanceId == null ? null : { kind: "actor", instance_id: instanceId };
      }
      case "session": {
        const sessionId = value("Session ID", current?.session_id ?? "session.what-if");
        return sessionId == null ? null : { kind: "session", session_id: sessionId };
      }
      case "unbound": return { kind: "unbound" };
      case "custom": {
        const kindId = value("Custom binding kind ID", current?.kind_id ?? "binding.what-if");
        if (kindId == null) return null;
        const contextId = value("Custom context ID", current?.context_id ?? "context.what-if");
        return contextId == null ? null : { kind: "custom", kind_id: kindId, context_id: contextId };
      }
      default:
        setStatus(`Unknown component binding kind ${kind.trim()}`, "bad");
        return null;
    }
  } catch (error) {
    setStatus(error.message, "bad");
    return null;
  }
}

function bindingSummary(binding) {
  if (!binding || typeof binding !== "object") return "unknown binding";
  const detail = Object.entries(binding)
    .filter(([key]) => key !== "kind")
    .map(([, value]) => String(value))
    .join("/");
  return detail ? `${binding.kind}:${detail}` : binding.kind;
}

async function editTheorycraftOverlays(edit) {
  if (!state.project?.start_state || state.readOnly) return;
  if (edit.kind === "clear" && !confirm("Remove every authored theorycraft overlay from this project?")) return;
  try {
    setStatus("Recomposing theorycraft sandbox...");
    const baseCatalog = state.project.theorycraft_base_catalog ?? state.project.catalog;
    const payload = await service({
      command: "edit_theorycraft_overlays",
      request_id: requestId("theorycraft"),
      base_catalog: baseCatalog,
      overlays: state.project.theorycraft_overlays,
      state: state.project.start_state,
      route_book: state.project.route_book ?? null,
      edit,
    });
    if (payload.kind !== "theorycraft_overlays_edited") {
      throw new Error(`Unexpected response ${payload.kind}`);
    }
    const selectedId = state.selected?.type === "node" ? state.selected.value.id : null;
    state.project.catalog = payload.catalog;
    state.project.theorycraft_overlays = payload.overlays;
    state.project.theorycraft_base_catalog = payload.overlays.length ? payload.base_catalog : null;
    state.project.route_book = payload.route_book;
    const projected = await service({
      command: "project_graph",
      request_id: requestId("project-after-theorycraft"),
      catalog: state.project.catalog,
      route_book: state.project.route_book ?? null,
    });
    if (projected.kind !== "graph") throw new Error(`Unexpected response ${projected.kind}`);
    state.graph = projected.graph;
    state.solveReport = null;
    state.transitionSearch = new Map(state.project.catalog.mechanics.transitions.map((transition) => [
      transition.id,
      transitionSearchText(transition),
    ]));
    state.transitionEvaluation = null;
    await refreshAuthoredRouteInspections();
    ensurePositions();
    const selected = state.graph.nodes.find((candidate) => candidate.id === selectedId);
    state.selected = selected ? { type: "node", value: selected } : null;
    markDirty();
    render();
    const action = payload.added_pack
      ? `Enabled ${payload.added_pack.manifest.id}`
      : `Removed ${payload.removed_pack_ids.length} theorycraft overlay${payload.removed_pack_ids.length === 1 ? "" : "s"}`;
    setStatus(`${action}; save to persist`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

function shortDigest(value) {
  return typeof value === "string" && value.length > 16
    ? `${value.slice(0, 8)}…${value.slice(-8)}`
    : value ?? "none";
}

function catalogCoverage(catalog) {
  const families = [catalog.facts?.aliases, catalog.facts?.derived_facts];
  for (const [name, records] of Object.entries(catalog.mechanics ?? {})) {
    if (name !== "schema" && Array.isArray(records)) families.push(records);
  }
  const facts = (catalog.facts?.aliases?.length ?? 0) + (catalog.facts?.derived_facts?.length ?? 0);
  const mechanics = families.slice(2).reduce((sum, records) => sum + records.length, 0);
  const unresolved = (catalog.mechanics?.obligations ?? []).filter((record) =>
    record.detail?.kind === "unresolved").length;
  return {
    facts,
    mechanics,
    familyCount: families.length,
    populatedFamilies: families.filter((records) => records.length).length,
    unresolved,
  };
}

function catalogConfidence(catalog) {
  const counts = {};
  const visit = (value) => {
    if (Array.isArray(value)) {
      value.forEach(visit);
      return;
    }
    if (!value || typeof value !== "object") return;
    if (value.evidence && typeof value.evidence.truth === "string") {
      counts[value.evidence.truth] = (counts[value.evidence.truth] ?? 0) + 1;
    }
    for (const [key, child] of Object.entries(value)) {
      if (key !== "evidence") visit(child);
    }
  };
  visit(catalog.facts);
  visit(catalog.mechanics);
  return counts;
}

function catalogCostAxes(catalog) {
  const values = new Map();
  for (const technique of catalog.mechanics?.techniques ?? []) {
    for (const [axis, value] of Object.entries(technique.cost?.axes ?? {})) {
      const range = values.get(axis) ?? { minimum: value, maximum: value, count: 0 };
      range.minimum = Math.min(range.minimum, value);
      range.maximum = Math.max(range.maximum, value);
      range.count += 1;
      values.set(axis, range);
    }
  }
  return [...values.entries()].sort(([left], [right]) => left.localeCompare(right)).map(([axis, range]) => [
    axis,
    `${range.minimum === range.maximum ? range.minimum : `${range.minimum}…${range.maximum}`} · ${range.count} technique(s)`,
  ]);
}

async function changeEvidenceMode(mode) {
  if (!state.project || state.readOnly || mode === state.project.evidence_mode) return;
  const previous = state.project.evidence_mode;
  state.project.evidence_mode = mode;
  try {
    setStatus(`Applying ${taggedValue(mode)} evidence policy...`);
    await refreshAuthoredRouteInspections();
    state.selected = null;
    state.transitionEvaluation = null;
    markDirty();
    render();
    setStatus(`Evidence policy changed to ${taggedValue(mode)}; save to persist`, "good");
  } catch (error) {
    state.project.evidence_mode = previous;
    render();
    setStatus(error.message, "bad");
  }
}

function renderEdges() {
  elements.edges.replaceChildren();
  const visible = new Set(visibleNodes().map((node) => node.id));
  const nodesById = new Map((state.graph?.nodes ?? []).map((node) => [node.id, node]));
  for (const edge of state.graph?.edges ?? []) {
    if (!visible.has(edge.source_node_id) || !visible.has(edge.target_node_id)) continue;
    const source = state.positions.get(edge.source_node_id);
    const target = state.positions.get(edge.target_node_id);
    if (!source || !target) continue;
    const sourceNode = nodesById.get(edge.source_node_id);
    const targetNode = nodesById.get(edge.target_node_id);
    const acceptedRouteJoin = edge.relation === "selects_action"
      && sourceNode?.payload.kind === "reference_step"
      && targetNode?.payload.kind === "transition";
    const path = svg("path", {
      class: `graph-edge${acceptedRouteJoin ? " route-accepted" : ""}${state.selected?.type === "edge" && state.selected.value.id === edge.id ? " selected" : ""}`,
      d: connector(source, target),
    });
    path.addEventListener("click", (event) => {
      event.stopPropagation();
      state.selected = { type: "edge", value: edge };
      state.transitionEvaluation = null;
      render();
    });
    elements.edges.append(path);
  }
  renderRejectedRouteJoin();
}

function renderRejectedRouteJoin() {
  if (state.transitionEvaluation?.kind !== "rejected_transition_join"
    || state.selected?.type !== "node"
    || state.selected.value.payload.kind !== "transition"
    || !state.project?.route_book) return;
  const method = state.project.route_book.methods.find((candidate) =>
    candidate.id === "method.authored-route");
  const frontierStepId = method?.step_ids.at(-1);
  const sourceNode = state.graph.nodes.find((node) =>
    node.payload.kind === "reference_step" && node.payload.step_id === frontierStepId);
  const source = sourceNode ? state.positions.get(sourceNode.id) : null;
  const target = state.positions.get(state.selected.value.id);
  const visible = new Set(visibleNodes().map((node) => node.id));
  if (!source || !target || !visible.has(sourceNode.id) || !visible.has(state.selected.value.id)) return;
  const classification = state.transitionEvaluation.assessment.classification;
  const joinClass = classification === "feasibility_unknown" ? "route-unknown" : "route-rejected";
  elements.edges.append(svg("path", {
    class: `graph-edge ${joinClass}`,
    d: connector(source, target),
    "data-rejected-route-join": classification,
  }));
}

function renderNodes() {
  elements.nodes.replaceChildren();
  for (const node of visibleNodes()) {
    const position = state.positions.get(node.id);
    const joinClass = transitionJoinClass(node);
    const preferenceClass = routePreferenceClass(node);
    const groupClass = state.groupSelection.has(node.id) ? " group-selected" : "";
    const group = svg("g", {
      class: `node ${node.payload.kind}${state.selected?.type === "node" && state.selected.value.id === node.id ? " selected" : ""}${groupClass}${joinClass ? ` ${joinClass}` : ""}${preferenceClass ? ` ${preferenceClass}` : ""}`,
      transform: `translate(${position.x} ${position.y})`,
      "data-node-id": node.id,
    });
    group.append(svg("rect", { width: NODE_WIDTH, height: NODE_HEIGHT }));
    const kind = svg("text", { class: "kind", x: 10, y: 15 });
    kind.textContent = node.payload.kind.replaceAll("_", " ");
    const label = svg("text", { x: 10, y: 35 });
    label.textContent = elide(node.label, 25);
    group.append(kind, label);
    group.addEventListener("pointerdown", (event) => beginNodeDrag(event, node));
    group.addEventListener("click", (event) => {
      event.stopPropagation();
      if (event.shiftKey) {
        toggleGroupSelection(node);
        render();
        return;
      }
      selectNode(node);
      render();
    });
    group.addEventListener("dblclick", (event) => {
      const owned = state.graph.regions.find((region) => region.owner_node_id === node.id);
      if (!owned) return;
      event.stopPropagation();
      enterRegion(owned.id);
    });
    elements.nodes.append(group);
  }
}

function openAddNodeMenu(event) {
  if (!state.graph || !state.project) return;
  event.preventDefault();
  event.stopPropagation();
  const bounds = elements["canvas-shell"].getBoundingClientRect();
  const menu = elements["add-node-menu"];
  menu.hidden = false;
  const width = 330;
  const height = Math.min(520, bounds.height - 24);
  menu.style.left = `${Math.max(8, Math.min(event.clientX - bounds.left, bounds.width - width - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(event.clientY - bounds.top, bounds.height - height - 8))}px`;
  elements["add-node-search"].value = "";
  renderAddNodeMenu();
  elements["add-node-search"].focus();
}

function closeAddNodeMenu() {
  elements["add-node-menu"].hidden = true;
}

function renderAddNodeMenu() {
  const results = elements["add-node-results"];
  results.replaceChildren();
  if (!state.graph) return;
  const query = elements["add-node-search"].value.trim().toLowerCase();
  const frontier = new Map((state.routeFrontier?.transitions ?? []).map((record) => [
    record.transition_id,
    record.assessment.classification,
  ]));
  const matches = state.graph.nodes
    .filter((node) => node.payload.kind === "transition")
    .map((node) => ({
      node,
      contract: selectedContract(node),
      classification: frontier.get(node.payload.transition_id) ?? "not_assessed",
    }))
    .filter(({ node }) =>
      !query || `${node.label} ${node.payload.transition_id} ${state.transitionSearch.get(node.payload.transition_id) ?? ""}`
        .toLowerCase().includes(query))
    .sort((left, right) => {
      const leftCategory = left.contract?.transition_kind ?? "other";
      const rightCategory = right.contract?.transition_kind ?? "other";
      const rank = (classification) =>
        classification === "executable" ? 0 : classification === "feasibility_unknown" ? 1 : 2;
      return leftCategory.localeCompare(rightCategory)
        || rank(left.classification) - rank(right.classification)
        || left.node.label.localeCompare(right.node.label);
    });
  let category = null;
  for (const match of matches) {
    const nextCategory = (match.contract?.transition_kind ?? "other").replaceAll("_", " ");
    if (nextCategory !== category) {
      category = nextCategory;
      const heading = document.createElement("h3");
      heading.className = "add-node-category";
      heading.textContent = category;
      results.append(heading);
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = "add-node-result";
    const label = document.createElement("strong");
    label.textContent = match.node.label;
    const compatibility = document.createElement("span");
    compatibility.className = `compatibility ${match.classification}`;
    compatibility.textContent = match.classification.replaceAll("_", " ");
    const id = document.createElement("small");
    id.textContent = match.node.payload.transition_id;
    button.append(label, compatibility, id);
    button.addEventListener("click", async () => {
      closeAddNodeMenu();
      selectNode(match.node);
      revealNode(match.node);
      render();
      if (state.readOnly) {
        setStatus("Library examples are read-only; use Save as before authoring", "bad");
      } else if (!state.project?.start_state) {
        setStatus("This graph needs a grounded Scenario Root before a transition can be added", "bad");
      } else {
        await insertSelectedTransition();
      }
    });
    results.append(button);
  }
  if (!matches.length) results.append(contentMessage("No compatible node kinds found."));
}

function renderPalette(selectedFeasibility = state.selectedStateFeasibility) {
  elements["palette-list"].replaceChildren();
  if (!state.graph || !state.project) return;
  const query = elements.search.value.trim().toLowerCase();
  const assessedTransitions = selectedFeasibility?.transitions.map((record) => ({
    transition_id: record.transition_id,
    assessment: record.modeled,
    diagnostics: {
      active_obstruction_ids: record.active_obstruction_ids,
      unknown_obstruction_ids: record.unknown_obstruction_ids,
      applied_resolver_ids: [],
      applicable_technique_ids: [],
    },
  })) ?? state.routeFrontier?.transitions ?? [];
  const frontierTransitions = new Map(assessedTransitions.map((record) => [
    record.transition_id,
    record,
  ]));
  const matches = state.graph.nodes.filter((node) => {
    const transition = node.payload.kind === "transition";
    const fact = node.payload.kind === "alias" || node.payload.kind === "derived_fact";
    if (!transition && (!query || !fact)) return false;
    const contract = selectedContract(node);
    const contractText = transition
      ? state.transitionSearch.get(node.payload.transition_id)
      : transitionSearchText(contract);
    return !query || `${node.label} ${node.id} ${contractText ?? ""}`.toLowerCase().includes(query);
  }).sort((left, right) => {
    const rank = (node) => {
      if (node.payload.kind !== "transition") return 3;
      const classification = frontierTransitions.get(node.payload.transition_id)?.assessment.classification;
      return classification === "executable" ? 0 : classification === "feasibility_unknown" ? 1 : 2;
    };
    return rank(left) - rank(right) || left.label.localeCompare(right.label);
  });
  for (const node of matches) {
    const button = document.createElement("button");
    const transition = node.payload.kind === "transition";
    button.className = `palette-item${transition ? "" : " fact"}`;
    button.draggable = transition;
    const label = document.createElement("span");
    label.textContent = node.label;
    const id = document.createElement("small");
    const frontier = transition ? frontierTransitions.get(node.payload.transition_id) : null;
    id.textContent = transition
      ? `${frontier?.assessment.classification?.replaceAll("_", " ") ?? "not assessed"} · ${node.payload.transition_id}`
      : `${node.payload.kind.replaceAll("_", " ")} · ${node.payload.fact_id}`;
    button.append(label, id);
    button.addEventListener("click", () => {
      state.selected = { type: "node", value: node };
      state.transitionEvaluation = null;
      revealNode(node);
      centerNode(node.id);
      render();
    });
    if (transition) {
      button.addEventListener("dragstart", (event) => {
        event.dataTransfer.effectAllowed = "copy";
        event.dataTransfer.setData("text/plain", node.id);
        setStatus(`Drop ${node.label} on the canvas or current route frontier`);
      });
    }
    elements["palette-list"].append(button);
  }
}

function selectNode(node) {
  state.selected = { type: "node", value: node };
  state.selectedStateFeasibility = null;
  if (node.payload.kind === "reference_step") {
    state.transitionEvaluation = state.routeStepInspections.get(node.payload.step_id) ?? null;
  } else if (node.payload.kind === "execution_state") {
    const inspection = state.executionStateInspections.get(node.id) ?? null;
    const producingStep = node.payload.route_step_id == null
      ? null
      : state.routeStepInspections.get(node.payload.route_step_id) ?? null;
    state.transitionEvaluation = producingStep ? {
      kind: "execution_state_inspection",
      step_id: node.payload.route_step_id,
      state_change: producingStep.state_change,
    } : inspection ? {
      kind: "execution_state_inspection",
      state_change: { before: inspection, after: null, diff: null },
    } : null;
    const selectedInspection = inspection ?? producingStep?.state_change.after ?? null;
    if (selectedInspection) refreshSelectedStateFeasibility(node, selectedInspection.state).catch((error) => {
      if (state.selected?.value.id === node.id) setStatus(error.message, "bad");
    });
  } else {
    state.transitionEvaluation = null;
  }
}

async function refreshSelectedStateFeasibility(node, executionState) {
  const payload = await service({
    command: "inspect_route_frontier",
    request_id: requestId("selected-state-feasibility"),
    state: executionState,
    catalog: state.project.catalog,
    equivalence_sets: state.project.equivalence_sets ?? [],
    route_book: null,
    evidence_mode: projectEvidenceMode(),
  });
  if (payload.kind !== "route_frontier") {
    throw new Error(`Unexpected response ${payload.kind}`);
  }
  if (state.selected?.type !== "node" || state.selected.value.id !== node.id) return;
  const feasibility = {
    transitions: payload.transitions.map((record) => ({
      transition_id: record.transition_id,
      modeled: record.assessment,
      active_obstruction_ids: record.diagnostics.active_obstruction_ids,
      unknown_obstruction_ids: record.diagnostics.unknown_obstruction_ids,
    })),
  };
  state.selectedStateFeasibility = feasibility;
  renderPalette(feasibility);
  const executable = feasibility.transitions.filter((record) =>
    record.modeled.classification === "executable").length;
  const graphTransitionIds = new Set(state.graph.nodes
    .filter((candidate) => candidate.payload.kind === "transition")
    .map((candidate) => candidate.payload.transition_id));
  const matched = feasibility.transitions.filter((record) =>
    graphTransitionIds.has(record.transition_id)).length;
  setStatus(`${executable} transition(s) executable from ${node.label} (${matched}/${feasibility.transitions.length} assessed)`, "good");
}

async function refreshAuthoredRouteInspections() {
  state.routeStepInspections = new Map();
  state.routeFrontier = null;
  if (!state.project?.start_state) return;
  const frontier = await service({
    command: "inspect_route_frontier",
    request_id: requestId("inspect-route-frontier"),
    state: state.project.start_state,
    catalog: state.project.catalog,
    equivalence_sets: state.project.equivalence_sets ?? [],
    route_book: state.project.route_book ?? null,
    evidence_mode: projectEvidenceMode(),
  });
  if (frontier.kind !== "route_frontier") {
    throw new Error(`Unexpected response ${frontier.kind}`);
  }
  state.routeFrontier = frontier;
  state.graph = frontier.graph;
  state.executionStateInspections = new Map();
  for (const node of state.graph.nodes.filter((candidate) =>
    candidate.payload.kind === "execution_state")) {
    const stepId = node.payload.route_step_id;
    const index = stepId == null ? 0 : 1 + (state.project.route_book?.methods
      .find((method) => method.id === "method.authored-route")?.step_ids.indexOf(stepId) ?? -2);
    const inspection = frontier.execution_states[index];
    if (inspection) state.executionStateInspections.set(node.id, inspection);
  }
  if (!state.project.route_book?.methods.some((method) =>
    method.id === "method.authored-route")) return;
  const payload = await service({
    command: "inspect_authored_route",
    request_id: requestId("inspect-authored-route"),
    state: state.project.start_state,
    catalog: state.project.catalog,
    equivalence_sets: state.project.equivalence_sets ?? [],
    route_book: state.project.route_book,
    evidence_mode: projectEvidenceMode(),
  });
  if (payload.kind !== "authored_route_inspection") {
    throw new Error(`Unexpected response ${payload.kind}`);
  }
  for (const step of payload.inspection.steps) {
    state.routeStepInspections.set(step.step_id, {
      kind: "authored_route_step_inspection",
      step_id: step.step_id,
      transition_id: step.transition_id,
      assessment: step.assessment,
      state_change: step.state_change,
    });
  }
  const rejection = payload.inspection.rejection;
  if (rejection) {
    state.routeStepInspections.set(rejection.failed_step_id, {
      kind: "rejected_authored_route_step",
      failed_step_id: rejection.failed_step_id,
      transition_id: rejection.transition_id,
      assessment: rejection.assessment,
      diagnostics: rejection.diagnostics,
      state_change: rejection.prefix_state_change,
    });
  }
}

function displayedRegions() {
  const planner = state.graph?.regions ?? [];
  const authored = (state.project?.presentation?.regions ?? []).map((region) => ({
    ...region,
    owner_node_id: null,
    collapsed_by_default: false,
    presentation_only: true,
  }));
  return [...planner, ...authored];
}

function displayedNodeRegionId(node) {
  return state.project?.presentation?.node_region_ids?.[node.id] ?? node.region_id ?? null;
}

function regionSnapshotNodeIds(region, visited = new Set()) {
  if (!region || visited.has(region.id)) return [];
  visited.add(region.id);
  if (region.derivation?.kind === "reference") {
    const source = displayedRegions().find((candidate) =>
      candidate.id === region.derivation.source_region_id);
    const snapshot = regionSnapshotNodeIds(source, visited);
    return snapshot.length ? snapshot : regionOwnedNodeIds(source);
  }
  return region.snapshot_node_ids ?? [];
}

function regionOwnedNodeIds(region) {
  if (!region) return [];
  const regions = displayedRegions();
  const enclosed = new Set([region.id]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const candidate of regions) {
      if (candidate.parent_region_id && enclosed.has(candidate.parent_region_id)
        && !enclosed.has(candidate.id)) {
        enclosed.add(candidate.id);
        changed = true;
      }
    }
  }
  return state.graph.nodes
    .filter((node) => enclosed.has(displayedNodeRegionId(node)))
    .map((node) => node.id)
    .sort();
}

function synchronizeRegions() {
  const regions = displayedRegions();
  const valid = new Set(regions.map((region) => region.id));
  for (const region of regions) {
    if (!state.knownRegionIds.has(region.id) && region.collapsed_by_default) {
      state.collapsedRegionIds.add(region.id);
    }
  }
  state.knownRegionIds = valid;
  state.collapsedRegionIds = new Set(
    [...state.collapsedRegionIds].filter((id) => valid.has(id)),
  );
  if (state.activeRegionId && !valid.has(state.activeRegionId)) state.activeRegionId = null;
}

function visibleNodes() {
  if (!state.graph) return [];
  const regions = displayedRegions();
  if (state.activeRegionId) {
    const active = regions.find((region) => region.id === state.activeRegionId);
    const snapshotNodes = new Set(regionSnapshotNodeIds(active));
    if (snapshotNodes.size) {
      return state.graph.nodes.filter((node) => snapshotNodes.has(node.id));
    }
    return state.graph.nodes.filter((node) => displayedNodeRegionId(node) === state.activeRegionId);
  }
  return state.graph.nodes.filter((node) => {
    let regionId = displayedNodeRegionId(node);
    const visited = new Set();
    while (regionId && !visited.has(regionId)) {
      if (state.collapsedRegionIds.has(regionId)) return false;
      visited.add(regionId);
      regionId = regions.find((region) => region.id === regionId)?.parent_region_id ?? null;
    }
    return true;
  });
}

function renderRegionNavigation() {
  const nav = elements["region-nav"];
  const regions = displayedRegions();
  nav.hidden = !regions.length;
  elements["region-breadcrumbs"].replaceChildren();
  elements["region-children"].replaceChildren();
  if (!regions.length) return;
  const all = document.createElement("button");
  all.type = "button";
  all.textContent = "All regions";
  all.classList.toggle("current", !state.activeRegionId);
  all.addEventListener("click", () => enterRegion(null));
  elements["region-breadcrumbs"].append(all);
  for (const region of activeRegionPath()) {
    const separator = document.createElement("span");
    separator.textContent = "›";
    separator.setAttribute("aria-hidden", "true");
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = region.label;
    button.classList.toggle("current", region.id === state.activeRegionId);
    button.addEventListener("click", () => enterRegion(region.id));
    elements["region-breadcrumbs"].append(separator, button);
  }
  const children = regions.filter((region) =>
    (region.parent_region_id ?? null) === state.activeRegionId);
  for (const region of children) {
    const row = document.createElement("span");
    row.className = `region-row${state.collapsedRegionIds.has(region.id) ? " collapsed" : ""}`;
    const enter = document.createElement("button");
    enter.type = "button";
    enter.className = "enter-region";
    enter.textContent = region.label;
    enter.title = "Enter region";
    enter.addEventListener("click", () => enterRegion(region.id));
    const inspect = document.createElement("button");
    inspect.type = "button";
    inspect.className = "inspect-region";
    const boundary = inspectRegionBoundary(region);
    inspect.textContent = `↔${boundary.boundary_edges.length}`;
    inspect.title = "Inspect every edge crossing this region boundary";
    inspect.addEventListener("click", () => selectRegionBoundary(region));
    const collapse = document.createElement("button");
    collapse.type = "button";
    collapse.className = "collapse-region";
    const collapsed = state.collapsedRegionIds.has(region.id);
    collapse.textContent = collapsed ? "+" : "−";
    collapse.title = collapsed ? "Expand region in the full graph" : "Collapse region in the full graph";
    collapse.addEventListener("click", () => toggleRegionCollapse(region.id));
    row.append(enter, inspect, collapse);
    elements["region-children"].append(row);
  }
}

function activeRegionPath() {
  const path = [];
  const regions = displayedRegions();
  let region = regions.find((candidate) => candidate.id === state.activeRegionId);
  const visited = new Set();
  while (region && !visited.has(region.id)) {
    path.unshift(region);
    visited.add(region.id);
    region = regions.find((candidate) => candidate.id === region.parent_region_id);
  }
  return path;
}

function enterRegion(regionId) {
  state.activeRegionId = regionId;
  if (regionId) state.collapsedRegionIds.delete(regionId);
  state.selected = null;
  state.transitionEvaluation = null;
  render();
  requestAnimationFrame(fitGraph);
}

function inspectRegionBoundary(region) {
  const regions = displayedRegions();
  const enclosedRegions = new Set([region.id]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const candidate of regions) {
      if (candidate.parent_region_id && enclosedRegions.has(candidate.parent_region_id)
        && !enclosedRegions.has(candidate.id)) {
        enclosedRegions.add(candidate.id);
        changed = true;
      }
    }
  }
  const enclosedNodes = new Set(state.graph.nodes
    .filter((node) => enclosedRegions.has(displayedNodeRegionId(node)))
    .map((node) => node.id));
  for (const enclosedRegionId of enclosedRegions) {
    const enclosedRegion = regions.find((candidate) => candidate.id === enclosedRegionId);
    for (const nodeId of regionSnapshotNodeIds(enclosedRegion)) enclosedNodes.add(nodeId);
  }
  const nodes = new Map(state.graph.nodes.map((node) => [node.id, node]));
  const boundaryEdges = [];
  for (const edge of state.graph.edges) {
    const sourceInside = enclosedNodes.has(edge.source_node_id);
    const targetInside = enclosedNodes.has(edge.target_node_id);
    if (sourceInside === targetInside) continue;
    const insideNodeId = sourceInside ? edge.source_node_id : edge.target_node_id;
    const outsideNodeId = sourceInside ? edge.target_node_id : edge.source_node_id;
    const insideNode = nodes.get(insideNodeId);
    const outsideNode = nodes.get(outsideNodeId);
    boundaryEdges.push({
      direction: sourceInside ? "outgoing" : "incoming",
      edge_id: edge.id,
      relation: edge.relation,
      inside_node_id: insideNodeId,
      inside_node_kind: insideNode?.payload.kind ?? "missing",
      outside_node_id: outsideNodeId,
      outside_node_kind: outsideNode?.payload.kind ?? "missing",
    });
  }
  boundaryEdges.sort((left, right) => left.edge_id.localeCompare(right.edge_id));
  return {
    kind: "presentation_region_boundary",
    id: region.id,
    label: region.label,
    parent_region_id: region.parent_region_id,
    version: region.version ?? 1,
    derivation: region.derivation ?? null,
    region_usages: (state.project?.presentation?.regions ?? [])
      .filter((candidate) => candidate.derivation?.source_region_id === region.id)
      .map((candidate) => ({
        region_id: candidate.id,
        label: candidate.label,
        version: candidate.version ?? 1,
        derivation_kind: candidate.derivation.kind,
        source_version: candidate.derivation.source_version,
      }))
      .sort((left, right) => left.region_id.localeCompare(right.region_id)),
    enclosed_region_ids: [...enclosedRegions].sort(),
    enclosed_node_ids: [...enclosedNodes].sort(),
    boundary_state_ids: [...new Set(boundaryEdges.flatMap((edge) => [
      edge.inside_node_kind === "execution_state" ? edge.inside_node_id : null,
      edge.outside_node_kind === "execution_state" ? edge.outside_node_id : null,
    ]).filter(Boolean))].sort(),
    boundary_edges: boundaryEdges,
  };
}

function selectRegionBoundary(region) {
  state.selected = { type: "region", value: inspectRegionBoundary(region) };
  state.transitionEvaluation = null;
  renderDetails();
  setStatus(`${region.label}: ${state.selected.value.boundary_edges.length} crossing edge(s)`);
}

function toggleRegionCollapse(regionId) {
  if (state.collapsedRegionIds.has(regionId)) state.collapsedRegionIds.delete(regionId);
  else state.collapsedRegionIds.add(regionId);
  render();
  requestAnimationFrame(fitGraph);
}

function revealNode(node) {
  const regionId = displayedNodeRegionId(node);
  if (!regionId) return;
  state.activeRegionId = regionId;
  state.collapsedRegionIds.delete(regionId);
}

function allowTransitionDrop(event) {
  if (state.workspace && Array.from(event.dataTransfer.types).includes(LIBRARY_DRAG_TYPE)) {
    event.preventDefault();
    event.dataTransfer.dropEffect = "link";
    return;
  }
  if (!state.project?.start_state || state.readOnly) return;
  event.preventDefault();
  event.dataTransfer.dropEffect = "copy";
}

async function dropTransitionAtRouteFrontier(event) {
  const libraryId = event.dataTransfer.getData(LIBRARY_DRAG_TYPE);
  if (libraryId && state.workspace) {
    event.preventDefault();
    const library = state.libraries.find((candidate) => candidate.id === libraryId);
    if (!library) {
      setStatus("The dropped Library item is no longer available", "bad");
      return;
    }
    await addLibraryReference(library);
    return;
  }
  if (!state.project?.start_state || state.readOnly) return;
  event.preventDefault();
  const nodeId = event.dataTransfer.getData("text/plain");
  const node = state.graph?.nodes.find((candidate) =>
    candidate.id === nodeId && candidate.payload.kind === "transition");
  if (!node) {
    setStatus("The dropped palette item is not a projected transition", "bad");
    return;
  }
  const targetElement = event.target.closest?.(".node.reference_step");
  if (targetElement && state.project.route_book) {
    const targetNode = state.graph.nodes.find((candidate) =>
      candidate.id === targetElement.dataset.nodeId);
    const method = state.project.route_book.methods.find((candidate) =>
      candidate.id === "method.authored-route");
    const frontierStepId = method?.step_ids.at(-1);
    if (targetNode?.payload.step_id !== frontierStepId) {
      setStatus(
        `Drop on the current route frontier ${frontierStepId ?? "or the empty canvas"}; middle insertion is ambiguous`,
        "bad",
      );
      return;
    }
  }
  const bounds = elements.canvas.getBoundingClientRect();
  state.positions.set(node.id, {
    x: (event.clientX - bounds.left - state.transform.x) / state.transform.scale - NODE_WIDTH / 2,
    y: (event.clientY - bounds.top - state.transform.y) / state.transform.scale - NODE_HEIGHT / 2,
  });
  state.selected = { type: "node", value: node };
  state.transitionEvaluation = null;
  render();
  await insertSelectedTransition();
}

function renderDetails() {
  const selected = state.selected;
  updateGroupSelectionControl();
  updateRegionControls();
  if (!selected) {
    elements["detail-title"].textContent = "Nothing selected";
    elements["detail-subtitle"].textContent = "Choose a node or connection to inspect its planner-owned identity.";
    elements["detail-json"].textContent = "{}";
    elements["evaluate-transition"].disabled = true;
    elements["solve-goal"].disabled = true;
    elements["insert-transition"].disabled = true;
    elements["suggest-transition-chain"].disabled = true;
    elements["suggest-transition-chain"].textContent = "Find producer chain";
    elements["replace-step"].disabled = true;
    elements["remove-step"].disabled = true;
    updateDirectiveControls(null);
    renderContractInspector(null);
    renderStateInspector();
    return;
  }
  elements["detail-title"].textContent = selected.type === "node" || selected.type === "region"
    ? selected.value.label
    : selected.value.relation;
  elements["detail-subtitle"].textContent = selected.value.id;
  const transition = selected.type === "node" && selected.value.payload.kind === "transition";
  const goal = selected.type === "node" && selected.value.payload.kind === "goal";
  const routeStep = selected.type === "node" && selected.value.payload.kind === "reference_step";
  elements["evaluate-transition"].disabled = !transition || !state.project?.start_state;
  elements["solve-goal"].disabled = !goal || !state.project?.start_state;
  elements["insert-transition"].disabled = !transition || !state.project?.start_state || state.readOnly;
  const suggestion = state.transitionEvaluation?.suggestion;
  const rejectedJoin = state.transitionEvaluation?.kind === "rejected_transition_join";
  elements["suggest-transition-chain"].disabled = !transition || !rejectedJoin
    || !state.project?.start_state || state.readOnly;
  elements["suggest-transition-chain"].textContent = suggestion?.transition_ids?.length
    ? `Insert ${suggestion.transition_ids.length}-step chain`
    : "Find producer chain";
  elements["replace-step"].disabled = (!routeStep && !(transition && state.replacementStep))
    || !state.project?.start_state || state.readOnly;
  elements["replace-step"].textContent = transition && state.replacementStep
    ? `Replace ${state.replacementStep.label}`
    : "Choose replacement transition";
  elements["remove-step"].disabled = !routeStep || !state.project?.start_state || state.readOnly;
  updateDirectiveControls(selected.type === "node" ? selected.value : null);
  elements["evaluate-transition"].title = state.project?.start_state
    ? "Run the authoritative transition evaluator"
    : "This project has no exact start state";
  const contract = selected.type === "node" ? selectedContract(selected.value) : null;
  elements["detail-json"].textContent = JSON.stringify({
    selection: selected.value,
    ...(contract ? { authoritative_contract: contract } : {}),
    ...(state.replacementStep ? { replacement_target: state.replacementStep } : {}),
    ...(state.transitionEvaluation ? { transition_evaluation: state.transitionEvaluation } : {}),
    ...(state.solveReport ? { solve_report: state.solveReport } : {}),
  }, null, 2);
  renderContractInspector(contract);
  renderStateInspector();
}

function updateGroupSelectionControl() {
  const count = state.groupSelection.size;
  elements["group-selection"].disabled = count === 0 || state.readOnly;
  elements["group-selection"].textContent = count
    ? `Group ${count} selected node${count === 1 ? "" : "s"}`
    : "Group selected nodes";
}

function selectedPresentationRegion() {
  if (state.selected?.type !== "region") return null;
  return (state.project?.presentation?.regions ?? []).find((region) =>
    region.id === state.selected.value.id) ?? null;
}

function updateRegionControls() {
  const region = selectedPresentationRegion();
  const editable = Boolean(region && !state.readOnly);
  for (const id of [
    "copy-region", "fork-region", "reference-region", "version-region", "region-usage",
  ]) elements[id].disabled = !editable;
  elements["replace-region"].disabled = !editable
    || (state.project?.presentation?.regions?.length ?? 0) < 2;
}

function nextPresentationRegionId(label) {
  const existing = new Set(displayedRegions().map((region) => region.id));
  const base = `region.presentation-${slug(label)}`;
  let id = base;
  let suffix = 2;
  while (existing.has(id)) id = `${base}-${suffix++}`;
  return id;
}

function createRegionDerivative(kind) {
  const source = selectedPresentationRegion();
  if (!source || state.readOnly) return;
  const defaultLabels = {
    copy: `${source.label} copy`,
    fork: `${source.label} fork`,
    reference: `${source.label} reference`,
    version: `${source.label} v${(source.version ?? 1) + 1}`,
  };
  const label = prompt("Derived region name", defaultLabels[kind])?.trim();
  if (!label) return;
  const boundary = inspectRegionBoundary(source);
  const derived = {
    id: nextPresentationRegionId(label),
    label,
    parent_region_id: source.parent_region_id ?? null,
    version: kind === "version" ? (source.version ?? 1) + 1 : 1,
    snapshot_node_ids: kind === "reference" ? [] : [...boundary.enclosed_node_ids].sort(),
    derivation: {
      kind,
      source_region_id: source.id,
      source_version: source.version ?? 1,
    },
  };
  state.project.presentation.regions.push(derived);
  state.activeRegionId = derived.id;
  state.selected = { type: "region", value: inspectRegionBoundary(derived) };
  markDirty();
  render();
  requestAnimationFrame(fitGraph);
  setStatus(`${derived.label} created as ${kind} of ${source.label}; save to persist`, "good");
}

function replaceRegionFromSelection() {
  const source = selectedPresentationRegion();
  if (!source || state.readOnly) return;
  const candidates = state.project.presentation.regions.filter((region) => region.id !== source.id);
  const entered = prompt(
    `Replace which region with ${source.label}?`,
    candidates[0]?.id ?? "",
  )?.trim();
  const target = candidates.find((region) => region.id === entered);
  if (!target) {
    setStatus("Replacement target must be an existing presentation region ID", "bad");
    return;
  }
  const boundary = inspectRegionBoundary(source);
  target.version = (target.version ?? 1) + 1;
  target.snapshot_node_ids = [...boundary.enclosed_node_ids].sort();
  target.derivation = {
    kind: "replacement",
    source_region_id: source.id,
    source_version: source.version ?? 1,
  };
  state.activeRegionId = target.id;
  state.selected = { type: "region", value: inspectRegionBoundary(target) };
  markDirty();
  render();
  requestAnimationFrame(fitGraph);
  setStatus(`${target.label} replaced from ${source.label} at version ${target.version}; save to persist`, "good");
}

function inspectSelectedRegionUsage() {
  const region = selectedPresentationRegion();
  if (!region) return;
  state.selected = { type: "region", value: inspectRegionBoundary(region) };
  renderDetails();
  const count = state.selected.value.region_usages.length;
  setStatus(`${region.label} has ${count} derived usage${count === 1 ? "" : "s"}`);
}

function toggleGroupSelection(node) {
  if (state.groupSelection.has(node.id)) state.groupSelection.delete(node.id);
  else state.groupSelection.add(node.id);
}

function groupSelectedNodes() {
  if (!state.groupSelection.size || state.readOnly) return;
  const label = prompt("Nested region name", "New region")?.trim();
  if (!label) return;
  const id = nextPresentationRegionId(label);
  const presentation = state.project.presentation ?? { positions: {} };
  const regions = [...(presentation.regions ?? []), {
    id,
    label,
    parent_region_id: state.activeRegionId ?? null,
    version: 1,
    snapshot_node_ids: [],
    derivation: null,
  }];
  const nodeRegionIds = { ...(presentation.node_region_ids ?? {}) };
  for (const nodeId of state.groupSelection) nodeRegionIds[nodeId] = id;
  state.project.presentation = {
    ...presentation,
    regions,
    node_region_ids: nodeRegionIds,
  };
  state.groupSelection = new Set();
  state.activeRegionId = id;
  state.collapsedRegionIds.delete(id);
  markDirty();
  render();
  requestAnimationFrame(fitGraph);
  setStatus(`${label} grouped as presentation-only graph region; save to persist`, "good");
}

function selectedContract(node) {
  const catalog = state.project?.catalog;
  const mechanics = catalog?.mechanics;
  if (!mechanics) return null;
  if (node.payload.kind === "alias") {
    return catalog.facts.aliases.find((record) => record.id === node.payload.fact_id) ?? null;
  }
  if (node.payload.kind === "derived_fact") {
    return catalog.facts.derived_facts.find((record) => record.id === node.payload.fact_id) ?? null;
  }
  const lookups = {
    transition: [mechanics.transitions, "transition_id"],
    obligation: [mechanics.obligations, "obligation_id"],
    obstruction: [mechanics.obstructions, "obstruction_id"],
    resolver: [mechanics.resolvers, "resolver_id"],
    technique: [mechanics.techniques, "technique_id"],
    writer: [mechanics.writers, "writer_id"],
    gate: [mechanics.gates, "gate_id"],
    reader: [mechanics.readers, "reader_id"],
    reconstruction: [mechanics.reconstruction_rules, "reconstruction_rule_id"],
    microtrace: [mechanics.microtraces, "microtrace_id"],
    goal: [mechanics.goals, "goal_id"],
  };
  const lookup = lookups[node.payload.kind];
  if (!lookup) return null;
  const [records, idField] = lookup;
  return records.find((record) => record.id === node.payload[idField]) ?? null;
}

function renderContractInspector(contract) {
  const container = elements["contract-inspector"];
  container.replaceChildren();
  if (!contract) {
    container.hidden = true;
    return;
  }
  container.hidden = false;
  const heading = document.createElement("h3");
  heading.textContent = "Authoritative contract";
  const card = document.createElement("section");
  card.className = "state-card";
  const title = document.createElement("h4");
  title.textContent = contract.label ?? contract.id;
  const metrics = document.createElement("dl");
  const evidence = contract.evidence?.truth ?? contract.evidence?.records?.[0]?.kind ?? "modeled";
  const action = contract.blocked_action_id ?? contract.obstruction_id ?? contract.gate_id
    ?? contract.transition_id ?? "—";
  const operations = contract.operations?.length
    ?? contract.effects?.operations?.length
    ?? contract.effect?.operations?.length
    ?? 0;
  const rows = [
    ["ID", contract.id],
    ["Evidence", taggedValue(evidence)],
    ["Scope", `${contract.scope?.selectors?.length ?? 0} exact selector(s)`],
    ["Related", action],
    ["Operations", operations],
  ];
  for (const [name, value] of rows) {
    const term = document.createElement("dt");
    term.textContent = name;
    const detail = document.createElement("dd");
    detail.textContent = String(value);
    detail.title = String(value);
    metrics.append(term, detail);
  }
  card.append(title, metrics);
  container.append(heading, card);
}

function transitionSearchText(transition) {
  const tokens = [];
  const visit = (value) => {
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
      tokens.push(String(value));
    } else if (Array.isArray(value)) {
      value.forEach(visit);
    } else if (value && typeof value === "object") {
      for (const [key, child] of Object.entries(value)) {
        tokens.push(key.replaceAll("_", " "));
        visit(child);
      }
    }
  };
  visit(transition);
  return tokens.join(" ");
}

function updateDirectiveControls(node) {
  const target = directiveTarget(node);
  const editable = Boolean(target && state.project?.route_book && !state.readOnly);
  const active = target ? activeDirectives(target) : [];
  for (const [mode, id] of [
    ["pin", "pin-selection"],
    ["ban", "ban-selection"],
    ["prefer", "prefer-selection"],
  ]) {
    const button = elements[id];
    const isActive = active.some((directive) => directiveMode(directive) === mode);
    button.disabled = !editable;
    button.classList.toggle("active", isActive);
    button.textContent = isActive ? `Un${mode}` : `${mode[0].toUpperCase()}${mode.slice(1)}`;
  }
  const method = node?.payload.kind === "plan_method"
    ? state.project?.route_book?.methods.find((candidate) => candidate.id === node.payload.method_id)
    : null;
  const region = method
    ? state.project.route_book.regions.find((candidate) => candidate.id === method.region_id)
    : null;
  const selected = Boolean(method && region?.selected_method_id === method.id);
  elements["select-method"].disabled = !editable || !method;
  elements["select-method"].classList.toggle("active", selected);
  elements["select-method"].textContent = selected ? "Clear selection" : "Select method";
}

function directiveTarget(node) {
  if (!node || !state.project?.route_book) return null;
  if (node.payload.kind === "transition") {
    const transition = state.project.catalog.mechanics.transitions.find((candidate) =>
      candidate.id === node.payload.transition_id);
    return transition ? {
      type: "action",
      id: transition.id,
      scope: transition.scope,
      action: { kind: "transition", transition_id: transition.id },
    } : null;
  }
  if (node.payload.kind === "plan_method") {
    const method = state.project.route_book.methods.find((candidate) =>
      candidate.id === node.payload.method_id);
    return method ? { type: "method", id: method.id, scope: method.scope } : null;
  }
  return null;
}

function activeDirectives(target) {
  return (state.project?.route_book?.directives ?? []).filter((record) => {
    if (target.type === "method") return record.directive.method_id === target.id;
    return record.directive.action?.kind === target.action.kind
      && record.directive.action.transition_id === target.action.transition_id;
  });
}

function directiveMode(record) {
  if (record.directive.kind.startsWith("pin_")) return "pin";
  if (record.directive.kind.startsWith("ban_")) return "ban";
  if (record.directive.kind.startsWith("prefer_")) return "prefer";
  return "";
}

function routePreferenceClass(node) {
  const target = directiveTarget(node);
  if (!target) return "";
  const active = activeDirectives(target).map(directiveMode);
  if (active.includes("ban")) return "directive-banned";
  if (active.includes("pin")) return "directive-pinned";
  if (active.includes("prefer")) return "directive-preferred";
  if (node.payload.kind === "plan_method") {
    const method = state.project.route_book.methods.find((candidate) => candidate.id === target.id);
    const region = state.project.route_book.regions.find((candidate) => candidate.id === method.region_id);
    if (region?.selected_method_id === method.id) return "method-selected";
  }
  return "";
}

async function editSelectedDirective(mode) {
  const node = state.selected?.type === "node" ? state.selected.value : null;
  const target = directiveTarget(node);
  if (!target || state.readOnly) return;
  const active = activeDirectives(target);
  const same = active.find((record) => directiveMode(record) === mode);
  let weight = 1;
  if (mode === "prefer" && !same) {
    const entered = prompt("Preference weight", "1");
    if (entered == null) return;
    weight = Number(entered);
    if (!Number.isInteger(weight) || weight <= 0 || weight > 0xffffffff) {
      setStatus("Preference weight must be an integer from 1 through 4294967295", "bad");
      return;
    }
  }
  const edits = active.map((record) => ({
    kind: "remove_directive",
    directive_id: record.id,
  }));
  if (!same) {
    const noun = target.type === "action" ? "action" : "method";
    const directive = target.type === "action"
      ? { kind: `${mode}_action`, action: target.action }
      : { kind: `${mode}_method`, method_id: target.id };
    if (mode === "prefer") directive.weight = weight;
    edits.push({
      kind: "upsert_directive",
      directive: {
        id: `directive.browser-${mode}-${noun}-${slug(target.id)}`,
        scope: target.scope,
        directive,
      },
    });
  }
  await editRouteBook(
    edits,
    same ? `${node.label} ${mode} removed` : `${node.label} marked ${mode}`,
  );
}

async function toggleSelectedMethod() {
  const node = state.selected?.type === "node" ? state.selected.value : null;
  if (node?.payload.kind !== "plan_method" || !state.project?.route_book || state.readOnly) return;
  const method = state.project.route_book.methods.find((candidate) =>
    candidate.id === node.payload.method_id);
  const region = state.project.route_book.regions.find((candidate) =>
    candidate.id === method?.region_id);
  if (!method || !region) return;
  const selected = region.selected_method_id === method.id;
  await editRouteBook([{
    kind: "set_selected_method",
    region_id: region.id,
    method_id: selected ? null : method.id,
  }], selected ? `${method.label} selection cleared` : `${method.label} selected`);
}

async function editRouteBook(edits, message) {
  try {
    setStatus("Validating route-book revision...");
    const validated = await service({
      command: "validate_route_book",
      request_id: requestId("validate-route-book"),
      book: state.project.route_book,
      catalog: state.project.catalog,
    });
    if (validated.kind !== "route_book_valid") {
      throw new Error(`Unexpected response ${validated.kind}`);
    }
    const edited = await service({
      command: "edit_route_book",
      request_id: requestId("edit-route-book"),
      book: state.project.route_book,
      catalog: state.project.catalog,
      edit_batch: {
        schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA,
        expected_route_book_sha256: validated.route_book_sha256,
        edits,
      },
    });
    if (edited.kind !== "edited_route_book") {
      throw new Error(`Unexpected response ${edited.kind}`);
    }
    const selectedId = state.selected?.type === "node" ? state.selected.value.id : null;
    state.project.route_book = edited.book;
    const projected = await service({
      command: "project_graph",
      request_id: requestId("project-after-route-book-edit"),
      catalog: state.project.catalog,
      route_book: state.project.route_book,
    });
    if (projected.kind !== "graph") throw new Error(`Unexpected response ${projected.kind}`);
    state.graph = projected.graph;
    state.solveReport = null;
    ensurePositions();
    const selected = state.graph.nodes.find((candidate) => candidate.id === selectedId);
    state.selected = selected ? { type: "node", value: selected } : null;
    markDirty();
    render();
    setStatus(`${message}; save to persist`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

function renderStateInspector() {
  const container = elements["state-inspector"];
  container.replaceChildren();
  const change = state.transitionEvaluation?.state_change;
  if (!change?.before) {
    container.hidden = true;
    return;
  }
  container.hidden = false;
  const heading = document.createElement("h3");
  heading.textContent = change.after ? "Exact state change" : "Exact inspected state";
  const columns = document.createElement("div");
  columns.className = "state-columns";
  columns.append(stateInspectionCard("Before", change.before));
  if (change.after) columns.append(stateInspectionCard("After", change.after));
  container.append(heading, columns);
  const deltas = stateDeltaChips(change.diff);
  if (deltas.length) {
    const list = document.createElement("div");
    list.className = "state-deltas";
    list.append(...deltas);
    container.append(list);
  } else if (change.after) {
    const empty = document.createElement("p");
    empty.className = "state-empty";
    empty.textContent = "No modeled state deltas.";
    container.append(empty);
  }
}

function stateInspectionCard(label, inspection) {
  const environment = inspection.state.snapshot.environment;
  const location = environment.location;
  const card = document.createElement("section");
  card.className = "state-card";
  const title = document.createElement("h4");
  title.textContent = label;
  const facts = inspection.facts.reduce((counts, fact) => {
    counts[fact.evaluated] = (counts[fact.evaluated] ?? 0) + 1;
    return counts;
  }, {});
  const metrics = document.createElement("dl");
  for (const [name, value] of [
    ["Location", `${location.stage} r${location.room} l${location.layer} s${location.spawn}`],
    ["Context", taggedValue(environment.execution_context)],
    ["Runtime", environment.active_runtime_file.id],
    ["Player", `${taggedValue(environment.player.form)} · ${environment.player.action}`],
    ["Components", environment.components.length],
    ["Facts", `${facts.true ?? 0} true · ${facts.false ?? 0} false · ${facts.unknown ?? 0} unknown`],
  ]) {
    const term = document.createElement("dt");
    term.textContent = name;
    const detail = document.createElement("dd");
    detail.textContent = String(value);
    detail.title = String(value);
    metrics.append(term, detail);
  }
  card.append(title, metrics);
  return card;
}

function taggedValue(value) {
  if (value == null) return "none";
  if (typeof value === "string") return value.replaceAll("_", " ");
  if (typeof value.kind === "string") return value.kind.replaceAll("_", " ");
  return "modeled";
}

function stateDeltaChips(diff) {
  if (!diff) return [];
  const stateDiff = diff.state_diff;
  const values = [
    [stateDiff.location_changed, "location changed", "changed"],
    [stateDiff.execution_context_changed, "context changed", "changed"],
    [stateDiff.player_changed, "player changed", "changed"],
    [stateDiff.component_deltas.length, `${stateDiff.component_deltas.length} component delta(s)`, "changed"],
    [stateDiff.semantic_deltas.length, `${stateDiff.semantic_deltas.length} semantic delta(s)`, "fact"],
    [diff.fact_deltas.length, `${diff.fact_deltas.length} evaluated fact delta(s)`, "fact"],
    [diff.gate_state_deltas.length, `${diff.gate_state_deltas.length} gate delta(s)`, "changed"],
    [diff.serialized_store_deltas.length, `${diff.serialized_store_deltas.length} store delta(s)`, "changed"],
    [diff.persistent_file_image_deltas.length, `${diff.persistent_file_image_deltas.length} file-image delta(s)`, "changed"],
  ];
  return values.filter(([present]) => Boolean(present)).map(([, label, className]) => {
    const chip = document.createElement("span");
    chip.className = `state-delta ${className}`;
    chip.textContent = label;
    return chip;
  });
}

async function removeSelectedRouteStep() {
  const node = state.selected?.type === "node" ? state.selected.value : null;
  if (node?.payload.kind !== "reference_step" || !state.project?.start_state
    || !state.project?.route_book || state.readOnly) return;
  try {
    setStatus(`Replaying route without ${node.label}...`);
    const payload = await service({
      command: "remove_authored_step",
      request_id: requestId("remove-step"),
      state: state.project.start_state,
      catalog: state.project.catalog,
      equivalence_sets: state.project.equivalence_sets ?? [],
      route_book: state.project.route_book,
      step_id: node.payload.step_id,
      evidence_mode: projectEvidenceMode(),
    });
    if (payload.kind === "rejected_route_edit") {
      state.transitionEvaluation = {
        ...payload,
        state_change: await inspectStateChange(
          state.project.start_state,
          payload.closest_before,
          "remove-rejection",
        ),
      };
      render();
      setStatus(
        `${joinRejectionSummary(node.label, payload, "removed")}; first broken downstream step ${payload.failed_step_id}`,
        "bad",
      );
      return;
    }
    if (payload.kind !== "removed_authored_step") {
      throw new Error(`Unexpected response ${payload.kind}`);
    }
    state.project.route_book = payload.book ?? null;
    if (state.replacementStep?.step_id === node.payload.step_id) state.replacementStep = null;
    const projected = await service({
      command: "project_graph",
      request_id: requestId("project-after-remove"),
      catalog: state.project.catalog,
      route_book: state.project.route_book,
    });
    if (projected.kind !== "graph") throw new Error(`Unexpected response ${projected.kind}`);
    state.graph = projected.graph;
    state.solveReport = null;
    await refreshAuthoredRouteInspections();
    state.selected = null;
    state.transitionEvaluation = null;
    ensurePositions();
    markDirty();
    render();
    setStatus(`${node.label} removed; downstream state replayed; save to persist`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function replaceSelectedRouteStep() {
  const node = state.selected?.type === "node" ? state.selected.value : null;
  if (!node || !state.project?.start_state || !state.project?.route_book || state.readOnly) return;
  if (node.payload.kind === "reference_step") {
    state.replacementStep = { step_id: node.payload.step_id, label: node.label };
    state.transitionEvaluation = null;
    render();
    setStatus(`Choose a transition to replace ${node.label}`);
    return;
  }
  if (node.payload.kind !== "transition" || !state.replacementStep) return;
  const replacement = state.replacementStep;
  try {
    setStatus(`Replaying route with ${node.label} replacing ${replacement.label}...`);
    const payload = await service({
      command: "replace_authored_step",
      request_id: requestId("replace-step"),
      state: state.project.start_state,
      catalog: state.project.catalog,
      equivalence_sets: state.project.equivalence_sets ?? [],
      route_book: state.project.route_book,
      step_id: replacement.step_id,
      transition_id: node.payload.transition_id,
      evidence_mode: projectEvidenceMode(),
    });
    if (payload.kind === "rejected_route_edit") {
      state.transitionEvaluation = {
        ...payload,
        state_change: await inspectStateChange(
          state.project.start_state,
          payload.closest_before,
          "replace-rejection",
        ),
      };
      render();
      setStatus(
        `${joinRejectionSummary(node.label, payload, "used as a replacement")}; first broken step ${payload.failed_step_id}`,
        "bad",
      );
      return;
    }
    if (payload.kind !== "replaced_authored_step") {
      throw new Error(`Unexpected response ${payload.kind}`);
    }
    state.project.route_book = payload.book;
    const stateChange = await inspectStateChange(
      state.project.start_state,
      payload.after,
      "replace-transition",
    );
    const projected = await service({
      command: "project_graph",
      request_id: requestId("project-after-replace"),
      catalog: state.project.catalog,
      route_book: state.project.route_book,
    });
    if (projected.kind !== "graph") throw new Error(`Unexpected response ${projected.kind}`);
    state.graph = projected.graph;
    state.solveReport = null;
    state.transitionEvaluation = {
      kind: payload.kind,
      step_id: payload.step_id,
      transition_id: payload.transition_id,
      route_book_sha256: payload.route_book_sha256,
      assessment: payload.assessment,
      after: payload.after,
      state_change: stateChange,
    };
    state.replacementStep = null;
    await refreshAuthoredRouteInspections();
    ensurePositions();
    const stepNode = state.graph.nodes.find((candidate) =>
      candidate.payload.kind === "reference_step" && candidate.payload.step_id === payload.step_id);
    if (stepNode) selectNode(stepNode);
    else state.selected = null;
    markDirty();
    render();
    setStatus(`${replacement.label} replaced with ${node.label}; downstream state replayed; save to persist`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function insertSelectedTransition() {
  const node = state.selected?.type === "node" ? state.selected.value : null;
  if (node?.payload.kind !== "transition" || !state.project?.start_state || state.readOnly) return;
  try {
    setStatus(`Propagating and inserting ${node.label}...`);
    const payload = await service({
      command: "append_transition",
      request_id: requestId("append-transition"),
      state: state.project.start_state,
      catalog: state.project.catalog,
      equivalence_sets: state.project.equivalence_sets ?? [],
      route_book: state.project.route_book ?? null,
      route_book_id: `route.${slug(state.project.id)}`,
      route_book_label: state.project.label,
      transition_id: node.payload.transition_id,
      evidence_mode: projectEvidenceMode(),
    });
    if (payload.kind === "rejected_transition_join") {
      state.transitionEvaluation = {
        ...payload,
        state_change: await inspectStateChange(
          state.project.start_state,
          payload.closest_before,
          "append-rejection",
        ),
      };
      render();
      setStatus(joinRejectionSummary(node.label, payload), "bad");
      return;
    }
    if (payload.kind !== "appended_transition") {
      throw new Error(`Unexpected response ${payload.kind}`);
    }
    state.project.route_book = payload.book;
    const stateChange = await inspectStateChange(
      state.project.start_state,
      payload.after,
      "append-transition",
    );
    state.transitionEvaluation = {
      kind: payload.kind,
      step_id: payload.step_id,
      route_book_sha256: payload.route_book_sha256,
      assessment: payload.assessment,
      after: payload.after,
      state_change: stateChange,
    };
    const projected = await service({
      command: "project_graph",
      request_id: requestId("project-after-append"),
      catalog: state.project.catalog,
      route_book: state.project.route_book,
    });
    if (projected.kind !== "graph") throw new Error(`Unexpected response ${projected.kind}`);
    state.graph = projected.graph;
    state.solveReport = null;
    await refreshAuthoredRouteInspections();
    ensurePositions();
    const stepNode = state.graph.nodes.find((candidate) =>
      candidate.payload.kind === "reference_step" && candidate.payload.step_id === payload.step_id);
    if (stepNode) selectNode(stepNode);
    markDirty();
    render();
    setStatus(`${node.label} inserted as ${payload.step_id}; save to persist`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function suggestOrInsertSelectedTransitionChain() {
  const node = state.selected?.type === "node" ? state.selected.value : null;
  if (node?.payload.kind !== "transition" || state.readOnly
    || state.transitionEvaluation?.kind !== "rejected_transition_join") return;
  const prior = state.transitionEvaluation.suggestion;
  if (prior?.transition_ids?.length) {
    await insertSuggestedTransitionChain(prior);
    return;
  }
  try {
    setStatus(`Searching exact producer chains for ${node.label}...`);
    const payload = await service({
      command: "suggest_transition_chain",
      request_id: requestId("suggest-transition-chain"),
      state: state.project.start_state,
      catalog: state.project.catalog,
      equivalence_sets: state.project.equivalence_sets ?? [],
      route_book: state.project.route_book ?? null,
      transition_id: node.payload.transition_id,
      evidence_mode: projectEvidenceMode(),
      max_depth: 12,
      max_states: 2048,
    });
    if (payload.kind !== "transition_chain_suggestion") {
      throw new Error(`Unexpected response ${payload.kind}`);
    }
    state.transitionEvaluation = { ...state.transitionEvaluation, suggestion: payload };
    render();
    if (payload.transition_ids.length) {
      const labels = payload.transition_ids.map((id) =>
        state.project.catalog.mechanics.transitions.find((candidate) => candidate.id === id)?.label ?? id);
      setStatus(`Suggested exact chain: ${labels.join(" → ")}`, "good");
    } else {
      const limit = payload.hit_search_limit ? " within the bounded search" : "";
      setStatus(`No executable producer chain found${limit}; rejection remains explicit`, "bad");
    }
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function insertSuggestedTransitionChain(suggestion) {
  const node = state.selected?.type === "node" ? state.selected.value : null;
  if (node?.payload.kind !== "transition" || !suggestion.transition_ids.length || state.readOnly) return;
  try {
    setStatus(`Validating and inserting ${suggestion.transition_ids.length}-step producer chain...`);
    let book = state.project.route_book ?? null;
    let appended = null;
    for (const transitionId of suggestion.transition_ids) {
      const payload = await service({
        command: "append_transition",
        request_id: requestId("append-suggested-transition"),
        state: state.project.start_state,
        catalog: state.project.catalog,
        equivalence_sets: state.project.equivalence_sets ?? [],
        route_book: book,
        route_book_id: `route.${slug(state.project.id)}`,
        route_book_label: state.project.label,
        transition_id: transitionId,
        evidence_mode: projectEvidenceMode(),
      });
      if (payload.kind !== "appended_transition") {
        throw new Error(`Suggested chain changed before insertion at ${transitionId}`);
      }
      book = payload.book;
      appended = payload;
    }
    state.project.route_book = book;
    const projected = await service({
      command: "project_graph",
      request_id: requestId("project-after-suggested-chain"),
      catalog: state.project.catalog,
      route_book: state.project.route_book,
    });
    if (projected.kind !== "graph") throw new Error(`Unexpected response ${projected.kind}`);
    state.graph = projected.graph;
    state.solveReport = null;
    await refreshAuthoredRouteInspections();
    ensurePositions();
    const stepNode = state.graph.nodes.find((candidate) =>
      candidate.payload.kind === "reference_step" && candidate.payload.step_id === appended.step_id);
    if (stepNode) selectNode(stepNode);
    else state.selected = null;
    markDirty();
    render();
    setStatus(`${suggestion.transition_ids.length}-step producer chain inserted; save to persist`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function evaluateSelectedTransition() {
  const node = state.selected?.type === "node" ? state.selected.value : null;
  const frontierState = state.routeFrontier?.frontier_state ?? state.project?.start_state;
  if (node?.payload.kind !== "transition" || !frontierState) return;
  try {
    setStatus(`Evaluating ${node.label}...`);
    const payload = await service({
      command: "evaluate_transition",
      request_id: requestId("transition"),
      state: frontierState,
      catalog: state.project.catalog,
      equivalence_sets: state.project.equivalence_sets ?? [],
      transition_id: node.payload.transition_id,
      evidence_mode: projectEvidenceMode(),
    });
    if (payload.kind !== "transition_evaluation") {
      throw new Error(`Unexpected response ${payload.kind}`);
    }
    state.transitionEvaluation = {
      ...payload,
      state_change: await inspectStateChange(
        frontierState,
        payload.after ?? null,
        "evaluate-transition",
      ),
    };
    renderDetails();
    const accepted = payload.assessment.classification === "executable";
    setStatus(
      accepted ? `${node.label} is executable from the exact route frontier`
        : `${node.label}: ${payload.assessment.classification.replaceAll("_", " ")}`,
      accepted ? "good" : "bad",
    );
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function solveSelectedGoal() {
  const node = state.selected?.type === "node" ? state.selected.value : null;
  if (node?.payload.kind !== "goal" || !state.project?.start_state) return;
  try {
    setStatus(`Solving ${node.label}...`);
    const payload = await service({
      command: "solve",
      request_id: requestId("solve-goal"),
      state: state.project.start_state,
      catalog: state.project.catalog,
      equivalence_sets: state.project.equivalence_sets ?? [],
      goal_id: node.payload.goal_id,
      options: {
        max_depth: 64,
        max_states: 50_000,
        max_resolution_combinations: 256,
        max_plans: 8,
        feasibility_mode: "modeled",
        evidence_mode: projectEvidenceMode(),
      },
      route_book: state.project.route_book ?? null,
    });
    if (payload.kind !== "solve_report") {
      throw new Error(`Unexpected response ${payload.kind}`);
    }
    state.graph = payload.proof_graph;
    state.solveReport = payload.report;
    state.transitionEvaluation = null;
    state.activeRegionId = "region.proof";
    state.collapsedRegionIds = new Set();
    state.knownRegionIds = new Set();
    ensurePositions();
    const selected = state.graph.nodes.find((candidate) => candidate.id === node.id);
    state.selected = selected ? { type: "node", value: selected } : null;
    render();
    requestAnimationFrame(fitGraph);
    const plans = 1 + (payload.report.summary.alternative_plans?.length ?? 0);
    setStatus(
      payload.report.summary.status === "reached"
        ? `${node.label} reached; projected ${plans} solver plan${plans === 1 ? "" : "s"}`
        : `${node.label}: ${payload.report.summary.status.replaceAll("_", " ")}`,
      payload.report.summary.status === "reached" ? "good" : "bad",
    );
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function inspectStateChange(before, after, id) {
  const beforePayload = await service({
    command: "inspect_state",
    request_id: requestId(`${id}-before`),
    state: before,
    catalog: state.project.catalog,
    equivalence_sets: state.project.equivalence_sets ?? [],
    evidence_mode: projectEvidenceMode(),
  });
  if (beforePayload.kind !== "state_inspection") {
    throw new Error(`Unexpected response ${beforePayload.kind}`);
  }
  if (!after || after.snapshot.sequence <= before.snapshot.sequence) {
    return { before: beforePayload.inspection, after: null, diff: null };
  }
  const afterPayload = await service({
    command: "inspect_state",
    request_id: requestId(`${id}-after`),
    state: after,
    catalog: state.project.catalog,
    equivalence_sets: state.project.equivalence_sets ?? [],
    evidence_mode: projectEvidenceMode(),
  });
  if (afterPayload.kind !== "state_inspection") {
    throw new Error(`Unexpected response ${afterPayload.kind}`);
  }
  const diffPayload = await service({
    command: "diff_state",
    request_id: requestId(`${id}-diff`),
    before,
    after,
    boundary: { kind: "custom", id: `browser.${id}` },
    catalog: state.project.catalog,
    equivalence_sets: state.project.equivalence_sets ?? [],
    evidence_mode: projectEvidenceMode(),
  });
  if (diffPayload.kind !== "state_inspection_diff") {
    throw new Error(`Unexpected response ${diffPayload.kind}`);
  }
  return {
    before: beforePayload.inspection,
    after: afterPayload.inspection,
    diff: diffPayload.inspection_diff,
  };
}

function transitionJoinClass(node) {
  if (state.selected?.type !== "node" || state.selected.value.id !== node.id) return "";
  const classification = state.transitionEvaluation?.assessment?.classification;
  if (classification === "executable") return "join-accepted";
  if (classification === "feasibility_unknown") return "join-unknown";
  return classification ? "join-rejected" : "";
}

function joinRejectionSummary(label, payload, action = "inserted") {
  const assessment = payload.assessment;
  const diagnostics = payload.diagnostics;
  const reasons = [
    ...assessment.outstanding_obligation_ids.map((id) => `missing ${id}`),
    ...assessment.unknown_obligation_ids.map((id) => `unknown ${id}`),
    ...assessment.unknown_requirement_ids.map((id) => `unknown ${id}`),
    ...diagnostics.active_obstruction_ids.map((id) => `obstructed by ${id}`),
    ...diagnostics.unknown_obstruction_ids.map((id) => `unknown obstruction ${id}`),
  ];
  if (!assessment.scope_applies) reasons.push("exact context does not apply");
  if (!assessment.evidence_permitted) reasons.push("evidence policy rejects this transition");
  const detail = reasons.length ? reasons.join(", ") : assessment.classification.replaceAll("_", " ");
  return `${label} was not ${action}: ${detail}`;
}

function beginPan(event) {
  if (event.button !== 0 || event.target.closest?.(".node")) return;
  elements.canvas.setPointerCapture(event.pointerId);
  state.selected = null;
  state.transitionEvaluation = null;
  state.gesture = { kind: "pan", pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, x: state.transform.x, y: state.transform.y };
  renderDetails();
}

function beginNodeDrag(event, node) {
  if (event.button !== 0) return;
  event.stopPropagation();
  if (event.shiftKey) return;
  elements.canvas.setPointerCapture(event.pointerId);
  const position = state.positions.get(node.id);
  selectNode(node);
  state.gesture = { kind: "node", pointerId: event.pointerId, nodeId: node.id, startX: event.clientX, startY: event.clientY, x: position.x, y: position.y };
  renderDetails();
}

function moveGesture(event) {
  const gesture = state.gesture;
  if (!gesture || event.pointerId !== gesture.pointerId) return;
  const dx = event.clientX - gesture.startX;
  const dy = event.clientY - gesture.startY;
  if (gesture.kind === "pan") {
    state.transform.x = gesture.x + dx;
    state.transform.y = gesture.y + dy;
    applyTransform();
  } else {
    state.positions.set(gesture.nodeId, { x: gesture.x + dx / state.transform.scale, y: gesture.y + dy / state.transform.scale });
    renderEdges();
    const node = elements.nodes.querySelector(`[data-node-id="${CSS.escape(gesture.nodeId)}"]`);
    const position = state.positions.get(gesture.nodeId);
    node?.setAttribute("transform", `translate(${position.x} ${position.y})`);
  }
}

function endGesture(event) {
  if (!state.gesture || event.pointerId !== state.gesture.pointerId) return;
  const changedLayout = state.gesture.kind === "node"
    && (event.clientX !== state.gesture.startX || event.clientY !== state.gesture.startY);
  state.gesture = null;
  if (elements.canvas.hasPointerCapture(event.pointerId)) elements.canvas.releasePointerCapture(event.pointerId);
  if (changedLayout) markDirty();
}

function onWheel(event) {
  event.preventDefault();
  const bounds = elements.canvas.getBoundingClientRect();
  const point = { x: event.clientX - bounds.left, y: event.clientY - bounds.top };
  setScale(state.transform.scale * Math.exp(-event.deltaY * 0.0012), point);
}

function zoomAt(factor) {
  const bounds = elements.canvas.getBoundingClientRect();
  setScale(state.transform.scale * factor, { x: bounds.width / 2, y: bounds.height / 2 });
}

function setScale(next, point) {
  next = Math.min(2.75, Math.max(0.18, next));
  const ratio = next / state.transform.scale;
  state.transform.x = point.x - (point.x - state.transform.x) * ratio;
  state.transform.y = point.y - (point.y - state.transform.y) * ratio;
  state.transform.scale = next;
  applyTransform();
}

function applyTransform() {
  elements.viewport.setAttribute("transform", `translate(${state.transform.x} ${state.transform.y}) scale(${state.transform.scale})`);
}

function fitGraph() {
  const positions = visibleNodes().map((node) => state.positions.get(node.id)).filter(Boolean);
  if (!positions.length) return;
  const minX = Math.min(...positions.map((position) => position.x));
  const minY = Math.min(...positions.map((position) => position.y));
  const maxX = Math.max(...positions.map((position) => position.x + NODE_WIDTH));
  const maxY = Math.max(...positions.map((position) => position.y + NODE_HEIGHT));
  const bounds = elements.canvas.getBoundingClientRect();
  const scale = Math.min(1.25, Math.max(0.18, Math.min(
    (bounds.width - 90) / Math.max(1, maxX - minX),
    (bounds.height - 90) / Math.max(1, maxY - minY),
  )));
  state.transform = {
    x: (bounds.width - (maxX - minX) * scale) / 2 - minX * scale,
    y: (bounds.height - (maxY - minY) * scale) / 2 - minY * scale,
    scale,
  };
  applyTransform();
}

function centerNode(id) {
  const position = state.positions.get(id);
  if (!position) return;
  const bounds = elements.canvas.getBoundingClientRect();
  state.transform.x = bounds.width / 2 - (position.x + NODE_WIDTH / 2) * state.transform.scale;
  state.transform.y = bounds.height / 2 - (position.y + NODE_HEIGHT / 2) * state.transform.scale;
  applyTransform();
}

function projectWithPresentation(project = state.project) {
  const allNodes = new Set(state.graph.nodes.map((node) => node.id));
  const positionNodes = new Set(state.graph.nodes
    .filter((node) => ![
      "execution_state", "proof_plan", "proof_step", "proof_state", "continuation_merge",
    ].includes(node.payload.kind))
    .map((node) => node.id));
  const positions = Object.fromEntries([...state.positions.entries()]
    .filter(([id]) => positionNodes.has(id))
    .sort(([left], [right]) => left.localeCompare(right)));
  const nodeRegionIds = Object.fromEntries(
    Object.entries(project.presentation?.node_region_ids ?? {})
      .filter(([id]) => allNodes.has(id))
      .sort(([left], [right]) => left.localeCompare(right)),
  );
  const regions = (project.presentation?.regions ?? []).map((region) => ({
    ...region,
    snapshot_node_ids: (region.snapshot_node_ids ?? []).filter((id) => allNodes.has(id)).sort(),
  }));
  return {
    ...project,
    presentation: {
      ...(project.presentation ?? {}),
      positions,
      regions,
      node_region_ids: nodeRegionIds,
    },
  };
}

async function persistProject(project, expectedRevision) {
  return projectApi(`/api/projects/${encodeURIComponent(project.id)}`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      schema: PROJECT_SAVE_SCHEMA,
      expected_revision_sha256: expectedRevision,
      project,
    }),
  });
}

async function saveProject() {
  if (!state.project || state.readOnly) return;
  try {
    const record = await persistProject(projectWithPresentation(), state.revision);
    state.project = record.project;
    state.revision = record.revision_sha256;
    state.readOnly = record.read_only;
    state.dirty = false;
    updateProjectControls();
    await refreshProjects(false, state.project.id);
    setStatus("Project saved", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

async function saveProjectAs() {
  if (!state.project) return;
  const suggested = state.readOnly ? state.project.id.replace(/^demo-/, "") : `${state.project.id}-copy`;
  const id = prompt("New project ID", suggested);
  if (id == null) return;
  const label = prompt("Project name", state.project.label);
  if (label == null) return;
  try {
    const copy = projectWithPresentation({ ...state.project, id: id.trim(), label: label.trim() });
    const record = await persistProject(copy, null);
    state.project = record.project;
    state.revision = record.revision_sha256;
    state.readOnly = false;
    state.dirty = false;
    updateProjectControls();
    await refreshProjects(false, state.project.id);
    setStatus("Project copy saved", "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

function exportProject() {
  if (!state.project) return;
  const project = projectWithPresentation();
  downloadJson(project, `${slug(project.label)}.planner.json`);
}

function markDirty() {
  if (!state.project) return;
  state.dirty = true;
  updateProjectControls();
}

function updateProjectControls() {
  const loaded = Boolean(state.project);
  elements["export-workspace"].disabled = !state.workspace;
  elements["save-project"].disabled = !loaded || state.readOnly || !state.dirty;
  elements["save-as-project"].disabled = !loaded;
  elements["export-project"].disabled = !loaded;
  elements["project-name"].textContent = loaded
    ? `${state.project.label}${state.readOnly ? " (read-only demo)" : ""}`
    : state.workspace?.manifest?.label ?? "No workspace open";
  elements["project-name"].className = `project-name${state.dirty ? " dirty" : ""}`;
}

function connector(source, target) {
  const sx = source.x + NODE_WIDTH;
  const sy = source.y + NODE_HEIGHT / 2;
  const tx = target.x;
  const ty = target.y + NODE_HEIGHT / 2;
  const bend = Math.max(45, Math.abs(tx - sx) * 0.45);
  return `M ${sx} ${sy} C ${sx + bend} ${sy}, ${tx - bend} ${ty}, ${tx} ${ty}`;
}

function svg(name, attributes) {
  const element = document.createElementNS("http://www.w3.org/2000/svg", name);
  for (const [key, value] of Object.entries(attributes)) element.setAttribute(key, value);
  return element;
}

function elide(value, length) {
  return value.length <= length ? value : `${value.slice(0, length - 1)}...`;
}

function slug(value) {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "route";
}

function workspaceAssetRoot(kind) {
  return {
    scenario: "scenarios",
    route_graph: "route-graphs",
    reusable_subgraph: "subgraphs",
    custom_node_definition: "custom-nodes",
    state_seed: "state-seeds",
    query_goal: "queries",
    route_book: "route-books",
    layout: "layouts",
  }[kind] ?? "assets";
}

function downloadJson(document, filename) {
  const blob = new Blob([`${JSON.stringify(document, null, 2)}\n`], {
    type: "application/json",
  });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 0);
}

function setStatus(message, kind = "") {
  elements.status.textContent = message;
  elements.status.className = `status ${kind}`;
}
