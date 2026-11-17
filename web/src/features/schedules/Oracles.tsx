// Schedules → Oracles: the workspace's oracles (small Python checks Home runs in Monty to answer True, False or
// Failed), one oracle's page (Definition with Test, Tools, History, Used by, Versions) and the New oracle form.
// Routines gate on them (`GateFields` in the routine editor).

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Fragment, memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { ActionButton, ActionMenuItem } from "@/components/ActionButton";
import { Badge, Choice, Choices, ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, MenuSection, MoreMenu, PageHeader, RowActions, StatusBadge, Tabs, Token } from "@/components/ui";
import { PatchView } from "@/components/diff/DiffReview";
import { highlight, type Tokens } from "@/features/code/highlight";
import { MAX_HIGHLIGHT_LINE } from "@/features/code/limits";
import type { Change } from "@/gen/Change";
import type { Gate } from "@/gen/Gate";
import type { OracleCall } from "@/gen/OracleCall";
import type { OracleDetail } from "@/gen/OracleDetail";
import type { OracleError } from "@/gen/OracleError";
import type { OracleEvaluation } from "@/gen/OracleEvaluation";
import type { OracleListItem } from "@/gen/OracleListItem";
import type { OracleOutcome } from "@/gen/OracleOutcome";
import type { OracleParam } from "@/gen/OracleParam";
import type { OracleProblem } from "@/gen/OracleProblem";
import type { OracleRunResult } from "@/gen/OracleRunResult";
import type { OracleSaved } from "@/gen/OracleSaved";
import type { OracleTestAnswer } from "@/gen/OracleTestAnswer";
import type { OracleTool } from "@/gen/OracleTool";
import type { OracleToolInfo } from "@/gen/OracleToolInfo";
import { api } from "@/lib/api";
import { runAction } from "@/lib/actions";
import { useBots } from "@/lib/bots";
import { daemonUrl } from "@/lib/connection";
import { ago, when } from "@/lib/format";
import { useKeptDraft, versionOf } from "@/lib/keptDraft";
import { CREDENTIALED, LEAK_WARNING, ORACLE_TOOLS, OUTCOME_LABEL, OUTCOME_SQUARE, OUTCOME_TONE, STARTER, broken, canLeak, failing, gateSummary, paramLabel, paramValue, plural, problemText, problemsOf, pyValue, signatureOf, toolsChange, toolsOf, useOracle, useOracleEvaluations, useOracleInterface, useOracleTools, useOracles, withTools } from "@/lib/oracles";
import { usePhone } from "@/lib/phone";
import { useTheme } from "@/stores/theme";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { resultLine } from "./OracleFailure";

export { gateSummary };

/** `?oracle=` of the New oracle form (never a valid slug). */
export const NEW_ORACLE = "_new";

export type OracleView = "definition" | "tools" | "history" | "used" | "versions";
const VIEWS: OracleView[] = ["definition", "tools", "history", "used", "versions"];
export const oracleView = (v: unknown): OracleView => (VIEWS.includes(v as OracleView) ? (v as OracleView) : "definition");

type Filter = "all" | "failing" | "unused";

/** Needs a look: failing (an open streak) or broken (the stored file does not pass the pre-check). */
const bad = (i: OracleListItem) => broken(i.oracle) || failing(i.streak);

// ---------------------------------------------------------------- list

/** The oracles of the workspace, with All / Failing (broken ones too) / Unused. */
export function OracleList({ workspace, onOpen }: { workspace: string; onOpen: (slug: string) => void }) {
  const list = useOracles(workspace);
  const [filter, setFilter] = useState<Filter>("all");
  const items = list.data ?? [];
  const failingCount = items.filter(bad).length;
  const unused = items.filter((i) => !i.used_by.length).length;
  const shown = items.filter((i) => filter === "all" || (filter === "failing" ? bad(i) : !i.used_by.length));
  if (list.isLoading) return <Loading what="oracles" />;
  if (list.error) return <ErrorState error={list.error} onRetry={() => void list.refetch()} />;
  if (!items.length) return <Empty icon="spark">No oracle yet. An oracle is a small Python check that Home runs to answer True, False or Failed; a routine can run only when one says so. Ask Launchpad to write one, or create it with New oracle.</Empty>;
  return <>
    <FilterChips label="Oracles" tour="oracles.filters">
      <FilterChip active={filter === "all"} count={items.length} onClick={() => setFilter("all")}>All</FilterChip>
      <FilterChip active={filter === "failing"} count={failingCount} countNeeds={failingCount > 0} title="Failing or broken" onClick={() => setFilter("failing")}>Failing</FilterChip>
      <FilterChip active={filter === "unused"} count={unused} onClick={() => setFilter("unused")}>Unused</FilterChip>
    </FilterChips>
    {shown.length ? <div className="grid grid-cols-[minmax(0,1fr)] overflow-hidden rounded-card border border-line" data-tour="oracles.list">
      {shown.map((i) => <OracleRow key={i.oracle.slug} item={i} workspace={workspace} onOpen={onOpen} />)}
    </div> : <Empty>{filter === "failing" ? "No oracle is failing or broken." : "Every oracle is used by a routine."}</Empty>}
  </>;
}

/** Recent outcomes as squares, oldest first. */
export function OutcomeSquares({ recent, size = 9 }: { recent: { outcome: OracleOutcome; at: number }[]; size?: number }) {
  if (!recent.length) return null;
  return <span className="flex items-center gap-1" aria-label="Recent evaluations">
    {[...recent].reverse().map((r, i) => <span key={`${r.at}-${i}`} style={{ width: size, height: size }} className={`rounded-xs ${OUTCOME_SQUARE[r.outcome]}`} title={`${when(r.at)} · ${OUTCOME_LABEL[r.outcome]}`} />)}
  </span>;
}

/** One line about the last answer: "True 6 min ago · tests.yml on 4be1c2a: failure", the failure streak, or why the
 * file is broken. */
