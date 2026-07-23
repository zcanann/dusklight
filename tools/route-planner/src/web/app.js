const SERVICE_SCHEMA = "dusklight.route-planner.service/v28";
const PROJECT_SCHEMA = "dusklight.route-planner.web-project/v1";
const NODE_WIDTH = 176;
const NODE_HEIGHT = 52;

const elements = Object.fromEntries([
  "open-project", "save-project", "project-file", "project-name", "status",
  "search", "palette-list", "canvas-shell", "canvas", "viewport", "edges",
  "nodes", "empty-state", "zoom-in", "zoom-out", "fit", "detail-title",
  "detail-subtitle", "detail-json",
].map((id) => [id, document.getElementById(id)]));

const state = {
  project: null,
  graph: null,
  positions: new Map(),
  selected: null,
  transform: { x: 70, y: 60, scale: 1 },
  gesture: null,
};

elements["open-project"].addEventListener("click", () => elements["project-file"].click());
elements["project-file"].addEventListener("change", openProject);
elements["save-project"].addEventListener("click", saveProjectCopy);
elements.search.addEventListener("input", renderPalette);
elements["zoom-in"].addEventListener("click", () => zoomAt(1.2));
elements["zoom-out"].addEventListener("click", () => zoomAt(1 / 1.2));
elements.fit.addEventListener("click", fitGraph);
elements.canvas.addEventListener("wheel", onWheel, { passive: false });
elements.canvas.addEventListener("pointerdown", beginPan);
window.addEventListener("pointermove", moveGesture);
window.addEventListener("pointerup", endGesture);

health();
applyTransform();

async function health() {
  try {
    const response = await fetch("/api/health", { cache: "no-store" });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    setStatus("Planner service ready", "good");
  } catch (error) {
    setStatus(`Planner service unavailable: ${error.message}`, "bad");
  }
}

async function openProject(event) {
  const file = event.target.files?.[0];
  event.target.value = "";
  if (!file) return;
  try {
    const project = JSON.parse(await file.text());
    validateProject(project);
    setStatus("Projecting graph…");
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
    ensurePositions();
    state.selected = null;
    elements["project-name"].textContent = project.label || file.name;
    elements["save-project"].disabled = false;
    elements["empty-state"].hidden = true;
    render();
    requestAnimationFrame(fitGraph);
    setStatus(`${state.graph.nodes.length} nodes · ${state.graph.edges.length} connections`, "good");
  } catch (error) {
    setStatus(error.message, "bad");
  }
}

function validateProject(project) {
  if (!project || project.schema !== PROJECT_SCHEMA) throw new Error(`Expected ${PROJECT_SCHEMA}`);
  if (!project.catalog || typeof project.catalog !== "object") throw new Error("Project has no catalog");
  if (project.route_book != null && typeof project.route_book !== "object") throw new Error("Project route_book is invalid");
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
  renderEdges();
  renderNodes();
  renderPalette();
  renderDetails();
}

function renderEdges() {
  elements.edges.replaceChildren();
  for (const edge of state.graph?.edges ?? []) {
    const source = state.positions.get(edge.source_node_id);
    const target = state.positions.get(edge.target_node_id);
    if (!source || !target) continue;
    const path = svg("path", {
      class: `graph-edge${state.selected?.type === "edge" && state.selected.value.id === edge.id ? " selected" : ""}`,
      d: connector(source, target),
    });
    path.addEventListener("click", (event) => {
      event.stopPropagation();
      state.selected = { type: "edge", value: edge };
      render();
    });
    elements.edges.append(path);
  }
}

function renderNodes() {
  elements.nodes.replaceChildren();
  for (const node of state.graph?.nodes ?? []) {
    const position = state.positions.get(node.id);
    const group = svg("g", {
      class: `node ${node.payload.kind}${state.selected?.type === "node" && state.selected.value.id === node.id ? " selected" : ""}`,
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
      state.selected = { type: "node", value: node };
      render();
    });
    elements.nodes.append(group);
  }
}

function renderPalette() {
  elements["palette-list"].replaceChildren();
  if (!state.graph) return;
  const query = elements.search.value.trim().toLowerCase();
  for (const node of state.graph.nodes.filter((node) => node.payload.kind === "transition" && (!query || `${node.label} ${node.id}`.toLowerCase().includes(query)))) {
    const button = document.createElement("button");
    button.className = "palette-item";
    button.innerHTML = `<span></span><small></small>`;
    button.querySelector("span").textContent = node.label;
    button.querySelector("small").textContent = node.payload.transition_id;
    button.addEventListener("click", () => {
      state.selected = { type: "node", value: node };
      centerNode(node.id);
      render();
    });
    elements["palette-list"].append(button);
  }
}

