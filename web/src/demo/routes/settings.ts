// Demo routes: harnesses and models, configuration, profiles, sign-in checks, usage history,
// devices and pairing.

import { toast } from "@/components/Toast";
import type { Config } from "@/gen/Config";
import type { LoginCheck } from "@/gen/LoginCheck";
import type { QuotaStatus } from "@/gen/QuotaStatus";
import type { SignInCheck } from "@/gen/SignInCheck";
import type { ProfileSpend } from "@/gen/ProfileSpend";
import { clock, DemoError, notInDemo, onClock, publish, route, type DemoRequest } from "../daemon";
import { world } from "../world";
import { demoNetworkCheck, seeded } from "../world/settings";

const configResponse = () => ({ config: world.config, error: null, remote: false });

/** Publishes the status like `StatusHandle::set` (`status/changed`). */
export function publishStatus() {
  world.status = { ...world.status };
  publish("status", "changed", world.status);
}

/** `profiles::account`: the harness plus the values of its account variables. */
function account(p: Config["profiles"][number]): string {
  const vars = world.harnesses.find((h) => h.id === p.harness)?.account_env ?? [];
  return [p.harness, ...vars.flatMap((name) => (p.env[name]?.trim() ? [`${name}=${p.env[name]}`] : []))].join(" ");
}

/** After a profile change: one quota monitor per subscription login, spending meters for limited pay-as-you-go profiles, host checks again. */
function syncProfiles() {
  const profiles = world.config.profiles;
  const quotas: Record<string, QuotaStatus> = {};
  const rand = seeded(profiles.length);
  for (const p of profiles.filter((p) => p.billing === "subscription" && world.harnesses.find((h) => h.id === p.harness)?.capabilities.quota)) {
    const key = account(p);
    const old = quotas[key] ?? world.status.quotas[key];
    quotas[key] = old
      ? { ...old, profiles: quotas[key] ? [...quotas[key]!.profiles, p.id] : [p.id] }
      : { harness: p.harness, profiles: [p.id], five_hour: { used_percent: Math.round(rand() * 20), resets_at: Math.floor(clock.ago(-180) / 1000) }, weekly: { used_percent: Math.round(rand() * 15), resets_at: Math.floor(clock.ago(-60 * 24 * 5) / 1000) }, quota_at: clock.now(), quota_error: null, refused: false };
  }
  const spend: ProfileSpend[] = profiles.filter((p) => p.billing === "pay_as_you_go" && p.spend_limits.length).map((p) => {
    const old = world.status.spend.find((s) => s.profile === p.id);
    return { profile: p.id, name: p.name, harness: p.harness, windows: p.spend_limits.map((l) => ({ hours: l.hours, limit_usd: l.usd, used_usd: old?.windows.find((w) => w.hours === l.hours)?.used_usd ?? 0, available_at: null })) };
  });
  world.status = { ...world.status, quotas, spend };
  publishStatus();
}

export function saveConfig(next: Config) {
  const profilesChanged = JSON.stringify(next.profiles) !== JSON.stringify(world.config.profiles);
  world.config = next;
  publish("config", "changed", null);
  if (profilesChanged) syncProfiles();
  return configResponse();
}

// Quota readings stay fresh while the demo runs (the status bar warns about readings older than 15 minutes).
setInterval(() => {
  if (!world) return;
  const quotas = Object.fromEntries(Object.entries(world.status.quotas).map(([k, q]) => [k, { ...q, quota_at: clock.now() }]));
  world.status = { ...world.status, quotas };
  publishStatus();
}, 5 * 60_000);
onClock(() => {
  // Fast-forwarding past a reset starts a new window (the quota scenario itself is driven by the agents area).
  const now = clock.now() / 1000;
  let changed = false;
  const quotas = Object.fromEntries(Object.entries(world.status.quotas).map(([k, q]) => {
    const roll = (w: QuotaStatus["five_hour"], hours: number) => (w && w.resets_at !== null && w.resets_at <= now ? ((changed = true), { used_percent: 0, resets_at: Math.floor(now + hours * 3600) }) : w);
    return [k, { ...q, five_hour: roll(q.five_hour, 5), weekly: roll(q.weekly, 24 * 7), quota_at: clock.now() }];
  }));
  if (changed) { world.status = { ...world.status, quotas }; publishStatus(); }
});