function lastLine(i: OracleListItem): { text: string; bad: boolean } {
  if (broken(i.oracle)) return { text: `Broken, not evaluated until saved again · ${problemText(i.oracle.problems[0])}`, bad: true };
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

/** The tools as chips; a credentialed tool next to http is amber (it can send what it reads anywhere). */
function ToolChips({ tools }: { tools: readonly OracleTool[] }) {
  const leak = canLeak(tools);
  if (!tools.length) return <span className="text-ink-3">no tools</span>;
  return <>{tools.map((t) => <Token key={t} tone={leak && (t === "http" || CREDENTIALED.includes(t)) ? "warn" : "neutral"} title={leak ? `With http and a credentialed tool, this oracle ${LEAK_WARNING}` : undefined}>{t}</Token>)}</>;
}

const OracleRow = memo(function OracleRow({ item, workspace, onOpen }: { item: OracleListItem; workspace: string; onOpen: (slug: string) => void }) {
  const bots = useBots().data ?? [];
  const phone = usePhone();
  const o = item.oracle;
  const isBroken = broken(o);
  const last = lastLine(item);
  const clamp = phone ? "line-clamp-2 break-words" : "truncate";
  const tone = o.paused ? "bg-ink-3/40" : isBroken ? "bg-gh-closed" : item.last ? OUTCOME_SQUARE[failing(item.streak) ? "failed" : item.last.outcome] : "bg-line";
  return <div className={`lp-row group grid min-h-[60px] w-full grid-cols-[12px_minmax(0,1fr)_auto] items-center gap-x-3 border-b border-line px-3 py-2 last:border-b-0 hover:bg-sunk ${isBroken && !o.paused ? "bg-diff-del/40" : "bg-paper"}`}>
    <span className={`size-2 rounded-full ${tone}`} aria-hidden="true" />
    <button className="min-w-0 text-left" onClick={() => onOpen(o.slug)}>
      <span className={`flex min-w-0 items-center gap-1.5 text-[13px] font-semibold`}><span className={`min-w-0 ${clamp}`}>{o.meta?.title ?? o.slug}<span className="font-normal text-ink-3"> · {o.slug}</span>{o.paused && <span className="font-normal text-ink-3"> · paused</span>}</span>{isBroken && <Badge tone="danger" icon="alert">Broken</Badge>}</span>
      <span className={`block text-[12px] ${clamp} ${last.bad && !o.paused ? "text-gh-closed" : "text-ink-3"}`} title={last.text}>{last.text}</span>
      <span className="flex min-w-0 flex-wrap items-center gap-1 text-[12px] text-ink-3"><span className="mr-1">{usedBy(item, bots)}</span><ToolChips tools={o.meta?.tools ?? []} /></span>
    </button>
    <span className="flex items-center gap-3">
      <span className="hidden sm:flex"><OutcomeSquares recent={item.recent} /></span>
      <RowActions width={phone ? 0 : undefined} label={`More actions for ${o.meta?.title ?? o.slug}`} main={phone ? undefined : <button className="btn btn-sm" onClick={() => onOpen(o.slug)}>Open</button>} more={(close) => <>
        {!isBroken && <ActionMenuItem name="oracle.test" params={{ workspace, slug: o.slug }} body={{}} close={close} detail="With the parameters of its last evaluation" onDone={(a: OracleTestAnswer) => toast(resultLine(a.result))} />}
        {o.paused ? <ActionMenuItem name="oracle.resume" params={{ workspace, slug: o.slug }} close={close} /> : <ActionMenuItem name="oracle.pause" params={{ workspace, slug: o.slug }} close={close} />}
      </>} />
    </span>
  </div>;
});

// ---------------------------------------------------------------- one oracle

/** What the last save answered, shown above the page until the next one (the editor remounts on each save). */
type SaveNote = { slug: string; saved: OracleSaved };

/** One oracle's page, or the New oracle form (`slug` = NEW_ORACLE). Its tab is `view` (the address's `?view=`). */
export function OraclePage({ workspace, slug, view, onView, onBack, onSaved, onDeleted }: { workspace: string; slug: string; view: OracleView; onView: (v: OracleView) => void; onBack: () => void; onSaved: (slug: string) => void; onDeleted: () => void }) {
  const isNew = slug === NEW_ORACLE;
  const detail = useOracle(workspace, isNew ? null : slug);
  const [note, setNote] = useState<SaveNote | null>(null);
  if (!isNew && detail.isLoading) return <Loading what="the oracle" />;
  if (!isNew && (detail.error || !detail.data)) return <div className="px-5 py-4"><ErrorState error={detail.error ?? "This oracle no longer exists"} action={<button className="btn btn-sm" onClick={onBack}>Back to oracles</button>} /></div>;
  return <OracleEditor key={isNew ? NEW_ORACLE : `${slug}-${detail.data!.oracle.sha256}`} workspace={workspace} detail={isNew ? null : detail.data!} view={view} onView={onView} onBack={onBack}
    note={note?.slug === slug ? note.saved : null} onDismissNote={() => setNote(null)}
    onSaved={(s, saved) => { setNote({ slug: s, saved }); onSaved(s); }} onDeleted={onDeleted} />;
}

/** Pre-check problems that refused this exact text (a save or a test): they block Save and Test until it changes. */
type Refused = { text: string; problems: OracleProblem[] };

function OracleEditor({ workspace, detail, view, onView, onBack, note, onDismissNote, onSaved, onDeleted }: { workspace: string; detail: OracleDetail | null; view: OracleView; onView: (v: OracleView) => void; onBack: () => void; note: OracleSaved | null; onDismissNote: () => void; onSaved: (slug: string, saved: OracleSaved) => void; onDeleted: () => void }) {
  const phone = usePhone();
  const qc = useQueryClient();
  const tools = useOracleTools(workspace).data;
  const saved = detail?.content ?? STARTER;
  const draft = useKeptDraft(`oracle.${workspace}.${detail?.oracle.slug ?? NEW_ORACLE}.${versionOf(saved)}`, saved);
  const [newSlug, setNewSlug] = useState("");
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<Refused | null>(null);
  // Type warnings of a draft, from its last test (the saved file's come with the detail).
  const [draftWarnings, setDraftWarnings] = useState<Refused | null>(null);
  const o = detail?.oracle ?? null;
  const meta = o?.meta ?? null;
  const slug = o?.slug ?? newSlug.trim();
  const dirty = !detail || draft.text !== saved;
  const errors = refused?.text === draft.text ? refused.problems : !dirty && o ? o.problems : [];
  const warnings = !dirty ? detail?.warnings ?? [] : draftWarnings?.text === draft.text ? draftWarnings.problems : [];
  const savedTools = meta?.tools ?? [];
  const draftTools = toolsOf(draft.text);
  const change = toolsChange(savedTools, draftTools ?? savedTools);
  const slugOk = /^[a-z0-9][a-z0-9_-]{0,62}$/.test(slug);
  const blocked = refused?.text === draft.text;
  const save = async () => {
    setBusy(true);
    const text = draft.text;
    try {
      const r = await runAction<OracleSaved>("oracle.save", { workspace, slug }, { content: text });
      draft.clear();
      await Promise.all([qc.invalidateQueries({ queryKey: ["oracles"] }), qc.invalidateQueries({ queryKey: ["oracle", workspace, slug] })]);
      toast(detail ? "Oracle saved" : `Oracle ${slug} created`);
      onSaved(slug, r);
    } catch (e) {
      const problems = problemsOf(e);
      if (problems) { setRefused({ text, problems }); if (view !== "definition") onView("definition"); } else toastError(e);
    } finally { setBusy(false); }
  };
  const users = detail?.used_by ?? [];
  const title = meta?.title ?? (detail ? o!.slug : "New oracle");
  const metaLine = detail ? [
    <code key="slug" className="rounded bg-sunk px-1 font-mono text-[11px]">{o!.slug}</code>,
    o!.changed_at ? `edited by ${!o!.changed_by || o!.changed_by.kind === "owner" ? "you" : o!.changed_by.name} ${ago(o!.changed_at)}` : null,
    meta ? (meta.tools.length ? `tools: ${meta.tools.join(", ")}` : "no tools") : null,
    broken(o) ? <span key="broken" className="font-semibold text-gh-closed">broken</span> : null,
    o!.paused ? "paused" : null,
  ].filter(Boolean) : null;
  const metaText = metaLine && <span>{metaLine.map((m, i) => <Fragment key={i}>{i > 0 && " · "}{m}</Fragment>)}</span>;
  const saveTitle = !slugOk ? "Give the oracle a slug: lowercase letters, digits, - and _" : blocked ? "Fix the errors of the pre-check first" : undefined;
  const saveButton = <button className="btn btn-sm btn-primary" disabled={busy || !dirty || !slugOk || blocked} title={saveTitle} onClick={() => void save()}><Icon name="check" size={12} />{busy ? "Saving…" : "Save"}</button>;
  // ⋯ of the page (laptop) and of the phone bar: the same entries.
  const menuItems = (close: () => void) => <>
    {draft.text !== saved && <MenuItem icon={<Icon name="x" size={14} />} label="Discard changes" detail="Back to the saved file" onClick={() => { close(); draft.setText(saved); }} />}
    {o!.paused ? <ActionMenuItem name="oracle.resume" params={{ workspace, slug: o!.slug }} close={close} onDone={() => qc.invalidateQueries({ queryKey: ["oracle", workspace, o!.slug] })} />
      : <ActionMenuItem name="oracle.pause" params={{ workspace, slug: o!.slug }} close={close} detail={users.length ? `Gated routines then record quiet checks: ${users.map((u) => `“${u.title}”`).join(", ")}` : "Nothing evaluates it until you resume it"} onDone={() => qc.invalidateQueries({ queryKey: ["oracle", workspace, o!.slug] })} />}
    <MenuSection><ActionButton name="oracle.delete" params={{ workspace, slug: o!.slug }} body={{ force: users.length > 0 }} consequence={users.length ? `Delete ${o!.slug}? Routines gated on it fail until you change their gate: ${users.map((u) => `“${u.title}”`).join(", ")}. Its file stays restorable 30 days.` : `Delete ${o!.slug} with its evaluations? Its file stays restorable 30 days.`} confirmLabel="Delete oracle" onDone={async () => { close(); await qc.invalidateQueries({ queryKey: ["oracles"] }); onDeleted(); }}>Delete</ActionButton></MenuSection>
  </>;
  const menu = detail && <MoreMenu label={`More actions for ${title}`} width={300}>{menuItems}</MoreMenu>;
  const tabs = <Tabs label="Oracle" value={view} onChange={onView} tour="oracles.tabs" items={[
    { id: "definition", label: "Definition" },
    { id: "tools", label: "Tools", count: (draftTools ?? savedTools).length || undefined },
    ...(detail ? [{ id: "history" as const, label: "History" }, { id: "used" as const, label: "Used by", count: users.length }, { id: "versions" as const, label: "Versions" }] : []),
  ]} />;
  const body = <div className="grid w-full max-w-page grid-cols-[minmax(0,1fr)] gap-3 px-5 py-4">
    {phone && tabs}
    {!detail && <label className="grid max-w-[360px] gap-1 text-[12px] font-semibold text-ink-2">Slug
      <input className="field-input font-mono text-[12px] font-normal" value={newSlug} placeholder="pypi-release" maxLength={63} onChange={(e) => setNewSlug(e.target.value.toLowerCase())} />
      <span className="font-normal text-ink-3">The file is saved as oracles/{slug || "<slug>"}.py; routines and agents name the oracle by it.</span>
    </label>}
    {broken(o) && <div className="grid gap-1 rounded-card border border-danger/40 bg-diff-del px-3 py-2 text-[12px]" role="alert">
      <b className="text-[13px] text-gh-closed">Broken: not evaluated until saved again</b>
      <span className="text-ink-2">The saved file does not pass the pre-check (an older format, or an error). Routines gated on it record quiet checks. Fix it in Definition, then Save.</span>
      {o!.problems.map((p, i) => <code key={i} className="font-mono text-[11px] break-words">{problemText(p)}</code>)}
    </div>}
    {note && <SavedNote saved={note} onDismiss={onDismissNote} />}
    {draft.kept && dirty && detail && <p className="m-0 text-[12px] text-ink-3">Draft kept: your unsaved changes are back.</p>}
    {(change.added.length > 0 || (canLeak(draftTools ?? []) && !canLeak(savedTools))) && view !== "tools" && <ToolsNotice change={change} tools={draftTools ?? []} users={users.length} />}
    {view === "definition" && <DefinitionTab workspace={workspace} detail={detail} text={draft.text} onText={draft.setText} slug={slug} errors={errors} warnings={warnings} blocked={blocked}
      onRefused={(text, problems) => setRefused({ text, problems })} onWarnings={(text, problems) => setDraftWarnings({ text, problems })} />}
    {view === "tools" && <ToolsTab text={draft.text} onText={draft.setText} saved={savedTools} infos={tools} users={users.length} change={change} />}
    {view === "history" && detail && <HistoryTab workspace={workspace} detail={detail} />}
    {view === "used" && detail && <UsedByTab detail={detail} />}
    {view === "versions" && detail && <VersionsTab workspace={workspace} slug={detail.oracle.slug} />}
  </div>;
  if (phone) return <div className="flex h-full min-h-0 flex-col">
    <PhoneHeader title={title} onBack={onBack} backLabel="Back to oracles" subtitle={metaText} primary={dirty ? saveButton : undefined} actions={detail ? menuItems : undefined} />
    <div className="min-h-0 flex-1 overflow-y-auto">{body}</div>
  </div>;
  return <div className="flex h-full min-h-0 flex-col">
    <PageHeader title={title} meta={metaText ?? "A Python check that Home runs to answer True, False or Failed"} actions={<>{saveButton}{menu}</>} tabs={tabs} />
    <div className="min-h-0 flex-1 overflow-y-auto">{body}</div>
  </div>;
}

/** What the last save said: the tools it added or removed, the credentialed tools this workspace cannot use yet and
 * the file's type warnings. */
function SavedNote({ saved, onDismiss }: { saved: OracleSaved; onDismiss: () => void }) {
  const { tools_added: added, tools_removed: removed, tools_missing: missing, warnings } = saved;
  if (!added.length && !removed.length && !missing.length && !warnings.length) return null;
  const warn = missing.length > 0;
  return <div className={`grid gap-1 rounded-card border px-3 py-2 text-[12px] text-ink-2 ${warn ? "border-attn-line bg-attn-soft" : "border-line"}`} role="status">
    <span className="flex items-center gap-2"><b className="min-w-0 flex-1 text-[13px] text-ink">Saved</b><button className="btn btn-sm" onClick={onDismiss}>Hide</button></span>
    {added.length > 0 && <span>Tools added: {added.join(", ")}</span>}
    {removed.length > 0 && <span>Tools removed: {removed.join(", ")}</span>}
    {missing.map((m) => <span key={m} className="text-attn">{m}: evaluations fail with class access until this is fixed (Settings → Credentials and tools)</span>)}
    {warnings.length > 0 && <span className="text-ink-3">{plural(warnings.length, "type warning")} (they do not block a save): see Definition</span>}
  </div>;
}

/** Saving this draft adds tools, or lets it send what it reads anywhere: highlighted before the save. */
function ToolsNotice({ change, tools, users }: { change: { added: OracleTool[]; removed: OracleTool[] }; tools: OracleTool[]; users: number }) {
  return <div className="grid gap-1 rounded-card border border-attn-line bg-attn-soft px-3 py-2 text-[12px] text-ink-2" role="status">
    <b className="text-[13px] text-ink">Saving changes what this oracle can call</b>
    {change.added.map((t) => <span key={t}>+ {t}</span>)}
    {change.removed.map((t) => <span key={t} className="text-ink-3">− {t}</span>)}
    {canLeak(tools) && <span className="text-attn">With http and a credentialed tool, it {LEAK_WARNING}.</span>}
    <span>Used by: {users ? plural(users, "routine") : "nothing yet"}</span>
    <span className="text-ink-3">Admin bots save credentialed tools only when their own session holds them; anything else waits for your card.</span>
  </div>;
}

/** A text area for the oracle's file, as tall as its text up to a limit, with Python colors drawn behind its
 * transparent text and the lines of `marks` tinted (red: pre-check errors, amber: type warnings). While a new reply
 * is on its way, lines the last colors do not match show plain. Lines too long to color (a pasted data blob) go to the
 * highlighter empty, so they stay plain and the rest of the file keeps its colors. */
function CodeArea({ value, onChange, label, marks, area }: { value: string; onChange: (t: string) => void; label: string; marks: Map<number, "error" | "warn">; area: React.RefObject<HTMLTextAreaElement | null> }) {
  const under = useRef<HTMLPreElement>(null);
  const theme = useTheme((s) => s.theme);
  const [tokens, setTokens] = useState<Tokens | null>(null);
  useLayoutEffect(() => { const el = area.current; if (!el) return; el.style.height = "auto"; el.style.height = `${Math.min(el.scrollHeight + 2, 720)}px`; }, [value, area]);
  useEffect(() => {
    let active = true;
    const colorable = value.split("\n").map((l) => l.length > MAX_HIGHLIGHT_LINE ? "" : l).join("\n");
    const timer = setTimeout(() => void highlight("oracle", colorable, "python", theme).then((t) => { if (active && t) setTokens(t); }), 150);
    return () => { active = false; clearTimeout(timer); };
  }, [value, theme]);
  const lines = useMemo(() => value.split("\n").map((line, i) => {
    const toks = tokens?.[i];
    if (!toks || toks.map((t) => t[0]).join("") !== line) return line;
    return toks.map(([text, color], j) => <span key={j} style={color ? { color } : undefined}>{text}</span>);
  }), [value, tokens]);
  const colored = tokens != null;
  const sync = () => { const a = area.current, b = under.current; if (a && b) { b.scrollTop = a.scrollTop; b.scrollLeft = a.scrollLeft; } };
  return <div className="relative grid">
    {colored && <pre ref={under} aria-hidden className="lp-code-under pointer-events-none absolute inset-0 m-0 overflow-hidden rounded-control border border-transparent bg-paper px-2.5 py-1.5 font-mono text-[12px] leading-5 whitespace-pre text-ink">
      {lines.map((l, i) => {
        const m = marks.get(i + 1);
        return m ? <span key={i} className={`-mx-2.5 block px-2.5 ${m === "error" ? "bg-diff-del" : "bg-attn-soft"}`}>{l}{"\n"}</span> : <Fragment key={i}>{l}{"\n"}</Fragment>;
      })}
    </pre>}
    <textarea ref={area} aria-label={label} wrap="off" className={`field-input min-h-[240px] resize-none overflow-x-auto py-1.5 font-mono text-[12px] leading-5 ${colored ? "relative bg-transparent text-transparent caret-ink" : ""}`} value={value} onChange={(e) => onChange(e.target.value)} onScroll={sync} spellCheck={false} />
  </div>;
}

/** Selects line `n` (1-based) of the text area, so a problem's line is one click away. */
function goToLine(el: HTMLTextAreaElement | null, n: number | null) {
  if (!el || n == null) return;
  const lines = el.value.split("\n");
  const start = lines.slice(0, n - 1).reduce((a, l) => a + l.length + 1, 0);
  el.focus();
  el.setSelectionRange(start, start + (lines[n - 1]?.length ?? 0));
}

/** Pre-check errors (red) or type warnings (amber) of the text, each with its line: a click selects that line. */
function Problems({ problems, tone, area }: { problems: OracleProblem[]; tone: "error" | "warn"; area: React.RefObject<HTMLTextAreaElement | null> }) {
  if (!problems.length) return null;
  return <div className={`grid gap-0.5 rounded-control border px-2.5 py-1.5 text-[12px] ${tone === "error" ? "border-danger/40 bg-diff-del" : "border-attn-line bg-attn-soft"}`} role={tone === "error" ? "alert" : "status"}>
    <b className={tone === "error" ? "text-gh-closed" : "text-attn"}>{tone === "error" ? `Refused by the pre-check: ${plural(problems.length, "error")}. Save and Test wait until you change the text.` : `${plural(problems.length, "type warning")} (they do not block Save or Test)`}</b>
    {problems.map((p, i) => <button key={i} className="min-w-0 text-left font-mono text-[11px] break-words hover:underline disabled:no-underline" disabled={p.line == null} onClick={() => goToLine(area.current, p.line)}>{problemText(p)}</button>)}
  </div>;
}

function DefinitionTab({ workspace, detail, text, onText, slug, errors, warnings, blocked, onRefused, onWarnings }: { workspace: string; detail: OracleDetail | null; text: string; onText: (t: string) => void; slug: string; errors: OracleProblem[]; warnings: OracleProblem[]; blocked: boolean; onRefused: (text: string, p: OracleProblem[]) => void; onWarnings: (text: string, p: OracleProblem[]) => void }) {
  const phone = usePhone();
  const area = useRef<HTMLTextAreaElement>(null);
  const [showInterface, setShowInterface] = useState(false);
  const meta = detail?.oracle.meta ?? null;
  const marks = useMemo(() => {
    const m = new Map<number, "error" | "warn">();
    for (const w of warnings) if (w.line != null) m.set(w.line, "warn");
    for (const e of errors) if (e.line != null) m.set(e.line, "error");
    return m;
  }, [errors, warnings]);
  const iface = detail?.interface ?? `/api/oracles/${encodeURIComponent(workspace)}/_interface`;
  return <div className={`grid min-w-0 gap-4 ${phone ? "grid-cols-[minmax(0,1fr)]" : "grid-cols-[minmax(0,1fr)_340px] items-start"}`}>
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-3">
      {meta?.description && <div className="grid gap-1"><b className="text-[12px] text-ink-2">Question</b><p className="m-0 text-[13px]">{meta.description}</p></div>}
      <div className="grid gap-1" data-tour="oracles.code"><b className="text-[12px] text-ink-2">oracles/{slug || "<slug>"}.py</b>
        <CodeArea value={text} onChange={onText} label="The oracle's file" marks={marks} area={area} />
        <Problems problems={errors} tone="error" area={area} />
        <Problems problems={warnings} tone="warn" area={area} />
        <span className="text-[12px] text-ink-3">The <code className="font-mono">{"# /// oracle"}</code> header holds the title, the question and the tools it calls; the arguments of <code className="font-mono">check</code> are its parameters, and it returns <code className="font-mono">verdict(…)</code> or calls <code className="font-mono">failed(…)</code>. Home lints and formats the file on save.</span>
        <span className="flex flex-wrap items-center gap-2 text-[12px]">
          <button className="btn btn-sm" aria-expanded={showInterface} onClick={() => setShowInterface(!showInterface)}><Icon name="file" size={12} />{showInterface ? "Hide the interface" : "Interface of launchpad_oracle"}</button>
          {!window.__lpDemo && <a href={daemonUrl(iface)} target="_blank" rel="noreferrer">Open it in a new tab</a>}
        </span>
        {showInterface && <InterfaceView url={iface} />}
      </div>
      {meta && <div className="grid gap-1"><b className="text-[12px] text-ink-2">Parameters</b>
        {meta.params.length || meta.cursor || meta.now ? <span className="flex flex-wrap gap-1">{meta.params.map((p) => <Token key={p.name}>{paramLabel(p)}</Token>)}{meta.cursor && <Token tone="teal" title="Each caller's own cursor">cursor</Token>}{meta.now && <Token tone="teal" title="The evaluation's start, in UTC">now</Token>}</span> : <span className="text-[12px] text-ink-3">None</span>}
        <span className="text-[12px] text-ink-3">Timeout {meta.timeout_ms / 1000} s · an answer is reused for {Math.round(meta.min_interval_ms / 60000)} min</span>
      </div>}
    </div>
    <TestPanel workspace={workspace} detail={detail} text={text} slug={slug} blocked={blocked || (!!detail && broken(detail.oracle) && text === detail.content)} onRefused={onRefused} onWarnings={onWarnings} />
  </div>;
}

function InterfaceView({ url }: { url: string }) {
  const q = useOracleInterface(url);
  if (q.isLoading) return <Loading what="the interface" />;
  if (q.error) return <ErrorState error={q.error} />;
  return <pre className="m-0 max-h-[420px] overflow-auto rounded-control border border-line bg-sunk p-2.5 font-mono text-[11px] leading-5 whitespace-pre">{q.data}</pre>;
}

/** The text a parameter's field starts with: its default, or nothing. */
const defaultText = (p: OracleParam) => (p.required || p.default == null ? "" : typeof p.default === "string" ? p.default : p.kind === "bool" ? String(!!p.default) : String(p.default));

/** Test: runs the saved oracle (recorded as a test, commits no cursor) or the unsaved draft (pre-checked, recorded
 * nowhere), on Home. A draft's fields come from its own `def check(...)`. */
function TestPanel({ workspace, detail, text, slug, blocked, onRefused, onWarnings }: { workspace: string; detail: OracleDetail | null; text: string; slug: string; blocked: boolean; onRefused: (text: string, p: OracleProblem[]) => void; onWarnings: (text: string, p: OracleProblem[]) => void }) {
  const qc = useQueryClient();
  const draft = !detail || text !== detail.content;
  const savedMeta = detail?.oracle.meta;
  const brokenSaved = !draft && !!detail && broken(detail.oracle);
  const sig = useMemo(() => (brokenSaved ? null : draft || !savedMeta ? signatureOf(text) : { params: savedMeta.params, cursor: savedMeta.cursor, now: savedMeta.now }), [brokenSaved, draft, savedMeta, text]);
  const params = sig?.params ?? [];
  // The fields start with the parameters of the last evaluation (what the routine or the last test passed).
  const [values, setValues] = useState<Record<string, string>>(() => Object.fromEntries(Object.entries(detail?.last?.params ?? {}).filter(([, v]) => v != null).map(([k, v]) => [k, typeof v === "string" ? v : String(v)])));
  const [cursor, setCursor] = useState("");
  const [answer, setAnswer] = useState<OracleTestAnswer | null>(null);
  const [busy, setBusy] = useState(false);
  const valueOf = (p: OracleParam) => values[p.name] ?? defaultText(p);
  const run = async () => {
    let body: Record<string, unknown>;
    try {
      const ps: Record<string, unknown> = {};
      for (const p of params) { const v = paramValue(p, valueOf(p)); if (v !== undefined) ps[p.name] = v; }
      body = { params: ps };
    } catch (e) { toastError(e); return; }
    if (draft) body.content = text;
    if (sig?.cursor && cursor.trim()) body.cursor = cursor.trim();
    setBusy(true);
    try {
      const a = await runAction<OracleTestAnswer>("oracle.test", { workspace, slug: slug || "draft" }, body);
      setAnswer(a);
      if (draft) onWarnings(text, a.warnings);
      if (a.evaluation_id) await qc.invalidateQueries({ queryKey: ["oracle", workspace, slug] });
    } catch (e) {
      const problems = problemsOf(e);
      if (problems) { setAnswer(null); onRefused(text, problems); } else toastError(e);
    } finally { setBusy(false); }
  };
  return <section className="grid min-w-0 gap-2 rounded-card border border-line bg-paper p-3" data-tour="oracles.test">
    <div className="flex flex-wrap items-baseline gap-x-2"><b className="text-[13px]">Test</b><span className="text-[12px] text-ink-3">{draft ? "runs these unsaved changes on Home; records nothing" : "runs the saved file on Home; shows in History, commits no cursor"}</span></div>
    {brokenSaved ? <span className="text-[12px] text-ink-3">The saved file is broken: fix it in the editor, then test your changes.</span>
      : !sig && <span className="text-[12px] text-attn">No <code className="font-mono">def check(...)</code> in the file.</span>}
    {params.map((p) => p.kind === "bool"
      ? <label key={p.name} className="flex min-h-9 items-center gap-2 text-[12px] font-semibold text-ink-2"><input type="checkbox" className="accent-teal" checked={valueOf(p) === "true"} onChange={(e) => setValues({ ...values, [p.name]: String(e.target.checked) })} />{p.name}<span className="font-normal text-ink-3">bool</span></label>
      : <label key={p.name} className="grid gap-1 text-[12px] font-semibold text-ink-2"><span>{p.name} <span className="font-normal text-ink-3">{p.kind}{p.optional ? " | None" : ""}{p.required ? ", required" : ""}</span></span>
        <input className="field-input font-mono text-[12px] font-normal" inputMode={p.kind === "int" || p.kind === "float" ? "decimal" : undefined} value={valueOf(p)} placeholder={p.optional ? "None" : p.required ? "required" : pyValue(p.default)} onChange={(e) => setValues({ ...values, [p.name]: e.target.value })} />
      </label>)}
    {sig?.cursor && <label className="grid gap-1 text-[12px] font-semibold text-ink-2">cursor<input className="field-input font-mono text-[12px] font-normal" value={cursor} placeholder="None (as a first run)" onChange={(e) => setCursor(e.target.value)} /></label>}
    <div className="flex items-center gap-2"><button className="btn btn-sm" disabled={busy || blocked || !text.trim() || !sig} title={blocked ? "Fix the errors of the pre-check first" : undefined} onClick={() => void run()}><Icon name="play" size={12} />{busy ? "Testing…" : "Test"}</button><span className="text-[12px] text-ink-3">in Monty, on Home</span></div>
    {answer && <TestResult r={answer.result} warnings={draft ? answer.warnings : []} />}
  </section>;
}

const size = (n: number) => (n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);

/** The call log: one line per tool call (tool, call, outcome, bytes, ms); the call the error points at is red. */
export function CallLog({ calls, error }: { calls: OracleCall[]; error: OracleError | null }) {
  if (!calls.length) return <span className="text-ink-3">No tool call.</span>;
  return <div className="grid min-w-0 gap-px rounded-control border border-line bg-paper py-1 font-mono text-[11px]" aria-label="Call log">
    {calls.map((c, i) => {
      const bad = error?.call === i;
      return <div key={i} className={`grid min-w-0 grid-cols-[minmax(0,1fr)_auto] gap-x-2 px-2 ${bad ? "bg-diff-del text-gh-closed" : ""}`} title={bad ? "The call this failure points at" : undefined}>
        <span className="min-w-0 break-all"><b>{c.tool}</b> {c.call}</span>
        <span className={`whitespace-nowrap ${bad ? "" : "text-ink-3"}`}>{c.outcome} · {size(c.bytes)} · {c.duration_ms} ms</span>
      </div>;
    })}
  </div>;
}

/** The answer of a test: outcome, duration, class, summary, fingerprint, proposed cursor, links, items, log and call
 * log, and a draft's type warnings. */
export function TestResult({ r, warnings = [] }: { r: OracleRunResult; warnings?: OracleProblem[] }) {
  const tone = r.outcome === "true" ? "border-gh-open/40 bg-diff-add" : r.outcome === "failed" ? "border-danger/40 bg-diff-del" : "border-line bg-sunk";
  return <div className={`grid min-w-0 gap-1.5 rounded-card border p-2.5 text-[12px] ${tone}`} role="status">
    <span className="flex items-center gap-2"><StatusBadge tone={OUTCOME_TONE[r.outcome]} label={OUTCOME_LABEL[r.outcome]} /><span className="text-ink-3">{(r.duration_ms / 1000).toFixed(2)} s</span>{r.error && <Badge tone="danger">{r.error.class}</Badge>}</span>
    {(r.error?.message || r.summary) && <span className="text-[13px] break-words">{r.error?.message ?? r.summary}</span>}
    <span className="font-mono text-[11px] break-all text-ink-3">fingerprint {r.fingerprint ?? "—"} · cursor {r.cursor === undefined ? "kept" : r.cursor === null ? "reset" : `→ ${typeof r.cursor === "string" ? r.cursor : JSON.stringify(r.cursor)}`}</span>
    {r.links.map((l) => <a key={l} href={l} target="_blank" rel="noreferrer" className="truncate">{l}</a>)}
    {r.items.length > 0 && <details><summary className="cursor-pointer text-ink-2">{plural(r.items.length, "item")}</summary><pre className="m-0 mt-1 max-h-[200px] overflow-auto rounded-control bg-paper p-2 font-mono text-[11px] whitespace-pre-wrap">{JSON.stringify(r.items, null, 2)}</pre></details>}
    <span className="text-ink-3">Calls</span>
    <CallLog calls={r.calls} error={r.error} />
    {r.log.trim() && <><span className="text-ink-3">Log</span><pre className="m-0 max-h-[160px] overflow-auto rounded-control bg-paper p-2 font-mono text-[11px] whitespace-pre-wrap text-ink-2">{r.log.trim()}</pre></>}
    {warnings.length > 0 && <span className="text-attn">{plural(warnings.length, "type warning")}: {warnings.map(problemText).join("; ")}</span>}
  </div>;
}

/** The five tools: a switch per tool rewrites the header's `tools = [...]` line; an added tool is amber before the
 * save, and a credentialed tool with http says it can send what it reads anywhere. */
function ToolsTab({ text, onText, saved, infos, users, change }: { text: string; onText: (t: string) => void; saved: OracleTool[]; infos: OracleToolInfo[] | undefined; users: number; change: { added: OracleTool[]; removed: OracleTool[] } }) {
  const phone = usePhone();
  const current = toolsOf(text);
  const editable = current != null;
  const on = current ?? saved;
  const set = (t: OracleTool) => { const next = withTools(text, on.includes(t) ? on.filter((x) => x !== t) : [...on, t]); if (next != null) onText(next); };
  const leak = canLeak(on);
  return <div className={`grid min-w-0 gap-4 ${phone ? "grid-cols-[minmax(0,1fr)]" : "grid-cols-[minmax(0,1fr)_340px] items-start"}`}>
    <div className="grid min-w-0 gap-2" data-tour="oracles.tools">
      <b className="text-[13px]">What this oracle can call</b>
      <p className="m-0 text-[12px] text-ink-3">Nothing until you add it. Each tool is a GET or a read command that Home runs and logs; tokens never reach the oracle's code.</p>
      <div className="grid grid-cols-[minmax(0,1fr)] overflow-hidden rounded-card border border-line">
        {ORACLE_TOOLS.map((t) => {
          const info = infos?.find((x) => x.tool === t);
          const isOn = on.includes(t), was = saved.includes(t);
          const state = isOn === was ? (isOn ? "on" : "off") : isOn ? "added" : "removed";
          const risky = leak && isOn && (t === "http" || CREDENTIALED.includes(t));
          return <label key={t} className={`grid min-h-[52px] grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 border-b border-line px-3 py-2 text-[13px] last:border-b-0 ${state === "added" ? "bg-attn-soft" : ""}`}>
            <input type="checkbox" className="accent-teal" checked={isOn} disabled={!editable} onChange={() => set(t)} />
            <span className="grid min-w-0">
              <span className="font-mono text-[12px] font-semibold">{t}</span>
              <span className="text-[12px] text-ink-3">{info?.reaches ?? ""}</span>
              <span className="text-[12px]">{info ? (info.registry ? <>token of <code className="font-mono">{info.registry}</code> · {info.ready ? <span className="text-gh-open">ready in this workspace</span> : <span className="text-attn">{info.problem ?? "not ready in this workspace"}</span>}</> : <span className="text-ink-3">needs no token</span>) : null}</span>
              {risky && <span className="text-[12px] text-attn">With {t === "http" ? "a credentialed tool" : "http"}, this oracle {LEAK_WARNING}.</span>}
            </span>
            <span className={`text-[12px] ${state === "added" ? "font-semibold text-attn" : "text-ink-3"}`}>{state}</span>
          </label>;
        })}
      </div>
      {!editable && <p className="m-0 text-[12px] text-attn">The header writes its tools another way, or is missing: edit the file in Definition.</p>}
    </div>
    <div className="grid min-w-0 gap-3">
      {change.added.length > 0 || (leak && !canLeak(saved)) ? <ToolsNotice change={change} tools={on} users={users} /> : change.removed.length > 0 && <div className="grid gap-1 rounded-card border border-line px-3 py-2 text-[12px] text-ink-2"><b className="text-[13px] text-ink">Saving removes tools</b>{change.removed.map((t) => <span key={t}>− {t}</span>)}</div>}
      <div className="grid gap-1 rounded-card border border-line px-3 py-2 text-[12px] text-ink-2">
        <b className="text-[13px] text-ink">How tools work</b>
        <span>The header's <code className="font-mono">tools = [...]</code> is what you approve; Home checks it on every call. A tool missing from it raises <code className="font-mono">PermissionError</code>.</span>
        <span className="text-ink-3">Credentialed tools use the workspace's token of their registry tool (Settings → Credentials and tools).</span>
      </div>
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
              {e.error ? <span className="break-words text-gh-closed">{e.error.class}: {e.error.message}</span> : e.summary && <span>{e.summary}</span>}
              <span className="font-mono text-[11px] break-all text-ink-3">params {JSON.stringify(e.params)} · fingerprint {e.fingerprint ?? "—"} · cursor in {e.cursor_in == null ? "—" : JSON.stringify(e.cursor_in)} · proposed {e.cursor_proposed == null ? "—" : JSON.stringify(e.cursor_proposed)}</span>
              {e.links.map((l) => <a key={l} href={l} target="_blank" rel="noreferrer" className="truncate">{l}</a>)}
              <span className="text-ink-3">Calls</span>
              <CallLog calls={e.calls} error={e.error} />
              {e.log.trim() && <><span className="text-ink-3">Log</span><pre className="m-0 max-h-[160px] overflow-auto rounded-control bg-sunk p-2 font-mono text-[11px] whitespace-pre-wrap">{e.log.trim()}</pre></>}
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
  const wrong = gate ? params.find((p) => { const v = (gate.params ?? {})[p.name]; return (p.kind === "int" || p.kind === "float") && v !== undefined && v !== null && typeof v !== "number"; }) : undefined;
  const missing = gate && picked ? params.find((p) => p.required && (gate.params ?? {})[p.name] === undefined) : undefined;
  const problem = gateProblem(gate) ?? (wrong ? `${wrong.name} takes ${wrong.kind === "int" ? "a whole number" : "a number"}.` : missing ? `${missing.name} is required.` : null);
  const set = (p: Partial<Gate>) => gate && onChange({ ...gate, ...p });
  // Leaving Failed out drops its instructions too (Home refuses [gate.failed] without "failed" in run_on).
  const toggle = (o: OracleOutcome) => gate && (gate.run_on.includes(o) ? set({ run_on: gate.run_on.filter((x) => x !== o), ...(o === "failed" ? { failed: undefined } : {}) }) : set({ run_on: [...gate.run_on, o] }));
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
        {params.map((p) => {
          const given = (gate.params ?? {})[p.name];
          const put = (v: unknown) => { const next = { ...(gate.params ?? {}) }; if (v === undefined) delete next[p.name]; else next[p.name] = v; set({ params: next }); };
          return p.kind === "bool"
            ? <label key={p.name} className="grid min-h-9 grid-cols-[120px_minmax(0,1fr)] items-center gap-2 text-[12px] text-ink-2"><span className="truncate" title={paramLabel(p)}>{p.name}</span>
              <span className="flex items-center gap-2"><input type="checkbox" className="accent-teal" checked={given === undefined ? !!p.default : given === true} onChange={(e) => put(e.target.checked === !!p.default && !p.required ? undefined : e.target.checked)} /><span className="text-ink-3">{given === undefined ? `default ${pyValue(p.default)}` : ""}</span></span>
            </label>
            : <label key={p.name} className="grid grid-cols-[120px_minmax(0,1fr)] items-center gap-2 text-[12px] text-ink-2"><span className="truncate" title={paramLabel(p)}>{p.name}</span>
              <input className="field-input font-mono text-[12px]" inputMode={p.kind === "int" || p.kind === "float" ? "decimal" : undefined} value={given === undefined || given === null ? "" : String(given)} placeholder={p.required ? `${p.kind} · required` : `${p.kind} · default ${pyValue(p.default)}`} onChange={(e) => {
                const t = e.target.value;
                if (!t.trim()) return put(undefined);
                if (p.kind === "int" || p.kind === "float") return put((p.kind === "int" ? /^-?\d+$/ : /^-?\d+(\.\d+)?$/).test(t.trim()) ? Number(t) : t);
                put(t);
              }} />
            </label>;
        })}
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
