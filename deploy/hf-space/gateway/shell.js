// Space shell: the setup pages, the slim bar above Launchpad and the Space settings drawer.
// Server state comes from GET /_space/status (see supervisor/main.py for the stages); every change goes
// through POST /_space/action/<name> with the CSRF token from that status. Sign-in terminals stream over
// /_space/login. Nothing here is stored in browser storage except the theme read in theme.js.
"use strict";

const $ = (id) => document.getElementById(id);
const params = new URLSearchParams(location.search);
// A profile that needs a sign-in: from the URL, or sent by the app ("Sign in from Space settings").
let requestedProfile = params.get("profile");

const HF_HINT = "Open the sign-in page, enter the code and approve access. This page continues when Hugging Face confirms.";
const TOOLS = {
  huggingface: {
    name: "Hugging Face",
    blurb: "Open models such as Qwen, Kimi, GLM and DeepSeek through Inference Providers, billed to your Hugging Face account. Launchpad also saves agent traces to your private bucket with this sign-in.",
    hint: HF_HINT,
  },
  space: {
    name: "Hugging Face",
    blurb: "Lets this Space manage its own secrets, backup bucket and updates. Agents never see this sign-in.",
    hint: HF_HINT,
  },
  claude: {
    name: "Claude Code",
    blurb: "Use your Claude Pro or Max subscription.",
    hint: "Open the sign-in page and approve access. Copy the code it gives you, paste it into the terminal and press Enter.",
  },
  codex: {
    name: "Codex",
    blurb: "Use your ChatGPT plan.",
    hint: "Open the sign-in page and enter the code. If ChatGPT refuses, turn on device code sign-in in its security settings first.",
  },
  opencode: {
    name: "OpenCode",
    blurb: "Any provider OpenCode supports, with your own account or key.",
    hint: "Choose a provider with the arrow keys and Enter, then follow the prompts in the terminal.",
  },
  github: {
    name: "GitHub",
    blurb: "Clone private repositories, review pull requests and use the Inbox.",
    hint: "Open the sign-in page and enter the code. Git over HTTPS uses this sign-in too.",
  },
};
const AGENTS = ["huggingface", "claude", "codex", "opencode"];

let state = null;
let connections = null;
let restarting = false; // Set after an action restarts the Space; cleared when a new process answers.
let restartingSince = 0;
const slowRestart = () => restartingSince && Date.now() - restartingSince > 5 * 60 * 1000;
let onboarding = false; // The page showed "connect your first agent"; show a finish page before the app.
let finished = false;
let selected = null; // Agent card chosen on the connect page.
let flow = null; // Active sign-in: { tool, terminal, socket, fit, observer }.
let routeLoaded = false;
let lastRoute = null;
let lastPanelRender = "";
let lastSetupRender = "";

// ---------- small helpers ----------

/** Creates an element. `attrs` keys starting with "on" become listeners; children may be strings or nodes. */
function h(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs || {})) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else if (key === "class") node.className = value;
    else node.setAttribute(key, value === true ? "" : value);
  }
  for (const child of children.flat()) if (child !== null && child !== undefined && child !== false) node.append(child);
  return node;
}

const external = (href, text, cls = "button") => h("a", { class: cls, href, target: "_blank", rel: "noreferrer" }, text, " ↗");

let toastTimer;
function toast(text, error = false) {
  const node = $("toast");
  node.textContent = text;
  node.classList.toggle("error", error);
  node.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (node.hidden = true), error ? 9000 : 5000);
}

function ago(at) {
  if (!at) return null;
  const seconds = Math.max(0, Math.floor(Date.now() / 1000 - at));
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86400)} d ago`;
}

function bytes(value) {
  if (!value && value !== 0) return null;
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (value >= 1024 && i < units.length - 1) { value /= 1024; i++; }
  return `${value.toFixed(i ? 1 : 0)} ${units[i]}`;
}

function copy(text, what) {
  navigator.clipboard.writeText(text).then(() => toast(`${what} copied`), () => toast("Copying is blocked here. Select the text and copy it.", true));
}

async function getJson(path) {
  const response = await fetch(path, { cache: "no-store" });
  const value = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(value.error || "The Space did not answer. Try again in a moment.");
  return value;
}

/** Runs a Space action, disabling `button` meanwhile. Returns the response or null after showing the error. */
async function action(name, body = {}, button = null) {
  if (button) button.disabled = true;
  try {
    const response = await fetch(`/_space/action/${name}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Space-CSRF": state?.csrf || "" },
      body: JSON.stringify(body),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || (response.status === 403 ? "Reload this page and try again." : "The Space did not answer. Try again in a moment."));
    if (value.restarting) restarting = true;
    if (value.message) toast(value.message);
    await refresh();
    return value;
  } catch (error) {
    toast(error.message, true);
    return null;
  } finally {
    if (button) button.disabled = false;
  }
}

