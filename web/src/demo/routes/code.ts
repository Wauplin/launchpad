// Demo routes: the Files viewer (`/api/code/*`), Find, commit diffs and task worktrees (`/api/worktrees*`).

import { toast } from "@/components/Toast";
import type { CodeFile } from "@/gen/CodeFile";
import type { CodeSources } from "@/gen/CodeSources";
import type { CommitInfo } from "@/gen/CommitInfo";
import type { FileList } from "@/gen/FileList";
import type { SearchHit } from "@/gen/SearchHit";
import type { SearchResult } from "@/gen/SearchResult";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import type { DiffFile } from "@/features/github/types";
import { clock, DemoError, publish, route } from "../daemon";
import { world } from "../world";
import { codeRepoOf, DEMO_COMMITS, languageOf, lineMarks, unifiedDiff, type Change } from "../world/code";
import { P } from "../world/ids";

// ---------------------------------------------------------------- sources

export type Source = { projectId: string; root: string; branch: string | null; files: Record<string, string>; changes: Change[]; base: string | null };

const HEADS: Record<string, string> = { [P.storefront]: DEMO_COMMITS.storefrontHead, [P.billing]: DEMO_COMMITS.billingHead, [P.docs]: DEMO_COMMITS.docsHead };
const headOf = (projectId: string) => HEADS[projectId] ?? fakeHead(projectId);
const fakeHead = (s: string) => Array.from({ length: 40 }, (_, i) => "0123456789abcdef"[(s.charCodeAt(i % s.length) * (i + 7)) % 16]).join("");

/** The base tree of a folder (folders added during the demo get a README only). */
function treeOf(projectId: string): Record<string, string> {
  const project = world.projects.find((p) => p.id === projectId);
  return world.codeTrees[projectId] ?? { "README.md": `# ${project?.name ?? projectId}\n\nA folder added during the demo.\n` };
}

/** Like `code::open`: only a registered folder or one of its worktrees, at an optional revision. */
export function openSource(root: string, rev?: string | null): Source {
  const project = world.projects.find((p) => p.path === root);
  if (project) {
    const wt = rev ? Object.entries(world.codeRoots).find(([, r]) => r.projectId === project.id && r.branch === rev) : undefined;
    if (wt) return overlay(project.id, wt[0], wt[1].branch, wt[1].overrides, wt[1].base);
    return { projectId: project.id, root, branch: project.gh_repo || world.codeTrees[project.id] ? "main" : null, files: { ...treeOf(project.id) }, changes: [], base: project.gh_repo ? "origin/main" : null };
  }
  const r = world.codeRoots[root];
  if (!r) throw new DemoError(400, `${root} is not a registered folder or one of its worktrees`);
  if (!r.exists && !rev) throw new DemoError(400, `${root} does not exist`);
  return overlay(r.projectId, root, r.branch, r.overrides, r.base);
}

function overlay(projectId: string, root: string, branch: string, overrides: Record<string, string | null>, base: string): Source {
  const files = { ...treeOf(projectId) };
  const changes: Change[] = [];
  for (const [path, after] of Object.entries(overrides)) {
    changes.push({ path, before: files[path] ?? null, after });
    if (after === null) delete files[path];
    else files[path] = after;
  }
  return { projectId, root, branch, files, changes, base };
}

/** A DiffFile for one change (`patch` included). */
export function diffFile(c: Change, withPatch = true): DiffFile {
  const d = unifiedDiff(c.before, c.after);
  return {
    filename: c.path, status: c.before === null ? "added" : c.after === null ? "removed" : "modified", additions: d.additions, deletions: d.deletions,
    has_patch: !!d.patch, language: languageOf(c.path)?.[0], ...(withPatch ? { patch: d.patch } : {}),
  };
}

const q = (query: URLSearchParams, key: string) => {
  const v = query.get(key);
  if (v === null) throw new DemoError(400, `missing ${key}`);
  return v;
};

function codeFile(src: Source, path: string): CodeFile {
  const text = src.files[path];
  if (text === undefined) throw new DemoError(404, `${path} does not exist here`);
  const lang = languageOf(path);
  const change = src.changes.find((c) => c.path === path);
  const marks = change && change.after !== null ? lineMarks(change.before, change.after) : [];
  const d = change ? unifiedDiff(change.before, change.after) : null;
  const image = /\.(png|jpe?g|gif|webp|svg|bmp|ico|avif)$/i.test(path);
  return {
    path, size: new TextEncoder().encode(text).length, kind: image ? "image" : "text", language: lang?.[0] ?? null, language_name: lang?.[1] ?? null,
    text: image ? null : text, marks, added: d?.additions ?? 0, removed: d?.deletions ?? 0, letter: change ? (change.before === null ? "A" : "M") : null,
    base: change ? src.base ?? "main" : null, dirty: false,
  };
}

const escapeRegex = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

