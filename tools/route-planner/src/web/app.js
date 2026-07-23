const SERVICE_SCHEMA = "dusklight.route-planner.service/v41";
const PROJECT_SCHEMA = "dusklight.route-planner.web-project/v2";
const LEGACY_PROJECT_SCHEMA = "dusklight.route-planner.web-project/v1";
const PROJECT_SAVE_SCHEMA = "dusklight.route-planner.web-project-save/v1";
const ROUTE_BOOK_EDIT_BATCH_SCHEMA = "dusklight.route-planner.route-book-edit-batch/v6";
const NODE_WIDTH = 176;
const NODE_HEIGHT = 52;

const elements = Object.fromEntries([
  "project-list", "new-project", "open-project", "save-project", "save-as-project",
  "export-project", "project-file", "project-name", "status", "search", "palette-list",
  "canvas-shell", "canvas", "viewport", "edges", "nodes", "empty-state", "zoom-in",
  "zoom-out", "fit", "detail-title", "detail-subtitle", "detail-json", "state-inspector",
  "contract-inspector",
  "region-nav", "region-breadcrumbs", "region-children",
  "evaluate-transition", "insert-transition", "suggest-transition-chain", "replace-step", "remove-step",
  "group-selection", "copy-region", "fork-region", "reference-region", "version-region",
  "replace-region", "region-usage", "pin-selection", "ban-selection", "prefer-selection", "select-method",
].map((id) => [id, document.getElementById(id)]));

const state = {
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
  groupSelection: new Set(),
};

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
elements.canvas.addEventListener("dragover", allowTransitionDrop);
elements.canvas.addEventListener("drop", dropTransitionAtRouteFrontier);
window.addEventListener("pointermove", moveGesture);
window.addEventListener("pointerup", endGesture);
window.addEventListener("beforeunload", (event) => {
  if (!state.dirty) return;
  event.preventDefault();
  event.returnValue = "";
});

applyTransform();
start();

async function start() {
  try {
    const response = await fetch("/api/health", { cache: "no-store" });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    setStatus("Planner service ready", "good");
    await refreshProjects(true);
  } catch (error) {
    setStatus(`Planner service unavailable: ${error.message}`, "bad");
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
  if (project?.schema === LEGACY_PROJECT_SCHEMA) project.schema = PROJECT_SCHEMA;
  if (!project || project.schema !== PROJECT_SCHEMA) throw new Error(`Expected ${PROJECT_SCHEMA}`);
  if (!project.id || typeof project.id !== "string") throw new Error("Project has no id");
  if (!project.label || typeof project.label !== "string") throw new Error("Project has no label");
  if (!project.catalog || typeof project.catalog !== "object") throw new Error("Project has no catalog");
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
  renderRegionNavigation();
  renderEdges();
  renderNodes();
  renderPalette();
  renderDetails();
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

function renderPalette(selectedFeasibility = state.selectedStateFeasibility) {
  elements["palette-list"].replaceChildren();
  if (!state.graph) return;
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
  if (!state.project?.start_state || state.readOnly) return;
  event.preventDefault();
  event.dataTransfer.dropEffect = "copy";
}

async function dropTransitionAtRouteFrontier(event) {
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
  const routeStep = selected.type === "node" && selected.value.payload.kind === "reference_step";
  elements["evaluate-transition"].disabled = !transition || !state.project?.start_state;
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
    .filter((node) => node.payload.kind !== "execution_state")
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
  const blob = new Blob([`${JSON.stringify(project, null, 2)}\n`], { type: "application/json" });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = `${slug(project.label)}.planner.json`;
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 0);
}

function markDirty() {
  if (!state.project) return;
  state.dirty = true;
  updateProjectControls();
}

function updateProjectControls() {
  const loaded = Boolean(state.project);
  elements["save-project"].disabled = !loaded || state.readOnly || !state.dirty;
  elements["save-as-project"].disabled = !loaded;
  elements["export-project"].disabled = !loaded;
  elements["project-name"].textContent = loaded
    ? `${state.project.label}${state.readOnly ? " (read-only demo)" : ""}`
    : "No project loaded";
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

function setStatus(message, kind = "") {
  elements.status.textContent = message;
  elements.status.className = `status ${kind}`;
}
