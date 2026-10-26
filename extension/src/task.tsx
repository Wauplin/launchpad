// One task: the conversation with a compact step list, owner review cards and the reply box.

import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Markdown } from "@/components/Markdown";
import { toastError } from "@/components/Toast";
import { buildRows, type Call, type Row } from "@/features/agents/rows";
import { splitPrompt } from "@/features/agents/traceRows";
import { useMarkSeen, useWindowFocused } from "@/features/agents/useSeen";
import type { FileAttachment } from "@/gen/FileAttachment";
import type { Session } from "@/gen/Session";
import { api } from "@/lib/api";
import { attachmentSrc } from "@/lib/attachments";
import { useSession } from "@/lib/queries";
import { emptyLog, useSessionLogs } from "@/lib/sessionStore";
import { isRunning, needsYou } from "@/lib/sessionStatus";
import { sendPresence } from "@/lib/ws";
import { Box, chipOf, useWindowTabs } from "./compose";
import { Favicon, Icon } from "./icons";
import { grantTabs, hostOf, live, rememberSites, requestSites, siteOf, stepLabel, useGrants, useReviews, useSites, type Grant, type Review, type TabChip } from "./lp";

export type Tone = "run" | "need" | "done" | "off" | "bad";
/** The one status word of a task, with the review cards of the panel counted as Needs you. */
export function taskStatus(s: Session, reviews: Review[] = []): { label: string; tone: Tone } {
  if (s.open_question || reviews.some((r) => r.session === s.id)) return { label: "Needs you", tone: "need" };
  if (isRunning(s)) return { label: "Working", tone: "run" };
  switch (s.status) {
    case "quota_paused": return { label: "Paused", tone: "off" };
    case "waiting": return { label: "Done", tone: needsYou(s) ? "need" : "done" };
    case "stopped": return { label: "Stopped", tone: "off" };
    case "interrupted": return { label: "Interrupted", tone: "bad" };
    case "error": return { label: "Failed", tone: "bad" };
    case "closed": return { label: "Archived", tone: "off" };
    case "in_terminal": return { label: "In terminal", tone: "off" };
  }
  return { label: s.status, tone: "off" };
}
export const StatusPill = ({ s, reviews }: { s: Session; reviews?: Review[] }) => {
  const st = taskStatus(s, reviews);
  return <span className={`x-status x-s-${st.tone}`}>{st.label}</span>;
};

export function TaskView({ id }: { id: string }) {
  const s = useSession(id);
  const client = useQueryClient();
  const log = useSessionLogs((st) => st.logs[id]) ?? emptyLog();
  const load = useSessionLogs((st) => st.load);
  const grantsQuery = useGrants();
  const allGrants = grantsQuery.data ?? [];
  const grants = allGrants.filter((g) => g.session === id && live(g));
  const reviews = (useReviews().data ?? []).filter((r) => r.session === id);
  const focused = useWindowFocused();
  useMarkSeen(s, focused);
  useEffect(() => { void load(id); }, [id, load]);
  useEffect(() => { sendPresence(focused, [id]); return () => sendPresence(false, []); }, [focused, id]);
  const typed = s?.launch_spec.text ?? "";
  const rows = useMemo(() => {
    const all = buildRows(log);
    // The launch prompt shows only what the owner typed, without the appended Launchpad instructions.
    const first = log.earlier ? -1 : all.findIndex((r) => r.t === "user");
    const row = all[first];
    const split = row?.t === "user" ? splitPrompt(row.text, typed) : null;
    if (row?.t === "user" && split) all[first] = { ...row, text: split.typed };
    return all;
  }, [log, typed]);
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [rows, reviews.length, grants.length]);
  if (!s) return <div className="x-body"><p className="x-faint">This task no longer exists.</p></div>;
  const running = isRunning(s);
  const control = async (g: Grant, action: string) => {
    try {
      await api.post(`/api/browser/grants/${g.id}/control`, { action });
      await chrome.runtime.sendMessage({ type: "release", tab: g.page.tab });
      await client.invalidateQueries({ queryKey: ["browser-grants"] });
    } catch (e) { toastError(e); }
  };
  const working = grants.filter((g) => g.work);
  return <>
    <div className="x-body" ref={scroller} onScroll={(e) => { const el = e.currentTarget; pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40; }}>
      {log.earlier && <button className="x-btn x-link x-small" onClick={() => void useSessionLogs.getState().loadEarlier(id)}>Show earlier messages</button>}
      {rows.map((r, i) => <RowView key={r.key} row={r} last={i === rows.length - 1} running={running} grants={allGrants} sessionId={id} />)}
      {running && !rows.some((r, i) => i === rows.length - 1 && r.t === "tools" && r.calls.some((c) => !c.result)) && <div className="x-line"><span className="x-spin" />Working…</div>}
      {s.quota_pause && <div className="x-banner x-warn"><Icon name="warn" size={15} /><span>{s.quota_pause.message}</span></div>}
      {grants.filter((g) => g.uncertain).map((g) => <div key={g.id} className="x-card x-alert" role="alert">
        <h3><Icon name="warn" size={16} className="x-bad" />Did the last action on “{g.page.title}” go through?</h3>
        <p className="x-small">The connection dropped during an action. The agent will not repeat it by itself. Check the page (or your email) for a confirmation, then continue and tell the agent what you saw.</p>
        <div className="x-btns"><button className="x-btn x-wide" onClick={() => void chrome.tabs.update(g.page.tab, { active: true }).catch(toastError)}>Show the tab</button><button className="x-btn x-pri x-wide" onClick={() => void control(g, "continue")}>I checked — continue</button></div>
      </div>)}
      {reviews.map((r) => <ReviewCard key={r.id} review={r} />)}
    </div>
    {(grants.length > 0 || working.length > 0) && <div style={{ padding: "0 12px" }}>
      <SharedTabs grants={grants} running={running} control={control} />
    </div>}
    <div className="x-foot"><Reply s={s} grants={grants} /></div>
  </>;
}

