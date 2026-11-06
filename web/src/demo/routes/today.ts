// Demo route: Today's decision list, computed in the browser from the demo world with the same rules as Home
// (crates/core/src/today.rs and the `today` modules of the feature crates).

import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Today } from "@/gen/Today";
import type { TodayFilter } from "@/gen/TodayFilter";
import type { TodayItem } from "@/gen/TodayItem";
import type { TodayKind } from "@/gen/TodayKind";
import { needsYou } from "@/lib/sessionStatus";
import { clock, route } from "../daemon";
import { world } from "../world";

type Place = { everywhere: true } | { workspace: string | null } | { project: string } | { session: string } | { repo: string };
type Draft = Omit<TodayItem, "workspaces"> & { place: Place };

const PREFIX: Record<TodayKind, string> = { question: "session", finished: "session", failed: "session", local_pr: "local-pr", proposal: "proposal", outbox: "outbox", suggestion: "suggestion", review_request: "review-request", routine: "routine", runner: "runner" };
const ORDER: TodayFilter[] = ["answer", "review", "approve", "send", "github", "fix"];

function item(kind: TodayKind, filter: TodayFilter, record: string, title: string, place: Place, at: number, more: Partial<Draft> = {}): Draft {
  return { id: `${PREFIX[kind]}:${record}`, kind, filter, title, detail: "", record, session_id: null, action: null, github: null, at, outcome: null, grouped: [], place, ...more };
}
const line = (text: string) => (text.split("\n").map((l) => l.trim()).find(Boolean) ?? "").slice(0, 160);
const specPlace = (spec: LaunchSpec): Place => spec.target.kind === "project" || spec.target.kind === "predraft" ? { project: spec.target.project_id } : { workspace: spec.workspace_id };

/** The workspaces of an item (empty: every workspace); null: not listed (no workspace). */
function resolve(place: Place): string[] | null {
  const projects = world.projects;
  const one = (w: string | null | undefined) => (w ? [w] : null);
  if ("everywhere" in place) return [];
  if ("workspace" in place) return one(place.workspace);
  if ("project" in place) return one(projects.find((p) => p.id === place.project)?.workspace_id);
  if ("session" in place) {
    const s = world.sessions.find((x) => x.id === place.session);
    if (!s) return null;
    const p = projects.find((x) => x.id === s.project_id) ?? projects.find((x) => s.cwd === x.path || s.cwd.startsWith(`${x.path}/`));
    return one(p?.workspace_id ?? s.launch_spec.workspace_id);
  }
  const repo = place.repo.toLowerCase();
  const out = [...new Set(projects.filter((p) => [p.gh_repo, p.gh_upstream].some((r) => r?.toLowerCase() === repo)).map((p) => p.workspace_id).filter((w): w is string => !!w))];
  return out.length ? out : null;
}

