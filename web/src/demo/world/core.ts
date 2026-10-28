// Core part of the demo world: status, configuration, profiles, workspaces and folders.

import type { Config } from "@/gen/Config";
import type { Project } from "@/gen/Project";
import type { Status } from "@/gen/Status";
import type { Workspace } from "@/gen/Workspace";
import { clock } from "../daemon";
import { ORG, P, ROOT, WS } from "./ids";
import { demoHost } from "./settings";

const project = (id: string, name: string, repo: string | null, description: string, extra: Partial<Project> = {}): Project => ({
  id, name, path: `${ROOT}/${name}`, remote_url: repo ? `git@github.com:${repo}.git` : null, gh_repo: repo, gh_upstream: null,
  description, is_temp: false, pinned: false, created_at: clock.ago(60 * 24 * 40), worktree_setup: "", merge_when_done: false, ...extra,
});

export function coreWorld() {
  const projects: Project[] = [
    project(P.storefront, "storefront", `${ORG}/storefront`, "TypeScript web shop", { worktree_setup: "pnpm install" }),
    project(P.billing, "billing-api", `${ORG}/billing-api`, "Rust billing service"),
    project(P.docs, "docs", `${ORG}/docs`, "Product documentation"),
    project(P.dotfiles, "dotfiles", null, "Personal configuration"),
    project(P.scratch, "tmp-1028-csv-cleanup", null, "", { path: `${ROOT}/tmp-1028-csv-cleanup`, is_temp: true }),
  ];
  const workspaces: Workspace[] = [
    { id: WS.acme, name: "Acme", project_ids: [P.storefront, P.billing, P.docs], created_at: clock.ago(60 * 24 * 30) },
    { id: WS.personal, name: "Personal", project_ids: [P.dotfiles], created_at: clock.ago(60 * 24 * 30) },
  ];
  const config: Config = {
    projects_root: ROOT,
    remove_published_worktree: true,
    server: { bind: "127.0.0.1", public_origin: null, auto_tailnet: true },
    commands: { browser: "" },
    github: { poll_interval_seconds: 60 },
    trace_backup: { enabled: false, bucket: "launchpad-traces" },
    agents: { notify: true },
    worktrees: { keep: 10 },
    presets: [
      { id: "tests", label: "Write tests", text: "Add or update tests for the code you change, and run them before you finish." },
      { id: "commits", label: "Small commits", text: "Commit in small, focused steps with clear messages." },
      { id: "local-pr", label: "Open a Local PR when done", text: "When the task is done, open a Local PR with `launchpad pr create`." },
    ],
    profiles: [
      { id: "claude-max", name: "Claude · Max", harness: "claude", env: {}, model: null, effort: null, billing: "subscription", budget_usd: null, models_url: null, spend_limits: [], send_credentials: false },
      { id: "codex-plus", name: "Codex · Plus", harness: "codex", env: {}, model: null, effort: null, billing: "subscription", budget_usd: null, models_url: null, spend_limits: [], send_credentials: false },
      { id: "opencode-router", name: "OpenCode · HF router", harness: "opencode", env: { OPENAI_API_KEY: "${HF_TOKEN}" }, model: "open-labs/open-coder-70b", effort: null, billing: "pay_as_you_go", budget_usd: null, models_url: "https://router.example.com/v1/models", spend_limits: [{ usd: 5, hours: 24 }], send_credentials: false },
    ],
    assistant: { profile: null, model: null, effort: null },
  };
  const status: Status = {
    env: "demo",
    commit: "demo",
    host: "this-computer",
    harnesses: {
      claude: { version: "2.1.0", version_error: null, updating_to: null },
      codex: { version: "0.80.0", version_error: null, updating_to: null },
      opencode: { version: "1.2.0", version_error: null, updating_to: null },
    },
    // One reading per subscription login (key: `profiles::account`, the harness alone without account variables).
    // Claude is close to its 5-hour limit, so the meters and quota protection have something to show.
    quotas: {
      claude: { harness: "claude", profiles: ["claude-max"], five_hour: { used_percent: 86, resets_at: Math.floor(clock.ago(-47) / 1000) }, weekly: { used_percent: 41, resets_at: Math.floor(clock.ago(-60 * 24 * 4.7) / 1000) }, quota_at: clock.ago(2), quota_error: null },
      codex: { harness: "codex", profiles: ["codex-plus"], five_hour: { used_percent: 23, resets_at: Math.floor(clock.ago(-60 * 3.2) / 1000) }, weekly: { used_percent: 35, resets_at: Math.floor(clock.ago(-60 * 24 * 2.4) / 1000) }, quota_at: clock.ago(1), quota_error: null },
    },
    sessions: { running: 0, done: 0, quota_paused: 0 },
    time_zone: Intl.DateTimeFormat().resolvedOptions().timeZone ?? null,
    utc_offset_minutes: -new Date().getTimezoneOffset(),
    hosts: [demoHost("gpu-box", "gpu-box", config.profiles)],
    spend: [{ profile: "opencode-router", name: "OpenCode · HF router", harness: "opencode", windows: [{ hours: 24, limit_usd: 5, used_usd: 1.84, available_at: null }] }],
    cost_unavailable_profiles: [],
  };
  return { projects, workspaces, config, status };
}
