// Demo routes: inbox, issues and PRs, repository metadata, Local PRs, suggestions and review state. GitHub is
// never contacted: simulated actions update the world, publish the bus topic the daemon would and say so.

import { toast } from "@/components/Toast";
import type { Comment, Item, ListItem } from "@/features/github/types";
import type { GhItem } from "@/gen/GhItem";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { InboxSnapshot } from "@/gen/InboxSnapshot";
import type { Note } from "@/gen/Note";
import type { Predraft } from "@/gen/Predraft";
import type { Suggestion } from "@/gen/Suggestion";
import type { ThreadPreview } from "@/gen/ThreadPreview";
import { clock, DemoError, githubDemo, publish, route } from "../daemon";
import { appendEvent, session } from "../store";
import { world } from "../world";
import { DEMO_COMMITS } from "../world/code";
import { addLocalPr, buildItem, fakeSha, localPrHooks, mdToHtml, prDiff, repoMeta, syncThreads } from "../world/github";
import { ROOT, USER } from "../world/ids";
import { diffFile, openSource } from "./code";

const now = () => new Date(clock.now()).toISOString();
const me = { login: USER };

// ---------------------------------------------------------------- inbox

route("GET", "/api/inbox", (): InboxSnapshot => ({
  active_ids: world.inboxThreads.map((t) => t.id),
  threads: world.inboxThreads,
  status: { syncing: false, last_sync: clock.ago(1), error: null, poll_interval: 60 },
  limits: { core: { remaining: 4812, reset_at: Math.floor(clock.ago(-38) / 1000), retry_at: null }, graphql: { remaining: 4930, reset_at: Math.floor(clock.ago(-38) / 1000), retry_at: null } },
}));

route("POST", "/api/inbox/sync", () => { publish("inbox", "updated", {}); return { ok: true }; });

route("POST", "/api/inbox/restore", ({ body }) => {
  const ids: string[] = body?.ids ?? [];
  const back = world.doneThreads.filter((t) => ids.includes(t.id));
  world.doneThreads = world.doneThreads.filter((t) => !ids.includes(t.id));
  world.inboxThreads = [...world.inboxThreads, ...back].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  publish("inbox", "updated", {});
  return { ok: true };
});

const threadOf = (id: string) => {
  const t = world.inboxThreads.find((x) => x.id === id) ?? world.doneThreads.find((x) => x.id === id);
  if (!t) throw new DemoError(400, "no such thread");
  return t;
};

route("GET", "/api/inbox/:id/preview", ({ params }): ThreadPreview | null => {
  const t = threadOf(params.id);
  const item = t.number !== null ? world.ghItems[`${t.repo}#${t.number}`] : undefined;
  if (!item) {
    const body = "## What's new\n\n- Saved carts behind a feature flag\n- Faster product pages\n\n## Fixes\n\n- Checkout no longer double-submits on slow networks";
    return { id: t.id, body, body_html: mdToHtml(body), author: "dev-alex", comments: [], fetched_at: clock.ago(2) };
  }
  const comments: Comment[] = item.timelineItems.nodes.filter((e) => e.__typename === "IssueComment" || (e.__typename === "PullRequestReview" && e.body)) as Comment[];
  return {
    id: t.id, body: item.body, body_html: item.bodyHTML, author: item.author.login ?? "", fetched_at: clock.ago(1),
    comments: comments.map((c) => ({ author: c.author?.login ?? "", body: c.body, body_html: c.bodyHTML, url: c.url, created_at: c.createdAt ?? c.submittedAt ?? now() })),
  };
});
route("POST", "/api/inbox/:id/preview", () => ({ ok: true }));

route("POST", "/api/inbox/:id/action", ({ params, body }) => {
  const t = threadOf(params.id);
  const action = String(body?.action);
  if (!["read", "unread", "done", "unsubscribe"].includes(action)) throw new DemoError(400, "unknown inbox action");
  if (action === "done" || action === "unsubscribe") {
    world.inboxThreads = world.inboxThreads.filter((x) => x.id !== t.id);
    if (action === "done") world.doneThreads.push(t);
    githubDemo(action === "done" ? `marked “${t.title}” as done` : `unsubscribed from “${t.title}”`);
  } else {
    t.unread = action === "unread";
    t.local_unread = action === "unread";
  }
  publish("inbox", "updated", {});
  return { ok: true };
});

route("POST", "/api/inbox/:id/reply", ({ params, body }) => {
  const t = threadOf(params.id);
  const item = t.number !== null ? world.ghItems[`${t.repo}#${t.number}`] : undefined;
  if (!item) throw new DemoError(400, "This notification has no conversation to reply to.");
  comment(item, String(body?.text ?? ""));
  githubDemo(`replied on #${item.number}`);
  return { ok: true };
});

// ---------------------------------------------------------------- GitHub links and suggestions in the composer