/** A button that asks for a second click before running a consequential action. */
function confirmButton(label, confirmLabel, run, cls = "") {
  let timer;
  const button = h("button", { type: "button", class: cls }, label);
  button.addEventListener("click", async () => {
    if (!button.classList.contains("confirming")) {
      button.classList.add("confirming", "danger");
      button.textContent = confirmLabel;
      timer = setTimeout(() => { button.classList.remove("confirming"); if (!cls.includes("danger")) button.classList.remove("danger"); button.textContent = label; }, 6000);
      return;
    }
    clearTimeout(timer);
    await run(button);
  });
  return button;
}

// ---------- text size ----------

// Same key and positions as Launchpad (web/src/lib/zoom.ts); the app in the iframe follows through storage events.
const ZOOMS = [100, 110, 120, 135, 150];
function zoomPosition() {
  try {
    const n = Number(localStorage.getItem("launchpad-zoom") ?? 2);
    return Number.isInteger(n) && n >= 0 && n <= 4 ? n : 2;
  } catch { return 2; }
}

function setZoom(position) {
  try { localStorage.setItem("launchpad-zoom", String(position)); } catch { /* This page still changes. */ }
  if (!matchMedia("(max-width: 719px)").matches) document.documentElement.style.setProperty("--zoom", String(ZOOMS[position] / 100));
  lastSetupRender = lastPanelRender = "";
  render();
}

function textSize() {
  const current = zoomPosition();
  return h("div", { class: "sizes", role: "group", "aria-label": "Text size" },
    h("span", {}, "Text size"),
    ZOOMS.map((value, i) => h("button", { type: "button", "aria-pressed": String(i === current), onclick: () => setZoom(i) }, `${value}%`)));
}

// ---------- status polling ----------

async function refresh() {
  let next;
  try {
    next = await getJson("/_space/status");
  } catch {
    // Expected while the Space restarts; the restarting page keeps polling.
    if (state && !restarting) { restarting = true; restartingSince = Date.now(); render(); }
    else if (slowRestart()) render();
    return;
  }
  // A restart ends when another process answers; until then keep the restarting page.
  if (state && next.boot !== state.boot) restarting = false;
  if (next.stage === "restarting") restarting = true;
  if (!restarting) restartingSince = 0;
  else if (!restartingSince) restartingSince = Date.now();
  const wasConnect = state?.stage === "connect";
  const finishedLogin = next.login?.result === "completed" && state?.login?.result !== "completed";
  state = next;
  // A finished sign-in changes what is connected: reload the list so the page shows it.
  if (finishedLogin) void loadConnections();
  if (state.stage === "connect") onboarding = true;
  if (wasConnect && state.stage === "ready" && onboarding) connections = null;
  render();
}

function schedule() {
  const fast = !state || restarting || !state.ready || $("panel").open || flow || state.login?.running;
  setTimeout(async () => {
    if (document.visibilityState === "visible" || restarting) await refresh();
    schedule();
  }, fast ? 2000 : 15000);
}

async function loadConnections() {
  try {
    connections = await getJson("/_space/connections");
  } catch (error) {
    toast(error.message, true);
  }
  render();
}

// ---------- rendering ----------

function render() {
  if (!state) return;
  const showApp = state.stage === "ready" && !restarting && (!onboarding || finished);
  document.body.dataset.stage = restarting ? "restarting" : state.stage;
  $("bar").hidden = !showApp;
  $("app").hidden = !showApp;
  $("setup").hidden = showApp;
  if (showApp) {
    renderBar();
    if (!routeLoaded) {
      routeLoaded = true;
      lastRoute = safeRoute(params.get("route") || "/agents");
      $("app").src = lastRoute;
      if (params.has("setup") || state.backup?.reconnect) openPanel();
    }
  } else {
    renderSetup();
  }
  if ($("panel").open) renderPanel();
}

function renderBar() {
  $("available-update").hidden = !state.update?.candidate;
  const chip = $("bar-status");
  const backup = state.backup || {};
  chip.className = "chip";
  if (state.maintenance) { chip.classList.add("attn"); chip.textContent = "Maintenance in progress"; }
  else if (backup.reconnect) { chip.classList.add("attn"); chip.textContent = "Reconnect your accounts"; }
  else if (backup.error) { chip.classList.add("attn"); chip.textContent = "Backup delayed"; }
  else chip.textContent = backup.checkpoint_at ? `Backed up ${ago(backup.checkpoint_at)}` : "Backup pending";
  $("bar-space").textContent = state.space.id;
}

function steps(current) {
  const names = [["token", "Hugging Face sign-in"], ["private", "Private Space"], ["backups", "Encrypted backups"], ["agent", "First agent"]];
  const order = names.map(([key]) => key);
  const index = order.indexOf(current);
  return names.map(([key, label], i) =>
    h("li", { class: i < index ? "done" : i === index ? (state.stage === "blocked" ? "blocked" : "current") : "" },
      h("span", { class: "dot", "aria-hidden": "true" }, i < index ? "✓" : String(i + 1)), label));
}

