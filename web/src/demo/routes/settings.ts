// Demo routes: harnesses and models, configuration, profiles, sign-in checks, usage history, machines (SSH hosts),
// devices and pairing, Web Push and the desktop-only actions (`/api/open`, `/api/window`).

import { toast } from "@/components/Toast";
import type { Config } from "@/gen/Config";
import type { HostStatus } from "@/gen/HostStatus";
import type { QuotaStatus } from "@/gen/QuotaStatus";
import type { SignInCheck } from "@/gen/SignInCheck";
import type { ProfileSpend } from "@/gen/ProfileSpend";
import { clock, DemoError, notInDemo, onClock, publish, route, type DemoRequest } from "../daemon";
import { world } from "../world";
import { demoHost, HOME, hostChecks, seeded } from "../world/settings";

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
      : { harness: p.harness, profiles: [p.id], five_hour: { used_percent: Math.round(rand() * 20), resets_at: Math.floor(clock.ago(-180) / 1000) }, weekly: { used_percent: Math.round(rand() * 15), resets_at: Math.floor(clock.ago(-60 * 24 * 5) / 1000) }, quota_at: clock.now(), quota_error: null };
  }
  const spend: ProfileSpend[] = profiles.filter((p) => p.billing === "pay_as_you_go" && p.spend_limits.length).map((p) => {
    const old = world.status.spend.find((s) => s.profile === p.id);
    return { profile: p.id, name: p.name, harness: p.harness, windows: p.spend_limits.map((l) => ({ hours: l.hours, limit_usd: l.usd, used_usd: old?.windows.find((w) => w.hours === l.hours)?.used_usd ?? 0, available_at: null })) };
  });
  const hosts = world.status.hosts.map((h) => (h.state === "connected" ? { ...h, checks: hostChecks(h.host.destination, profiles), checked_at: clock.now() } : h));
  world.status = { ...world.status, quotas, spend, hosts };
  publishStatus();
}

function saveConfig(next: Config) {
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
  projects_root: string; remove_published_worktree: boolean; presets: Config["presets"]; notify: boolean; github_poll_interval_seconds: number;
  trace_backup_enabled: boolean; trace_backup_bucket: string; worktrees_keep: number; browser: string;
  assistant_profile: string; assistant_model: string; assistant_effort: string; assistant_auto_mode: boolean;
}>;

