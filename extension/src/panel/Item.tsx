// P2 This item: on a GitHub pull request or issue, what Launchpad has about it (agents that named it, suggestions with
// Insert, Local PRs) with the same buttons as the page's sidebar box. It is also where a feature lands when its anchor
// on the page stopped matching (the repository state line covers the badge).

import { useCallback, useEffect, useState } from "react";
import type { ExtAgent } from "@/gen/ExtAgent";
import type { ExtContext } from "@/gen/ExtContext";
import type { ExtSuggestion } from "@/gen/ExtSuggestion";
import { ago } from "@/lib/format";
import type { Page } from "../content/github/page";
import { Icon } from "../icons";
import { CONTEXT_TOPIC, request, subscribe, type ConnectionState, type TabCommand } from "../shared/messages";
import type { ActiveTab } from "./tab";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export const ITEM_KINDS = ["pr_conversation", "pr_files", "pr_commits", "issue"] as const;

const REVIEW = "Review this pull request. Put findings in a suggested review (launchpad suggest review with --head) for me to post.";
const DRAFT = "Draft a reply to this issue as a suggested comment (launchpad suggest comment) for me to post.";

export function statusOf(a: { status: string; needs_you: boolean; open_question: boolean }): { label: string; tone: string } {
  if (a.open_question) return { label: "Asks you", tone: "x-s-need" };
  if (["starting", "working", "background"].includes(a.status)) return { label: "Working", tone: "x-s-run" };
  if (a.status === "quota_paused") return { label: "Quota paused", tone: "x-s-need" };
  if (a.status === "error" || a.status === "interrupted") return { label: a.status === "error" ? "Failed" : "Interrupted", tone: "x-s-bad" };
  if (a.status === "waiting") return { label: a.needs_you ? "Done · new" : "Done", tone: a.needs_you ? "x-s-need" : "x-s-done" };
  return { label: a.status.replace(/_/g, " "), tone: "x-s-off" };
}

