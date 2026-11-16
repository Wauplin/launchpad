// Schedules → Oracles: the workspace's oracles (small Python checks Home runs to answer True, False or Failed), one
// oracle's page (Definition with Test, Access, Dependencies, History, Used by, Versions) and the New oracle form.
// Routines gate on them (`GateFields` in the routine editor).

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { memo, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { ActionButton, ActionMenuItem } from "@/components/ActionButton";
import { Badge, Choice, Choices, ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, MenuSection, MoreMenu, PageHeader, RowActions, StatusBadge, Tabs, Token } from "@/components/ui";
import { PatchView } from "@/components/diff/DiffReview";
import type { Change } from "@/gen/Change";
import type { Gate } from "@/gen/Gate";
import type { Oracle } from "@/gen/Oracle";
import type { OracleAccess } from "@/gen/OracleAccess";
import type { OracleDetail } from "@/gen/OracleDetail";
import type { OracleEvaluation } from "@/gen/OracleEvaluation";
import type { OracleListItem } from "@/gen/OracleListItem";
import type { OracleOutcome } from "@/gen/OracleOutcome";
import type { OracleRunResult } from "@/gen/OracleRunResult";
import type { OracleTestAnswer } from "@/gen/OracleTestAnswer";
import { api } from "@/lib/api";
import { runAction } from "@/lib/actions";
import { useBots } from "@/lib/bots";
import { ago, when } from "@/lib/format";
import { useKeptDraft, versionOf } from "@/lib/keptDraft";
import { gateSummary, OUTCOME_LABEL, OUTCOME_SQUARE, OUTCOME_TONE, STARTER, accessChange, accessOf, accessSummary, dependencySummary, failing, paramLabel, paramsOf, plural, useOracle, useOracleEvaluations, useOracleTools, useOracles, withAccess, type OracleTool } from "@/lib/oracles";
import { usePhone } from "@/lib/phone";
import { PhoneHeader } from "@/mobile/PhoneHeader";

export { gateSummary };

/** `?oracle=` of the New oracle form (never a valid slug). */
export const NEW_ORACLE = "_new";

export type OracleView = "definition" | "access" | "dependencies" | "history" | "used" | "versions";
const VIEWS: OracleView[] = ["definition", "access", "dependencies", "history", "used", "versions"];
export const oracleView = (v: unknown): OracleView => (VIEWS.includes(v as OracleView) ? (v as OracleView) : "definition");

type Filter = "all" | "failing" | "unused";

// ---------------------------------------------------------------- list

/** The oracles of the workspace, with All / Failing / Unused. */
export function OracleList({ workspace, onOpen }: { workspace: string; onOpen: (slug: string) => void }) {
  const list = useOracles(workspace);
  const tools = useOracleTools(workspace).data ?? [];
  const [filter, setFilter] = useState<Filter>("all");
  const items = list.data ?? [];
  const failingCount = items.filter((i) => failing(i.streak)).length;
  const unused = items.filter((i) => !i.used_by.length).length;
  const shown = items.filter((i) => filter === "all" || (filter === "failing" ? failing(i.streak) : !i.used_by.length));
  if (list.isLoading) return <Loading what="oracles" />;
  if (list.error) return <ErrorState error={list.error} onRetry={() => void list.refetch()} />;
  if (!items.length) return <Empty icon="spark">No oracle yet. An oracle is a small Python check that Home runs to answer True, False or Failed; a routine can run only when one says so. Ask Launchpad to write one, or create it with New oracle.</Empty>;
  return <>
    <FilterChips label="Oracles" tour="oracles.filters">
      <FilterChip active={filter === "all"} count={items.length} onClick={() => setFilter("all")}>All</FilterChip>
      <FilterChip active={filter === "failing"} count={failingCount} countNeeds={failingCount > 0} onClick={() => setFilter("failing")}>Failing</FilterChip>
      <FilterChip active={filter === "unused"} count={unused} onClick={() => setFilter("unused")}>Unused</FilterChip>
    </FilterChips>
    {shown.length ? <div className="grid grid-cols-[minmax(0,1fr)] overflow-hidden rounded-card border border-line" data-tour="oracles.list">
      {shown.map((i) => <OracleRow key={i.oracle.slug} item={i} tools={tools} workspace={workspace} onOpen={onOpen} />)}
    </div> : <Empty>{filter === "failing" ? "No oracle is failing." : "Every oracle is used by a routine."}</Empty>}
  </>;
}

/** Recent outcomes as squares, oldest first. */
export function OutcomeSquares({ recent, size = 9 }: { recent: { outcome: OracleOutcome; at: number }[]; size?: number }) {
  if (!recent.length) return null;
  return <span className="flex items-center gap-1" aria-label="Recent evaluations">
    {[...recent].reverse().map((r, i) => <span key={`${r.at}-${i}`} style={{ width: size, height: size }} className={`rounded-xs ${OUTCOME_SQUARE[r.outcome]}`} title={`${when(r.at)} · ${OUTCOME_LABEL[r.outcome]}`} />)}
  </span>;
}

/** One line about the last answer: "True 6 min ago · tests.yml on 4be1c2a: failure", or the failure streak. */
function lastLine(i: OracleListItem): { text: string; bad: boolean } {
  if (i.oracle.error) return { text: `The file cannot be used: ${i.oracle.error}`, bad: true };
  if (failing(i.streak) && i.streak) {
    const e = i.streak.last_error;
    return { text: `Failed ${i.streak.count}× since ${when(i.streak.first_at ?? i.streak.last_at ?? 0)}${e ? ` · ${e.class}: ${e.message}` : ""}`, bad: true };
  }
  if (!i.last) return { text: "Never evaluated", bad: false };
  const l = i.last;
  const what = l.outcome === "failed" && l.error ? `${l.error.class}: ${l.error.message}` : l.summary;
  return { text: `${OUTCOME_LABEL[l.outcome]} ${ago(l.created_at)}${what ? ` · ${what}` : ""}`, bad: l.outcome === "failed" };
}

function usedBy(i: OracleListItem, bots: { id: string; settings: { name: string } }[]): string {
  if (!i.used_by.length) return "unused";
  if (i.used_by.length === 1) {
    const u = i.used_by[0];
    const bot = bots.find((b) => b.id === u.bot_id)?.settings.name;
    return `used by ${bot ? `${bot} ` : ""}“${u.title}”`;
  }
  return `used by ${plural(i.used_by.length, "routine")}`;
}

const OracleRow = memo(function OracleRow({ item, tools, workspace, onOpen }: { item: OracleListItem; tools: OracleTool[]; workspace: string; onOpen: (slug: string) => void }) {
  const bots = useBots().data ?? [];
  const phone = usePhone();
  const o = item.oracle;
  const last = lastLine(item);
  const clamp = phone ? "line-clamp-2 break-words" : "truncate";
  const tone = o.paused ? "bg-ink-3/40" : item.last ? OUTCOME_SQUARE[failing(item.streak) ? "failed" : item.last.outcome] : "bg-line";
  const facts = [usedBy(item, bots), accessSummary(o.meta?.access, tools), dependencySummary(o.meta)];
  return <div className="lp-row grid min-h-[60px] w-full grid-cols-[12px_minmax(0,1fr)_auto] items-center gap-x-3 border-b border-line bg-paper px-3 py-2 last:border-b-0 hover:bg-sunk">
    <span className={`size-2 rounded-full ${tone}`} aria-hidden="true" />
    <button className="min-w-0 text-left" onClick={() => onOpen(o.slug)}>
      <span className={`block text-[13px] font-semibold ${clamp}`}>{o.meta?.title ?? o.slug}<span className="font-normal text-ink-3"> · {o.slug}</span>{o.paused && <span className="font-normal text-ink-3"> · paused</span>}</span>
      <span className={`block text-[12px] ${clamp} ${last.bad && !o.paused ? "text-gh-closed" : "text-ink-3"}`} title={last.text}>{last.text}</span>
      <span className={`block text-[12px] text-ink-3 ${clamp}`}>{facts.join(" · ")}</span>
    </button>
    <span className="flex items-center gap-3">
      <span className="hidden sm:flex"><OutcomeSquares recent={item.recent} /></span>
      <RowActions width={phone ? 0 : undefined} label={`More actions for ${o.meta?.title ?? o.slug}`} main={phone ? undefined : <button className="btn btn-sm" onClick={() => onOpen(o.slug)}>Open</button>} more={(close) => <>
        <ActionMenuItem name="oracle.test" params={{ workspace, slug: o.slug }} body={{}} close={close} onDone={(a: OracleTestAnswer) => toast(resultLine(a.result))} />
        {o.paused ? <ActionMenuItem name="oracle.resume" params={{ workspace, slug: o.slug }} close={close} /> : <ActionMenuItem name="oracle.pause" params={{ workspace, slug: o.slug }} close={close} />}
      </>} />
    </span>
  </div>;
});

/** "True · tests.yml on 4be1c2a: failure" */
export function resultLine(r: Pick<OracleRunResult, "outcome" | "summary" | "error">): string {
  const what = r.outcome === "failed" && r.error ? `${r.error.class}: ${r.error.message}` : r.summary;
  return `${OUTCOME_LABEL[r.outcome]}${what ? ` · ${what}` : ""}`;
}

// ---------------------------------------------------------------- one oracle

/** One oracle's page, or the New oracle form (`slug` = NEW_ORACLE). Its tab is `view` (the address's `?view=`). */
export function OraclePage({ workspace, slug, view, onView, onBack, onSaved, onDeleted }: { workspace: string; slug: string; view: OracleView; onView: (v: OracleView) => void; onBack: () => void; onSaved: (slug: string) => void; onDeleted: () => void }) {
  const isNew = slug === NEW_ORACLE;
  const detail = useOracle(workspace, isNew ? null : slug);
  if (!isNew && detail.isLoading) return <Loading what="the oracle" />;
  if (!isNew && (detail.error || !detail.data)) return <div className="px-5 py-4"><ErrorState error={detail.error ?? "This oracle no longer exists"} action={<button className="btn btn-sm" onClick={onBack}>Back to oracles</button>} /></div>;
  return <OracleEditor key={isNew ? NEW_ORACLE : `${slug}-${detail.data!.oracle.sha256}`} workspace={workspace} detail={isNew ? null : detail.data!} view={view} onView={onView} onBack={onBack} onSaved={onSaved} onDeleted={onDeleted} />;
}

function OracleEditor({ workspace, detail, view, onView, onBack, onSaved, onDeleted }: { workspace: string; detail: OracleDetail | null; view: OracleView; onView: (v: OracleView) => void; onBack: () => void; onSaved: (slug: string) => void; onDeleted: () => void }) {
  const phone = usePhone();
  const qc = useQueryClient();
  const tools = useOracleTools(workspace).data ?? [];
  const saved = detail?.content ?? STARTER;
  const draft = useKeptDraft(`oracle.${workspace}.${detail?.oracle.slug ?? NEW_ORACLE}.${versionOf(saved)}`, saved);
  const [newSlug, setNewSlug] = useState("");
  const [busy, setBusy] = useState(false);
  const o = detail?.oracle ?? null;
  const meta = o?.meta ?? null;
  const slug = o?.slug ?? newSlug.trim();
  const dirty = !detail || draft.text !== saved;
  const draftAccess = accessOf(draft.text);
  const change = accessChange(meta?.access, draftAccess);
  const slugOk = /^[a-z0-9][a-z0-9_-]{0,62}$/.test(slug);
  const save = async () => {
    setBusy(true);
    try {
      await runAction<Oracle>("oracle.save", { workspace, slug }, { content: draft.text });
      draft.clear();
      await Promise.all([qc.invalidateQueries({ queryKey: ["oracles"] }), qc.invalidateQueries({ queryKey: ["oracle", workspace, slug] })]);
      toast(detail ? "Oracle saved" : `Oracle ${slug} created`);
      onSaved(slug);
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const users = detail?.used_by ?? [];
  const title = meta?.title ?? (detail ? o!.slug : "New oracle");
  const metaLine = detail ? [
    <code key="slug" className="rounded bg-sunk px-1 font-mono text-[11px]">{o!.slug}</code>,
    o!.changed_at ? `edited by ${!o!.changed_by || o!.changed_by.kind === "owner" ? "you" : o!.changed_by.name} ${ago(o!.changed_at)}` : null,
    meta?.requires_python ? `Python ${meta.requires_python}` : null,
    meta ? dependencySummary(meta) : null,
    o!.paused ? "paused" : null,
  ].filter(Boolean) : null;
  const saveButton = <button className="btn btn-sm btn-primary" disabled={busy || !dirty || !slugOk} title={!slugOk ? "Give the oracle a slug: lowercase letters, digits, - and _" : undefined} onClick={() => void save()}><Icon name="check" size={12} />{busy ? "Saving…" : "Save"}</button>;
  const menu = detail && <MoreMenu label={`More actions for ${title}`} width={300}>{(close) => <>
    {draft.text !== saved && <MenuItem icon={<Icon name="x" size={14} />} label="Discard changes" detail="Back to the saved file" onClick={() => { close(); draft.setText(saved); }} />}
    {o!.paused ? <ActionMenuItem name="oracle.resume" params={{ workspace, slug: o!.slug }} close={close} onDone={() => qc.invalidateQueries({ queryKey: ["oracle", workspace, o!.slug] })} />
      : <ActionMenuItem name="oracle.pause" params={{ workspace, slug: o!.slug }} close={close} detail={users.length ? `Gated routines then record quiet checks: ${users.map((u) => `“${u.title}”`).join(", ")}` : "Nothing evaluates it until you resume it"} onDone={() => qc.invalidateQueries({ queryKey: ["oracle", workspace, o!.slug] })} />}
    <MenuSection><ActionButton name="oracle.delete" params={{ workspace, slug: o!.slug }} body={{ force: users.length > 0 }} consequence={users.length ? `Delete ${o!.slug}? Routines gated on it fail until you change their gate: ${users.map((u) => `“${u.title}”`).join(", ")}. Its file stays restorable 30 days.` : `Delete ${o!.slug} with its evaluations? Its file stays restorable 30 days.`} confirmLabel="Delete oracle" onDone={async () => { close(); await qc.invalidateQueries({ queryKey: ["oracles"] }); onDeleted(); }}>Delete</ActionButton></MenuSection>
  </>}</MoreMenu>;
  const tabs = <Tabs label="Oracle" value={view} onChange={onView} tour="oracles.tabs" items={[
    { id: "definition", label: "Definition" },
    { id: "access", label: "Access", count: draftAccess ? draftAccess.tools.length + draftAccess.hosts.length + (draftAccess.launchpad === "read" ? 1 : 0) || undefined : undefined },
    { id: "dependencies", label: "Dependencies", count: meta?.dependencies.length },
    ...(detail ? [{ id: "history" as const, label: "History" }, { id: "used" as const, label: "Used by", count: users.length }, { id: "versions" as const, label: "Versions" }] : []),
  ]} />;
  const body = <div className="grid w-full max-w-page grid-cols-[minmax(0,1fr)] gap-3 px-5 py-4">
    {phone && tabs}
    {!detail && <label className="grid max-w-[360px] gap-1 text-[12px] font-semibold text-ink-2">Slug
      <input className="field-input font-mono text-[12px] font-normal" value={newSlug} placeholder="pypi-release" maxLength={63} onChange={(e) => setNewSlug(e.target.value.toLowerCase())} />
      <span className="font-normal text-ink-3">The file is saved as oracles/{slug || "<slug>"}.py; routines and agents name the oracle by it.</span>
    </label>}
    {o?.error && <p className="m-0 rounded-control bg-attn-soft px-2 py-1 text-[12px] text-attn">The saved file cannot be used: {o.error}. Nothing evaluates it until you fix it.</p>}
    {draft.kept && dirty && detail && <p className="m-0 text-[12px] text-ink-3">Draft kept: your unsaved changes are back.</p>}
    {change.added.length > 0 && view !== "access" && <AccessNotice change={change} users={users.length} />}
    {view === "definition" && <DefinitionTab workspace={workspace} detail={detail} text={draft.text} onText={draft.setText} slug={slug} />}
    {view === "access" && <AccessTab text={draft.text} onText={draft.setText} saved={meta?.access ?? null} tools={tools} users={users.length} change={change} />}
    {view === "dependencies" && <DependenciesTab detail={detail} slug={slug} />}
    {view === "history" && detail && <HistoryTab workspace={workspace} detail={detail} />}
    {view === "used" && detail && <UsedByTab detail={detail} />}
    {view === "versions" && detail && <VersionsTab workspace={workspace} slug={detail.oracle.slug} />}
  </div>;
  if (phone) return <div className="flex h-full min-h-0 flex-col">
    <PhoneHeader title={title} onBack={onBack} backLabel="Back to oracles" subtitle={metaLine && <span className="flex flex-wrap gap-x-1.5">{metaLine.map((m, i) => <span key={i}>{m}</span>)}</span>} primary={dirty ? saveButton : undefined} actions={detail ? (close) => <>
      {o!.paused ? <ActionMenuItem name="oracle.resume" params={{ workspace, slug: o!.slug }} close={close} /> : <ActionMenuItem name="oracle.pause" params={{ workspace, slug: o!.slug }} close={close} />}
      <MenuSection><ActionButton name="oracle.delete" params={{ workspace, slug: o!.slug }} body={{ force: users.length > 0 }} confirmLabel="Delete oracle" onDone={async () => { close(); await qc.invalidateQueries({ queryKey: ["oracles"] }); onDeleted(); }}>Delete</ActionButton></MenuSection>
    </> : undefined} />
    <div className="min-h-0 flex-1 overflow-y-auto">{body}</div>
  </div>;
  return <div className="flex h-full min-h-0 flex-col">
    <PageHeader title={title} meta={metaLine ? metaLine.map((m, i) => <span key={i}>{i > 0 && "· "}{m}</span>) : "A Python check that Home runs to answer True, False or Failed"} actions={<>{saveButton}{menu}</>} tabs={tabs} />
    <div className="min-h-0 flex-1 overflow-y-auto">{body}</div>
  </div>;
}

/** Saving this draft adds access: highlighted before the save (an agent can only propose it). */
function AccessNotice({ change, users }: { change: { added: string[]; removed: string[] }; users: number }) {
  return <div className="grid gap-1 rounded-card border border-attn-line bg-attn-soft px-3 py-2 text-[12px] text-ink-2" role="status">
    <b className="text-[13px] text-ink">Saving changes access</b>
    {change.added.map((a) => <span key={a}>+ {a}</span>)}
    {change.removed.map((a) => <span key={a} className="text-ink-3">− {a}</span>)}
    <span>Used by: {users ? plural(users, "routine") : "nothing yet"}</span>
    <span className="text-ink-3">A bot can propose more access but never apply it. Narrowing access applies at once, for you and for admin bots.</span>
  </div>;
}

/** A text area for the oracle's file, as tall as its text up to a limit. */
function CodeArea({ value, onChange, label }: { value: string; onChange: (t: string) => void; label: string }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => { const el = ref.current; if (!el) return; el.style.height = "auto"; el.style.height = `${Math.min(el.scrollHeight + 2, 720)}px`; }, [value]);
  return <textarea ref={ref} aria-label={label} wrap="off" className="field-input min-h-[240px] resize-none overflow-x-auto py-1.5 font-mono text-[12px] leading-5" value={value} onChange={(e) => onChange(e.target.value)} spellCheck={false} />;
}

function DefinitionTab({ workspace, detail, text, onText, slug }: { workspace: string; detail: OracleDetail | null; text: string; onText: (t: string) => void; slug: string }) {
  const phone = usePhone();
  const meta = detail?.oracle.meta ?? null;
  return <div className={`grid min-w-0 gap-4 ${phone ? "grid-cols-[minmax(0,1fr)]" : "grid-cols-[minmax(0,1fr)_340px] items-start"}`}>
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-3">
      {meta?.description && <div className="grid gap-1"><b className="text-[12px] text-ink-2">Question</b><p className="m-0 text-[13px]">{meta.description}</p></div>}
      <div className="grid gap-1" data-tour="oracles.code"><b className="text-[12px] text-ink-2">oracles/{slug || "<slug>"}.py</b>
        <CodeArea value={text} onChange={onText} label="The oracle's file" />
        <span className="text-[12px] text-ink-3">The <code className="font-mono">{"# /// script"}</code> block holds its title, description, parameters, access and dependencies; <code className="font-mono">check(ctx)</code> returns <code className="font-mono">verdict(…)</code> or calls <code className="font-mono">failed(…)</code>.</span>
      </div>
      {meta && <div className="grid gap-1"><b className="text-[12px] text-ink-2">Parameters</b>
        {meta.params.length ? <span className="flex flex-wrap gap-1">{meta.params.map((p) => <Token key={p.name}>{paramLabel(p)}</Token>)}</span> : <span className="text-[12px] text-ink-3">None</span>}
        <span className="text-[12px] text-ink-3">Timeout {meta.timeout_ms / 1000} s · an answer is reused for {Math.round(meta.min_interval_ms / 60000)} min</span>
      </div>}
    </div>
    <TestPanel workspace={workspace} detail={detail} text={text} slug={slug} />
  </div>;
}

/** Test: runs the saved oracle (recorded as a test, commits no cursor) or the unsaved draft (not recorded), on Home. */
function TestPanel({ workspace, detail, text, slug }: { workspace: string; detail: OracleDetail | null; text: string; slug: string }) {
  const qc = useQueryClient();
  const draft = !detail || text !== detail.content;
  const params = draft ? paramsOf(text) : detail?.oracle.meta?.params ?? [];
  const [values, setValues] = useState<Record<string, string>>({});
  const [cursor, setCursor] = useState("");
  const [result, setResult] = useState<OracleRunResult | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    try {
      const body: Record<string, unknown> = { params: Object.fromEntries(Object.entries(values).filter(([, v]) => v.trim() !== "")) };
      if (draft) body.content = text;
      if (cursor.trim()) { try { body.cursor = JSON.parse(cursor); } catch { body.cursor = cursor; } }
      const a = await runAction<OracleTestAnswer>("oracle.test", { workspace, slug: slug || "draft" }, body);
      setResult(a.result);
      if (a.evaluation_id) await qc.invalidateQueries({ queryKey: ["oracle", workspace, slug] });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <section className="grid min-w-0 gap-2 rounded-card border border-line bg-paper p-3" data-tour="oracles.test">
    <div className="flex flex-wrap items-baseline gap-x-2"><b className="text-[13px]">Test</b><span className="text-[12px] text-ink-3">{draft ? "runs this draft on Home; records nothing" : "runs on Home; recorded as a test, commits no cursor"}</span></div>
    {params.map((p) => <label key={p.name} className="grid gap-1 text-[12px] font-semibold text-ink-2">{p.name}
      <input className="field-input font-mono text-[12px] font-normal" value={values[p.name] ?? ""} placeholder={p.default == null ? `${p.kind} · required` : String(p.default)} onChange={(e) => setValues({ ...values, [p.name]: e.target.value })} />
    </label>)}
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Cursor<input className="field-input font-mono text-[12px] font-normal" value={cursor} placeholder="none (as a first run)" onChange={(e) => setCursor(e.target.value)} /></label>
    <div className="flex items-center gap-2"><button className="btn btn-sm" disabled={busy || !text.trim()} onClick={() => void run()}><Icon name="play" size={12} />{busy ? "Testing…" : "Test"}</button><span className="text-[12px] text-ink-3">uv run on Home</span></div>
    {result && <TestResult r={result} />}
  </section>;
}

/** The answer of a test: outcome, duration, summary, fingerprint, proposed cursor and, when it failed, the class
 * and the end of stderr. */
export function TestResult({ r }: { r: OracleRunResult }) {
  const tone = r.outcome === "true" ? "border-gh-open/40 bg-diff-add" : r.outcome === "failed" ? "border-danger/40 bg-diff-del" : "border-line bg-sunk";
  return <div className={`grid min-w-0 gap-1 rounded-card border p-2.5 text-[12px] ${tone}`} role="status">
    <span className="flex items-center gap-2"><StatusBadge tone={OUTCOME_TONE[r.outcome]} label={OUTCOME_LABEL[r.outcome]} /><span className="text-ink-3">{r.duration_ms} ms</span>{r.error && <Badge tone="danger">{r.error.class}</Badge>}</span>
    {(r.error?.message || r.summary) && <span className="text-[13px] break-words">{r.error?.message ?? r.summary}</span>}
    <span className="font-mono text-[11px] break-all text-ink-3">fingerprint {r.fingerprint ?? "—"} · cursor {r.cursor == null ? "—" : JSON.stringify(r.cursor)}</span>
    {r.links.map((l) => <a key={l} href={l} target="_blank" rel="noreferrer" className="truncate">{l}</a>)}
    {r.stderr_tail.trim() && <pre className="m-0 max-h-[160px] overflow-auto rounded-control bg-paper p-2 font-mono text-[11px] whitespace-pre-wrap text-ink-2">{r.stderr_tail.trim()}</pre>}
    {r.stderr_tail.includes("[redacted]") && <span className="text-ink-3">A secret of this oracle was hidden as [redacted].</span>}
  </div>;
}

/** What it reaches: the workspace's tools (each with the hosts that receive its token), other hosts and Launchpad
 * read. A switch rewrites the file's `access = { … }` line; widening is highlighted before saving. */
function AccessTab({ text, onText, saved, tools, users, change }: { text: string; onText: (t: string) => void; saved: OracleAccess | null; tools: OracleTool[]; users: number; change: { added: string[]; removed: string[] } }) {
  const phone = usePhone();
  const access: OracleAccess = accessOf(text) ?? { tools: [], hosts: [], launchpad: "none" };
  const [host, setHost] = useState("");
  const editable = withAccess(text, access) != null;
  const set = (a: OracleAccess) => { const next = withAccess(text, a); if (next != null) onText(next); else toast("Edit the access in the file: it is not written as one access = { … } line"); };
  const state = (on: boolean, was: boolean) => (on === was ? (on ? "on" : "off") : on ? "added" : "removed");
  const toolRows = [...tools, ...access.tools.filter((id) => !tools.some((t) => t.id === id)).map((id) => ({ id, name: id, env: [], hosts: [], missing: true }))];
  const row = (key: string, label: ReactNode, on: boolean, was: boolean, toggle: () => void, extra?: ReactNode) => {
    const s = state(on, was);
    return <label key={key} className="grid min-h-9 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 text-[13px]">
      <input type="checkbox" className="accent-teal" checked={on} disabled={!editable} onChange={toggle} />
      <span className="min-w-0">{label}{extra}</span>
      <span className={`text-[12px] ${s === "added" ? "font-semibold text-attn" : "text-ink-3"}`}>{s}</span>
    </label>;
  };
  return <div className={`grid min-w-0 gap-4 ${phone ? "grid-cols-[minmax(0,1fr)]" : "grid-cols-[minmax(0,1fr)_340px] items-start"}`}>
    <div className="grid min-w-0 gap-2" data-tour="oracles.access">
      <b className="text-[13px]">What this oracle can reach</b>
      <p className="m-0 text-[12px] text-ink-3">Nothing until you add it. Each tool listed here puts its real token in the oracle's environment; <code className="font-mono">ctx.http</code> calls only the allowed hosts.</p>
      <span className="seclabel">Tools</span>
      {toolRows.length ? toolRows.map((t) => row(`tool-${t.id}`, <>{t.name}<span className="text-ink-3">{t.hosts.length ? ` · ${t.hosts.join(", ")}` : ""}{t.env.length ? ` · ${t.env.join(", ")}` : ""}{"missing" in t ? " · not granted to this workspace" : ""}</span></>, access.tools.includes(t.id), !!saved?.tools.includes(t.id), () => set({ ...access, tools: access.tools.includes(t.id) ? access.tools.filter((x) => x !== t.id) : [...access.tools, t.id] })))
        : <p className="m-0 text-[12px] text-ink-3">No tool is granted to this workspace (Settings → Credentials and tools).</p>}
      <span className="seclabel">Other access</span>
      {access.hosts.map((h) => row(`host-${h}`, <span className="font-mono text-[12px]">{h === "*" ? "* (every host)" : h}</span>, true, !!saved?.hosts.includes(h), () => set({ ...access, hosts: access.hosts.filter((x) => x !== h) })))}
      {editable && <div className="flex flex-wrap items-center gap-2">
        <input className="field-input w-[220px] font-mono text-[12px]" value={host} placeholder="api.example.com, or *" onChange={(e) => setHost(e.target.value.trim().toLowerCase())} />
        <button className="btn btn-sm" disabled={!host || access.hosts.includes(host)} onClick={() => { set({ ...access, hosts: [...access.hosts, host] }); setHost(""); }}><Icon name="plus" size={12} />Host</button>
      </div>}
      {row("launchpad", "Launchpad state (read, this workspace)", access.launchpad === "read", saved?.launchpad === "read", () => set({ ...access, launchpad: access.launchpad === "read" ? "none" : "read" }))}
      {!editable && <p className="m-0 text-[12px] text-ink-3">This file writes its access another way: edit it in Definition.</p>}
    </div>
    <div className="grid min-w-0 gap-3">
      {change.added.length > 0 ? <AccessNotice change={change} users={users} /> : change.removed.length > 0 && <div className="grid gap-1 rounded-card border border-line px-3 py-2 text-[12px] text-ink-2"><b className="text-[13px] text-ink">Saving narrows access</b>{change.removed.map((a) => <span key={a}>− {a}</span>)}</div>}
      <div className="grid gap-1 rounded-card border border-line px-3 py-2 text-[12px]">
        <b className="text-[13px]">Environment · what the process gets</b>
        <span className="font-mono text-[11px] text-ink-2">{access.tools.length ? `${access.tools.map((id) => `the ${tools.find((t) => t.id === id)?.name ?? id} token, under ${tools.find((t) => t.id === id)?.env.join(", ") || "its variable"}`).join("; ")}` : "no token"}{access.launchpad === "read" ? "; LAUNCHPAD_URL, LAUNCHPAD_TOKEN (read only, this workspace, this evaluation)" : ""} · nothing else from Home</span>
      </div>
    </div>
  </div>;
}

function DependenciesTab({ detail, slug }: { detail: OracleDetail | null; slug: string }) {
  const phone = usePhone();
  const meta = detail?.oracle.meta ?? null;
  const last = detail?.last;
  return <div className={`grid min-w-0 gap-4 ${phone ? "grid-cols-[minmax(0,1fr)]" : "grid-cols-[minmax(0,1fr)_340px] items-start"}`}>
    <div className="grid min-w-0 gap-3" data-tour="oracles.deps">
      <div className="grid gap-1"><b className="text-[12px] text-ink-2">Python</b><code className="rounded-control bg-sunk px-2 py-1.5 font-mono text-[12px]">requires-python = {JSON.stringify(meta?.requires_python ?? "any")}</code></div>
      <div className="grid gap-1"><b className="text-[12px] text-ink-2">Dependencies <span className="font-normal text-ink-3">· inline in the file (PEP 723), no lock</span></b>
        {meta?.dependencies.length ? meta.dependencies.map((d) => <code key={d} className="rounded-control border border-line px-2 py-1.5 font-mono text-[12px]">{d}</code>) : <span className="text-[12px] text-ink-3">None. Add libraries to <code className="font-mono">dependencies = [ … ]</code> in the file (Definition).</span>}
      </div>
      <div className="grid gap-1"><b className="text-[12px] text-ink-2">Added by Home on every run</b><code className="rounded-control bg-sunk px-2 py-1.5 font-mono text-[12px]">launchpad_oracle (bundled with Home) · httpx2</code></div>
    </div>
    <div className="grid min-w-0 gap-3">
      <div className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1.5 rounded-card border border-line px-3 py-2 text-[12px]">
        <b className="col-span-2 text-[13px]">How it runs</b>
        <span className="text-ink-3">Command</span><code className="font-mono text-[11px] break-all">uv run --with launchpad_oracle --script oracles/{slug || "<slug>"}.py</code>
        <span className="text-ink-3">Environment</span><span>created and cached by uv; rebuilt by uv after a restart</span>
        <span className="text-ink-3">Last run</span><span>{last ? <><StatusBadge tone={OUTCOME_TONE[last.outcome]} label={OUTCOME_LABEL[last.outcome]} /> {ago(last.created_at)}</> : "never"}</span>
      </div>
      <div className="grid gap-1 rounded-card border border-attn-line bg-attn-soft px-3 py-2 text-[12px] text-ink-2"><b className="text-[13px] text-ink">Dependency updates are yours</b>Launchpad keeps no lock and does not manage updates. If a new release breaks this oracle, it shows Failed in Today. Pin a version in the file if you need one.</div>
    </div>
  </div>;
}

/** Who asked and what came of it: "Routine “Fix red CI” · fired → run". */
function callerLine(e: OracleEvaluation): string {
  return e.callers.map((c) => {
    const who = c.caller.kind === "routine" ? `Routine “${c.caller.name ?? "?"}”` : c.caller.kind === "cli" ? `Agent “${c.caller.name ?? "?"}”` : c.caller.kind === "test" ? "Test now" : c.caller.kind === "owner" ? "You" : c.caller.name ?? c.caller.kind;
    return [who, c.how !== "started" ? c.how : null, c.consequence, c.detail && c.consequence !== "returned, cursor committed" ? c.detail : null].filter(Boolean).join(" · ");
  }).join("; ");
}

/** A quiet evaluation: True or False, and no caller acted on it (quiet, returned or no consequence). */
const quiet = (e: OracleEvaluation) => e.outcome !== "failed" && e.callers.every((c) => !c.consequence || c.consequence === "quiet" || c.consequence === "counted");

type HistoryEntry = { e: OracleEvaluation; more: OracleEvaluation[] };

/** Pure: consecutive quiet evaluations with the same outcome and fingerprint collapse into one entry. */
export function collapse(list: OracleEvaluation[]): HistoryEntry[] {
  const out: HistoryEntry[] = [];
  for (const e of list) {
    const prev = out.at(-1);
    if (prev && quiet(e) && quiet(prev.e) && prev.e.outcome === e.outcome && prev.e.fingerprint === e.fingerprint) prev.more.push(e);
    else out.push({ e, more: [] });
  }
  return out;
}

function HistoryTab({ workspace, detail }: { workspace: string; detail: OracleDetail }) {
  const [failedOnly, setFailedOnly] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const list = useOracleEvaluations(workspace, detail.oracle.slug, failedOnly);
  const evals = useMemo(() => list.data ?? [], [list.data]);
  const entries = useMemo(() => collapse(evals), [evals]);
  const recent = evals.slice(0, 24);
  const failedCount = recent.filter((e) => e.outcome === "failed").length;
  const lastGood = evals.find((e) => e.outcome !== "failed");
  return <div className="grid min-w-0 gap-3" data-tour="oracles.history">
    <div className="grid gap-1">
      <OutcomeSquares recent={recent.map((e) => ({ outcome: e.outcome, at: e.created_at }))} size={12} />
      <span className="text-[12px] text-ink-3">Last {recent.length} evaluations · {failedCount} Failed{lastGood ? ` · last good ${ago(lastGood.created_at)}` : ""}</span>
    </div>
    <FilterChips label="History">
      <FilterChip active={!failedOnly} onClick={() => setFailedOnly(false)}>All</FilterChip>
      <FilterChip active={failedOnly} onClick={() => setFailedOnly(true)}>Failed</FilterChip>
    </FilterChips>
    {list.isLoading ? <Loading what="evaluations" /> : list.error ? <ErrorState error={list.error} /> : !entries.length ? <Empty>{failedOnly ? "No failed evaluation." : "No evaluation yet: Test it, or gate a routine on it."}</Empty>
      : <div className="grid grid-cols-[minmax(0,1fr)] overflow-hidden rounded-card border border-line">
        {entries.map(({ e, more }) => {
          const expanded = open === e.id;
          const what = e.outcome === "failed" && e.error ? `${e.error.class}: ${e.error.message}` : more.length ? `${more.length + 1} checks since ${when(more.at(-1)!.created_at)}` : e.summary;
          return <div key={e.id} className="grid min-w-0 gap-1 border-b border-line px-3 py-2 last:border-b-0">
            <button className="flex min-w-0 items-center gap-2 text-left" aria-expanded={expanded} onClick={() => setOpen(expanded ? null : e.id)}>
              <span className={`size-2 flex-none rounded-full ${OUTCOME_SQUARE[e.outcome]}`} aria-hidden="true" />
              <b className="min-w-0 flex-1 truncate text-[13px]">{when(e.created_at)} · {OUTCOME_LABEL[e.outcome]}{what ? ` · ${what}` : ""}</b>
            </button>
            <span className="truncate pl-4 text-[12px] text-ink-3">{more.length ? `collapsed · ${callerLine(e) || "no caller"}` : `${callerLine(e) || "no caller"} · ${e.duration_ms} ms${e.reused ? " · reused" : ""}`}</span>
            {expanded && <div className="grid gap-1 pl-4 text-[12px]">
              {more.length > 0 && more.map((m) => <span key={m.id} className="text-ink-3">{when(m.created_at)} · {OUTCOME_LABEL[m.outcome]} · {callerLine(m)}</span>)}
              {e.summary && <span>{e.summary}</span>}
              <span className="font-mono text-[11px] break-all text-ink-3">params {JSON.stringify(e.params)} · fingerprint {e.fingerprint ?? "—"} · cursor in {e.cursor_in == null ? "—" : JSON.stringify(e.cursor_in)} · proposed {e.cursor_proposed == null ? "—" : JSON.stringify(e.cursor_proposed)}</span>
              {e.links.map((l) => <a key={l} href={l} target="_blank" rel="noreferrer" className="truncate">{l}</a>)}
              {e.stderr_tail.trim() && <pre className="m-0 max-h-[160px] overflow-auto rounded-control bg-sunk p-2 font-mono text-[11px] whitespace-pre-wrap">{e.stderr_tail.trim()}</pre>}
              {e.stderr_tail.includes("[redacted]") && <span className="text-ink-3">A secret of this oracle was hidden as [redacted].</span>}
            </div>}
          </div>;
        })}
      </div>}
  </div>;
}

function UsedByTab({ detail }: { detail: OracleDetail }) {
  const bots = useBots().data ?? [];
  if (!detail.used_by.length) return <Empty>No routine gates on this oracle yet. In a routine's Edit, “Only when an oracle says” picks it.</Empty>;
  return <div className="grid grid-cols-[minmax(0,1fr)] overflow-hidden rounded-card border border-line" data-tour="oracles.used">
    {detail.used_by.map((u) => {
      const bot = bots.find((b) => b.id === u.bot_id);
      return <div key={u.routine_id} className="grid min-h-[52px] grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 border-b border-line px-3 py-2 last:border-b-0">
        <span className="min-w-0">
          <span className="block truncate text-[13px] font-semibold">Routine{bot ? ` · ${bot.settings.name}` : ""} · “{u.title}”{u.paused && <span className="font-normal text-ink-3"> · paused</span>}</span>
          <span className="block truncate text-[12px] text-ink-3">runs on {u.run_on.map((o) => OUTCOME_LABEL[o]).join(", ") || "nothing"} · fires {u.fire === "every" ? "every time" : u.fire === "change" ? "when it changes" : "(no fire policy)"}</span>
        </span>
        <Link className="btn btn-sm no-underline" to="/schedules" search={{ routine: u.routine_id }}>Open</Link>
      </div>;
    })}
  </div>;
}

function VersionsTab({ workspace, slug }: { workspace: string; slug: string }) {
  const versions = useQuery({ queryKey: ["oracle", workspace, slug, "versions"], queryFn: () => api.get<Change[]>(`/api/oracles/${encodeURIComponent(workspace)}/${encodeURIComponent(slug)}/versions`) });
  if (versions.isLoading) return <Loading what="versions" />;
  if (versions.error) return <ErrorState error={versions.error} />;
  if (!versions.data?.length) return <Empty>No change yet.</Empty>;
  return <div className="grid min-w-0 gap-2" data-tour="oracles.versions">
    <span className="text-[12px] text-ink-3">Every version of oracles/{slug}.py for 30 days; Restore puts one back, and is itself a version.</span>
    {versions.data.map((c) => <OracleVersion key={c.id} workspace={workspace} slug={slug} change={c} />)}
  </div>;
}

function OracleVersion({ workspace, slug, change }: { workspace: string; slug: string; change: Change }) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const base = `/api/oracles/${encodeURIComponent(workspace)}/${encodeURIComponent(slug)}`;
  const diff = useQuery({ queryKey: ["oracle", workspace, slug, "diff", change.id], queryFn: () => api.get<{ patch: string }>(`${base}/versions/${change.id}`), enabled: open, staleTime: Infinity });
  return <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1">
    <div className="flex min-w-0 items-center gap-2 text-[12px]">
      <button className="min-w-0 flex-1 truncate text-left hover:underline" aria-expanded={open} onClick={() => setOpen(!open)}>{change.author?.name ?? "You"} · {ago(change.at)}{change.added != null && <span className="text-ink-3"> · +{change.added} −{change.removed ?? 0}</span>}{!change.sha256 && <span className="text-ink-3"> · created</span>}</button>
      {change.sha256 && <span className="flex-none"><ConfirmButton icon="refresh" tone="publish" consequence={`Puts back oracles/${slug}.py as it was before this change. The restore is itself a version.`} confirmLabel="Restore" onConfirm={async () => { try { await runAction("oracle.restore", { workspace, slug, change: String(change.id) }); await Promise.all([qc.invalidateQueries({ queryKey: ["oracle", workspace, slug] }), qc.invalidateQueries({ queryKey: ["oracles"] })]); toast("Restored"); } catch (e) { toastError(e); } }}>Restore previous</ConfirmButton></span>}
    </div>
    {open && (diff.isLoading ? <Loading what="the diff" /> : diff.data?.patch ? <PatchView patch={diff.data.patch} path={`oracles/${slug}.py`} language="python" split={false} wrap /> : <p className="m-0 text-[12px] text-ink-3">No text difference.</p>)}
  </div>;
}

// ---------------------------------------------------------------- routine gate

const DURATION = /^(\d+(ms|s|m|h|d)?)?$/;

/** Pure: whether a gate can be saved (Home's `gates::check`): an oracle, at least one outcome, a fire policy and
 * durations it reads. Null: valid. */
export function gateProblem(g: Gate | null | undefined): string | null {
  if (!g) return null;
  if (!g.oracle) return "Pick an oracle.";
  if (!g.run_on.length) return "Pick at least one outcome that starts a run.";
  if (!g.fire) return "Choose when a run fires: when it changes, or every time.";
  if (!DURATION.test((g.cooldown ?? "").trim()) || !DURATION.test((g.remind ?? "").trim())) return "Write durations like 30m, 6h or 1d.";
  return null;
}

/** "Only when an oracle says": the gate fields of a schedule routine (routine editor). `gate` null: no gate. */
export function GateFields({ workspace, gate, onChange }: { workspace: string | null; gate: Gate | null; onChange: (g: Gate | null) => void }) {
  const oracles = useOracles(workspace).data ?? [];
  const picked = oracles.find((o) => o.oracle.slug === gate?.oracle);
  const params = picked?.oracle.meta?.params ?? [];
  const problem = gateProblem(gate);
  const set = (p: Partial<Gate>) => gate && onChange({ ...gate, ...p });
  const toggle = (o: OracleOutcome) => gate && set({ run_on: gate.run_on.includes(o) ? gate.run_on.filter((x) => x !== o) : [...gate.run_on, o] });
  const OUT: { o: OracleOutcome; on: string; off: string }[] = [
    { o: "true", on: "Start a run, with the routine's instructions and the evidence", off: "Stay silent: recorded as a check; no thread, no quota" },
    { o: "false", on: "Start a run, with the routine's instructions and the evidence", off: "Stay silent: recorded as a check; no thread, no quota" },
    { o: "failed", on: "Repair run: once per failure streak, with its own instructions; always also the Today row", off: "Stay silent; the oracle's Today row says it fails" },
  ];
  return <div className="grid gap-2 rounded-card border border-line p-2.5" data-tour="routines.gate">
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Only when an oracle says
      <select className="field-input font-normal" value={gate?.oracle ?? ""} onChange={(e) => onChange(e.target.value ? { ...(gate ?? { run_on: ["true"], fire: null }), oracle: e.target.value, params: {} } : null)}>
        <option value="">Always (no oracle)</option>
        {oracles.map((o) => <option key={o.oracle.slug} value={o.oracle.slug}>{o.oracle.meta?.title ?? o.oracle.slug} · {o.oracle.slug}{o.oracle.paused ? " (paused)" : ""}</option>)}
        {gate?.oracle && !picked && <option value={gate.oracle}>{gate.oracle}</option>}
      </select>
    </label>
    {!gate && !oracles.length && <span className="text-[12px] text-ink-3">No oracle in this workspace yet (Schedules → Oracles).</span>}
    {gate && <>
      {params.length > 0 && <div className="grid gap-1"><span className="text-[12px] font-semibold text-ink-2">with</span>
        {params.map((p) => <label key={p.name} className="grid grid-cols-[120px_minmax(0,1fr)] items-center gap-2 text-[12px] text-ink-2">{p.name}
          <input className="field-input font-mono text-[12px]" value={String((gate.params ?? {})[p.name] ?? "")} placeholder={p.default == null ? `${p.kind} · required` : String(p.default)} onChange={(e) => { const next = { ...(gate.params ?? {}) }; if (e.target.value) next[p.name] = e.target.value; else delete next[p.name]; set({ params: next }); }} />
        </label>)}
      </div>}
      <div className="grid gap-1"><span className="text-[12px] font-semibold text-ink-2">Outcomes</span>
        {OUT.map(({ o, on, off }) => <label key={o} className="grid min-h-9 grid-cols-[auto_64px_minmax(0,1fr)] items-center gap-2 text-[12px]">
          <input type="checkbox" className="accent-teal" checked={gate.run_on.includes(o)} onChange={() => toggle(o)} />
          <StatusBadge tone={OUTCOME_TONE[o]} label={OUTCOME_LABEL[o]} />
          <span className={gate.run_on.includes(o) ? "text-ink" : "text-ink-3"}>{gate.run_on.includes(o) ? on : off}</span>
        </label>)}
      </div>
      <div className="grid gap-1"><span className="text-[12px] font-semibold text-ink-2">Fire <span className="font-normal text-ink-3">· required, no default</span></span>
        <Choices label="Fire">
          <Choice active={gate.fire === "change"} onClick={() => set({ fire: "change" })} title="A new outcome, or the same True with a new fingerprint">When it changes</Choice>
          <Choice active={gate.fire === "every"} onClick={() => set({ fire: "every" })} title="Every routed answer starts a run">Every time</Choice>
        </Choices>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Cooldown<input className="field-input font-normal" value={gate.cooldown ?? ""} placeholder="none, or 6h" onChange={(e) => set({ cooldown: e.target.value.trim() || undefined })} /></label>
        <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Remind<input className="field-input font-normal" value={gate.remind ?? ""} placeholder="none, or 24h" onChange={(e) => set({ remind: e.target.value.trim() || undefined })} /></label>
      </div>
      {gate.run_on.includes("failed") && <label className="grid gap-1 text-[12px] font-semibold text-ink-2">When it fails, the run does
        <textarea className="field-input min-h-[60px] py-1.5 font-normal leading-5" value={gate.failed?.instructions ?? ""} placeholder="The oracle could not decide. Diagnose why and fix it." onChange={(e) => set({ failed: e.target.value.trim() ? { instructions: e.target.value } : undefined })} />
      </label>}
      {problem && <p role="alert" className="m-0 text-[12px] text-gh-closed">{problem}</p>}
      <p className="m-0 text-[12px] text-ink-3">Each due time evaluates the oracle first. Outcomes left off are quiet checks: no thread, no session, no quota.</p>
    </>}
  </div>;
}


/** A Today card's details of a failing oracle: the error, first and last failure, what waits on it. */
export function OracleFailure({ workspace, slug }: { workspace: string; slug: string }) {
  const d = useOracle(workspace, slug).data;
  const bots = useBots().data ?? [];
  const s = d?.streak;
  if (!d || !s) return null;
  const classes = Object.entries(s.classes).sort((a, b) => b[1] - a[1]);
  return <div className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-[12px]">
    <span className="text-ink-3">Error</span><span className="break-words">{s.last_error ? <><b>{s.last_error.class}</b> · {s.last_error.message}</> : "—"}{classes.length > 0 && <span className="text-ink-3"> ({classes.map(([c, n]) => `${n} ${c}`).join(", ")} of {s.count})</span>}</span>
    <span className="text-ink-3">First / last</span><span>{s.first_at ? when(s.first_at) : "—"} / {s.last_at ? when(s.last_at) : "—"}</span>
    <span className="text-ink-3">Waiting on it</span><span>{d.used_by.length ? d.used_by.map((u) => `${bots.find((b) => b.id === u.bot_id)?.settings.name ?? "Routine"} “${u.title}”${u.run_on.includes("failed") ? " (repair run on Failed)" : " (quiet while it fails)"}`).join(", ") : "nothing"}</span>
    {s.last_good_at && <><span className="text-ink-3">Last good</span><span>{ago(s.last_good_at)}</span></>}
  </div>;
}