function page(title, lead, step, ...body) {
  $("title").textContent = title;
  $("lead").textContent = lead || "";
  $("lead").hidden = !lead;
  $("steps").replaceChildren(...(step ? steps(step) : []));
  $("steps").hidden = !step;
  $("stage-body").replaceChildren(...body.flat().filter(Boolean));
  document.querySelector(".brand .sizes")?.remove();
  document.querySelector(".brand").append(textSize());
}

const logsUrl = () => `${state.space.url}?logs=container`;

function renderSetup() {
  // Polling re-renders only when something visible changed, so typed text and focus survive.
  const login = state.login || {};
  const key = JSON.stringify([restarting, slowRestart(), zoomPosition(), state.stage, state.message, state.needs_bucket, state.needs_key, state.signed_in_as, connections, selected,
    login.tool, login.profile, login.result, login.running, login.link, login.code, finished]);
  if (key === lastSetupRender) return;
  lastSetupRender = key;
  if (restarting) {
    return page("Restarting your Space", null, null,
      h("div", { class: "card" },
        h("div", { class: "waiting" }, h("span", { class: "spinner" }), h("b", {}, state?.message || "Your Space is restarting.")),
        h("p", { class: "muted small-text", style: "margin-top:10px" }, "This usually takes one to two minutes. This page reconnects on its own."),
        h("div", { class: "actions" }, external(logsUrl(), "Open Space logs")),
        slowRestart() ? h("div", {},
          h("p", { class: "small-text", style: "margin-top:14px" }, "Still not back after five minutes? Check the build in the logs. If an update failed to build, roll it back."),
          h("div", { class: "row" }, restartButton("Restart again"),
            state?.update?.rollback ? confirmButton("Roll back the update", "Confirm: lose changes since the update", (b) => action("rollback", { confirmed: true }, b), "danger") : null)) : null));
  }
  const renderers = { template, token, blocked, backups, starting, connect, ready: finish, stopped, error, loading: starting };
  (renderers[state.stage] || error)();
}

function template() {
  page("Your own Launchpad, in a private Space",
    "Run Claude Code, Codex and open-model agents from any browser. Your workspace, sign-ins and agent traces are backed up to a private bucket in your account.",
    null,
    h("div", { class: "card" },
      h("h2", {}, "Get started in three steps"),
      h("ol", { class: "small-text", style: "padding-left:18px;margin:8px 0 0;display:grid;gap:6px;color:var(--ink-2)" },
        h("li", {}, "Duplicate this Space. Keep its visibility ", h("b", {}, "Private"), ": everyone who can open it controls your agents."),
        h("li", {}, "Pick hardware. ", h("b", {}, "CPU upgrade"), " keeps agents running after you close the tab; the free CPU sleeps after 48 hours without visits."),
        h("li", {}, "Open your new Space and follow its setup. It takes about five minutes.")),
      h("div", { class: "actions" },
        h("a", { class: "button primary", href: state.duplicate_url, target: "_top" }, "Duplicate this Space"))));
}

function token() {
  const login = state.login || {};
  const save = h("button", { type: "button", class: "primary" }, "Save and continue");
  save.addEventListener("click", () => action("setup", {}, save));
  page("Connect this Space to Hugging Face",
    "Sign in with your Hugging Face account so Launchpad can create its private backup bucket and save an encryption key in this Space's secrets. Agents never see this sign-in.",
    "token",
    h("div", { class: login.tool === "space" && !state.signed_in_as ? "" : "card" },
      /expired/.test(state.message || "") ? h("p", { class: "result fail" }, state.message) : null,
      state.signed_in_as
        ? h("div", {}, h("p", { class: "result ok" }, `✓ Signed in as ${state.signed_in_as}`), h("div", { class: "actions" }, save))
        : connectFlow("space", true),
      h("p", { class: "muted small-text", style: "margin-top:12px" },
        "Launchpad stores the sign-in as the ", h("code", {}, "LP_SPACE_MANAGEMENT_TOKEN"), " secret of this Space and refreshes it automatically, creates the private bucket ",
        h("code", {}, state.bucket.id), " and an encryption key, then restarts the Space once.")));
  remountTerminal();
}

function blocked() {
  page(/bucket/i.test(state.message) ? "Make the backup bucket private" : "Make this Space private", state.message, "private",
    h("div", { class: "card" },
      h("p", {}, "Launchpad refuses to start while anyone could reach it: every visitor gets full access to your agents, files and sign-ins."),
      h("div", { class: "actions" },
        external(state.space.settings, "Open Space settings", "button primary"),
        restartButton()),
      h("p", { class: "muted small-text", style: "margin-top:12px" }, "After changing the visibility, restart the Space so Launchpad checks it again.")));
}

