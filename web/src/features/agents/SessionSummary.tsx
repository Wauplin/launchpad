import { Fragment, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import type { Session } from "@/gen/Session";
import type { Predraft } from "@/gen/Predraft";
import type { LaunchDelay } from "@/gen/LaunchDelay";
import type { SessionGitStatus } from "@/gen/SessionGitStatus";
import { Icon } from "@/components/Icon";
import { Markdown } from "@/components/Markdown";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useStatus } from "@/app/StatusBar";
import { copyText } from "@/lib/clipboard";
import { formatAge } from "@/lib/format";
import { useProjects, useSchedules, useSessions } from "@/lib/queries";
import { conditionsSummary, DelaySelector } from "@/features/launch/DelaySelector";
import { compact, statusInfo, STYLES } from "./SessionBits";
import type { Row } from "./rows";

export function StateBand({ session: s, children }: { session: Session; children?: ReactNode }) {
  const { style } = statusInfo(s);
  const label = s.status === "waiting" ? s.open_question ? "Question" : "Done" : s.status === "quota_paused" ? "Quota paused" : s.status === "closed" ? "Archived" : s.status === "error" ? "Failed" : s.status === "stopped" ? "Stopped" : s.status === "in_terminal" ? "In terminal" : "Interrupted";
  const icon = s.status === "waiting" ? s.open_question ? "alert" : "okcircle" : s.status === "closed" ? "archive" : s.status === "stopped" ? "stop" : s.status === "error" ? "xcircle" : "refresh";
  const at = s.status === "closed" ? s.ended_at ?? s.status_since : s.status_since;
  return <div className={`flex flex-wrap items-center gap-2 border-b border-line px-4 py-2 text-xs ${STYLES[style]} ${s.status === "stopped" ? "border-dashed" : ""}`}>
    <Icon name={icon} size={16} /><b>{label}</b>
    <span>{s.status === "waiting" ? s.open_question ? "The agent needs an answer" : "Waiting for you" : s.status === "closed" ? "Resumable" : s.status === "stopped" ? "By you, during a turn" : null}</span>
    <span className="ml-auto" title={new Date(at).toLocaleString()}>{s.status === "interrupted" ? "Recorded " : ""}{new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })} · {formatAge(at)} ago</span>
    {children}
  </div>;
}

function renderedText(element: HTMLElement | null, fallback: string): string {
  if (!element) return fallback;
  const copy = element.cloneNode(true) as HTMLElement;
  copy.querySelectorAll("button").forEach((b) => b.remove());
  copy.style.position = "fixed";
  copy.style.left = "-10000px";
  document.body.append(copy);
  const text = copy.innerText;
  copy.remove();
  return text;
}

function Section({ title, actions, children }: { title: string; actions?: ReactNode; children: ReactNode }) {
  return <section className="border-b border-line px-4 py-3">
    <div className="mb-2 flex flex-wrap items-center gap-2"><h3 className="text-[10.5px] font-semibold tracking-wider text-ink-3 uppercase">{title}</h3>{actions && <span className="ml-auto flex gap-1">{actions}</span>}</div>
    {children}
  </section>;
}

function Fold({ children, lines, label }: { children: ReactNode; lines: number; label: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [long, setLong] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const check = () => setLong(el.scrollHeight > parseFloat(getComputedStyle(el).lineHeight) * lines + 2);
    const observer = new ResizeObserver(check);
    observer.observe(el); check();
    return () => observer.disconnect();
  }, [lines, children]);
  return <><div ref={ref} className="overflow-hidden text-[12.5px] leading-[1.6]" style={{ maxHeight: expanded ? undefined : `${lines * 1.6}em` }}>{children}</div>{long && <button className="btn btn-sm mt-2" onClick={() => setExpanded(!expanded)}>{expanded ? "Show less" : label}</button>}</>;
}

function NextStep({ source, text, button, onClick, disabled }: { source: string; text: string; button: string; onClick: () => void; disabled?: boolean }) {
  return <div className="flex flex-wrap items-center gap-2 border-t border-line-2 py-2 first:border-0">
    <span className="w-14 shrink-0 text-[10px] font-semibold text-ink-3 uppercase">{source}</span><span className="min-w-0 flex-1 basis-40 text-xs">{text}</span>
    <button className="btn btn-sm" disabled={disabled} onClick={onClick}>{button}</button>
  </div>;
}