// ---------------------------------------------------------------- harnesses and models

route("GET", "/api/harnesses", () => world.harnesses);
route("GET", "/api/harness/:kind/defaults", ({ params }) => world.harnessDefaults[params.kind] ?? notFound("unknown harness"));
route("POST", "/api/harness/:kind/defaults", ({ params }) => world.harnessDefaults[params.kind] ?? notFound("unknown harness"));
route("POST", "/api/profiles/models", () => world.routerModels);

function notFound(message: string): never { throw new DemoError(404, message); }

// ---------------------------------------------------------------- configuration

route("PUT", "/api/config", ({ body }) => {
  const next = body as Config;
  if (JSON.stringify(next.server) !== JSON.stringify(world.config.server)) throw new DemoError(400, "Edit [server] in config.toml and restart the daemon");
  const ids = new Set<string>();
  for (const p of next.profiles) {
    if (!/^[a-z0-9-]+$/.test(p.id)) throw new DemoError(400, `profile ID "${p.id}": use lowercase letters, digits and dashes`);
    if (ids.has(p.id)) throw new DemoError(400, `profile ID "${p.id}" is used twice`);
    ids.add(p.id);
  }
  return saveConfig(next);
});

type GeneralPatch = Partial<{
  presets: Config["presets"]; github_poll_interval_seconds: number; worktrees_keep: number;
}>;

route("PATCH", "/api/config", ({ body }) => {
  const p = (body ?? {}) as GeneralPatch;
  const c: Config = structuredClone(world.config);
  if (p.presets !== undefined) c.presets = p.presets;
  if (p.github_poll_interval_seconds !== undefined) c.github.poll_interval_seconds = Math.max(15, p.github_poll_interval_seconds);
  if (p.worktrees_keep !== undefined) c.worktrees.keep = p.worktrees_keep;
  return saveConfig(c);
});

// ---------------------------------------------------------------- sign-in checks and usage

route("GET", "/api/setup/sign-in", (): SignInCheck[] => {
  const ok = (id: string, name: string, detail: string, command: string, profile: string | null = null): SignInCheck => ({ id, profile, name, cli_found: true, signed_in: true, detail, command });
  const commands: Record<string, string> = { claude: "claude auth login", codex: "codex login --device-auth", opencode: "opencode auth login" };
  return [
    ok("claude", "Claude Code", "Claude Max · demo-user@example.com", commands.claude!),
    ok("codex", "Codex", "Logged in using ChatGPT", commands.codex!),
    ok("opencode", "OpenCode", "1 stored credential", commands.opencode!),
    ...world.config.profiles.filter((p) => Object.keys(p.env).length).map((p) => {
      const vars = world.harnesses.find((h) => h.id === p.harness)?.account_env ?? [];
      const prefix = vars.flatMap((v) => (p.env[v] ? [`${v}=${p.env[v]}`] : [])).join(" ");
      return ok(p.harness, p.name, p.billing === "subscription" ? "Signed in" : "HF_TOKEN from the hf CLI", `${prefix ? `${prefix} ` : ""}${commands[p.harness] ?? ""}`, p.id);
    }),
    world.githubCheck,
  ];
});
route("GET", "/api/setup/huggingface", () => world.huggingface);

const inRange = (req: DemoRequest) => {
  const from = Number(req.query.get("from"));
  const until = Number(req.query.get("until"));
  if (!(from >= 0) || !(until > from) || until - from > 32 * 86_400_000) throw new DemoError(400, "Select a calendar month");
  return (t: number) => t >= from && t < until;
};
route("GET", "/api/analytics/runs", (req) => { const ok = inRange(req); return world.agentRuns.filter((r) => ok(r.started_at)); });

