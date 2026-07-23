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
const until = async (label, operation, timeoutMilliseconds = 15_000) => {
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
  const browserUntil = (label, expression) => until(label, () => evaluate(expression));

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
  await evaluate(`(() => {
    const answers = ["browser-keyed-door", "Browser keyed-door acceptance"];
    window.prompt = () => answers.shift() ?? null;
    document.getElementById("save-as-project").click();
    return true;
  })()`);
  await browserUntil(
    "editable demonstration copy",
    `document.getElementById("status").textContent === "Project copy saved"`,
  );

  const transitions = [
    "transition.gz2e01-door1-01-offer-event",
    "transition.gz2e01-door1-02-demo-action8",
    "transition.gz2e01-door1-03-finish-keyhole",
    "transition.gz2e01-door1-04-flush-key-delta",
    "transition.gz2e01-door1-05-open-init",
    "transition.gz2e01-door1-06-open-proc",
    "transition.gz2e01-door1-07-cross-room-adjacency",
    "transition.gz2e01-door1-08-close-init",
    "transition.gz2e01-door1-09-close-end",
  ];
  for (let index = 0; index < transitions.length; index += 1) {
    const transition = transitions[index];
    await evaluate(`(() => {
      const search = document.getElementById("search");
      search.value = ${JSON.stringify(transition)};
      search.dispatchEvent(new Event("input", { bubbles: true }));
      const item = [...document.querySelectorAll("#palette-list .palette-item")]
        .find((button) => button.querySelector("small")?.textContent.endsWith(${JSON.stringify(`· ${transition}`)}));
      if (!item) throw new Error("transition is absent from the browser palette");
      item.click();
      document.getElementById("insert-transition").click();
      return true;
    })()`);
    await browserUntil(
      `route insertion ${index}`,
      `document.getElementById("status").textContent.includes("inserted as step.route-${String(index).padStart(4, "0")}")`,
    );
  }
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
    `document.querySelectorAll("#nodes .node.reference_step").length === 9`,
  );
  await browserUntil(
    "projected execution states",
    `document.querySelectorAll("#nodes .node.execution_state").length === 10`,
  );
  await evaluate(`(() => {
    const terminal = document.querySelector('[data-node-id="execution-state/after/step.route-0008"]');
    if (!terminal) throw new Error("terminal execution state is absent");
    terminal.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    return true;
  })()`);
  await browserUntil(
    "execution state inspection",
    `document.getElementById("state-inspector").textContent.includes("D_MN05 r2")`,
  );
  await evaluate(`(() => {
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
  await evaluate(`document.getElementById("save-project").click()`);
  await browserUntil("saved browser edit", `document.getElementById("status").textContent === "Project saved"`);
  const beforeReload = await evaluate(`fetch("/api/projects/browser-keyed-door")
    .then((response) => response.json())
    .then((record) => ({
      revision: record.revision_sha256,
      actions: record.project.route_book.steps.map((step) => step.action.transition_id),
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
    }))`);
  if (beforeReload.revision !== afterReload.revision
    || JSON.stringify(beforeReload.actions) !== JSON.stringify(afterReload.actions)) {
    throw new Error("saved and reloaded browser project identities differ");
  }
  socket.close();
} finally {
  await Promise.all([stopChild(brave, true), stopChild(planner)]);
  await rm(temporaryRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