function drafts(): Draft[] {
  const out: Draft[] = [];
  for (const s of world.sessions) {
    if (!needsYou(s)) continue;
    const place = { session: s.id };
    const more = { detail: s.last_message ? line(s.last_message) : "", session_id: s.id };
    if (s.open_question) out.push(item("question", "answer", s.id, s.title, place, s.status_since, { ...more, action: "session.answer" }));
    else if (s.status === "waiting") out.push(item("finished", "review", s.id, s.title, place, s.status_since, { ...more, action: "worktree.merge" }));
    else out.push(item("failed", "review", s.id, s.title, place, s.status_since, { ...more, detail: s.status === "error" ? "Failed" : "Interrupted", action: "session.resume" }));
  }
  // One card per session: a Local PR of an agent that needs you goes inside its card.
  for (const p of world.predrafts) {
    if (p.archived || p.published_url || p.merged) continue;
    const pr = item("local_pr", "review", p.id, p.title, { project: p.project_id }, p.created_at, { detail: `Local PR · ${p.branch} → ${p.base_branch}`, session_id: p.session_id, action: "local-pr.publish" });
    const s = p.session_id ? out.find((x) => x.session_id === p.session_id && PREFIX[x.kind] === "session") : undefined;
    if (!s) { out.push(pr); continue; }
    if (s.kind === "finished") s.action = pr.action;
    const { place: _, ...g } = pr;
    s.grouped.push({ ...g, workspaces: [] });
  }
  for (const p of world.proposals) {
    const b = p.body;
    // Without a workspace: the workspace of what it acts on; tools and presets are shared.
    const place: Place = p.workspace_id ? { workspace: p.workspace_id }
      : b.kind === "launch" || b.kind === "schedule" || b.kind === "cron" ? specPlace(b.spec)
      : b.kind === "message" ? { session: b.session_id }
      : b.kind === "status" && b.session_ids.length ? { session: b.session_ids[0] }
      : b.kind === "tool" || b.kind === "preset" ? { everywhere: true }
      : p.session_id ? { session: p.session_id } : { workspace: null };
    const card = (at: number, more: Partial<Draft>) => out.push(item("proposal", "approve", p.id, p.title, place, at, { session_id: p.session_id, ...more }));
    if (p.state === "proposed") card(p.created_at, { detail: p.note ? line(p.note) : "", action: p.body.kind === "ready" ? p.body.action : p.body.kind === "status" ? "proposal.dismiss" : "proposal.start" });
    else if (p.state === "failed") card(p.updated_at, { detail: `Failed: ${line(p.error ?? "")}`, action: "proposal.start" });
    else if (p.state === "uncertain") card(p.updated_at, { detail: "Uncertain: check whether it started before you dismiss it", action: "proposal.dismiss" });
    else if (p.state === "started") card(p.updated_at, { outcome: "Started" });
    else if (p.state === "dismissed") card(p.updated_at, { outcome: "Dismissed" });
  }
  for (const c of world.crons) {
    const last = c.history.at(-1);
    if (c.paused || c.bot_paused || !last || (last.outcome !== "failed" && last.outcome !== "interrupted")) continue;
    out.push(item("routine", "fix", c.id, `Routine “${c.title}” ${last.outcome === "failed" ? "failed" : "was interrupted"}`, specPlace(c.spec), last.at, { detail: last.detail ? line(last.detail) : "", session_id: last.session_id }));
  }
  for (const m of world.outbox) {
    const more = { session_id: m.session_id, detail: `${m.channel} · drafted by ${m.author}` };
    const add = (at: number, extra: Partial<Draft>) => out.push(item("outbox", "send", m.id, m.title?.trim() || line(m.body), m.session_id ? { session: m.session_id } : { everywhere: true }, at, { ...more, ...extra }));
    if (m.status === "draft") add(m.created_at, { action: "outbox.send" });
    else if (m.status === "uncertain" || m.status === "sending") add(m.created_at, { detail: `Uncertain: check ${m.channel} before anything else`, action: "outbox.discard" });
    else if (m.status === "sent") add(m.sent_at ?? m.created_at, { outcome: `Sent to ${m.channel}` });
    else add(m.created_at, { outcome: "Discarded" });
  }
  const reviewed = new Set<string>();
  for (const s of world.suggestions) {
    if (s.target.kind !== "git_hub") continue;
    const { repo, number } = s.target;
    const add = (extra: Partial<Draft>) => out.push(item("suggestion", "github", s.id, `${s.kind === "review" ? "Review" : "Comment"} on ${repo}#${number}`, { repo }, s.created_at, { session_id: s.session_id, github: `${repo}#${number}`, detail: `Suggestion by ${s.author} · ${line(s.body)}`, ...extra }));
    if (s.status === "posted") add({ outcome: `Posted to ${repo}#${number}` });
    else if (s.status === "posting" || s.status === "uncertain") add({ detail: "Uncertain: check GitHub before you discard it", action: "suggestion.discard" });
    else { if (s.kind === "review") reviewed.add(`${repo.toLowerCase()}#${number}`); add({ action: "suggestion.post" }); }
  }
  for (const t of world.inboxThreads) {
    if (t.number === null || t.reason !== "review_requested" || t.kind !== "PullRequest" || t.state === "closed" || t.state === "merged" || reviewed.has(`${t.repo.toLowerCase()}#${t.number}`)) continue;
    out.push(item("review_request", "github", t.id, t.title, { repo: t.repo }, Date.parse(t.updated_at), { detail: `${t.repo}#${t.number} · review requested`, github: `${t.repo}#${t.number}` }));
  }
  if (world.status.role !== "runner") {
    for (const r of world.runners) {
      if (r.compat !== "outdated" && r.compat !== "incompatible") continue;
      out.push(item("runner", "fix", r.runner.id, `Update ${r.runner.name}`, { everywhere: true }, r.runner.last_seen_at ?? 0, { detail: r.compat === "outdated" ? "Runs another build: new sessions wait until you update it" : "Another protocol: it cannot connect until you update it" }));
    }
  }
  return out;
}

route("GET", "/api/today", (): Today => {
  const since = clock.now() - 30 * 86_400_000;
  const items: TodayItem[] = [];
  const done: TodayItem[] = [];
  for (const { place, ...d } of drafts()) {
    const workspaces = resolve(place);
    if (!workspaces) continue;
    const it = { ...d, workspaces, grouped: d.grouped.map((g) => ({ ...g, workspaces })) };
    if (it.outcome !== null) { if (it.at >= since) done.push(it); } else items.push(it);
  }
  items.sort((a, b) => ORDER.indexOf(a.filter) - ORDER.indexOf(b.filter) || b.at - a.at);
  done.sort((a, b) => b.at - a.at);
  return { items, done: done.slice(0, 300) };
});
