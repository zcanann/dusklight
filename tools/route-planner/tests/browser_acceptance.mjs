import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";
import { createServer } from "node:net";

const plannerBinary = process.env.ROUTE_PLANNER_BINARY;
const braveBinary = process.env.ROUTE_PLANNER_BRAVE;
if (!plannerBinary || !braveBinary) throw new Error("browser test binaries were not supplied");

const temporaryRoot = await mkdtemp(join(tmpdir(), "dusklight-route-browser-"));
const projectsRoot = join(temporaryRoot, "projects");
const braveRoot = join(temporaryRoot, "brave");
let planner;
let brave;

const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const stopChild = async (child, processGroup = false) => {
  if (!child) return;
  const signal = (name) => {
    try {
      if (processGroup) process.kill(-child.pid, name);
      else child.kill(name);
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  };
  signal("SIGTERM");
  if (child.exitCode != null || child.signalCode != null) return;
  let exited = new Promise((resolve) => child.once("exit", resolve));
  await Promise.race([exited, delay(2_000)]);
  if (child.exitCode == null && child.signalCode == null) {
    exited = new Promise((resolve) => child.once("exit", resolve));
    signal("SIGKILL");
    await Promise.race([exited, delay(2_000)]);
  }
};
const freePort = () => new Promise((resolve, reject) => {
  const server = createServer();
  server.once("error", reject);
  server.listen(0, "127.0.0.1", () => {
    const { port } = server.address();
    server.close((error) => error ? reject(error) : resolve(port));
  });
});
const until = async (label, operation, timeoutMilliseconds = 45_000) => {
  const deadline = Date.now() + timeoutMilliseconds;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await operation();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await delay(50);
  }
  throw new Error(`timed out waiting for ${label}${lastError ? `: ${lastError.message}` : ""}`);
};

