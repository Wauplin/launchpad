// P2 This item: on a GitHub pull request or issue, what Launchpad has about it (agents that named it, suggestions with
// Insert, a suggested review's line comments and how many are outdated, Local PRs, an agent branch ready to push with
// Push to this PR, Not on GitHub yet, a fork's edit permission) with the same buttons as the page's sidebar box. It is also where a feature lands when its anchor
// on the page stopped matching (the repository state line covers the badge).

import { useCallback, useEffect, useState } from "react";
import type { ExtAgent } from "@/gen/ExtAgent";
import type { ExtBranch } from "@/gen/ExtBranch";
import type { ExtContext } from "@/gen/ExtContext";
import type { ExtNote } from "@/gen/ExtNote";
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

export function Item({ state, tab, page, chatWith }: { state: ConnectionState; tab: ActiveTab; page: Page; chatWith: (bot: string, thread?: string | null) => void }) {
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
      // No thread named: Home sends it to this item's thread of the bot (parallel to its other items).
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

  const files = () => {
    if (!page.repo || !page.number) return;
    void chrome.tabs.update(tab.id, { url: `https://github.com/${page.repo}/pull/${page.number}/files` }).catch((e) => setError(message(e)));
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
          ? <button className="x-btn x-sm x-pri" disabled={!main.bot_id && !bot} title={main.bot ? `Continue this item's thread of ${main.bot}` : undefined} onClick={() => chatWith(main.bot_id ?? bot!.id, main.bot_id ? main.thread : null)}>Continue</button>
          : <button className="x-btn x-sm x-pri" disabled={!bot || !!busy} title={bot ? `Ask ${bot.name}` : "No bot in this workspace"} onClick={() => void ask(isPr ? REVIEW : DRAFT, isPr ? "Review with agent" : "Draft a reply")}>{busy ?? (isPr ? "Review with agent" : "Draft a reply")}</button>}
        {f && <button className="x-btn x-sm x-link" onClick={() => open(`/projects/${encodeURIComponent(f.id)}`)}><Icon name="ext" size={14} /> Folder</button>}
      </div>
      {main?.bot && <p className="x-small x-muted" data-tour="ext.item-thread">{main.bot}'s thread for this item · {statusOf(main).label}{main.last_message ? ` · ${main.last_message}` : ""}</p>}
      {bot && <p className="x-small x-faint">{main ? "Continue chats with its bot here, in this item's thread." : `Goes to ${bot.name} (${bot.source === "folder" ? "the folder's bot" : bot.source === "repository" ? "last used on this repository" : "the workspace's Ask bot"}).`}</p>}
      {note && <p className="x-small x-muted" role="status">{note}</p>}
      {error && <p className="x-error">{error}</p>}
    </div>

    {page.repo && page.number ? <>
      <div className="x-sect">Private timeline</div>
      {ctx.notes.map((n) => <NoteCard key={n.id} n={n} ctx={ctx} open={open} onError={setError} onDone={() => void load(true)} />)}
      <NoteBox repo={page.repo} number={page.number} ws={ws} onDone={() => void load(true)} />
    </> : null}

    {ctx.suggestions.length > 0 && <>
      <div className="x-sect">Waiting for you</div>
      {ctx.suggestions.map((s) => <div key={s.id} className="x-card">
        <h3><span style={{ flex: 1 }}>{s.bot ?? "An agent"} · {s.kind === "review" ? "review" : s.reply_to ? `reply to ${s.reply_to.author ?? "a comment"}` : "comment"}</span>{s.status === "inserted" && <span className="x-status x-s-run">Inserted on GitHub</span>}</h3>
        <p className="x-small" style={{ whiteSpace: "pre-wrap", maxHeight: "9em", overflow: "hidden" }}>{s.text}</p>
        {s.kind === "review" && s.lines.length > 0 && <p className="x-small x-muted">{s.lines.length} line {s.lines.length === 1 ? "comment" : "comments"} on Files changed{s.lines.some((l) => l.state === "posted_by_hand" || l.state === "confirmed") ? ` · ${s.lines.filter((l) => l.state === "posted_by_hand" || l.state === "confirmed").length} added to your review` : ""} · suggests {s.event === "APPROVE" ? "Approve" : s.event === "REQUEST_CHANGES" ? "Request changes" : "Comment"}</p>}
        {s.outdated > 0 && <p className="x-small" style={{ color: "var(--color-attn)" }}>Written at {s.head?.slice(0, 7) ?? "an older commit"}, the pull request is now at {ctx.page.head?.slice(0, 7) ?? "a newer one"}: {s.outdated} line {s.outdated === 1 ? "comment" : "comments"} outdated, inserted only after a second click.</p>}
        <div className="x-btns">
          {s.kind === "comment" && s.status !== "inserted" && <button className="x-btn x-sm" onClick={() => void insert(s)}>Insert</button>}
          {s.kind === "review" && page.kind !== "pr_files" && s.lines.length > 0 && <button className="x-btn x-sm" onClick={files}>Show on Files changed</button>}
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

    {isPr && <Branches ctx={ctx} page={page} onError={setError} onDone={() => void load(true)} />}
    {!agents.length && !ctx.suggestions.length && !ctx.local_prs.length && !ctx.notes.length && <p className="x-muted x-small" style={{ margin: "0 4px" }}>No agent worked on this {isPr ? "pull request" : "issue"} yet.</p>}
  </div>;
}

/** A note of the item's private timeline (an agent's `launchpad note`, or yours): only you see it; Delete asks first. */
function NoteCard({ n, ctx, open, onError, onDone }: { n: ExtNote; ctx: ExtContext; open: (path: string) => void; onError: (e: string | null) => void; onDone: () => void }) {
  const [all, setAll] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const long = n.body.length > 600;
  const remove = async () => {
    onError(null);
    try { await request("note.delete", { id: n.id }); onDone(); } catch (e) { onError(message(e)); } finally { setConfirm(false); }
  };
  const answers = n.reply_to ? ctx.notes.find((x) => x.id === n.reply_to) : undefined;
  const s = n.suggestion;
  const waiting = s ? ctx.suggestions.find((x) => x.id === s.id) : undefined;
  return <div className="x-card">
    <h3><span style={{ flex: 1 }}>{n.author}{answers ? <span className="x-small x-faint"> · answers {answers.by_owner ? "your note" : answers.author}</span> : null}</span><span className="x-small x-faint">{ago(n.created_at)}</span></h3>
    <p className="x-small" style={{ whiteSpace: "pre-wrap", ...(long && !all ? { maxHeight: "12em", overflow: "hidden" } : {}) }}>{n.body}</p>
    {n.head && ctx.page.head && n.head !== ctx.page.head && <p className="x-small x-faint">Written at {n.head.slice(0, 7)}; the pull request is now at {ctx.page.head.slice(0, 7)}.</p>}
    {s && <p className="x-small x-muted">{waiting ? "With a suggested comment: see Waiting for you below, or post it from the page." : s.status === "posted" || s.status === "posted_by_hand" ? "Its suggested comment was posted." : s.status === "discarded" ? "Its suggested comment was discarded." : `Suggested comment: ${s.status.replace(/_/g, " ")}.`}</p>}
    <div className="x-btns">
      {long && <button className="x-btn x-sm" onClick={() => setAll(!all)}>{all ? "Show less" : "Show all"}</button>}
      {n.session_id && <button className="x-btn x-sm x-link" onClick={() => open(`/agents?session=${encodeURIComponent(n.session_id!)}`)}>Agent</button>}
      {confirm
        ? <><button className="x-btn x-sm x-danger" onClick={() => void remove()}>Delete note</button><button className="x-btn x-sm" onClick={() => setConfirm(false)}>Cancel</button></>
        : <button className="x-btn x-sm x-link" onClick={() => setConfirm(true)}>Delete…</button>}
    </div>
  </div>;
}

/** Your private note to the item's agents (`note.write`): stored in Launchpad only, then sent to the agents that wrote
 * about the item (else its default bot), who answer with a note when useful. */
function NoteBox({ repo, number, ws, onDone }: { repo: string; number: number; ws: string; onDone: () => void }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<{ text: string; error: boolean } | null>(null);
  useEffect(() => { setText(""); setStatus(null); }, [repo, number]);
  const add = async (e: React.MouseEvent) => {
    if (!e.nativeEvent.isTrusted || busy || !text.trim()) return;
    setBusy(true); setStatus(null);
    try {
      const r = await request("note.write", { repo, number, body: text, workspace: ws || null });
      const ok = r.sent.filter((x) => !x.error).map((x) => x.recipient.name);
      const bad = r.sent.filter((x) => x.error).map((x) => `${x.recipient.name} (${x.error})`);
      setText("");
      setStatus({ text: r.sent.length ? [ok.length ? `Sent to ${ok.join(", ")}.` : "", bad.length ? `Not sent to ${bad.join(", ")}.` : ""].filter(Boolean).join(" ") : "Saved. No agent to tell about it.", error: !ok.length && !!bad.length });
      onDone();
    } catch (err) { setStatus({ text: message(err), error: true }); } finally { setBusy(false); }
  };
  return <div className="x-card">
    <textarea className="x-field" rows={3} value={text} placeholder="A private note to your agents about this item. Only Launchpad keeps it." onChange={(e) => setText(e.target.value)} />
    <div className="x-btns"><button className="x-btn x-sm x-pri" disabled={busy || !text.trim()} onClick={(e) => void add(e)}>{busy ? "Adding…" : "Add private note"}</button></div>
    {status && <p className={status.error ? "x-error" : "x-small x-muted"} role="status">{status.text}</p>}
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

/** The pull request's branch states: agent branches ready to push (Push to this PR, confirmed here first), Not on
 * GitHub yet with the last error, and whether maintainers can edit a fork's branch. */
function Branches({ ctx, page, onError, onDone }: { ctx: ExtContext; page: Page; onError: (e: string | null) => void; onDone: () => void }) {
  const [confirming, setConfirming] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fork = !!ctx.page.head_repo && !!ctx.page.repo && ctx.page.head_repo.toLowerCase() !== ctx.page.repo.toLowerCase();
  const own = fork && !!ctx.login && ctx.page.head_repo!.split("/")[0]?.toLowerCase() === ctx.login.toLowerCase();
  const canPush = !fork || own || ctx.page.maintainer_can_modify !== false;
  const target = ctx.page.head_repo && ctx.page.head_branch ? `${ctx.page.head_repo}:${ctx.page.head_branch}` : "this pull request's branch";
  const push = async (b: ExtBranch, e: React.MouseEvent) => {
    // Only a real click of the owner pushes.
    if (!e.nativeEvent.isTrusted || busy || !page.repo || !page.number || !b.push_ready) return;
    setBusy(true); onError(null);
    try {
      await request("branch.push-to-pr", { repo: page.repo, number: page.number, branch: b.push_ready.branch, head: b.push_ready.head });
      setConfirming(null);
      onDone();
    } catch (err) { onError(message(err)); } finally { setBusy(false); }
  };
  const ready = ctx.branches.filter((b) => b.push_ready);
  const behind = ctx.branches.filter((b) => !b.push_ready && (b.not_on_github || b.error));
  if (!ready.length && !behind.length && !(fork && ctx.page.maintainer_can_modify !== null)) return null;
  return <>
    <div className="x-sect">Branch</div>
    {ready.map((b) => <div key={b.branch} className="x-card">
      <h3>{b.push_ready!.ahead} {b.push_ready!.ahead === 1 ? "commit" : "commits"} ready to push</h3>
      <p className="x-small x-muted">{b.branch} is {b.push_ready!.ahead} ahead of {target}{b.push_ready!.title ? ` (${b.push_ready!.title})` : ""}.</p>
      {confirming === b.branch && <p className="x-small x-confirm">Home pushes {b.branch} at {b.push_ready!.head.slice(0, 8)} to {target} on GitHub. Fast-forward only, never forced; refused if the pull request moved. Then Home keeps pushing the agent's new commits there.</p>}
      <div className="x-btns">
        {confirming === b.branch
          ? <><button className="x-btn x-sm x-pri" disabled={busy} onClick={(e) => void push(b, e)}>{busy ? "Pushing…" : "Push to this PR"}</button><button className="x-btn x-sm" onClick={() => setConfirming(null)}>Cancel</button></>
          : <button className="x-btn x-sm" disabled={!canPush} title={canPush ? "You confirm first" : "Maintainers cannot edit this pull request's branch"} onClick={() => setConfirming(b.branch)}>Push to this PR…</button>}
      </div>
    </div>)}
    {behind.map((b) => <div key={b.branch} className="x-banner x-warn"><Icon name="warn" size={14} /><span>Not on GitHub yet: {b.branch}{b.error ? `, push refused: ${b.error}` : ""}{b.head ? ` (commit ${b.head.slice(0, 7)})` : ""}</span></div>)}
    {fork && ctx.page.maintainer_can_modify !== null && <p className="x-small x-muted" style={{ margin: "0 4px" }}>{canPush ? "Maintainers can edit this fork's branch." : "Maintainers cannot edit: agents cannot push here. Continue makes a Local PR on top of this branch, or a review with suggestion blocks."}</p>}
  </>;
}