route("GET", "/api/code/sources", ({ query }): CodeSources => {
  const project = world.projects.find((p) => p.id === q(query, "project"));
  if (!project) throw new DemoError(404, "no such project");
  const git = !!project.gh_repo || !!world.codeTrees[project.id] && project.id !== P.scratch;
  if (!git) return { git: false, default_branch: null, worktrees: [{ path: project.path, branch: null, main: true }], refs: [] };
  const roots = Object.entries(world.codeRoots).filter(([, r]) => r.projectId === project.id);
  return {
    git: true, default_branch: project.gh_repo ? "origin/main" : null,
    worktrees: [{ path: project.path, branch: null, main: true }, ...roots.filter(([, r]) => r.exists).map(([path, r]) => ({ path, branch: r.branch, main: false }))],
    refs: [
      { name: "main", kind: "branch", label: "main", date: Math.floor(clock.ago(60) / 1000) },
      ...roots.map(([, r], i) => ({ name: r.branch, kind: "branch", label: r.branch, date: Math.floor(clock.ago(90 + i * 60) / 1000) })),
      ...(project.gh_repo ? [{ name: "origin/main", kind: "remote", label: "origin/main", date: Math.floor(clock.ago(120) / 1000) }, { name: "v2.4.0", kind: "tag", label: "v2.4.0", date: Math.floor(clock.ago(60 * 30) / 1000) }] : []),
    ],
  };
});

route("GET", "/api/code/files", ({ query }): FileList => {
  const src = openSource(q(query, "root"), query.get("rev"));
  return {
    git: !!src.branch, files: Object.keys(src.files).sort(), base: src.changes.length ? src.base ?? "main" : src.base,
    changes: Object.fromEntries(src.changes.filter((c) => c.after !== null).map((c) => [c.path, c.before === null ? "A" : "M"])),
  };
});

route("GET", "/api/code/file", ({ query }) => codeFile(openSource(q(query, "root"), query.get("rev")), q(query, "path")));

/** Raw bytes for the image preview and Download (the UI uses data URLs from `demo/assets.ts`; this answers direct requests). */
function rawResponse(root: string, rev: string | null, path: string, download: boolean) {
  const text = openSource(root, rev).files[path];
  if (text === undefined) throw new DemoError(404, `${path} does not exist here`);
  const type = path.endsWith(".svg") ? "image/svg+xml" : download ? "application/octet-stream" : "text/plain";
  return new Response(text, { status: 200, headers: { "Content-Type": type } });
}
route("GET", "/api/code/raw", ({ query }) => rawResponse(q(query, "root"), query.get("rev"), q(query, "path"), false));
route("GET", "/api/code/download", ({ query }) => rawResponse(q(query, "root"), query.get("rev"), q(query, "path"), true));

route("GET", "/api/code/search", ({ query }): SearchResult => {
  const src = openSource(q(query, "root"), query.get("rev"));
  const text = q(query, "q");
  if (!text) return { hits: [], truncated: false };
  const flags = query.get("case") === "true" ? "" : "i";
  let pattern = query.get("regex") === "true" ? text : escapeRegex(text);
  if (query.get("word") === "true") pattern = `\\b(?:${pattern})\\b`;
  let re: RegExp;
  try { re = new RegExp(pattern, flags); } catch (e) { throw new DemoError(400, `invalid regular expression: ${(e as Error).message}`); }
  const hits: SearchHit[] = [];
  for (const path of Object.keys(src.files).sort()) {
    src.files[path].split("\n").forEach((line, i) => { if (re.test(line)) hits.push({ path, line: i + 1, text: line.length > 300 ? line.slice(0, 300) : line }); });
  }
  return { hits: hits.slice(0, 1000), truncated: hits.length > 1000 };
});

/** The commit a revision names: a full or abbreviated demo commit ID, a branch or nothing (the folder's HEAD). */
function commitOf(src: Source, rev: string | null) {
  if (rev && /^[0-9a-f]{7,40}$/i.test(rev)) return world.codeCommits.find((c) => c.sha.startsWith(rev.toLowerCase())) ?? null;
  const sha = src.branch?.startsWith("lp/fix-flaky-checkout") ? DEMO_COMMITS.checkoutFix : headOf(src.projectId);
  return world.codeCommits.find((c) => c.sha === sha) ?? null;
}

route("GET", "/api/code/commit", ({ query }): CommitInfo => {
  const root = q(query, "root"), rev = query.get("rev");
  const src = openSource(root, rev);
  const commit = commitOf(src, rev);
  const repo = codeRepoOf(src.projectId);
  return { sha: commit?.sha ?? (src.branch ? headOf(src.projectId) : null), github: repo && src.root === world.projects.find((p) => p.id === src.projectId)?.path ? repo : null };
});