function restartButton(label = "Restart Space") {
  return confirmButton(label, "Confirm restart", (button) => action("restart-space", { confirmed: true }, button));
}

function backups() {
  const manual = h("div", {});
  const generate = h("button", {
    type: "button", class: "small",
    onclick: async () => {
      const value = await action("generate-key", {}, generate);
      if (!value?.backup_key) return;
      const field = h("input", { type: "text", readonly: true, value: value.backup_key, class: "mono" });
      manual.replaceChildren(
        h("label", {}, "LP_SPACE_BACKUP_KEY"),
        h("div", { class: "row" }, h("div", { class: "grow" }, field), h("button", { type: "button", class: "small", onclick: () => copy(value.backup_key, "Key") }, "Copy")),
        h("p", { class: "muted small-text", style: "margin-top:8px" }, "Add it as a secret named LP_SPACE_BACKUP_KEY in Space settings. This page forgets it when it closes."));
    },
  }, "Generate a key");
  const create = h("button", { type: "button", class: "primary" }, "Create backups and restart");
  create.addEventListener("click", () => action("setup", {}, create));
  page("Turn on encrypted backups",
    "A Space loses its disk when it restarts or rebuilds. Launchpad saves your workspace to a private bucket every minute and restores it at startup.",
    "backups",
    h("div", { class: "card" },
      h("dl", { class: "facts", style: "margin-top:0" },
        h("dt", {}, "Bucket"), h("dd", {}, h("code", {}, state.bucket.id), state.needs_bucket ? " · created private" : " · already exists"),
        h("dt", {}, "Readable"), h("dd", {}, "Files, Git history, settings and agent traces, so you can browse them on the Hub"),
        h("dt", {}, "Encrypted"), h("dd", {}, "Sign-ins, API keys and SSH keys, with a key kept in this Space's secrets")),
      h("div", { class: "actions" }, create),
      h("p", { class: "muted small-text", style: "margin-top:12px" }, "The Space restarts once (about a minute). Existing buckets are never changed."),
      state.needs_key ? h("details", {}, h("summary", {}, "Save the encryption key yourself"),
        h("p", { class: "small-text muted", style: "margin-top:8px" }, "Use this if the sign-in cannot save Space secrets."), generate, manual) : null));
}

function starting() {
  page("Starting your workspace", null, "agent",
    h("div", { class: "card" },
      h("div", { class: "waiting" }, h("span", { class: "spinner" }), h("b", {}, state.message)),
      h("p", { class: "muted small-text", style: "margin-top:10px" }, "Restoring takes longer when the backup is large. You can keep this tab open.")));
}

function connect() {
  if (!connections) void loadConnections();
  if (state.login?.tool && AGENTS.includes(state.login.tool)) selected = state.login.tool;
  const signedIn = (tool) => connections?.connections.find((c) => c.id === tool);
  const cards = AGENTS.map((tool) => {
    const info = TOOLS[tool];
    const row = signedIn(tool);
    return h("button", {
      type: "button", class: `choice${selected === tool ? " selected" : ""}`,
      onclick: () => { selected = tool; render(); },
    },
    h("b", {}, info.name),
    tool === "huggingface" ? h("span", { class: "badge" }, "Uses your HF account") : row?.connected ? h("span", { class: "badge plain" }, "Signed in") : h("span", {}),
    h("span", { class: "muted" }, info.blurb));
  });
  page("Connect your first agent", "Choose how your agents pay for models. You can add others later in Space settings.", "agent",
    h("div", { class: "choices" }, cards),
    selected ? h("div", { class: state.login?.tool === selected ? "" : "card", style: "margin-top:14px" }, connectFlow(selected, true)) : null,
    h("details", { class: "card", style: "margin-top:14px;box-shadow:none" },
      h("summary", {}, "Use a provider API key instead"), apiKeyForm()));
  remountTerminal();
}

function finish() {
  if (!connections) void loadConnections();
  const github = connections?.connections.find((c) => c.id === "github");
  const agent = connections?.connections.find((c) => c.id !== "github" && (c.profile || c.connected));
  const open = h("button", { type: "button", class: "primary", onclick: () => { finished = true; render(); } }, "Open Launchpad");
  page("You're all set", agent ? `${TOOLS[agent.id].name} is connected. One optional step remains.` : "Your first agent is connected.", null,
    h("div", { class: "card" },
      h("h2", {}, "Connect GitHub (optional)"),
      h("p", {}, TOOLS.github.blurb, " Git over HTTPS then uses this sign-in too."),
      github?.connected
        ? h("p", { class: "result ok" }, "✓ GitHub is connected", github.detail ? ` · ${github.detail}` : "")
        : state.login?.tool === "github" ? connectFlow("github", true) : h("button", { type: "button", onclick: (e) => startLogin("github", null, e.currentTarget) }, "Connect GitHub"),
      h("div", { class: "actions" }, open)));
  remountTerminal();
}

