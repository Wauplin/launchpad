// Status, configuration, workspaces, folders and harness metadata.

import type { Project } from "@/gen/Project";
import type { Workspace } from "@/gen/Workspace";
import { clock, DemoError, githubDemo, notInDemo, publish, route } from "../daemon";
import { world } from "../world";
import { ORG, P, ROOT } from "../world/ids";
import { fakeSha } from "../world/github";
import { toast } from "@/components/Toast";

route("GET", "/api/status", () => world.status);
route("GET", "/api/config", () => ({ config: world.config, error: null, remote: false }));
route("GET", "/api/workspaces", () => world.workspaces);
route("GET", "/api/projects", () => world.projects);
route("POST", "/api/log", () => ({ ok: true }));

// ---------------------------------------------------------------- folders and workspaces (mutations)

const projectOf = (id: string) => {
  const p = world.projects.find((x) => x.id === id);
  if (!p) throw new DemoError(404, "no such project");
  return p;
};
const projectsChanged = () => publish("projects", "changed", null);
const workspacesChanged = () => publish("workspaces", "changed", null);

/** A folder of Home: a cloned repository (`repo`), or a plain folder (`repo` null). */
function register(path: string, repo: string | null, workspaceId: string | null): Project {
  const clean = path.replace(/\/+$/, "");
  const workspace = world.workspaces.find((x) => x.id === workspaceId);
  if (!workspace) throw new DemoError(400, "choose a workspace for the folder");
  const existing = world.projects.find((p) => p.path === clean);
  if (existing) throw new DemoError(400, `${clean} is already registered`);
  const name = clean.split("/").pop() || "folder";
  const p: Project = {
    id: `demo-${name}-${clock.now().toString(36)}`, name, path: clean, remote_url: repo ? `https://github.com/${repo}.git` : null, gh_repo: repo, gh_upstream: null,
    description: "", is_temp: false, pinned: false, created_at: clock.now(), worktree_setup: "", when_done: "keep", workspace_id: workspace.id, home_git: !!repo, plain: !repo,
  };
  world.projects = [...world.projects, p];
  projectsChanged();
  return p;
}

