// Core part of the demo world: status, configuration, profiles, workspaces and folders.

import type { Config } from "@/gen/Config";
import type { Project } from "@/gen/Project";
import type { Status } from "@/gen/Status";
import type { Workspace } from "@/gen/Workspace";
import { clock } from "../daemon";
import { ORG, P, ROOT, TMP, WS } from "./ids";

const project = (id: string, name: string, repo: string | null, description: string, extra: Partial<Project> = {}): Project => ({
  id, name, path: `${ROOT}/${name}`, remote_url: repo ? `git@github.com:${repo}.git` : null, gh_repo: repo, gh_upstream: null,
  description, is_temp: false, pinned: false, created_at: clock.ago(60 * 24 * 40), worktree_setup: "", when_done: "keep", workspace_id: WS.acme, home_git: !!repo, plain: false, ...extra,
});

export function coreWorld() {
  const projects: Project[] = [
    project(P.storefront, "storefront", `${ORG}/storefront`, "TypeScript web shop", { worktree_setup: "pnpm install" }),
    project(P.billing, "billing-api", `${ORG}/billing-api`, "Rust billing service"),
    project(P.docs, "docs", `${ORG}/docs`, "Product documentation"),
    project(P.dotfiles, "dotfiles", null, "Personal configuration", { workspace_id: WS.personal }),
    project(P.scratch, "tmp-1028-csv-cleanup", null, "", { path: `${TMP}/tmp-1028-csv-cleanup`, is_temp: true }),
    // Ask Launchpad has no folder: its session runs in a temporary folder.
    project(P.ask, "tmp-1105-whats-running-in-acme", null, "", { path: `${TMP}/tmp-1105-whats-running-in-acme`, is_temp: true }),
  ];
  const workspaces: Workspace[] = [
    { id: WS.acme, name: "Acme", created_at: clock.ago(60 * 24 * 30) },
    { id: WS.personal, name: "Personal", created_at: clock.ago(60 * 24 * 30) },
  ];
  const config: Config = {
    projects_root: ROOT,
    remove_published_worktree: true,
    server: { bind: "127.0.0.1", public_origin: null, auto_tailnet: true },
    commands: { browser: "" },
    github: { poll_interval_seconds: 60 },
    agents: { notify: true },
    worktrees: { keep: 10 },
    presets: [
      { id: "implement-review", label: "Implement, then review", text: "Please implement the feature using Claude, then launch a review using Codex.", bots: true, agent: false },
      { id: "tests", label: "Write tests", text: "Add or update tests for the code you change, and run them before you finish.", bots: false, agent: true },
      { id: "commits", label: "Small commits", text: "Commit in small, focused steps with clear messages.", bots: false, agent: true },
      { id: "local-pr", label: "Open a Local PR when done", text: "When the task is done, open a Local PR with `launchpad pr create`.", bots: false, agent: true },
    ],
    profiles: [
      { id: "claude-max", name: "Claude · Max", harness: "claude", env: {}, model: null, effort: null, billing: "subscription", budget_usd: null, models_url: null, spend_limits: [], login: null },
      { id: "codex-plus", name: "Codex · Plus", harness: "codex", env: {}, model: null, effort: null, billing: "subscription", budget_usd: null, models_url: null, spend_limits: [], login: null },
      { id: "opencode-router", name: "OpenCode · HF router", harness: "opencode", env: { OPENAI_API_KEY: "${HF_TOKEN}" }, model: "open-labs/open-coder-70b", effort: null, billing: "pay_as_you_go", budget_usd: null, models_url: "https://router.example.com/v1/models", spend_limits: [{ usd: 5, hours: 24 }], login: null },
    ],
    home: { backup: "", git_name: "", git_email: "", burst: { enabled: true, monthly_cap_usd: 20, flavor: "cpu-basic" }, time_zone: "" },
    runner: { boundary: "jail" },
  };
  const status: Status = {
    env: "demo",
    role: "all",
    home: null,
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
    spend: [{ profile: "opencode-router", name: "OpenCode · HF router", harness: "opencode", windows: [{ hours: 24, limit_usd: 5, used_usd: 1.84, available_at: null }] }],
    cost_unavailable_profiles: [], runner_update: null,
  };
  return { projects, workspaces, config, status };
}