export function Item({ state, tab, page, chatWith }: { state: ConnectionState; tab: ActiveTab; page: Page; chatWith: (bot: string) => void }) {
  const [ctx, setCtx] = useState<ExtContext | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const load = useCallback(async (fresh = false) => {
    try { setCtx(await request("context", { url: tab.url, fresh })); setError(null); } catch (e) { setError(message(e)); }
  }, [tab.url]);
  useEffect(() => {
    setCtx(null); setNote(null);
    void load();
    const page = new URL(tab.url);
    page.hash = "";
    return subscribe([CONTEXT_TOPIC], {
      event: (e) => {
        const urls = (e.payload as { urls?: string[] | "*" } | null)?.urls;
        if (urls === "*" || (Array.isArray(urls) && urls.includes(page.toString()))) void load();
      },
      resync: () => void load(true),
    });
  }, [load, tab.url, state.workspace]);

  const ws = ctx?.workspace?.id ?? state.workspace ?? "";
  const open = (path: string) => void request("open", { path: `${path}${path.includes("?") ? "&" : "?"}ws=${encodeURIComponent(ws)}` }).catch((e) => setError(message(e)));
  const isPr = page.kind !== "issue";
  const bot = ctx?.defaults.bot ?? null;

  const ask = async (text: string, label: string) => {
    if (!bot || !page.repo || !page.number) return;
    setBusy(label); setError(null);
    try {
      const url = `https://github.com/${page.repo}/${isPr ? "pull" : "issues"}/${page.number}`;
      await request("bot.message", { bot: bot.id, text, github_refs: [{ url, kind: isPr ? "pr" : "issue", repo: page.repo, number: page.number, title: ctx?.page.title ?? null }], key: crypto.randomUUID() });
      setNote(`Sent to ${bot.name}. Its answer shows here and in the chat.`);
      void load(true);
    } catch (e) { setError(message(e)); } finally { setBusy(null); }
  };

  const insert = async (s: ExtSuggestion) => {
    setError(null);
    try {
      const answer = await chrome.tabs.sendMessage<TabCommand, { ok: boolean; error?: string } | undefined>(tab.id, { type: "insert", suggestion: s.id });
      if (!answer?.ok) throw new Error(answer?.error ?? "The page did not answer: reload the GitHub tab and try again.");
      setNote("Inserted into GitHub's comment box: edit it there, then post with GitHub's button.");
    } catch (e) { setError(message(e)); }
  };

  if (error && !ctx) return <div className="x-body"><p className="x-error">{error}</p><div className="x-btns"><button className="x-btn x-sm" onClick={() => void load(true)}>Try again</button></div></div>;
  if (!ctx) return <div className="x-body"><div className="x-line"><span className="x-spin" />Loading…</div></div>;

  const f = ctx.folder;
  const agents = ctx.agents.filter((a) => a.number === page.number || a.number === null);
  const main = agents[0] ?? null;
  return <div className="x-body">
    <div className="x-card">
      <h3>{ctx.page.title ?? `${page.repo}#${page.number}`}</h3>
      <p className="x-small x-muted">{page.repo}#{page.number} · {f ? f.relation === "fork" ? `fork of ${f.repo ?? f.name} in ${f.workspace}` : `In Launchpad · ${f.workspace} / ${f.name}` : `Not in Launchpad · ${ctx.workspace?.name ?? "this workspace"}`}</p>
      <div className="x-btns">
        {main
          ? <button className="x-btn x-sm x-pri" disabled={!main.bot_id && !bot} onClick={() => chatWith(main.bot_id ?? bot!.id)}>Continue</button>
          : <button className="x-btn x-sm x-pri" disabled={!bot || !!busy} title={bot ? `Ask ${bot.name}` : "No bot in this workspace"} onClick={() => void ask(isPr ? REVIEW : DRAFT, isPr ? "Review with agent" : "Draft a reply")}>{busy ?? (isPr ? "Review with agent" : "Draft a reply")}</button>}
        {f && <button className="x-btn x-sm x-link" onClick={() => open(`/projects/${encodeURIComponent(f.id)}`)}><Icon name="ext" size={14} /> Folder</button>}
      </div>
      {bot && <p className="x-small x-faint">{main ? "Continue chats with its bot here." : `Goes to ${bot.name} (${bot.source === "folder" ? "the folder's bot" : bot.source === "repository" ? "last used on this repository" : "the workspace's Ask bot"}).`}</p>}
      {note && <p className="x-small x-muted" role="status">{note}</p>}
      {error && <p className="x-error">{error}</p>}
    </div>

    {ctx.suggestions.length > 0 && <>
      <div className="x-sect">Waiting for you</div>
      {ctx.suggestions.map((s) => <div key={s.id} className="x-card">
        <h3><span style={{ flex: 1 }}>{s.bot ?? "An agent"} · {s.kind === "review" ? "review" : s.reply_to ? `reply to ${s.reply_to.author ?? "a comment"}` : "comment"}</span>{s.status === "inserted" && <span className="x-status x-s-run">Inserted on GitHub</span>}</h3>
        <p className="x-small" style={{ whiteSpace: "pre-wrap", maxHeight: "9em", overflow: "hidden" }}>{s.text}</p>
        {s.outdated > 0 && <p className="x-small" style={{ color: "var(--color-attn)" }}>Written for an older commit: {s.outdated} line {s.outdated === 1 ? "comment" : "comments"} may not apply.</p>}
        <div className="x-btns">
          {s.kind === "comment" && s.status !== "inserted" && <button className="x-btn x-sm" onClick={() => void insert(s)}>Insert</button>}
          {s.kind === "review" && <button className="x-btn x-sm" onClick={() => open(`/github/${page.repo}/${page.number}?tab=files&draft=${encodeURIComponent(s.id)}`)}>Review in Launchpad</button>}
          {s.session_id && <button className="x-btn x-sm x-link" onClick={() => open(`/agents?session=${encodeURIComponent(s.session_id!)}`)}>Agent</button>}
        </div>
      </div>)}
    </>}

    {agents.length > 0 && <>
      <div className="x-sect">Agents on this {isPr ? "pull request" : "issue"}</div>
      {agents.map((a) => <AgentRow key={a.session_id} a={a} open={open} onError={setError} />)}
    </>}

    {ctx.local_prs.length > 0 && <>
      <div className="x-sect">Local PRs</div>
      {ctx.local_prs.map((p) => <button key={p.id} className="x-task" onClick={() => open(`/predrafts/${encodeURIComponent(p.id)}`)}>
        <Icon name="list" size={16} /><span className="x-d"><b>{p.title}</b><small>{p.branch} → {p.base_branch} · {p.state === "open" ? "to publish" : p.state}</small></span>
      </button>)}
    </>}

    {ctx.branches.filter((b) => b.not_on_github).map((b) => <div key={b.branch} className="x-banner x-warn"><Icon name="warn" size={14} /><span>{b.branch}: not on GitHub yet{b.error ? ` (${b.error})` : ""}</span></div>)}
    {!agents.length && !ctx.suggestions.length && !ctx.local_prs.length && <p className="x-muted x-small" style={{ margin: "0 4px" }}>No agent worked on this {isPr ? "pull request" : "issue"} yet.</p>}
  </div>;
}

export function AgentRow({ a, open, onError }: { a: Pick<ExtAgent, "session_id" | "title" | "status" | "needs_you" | "open_question" | "bot" | "since" | "last_message">; open: (path: string) => void; onError: (e: string) => void }) {
  const s = statusOf(a);
  const working = ["starting", "working", "background"].includes(a.status);
  return <div className="x-task" role="group">
    <span className="x-d">
      <b>{a.bot ? `${a.bot} · ` : ""}{a.title}</b>
      <small>{a.last_message ?? `since ${ago(a.since)}`}</small>
    </span>
    <span className={`x-status ${s.tone}`}>{s.label}</span>
    {working && <button className="x-btn x-sm" onClick={() => void request("session.stop", { id: a.session_id }).catch((e) => onError(message(e)))}>Stop</button>}
    <button className="x-ib" aria-label="Open in Launchpad" title="Open in Launchpad" onClick={() => open(`/agents?session=${encodeURIComponent(a.session_id)}`)}><Icon name="ext" size={16} /></button>
  </div>;
}