const cloneSuggestions = () => [{ repo: `${ORG}/analytics`, notifications: 6 }, { repo: `${ORG}/design-system`, notifications: 2 }, { repo: `${ORG}/infra`, notifications: 0 }].filter((r) => !world.projects.some((p) => p.gh_repo === r.repo));
route("GET", "/api/projects/clone", cloneSuggestions);
route("POST", "/api/projects/clone/suggestions", cloneSuggestions);
route("POST", "/api/projects/clone", ({ body }) => {
  const repo = String(body?.repo ?? "").replace(/^https:\/\/github\.com\//, "").replace(/\.git$/, "");
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new DemoError(400, "Expected owner/repo or a GitHub URL.");
  const p = register(`${ROOT}/${repo.split("/")[1]}`, repo, body?.workspace_id ?? null);
  githubDemo(`cloned ${repo} (simulated, no files were downloaded)`);
  return p;
});

route("GET", "/api/projects/:id", ({ params }) => projectOf(params.id));
route("PATCH", "/api/projects/:id", ({ params, body }) => {
  const p = projectOf(params.id);
  if (body?.workspace_id != null && !world.workspaces.some((w) => w.id === body.workspace_id)) throw new DemoError(400, "no such workspace");
  for (const key of ["name", "description", "pinned", "worktree_setup", "when_done", "workspace_id"] as const) {
    if (body?.[key] !== null && body?.[key] !== undefined) (p as Record<string, unknown>)[key] = body[key];
  }
  projectsChanged();
  return p;
});
route("DELETE", "/api/projects/:id", ({ params }) => {
  const p = projectOf(params.id);
  world.projects = world.projects.filter((x) => x.id !== p.id);
  projectsChanged();
  return { ok: true };
});
/** Home Git: Launchpad branches of the folder's task worktrees and Local PRs, one with a previous tip. */
route("GET", "/api/home-git/:id/branches", ({ params }) => {
  const p = projectOf(params.id);
  const names = [...new Set([...world.taskWorktrees.filter((i) => i.worktree.project_id === p.id).map((i) => i.worktree.branch), ...world.predrafts.filter((d) => d.project_id === p.id).map((d) => d.branch)])];
  return names.map((name, i) => ({
    name, head: fakeSha(`${name}:head`), subject: i === 0 ? "Address review notes" : "Work in progress", committed_at: clock.ago(30 + i * 90),
    kept: i === 0 ? [{ commit: fakeSha(`${name}:before`), subject: "First draft", replaced_at: clock.ago(45) }] : [],
  }));
});
/** GitHub has every commit of the pull requests the demo's branches continue. */
route("GET", "/api/home-git/:id/follow", () => null);
route("POST", "/api/home-git/:id/restore", ({ body }) => {
  toast(`Demo: ${String(body?.branch ?? "the branch")} would point back to ${String(body?.commit ?? "").slice(0, 8)} on Home. Nothing changed.`);
  return { ok: true };
});
route("GET", "/api/plain/assets/:project", ({ params }) => params.project === P.storefront ? [
  { asset: { id: "demo-asset-fixtures", project_id: P.storefront, name: "lp/checkout-test/test/fixtures", created_at: clock.ago(60 * 24 * 3), trashed_at: null, restored_at: null }, files: 42, size: 18_400_000, last_change: clock.ago(60 * 5), in_use: true },
  { asset: { id: "demo-asset-screens", project_id: P.storefront, name: "lp/darkmode/screenshots", created_at: clock.ago(60 * 24 * 20), trashed_at: clock.ago(60 * 24 * 2), restored_at: null }, files: 0, size: 0, last_change: clock.ago(60 * 24 * 2), in_use: false },
] : []);
route("POST", "/api/plain/:id/untrash", () => notInDemo());
route("GET", "/api/plain/:id/history", () => ({ changes: [], locked_by: null }));
route("POST", "/api/projects/plain", ({ body }) => {
  const name = String(body?.name ?? "").trim();
  if (!name || name.includes("/")) throw new DemoError(400, "invalid folder name");
  return register(`${ROOT}/${name}`, null, body?.workspace_id ?? null);
});
route("GET", "/api/projects/:id/branches", ({ params }) => {
  const p = projectOf(params.id);
  if (!p.gh_repo && !world.codeTrees[p.id] || p.is_temp) throw new DemoError(400, "not a git repository");
  const task = Object.values(world.codeRoots).filter((r) => r.projectId === p.id).map((r) => r.branch);
  return { current: "main", branches: ["main", ...task, ...(p.gh_repo ? ["release/2.4"] : [])] };
});
route("POST", "/api/projects/:id/promote", ({ params, body }) => {
  const p = projectOf(params.id);
  const name = String(body?.name ?? "").trim();
  if (!/^[\w.-]+$/.test(name)) throw new DemoError(400, "Use letters, digits, dots, dashes or underscores.");
  const old = p.path;
  Object.assign(p, { name, path: `/home/demo/.local/share/launchpad-runner/plain/${p.id}`, is_temp: false, plain: true });
  if (world.codeTrees[p.id] === undefined && world.codeRoots[old]) delete world.codeRoots[old];
  projectsChanged();
  return p;
});

route("POST", "/api/workspaces", ({ body }): Workspace => {
  const name = String(body?.name ?? "").trim();
  if (!name) throw new DemoError(400, "a workspace needs a name");
  if (world.workspaces.some((w) => w.name.toLowerCase() === name.toLowerCase())) throw new DemoError(400, `a workspace named ${name} already exists`);
  const w: Workspace = { id: `demo-ws-${clock.now().toString(36)}`, name, created_at: clock.now() };
  world.workspaces = [...world.workspaces, w];
  workspacesChanged();
  return w;
});
route("PATCH", "/api/workspaces/:id", ({ params, body }): Workspace => {
  const w = world.workspaces.find((x) => x.id === params.id);
  if (!w) throw new DemoError(400, "no such workspace");
  if (body?.name != null) {
    const name = String(body.name).trim();
    if (!name) throw new DemoError(400, "a workspace needs a name");
    w.name = name;
  }
  workspacesChanged();
  return w;
});
route("DELETE", "/api/workspaces/:id", ({ params }) => {
  if (world.workspaces.length <= 1) throw new DemoError(400, "the last workspace cannot be deleted: Launchpad always shows one");
  const folders = world.projects.filter((p) => p.workspace_id === params.id).length;
  if (folders) throw new DemoError(400, `move or remove its ${folders} folder${folders === 1 ? "" : "s"} first`);
  // Its bots go with it (`workspace::delete`), with their routines.
  const bots = new Set(world.bots.filter((b) => b.workspace_id === params.id).map((b) => b.id));
  world.bots = world.bots.filter((b) => !bots.has(b.id));
  world.crons = world.crons.filter((c) => !c.bot_id || !bots.has(c.bot_id));
  world.workspaces = world.workspaces.filter((w) => w.id !== params.id);
  if (bots.size) publish("bots", "changed", {});
  workspacesChanged();
  return { ok: true };
});

// The setup checklist checks the real machine: it is not part of the demo (the sidebar entry hides there).
route("GET", "/api/setup/checklist", notInDemo);