export function FollowUpBands({ sessionId }: { sessionId: string }) {
  const schedules = useSchedules().data ?? [];
  const sessions = useSessions().data ?? [];
  const [busy, setBusy] = useState(false);
  return <>{schedules.filter((q) => q.resume_session_id === sessionId).map((q) => <div key={q.id} className="flex flex-wrap items-center gap-2 border-b border-teal bg-teal-soft px-4 py-2 text-xs text-teal-ink">
    <Icon name="clock" size={16} /><b>{q.status === "failed" ? "Follow-up needs attention" : "Follow-up scheduled"}</b><span className="min-w-0 flex-1">{conditionsSummary(q, sessions, schedules)}{q.error && ` · ${q.error}`}</span>
    <button className="btn btn-sm" disabled={busy || q.status === "starting"} onClick={async () => { setBusy(true); try { await api.delete(`/api/launch/schedules/${q.id}`); } catch (e) { toastError(e); } finally { setBusy(false); } }}>Cancel</button>
  </div>)}</>;
}

export function SessionSummary({ session: s, rows, localPr, reviewDiff, showTrace }: { session: Session; rows: Row[]; localPr?: Predraft; reviewDiff: (main: boolean) => void; showTrace: () => void }) {
  const navigate = useNavigate();
  const projects = useProjects().data ?? [];
  const schedules = useSchedules().data ?? [];
  const git = useQuery({ queryKey: ["session-git", s.id], queryFn: () => api.get<SessionGitStatus>(`/api/sessions/${s.id}/git`), refetchInterval: 5000 });
  const [scheduling, setScheduling] = useState(false);
  const [message, setMessage] = useState("");
  const [conditions, setConditions] = useState<LaunchDelay[]>([{ kind: "in", minutes: 30 }]);
  const [valid, setValid] = useState(true);
  const [busy, setBusy] = useState(false);
  const [choices, setChoices] = useState<Record<number, string>>({});
  const finalRef = useRef<HTMLDivElement>(null);
  const final = rows.filter((r) => r.t === "assistant").at(-1);
  const task = rows.find((r) => r.t === "user");
  const turns = rows.filter((r) => r.t === "turn");
  const duration = turns.length && turns.every((r) => r.duration_ms != null) ? turns.reduce((n, r) => n + r.duration_ms!, 0) : null;
  const commands = rows.flatMap((r) => r.t === "tools" ? r.calls : []).filter((c) => /^(Bash|Shell)$/.test(c.name));
  const lastCommand = commands.at(-1);
  const lastAsk = rows.filter((r) => r.t === "ask").at(-1);
  const question = s.open_question && lastAsk && !lastAsk.answers ? lastAsk : null;
  const refs = [...s.launch_spec.github_refs];
  if (localPr?.published_url && !refs.some((r) => r.url === localPr.published_url)) refs.push({ url: localPr.published_url, kind: "pr", repo: "", number: localPr.published_number, title: localPr.title });
  const tokens = s.usage.input_tokens + s.usage.cached_tokens + s.usage.output_tokens;
  const changes = git.data?.main_changes;
  const metadata: [string, string][] = [];
  if (turns.length) metadata.push(["Time", `${duration == null ? "" : `${Math.floor(duration / 60000)} min ${Math.round(duration / 1000) % 60} s · `}${turns.length} recorded ${turns.length === 1 ? "turn" : "turns"}`]);
  if (s.harness === "claude" && s.usage.cost_usd != null) metadata.push(["Cost", `$${s.usage.cost_usd.toFixed(2)}${tokens ? ` · ${compact(tokens)} tokens` : ""}`]);
  else if (tokens) metadata.push(["Tokens", compact(tokens)]);
  const model = [s.model, s.effort, s.usage.context_tokens != null && s.usage.context_window ? `${Math.round(100 * s.usage.context_tokens / s.usage.context_window)}% context` : null].filter(Boolean).join(" · ");
  if (model) metadata.push(["Model", model]);
  metadata.push(["Paid usage", s.launch_spec.allow_paid_usage ? "Allowed" : "Quota protection enabled (best effort)"]);
  metadata.push(["Folder", s.cwd]);
  if (git.data) metadata.push(["Branch", `${git.data.branch} · ${git.data.files.length ? `${git.data.files.length} uncommitted files` : "clean"}${git.data.unpushed_commits == null ? "" : git.data.unpushed_commits ? ` · ${git.data.unpushed_commits} commits absent from cached remote refs` : " · present in cached remote refs"}`]);
  if (changes) metadata.push(["Changes", `${changes.files} tracked files compared with main${changes.additions == null ? "" : ` · +${changes.additions} −${changes.deletions}`}`]);
  const context = [...s.launch_spec.github_refs.map((r) => `${r.repo}${r.number ? ` #${r.number}` : ""} · ${r.kind}`), ...s.launch_spec.context_projects.map((r) => `${projects.find((p) => p.id === r.project_id)?.name ?? r.project_id}${r.branch ? ` (${r.branch})` : ""}`), ...s.launch_spec.files.map((r) => `${r.path}${r.start_line ? `:${r.start_line}${r.end_line ? `–${r.end_line}` : ""}` : ""}`), ...s.launch_spec.presets];
  if (context.length) metadata.push(["Context", context.join(" · ")]);
  if (lastCommand) metadata.push([lastCommand.result ? "Last command" : s.status === "stopped" ? "Cut off" : "Last command", `${lastCommand.summary}${lastCommand.result?.exit_code != null ? ` · exit ${lastCommand.result.exit_code}` : ""}`]);
  const run = async (action: () => Promise<unknown>) => { setBusy(true); try { await action(); } catch (e) { toastError(e); } finally { setBusy(false); } };
  const openPr = () => void run(async () => {
    const p = localPr ?? await api.post<Predraft>("/api/predrafts", { cwd: s.cwd, title: s.title });
    localStorage.setItem(`launchpad.session-predraft.${s.id}`, p.id);
    await navigate({ to: "/predrafts/$id", params: { id: p.id } });
  });
  const answer = (i: number, label: string) => {
    if (!question) return;
    const selected = { ...choices, [i]: label }; setChoices(selected);
    if (question.questions.every((_, j) => selected[j])) void run(async () => {
      if (s.live) await api.post(`/api/sessions/${s.id}/answer`, { request_id: question.request_id, answers: question.questions.map((_, j) => selected[j]) });
      else await api.post(`/api/sessions/${s.id}/input`, { text: question.questions.map((q, j) => `${q.question}\n${selected[j]}`).join("\n\n") });
      showTrace();
    });
  };
  const followUp = schedules.some((q) => q.resume_session_id === s.id);
  const unfinished = ["stopped", "interrupted", "error"].includes(s.status);
  return <div className="min-h-0 flex-1 overflow-y-auto bg-white">
    <StateBand session={s} /><FollowUpBands sessionId={s.id} /><QuotaBand session={s} />
    <Section title="Run"><dl className="grid grid-cols-[90px_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-xs">{metadata.map(([key, value]) => <Fragment key={key}><dt className="text-ink-3">{key}</dt><dd className="break-words whitespace-pre-wrap">{value}</dd></Fragment>)}</dl></Section>
    <Section title="Task · first prompt"><Fold lines={4} label="Show full task"><div className="whitespace-pre-wrap">{task?.text ?? s.prompt}</div></Fold></Section>
    {final && <Section title={s.status === "stopped" || unfinished ? "Last agent message" : "Final message"} actions={<><button className="btn btn-sm" onClick={() => void copyText(final.text).then(() => toast("Copied"))}>Copy</button><button className="btn btn-sm" onClick={() => void copyText(renderedText(finalRef.current, final.text)).then(() => toast("Copied as text"))}>Copy as text</button></>}><Fold lines={30} label="Show full message"><div ref={finalRef}><Markdown text={final.text} /></div></Fold></Section>}
    <Section title="Next steps">
      {(localPr && !localPr.published_url || (git.data?.unpushed_commits ?? 0) > 0 || git.data && git.data.branch !== "main" && !refs.some((r) => r.kind === "pr")) && s.project_id && <NextStep source="Git" text={localPr ? "This session has a local pre-draft PR." : (git.data?.unpushed_commits ?? 0) > 0 ? `${git.data!.unpushed_commits} commits are absent from cached remote refs.` : "No PR is linked to this session."} button="Open pre-draft PR" onClick={openPr} disabled={busy} />}
      {((changes?.files ?? 0) > 0 || (git.data?.files.length ?? 0) > 0) && <NextStep source="Git" text={`${git.data?.files.length ?? 0} uncommitted files${changes ? `; ${changes.files} tracked files differ from main` : ""}.`} button="Review diff" onClick={() => reviewDiff((changes?.files ?? 0) > 0)} />}
      {(unfinished || s.status === "closed") && <NextStep source="Session" text={s.status === "closed" ? "This archived session is still resumable." : "The last turn did not finish successfully."} button="Resume" disabled={busy} onClick={() => void run(async () => { if (unfinished) await api.post(`/api/sessions/${s.id}/input`, { text: "Continue the unfinished turn." }); else await api.post(`/api/sessions/${s.id}/resume`); showTrace(); })} />}
      {refs.map((r, i) => <NextStep key={i} source="GitHub" text={r.title ?? `Linked ${r.kind}${r.number ? ` #${r.number}` : ""}.`} button={`Open ${r.number ? `#${r.number}` : r.kind}`} onClick={() => { if (r.number && r.repo.includes("/")) { const [owner, repo] = r.repo.split("/"); void navigate({ to: "/github/$owner/$repo/$number", params: { owner, repo, number: String(r.number) } }); } else void api.post("/api/open", { url: r.url }).catch(toastError); }} />)}
      <NextStep source="Session" text={followUp ? "A follow-up is already scheduled for this session." : "Continue this session later with a new message."} button="Schedule follow-up" disabled={followUp || s.status === "quota_paused"} onClick={() => setScheduling(true)} />
      {question && question.questions.length > 1 && <p className="py-1 text-xs text-ink-3">Choose an answer for each question; the last choice sends all answers.</p>}
      {question?.questions.flatMap((q, i) => q.options.map((o) => <NextStep key={`${i}-${o.label}`} source="Agent" text={`${q.question} ${o.description}`} button={`${choices[i] === o.label ? "✓ " : ""}${o.label}`} disabled={busy} onClick={() => answer(i, o.label)} />))}
      {scheduling && <div className="grid gap-3 border-t border-line py-3"><b className="text-xs">Schedule follow-up · same session, new message</b><textarea className="field-input min-h-20" aria-label="Follow-up message" placeholder="Message to send later…" value={message} onChange={(e) => setMessage(e.target.value)} /><DelaySelector conditions={conditions} harness={s.harness} onChange={setConditions} onValidityChange={setValid} /><p className="text-xs text-ink-3">If this session is running, in a terminal or awaiting an answer at that time, the follow-up stays queued until it is ready.</p><div className="flex gap-2"><button className="btn btn-sm btn-primary" disabled={busy || !message.trim() || !conditions.length || !valid} onClick={() => void run(async () => { await api.post(`/api/sessions/${s.id}/follow-up`, { message, conditions }); setScheduling(false); setMessage(""); })}>Schedule</button><button className="btn btn-sm" onClick={() => setScheduling(false)}>Cancel</button></div></div>}
    </Section>
  </div>;
}

export function QuotaBand({ session: s }: { session: Session }) {
  const [busy, setBusy] = useState(false);
  const quota = useStatus().data?.[s.harness];
  const p = s.quota_pause;
  if (!p) return null;
  const act = async (action: string) => {
    setBusy(true);
    try { await api.post(`/api/sessions/${s.id}/${action}`); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <div className="grid gap-2 border-b border-attn-line bg-attn-soft px-4 py-3 text-xs text-attn">
    <b>{p.phase === "needs_attention" ? "Quota pause needs attention" : "Included quota exhausted · automatic continuation pending"}</b>
    <span>{p.blockers.map((b) => `${b.window} reset: ${new Date(b.resets_at * 1000).toLocaleString()}`).join(" · ")}</span>
    <span>{p.error ?? (p.phase === "pausing" ? "Interrupt requested; waiting for the turn and commands to finish." : p.phase === "resuming" ? "Continuing…" : "Waiting for a fresh quota reading confirming both windows are available. Reset times are estimates; failed readings keep this paused.")}</span>
    {quota?.quota_error && <span>Quota reading unavailable: {quota.quota_error}</span>}
    <span>Protection is best effort. Some paid usage may occur before interruption; running commands may still finish.</span>
    <div className="flex flex-wrap gap-2"><button className="btn btn-sm" disabled={busy || p.phase === "resuming"} onClick={() => void act("interrupt")}>Stop</button><button className="btn btn-sm" title="Allow paid usage for the rest of this session and continue now" disabled={busy || p.phase === "pausing" || p.phase === "resuming"} onClick={() => void act("resume-paid")}>Resume with paid usage</button></div>
  </div>;
}