// Home and runners: the demo is a Home with the owner's laptop runner and recent Burst jobs.
route("GET", "/api/runners", () => world.runners);
route("GET", "/api/runners/:id/status", () => ({ ...world.status, role: "runner", runner_update: null }));
route("POST", "/api/runners/:id/update", notInDemo);
// Agent CLI updates: the demo world's records; approving or installing needs a real runner.
const cliUpdates = () => ({
  harnesses: [["claude", "Claude Code", "@anthropic-ai/claude-code", "2.1.315"], ["codex", "Codex", "@openai/codex", "0.167.0"], ["opencode", "OpenCode", "opencode-ai", "1.18.35"]]
    .map(([harness, name, npm, latest]) => ({ harness, name, npm, latest, auto: world.config.home.cli_updates[harness] !== false, error: null })),
  checked_at: clock.ago(60 * 5),
  updates: world.cliUpdates,
});
route("GET", "/api/cli-updates", cliUpdates);
route("POST", "/api/cli-updates/check", cliUpdates);
route("PUT", "/api/cli-updates/auto", notInDemo);
route("POST", "/api/cli-updates/:id/approve", notInDemo);
route("POST", "/api/cli-updates/:id/retry", notInDemo);
route("POST", "/api/cli-updates/:id/dismiss", ({ params }: DemoRequest) => {
  const u = world.cliUpdates.find((x) => x.id === params.id);
  if (!u) throw new DemoError(404, "no such CLI update");
  u.dismissed_at = clock.now();
  publish("cli_updates", "changed", null);
  publish("today", "changed", null);
  return u;
});
// Home updates: the demo's Home is not on a Space.
const homeUpdate = () => ({ unsupported: "Only a Home on a Hugging Face Space updates itself. Update this one the way you installed it.", space: null, channel: null, deployed: null, running: world.status.commit, latest: null, available: false, building: false, stage: null, checked_at: null, error: null, pending: null });
route("GET", "/api/home/update", homeUpdate);
route("POST", "/api/home/update/check", homeUpdate);
route("POST", "/api/home/update", notInDemo);
route("POST", "/api/runners/:id/network-check", () => {
  const check = demoNetworkCheck(clock.now());
  for (const r of world.runners) r.network_check = check;
  return check;
});
route("DELETE", "/api/runners/:id/update", notInDemo);
route("GET", "/api/home/backup", () => ({ destination: null, last_complete_at: null, error: null, credentials_encrypted: false, traces_error: null, pruned_at: null, prune: null, prune_error: null, missing_blobs: null }));

// Credentials and tools: a sample vault. Secrets never reach the browser, in the demo either.
const demoTemplates = [
  { id: "gh", name: "GitHub CLI", install: { kind: "github_release" as const, source: "cli/cli", asset: "gh_*_linux_{arch}.tar.gz" }, credential: "github", env: ["GH_TOKEN", "GITHUB_TOKEN"], hosts: [{ host: "api.github.com", auth: { kind: "bearer" as const } }, { host: "github.com", auth: { kind: "basic" as const, user: "x-access-token" } }], workspaces: ["*"], check_url: "https://api.github.com/user", vars: {} },
  { id: "hf", name: "Hugging Face CLI", install: { kind: "uv" as const, source: "huggingface_hub", asset: "" }, credential: "huggingface", env: ["HF_TOKEN"], hosts: [{ host: "huggingface.co", auth: { kind: "bearer" as const } }], workspaces: ["*"], check_url: "https://huggingface.co/api/whoami-v2", vars: {} },
  ...[false, true].map((write) => ({ id: write ? "slack-write" : "slack-read", name: write ? "Slack (write)" : "Slack (read-only)", install: { kind: "github_release" as const, source: "co42/slackline", asset: "slackline-*-{uarch}-unknown-linux-gnu.tar.gz" }, credential: write ? "slack-write" : "slack-read", env: ["SLACK_TOKEN"], hosts: [{ host: "slack.com", auth: { kind: "bearer" as const } }, { host: "files.slack.com", auth: { kind: "bearer" as const } }], workspaces: ["*"], check_url: "https://slack.com/api/auth.test", vars: write ? {} : { SLACKLINE_READONLY: "1" } as Record<string, string> })),
];
const slackSteps = ["Open Slack: the app and its permissions are already filled in.", "Pick your Slack workspace, then click Next and Create.", "In the sidebar, open Install App, click Install to Workspace, then Allow.", "Copy the User OAuth Token (it starts with xoxp-) and paste it here."];
route("GET", "/api/vault", () => ({
  entries: [
    { id: "claude-max", kind: "claude_login", name: "Claude Max", detail: "…k3Qa", updated_at: clock.now() - 86_400_000 },
    // The demo's Codex profile shows quota: its login is in the vault too.
    { id: "codex-chatgpt", kind: "codex_login", name: "ChatGPT Pro", detail: "demo-user@example.com", updated_at: clock.now() - 2 * 86_400_000 },
    { id: "github", kind: "token", name: "GitHub fine-grained", detail: "…9fZ2", updated_at: clock.now() - 3 * 86_400_000 },
  ],
  tools: [demoTemplates[0]],
  templates: demoTemplates,
  template_help: [
    { id: "gh", token_url: "https://github.com/settings/personal-access-tokens/new", home_token: "github", link_label: "Create a token", steps: [] },
    { id: "hf", token_url: "https://huggingface.co/settings/tokens/new?tokenType=fineGrained", home_token: "huggingface", link_label: "Create a token", steps: [] },
    { id: "slack-read", token_url: "https://api.slack.com/apps?new_app=1", home_token: null, link_label: "Open Slack to create the app", steps: slackSteps },
    { id: "slack-write", token_url: "https://api.slack.com/apps?new_app=1", home_token: null, link_label: "Open Slack to create the app", steps: slackSteps },
  ],
  home_tokens: [],
  logins: [
    { harness: "claude", name: "Claude Code", kind: "claude_login", how: "Run claude setup-token on any machine and paste the token it prints." },
    { harness: "codex", name: "Codex", kind: "codex_login", how: "Sign in with codex login (ChatGPT) on any machine and paste the whole ~/.codex/auth.json." },
  ],
  git_name: "Demo Owner",
  git_email: "owner@example.com",
}));
route("GET", "/api/vault/leases", () => []);