try {
  const plannerPort = await freePort();
  planner = spawn(plannerBinary, [
    "serve-web",
    "--listen", `127.0.0.1:${plannerPort}`,
    "--projects", projectsRoot,
  ], { stdio: ["ignore", "pipe", "pipe"] });
  const plannerUrl = `http://127.0.0.1:${plannerPort}`;
  await until("planner health", async () => (await fetch(`${plannerUrl}/api/health`)).ok);

  brave = spawn(braveBinary, [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    "--remote-debugging-port=0",
    `--user-data-dir=${braveRoot}`,
    "about:blank",
  ], { stdio: ["ignore", "pipe", "pipe"], detached: true });
  const devtools = await until("Brave DevTools port", async () => {
    const text = await readFile(join(braveRoot, "DevToolsActivePort"), "utf8");
    const [port] = text.trim().split(/\s+/);
    return Number(port) || null;
  });
  const targetResponse = await fetch(
    `http://127.0.0.1:${devtools}/json/new?${encodeURIComponent(plannerUrl)}`,
    { method: "PUT" },
  );
  if (!targetResponse.ok) throw new Error(`DevTools target creation returned ${targetResponse.status}`);
  const target = await targetResponse.json();
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  let commandId = 0;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (!message.id || !pending.has(message.id)) return;
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(message.error.message));
    else resolve(message.result);
  });
  const command = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++commandId;
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (expression) => {
    const result = await command("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.exception?.description ?? "browser evaluation failed");
    }
    return result.result.value;
  };
  const browserUntil = (label, expression, timeoutMilliseconds) =>
    until(label, () => evaluate(expression), timeoutMilliseconds);

  await command("Runtime.enable");
  await command("Page.enable");
  await browserUntil(
    "planner application load",
    `document.readyState === "complete" && document.querySelectorAll("#project-list option").length >= 7`,
  );
  await evaluate(`(() => {
    const list = document.getElementById("project-list");
    list.value = "demo-forest-keyed-door";
    list.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  })()`);
  await browserUntil(
    "keyed-door demonstration",
    `document.getElementById("project-name").textContent.includes("Forest Temple small-key door")`,
  );
  await browserUntil(
    "exact model context",
    `(() => {
      const panel = document.getElementById("model-context-body");
      const text = panel.textContent;
      return text.includes("Exact runtime")
        && text.includes("Language")
        && text.includes("en")
        && text.includes("Catalog provenance")
        && text.includes("Active packs & overlays")
        && text.includes("Coverage")
        && text.includes("Confidence")
        && text.includes("Route-cost model")
        && panel.querySelector('select[aria-label="Evidence policy"]').disabled;
    })()`,
  );
  await evaluate(`(() => {
    const answers = ["browser-keyed-door", "Browser keyed-door acceptance"];
    window.prompt = () => answers.shift() ?? null;
    document.getElementById("save-as-project").click();
    return true;
  })()`);
  await browserUntil(
    "editable demonstration copy",
    `(() => {
      const status = document.getElementById("status");
      if (status.textContent !== "Project copy saved") throw new Error(status.textContent);
      return true;
    })()`,
  );

  await evaluate(`(() => {
    const transition = ${JSON.stringify("transition.gz2e01-door1-09-close-end")};
    const search = document.getElementById("search");
    search.value = transition;
    search.dispatchEvent(new Event("input", { bubbles: true }));
    const item = [...document.querySelectorAll("#palette-list .palette-item")]
      .find((button) => button.querySelector("small")?.textContent.endsWith("· " + transition));
    if (!item) throw new Error("rejected transition is absent from the browser palette");
    item.click();
    document.getElementById("insert-transition").click();
    return true;
  })()`);
  await browserUntil(
    "typed rejected join",
    `(() => {
      const status = document.getElementById("status");
      if (!status.textContent.includes("was not inserted")) throw new Error(status.textContent);
      return true;
    })()`,
  );
  await evaluate(`document.getElementById("suggest-transition-chain").click()`);
  await browserUntil(
    "producer-chain suggestion",
    `(() => {
      const status = document.getElementById("status");
      const button = document.getElementById("suggest-transition-chain");
      if (!status.textContent.includes("Suggested exact chain")
        || button.textContent !== "Insert 8-step chain") {
        throw new Error(status.textContent + "; button: " + button.textContent);
      }
      return true;
    })()`,
  );
  await evaluate(`document.getElementById("suggest-transition-chain").click()`);
  await browserUntil(
    "atomic producer-chain insertion",
    `document.getElementById("status").textContent.includes("8-step producer chain inserted")`,
  );
  await evaluate(`(() => {
    document.querySelector("#region-breadcrumbs button")?.click();
    const plans = [...document.querySelectorAll("#region-children .enter-region")]
      .find((button) => button.textContent === "Browser keyed-door acceptance");
    if (!plans) throw new Error("plan region is absent from browser navigation");
    plans.click();
    const authored = [...document.querySelectorAll("#region-children .enter-region")]
      .find((button) => button.textContent === "Authored route");
    if (!authored) throw new Error("authored route is absent from plan navigation");
    authored.click();
    return true;
  })()`);
  await browserUntil(
    "authored route region contents",
    `document.querySelectorAll("#nodes .node.reference_step").length === 8`,
  );
  await browserUntil(
    "projected execution states",
    `document.querySelectorAll("#nodes .node.execution_state").length === 9`,
  );
  await evaluate(`(() => {
    const step = document.querySelector('[data-node-id="plan-step/step.route-0007"]');
    const terminal = document.querySelector('[data-node-id="execution-state/after/step.route-0007"]');
    if (!step || !terminal) throw new Error("terminal state/step grouping pair is absent");
    step.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
    terminal.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
    window.prompt = () => "Closing subgraph";
    document.getElementById("group-selection").click();
    return true;
  })()`);
  await browserUntil(
    "presentation-only nested grouping",
    `document.getElementById("status").textContent.includes("presentation-only graph region")
      && document.getElementById("region-breadcrumbs").textContent.includes("Closing subgraph")
      && document.querySelectorAll("#nodes .node.reference_step").length === 1
      && document.querySelectorAll("#nodes .node.execution_state").length === 1`,
  );
  await evaluate(`(() => {
    const terminal = document.querySelector('[data-node-id="execution-state/after/step.route-0007"]');
    if (!terminal) throw new Error("terminal execution state is absent from the grouped region");
    terminal.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
    window.prompt = () => "Terminal state";
    document.getElementById("group-selection").click();
    return true;
  })()`);
  await browserUntil(
    "nested region breadcrumbs",
    `document.getElementById("region-breadcrumbs").textContent.includes("Closing subgraph")
      && document.getElementById("region-breadcrumbs").textContent.includes("Terminal state")
      && document.querySelectorAll("#nodes .node.reference_step").length === 0
      && document.querySelectorAll("#nodes .node.execution_state").length === 1`,
  );
  await evaluate(`(() => {
    const closing = [...document.querySelectorAll("#region-breadcrumbs button")]
      .find((button) => button.textContent === "Closing subgraph");
    if (!closing) throw new Error("closing-region breadcrumb is absent");
    closing.click();
    const terminalRow = [...document.querySelectorAll("#region-children .region-row")]
      .find((row) => row.querySelector(".enter-region")?.textContent === "Terminal state");
    if (!terminalRow) throw new Error("terminal-state child region is absent");
    terminalRow.querySelector(".inspect-region").click();
    return true;
  })()`);
  await browserUntil(
    "nested region boundary inspection",
    `document.getElementById("detail-json").textContent.includes('"boundary_edges"')
      && document.getElementById("detail-json").textContent.includes("execution-state/after/step.route-0007")`,
  );
  const deriveRegion = async (buttonId, promptValue, expectedStatus) => {
    await evaluate(`(() => {
      const closing = [...document.querySelectorAll("#region-breadcrumbs button")]
        .find((button) => button.textContent === "Closing subgraph");
      if (closing) closing.click();
      const terminalRow = [...document.querySelectorAll("#region-children .region-row")]
        .find((row) => row.querySelector(".enter-region")?.textContent === "Terminal state");
      if (!terminalRow) throw new Error("terminal-state source region is absent");
      terminalRow.querySelector(".inspect-region").click();
      window.prompt = () => ${JSON.stringify(promptValue)};
      document.getElementById(${JSON.stringify(buttonId)}).click();
      return true;
    })()`);
    await browserUntil(
      `region ${buttonId}`,
      `document.getElementById("status").textContent.includes(${JSON.stringify(expectedStatus)})`,
    );
  };
  await deriveRegion("reference-region", "Terminal reference", "created as reference");
  await deriveRegion("copy-region", "Terminal copy", "created as copy");
  await deriveRegion("fork-region", "Terminal fork", "created as fork");
  await deriveRegion("version-region", "Terminal v2", "created as version");
  await deriveRegion(
    "replace-region",
    "region.presentation-terminal-copy",
    "Terminal copy replaced from Terminal state at version 2",
  );
  await evaluate(`(() => {
    const closing = [...document.querySelectorAll("#region-breadcrumbs button")]
      .find((button) => button.textContent === "Closing subgraph");
    if (closing) closing.click();
    const terminalRow = [...document.querySelectorAll("#region-children .region-row")]
      .find((row) => row.querySelector(".enter-region")?.textContent === "Terminal state");
    terminalRow.querySelector(".inspect-region").click();
    document.getElementById("region-usage").click();
    return true;
  })()`);
  await browserUntil(
    "region usage inspection",
    `document.getElementById("status").textContent.includes("has 4 derived usages")
      && document.getElementById("detail-json").textContent.includes('"derivation_kind": "replacement"')`,
  );
  await evaluate(`(() => {
    const terminal = [...document.querySelectorAll("#region-children .enter-region")]
      .find((button) => button.textContent === "Terminal state");
    if (!terminal) throw new Error("terminal-state enter control is absent");
    terminal.click();
    return true;
  })()`);
  await evaluate(`(() => {
    const terminal = document.querySelector('[data-node-id="execution-state/after/step.route-0007"]');
    if (!terminal) throw new Error("terminal execution state is absent");
    terminal.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    return true;
  })()`);
  await browserUntil(
    "execution state inspection",
    `document.getElementById("state-inspector").textContent.includes("D_MN05 r2")`,
  );
  await browserUntil(
    "execution state transition listing",
    `(() => {
      const status = document.getElementById("status");
      if (status.classList.contains("bad")) throw new Error(status.textContent);
      const ready = status.textContent.includes("transition(s) executable from After step.route-0007")
        && !document.getElementById("palette-list").textContent.includes("not assessed");
      if (!ready) throw new Error("current status: " + status.textContent
        + "; palette: " + document.getElementById("palette-list").textContent);
      return true;
    })()`,
    10_000,
  );
  await evaluate(`(() => {
    const closing = [...document.querySelectorAll("#region-breadcrumbs button")]
      .find((button) => button.textContent === "Closing subgraph");
    if (!closing) throw new Error("closing-region breadcrumb is absent");
    closing.click();
    const step = [...document.querySelectorAll("#nodes .node.reference_step")].at(-1);
    if (!step) throw new Error("terminal route step is absent from the projected graph");
    step.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    document.getElementById("remove-step").click();
    return true;
  })()`);
  await browserUntil(
    "downstream state replay after removal",
    `document.getElementById("status").textContent.includes("removed; downstream state replayed")`,
  );
  await evaluate(`(() => {
    const policy = document.querySelector('select[aria-label="Evidence policy"]');
    if (!policy || policy.disabled) throw new Error("editable evidence policy is absent");
    policy.value = "research";
    policy.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  })()`);
  await browserUntil(
    "evidence-policy edit",
    `document.getElementById("status").textContent.includes("Evidence policy changed to research")`,
  );
  await evaluate(`(async () => {
    const record = await fetch("/api/projects/browser-keyed-door").then((response) => response.json());
    const component = record.project.start_state.snapshot.environment.components[0];
    if (!component) throw new Error("keyed-door start state has no component to theorycraft");
    const answers = [component.id, "global", "Browser component rebind", "what-if.browser-component-rebind"];
    window.prompt = () => answers.shift() ?? null;
    window.confirm = () => true;
    const button = [...document.querySelectorAll(".context-actions button")]
      .find((candidate) => candidate.textContent === "Rebind");
    if (!button) throw new Error("theorycraft rebind control is absent");
    button.click();
    return true;
  })()`);
  await browserUntil(
    "theorycraft component rebind",
    `(() => {
      const status = document.getElementById("status");
      if (status.classList.contains("bad")) throw new Error(status.textContent);
      return status.textContent.includes("Enabled what-if.browser-component-rebind")
        && document.getElementById("model-context-body").textContent.includes("what-if.browser-component-rebind");
    })()`,
  );
  await evaluate(`document.getElementById("save-project").click()`);
  await browserUntil("saved browser edit", `document.getElementById("status").textContent === "Project saved"`);
  const beforeReload = await evaluate(`fetch("/api/projects/browser-keyed-door")
    .then((response) => response.json())
    .then((record) => ({
      revision: record.revision_sha256,
      actions: record.project.route_book.steps.map((step) => step.action.transition_id),
      evidenceMode: record.project.evidence_mode,
      overlays: record.project.theorycraft_overlays.map((pack) => pack.manifest.id),
      hasTheorycraftBase: record.project.theorycraft_base_catalog != null,
    }))`);
  await evaluate(`(() => {
    const list = document.getElementById("project-list");
    list.value = "browser-keyed-door";
    list.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  })()`);
  await browserUntil(
    "reloaded browser project",
    `document.getElementById("project-name").textContent.includes("Browser keyed-door acceptance")
      && document.getElementById("save-project").disabled`,
  );
  const afterReload = await evaluate(`fetch("/api/projects/browser-keyed-door")
    .then((response) => response.json())
    .then((record) => ({
      revision: record.revision_sha256,
      actions: record.project.route_book.steps.map((step) => step.action.transition_id),
      evidenceMode: record.project.evidence_mode,
      overlays: record.project.theorycraft_overlays.map((pack) => pack.manifest.id),
      hasTheorycraftBase: record.project.theorycraft_base_catalog != null,
    }))`);
  if (beforeReload.revision !== afterReload.revision
    || JSON.stringify(beforeReload.actions) !== JSON.stringify(afterReload.actions)
    || beforeReload.evidenceMode !== "research"
    || afterReload.evidenceMode !== "research"
    || JSON.stringify(beforeReload.overlays) !== JSON.stringify(["what-if.browser-component-rebind"])
    || JSON.stringify(afterReload.overlays) !== JSON.stringify(beforeReload.overlays)
    || !beforeReload.hasTheorycraftBase
    || !afterReload.hasTheorycraftBase) {
    throw new Error("saved and reloaded browser project identities differ");
  }
  await evaluate(`(() => {
    const button = [...document.querySelectorAll(".context-pack-remove")]
      .find((candidate) => candidate.getAttribute("aria-label")?.includes("what-if.browser-component-rebind"));
    if (!button) throw new Error("saved theorycraft overlay has no remove control");
    button.click();
    return true;
  })()`);
  await browserUntil(
    "reversible theorycraft removal",
    `(() => {
      const status = document.getElementById("status");
      if (status.classList.contains("bad")) throw new Error(status.textContent);
      return status.textContent.includes("Removed 1 theorycraft overlay")
        && !document.getElementById("model-context-body").textContent.includes("what-if.browser-component-rebind");
    })()`,
  );
  await evaluate(`(() => {
    const goal = document.querySelector("#nodes .node.goal");
    if (!goal) throw new Error("planner graph has no selectable goal");
    goal.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    const solve = document.getElementById("solve-goal");
    if (solve.disabled) throw new Error("selected goal did not enable the solve control");
    solve.click();
    return true;
  })()`);
  await browserUntil(
    "nested solver proof navigation",
    `document.getElementById("region-breadcrumbs").textContent.includes("Solver proof")
      && document.getElementById("detail-json").textContent.includes('"solve_report"')`,
    20_000,
  );
  socket.close();
} finally {
  await Promise.all([stopChild(brave, true), stopChild(planner)]);
  await rm(temporaryRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
