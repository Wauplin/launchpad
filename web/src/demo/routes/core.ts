// Status, configuration, workspaces, folders and harness metadata.

import type { Discovered } from "@/gen/Discovered";
import type { Project } from "@/gen/Project";
import type { Workspace } from "@/gen/Workspace";
import { clock, DemoError, githubDemo, notInDemo, publish, route } from "../daemon";
import { world } from "../world";
import { ORG, ROOT } from "../world/ids";

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

/** Folders that exist on the fake disk but are not registered yet (Add folder, Clone from GitHub). */
const EXTRA: { name: string; git: boolean; repo: string | null }[] = [
  { name: "analytics", git: true, repo: `${ORG}/analytics` },
  { name: "design-system", git: true, repo: `${ORG}/design-system` },
  { name: "notes", git: false, repo: null },
];

function register(path: string, repo: string | null, workspaceId: string | null): Project {
  const clean = path.replace(/\/+$/, "");
  const existing = world.projects.find((p) => p.path === clean);
  if (existing) throw new DemoError(400, `${clean} is already registered`);
  const name = clean.split("/").pop() || "folder";
  const known = EXTRA.find((e) => e.name === name);
  const p: Project = {
    id: `demo-${name}-${clock.now().toString(36)}`, name, path: clean, remote_url: (repo ?? known?.repo) ? `git@github.com:${repo ?? known?.repo}.git` : null, gh_repo: repo ?? known?.repo ?? null, gh_upstream: null,
    description: "", is_temp: false, pinned: false, created_at: clock.now(), worktree_setup: "", merge_when_done: false,
  };
  world.projects = [...world.projects, p];
  if (workspaceId) {
    const w = world.workspaces.find((x) => x.id === workspaceId);
    if (w && !w.project_ids.includes(p.id)) w.project_ids = [...w.project_ids, p.id];
    workspacesChanged();
  }
  projectsChanged();
  return p;
}

route("GET", "/api/projects/browse", ({ query }) => {
  const home = "/home/demo";
  const path = (query.get("path") || ROOT).replace(/^~/, home).replace(/\/+$/, "") || "/";
  const registered = world.projects.filter((p) => !p.is_temp).map((p) => p.name);
  const entries: Record<string, { name: string; git: boolean }[]> = {
    [home]: [{ name: "Documents", git: false }, { name: "Downloads", git: false }, { name: "projects", git: false }],
    [ROOT]: [...registered.map((name) => ({ name, git: name !== "tmp-1028-csv-cleanup" })), ...EXTRA.map((e) => ({ name: e.name, git: e.git }))].sort((a, b) => a.name.localeCompare(b.name)),
    [`${home}/Documents`]: [{ name: "invoices-2026", git: false }],
  };
  if (!path.startsWith("/")) throw new DemoError(400, "use an absolute folder or ~/…");
  const parent = path === "/" ? null : path.slice(0, path.lastIndexOf("/")) || "/";
  const name = path.split("/").pop() ?? "";
  const git = world.projects.some((p) => p.path === path && p.gh_repo) || EXTRA.some((e) => `${ROOT}/${e.name}` === path && e.git);
  return { path, location: path, parent, home, git, entries: entries[path] ?? (path === "/home" ? [{ name: "demo", git: false }] : path === "/" ? [{ name: "home", git: false }] : name ? [] : []) };
});

route("POST", "/api/projects/discover", (): Discovered[] => [
  ...world.projects.filter((p) => !p.is_temp && p.path.startsWith(`${ROOT}/`)).map((p) => ({ name: p.name, path: p.path, gh_repo: p.gh_repo, registered: true })),
  ...EXTRA.filter((e) => e.git && !world.projects.some((p) => p.path === `${ROOT}/${e.name}`)).map((e) => ({ name: e.name, path: `${ROOT}/${e.name}`, gh_repo: e.repo, registered: false })),
]);

route("POST", "/api/projects", ({ body }) => {
  if (body?.host) throw new DemoError(400, "SSH machines are not part of this step of the demo.");
  return register(String(body?.path ?? "").replace(/^~/, "/home/demo"), null, body?.workspace_id ?? null);
});

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
  for (const key of ["name", "description", "pinned", "worktree_setup", "merge_when_done"] as const) {
    if (body?.[key] !== null && body?.[key] !== undefined) (p as Record<string, unknown>)[key] = body[key];
  }
  projectsChanged();
  return p;
});
route("DELETE", "/api/projects/:id", ({ params }) => {
  const p = projectOf(params.id);
  world.projects = world.projects.filter((x) => x.id !== p.id);
  for (const w of world.workspaces) w.project_ids = w.project_ids.filter((id) => id !== p.id);
  projectsChanged();
  workspacesChanged();
  return { ok: true };
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
  if (world.projects.some((x) => x.name === name && x.id !== p.id)) throw new DemoError(400, `${name} already exists in the folders root`);
  const old = p.path;
  Object.assign(p, { name, path: `${ROOT}/${name}`, is_temp: false });
  if (world.codeTrees[p.id] === undefined && world.codeRoots[old]) delete world.codeRoots[old];
  projectsChanged();
  return p;
});

route("POST", "/api/workspaces", ({ body }): Workspace => {
  const name = String(body?.name ?? "").trim();
  if (!name) throw new DemoError(400, "a workspace needs a name");
  if (world.workspaces.some((w) => w.name.toLowerCase() === name.toLowerCase())) throw new DemoError(400, `a workspace named ${name} already exists`);
  const w: Workspace = { id: `demo-ws-${clock.now().toString(36)}`, name, project_ids: body?.project_ids ?? [], created_at: clock.now() };
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
  if (body?.project_ids != null) w.project_ids = body.project_ids;
  workspacesChanged();
  return w;
});
route("DELETE", "/api/workspaces/:id", ({ params }) => {
  world.workspaces = world.workspaces.filter((w) => w.id !== params.id);
  workspacesChanged();
  return { ok: true };
});

// The setup checklist checks the real machine: it is not part of the demo (the sidebar entry hides there).
route("GET", "/api/setup/checklist", notInDemo);
route("GET", "/api/setup/checklist/summary", notInDemo);