// MCP servers: a sample server whose destructive tool is off.
let demoMcp = [{
  id: "gmail", name: "Gmail", url: "https://mcp.example.com/gmail", auth: { kind: "bearer" as const }, workspaces: ["*"],
  tools: [
    { name: "search_messages", description: "Search your mailbox", read_only: true, destructive: false },
    { name: "draft_reply", description: "Create a draft reply (not sent)", read_only: false, destructive: false },
    { name: "send_message", description: "Send an email", read_only: false, destructive: true },
  ],
  tools_off: ["send_message"], refreshed_at: clock.now() - 3_600_000, error: null as string | null,
}];
route("GET", "/api/mcp", () => demoMcp);
route("PUT", "/api/mcp/:id", notInDemo);
route("DELETE", "/api/mcp/:id", notInDemo);
route("POST", "/api/mcp/:id/refresh", ({ params }) => demoMcp.find((s) => s.id === params.id));
route("PUT", "/api/mcp/:id/tools/:tool", ({ params, body }) => {
  const on = (body as { on: boolean }).on;
  demoMcp = demoMcp.map((s) => s.id !== params.id ? s : { ...s, tools_off: on ? s.tools_off.filter((t) => t !== params.tool) : [...s.tools_off.filter((t) => t !== params.tool), params.tool] });
  return demoMcp.find((s) => s.id === params.id);
});
// Default tools of workspaces and folders, kept in memory.
const demoToolDefaults: { workspaces: Record<string, Record<string, boolean>>; folders: Record<string, Record<string, boolean>> } = { workspaces: {}, folders: {} };
route("GET", "/api/tool-defaults", () => demoToolDefaults);
route("GET", "/api/tool-defaults/:scope/:id", ({ params }) => ({ tools: (params.scope === "folder" ? demoToolDefaults.folders : demoToolDefaults.workspaces)[params.id] ?? {} }));
route("PUT", "/api/tool-defaults/:scope/:id", ({ params, body }) => {
  const tools = (body as { tools: Record<string, boolean> }).tools;
  const layer = params.scope === "folder" ? demoToolDefaults.folders : demoToolDefaults.workspaces;
  if (Object.keys(tools).length) layer[params.id] = tools; else delete layer[params.id];
  return { tools };
});
route("GET", "/api/burst", () => ({ config: world.config.home.burst, unavailable: null, month_usd: 0.01, runs: world.burstRuns }));
route("GET", "/api/vault/tools.toml", () => ({ toml: "[[tools]]\nid = \"gh\"\n" }));
route("PUT", "/api/vault/entries/:id", notInDemo);
route("POST", "/api/vault/entries/:id/from-home", notInDemo);
// Refresh of a harness login: the demo's Claude token always works.
route("POST", "/api/vault/entries/:id/refresh", (): LoginCheck => ({ ok: true, message: "The login works; quota read", sign_in: false, refreshed: false }));
route("DELETE", "/api/vault/entries/:id", notInDemo);
route("POST", "/api/vault/tools/:id/test", () => ({ ok: true, status: 200, message: "The token works on api.github.com." }));
route("PUT", "/api/vault/tools/:id", notInDemo);
route("DELETE", "/api/vault/tools/:id", notInDemo);
route("PUT", "/api/vault/identity", notInDemo);
route("POST", "/api/vault/leases/:id/revoke", notInDemo);
route("GET", "/api/sessions/:id/access", () => []);