function stopped() {
  const restart = h("button", { type: "button", class: "primary" }, "Restart Launchpad");
  restart.addEventListener("click", () => action("restart-daemon", {}, restart));
  page("Launchpad stopped", state.message, null,
    h("div", { class: "card" },
      h("p", {}, "Your files and backups are safe. Restarting starts Launchpad from this machine's current state."),
      h("div", { class: "actions" }, restart, external(logsUrl(), "Open Space logs"))));
}

function error() {
  const rollback = state.update?.rollback
    ? confirmButton("Roll back the last update", "Confirm: lose changes since the update", (b) => action("rollback", { confirmed: true }, b), "danger")
    : null;
  page("Setup could not finish", state.message, null,
    h("div", { class: "card" },
      h("p", {}, "Your backups are untouched. Fix the cause on the Hub, then restart the Space."),
      h("div", { class: "actions" }, restartButton(), external(state.space.settings, "Open Space settings"), external(logsUrl(), "Open Space logs")),
      rollback ? h("div", { class: "actions" }, rollback) : null));
}

// ---------- sign-in flows ----------

/** The sign-in UI for one tool: a button, then a live terminal with the sign-in page and the code. */
function connectFlow(tool, primary = false) {
  const info = TOOLS[tool];
  const login = state.login || {};
  const mine = login.tool === tool;
  const profile = connections?.profiles.find((p) => p.id === (mine ? login.profile : requestedProfile));
  const title = `Sign in to ${info.name}${profile ? ` for profile ${profile.name}` : ""}`;
  if (!mine) {
    const start = h("button", { type: "button", class: primary ? "primary" : "" }, `Sign in to ${info.name}`);
    start.addEventListener("click", () => startLogin(tool, profile?.id || null, start));
    return h("div", {}, h("h2", {}, title), h("p", { class: "muted" }, info.blurb), h("div", { class: "actions", style: "margin-top:8px" }, start));
  }
  const node = h("div", { class: "flow", id: "flow" },
    h("div", { class: "flow-head" },
      h("b", {}, title),
      login.running ? h("span", { class: "spinner", title: "Waiting for sign-in" }) : null),
    h("p", { class: "hint" }, info.hint));
  const links = h("div", { class: "flow-actions" });
  if (login.link) links.append(h("a", { class: "button primary", href: login.link, target: "_blank", rel: "noreferrer" }, "Open sign-in page ↗"));
  if (login.code) links.append(h("span", { class: "code" }, login.code), h("button", { type: "button", class: "small", onclick: () => copy(login.code, "Code") }, "Copy code"));
  if (links.childElementCount) node.append(links);
  node.append(h("div", { class: "term", id: "term" }));
  if (login.result === "completed") node.append(h("p", { class: "result ok" }, "✓ Signed in"));
  if (login.result === "failed" || login.result === "expired") {
    node.append(h("p", { class: "result fail" }, login.result === "expired" ? "The sign-in expired after 15 minutes." : "The sign-in did not finish."));
  }
  const close = h("button", { type: "button", class: "small" }, login.running ? "Cancel" : "Close");
  close.addEventListener("click", async () => { closeTerminal(); await action("login-cancel", {}, close); });
  const retry = !login.running && login.result !== "completed"
    ? h("button", { type: "button", class: "small", onclick: (e) => startLogin(tool, login.profile, e.currentTarget) }, "Try again") : null;
  node.append(h("div", { class: "flow-actions", style: "margin-bottom:0" }, close, retry));
  return node;
}

async function startLogin(tool, profile, button) {
  closeTerminal();
  const value = tool === "space" ? await action("space-login", {}, button) : await action("login", { tool, profile }, button);
  if (value) {
    lastSetupRender = lastPanelRender = "";
    render();
    $("flow")?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }
}

function apiKeyForm() {
  const tool = h("select", { id: "api-tool" },
    h("option", { value: "claude" }, "Claude Code · Anthropic key"),
    h("option", { value: "codex" }, "Codex · OpenAI key"),
    h("option", { value: "opencode-anthropic" }, "OpenCode · Anthropic key"),
    h("option", { value: "opencode-openai" }, "OpenCode · OpenAI key"));
  const key = h("input", { type: "password", id: "api-key", autocomplete: "off", spellcheck: "false", required: true });
  const paid = h("input", { type: "checkbox", required: true });
  const submit = h("button", { type: "submit" }, "Verify and add");
  return h("form", {
    onsubmit: async (event) => {
      event.preventDefault();
      const [harness, provider] = tool.value.includes("-") ? tool.value.split("-") : [tool.value, tool.value === "codex" ? "openai" : "anthropic"];
      const value = key.value.trim();
      key.value = "";
      if (await action("api-key", { tool: harness, provider, key: value, paid: paid.checked }, submit)) await loadConnections();
    },
  },
  h("p", { class: "muted small-text", style: "margin-top:8px" }, "API usage is billed by the provider, separately from any subscription. Launchpad checks the key before saving it and encrypts it in backups."),
  h("label", { for: "api-tool" }, "Agent and provider"), tool,
  h("label", { for: "api-key" }, "API key"), key,
  h("label", { class: "check" }, paid, h("span", {}, "I understand that usage is billed to this key")),
  h("div", { class: "actions", style: "margin-top:10px" }, submit));
}

