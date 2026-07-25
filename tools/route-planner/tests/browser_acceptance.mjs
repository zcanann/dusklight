import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";
import { createServer } from "node:net";

const plannerBinary = process.env.ROUTE_PLANNER_BINARY;
const browserBinary = process.env.ROUTE_PLANNER_BROWSER;
if (!plannerBinary || !browserBinary) throw new Error("browser test binaries were not supplied");

const temporaryRoot = await mkdtemp(join(tmpdir(), "dusklight-route-browser-"));
const projectsRoot = join(temporaryRoot, "projects");
const browserRoot = join(temporaryRoot, "browser");
let planner;
let browser;
const browserProcessGroup = process.platform !== "win32";

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

  browser = spawn(browserBinary, [
    "--headless=new",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--no-first-run",
    "--no-default-browser-check",
    "--remote-debugging-port=0",
    `--user-data-dir=${browserRoot}`,
    "about:blank",
  ], { stdio: ["ignore", "pipe", "pipe"], detached: browserProcessGroup });
  const devtools = await until("browser DevTools port", async () => {
    const text = await readFile(join(browserRoot, "DevToolsActivePort"), "utf8");
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
  await evaluate(`window.dispatchEvent(new KeyboardEvent("keydown", {
    key: "P",
    ctrlKey: true,
    shiftKey: true,
    bubbles: true,
  }))`);
  await browserUntil(
    "keyboard command palette",
    `document.getElementById("command-palette").open
      && document.querySelectorAll("#command-results .command-result").length >= 30
      && document.getElementById("command-results").textContent.includes("New workspace")
      && document.getElementById("command-results").textContent.includes("Find producer chain")`,
  );
  await evaluate(`(() => {
    const search = document.getElementById("command-search");
    search.value = "new workspace";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    search.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    }));
    if (!document.getElementById("new-workspace-dialog").open) {
      throw new Error("command palette did not run New workspace");
    }
    document.getElementById("new-workspace-label").value = "Browser workspace";
    document.getElementById("new-workspace-id").value = "browser-workspace";
    document.getElementById("new-workspace-form").requestSubmit();
    return true;
  })()`);
  await browserUntil(
    "workspace creation",
    `document.getElementById("status").textContent === "Workspace created"
      && document.getElementById("workspace-list").value === "browser-workspace"`,
  );
  await evaluate(`(() => {
    const group = [...document.querySelectorAll(".content-group")].find((candidate) =>
      candidate.querySelector(":scope > summary > span")?.textContent.startsWith("Custom nodes"));
    const create = group?.querySelector(":scope > summary .folder-add");
    if (!create) throw new Error("Custom nodes root has no folder creation action");
    create.click();
    if (!document.getElementById("folder-dialog").open) {
      throw new Error("folder creation dialog did not open");
    }
    document.getElementById("folder-label").value = "Research";
    document.getElementById("folder-id").value = "folder.browser-research";
    document.getElementById("folder-directory").value = "research";
    document.getElementById("folder-form").requestSubmit();
    return true;
  })()`);
  await browserUntil(
    "folder creation",
    `document.getElementById("status").textContent === "Folder created with a stable identity"
      && [...document.querySelectorAll(".content-folder > summary > span")]
        .some((label) => label.textContent.startsWith("Research"))`,
  );
  await evaluate(`(() => {
    const branch = [...document.querySelectorAll(".content-folder")].find((candidate) =>
      candidate.querySelector(":scope > summary > span")?.textContent.startsWith("Research"));
    const rename = [...branch.querySelectorAll(":scope > summary .folder-actions > button")]
      .find((button) => button.textContent === "Rename");
    if (!rename) throw new Error("folder has no Rename action");
    rename.click();
    document.getElementById("folder-label").value = "Research notes";
    document.getElementById("folder-directory").value = "research-notes";
    document.getElementById("folder-form").requestSubmit();
    return true;
  })()`);
  await browserUntil(
    "folder rename",
    `(async () => {
      if (document.getElementById("status").textContent
        !== "Folder renamed; paths and references preserved") return false;
      const workspace = await fetch("/api/workspaces/browser-workspace").then((response) => response.json());
      const folder = workspace.folders.find((candidate) => candidate.id === "folder.browser-research");
      return folder?.label === "Research notes"
        && String(folder.relative_path).replaceAll("\\\\", "/") === "custom-nodes/research-notes";
    })()`,
  );
  await evaluate(`(() => {
    const branch = [...document.querySelectorAll(".content-folder")].find((candidate) =>
      candidate.querySelector(":scope > summary > span")?.textContent.startsWith("Research notes"));
    const duplicate = [...branch.querySelectorAll(":scope > summary .folder-actions > button")]
      .find((button) => button.textContent === "Duplicate");
    if (!duplicate) throw new Error("folder has no Duplicate action");
    duplicate.click();
    document.getElementById("folder-label").value = "Research copy";
    document.getElementById("folder-id").value = "folder.browser-research-copy";
    document.getElementById("folder-directory").value = "research-copy";
    document.getElementById("folder-form").requestSubmit();
    return true;
  })()`);
  await browserUntil(
    "folder duplication",
    `document.getElementById("status").textContent
      === "Folder duplicated; cloned asset references were remapped"
      && [...document.querySelectorAll(".content-folder > summary > span")]
        .some((label) => label.textContent.startsWith("Research copy"))`,
  );
  await evaluate(`(() => {
    window.confirm = () => true;
    const branch = [...document.querySelectorAll(".content-folder")].find((candidate) =>
      candidate.querySelector(":scope > summary > span")?.textContent.startsWith("Research copy"));
    const remove = [...branch.querySelectorAll(":scope > summary .folder-actions > button")]
      .find((button) => button.textContent === "Delete to Trash");
    if (!remove) throw new Error("folder has no delete-to-Trash action");
    remove.click();
    return true;
  })()`);
  await browserUntil(
    "grouped folder Trash",
    `document.getElementById("status").textContent
      === "Folder subtree moved to Trash as one recoverable group"
      && [...document.querySelectorAll(".content-asset-row strong")]
        .some((label) => label.textContent === "Research copy")`,
  );
  await evaluate(`(() => {
    const row = [...document.querySelectorAll(".content-asset-row")].find((candidate) =>
      candidate.querySelector("strong")?.textContent === "Research copy");
    const restore = [...row.querySelectorAll(".asset-actions > button")]
      .find((button) => button.textContent === "Restore group");
    if (!restore) throw new Error("grouped folder Trash has no restore action");
    restore.click();
    return true;
  })()`);
  await browserUntil(
    "grouped folder restore",
    `document.getElementById("status").textContent === "Folder subtree restored"
      && [...document.querySelectorAll(".content-folder > summary > span")]
        .some((label) => label.textContent.startsWith("Research copy"))`,
  );
  await evaluate(`(() => {
    document.getElementById("new-asset").click();
    document.getElementById("new-asset-label").value = "Browser mechanic";
    document.getElementById("new-asset-id").value = "custom.browser-mechanic";
    document.getElementById("new-asset-form").requestSubmit();
    return true;
  })()`);
  await browserUntil(
    "custom-node editor tab",
    `document.getElementById("status").textContent === "Hypothetical custom node created"
      && document.querySelector(
        '#editor-tabs .editor-tab.active[data-asset-id="custom.browser-mechanic"]'
      ) != null
      && !document.getElementById("workspace-asset-editor").hidden`,
  );
  await evaluate(`(() => {
    const name = document.querySelector("#workspace-asset-editor input");
    name.value = "Browser mechanic revised";
    name.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
  await browserUntil(
    "custom-node unsaved indicator",
    `document.querySelector(
      '#editor-tabs .editor-tab[data-asset-id="custom.browser-mechanic"] .editor-tab-dirty'
    ) != null && !document.getElementById("save-project").disabled`,
  );
  await evaluate(`document.querySelector("#workspace-asset-editor input").dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "s",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    }),
  )`);
  await browserUntil(
    "custom-node tab save",
    `(async () => {
      if (document.getElementById("status").textContent !== "Custom node saved") return false;
      if (document.querySelector(
        '#editor-tabs .editor-tab[data-asset-id="custom.browser-mechanic"] .editor-tab-dirty'
      )) return false;
      const record = await fetch(
        "/api/workspaces/browser-workspace/assets/custom.browser-mechanic",
      ).then((response) => response.json());
      return record.asset.header.label === "Browser mechanic revised";
    })()`,
  );
  await evaluate(`window.dispatchEvent(new KeyboardEvent("keydown", {
    key: "w",
    altKey: true,
    bubbles: true,
  }))`);
  await browserUntil(
    "custom-node tab close",
    `document.querySelectorAll("#editor-tabs .editor-tab").length === 0`,
  );
  await evaluate(`document.getElementById("new-scenario").click()`);
  await browserUntil(
    "new scenario exact context choices",
    `document.getElementById("new-scenario-dialog").open
      && [...document.getElementById("new-scenario-library").options]
        .some((option) => option.value === "demo-forest-keyed-door")`,
  );
  await evaluate(`(() => {
    const library = document.getElementById("new-scenario-library");
    library.value = "demo-forest-keyed-door";
    library.dispatchEvent(new Event("change", { bubbles: true }));
    document.getElementById("new-scenario-label").value = "Browser grounded route";
    document.getElementById("new-scenario-id").value = "browser-grounded-route";
    if (!document.getElementById("new-scenario-context").textContent.includes("en")) {
      throw new Error("new scenario does not expose the selected exact runtime context");
    }
    if (document.getElementById("new-scenario-anchor").value !== "library_state") {
      throw new Error("new scenario does not expose its authenticated state anchor");
    }
    if (!document.getElementById("new-scenario-goal").value) {
      throw new Error("new scenario does not require an authored goal");
    }
    document.getElementById("new-scenario-form").requestSubmit();
    return true;
  })()`);
  await browserUntil(
    "grounded scenario authoring context",
    `document.getElementById("status").textContent
        === "Empty grounded scenario created from selected context, anchor, and goal"
      && document.getElementById("project-name").textContent.includes("Browser grounded route")
      && document.getElementById("canvas").dataset.routeStepCount === "0"
      && document.querySelector('#node-kind-list [data-node-kind="mechanic"]') != null`,
  );
  await evaluate(`(() => {
    const canvas = document.getElementById("canvas");
    canvas.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 420,
      clientY: 260,
    }));
    if (document.getElementById("add-node-menu").hidden) {
      throw new Error("workspace graph did not open the Add Node menu");
    }
    if (!document.querySelector("#add-node-results .add-node-result")) {
      throw new Error("workspace graph has no exact Library mechanics to add");
    }
    return true;
  })()`);
  await evaluate(`(() => {
    const executable = document.querySelector(
      "#add-node-results .add-node-result .compatibility.executable",
    )?.closest(".add-node-result");
    if (!executable) throw new Error("workspace Add Node menu has no executable mechanic");
    executable.click();
    return true;
  })()`);
  await browserUntil(
    "first workspace route step",
    `document.getElementById("status").textContent.includes("inserted as step.route-0000")
      && !document.getElementById("save-project").disabled
      && !document.getElementById("undo").disabled`,
  );
  await evaluate(`(() => {
    document.querySelector("#region-breadcrumbs button")?.click();
    const route = [...document.querySelectorAll("#region-children .enter-region")]
      .find((button) => button.textContent.includes("Browser grounded route"));
    if (!route) throw new Error("authored route has no top-level graph region");
    route.click();
    const authored = [...document.querySelectorAll("#region-children .enter-region")]
      .find((button) => button.textContent === "Authored route");
    if (!authored) throw new Error("authored route graph region is absent");
    authored.click();
    return true;
  })()`);
  const executionPin = await browserUntil(
    "typed execution output pin",
    `(() => {
      const pin = document.querySelector(
        '#nodes .execution-pin[data-pin-type="execution_state"][data-route-step-id="step.route-0000"]',
      );
      if (!pin) {
        throw new Error("pins="
          + [...document.querySelectorAll("#nodes .execution-pin")]
            .map((candidate) => candidate.getAttribute("data-route-step-id")).join(",")
          + "; nodes="
          + [...document.querySelectorAll("#nodes .node")]
            .map((candidate) => candidate.dataset.nodeId).join(","));
      }
      const bounds = pin.getBoundingClientRect();
      if (bounds.right < 0 || bounds.bottom < 0
        || bounds.left > innerWidth || bounds.top > innerHeight) {
        throw new Error("pin is outside viewport: " + JSON.stringify({
          left: bounds.left,
          top: bounds.top,
          right: bounds.right,
          bottom: bounds.bottom,
          width: innerWidth,
          height: innerHeight,
          transform: document.getElementById("viewport").getAttribute("transform"),
          nodes: [...document.querySelectorAll("#nodes .node")].map((candidate) => ({
            id: candidate.dataset.nodeId,
            transform: candidate.getAttribute("transform"),
          })),
        }));
      }
      return {
        x: bounds.left + bounds.width / 2,
        y: bounds.top + bounds.height / 2,
        label: pin.getAttribute("aria-label"),
      };
    })()`,
  );
  if (!executionPin.label.includes("step.route-0000")) {
    throw new Error(`execution pin is not labelled by its exact route boundary: ${executionPin.label}`);
  }
  await command("Input.dispatchMouseEvent", {
    type: "mousePressed",
    x: executionPin.x,
    y: executionPin.y,
    button: "left",
    buttons: 1,
    clickCount: 1,
  });
  await command("Input.dispatchMouseEvent", {
    type: "mouseMoved",
    x: executionPin.x + 90,
    y: executionPin.y + 25,
    button: "left",
    buttons: 1,
  });
  await browserUntil(
    "execution pin connection preview",
    `(() => {
      if (document.querySelector("#edges .pin-connection-preview")) return true;
      const target = document.elementFromPoint(${JSON.stringify(executionPin.x)}, ${JSON.stringify(executionPin.y)});
      throw new Error("pin pointer gesture did not begin; target="
        + (target?.className?.baseVal || target?.className || target?.tagName || "none")
        + "; selected=" + (document.querySelector("#nodes .node.selected")?.dataset.nodeId || "none"));
    })()`,
  );
  await command("Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x: executionPin.x + 90,
    y: executionPin.y + 25,
    button: "left",
    clickCount: 1,
  });
  await browserUntil(
    "exact-state compatible pin catalogue",
    `(() => {
      const menu = document.getElementById("add-node-menu");
      const results = [...document.querySelectorAll("#add-node-results .add-node-result")];
      const classifications = results.map((result) =>
        result.querySelector(".compatibility")?.classList.contains("executable")
          ? "executable"
          : result.querySelector(".compatibility")?.classList.contains("feasibility_unknown")
            ? "feasibility_unknown"
            : "other");
      const ready = !menu.hidden
        && menu.querySelector("header strong").textContent.includes("after step.route-0000")
        && results.length > 0
        && classifications.every((classification) =>
          ["executable", "feasibility_unknown"].includes(classification))
        && document.getElementById("status").textContent.includes(
          "compatible mechanic(s) after step.route-0000"
        );
      if (ready) return true;
      throw new Error(JSON.stringify({
        hidden: menu.hidden,
        heading: menu.querySelector("header strong").textContent,
        results: results.length,
        classifications,
        status: document.getElementById("status").textContent,
        text: document.getElementById("add-node-results").textContent,
      }));
    })()`,
  );
  await evaluate(`(() => {
    const executable = document.querySelector(
      "#add-node-results .add-node-result .compatibility.executable",
    )?.closest(".add-node-result");
    if (!executable) throw new Error("pin-filtered catalogue has no executable mechanic");
    executable.click();
    return true;
  })()`);
  await browserUntil(
    "execution pin insertion",
    `document.getElementById("status").textContent.includes("after step.route-0000")
      && document.getElementById("canvas").dataset.routeStepCount === "2"`,
  );
  await evaluate(`document.getElementById("undo").click()`);
  await browserUntil(
    "undo execution pin insertion",
    `document.getElementById("status").textContent.includes("Undid: Insert")
      && document.getElementById("canvas").dataset.routeStepCount === "1"`,
  );
  await evaluate(`(() => {
    const step = document.querySelector("#nodes .node.reference_step");
    if (!step) throw new Error("the authored route step is not visible");
    step.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    const canvas = document.getElementById("canvas");
    const bounds = canvas.getBoundingClientRect();
    canvas.dispatchEvent(new WheelEvent("wheel", {
      bubbles: true,
      cancelable: true,
      deltaY: -120,
      clientX: bounds.left + bounds.width / 2,
      clientY: bounds.top + bounds.height / 2,
    }));
    window.__workspaceTabState = {
      transform: document.getElementById("viewport").getAttribute("transform"),
      selectedId: document.querySelector("#nodes .node.selected")?.dataset.nodeId,
    };
    const routeBooks = [...document.querySelectorAll(".content-group")].find((group) =>
      group.querySelector(":scope > summary")?.textContent.trim().startsWith("Route books"));
    const item = routeBooks?.querySelector(".content-item");
    if (!item) throw new Error("the grounded scenario has no Route Book asset to inspect");
    item.click();
    return true;
  })()`);
  await browserUntil(
    "multiple workspace asset tabs",
    `(() => {
      const tabs = [...document.querySelectorAll("#editor-tabs .editor-tab")];
      const active = tabs.find((tab) => tab.classList.contains("active"));
      const graph = tabs.find((tab) => tab.dataset.assetId.startsWith("route-graph."));
      return tabs.length === 2
        && active?.dataset.assetId.startsWith("route-book.")
        && graph?.querySelector(".editor-tab-dirty")
        && document.getElementById("editor-breadcrumbs").textContent.includes("Route books");
    })()`,
  );
  await evaluate(`window.dispatchEvent(new KeyboardEvent("keydown", {
    key: "]",
    altKey: true,
    bubbles: true,
  }))`);
  await browserUntil(
    "graph tab restores working state",
    `(() => {
      const selected = document.querySelector("#nodes .node.selected");
      const active = document.querySelector("#editor-tabs .editor-tab.active");
      return active?.dataset.assetId.startsWith("route-graph.")
        && active.querySelector(".editor-tab-dirty")
        && document.getElementById("canvas").dataset.routeStepCount === "1"
        && selected?.dataset.nodeId === window.__workspaceTabState.selectedId
        && document.getElementById("viewport").getAttribute("transform")
          === window.__workspaceTabState.transform
        && document.getElementById("editor-breadcrumbs").textContent.includes("Route graphs");
    })()`,
  );
  await evaluate(`window.dispatchEvent(new KeyboardEvent("keydown", {
    key: "z",
    ctrlKey: true,
    bubbles: true,
  }))`);
  await browserUntil(
    "semantic route undo",
    `document.getElementById("status").textContent.includes("Undid: Insert")
      && document.getElementById("undo").disabled
      && !document.getElementById("redo").disabled
      && document.getElementById("save-project").disabled
      && document.querySelector(
        "#editor-tabs .editor-tab.active .editor-tab-dirty"
      ) == null
      && document.getElementById("canvas").dataset.routeStepCount === "0"`,
  );
  await evaluate(`document.getElementById("redo").click()`);
  await browserUntil(
    "semantic route redo completion",
    `document.getElementById("status").textContent.includes("Redid:")
      || document.getElementById("status").textContent.includes("Redo failed:")`,
  );
  const redoState = await evaluate(`({
    status: document.getElementById("status").textContent,
    undoDisabled: document.getElementById("undo").disabled,
    redoDisabled: document.getElementById("redo").disabled,
    saveDisabled: document.getElementById("save-project").disabled,
    tabDirty: document.querySelector(
      "#editor-tabs .editor-tab.active .editor-tab-dirty"
    ) != null,
    routeSteps: Number(document.getElementById("canvas").dataset.routeStepCount),
  })`);
  if (!redoState.status.includes("Redid: Insert")
    || redoState.undoDisabled
    || redoState.saveDisabled
    || !redoState.tabDirty
    || redoState.routeSteps !== 1) {
    throw new Error(`semantic route redo did not restore the authored step: ${JSON.stringify(redoState)}`);
  }
  await evaluate(`(() => {
    const step = document.querySelector("#nodes .node.reference_step");
    if (!step) throw new Error("the authored route step is not visible on its graph");
    step.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 520,
      clientY: 320,
    }));
    const executable = document.querySelector(
      "#add-node-results .add-node-result .compatibility.executable",
    )?.closest(".add-node-result");
    if (!executable) throw new Error("route-step context menu has no executable mechanic");
    executable.click();
    return true;
  })()`);
  await browserUntil(
    "route-step context insertion",
    `document.getElementById("status").textContent.includes("after step.route-0000")
      && document.getElementById("canvas").dataset.routeStepCount === "2"
      && !document.getElementById("undo").disabled`,
  );
  await evaluate(`document.getElementById("undo").click()`);
  await browserUntil(
    "undo context insertion",
    `document.getElementById("status").textContent.includes("Undid: Insert")
      && document.getElementById("canvas").dataset.routeStepCount === "1"
      && !document.getElementById("save-project").disabled`,
  );
  await evaluate(`window.dispatchEvent(new KeyboardEvent("keydown", {
    key: "s",
    ctrlKey: true,
    bubbles: true,
  }))`);
  await browserUntil(
    "atomic workspace route save",
    `document.getElementById("status").textContent
      .includes("Route Book, graph projection, and layout saved atomically")`,
  );
  await browserUntil(
    "persisted workspace route semantics",
    `(async () => {
      const workspace = await fetch("/api/workspaces/browser-workspace").then((response) => response.json());
      const routeBook = workspace.assets.find((asset) => asset.kind === "route_book");
      if (!routeBook) throw new Error("workspace Route Book listing is absent");
      const record = await fetch(
        "/api/workspaces/browser-workspace/assets/" + encodeURIComponent(routeBook.id),
      ).then((response) => response.json());
      return record.asset.payload.route_book.steps.length === 1
        && record.asset.payload.route_book.methods[0].step_ids[0] === "step.route-0000";
    })()`,
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
    "friendly default terminology",
    `(() => {
      const kinds = [...document.querySelectorAll("#nodes .kind")]
        .map((node) => node.textContent);
      return kinds.includes("Mechanic")
        && kinds.every((label) => !label.includes("_"))
        && document.querySelector("#model-context > summary").textContent.includes("Advanced")
        && document.querySelector(".diagnostics-drawer > summary").textContent.includes("Advanced");
    })()`,
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
  await browserUntil(
    "code-authored node kinds",
    `(() => {
      const kinds = [...document.querySelectorAll("#node-kind-list [data-node-kind]")]
        .map((button) => button.dataset.nodeKind);
      return ["mechanic", "goal", "condition"].every((kind) => kinds.includes(kind))
        && document.querySelectorAll('#content-browser-list [data-node-kind]').length === 0;
    })()`,
  );
  await evaluate(`document.querySelector('#node-kind-list [data-node-kind="goal"]').click()`);
  await browserUntil(
    "goal content separated from its built-in kind",
    `document.querySelector('#palette-list .palette-item[data-node-kind="goal"]') != null
      && document.getElementById("palette-list").textContent.includes("Model content")`,
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
    const canvas = document.getElementById("canvas");
    canvas.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 420,
      clientY: 260,
    }));
    const search = document.getElementById("add-node-search");
    search.value = transition;
    search.dispatchEvent(new Event("input", { bubbles: true }));
    const item = document.querySelector(
      '#add-node-results .add-node-result[data-transition-id="' + transition + '"]',
    );
    if (!item) throw new Error("rejected transition is absent from the right-click Add Node menu");
    item.click();
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
    `document.getElementById("status").textContent.includes("Evidence policy changed to Research")`,
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
  await Promise.all([stopChild(browser, browserProcessGroup), stopChild(planner)]);
  await rm(temporaryRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
