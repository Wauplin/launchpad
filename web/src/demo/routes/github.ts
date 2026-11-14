// Demo routes: GitHub links and `#` suggestions, rate limits, Local PRs, suggestions and review state. GitHub is
// never contacted: simulated actions update the world, publish the bus topic the daemon would and say so.

import { toast } from "@/components/Toast";
import type { GhItem } from "@/gen/GhItem";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Note } from "@/gen/Note";
import type { Predraft } from "@/gen/Predraft";
import type { RateLimit } from "@/gen/RateLimit";
import type { Suggestion } from "@/gen/Suggestion";
import { clock, DemoError, githubDemo, publish, route } from "../daemon";
import { appendEvent, session } from "../store";
import { world } from "../world";
import { DEMO_COMMITS } from "../world/code";
import { addLocalPr, fakeSha, localPrHooks, predraftRoot } from "../world/github";
import { ROOT, USER } from "../world/ids";
import { diffFile, openSource } from "./code";

const now = () => new Date(clock.now()).toISOString();

// ---------------------------------------------------------------- GitHub

route("GET", "/api/gh/limits", (): Record<string, RateLimit> => ({ core: { remaining: 4812, reset_at: Math.floor(clock.ago(-38) / 1000), retry_at: null }, graphql: { remaining: 4930, reset_at: Math.floor(clock.ago(-38) / 1000), retry_at: null } }));

route("POST", "/api/gh/resolve", ({ body }): GitHubRef => {
  const m = /^https:\/\/github\.com\/([^/]+\/[^/]+)\/(pull|issues)\/(\d+)/.exec(String(body?.url ?? ""));
  if (!m) throw new DemoError(400, "Not a GitHub issue or pull request link.");
  const item = world.ghItems.find((i) => i.repo === m[1] && i.number === Number(m[3]));
  return { url: m[0], kind: m[2] === "pull" ? "pr" : "issue", repo: m[1], number: Number(m[3]), title: item?.title ?? null };
});