function commitFor(query: URLSearchParams) {
  const src = openSource(q(query, "root"));
  const rev = q(query, "rev");
  const commit = world.codeCommits.find((c) => c.sha.startsWith(rev.toLowerCase()) && c.project === src.projectId) ?? world.codeCommits.find((c) => c.sha.startsWith(rev.toLowerCase()));
  if (!commit) throw new DemoError(400, `${rev} is not a commit of this folder`);
  return commit;
}

route("GET", "/api/code/commit-diff", ({ query }) => {
  const c = commitFor(query);
  return { sha: c.sha, subject: c.subject, parent: c.parent, files: c.changes.map((ch) => diffFile(ch, false)) };
});

route("GET", "/api/code/commit-diff/file", ({ query }) => {
  const c = commitFor(query);
  const ch = c.changes.find((x) => x.path === q(query, "path"));
  if (!ch) throw new DemoError(400, "this file did not change in the commit");
  return diffFile(ch);
});

route("GET", "/api/code/resolve-commits", ({ query }) => {
  const src = openSource(q(query, "root"));
  const oids = q(query, "oids").split(",");
  if (oids.length > 64 || oids.some((o) => !/^[0-9a-f]{7,40}$/i.test(o))) throw new DemoError(400, "expected at most 64 hexadecimal commit IDs of 7–40 characters");
  const out: Record<string, string> = {};
  for (const oid of oids) {
    const found = world.codeCommits.filter((c) => c.project === src.projectId && c.sha.startsWith(oid.toLowerCase()));
    if (found.length === 1) out[oid] = found[0].sha;
  }
  return out;
});

// ---------------------------------------------------------------- task worktrees

/** Recomputes the Git facts of a worktree from its overrides (dirty files for running work, commits otherwise). */
function refreshInfo(i: WorktreeInfo): WorktreeInfo {
  const r = world.codeRoots[i.worktree.root];
  return { ...i, exists: !!r?.exists && i.worktree.state !== "removed" && i.worktree.state !== "discarded" };
}

function publishWorktree(i: WorktreeInfo) {
  publish("worktrees", "updated", i.worktree);
}

const localToast = (what: string) => toast(`Demo: ${what}. No files changed on disk.`);

route("GET", "/api/worktrees", () => world.taskWorktrees.map(refreshInfo));
route("POST", "/api/worktrees/sizes", () => Object.fromEntries(world.taskWorktrees.filter((i) => i.worktree.state === "active").map((i, n) => [i.worktree.id, 48_000_000 + n * 7_300_000])));
route("GET", "/api/sessions/:id/worktrees", ({ params }) => world.taskWorktrees.filter((i) => i.worktree.sessions.includes(params.id)).map(refreshInfo));
route("POST", "/api/worktrees/finish", ({ body }) => {
  const i = world.taskWorktrees.find((w) => w.worktree.sessions.includes(body?.session_id));
  if (!i) throw new DemoError(400, "this session has no task worktree");
  requestMerge(i, true);
  return { message: `Merge of ${i.worktree.branch} into ${i.worktree.base_branch} requested.` };
});
route("POST", "/api/worktrees/add-project", () => ({ message: "Demo: adding a folder during a task is not part of the demo." }));

function requestMerge(i: WorktreeInfo, byAgent: boolean) {
  const w = i.worktree;
  w.merge = { phase: "waiting", reason: "Waiting for the agent to be idle", by_agent: byAgent, agent_retries: 0, requested_at: clock.now() };
  publishWorktree(i);
  setTimeout(() => {
    if (!w.merge) return;
    const head = fakeHead(`${w.id}:merged`);
    w.merge = null;
    w.merged = { base: w.base_commit, into: w.base_branch ?? "main", head, at: clock.now() };
    i.ahead = 0;
    i.dirty_files = 0;
    publishWorktree(i);
    localToast(`merged ${w.branch} into ${w.base_branch ?? "main"}`);
  }, 1500);
}

route("POST", "/api/worktrees/:id/:action", ({ params }) => {
  const i = world.taskWorktrees.find((w) => w.worktree.id === params.id);
  if (!i) throw new DemoError(404, "no such worktree");
  const w = i.worktree, r = world.codeRoots[w.root];
  switch (params.action) {
    case "merge": case "fix": requestMerge(i, false); break;
    case "cancel-merge": w.merge = null; break;
    case "handoff": w.state = "handed_off"; w.handoff_from = "main"; localToast(`checked out ${w.branch} in the folder`); break;
    case "handback": w.state = "active"; w.handoff_from = null; localToast(`checked out main again`); break;
    case "remove": w.state = "removed"; if (r) r.exists = false; localToast("removed the worktree folder; the branch stays"); break;
    case "restore": w.state = "active"; if (r) r.exists = true; localToast("restored the worktree folder"); break;
    case "discard": w.state = "discarded"; if (r) r.exists = false; localToast("discarded the worktree and its branch"); break;
    default: throw new DemoError(404, "unknown worktree action");
  }
  publishWorktree(i);
  return refreshInfo(i);
});