function SharedTabs({ grants, running, control }: { grants: Grant[]; running: boolean; control: (g: Grant, action: string) => Promise<void> }) {
  const client = useQueryClient();
  const stopSharing = (g: Grant) => void api.delete(`/api/browser/grants/${g.id}`).then(() => client.invalidateQueries({ queryKey: ["browser-grants"] })).catch(toastError);
  const paused = grants.filter((g) => g.work && g.paused);
  const acting = grants.filter((g) => g.work && !g.paused);
  return <div className="x-banner x-run" style={{ flexWrap: "wrap" }}>
    <Icon name={acting.length ? "hand" : "eye"} size={15} />
    <span className="x-shared">{grants.map((g) => <span key={g.id} className="x-chip" title={g.page.url}><Favicon url={g.page.url} title={g.page.title} /><span style={{ maxWidth: 150 }}>{g.page.title}</span><button aria-label={`Stop sharing ${g.page.title}`} title="Stop sharing" onClick={() => stopSharing(g)}><Icon name="close" size={12} /></button></span>)}</span>
    {paused.length > 0 ? <button className="x-btn x-sm" title="Give control back to the agent" onClick={() => void Promise.all(paused.map((g) => control(g, "continue")))}>Resume</button>
      : acting.length > 0 && running && <button className="x-btn x-sm" title="Pause the agent's control so you can use the tab" onClick={() => void Promise.all(acting.map((g) => control(g, "take-over")))}>Take control</button>}
  </div>;
}

const RowView = memo(function RowView({ row, last, running, grants, sessionId }: { row: Row; last: boolean; running: boolean; grants: Grant[]; sessionId: string }) {
  switch (row.t) {
    case "user": return <div className="x-user">{row.text}{row.images.map((img, i) => <img key={i} src={attachmentSrc(img.data_url)} alt={img.name} />)}{row.queued && <div className="x-small x-faint">Sent at the next step</div>}</div>;
    case "assistant": return <div className="x-ai"><Markdown text={row.text} /></div>;
    case "tools": return <Steps calls={row.calls} open={last && running} grants={grants} />;
    case "edit": return <div className="x-line"><Icon name="edit" size={14} />{row.change === "add" ? "Created" : row.change === "delete" ? "Deleted" : "Edited"} {row.path.split("/").pop()}</div>;
    case "ask": return row.answers === null ? <AskCard row={row} sessionId={sessionId} /> : null;
    case "error": return <div className="x-error">{row.text}</div>;
    case "turn": return row.error ? <div className="x-error">{row.error}</div> : row.interrupted ? <div className="x-line">Stopped</div> : null;
    case "notice": return /^(Resumed|Stopped)/.test(row.text) ? null : <div className="x-line">{row.text}</div>;
    default: return null;
  }
});

function Steps({ calls, open, grants }: { calls: Call[]; open: boolean; grants: Grant[] }) {
  const [shown, setShown] = useState(false);
  const pending = calls.filter((c) => !c.result);
  const failed = calls.some((c) => c.result?.is_error);
  const expanded = shown || open;
  const visible = expanded ? calls : [];
  return <div className="x-steps">
    <button onClick={() => setShown(!shown)} aria-expanded={expanded}>
      {pending.length ? <span className="x-spin" /> : <Icon name={failed ? "warn" : "check"} size={15} className={failed ? "x-bad" : "x-ok"} />}
      <b>{pending.length ? stepLabel(pending[0].name, pending[0].input, pending[0].summary, grants) : `${calls.length} step${calls.length === 1 ? "" : "s"}`}</b>
      {!pending.length && <span className="x-faint">· {expanded ? "hide" : "show"}</span>}
    </button>
    {visible.length > 0 && <div className="x-act">{visible.map((c) => <div key={c.id} title={c.result?.is_error ? c.result.output.slice(0, 400) : undefined}>
      {!c.result ? <span className="x-spin" style={{ width: 12, height: 12 }} /> : <Icon name={c.result.is_error ? "close" : "check"} size={13} className={c.result.is_error ? "x-bad" : "x-ok"} />}
      <span>{stepLabel(c.name, c.input, c.summary, grants)}</span>
    </div>)}</div>}
  </div>;
}