// ---------- terminal ----------

function closeTerminal() {
  if (!flow) return;
  flow.observer?.disconnect();
  flow.socket?.close();
  flow.terminal?.dispose();
  flow = null;
}

/** Keeps one xterm attached to the sign-in PTY; re-renders move its element instead of reconnecting. */
function remountTerminal() {
  const slot = $("term");
  const login = state.login || {};
  if (!slot || !login.tool) {
    if (!login.tool) closeTerminal();
    return;
  }
  if (flow && flow.tool === login.tool) {
    slot.replaceWith(flow.root);
    flow.root.id = "term";
    return;
  }
  closeTerminal();
  if (!window.Terminal) return;
  const terminal = new window.Terminal({
    fontFamily: '"Ubuntu Sans Mono", "Ubuntu Mono", ui-monospace, monospace',
    fontSize: 13,
    cursorBlink: true,
    theme: { background: "#0d1117", foreground: "#e6edf3" },
  });
  const fit = new window.FitAddon.FitAddon();
  terminal.loadAddon(fit);
  terminal.open(slot);
  const socket = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/_space/login`);
  socket.binaryType = "arraybuffer";
  const sendSize = () => {
    try { fit.fit(); } catch { /* Hidden element. */ }
    if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ rows: terminal.rows, cols: terminal.cols }));
  };
  socket.addEventListener("open", sendSize);
  socket.addEventListener("message", (event) => terminal.write(new Uint8Array(event.data)));
  socket.addEventListener("close", () => void refresh());
  terminal.onData((data) => socket.readyState === WebSocket.OPEN && socket.send(JSON.stringify({ input: data })));
  const observer = new ResizeObserver(sendSize);
  observer.observe(slot);
  flow = { tool: login.tool, terminal, socket, fit, observer, root: slot };
  terminal.focus();
}

// ---------- Space settings drawer ----------

function openPanel() {
  if (!$("panel").open) $("panel").showModal();
  lastPanelRender = "";
  renderPanel();
  void loadConnections();
}

function renderPanel() {
  // Rebuilding while a form has focus would drop typed text; skip until focus leaves.
  const active = document.activeElement;
  // The sign-in terminal's hidden textarea is not a form field: keep updating around it.
  if (active && $("panel").contains(active) && /INPUT|TEXTAREA|SELECT/.test(active.tagName)
      && !active.classList.contains("xterm-helper-textarea")) return;
  const login = state.login || {};
  const key = JSON.stringify([state.stage, state.backup, state.update, state.maintenance, connections, login, Math.floor(Date.now() / 30000)]);
  if (key === lastPanelRender) return;
  lastPanelRender = key;
  $("panel-body").replaceChildren(accountsSection(), backupSection(), updatesSection(), spaceSection(),
    matchMedia("(max-width: 719px)").matches ? null : h("section", {}, h("h3", {}, "This device"), textSize(),
      h("p", { class: "muted small-text", style: "margin-top:8px" }, "Applies to Launchpad and this page in this browser. Also in Launchpad → Settings → Appearance.")));
  remountTerminal();
}

function accountsSection() {
  const login = state.login || {};
  const wanted = requestedProfile;
  const rows = (connections?.connections || []).map((c) => {
    const info = TOOLS[c.id];
    const button = h("button", { type: "button", class: "small" }, c.connected ? "Reconnect" : "Connect");
    button.addEventListener("click", () => startLogin(c.id, null, button));
    // Drop file locations such as "(/home/agent/.config/gh/hosts.yml)": they only make the line longer.
    const account = c.detail?.replace(/\s*\((?:\/|~\/)[^)]*\)/g, "").trim();
    const detail = c.connected ? (account ? `Connected · ${account}` : "Connected") : c.profile ? "Profile saved · not signed in" : "Not connected";
    return h("div", { class: "item" },
      h("span", { class: `state-dot${c.connected ? " on" : c.profile ? " warn" : ""}` }),
      h("div", { class: "grow" }, h("b", {}, info.name), h("span", { title: detail }, detail)),
      button);
  });
  const profileRequest = wanted && connections?.profiles.find((p) => p.id === wanted);
  return h("section", {},
    h("h3", {}, "Agents and accounts"),
    profileRequest && login.tool !== profileRequest.harness
      ? h("div", { class: "notice attn" }, h("p", {}, `Profile ${profileRequest.name} needs a sign-in.`),
        h("button", { type: "button", class: "primary small", onclick: (e) => startLogin(profileRequest.harness, profileRequest.id, e.currentTarget) }, `Sign in to ${TOOLS[profileRequest.harness]?.name || profileRequest.harness}`))
      : null,
    connections ? h("div", { class: "list" }, rows) : h("div", { class: "waiting muted" }, h("span", { class: "spinner" }), "Checking sign-ins…"),
    login.tool ? connectFlow(login.tool) : null,
    h("details", {}, h("summary", {}, "Add a provider API key"), apiKeyForm()),
    h("details", {}, h("summary", {}, "SSH key"), sshBlock()));
}

function sshBlock() {
  const out = h("div", {});
  const show = h("button", { type: "button", class: "small" }, connections?.ssh ? "Show public key" : "Create an SSH key");
  show.addEventListener("click", async () => {
    const value = await action("ssh-key", {}, show);
    if (!value?.public_key) return;
    out.replaceChildren(h("pre", {}, value.public_key), h("p", { class: "muted small-text" }, value.fingerprint),
      h("button", { type: "button", class: "small", onclick: () => copy(value.public_key, "Public key") }, "Copy public key"));
  });
  const privateKey = h("textarea", { rows: "4", autocomplete: "off", spellcheck: "false", required: true });
  const submit = h("button", { type: "submit", class: "small" }, "Import key");
  return h("div", {},
    h("p", { class: "muted small-text" }, "Prefer Git over HTTPS (connect GitHub): Spaces may block outbound SSH on port 22. Add the public key to your Git host if you use SSH."),
    h("div", { class: "row" }, show), out,
    connections?.ssh ? null : h("details", {}, h("summary", {}, "Import an existing key"),
      h("form", {
        onsubmit: async (event) => {
          event.preventDefault();
          const value = privateKey.value;
          privateKey.value = "";
          if (await action("ssh-import", { private_key: value }, submit)) await loadConnections();
        },
      }, h("label", {}, "Unencrypted private key"), privateKey, h("div", { class: "actions", style: "margin-top:8px" }, submit))));
}

function backupSection() {
  const b = state.backup || {};
  const now = h("button", { type: "button", class: "small" }, "Back up now");
  now.addEventListener("click", () => action("backup", {}, now));
  const reconnect = h("button", { type: "button", class: "primary small" }, "Start reconnecting");
  reconnect.addEventListener("click", () => action("reconnect", {}, reconnect));
  return h("section", {},
    h("h3", {}, "Backups"),
    b.reconnect ? h("div", { class: "notice attn" },
      h("p", {}, h("strong", {}, "Sign-ins could not be restored. "), b.reconnect_reason || ""),
      h("p", { class: "small-text" }, "Your files and settings were recovered. Start a new encrypted copy, then reconnect your accounts above. The old copy stays in the bucket."),
      reconnect) : null,
    b.error ? h("div", { class: "notice attn" }, h("strong", {}, "Backup delayed. "), b.error) : null,
    h("div", { class: "lines" },
      h("div", {}, h("span", {}, "Workspace"), h("span", {}, b.checkpoint_at ? `Saved ${ago(b.checkpoint_at)}${b.bytes ? ` · ${bytes(b.bytes)}` : ""}` : "Not saved yet")),
      h("div", {}, h("span", {}, "Sign-ins"), h("span", {}, b.credentials_pending ? "Encrypting…" : b.credentials_at ? `Encrypted ${ago(b.credentials_at)}` : "Nothing to save yet")),
      h("div", {}, h("span", {}, "Agent traces"), h("span", {}, "Launchpad → Settings → Hugging Face"))),
    h("div", { class: "row" }, now, external(state.bucket.url, "Open bucket", "button small")),
    h("p", { class: "muted small-text", style: "margin-top:10px" }, "Saved every minute to the private bucket. Running commands and terminal screens cannot be restored after a restart."));
}

function updatesSection() {
  const u = state.update || {};
  const release = state.release || {};
  const check = h("button", { type: "button", class: "small" }, "Check now");
  check.addEventListener("click", () => action("check-updates", {}, check));
  const c = u.candidate;
  const managedImage = Boolean(u.installed?.image);
  return h("section", { id: "updates-section" },
    h("h3", {}, "Updates"),
    h("p", { class: "muted small-text" }, "Checks main automatically about every six hours. Each new commit becomes available after its runtime build passes. Updates only run when you confirm."),
    h("div", { class: "lines" },
      h("div", {}, h("span", {}, "Launchpad"), h("span", { class: "mono" }, (release.launchpad || "unknown").slice(0, 12))),
      h("div", {}, h("span", {}, "Space connector"), h("span", {}, release.adapter || "unknown")),
      h("div", {}, h("span", {}, "Last check"), h("span", {}, u.error || (u.checked_at ? ago(u.checked_at) : "Not checked yet")))),
    c ? h("div", { class: "notice ok" },
      h("p", {}, h("b", {}, `A new version is available: ${c.launchpad.slice(0, 12)}`)),
      h("p", {}, "Would you like to update Launchpad?"),
      h("pre", {}, Object.entries(c.tools).map(([k, v]) => `${k}: ${release.tools?.[k] || "—"} → ${v}`).join("\n")),
      h("p", { class: "small-text" }, managedImage
        ? "Stop agents and close terminals first. Launchpad saves a verified backup, then the Space rebuilds and reconnects this page."
        : "This Space was built from source. Back up now and stop agents and terminals, then replace its Dockerfile in Hub with the one in the template download. This one-time setup enables future confirmed updates."),
      h("div", { class: "row" },
        managedImage
          ? confirmButton("Update and restart", "Confirm update", (b) => action("prepare-update", { image: c.image, confirmed: true }, b), "primary small")
          : external("https://github.com/Wauplin/launchpad/raw/refs/heads/hf-space-channel/hf-space-template.zip", "Download managed template", "button primary small"),
        typeof c.notes === "string" && c.notes.startsWith("https://") ? external(c.notes, "View commit", "button small") : null)) : null,
    h("div", { class: "row" }, check));
}

function spaceSection() {
  const runtime = state.policy?.runtime || {};
  const hardware = runtime.hardware?.current || runtime.hardware?.requested || "Unknown";
  const sleep = runtime.gcTimeout === -1 || runtime.gcTimeout === null ? "Never" : runtime.gcTimeout ? `After ${Math.round(runtime.gcTimeout / 3600)} h without visits` : "Unknown";
  const u = state.update || {};
  return h("section", {},
    h("h3", {}, "This Space"),
    h("div", { class: "lines" },
      h("div", {}, h("span", {}, "Space"), h("span", {}, state.space.id)),
      h("div", {}, h("span", {}, "Hardware"), h("span", {}, hardware)),
      h("div", {}, h("span", {}, "Sleeps"), h("span", {}, sleep)),
      h("div", {}, h("span", {}, "Free disk"), h("span", {}, bytes(state.disk?.free) || "Unknown"))),
    /cpu-basic/.test(hardware) ? h("p", { class: "muted small-text" }, "Free CPU sleeps after 48 hours without visits. Choose CPU upgrade in Space settings to keep agents running.") : null,
    h("div", { class: "row" },
      external(state.space.settings, "Space settings", "button small"),
      confirmButton("Restart…", "Confirm restart", (b) => action("restart", { confirmed: true }, b), "small")),
    h("details", {},
      h("summary", {}, "Advanced"),
      h("p", { class: "muted small-text" }, "Re-encrypt every saved sign-in with a new key and save it as a new Space secret. Agents must be idle; the Space restarts."),
      confirmButton("Rotate encryption key", "Confirm rotation", (b) => action("rotate-key", { confirmed: true }, b), "small"),
      u.rollback ? h("div", {},
        h("p", { class: "muted small-text", style: "margin-top:12px" }, "Restore the previous version and the backup taken just before the last update. Changes made since then are lost."),
        confirmButton("Roll back the last update", "Confirm roll back", (b) => action("rollback", { confirmed: true }, b), "small danger")) : null,
      h("pre", {}, `Image: ${u.installed?.image || "built from source"}\nTools: ${Object.entries(state.release?.tools || {}).map(([k, v]) => `${k} ${v}`).join(", ")}`)));
}

// ---------- app route sync ----------

function safeRoute(value) {
  try {
    const url = new URL(value, location.origin);
    return url.origin === location.origin && !url.pathname.startsWith("/_space") ? url.pathname + url.search + url.hash : "/agents";
  } catch {
    return "/agents";
  }
}

function syncRoute(route) {
  const next = safeRoute(route);
  if (next === lastRoute) return;
  lastRoute = next;
  history.replaceState({}, "", `/_space/shell?route=${encodeURIComponent(next)}`);
}

window.addEventListener("message", (event) => {
  if (event.origin !== location.origin || event.source !== $("app").contentWindow) return;
  if (event.data?.type === "launchpad-route") syncRoute(event.data.route);
  if (event.data?.type === "launchpad-space-settings") {
    requestedProfile = typeof event.data.profile === "string" ? event.data.profile : null;
    openPanel();
  }
});
$("app").addEventListener("load", () => {
  try { syncRoute($("app").contentWindow.location.href); } catch { /* Cross-origin pages are not ours. */ }
});

$("available-update").addEventListener("click", () => { openPanel(); $("updates-section")?.scrollIntoView({ block: "start" }); });
$("open-panel").addEventListener("click", openPanel);
$("bar-status").addEventListener("click", openPanel);
$("close-panel").addEventListener("click", () => $("panel").close());
$("panel").addEventListener("close", () => { if (!state?.login?.running) { selected = null; } });

void refresh().then(schedule);