const kindOf = (item: Item) => (item.__typename === "PullRequest" ? "pr" : "issue");

route("POST", "/api/gh/resolve", ({ body }): GitHubRef => {
  const m = /^https:\/\/github\.com\/([^/]+\/[^/]+)\/(pull|issues)\/(\d+)/.exec(String(body?.url ?? ""));
  if (!m) throw new DemoError(400, "Not a GitHub issue or pull request link.");
  const item = world.ghItems[`${m[1]}#${m[3]}`];
  return { url: m[0], kind: m[2] === "pull" ? "pr" : "issue", repo: m[1], number: Number(m[3]), title: item?.title ?? null };
});

route("GET", "/api/gh/:owner/:repo/suggest", ({ params, query }): GhItem[] => {
  const repo = `${params.owner}/${params.repo}`, text = (query.get("q") ?? "").toLowerCase();
  return Object.values(world.ghItems)
    .filter((i) => i.repo === repo && (!text || i.title.toLowerCase().includes(text) || String(i.number).startsWith(text.replace(/^#/, ""))))
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    .map((i) => ({ repo, number: i.number, title: i.title, kind: kindOf(i), state: i.merged ? "merged" : i.isDraft ? "draft" : i.state.toLowerCase(), author: i.author.login ?? "", url: i.url, updated_at: i.updatedAt }));
});

// ---------------------------------------------------------------- items

const itemOf = (owner: string, repo: string, n: string) => {
  const item = world.ghItems[`${owner}/${repo}#${n}`];
  if (!item) throw new DemoError(404, `${owner}/${repo}#${n} is not part of the demo.`);
  return item;
};
const fresh = (item: Item): Item => ({ ...item, fetched_at: clock.now() });
const touch = (item: Item) => { item.updatedAt = now(); syncThreads(item); };

function comment(item: Item, text: string) {
  if (!text.trim()) throw new DemoError(400, "empty comment");
  const n = item.timelineItems.nodes.length;
  item.timelineItems.nodes.push({ __typename: "IssueComment", id: `${item.id}_new${n}`, body: text, bodyHTML: mdToHtml(text), url: `${item.url}#issuecomment-new${n}`, createdAt: now(), author: me });
  if (!item.participants.nodes.some((p) => p.login === USER)) item.participants.nodes.push(me);
  touch(item);
}

route("GET", "/api/items/:owner/:repo/:number/diff/file", ({ params, query }) => {
  const item = itemOf(params.owner, params.repo, params.number);
  const c = prDiff(item.repo, item.number).find((x) => x.path === query.get("path"));
  if (!c) throw new DemoError(400, "this file is not part of the PR diff");
  return diffFile(c);
});
route("GET", "/api/items/:owner/:repo/:number", ({ params }) => fresh(itemOf(params.owner, params.repo, params.number)));
route("POST", "/api/items/:owner/:repo/:number", ({ params }) => fresh(itemOf(params.owner, params.repo, params.number)));

const REVIEW: Record<string, string> = { COMMENT: "COMMENTED", APPROVE: "APPROVED", REQUEST_CHANGES: "CHANGES_REQUESTED" };

route("POST", "/api/items/:owner/:repo/:number/:action", ({ params, body }) => {
  const item = itemOf(params.owner, params.repo, params.number);
  const n = item.timelineItems.nodes.length;
  const event = (typename: string, extra: object = {}) => item.timelineItems.nodes.push({ __typename: typename, id: `${item.id}_e${n}`, createdAt: now(), actor: me, ...extra });
  switch (params.action) {
    case "diff": {
      const project = world.projects.find((p) => p.gh_repo === item.repo);
      return { files: prDiff(item.repo, item.number).map((c) => diffFile(c)), project_id: project?.id ?? null, head: item.headRefOid ?? "", base: item.baseRefOid ?? "", fallback: false };
    }
    case "comment":
      comment(item, String(body?.body ?? ""));
      githubDemo(`commented on #${item.number}`);
      break;
    case "review": {
      const state = REVIEW[body?.event];
      if (!state) throw new DemoError(400, "invalid review");
      postReview(item, state, String(body?.body ?? ""), body?.comments ?? []);
      githubDemo(`posted a review on #${item.number}`);
      break;
    }
    case "reviewers": {
      const names: string[] = [...(body?.reviewers ?? []), ...(body?.team_reviewers ?? []).map((t: string) => `team:${t}`)];
      for (const r of names) {
        const who = r.startsWith("team:") ? { name: r.slice(5), slug: r.slice(5) } : { login: r };
        item.reviewRequests = { nodes: [...(item.reviewRequests?.nodes ?? []), { requestedReviewer: who }] };
        event("ReviewRequestedEvent", { requestedReviewer: who });
      }
      touch(item);
      githubDemo(`requested reviews on #${item.number}`);
      break;
    }
    case "labels": {
      const names: string[] = body?.labels ?? [];
      const colors = new Map((repoMeta[item.repo]?.labels ?? []).map((l) => [l.name, l.color]));
      const before = item.labels.nodes.map((l) => l.name);
      for (const name of names.filter((x) => !before.includes(x))) event("LabeledEvent", { label: { name } });
      for (const name of before.filter((x) => !names.includes(x))) event("UnlabeledEvent", { label: { name } });
      item.labels = { nodes: names.map((name) => ({ name, color: colors.get(name) ?? item.labels.nodes.find((l) => l.name === name)?.color ?? "ededed" })) };
      touch(item);
      githubDemo(`saved the labels of #${item.number}`);
      break;
    }
    case "state": {
      const open = body?.state === "open";
      item.state = open ? "OPEN" : "CLOSED";
      event(open ? "ReopenedEvent" : "ClosedEvent");
      touch(item);
      githubDemo(`${open ? "reopened" : "closed"} #${item.number}`);
      break;
    }
    case "merge": {
      if (item.__typename !== "PullRequest" || item.isDraft || item.mergeable !== "MERGEABLE") throw new DemoError(400, "Pull Request is not mergeable");
      item.merged = true;
      item.state = "MERGED";
      event("MergedEvent");
      touch(item);
      githubDemo(`merged #${item.number} (${body?.merge_method ?? "squash"})`);
      break;
    }
    case "draft": case "ready":
      item.isDraft = params.action === "draft";
      event(item.isDraft ? "ConvertToDraftEvent" : "ReadyForReviewEvent");
      touch(item);
      githubDemo(item.isDraft ? `converted #${item.number} to a draft` : `marked #${item.number} ready for review`);
      break;
    default:
      throw new DemoError(400, "unknown action");
  }
  return { ok: true };
});

function postReview(item: Item, state: string, text: string, lineComments: { path: string; line: number; body: string }[]) {
  const n = item.timelineItems.nodes.length;
  item.timelineItems.nodes.push({ __typename: "PullRequestReview", id: `${item.id}_rv${n}`, body: text, bodyHTML: mdToHtml(text), url: `${item.url}#pullrequestreview-new${n}`, submittedAt: now(), createdAt: now(), author: me, state });
  item.reviews = { nodes: [...(item.reviews?.nodes ?? []), { state, author: me }] };
  item.reviewRequests = { nodes: (item.reviewRequests?.nodes ?? []).filter((r) => r.requestedReviewer.login !== USER) };
  if (state === "APPROVED") item.reviewDecision = "APPROVED";
  if (state === "CHANGES_REQUESTED") item.reviewDecision = "CHANGES_REQUESTED";
  const threads = item.reviewThreads?.nodes ?? [];
  lineComments.forEach((c, i) => threads.push({ id: `${item.id}_nt${n}_${i}`, path: c.path, line: c.line, isResolved: false, isOutdated: false, comments: { nodes: [{ id: `${item.id}_ntc${n}_${i}`, body: c.body, bodyHTML: mdToHtml(c.body), url: item.url, createdAt: now(), author: me }] } }));
  item.reviewThreads = { nodes: threads };
  if (!item.participants.nodes.some((p) => p.login === USER)) item.participants.nodes.push(me);
  touch(item);
}

/** The cached open items of a repository (the 30 most recently updated, like the daemon). */
function repoItems(repo: string) {
  const all = Object.values(world.ghItems).filter((i) => i.repo === repo && i.state === "OPEN").sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  const list = (i: Item): ListItem => ({ labels: i.labels, number: i.number, title: i.title, url: i.url, state: i.state, isDraft: i.isDraft, updatedAt: i.updatedAt, author: i.author });
  return { fetched_at: clock.now(), prs: all.filter((i) => i.__typename === "PullRequest").slice(0, 30).map(list), issues: all.filter((i) => i.__typename === "Issue").slice(0, 30).map(list) };
}
route("GET", "/api/repo-items/:owner/:repo", ({ params }) => repoItems(`${params.owner}/${params.repo}`));
route("POST", "/api/repo-items/:owner/:repo", ({ params }) => repoItems(`${params.owner}/${params.repo}`));
route("GET", "/api/repo-meta/:owner/:repo", ({ params }) => repoMeta[`${params.owner}/${params.repo}`] ?? { labels: [], reviewers: [] });

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
  if (!project) return { predraft: p, status, files: [], commits: [], base_ahead: 0, dirty: false, project_missing: true, project_id: null, head: "", base: "", fallback: false, default_remote_branch: p.default_remote_branch };
  const r = world.codeRoots[p.root];
  const changes = r ? openSource(project.path, r.branch).changes : [];
  const head = p.merged?.head ?? p.pushed_head ?? (p.id === "demo-pr-refunds" ? DEMO_COMMITS.refundsDoc : fakeSha(`${p.id}:head`));
  const subjects = p.id === "demo-pr-refunds" ? ["Document the refund flow"] : p.id === "demo-pr-prices" ? ["Format prices in the visitor's locale"] : [p.title.replace(/ \(#\d+\)$/, "")];
  return {
    predraft: p, status, files: changes.map((c) => diffFile(c)), project_id: p.project_id, head, base: p.initial_base, fallback: false,
    commits: subjects.map((s, i) => `${(i ? fakeSha(`${p.id}${i}`) : head).slice(0, 8)} ${s}`), base_ahead: 0, dirty: false, default_remote_branch: p.default_remote_branch,
  };
}

route("GET", "/api/predrafts", () => world.predrafts);
route("GET", "/api/predrafts/resolve", ({ query }) => {
  const p = world.predrafts.find((x) => x.root === query.get("cwd") && !x.archived);
  if (!p) throw new DemoError(400, "no pre-draft for this folder");
  return p;
});
route("POST", "/api/predrafts", ({ body }) => {
  const cwd: string | undefined = body?.cwd ?? undefined;
  const existing = cwd ? world.predrafts.find((x) => x.root === cwd && !x.archived && !x.published_url && !x.merged) : undefined;
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
  if (task) { task.worktree.state = "predraft"; task.worktree.predraft_id = p.id; publish("worktrees", "updated", task.worktree); }
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
      const text = `Feedback on the Local PR “${p.title}”:\n\n${p.notes.map((n) => `- ${n.path ? `${n.path}: ` : ""}${n.text}`).join("\n")}`;
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
    case "publish": {
      if (!open) throw new DemoError(400, "this Local PR is not open");
      const project = world.projects.find((x) => x.id === p.project_id);
      const repo = project?.gh_repo;
      if (!repo) throw new DemoError(400, "This folder has no GitHub repository.");
      const number = Math.max(0, ...Object.values(world.ghItems).filter((i) => i.repo === repo).map((i) => i.number)) + 1;
      const head = String(body?.head ?? fakeSha(p.id));
      p.remote_branch = String(body?.remote_branch ?? p.default_remote_branch);
      p.pushed_head = head;
      p.published_number = number;
      p.published_url = `https://github.com/${repo}/pull/${number}`;
      world.ghItems[`${repo}#${number}`] = buildItem({ repo, number, pr: true, draft: !!body?.draft, title: p.title, author: USER, body: p.body, labels: [], minutesAgo: 0, updatedAgo: 0, head, branch: p.remote_branch, checks: [{ name: "test", conclusion: null, status: "IN_PROGRESS" }], mergeState: "BLOCKED" });
      world.prDiffs[`${repo}#${number}`] = openSource(project!.path, world.codeRoots[p.root]?.branch).changes;
      githubDemo(`pushed ${p.remote_branch} and opened ${body?.draft ? "a draft " : ""}PR #${number}`);
      return savePredraft(p);
    }
    case "merge-local": {
      if (!open) throw new DemoError(400, "this Local PR is not open");
      const project = world.projects.find((x) => x.id === p.project_id);
      p.merged = { checkout: project?.path ?? p.root, head: String(body?.head ?? fakeSha(p.id)), at: clock.now() };
      toast(`Demo: merged ${p.branch} into the local ${p.base_branch} branch. Nothing was pushed and GitHub was not contacted.`);
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
  };
  world.suggestions = [...world.suggestions, s];
  return saveSuggestion(s);
});
route("PATCH", "/api/suggestions/:id", ({ params, body }) => {
  const s = suggestionOf(params.id);
  if (s.status !== "draft") throw new DemoError(400, "only unposted drafts can be edited");
  if (!String(body?.body ?? "").trim()) throw new DemoError(400, "suggestion must contain 1–65536 bytes of text");
  s.body = body.body;
  s.event = body.event ?? s.event;
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
    p.notes = [...p.notes, { path: null, text: s.body }];
    savePredraft(p);
  } else {
    const item = world.ghItems[`${s.target.repo}#${s.target.number}`];
    if (!item) throw new DemoError(400, "This item is not part of the demo.");
    if (s.kind === "review") postReview(item, REVIEW[s.event] ?? "COMMENTED", s.body, []);
    else comment(item, s.body);
    s.url = item.url;
    githubDemo(`posted the agent's ${s.kind} on #${item.number}`);
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
route("POST", "/api/review-state/import", ({ body }) => {
  const entries = Object.entries((body ?? {}) as Record<string, unknown>);
  for (const [key, value] of entries) if (!world.reviewState[key]) world.reviewState[key] = { value, updated_at: clock.now() };
  publish("review-state", "changed", { key: null, updated_at: clock.now(), origin: null });
  return { imported: entries.length };
});