function AskCard({ row, sessionId }: { row: Extract<Row, { t: "ask" }>; sessionId: string }) {
  const [picked, setPicked] = useState<Record<number, string[]>>({});
  const [busy, setBusy] = useState(false);
  const send = async (all: Record<number, string[]>) => {
    setBusy(true);
    try { await api.post(`/api/sessions/${sessionId}/answer`, { request_id: row.request_id, answers: row.questions.map((_, i) => (all[i] ?? []).join(", ")) }); }
    catch (e) { toastError(e); }
    finally { setBusy(false); }
  };
  const choose = (qi: number, label: string) => {
    const q = row.questions[qi];
    const cur = picked[qi] ?? [];
    const all = { ...picked, [qi]: q.multi_select ? (cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label]) : [label] };
    setPicked(all);
    if (!q.multi_select && row.questions.every((_, i) => (all[i] ?? []).length > 0)) void send(all);
  };
  return <div className="x-card x-ask">
    {row.questions.map((q, qi) => <div key={qi} style={{ display: "grid", gap: 6 }}>
      <h3><Icon name={q.permission ? "lock" : "q"} size={16} />{q.permission ? "Approval needed" : q.header || "Question"}</h3>
      {q.permission ? <pre>{q.question}</pre> : <p>{q.question}</p>}
      {q.options.map((o) => <button key={o.label} disabled={busy} className={`x-btn ${(picked[qi] ?? []).includes(o.label) ? "x-pri" : ""}`} style={{ textAlign: "left", fontWeight: 500 }} onClick={() => choose(qi, o.label)}>
        <b>{o.label}</b>{o.description && <span className="x-small x-faint" style={{ display: "block" }}>{o.description}</span>}
      </button>)}
    </div>)}
    {row.questions.some((q) => q.multi_select) && <button className="x-btn x-pri" disabled={busy} onClick={() => void send(picked)}>Send answer</button>}
    {!row.questions.some((q) => q.permission) && <p className="x-small x-faint">Or type another answer below.</p>}
  </div>;
}

const verbs: Record<string, string> = { browser_click: "Click", browser_type: "Type in", browser_select: "Choose in", browser_press: "Press", browser_scroll: "Scroll", browser_wait: "Wait on" };

function ReviewCard({ review }: { review: Review }) {
  const client = useQueryClient();
  const [busy, setBusy] = useState(false);
  const { tool, args, target, page } = review.details;
  const url = target.url;
  const decide = async (allow: boolean, always = false) => {
    setBusy(true);
    try {
      // Chrome asks for the destination site during this click.
      if (allow && url && !await chrome.permissions.request({ origins: [`${siteOf(url)}/*`] })) throw new Error("Chrome access to the site was declined");
      await api.post(`/api/browser/reviews/${review.id}`, { allow, always });
      await client.invalidateQueries({ queryKey: ["browser-reviews"] });
      if (always) await client.invalidateQueries({ queryKey: ["browser-sites"] });
    } catch (e) { toastError(e); }
    finally { setBusy(false); }
  };
  const site = hostOf(page.url);
  if (tool === "browser_handover") return <div className="x-card x-ask" role="region" aria-label="Your turn">
    <h3><Icon name="lock" size={16} />Your turn on {site}</h3>
    <p className="x-small">The agent asked you to take over (for example to sign in, solve a check, upload a file or pay). It never types passwords or codes, and it is not controlling this tab now.</p>
    <div className="x-btns"><button className="x-btn x-wide" disabled={busy} onClick={() => void chrome.tabs.update(page.tab, { active: true }).catch(toastError)}>Show the tab</button><button className="x-btn x-pri x-wide" disabled={busy} onClick={() => void decide(true)}>I'm done — continue</button></div>
    <button className="x-btn x-link x-small" disabled={busy} onClick={() => void decide(false)}>Don't continue</button>
  </div>;
  if (tool === "browser_navigate" || tool === "browser_open_tab") return <div className="x-card x-ask" role="region" aria-label="Site review">
    <h3><Icon name="globe" size={16} />Go to {url ? hostOf(url) : "another site"}?</h3>
    <div className="x-kv"><i>From</i><span>{site}</span><i>To</i><span>{url}</span></div>
    <div className="x-btns"><button className="x-btn x-pri x-wide" disabled={busy} onClick={() => void decide(true)}>Allow</button><button className="x-btn x-wide" disabled={busy} onClick={() => void decide(false)}>Not now</button></div>
    {url && <button className="x-btn x-link x-small" disabled={busy} onClick={() => void decide(true, true)}>Always allow {hostOf(url)}</button>}
  </div>;
  const fields = target.targets ?? [];
  const form = tool === "browser_propose_form";
  const label = fields[0]?.label;
  return <div className="x-card x-ask" role="region" aria-label="Action review">
    <h3><Icon name="warn" size={16} />{form ? `Fill in ${fields.length} field${fields.length === 1 ? "" : "s"}?` : tool === "browser_press" ? `Press ${String(args.key ?? "a key")}${label ? ` in “${label}”` : ""}?` : `${verbs[tool] ?? "Act on"} “${label || "this element"}”?`}</h3>
    <div className="x-kv"><i>Site</i><span>{site}</span>{!form && fields[0]?.value && <><i>Value</i><span>{fields[0].value}</span></>}</div>
    {form && <div className="x-kv">{fields.map((f) => <><i key={`l${f.ref}`}>{f.label || f.ref}</i><span key={`v${f.ref}`}>{f.value}</span></>)}</div>}
    <p className="x-small x-faint">{form ? "Nothing is typed before you allow it. Sites can save as you type." : "This may submit, send, pay or delete something. Allow applies to this one action only."}</p>
    <div className="x-btns"><button className="x-btn x-pri x-wide" disabled={busy} onClick={() => void decide(true)}>{form ? "Fill in" : "Allow"}</button><button className="x-btn x-wide" disabled={busy} onClick={() => void decide(false)}>Not now</button></div>
  </div>;
}