route("GET", "/api/gh/:owner/:repo/suggest", ({ params, query }): GhItem[] => {
  const repo = `${params.owner}/${params.repo}`, text = (query.get("q") ?? "").toLowerCase();
  return world.ghItems
    .filter((i) => i.repo === repo && (!text || i.title.toLowerCase().includes(text) || String(i.number).startsWith(text.replace(/^#/, ""))))
    .sort((a, b) => b.updated_at.localeCompare(a.updated_at));
});

route("POST", "/api/items/:owner/:repo/:number/run-ci", ({ params }) => {
  githubDemo(`approved the waiting workflows of #${params.number}`);
  return { ok: true };
});

// ---------------------------------------------------------------- Local PRs

const predraftOf = (id: string) => {
  const p = world.predrafts.find((x) => x.id === id);
  if (!p) throw new DemoError(400, "no such pre-draft");
  return p;
};
const savePredraft = (p: Predraft) => { publish("predrafts", "updated", p); return p; };

/** `predrafts::detail`: the record, its agent's status and the diff of its branch. */
function detail(p: Predraft) {
  const status = p.session_id ? session(p.session_id)?.status ?? null : null;
  const project = world.projects.find((x) => x.id === p.project_id);
  if (!project) return { predraft: p, status, files: [], commits: [], base_ahead: 0, project_missing: true, project_id: null, head: "", base: "", fallback: false, default_remote_branch: p.default_remote_branch };
  const r = world.codeRoots[predraftRoot(p)];
  const changes = r ? openSource(project.path, r.branch).changes : [];
  const head = p.merged?.head ?? p.pushed_head ?? (p.id === "demo-pr-refunds" ? DEMO_COMMITS.refundsDoc : fakeSha(`${p.id}:head`));
  const subjects = p.id === "demo-pr-refunds" ? ["Document the refund flow"] : p.id === "demo-pr-prices" ? ["Format prices in the visitor's locale"] : [p.title.replace(/ \(#\d+\)$/, "")];
  return {
    predraft: p, status, files: changes.map((c) => diffFile(c)), project_id: p.project_id, head, base: p.initial_base, fallback: false,
    commits: subjects.map((s, i) => `${(i ? fakeSha(`${p.id}${i}`) : head).slice(0, 8)} ${s}`), base_ahead: 0, default_remote_branch: p.default_remote_branch,
  };
}

route("GET", "/api/predrafts", () => world.predrafts);
route("GET", "/api/predrafts/resolve", ({ query }) => {
  const p = world.predrafts.find((x) => predraftRoot(x) === query.get("cwd") && !x.archived);
  if (!p) throw new DemoError(400, "no pre-draft for this folder");
  return p;
});
route("POST", "/api/predrafts", ({ body }) => {
  const cwd: string | undefined = body?.cwd ?? undefined;
  const existing = cwd ? world.predrafts.find((x) => predraftRoot(x) === cwd && !x.archived && !x.published_url && !x.merged) : undefined;
  if (existing) return existing;
  const projectId: string | undefined = body?.project_id ?? world.taskWorktrees.find((w) => w.worktree.root === cwd)?.worktree.project_id ?? world.projects.find((p) => p.path === cwd)?.id;
  const project = world.projects.find((p) => p.id === projectId);
  if (!project) throw new DemoError(400, "This folder is not a Git folder known to the demo.");
  const task = world.taskWorktrees.find((w) => w.worktree.root === cwd);
  const slug = String(body?.title ?? "local-pr").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 40) || "local-pr";
  const root = task ? task.worktree.root : `${ROOT}/.worktrees/${project.id}/${slug}`;
  const p = addLocalPr({
    projectId: project.id, title: body?.title ?? "Local PR", body: body?.body ?? "", sessionId: task?.worktree.sessions[0] ?? null, root,
    branch: task?.worktree.branch ?? `lp/${slug}`, changes: world.codeRoots[root]?.overrides ?? {},
  });
  if (task) { task.worktree.predraft_id = p.id; publish("worktrees", "updated", task.worktree); }
  return p;
});
route("GET", "/api/predrafts/:id", ({ params }) => detail(predraftOf(params.id)));
route("PATCH", "/api/predrafts/:id", ({ params, body }) => {
  const p = predraftOf(params.id);
  if (body?.title != null) p.title = body.title;
  if (body?.body != null) p.body = body.body;
  if (body?.notes != null) p.notes = body.notes as Note[];
  return savePredraft(p);
});
route("POST", "/api/predrafts/:id/:action", ({ params, body }) => {
  const p = predraftOf(params.id);
  const open = !p.archived && !p.published_url && !p.merged;
  switch (params.action) {
    case "refresh": return detail(p);
    case "feedback": {
      if (!p.session_id || !p.notes.length) throw new DemoError(400, "no notes to send");
      const text = `Feedback on the Local PR “${p.title}”:\n\n${p.notes.map((n) => `- ${n.path ? `${n.path}${n.line ? `:${n.line} (${n.side ?? "new"})` : ""}: ` : ""}${n.text}`).join("\n")}`;
      if (localPrHooks.feedback) localPrHooks.feedback(p, text);
      else appendEvent(p.session_id, { kind: "user_message", text, client_id: `demo-feedback-${clock.now()}`, queued: false, images: [] });
      p.notes = [];
      savePredraft(p);
      return { ok: true };
    }
    case "discard":
      if (!open) throw new DemoError(400, "this Local PR is not open");
      p.archived = true;
      toast("Demo: Local PR discarded. No files changed on disk.");
      return savePredraft(p);
    case "unlock":
      if (!open || !p.pushed_head) throw new DemoError(400, "publication has not started");
      p.pushed_head = null;
      return savePredraft(p);
    case "publish": {
      if (!open) throw new DemoError(400, "this Local PR is not open");
      const project = world.projects.find((x) => x.id === p.project_id);
      const repo = project?.gh_repo;
      if (!repo) throw new DemoError(400, "This folder has no GitHub repository.");
      const number = Math.max(0, ...world.ghItems.filter((i) => i.repo === repo).map((i) => i.number)) + 1;
      const head = String(body?.head ?? fakeSha(p.id));
      p.remote_branch = String(body?.remote_branch ?? p.default_remote_branch);
      p.pushed_head = head;
      p.published_number = number;
      p.published_url = `https://github.com/${repo}/pull/${number}`;
      world.ghItems.push({ repo, number, kind: "pr", title: p.title, state: body?.draft ? "draft" : "open", author: USER, url: p.published_url, updated_at: now() });
      githubDemo(`pushed ${p.remote_branch} and opened ${body?.draft ? "a draft " : ""}PR #${number}`);
      return savePredraft(p);
    }
    case "merge-local": {
      if (!open) throw new DemoError(400, "this Local PR is not open");
      p.merged = { head: String(body?.head ?? fakeSha(p.id)), at: clock.now() };
      toast(`Demo: merged ${p.branch} into ${p.base_branch} on Home. Nothing was pushed and GitHub was not contacted.`);
      return savePredraft(p);
    }
    default:
      throw new DemoError(400, "unknown pre-draft action");
  }
});

// ---------------------------------------------------------------- suggestions

const suggestionOf = (id: string) => {
  const s = world.suggestions.find((x) => x.id === id);
  if (!s) throw new DemoError(400, "suggestion not found");
  return s;
};
const saveSuggestion = (s: Suggestion) => { publish("suggestions", "updated", s); return s; };

route("GET", "/api/suggestions", () => world.suggestions);
route("POST", "/api/suggestions", ({ body }) => {
  const s: Suggestion = {
    id: `demo-sg-${clock.now().toString(36)}`, target: body.target, kind: body.kind, body: String(body.body ?? ""), event: body.event ?? "COMMENT", head: body.head ?? null,
    author: "Agent", session_id: body.session_id ?? null, created_at: clock.now(), status: "draft", url: null,
    merge: body.merge ?? null, run_ci: !!body.run_ci, followup_error: null, comments: Array.isArray(body.comments) ? body.comments : [],
  };
  world.suggestions = [...world.suggestions, s];
  return saveSuggestion(s);
});
route("PATCH", "/api/suggestions/:id", ({ params, body }) => {
  const s = suggestionOf(params.id);
  if (s.status !== "draft") throw new DemoError(400, "only unposted drafts can be edited");
  if (!String(body?.body ?? "").trim()) throw new DemoError(400, "suggestion must contain 1–65536 bytes of text");
  s.body = body.body;
  if (Array.isArray(body.comments)) s.comments = body.comments;
  s.event = body.event ?? s.event;
  s.merge = s.event === "APPROVE" ? body.merge ?? null : null;
  s.run_ci = !!body.run_ci && !s.merge;
  return saveSuggestion(s);
});
route("DELETE", "/api/suggestions/:id", ({ params }) => {
  const s = suggestionOf(params.id);
  if (s.status === "posted") throw new DemoError(400, "suggestion is already posted");
  world.suggestions = world.suggestions.filter((x) => x.id !== s.id);
  publish("suggestions", "discarded", { id: s.id });
  return { ok: true };
});
route("POST", "/api/suggestions/:id/post", ({ params }) => {
  const s = suggestionOf(params.id);
  if (s.status === "posted") throw new DemoError(400, "publication already attempted; check GitHub before discarding or creating another suggestion");
  if (s.target.kind === "predraft") {
    const p = predraftOf(s.target.id);
    p.notes = [...p.notes, { path: null, text: s.body }, ...s.comments.map((c) => ({ path: c.path, line: c.line, side: c.side === "LEFT" ? "old" : "new", text: c.body }))];
    savePredraft(p);
  } else {
    const t = s.target;
    s.url = `https://github.com/${t.repo}/${s.kind === "review" ? "pull" : "issues"}/${t.number}`;
    githubDemo(`posted the agent's ${s.kind} on #${t.number}`);
    if (s.kind === "review" && s.merge) githubDemo(`merged #${t.number} (${s.merge})`);
    else if (s.kind === "review" && s.run_ci) githubDemo(`approved the waiting workflows of #${t.number}`);
  }
  s.status = "posted";
  return saveSuggestion(s);
});

// ---------------------------------------------------------------- review state

route("GET", "/api/review-state", ({ query }) => {
  const key = query.get("key") ?? "";
  const e = world.reviewState[key];
  return e ? { key, value: e.value, updated_at: e.updated_at } : { key, value: null, updated_at: null };
});
route("PUT", "/api/review-state", ({ query, body }) => {
  const key = query.get("key") ?? "";
  const e = world.reviewState[key];
  if (e && body?.base_revision && body.base_revision !== e.updated_at) throw new DemoError(409, "Review draft changed elsewhere; choose which version to keep");
  const updated_at = Math.max(clock.now(), (e?.updated_at ?? 0) + 1);
  if (body?.value === null || body?.value === undefined) delete world.reviewState[key];
  else world.reviewState[key] = { value: body.value, updated_at };
  publish("review-state", "changed", { key, updated_at, origin: body?.origin ?? null });
  return { ok: true, updated_at };
});