function renderDetails() {
  const selected = state.selected;
  if (!selected) {
    elements["detail-title"].textContent = "Nothing selected";
    elements["detail-subtitle"].textContent = "Choose a node or connection to inspect its planner-owned identity.";
    elements["detail-json"].textContent = "{}";
    return;
  }
  elements["detail-title"].textContent = selected.type === "node" ? selected.value.label : selected.value.relation;
  elements["detail-subtitle"].textContent = selected.value.id;
  elements["detail-json"].textContent = JSON.stringify(selected.value, null, 2);
}

function beginPan(event) {
  if (event.button !== 0 || event.target.closest?.(".node")) return;
  elements.canvas.setPointerCapture(event.pointerId);
  state.selected = null;
  state.gesture = { kind: "pan", pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, x: state.transform.x, y: state.transform.y };
  renderDetails();
}

function beginNodeDrag(event, node) {
  if (event.button !== 0) return;
  event.stopPropagation();
  elements.canvas.setPointerCapture(event.pointerId);
  const position = state.positions.get(node.id);
  state.selected = { type: "node", value: node };
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
  state.gesture = null;
  if (elements.canvas.hasPointerCapture(event.pointerId)) elements.canvas.releasePointerCapture(event.pointerId);
}

function onWheel(event) {
  event.preventDefault();
  const bounds = elements.canvas.getBoundingClientRect();
  const point = { x: event.clientX - bounds.left, y: event.clientY - bounds.top };
  const factor = Math.exp(-event.deltaY * 0.0012);
  setScale(state.transform.scale * factor, point);
}

function zoomAt(factor) {
  const bounds = elements.canvas.getBoundingClientRect();
  setScale(state.transform.scale * factor, { x: bounds.width / 2, y: bounds.height / 2 });
}

function setScale(next, point) {
  next = Math.min(2.75, Math.max(.18, next));
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
  if (!state.positions.size) return;
  const positions = [...state.positions.values()];
  const minX = Math.min(...positions.map((p) => p.x));
  const minY = Math.min(...positions.map((p) => p.y));
  const maxX = Math.max(...positions.map((p) => p.x + NODE_WIDTH));
  const maxY = Math.max(...positions.map((p) => p.y + NODE_HEIGHT));
  const bounds = elements.canvas.getBoundingClientRect();
  const scale = Math.min(1.25, Math.max(.18, Math.min((bounds.width - 90) / Math.max(1, maxX - minX), (bounds.height - 90) / Math.max(1, maxY - minY))));
  state.transform = { x: (bounds.width - (maxX - minX) * scale) / 2 - minX * scale, y: (bounds.height - (maxY - minY) * scale) / 2 - minY * scale, scale };
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

function saveProjectCopy() {
  if (!state.project) return;
  const positions = Object.fromEntries([...state.positions.entries()].sort(([a], [b]) => a.localeCompare(b)));
  const project = { ...state.project, presentation: { ...(state.project.presentation ?? {}), positions } };
  const blob = new Blob([`${JSON.stringify(project, null, 2)}\n`], { type: "application/json" });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = `${slug(project.label || "route")}.planner.json`;
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 0);
}

function connector(source, target) {
  const sx = source.x + NODE_WIDTH;
  const sy = source.y + NODE_HEIGHT / 2;
  const tx = target.x;
  const ty = target.y + NODE_HEIGHT / 2;
  const bend = Math.max(45, Math.abs(tx - sx) * .45);
  return `M ${sx} ${sy} C ${sx + bend} ${sy}, ${tx - bend} ${ty}, ${tx} ${ty}`;
}

function svg(name, attributes) {
  const element = document.createElementNS("http://www.w3.org/2000/svg", name);
  for (const [key, value] of Object.entries(attributes)) element.setAttribute(key, value);
  return element;
}

function elide(value, length) {
  return value.length <= length ? value : `${value.slice(0, length - 1)}…`;
}

function slug(value) {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "route";
}

function setStatus(message, kind = "") {
  elements.status.textContent = message;
  elements.status.className = `status ${kind}`;
}