function Reply({ s, grants }: { s: Session; grants: Grant[] }) {
  const client = useQueryClient();
  const tabs = useWindowTabs();
  const sites = useSites().data ?? [];
  const key = `launchpad-extension-reply:${s.id}`;
  const [text, setText] = useState(() => localStorage.getItem(key) ?? "");
  const [chips, setChips] = useState<TabChip[]>([]);
  const [images, setImages] = useState<FileAttachment[]>([]);
  const [busy, setBusy] = useState(false);
  useEffect(() => { if (text) localStorage.setItem(key, text); else localStorage.removeItem(key); }, [key, text]);
  const target = s.launch_spec.target;
  const act = target.kind === "browser" && target.act;
  const running = isRunning(s) || s.status === "quota_paused";
  // Without any shared tab, the current tab is proposed again, like in a new task.
  const current = tabs.find((t) => t.active);
  const suggested = !grants.length && !chips.length && current?.id != null && /^https?:/.test(current.url ?? "") ? [chipOf(current)] : [];
  const shown = chips.length ? chips : suggested;
  const send = async () => {
    const message = text.trim();
    if (!message && !images.length && !shown.length) return;
    setBusy(true);
    try {
      if (shown.length) {
        if (!await requestSites(shown)) throw new Error("Chrome access to the site was declined");
        for (const g of await grantTabs(shown)) await api.post(`/api/browser/sessions/${s.id}/grants/${g.id}`);
        await client.invalidateQueries({ queryKey: ["browser-grants"] });
      }
      if (message || images.length) await api.post(`/api/sessions/${s.id}/input`, { text: message, images });
      setText(""); setChips([]); setImages([]);
    } catch (e) { toastError(e); }
    finally { setBusy(false); }
  };
  const setAct = async (next: boolean, always?: { sites: string[]; on: boolean }) => {
    try {
      await rememberSites(always, sites);
      await api.post(`/api/browser/sessions/${s.id}/mode`, { act: next });
      await client.invalidateQueries({ queryKey: ["browser-grants"] });
      await client.invalidateQueries({ queryKey: ["browser-sites"] });
    } catch (e) { toastError(e); }
  };
  const archived = s.status === "closed";
  return <Box reply chips={shown} setChips={setChips} shared={grants.map((g) => g.page.tab)} text={text} setText={setText} images={images} setImages={setImages}
    act={act} setAct={(a, always) => void setAct(a, always)} sites={sites}
    running={running} busy={busy} canSend={!!(text.trim() || images.length || shown.length)}
    placeholder={archived ? "Reply to continue this task…" : running ? "Add a message…" : "Reply…"}
    onSend={() => void send()} onStop={() => void api.post(`/api/sessions/${s.id}/interrupt`).catch(toastError)}
    hint={running ? "You can type while it works. Your message is read at the next step." : !grants.length && !shown.length ? "No tab is shared with this task anymore. Add one with + Tabs." : undefined} />;
}