route("PATCH", "/api/config", ({ body }) => {
  const p = (body ?? {}) as GeneralPatch;
  const c: Config = structuredClone(world.config);
  const opt = (v: string | undefined, old: string | null) => (v === undefined ? old : v || null);
  if (p.trace_backup_enabled !== undefined) {
    c.trace_backup.enabled = p.trace_backup_enabled;
    // Nothing is uploaded in the demo; the status line still looks like a working backup.
    world.traceBackup.last_upload = p.trace_backup_enabled ? clock.now() : world.traceBackup.last_upload;
  }
  if (p.trace_backup_bucket !== undefined) c.trace_backup.bucket = p.trace_backup_bucket;
  if (p.projects_root !== undefined) c.projects_root = p.projects_root;
  if (p.remove_published_worktree !== undefined) c.remove_published_worktree = p.remove_published_worktree;
  if (p.presets !== undefined) c.presets = p.presets;
  if (p.notify !== undefined) c.agents.notify = p.notify;
  if (p.github_poll_interval_seconds !== undefined) c.github.poll_interval_seconds = Math.max(15, p.github_poll_interval_seconds);
  if (p.worktrees_keep !== undefined) c.worktrees.keep = p.worktrees_keep;
  if (p.browser !== undefined) c.commands.browser = p.browser;
  c.assistant = { profile: opt(p.assistant_profile, c.assistant.profile), model: opt(p.assistant_model, c.assistant.model), effort: opt(p.assistant_effort, c.assistant.effort), auto_mode: p.assistant_auto_mode ?? c.assistant.auto_mode };
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
route("GET", "/api/setup/trace-backup", () => world.traceBackup);

const inRange = (req: DemoRequest) => {
  const from = Number(req.query.get("from"));
  const until = Number(req.query.get("until"));
  if (!(from >= 0) || !(until > from) || until - from > 32 * 86_400_000) throw new DemoError(400, "Select a calendar month");
  return (t: number) => t >= from && t < until;
};
route("GET", "/api/analytics/runs", (req) => { const ok = inRange(req); return world.agentRuns.filter((r) => ok(r.started_at)); });

// ---------------------------------------------------------------- machines (SSH hosts)

const host = (id: string): HostStatus => world.status.hosts.find((h) => h.host.id === id) ?? notFound(`unknown host ${id}`);
function setHost(id: string, patch: Partial<HostStatus>) {
  world.status = { ...world.status, hosts: world.status.hosts.map((h) => (h.host.id === id ? { ...h, ...patch } : h)) };
  publishStatus();
}
/** Connecting takes a moment, then the machine is ready (`connecting` → `connected` with fresh checks). */
function connect(id: string) {
  setHost(id, { state: "connecting", since: clock.now(), error: null, info: null, latency_ms: null, checks: [], checked_at: null });
  setTimeout(() => {
    const h = world.status.hosts.find((x) => x.host.id === id);
    if (!h) return;
    const ready = demoHost(id, h.host.destination, world.config.profiles, h.host.projects_root);
    setHost(id, { ...ready, host: h.host, since: clock.now(), checked_at: clock.now() });
  }, 1500);
}

route("GET", "/api/hosts", () => world.status.hosts);
route("GET", "/api/hosts/ssh-config", () => world.sshConfig);
route("POST", "/api/hosts", ({ body }) => {
  const b = body as { destination: string; id?: string | null; port?: number | null; projects_root?: string | null };
  const destination = b.destination?.trim();
  if (!destination) throw new DemoError(400, "Enter an SSH destination");
  const id = (b.id?.trim() || destination.split("@").pop()!.split(".")[0]!).toLowerCase().replace(/[^a-z0-9-]/g, "-");
  if (world.status.hosts.some((h) => h.host.id === id)) throw new DemoError(400, `a host named ${id} already exists`);
  const hostRecord = { id, destination, port: b.port ?? null, projects_root: b.projects_root?.trim() || "~/projects", created_at: clock.now() };
  world.status = { ...world.status, hosts: [...world.status.hosts, { host: hostRecord, state: "connecting", error: null, since: clock.now(), info: null, latency_ms: null, prompt: null, checks: [], checked_at: null }] };
  connect(id);
  return hostRecord;
});
route("PATCH", "/api/hosts/:id", ({ params, body }) => {
  const h = host(params.id);
  const projects_root = (body as { projects_root?: string | null }).projects_root?.trim() || h.host.projects_root;
  setHost(params.id, { host: { ...h.host, projects_root } });
  return { ...h.host, projects_root };
});
route("DELETE", "/api/hosts/:id", ({ params }) => {
  host(params.id);
  const used = world.projects.filter((p) => p.path.startsWith(`${params.id}:`));
  if (used.length) throw new DemoError(400, `Remove the folders on ${params.id} first: ${used.map((p) => p.name).join(", ")}`);
  world.status = { ...world.status, hosts: world.status.hosts.filter((h) => h.host.id !== params.id) };
  publishStatus();
  return { ok: true };
});
route("POST", "/api/hosts/:id/retry", ({ params }) => { host(params.id); connect(params.id); return { ok: true }; });
route("POST", "/api/hosts/:id/checks", ({ params }) => {
  const h = host(params.id);
  const checks = h.state === "connected" ? hostChecks(h.host.destination, world.config.profiles) : [{ id: "ssh", name: "SSH connection", ok: false, detail: h.error ?? "Not connected", command: null, found: null, signed_in: null }];
  setHost(params.id, { checks, checked_at: clock.now() });
  return checks;
});
route("POST", "/api/hosts/:id/prompt", ({ params }) => { setHost(params.id, { prompt: null }); return { ok: true }; });
route("GET", "/api/hosts/:id/browse", ({ params, query }) => {
  const h = host(params.id);
  if (h.state !== "connected") throw new DemoError(400, `${params.id} is not connected`);
  const asked = query.get("path")?.trim();
  const path = !asked ? `${HOME}/projects` : asked === "~" ? HOME : asked.startsWith("~/") ? `${HOME}/${asked.slice(2)}` : asked;
  if (!path.startsWith("/")) throw new DemoError(400, "use an absolute folder or ~/…");
  const tree: Record<string, string[]> = {
    [HOME]: ["datasets", "projects"],
    [`${HOME}/projects`]: ["inference-bench", "model-training"],
    [`${HOME}/datasets`]: ["invoices-2026"],
  };
  const parent = path === "/" ? null : path.replace(/\/[^/]+\/?$/, "") || "/";
  return { path, location: `${params.id}:${path}`, parent, home: HOME, git: /\/projects\/[^/]+$/.test(path), entries: (tree[path] ?? []).map((name) => ({ name })) };
});

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

// ---------------------------------------------------------------- Web Push and desktop actions

const NO_PUSH = "Push notifications are not part of the demo.";
route("GET", "/api/push", () => ({ public_key: "", subscribed: false, titles: false }));
route("POST", "/api/push/subscribe", () => { throw new DemoError(400, NO_PUSH); });
route("POST", "/api/push/unsubscribe", () => ({ ok: true }));
route("POST", "/api/push/test", () => { throw new DemoError(400, NO_PUSH); });

const desktopOnly = () => { toast("Demo: opening links and windows is disabled."); return { ok: true }; };
route("POST", "/api/open", desktopOnly);
route("POST", "/api/window", desktopOnly);