// ---------------------------------------------------------------- devices and pairing

route("GET", "/api/devices", () => world.devices);
route("DELETE", "/api/devices/:id", ({ params }) => {
  const d = world.devices.find((x) => x.id === params.id) ?? notFound("no such device");
  if (d.current) throw new DemoError(400, "Signing out of this browser is not part of the demo.");
  world.devices = world.devices.filter((x) => x.id !== d.id);
  toast(`Demo: ${d.name} was revoked in the demo only.`);
  return { ok: true };
});
route("GET", "/api/devices/pairing-info", () => ({ origin: "http://127.0.0.1:7317", daemon_port: 7317, https_port: null, installed: false, connected: false }));
route("POST", "/api/devices/pair", () => {
  const code = Array.from({ length: 4 }, () => Math.floor(Math.random() * 36 ** 4).toString(36).padStart(4, "0")).join("");
  return { url: `https://demo.invalid/auth#code=demo-${code}`, svg: qrSvg(code), expires_at: Math.floor(Date.now() / 1000) + 300, warning: null };
});
route("POST", "/api/devices/reverse/check", notInDemo);
route("POST", "/api/devices/reverse/approve", notInDemo);
route("POST", "/api/devices/https/run", notInDemo);
route("POST", "/api/devices/https/check", notInDemo);

/**
 * A QR-code-looking SVG (finder, timing and alignment patterns around seeded modules). It does not encode anything:
 * the pairing link of the demo leads nowhere.
 */
function qrSvg(seed: string): string {
  const n = 29;
  const rand = seeded([...seed].reduce((a, c) => a * 31 + c.charCodeAt(0), 7));
  const grid: boolean[][] = Array.from({ length: n }, () => Array.from({ length: n }, () => rand() < 0.48));
  const reserved: boolean[][] = Array.from({ length: n }, () => Array(n).fill(false));
  const square = (x0: number, y0: number, size: number) => {
    for (let y = -1; y <= size; y++) for (let x = -1; x <= size; x++) {
      const gx = x0 + x, gy = y0 + y;
      if (gx < 0 || gy < 0 || gx >= n || gy >= n) continue;
      const edge = Math.max(Math.abs(x - (size - 1) / 2), Math.abs(y - (size - 1) / 2));
      grid[gy]![gx] = size === 7 ? edge !== 2 && edge <= 3 : edge === 0 || edge === 2;
      reserved[gy]![gx] = true;
    }
  };
  square(0, 0, 7); square(n - 7, 0, 7); square(0, n - 7, 7); square(n - 9, n - 9, 5);
  for (let i = 8; i < n - 8; i++) { if (!reserved[6]![i]) grid[6]![i] = i % 2 === 0; if (!reserved[i]![6]) grid[i]![6] = i % 2 === 0; }
  const q = 4, size = n + 2 * q;
  let d = "";
  grid.forEach((row, y) => row.forEach((on, x) => { if (on) d += `M${x + q} ${y + q}h1v1h-1z`; }));
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}" shape-rendering="crispEdges"><rect width="${size}" height="${size}" fill="#fff"/><path fill="#000" d="${d}"/></svg>`;
}

