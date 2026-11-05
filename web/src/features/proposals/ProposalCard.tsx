import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { memo, useState } from "react";
import type { Proposal } from "@/gen/Proposal";
import type { Session } from "@/gen/Session";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { ConfirmButton, ErrorState, Token } from "@/components/ui";
import { api } from "@/lib/api";
import { useProfiles } from "@/lib/profiles";
import { useProjects, useSessions } from "@/lib/queries";
import { useComposer } from "@/stores/composer";
import { useWorkspace } from "@/stores/workspace";
import { useBots } from "@/lib/bots";
import { browserZone, triggerSummary } from "@/lib/routines";
import { PatchView } from "@/features/github/DiffReview";
import { useLayout } from "@/stores/layout";
import { StatusBadge } from "@/features/agents/SessionBits";

export const useProposalList = (enabled = true) => useQuery({ enabled, queryKey: ["proposals"], queryFn: () => api.get<Proposal[]>("/api/proposals"), staleTime: 30_000 });

const START: Record<Proposal["body"]["kind"], string> = { launch: "Start", message: "Send", schedule: "Schedule", cron: "Create routine", status: "", preset: "Save preset", tool: "Add tool", bot: "Create bot", routine: "Add routine", bot_instructions: "Apply to BOT.md" };

/** A new BOT.md: the bot and the diff. Applying fails when BOT.md changed since. */
function BotInstructionsBody({ body }: { body: Extract<Proposal["body"], { kind: "bot_instructions" }> }) {
  const bots = useBots().data ?? [];
  return (
    <>
      <span className="flex flex-wrap gap-1"><Token icon="file">BOT.md of {bots.find((b) => b.id === body.bot_id)?.settings.name ?? "a bot"}</Token></span>
      <div className="max-h-[320px] overflow-auto rounded-md border border-line"><PatchView patch={body.patch} path="BOT.md" language="markdown" split={false} wrap /></div>
    </>
  );
}

/** A routine card: the bot, when it runs and what it does at each run. Adding it starts nothing now. */
function RoutineBody({ body }: { body: Extract<Proposal["body"], { kind: "routine" }> }) {
  const bots = useBots().data ?? [];
  const projects = useProjects().data ?? [];
  return (
    <>
      <span className="flex flex-wrap gap-1">
        <Token icon="refresh" tone="warn">Routine of {bots.find((b) => b.id === body.bot_id)?.settings.name ?? "a bot"}</Token>
        <Token>{triggerSummary({ ...body, time_zone: browserZone() }, (id) => projects.find((p) => p.id === id)?.name ?? "a folder")}</Token>
      </span>
      <p className="m-0 line-clamp-4 text-[12px] whitespace-pre-wrap text-ink-2">{body.instructions}</p>
    </>
  );
}

/** A bot card: its name, profile, folder and instructions (BOT.md). Creating it starts nothing. */
function BotBody({ body }: { body: Extract<Proposal["body"], { kind: "bot" }> }) {
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  return (
    <>
      <span className="flex flex-wrap gap-1">
        <Token icon="spark">New bot · {body.name}</Token>
        <Token>{profiles.find((p) => p.id === body.profile)?.name ?? body.profile ?? "no profile"}</Token>
        <Token icon="folder">{body.folder ? projects.find((p) => p.id === body.folder)?.name ?? "a folder" : "its own new folder"}</Token>
      </span>
      <p className="m-0 line-clamp-4 text-[12px] whitespace-pre-wrap text-ink-2">{body.instructions}</p>
    </>
  );
}

/** A preset card: new or updated label, availability and text. */
function PresetBody({ body }: { body: Extract<Proposal["body"], { kind: "preset" }> }) {
  return (
    <>
      <span className="flex flex-wrap gap-1">
        <Token icon="preset">{body.preset_id ? "Update preset" : "New preset"} · {body.label}</Token>
        <Token>{body.agent ? "agent composer / steering" : "not in agent composer"}</Token>
      </span>
      <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">{body.text}</p>
    </>
  );
}

/** A tool card: what runners install, the variables agents get and the hosts that receive the token. */
function ToolBody({ body }: { body: Extract<Proposal["body"], { kind: "tool" }> }) {
  const t = body.tool;
  return (
    <span className="flex flex-wrap gap-1">
      <Token icon="term">{t.name}</Token>
      {t.install && <Token>installs {t.install.kind === "github_release" ? `GitHub release ${t.install.source}` : `${t.install.kind} ${t.install.source}`}</Token>}
      {t.env.length > 0 && <Token>{t.env.join(", ")}</Token>}
      {t.hosts.length > 0 && <Token tone="warn">token sent to {t.hosts.map((h) => h.host).join(", ")}</Token>}
      {body.token_url && <a className="text-[12px] text-teal-ink hover:underline" href={body.token_url} target="_blank" rel="noreferrer">Create the token</a>}
    </span>
  );
}

function specOf(p: Proposal): LaunchSpec | null {
  return p.body.kind === "launch" || p.body.kind === "schedule" || p.body.kind === "cron" ? p.body.spec : null;
}

/** Profile, then only values that differ from defaults, then warnings, like the composer's option line. */
function SpecTokens({ spec }: { spec: LaunchSpec }) {
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  const profile = profiles.find((x) => x.id === spec.profile);
  const folder = spec.target.kind === "project" || spec.target.kind === "predraft" ? projects.find((x) => x.id === (spec.target as { project_id: string }).project_id)?.name : spec.target.kind === "temp" ? "temporary folder" : null;
  return (
    <span className="flex flex-wrap gap-1">
      {folder && <Token icon="folder">{folder}</Token>}
      <Token>{spec.harness && <HarnessMark harness={spec.harness} size={11} />}{profile?.name ?? spec.profile ?? "no profile"}</Token>
      {spec.model && spec.model !== profile?.model && <Token>{spec.model}</Token>}
      {spec.target.kind === "project" && !spec.worktree && <Token>checkout</Token>}
      {spec.worktree?.when_done === "merge" && <Token>merge when done</Token>}
      {spec.worktree?.when_done === "local_pr" && <Token>Local PR when done</Token>}
      {spec.github_refs.length > 0 && <Token icon="pr">{spec.github_refs.length} GitHub item{spec.github_refs.length === 1 ? "" : "s"}</Token>}
      {(spec.agent_traces ?? []).length > 0 && <Token icon="agents">+ {spec.agent_traces.length} trace</Token>}
      {profile?.billing === "pay_as_you_go" && <Token tone="warn">pay as you go</Token>}
      {spec.budget_usd != null && <Token tone="warn">limit ${spec.budget_usd.toFixed(2)}</Token>}
    </span>
  );
}

/** Replaces the card (so it cannot start twice), then puts its spec into the composer draft. */
async function editInComposer(p: Proposal, qc: QueryClient) {
  const spec = specOf(p);
  if (!spec) return;
  try {
    const next = await api.post<Proposal>(`/api/proposals/${p.id}/edit`);
    qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
  } catch (e) { toastError(e); return; }
  const c = useComposer.getState();
  useComposer.setState({ open: true, minimized: false });
  c.update((s) => ({ ...s, ...spec }));
  if (p.body.kind === "schedule") c.setConditions(p.body.conditions);
  if (p.body.kind === "cron") toast(`Pick Repeat on a schedule… in Launch ▾ (cron ${p.body.schedule})`);
}

function EditButton({ p }: { p: Proposal }) {
  const qc = useQueryClient();
  const hasDraft = useComposer((s) => s.open && (!!s.spec.text.trim() || s.spec.github_refs.length > 0));
  if (!specOf(p)) return null;
  if (hasDraft) return <ConfirmButton tone="publish" consequence="Replace the current draft? This card is then replaced by what you launch." confirmLabel="Replace draft" onConfirm={() => editInComposer(p, qc)}>Edit</ConfirmButton>;
  return <button className="btn btn-sm" title="Open it in the composer to change anything; this card is then replaced by what you launch" onClick={() => void editInComposer(p, qc)}>Edit</button>;
}

const SessionRow = memo(function SessionRow({ s }: { s: Session }) {
  const show = useLayout((x) => x.show);
  const navigate = useNavigate();
  return (
    <button className="flex min-h-10 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-sunk" onClick={() => { show(s.id); void navigate({ to: "/agents", search: { session: s.id } }); }}>
      <span className="min-w-0 flex-1"><span className="block truncate text-[13px] font-semibold">{s.title}</span>{s.last_message && <span className="block truncate text-[12px] text-ink-3">{s.last_message}</span>}</span>
      <StatusBadge s={s} />
    </button>
  );
});

export const ProposalCard = memo(function ProposalCard({ p, onAccepted, anchor = false }: { p: Proposal; onAccepted?: () => void; anchor?: boolean }) {
  const qc = useQueryClient();
  const sessions = useSessions().data ?? [];
  const [busy, setBusy] = useState(false);
  const act = async (action: "accept" | "dismiss" | "recheck") => {
    setBusy(true);
    try {
      // A routine or cron keeps the time zone of the browser that accepts it.
      const next = await api.post<Proposal>(`/api/proposals/${p.id}/${action}${action === "accept" ? `?time_zone=${encodeURIComponent(browserZone())}` : ""}`);
      qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
      if (next.state === "started" && (action === "accept" || action === "recheck")) {
        await qc.invalidateQueries({ queryKey: ["sessions"] });
        onAccepted?.();
      }
      if (next.state === "failed") toastError(new Error(next.error ?? "It could not start"));
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const spec = specOf(p);
  const started = p.state === "started";
  const session = started && (p.body.kind === "launch" || p.body.kind === "message") ? sessions.find((s) => s.id === p.result_id) : undefined;
  const statusSessions = p.body.kind === "status" ? p.body.session_ids.map((id) => sessions.find((s) => s.id === id)).filter((s): s is Session => !!s) : [];
  return (
    <article id={anchor ? `proposal-${p.id}` : undefined} data-tour="proposal.card" data-tour-id={p.id} tabIndex={-1} className={`grid gap-2 rounded-xl border bg-paper p-3 ${(p.body.kind === "cron" || p.body.kind === "routine") && !started ? "border-attn-line" : "border-line"}`}>
      <div className="flex items-start gap-2">
        <Icon name={p.body.kind === "message" ? "send" : p.body.kind === "schedule" ? "clock" : p.body.kind === "cron" || p.body.kind === "routine" ? "refresh" : p.body.kind === "status" ? "agents" : p.body.kind === "preset" ? "preset" : p.body.kind === "tool" ? "term" : p.body.kind === "bot" ? "spark" : p.body.kind === "bot_instructions" ? "file" : "play"} size={14} className="mt-0.5 flex-none text-ink-3" />
        <div className="min-w-0 flex-1">
          <b className="block text-[13px]">{p.title}</b>
          {p.note && <p className="m-0 text-[12px] text-ink-2">{p.note}</p>}
        </div>
      </div>
      {spec && <SpecTokens spec={spec} />}
      {p.body.kind === "cron" && <Token tone="warn" icon="refresh">Creates a routine · {p.body.schedule} ({browserZone()})</Token>}
      {p.body.kind === "schedule" && <Token tone="warn" icon="clock">{p.body.conditions.length} start condition{p.body.conditions.length === 1 ? "" : "s"}</Token>}
      {spec && !started && <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">{spec.text}</p>}
      {p.body.kind === "message" && !started && <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">To “{sessions.find((s) => s.id === (p.body as { session_id: string }).session_id)?.title ?? "an agent"}”: {p.body.text}</p>}
      {p.body.kind === "preset" && <PresetBody body={p.body} />}
      {p.body.kind === "tool" && <ToolBody body={p.body} />}
      {p.body.kind === "bot" && <BotBody body={p.body} />}
      {p.body.kind === "routine" && <RoutineBody body={p.body} />}
      {p.body.kind === "bot_instructions" && <BotInstructionsBody body={p.body} />}
      {statusSessions.length > 0 && <div className="grid">{statusSessions.map((s) => <SessionRow key={s.id} s={s} />)}</div>}
      {session && <SessionRow s={session} />}
      {started && !session && p.body.kind !== "status" && <span className="text-[12px] text-gh-open"><Icon name="check" size={11} /> {p.body.kind === "schedule" ? <>Scheduled: see <Link className="underline" to="/schedules" search={{ tab: "launches" }}>Schedules</Link></> : p.body.kind === "bot_instructions" ? <>Applied: see the bot's <Link className="underline" to="/bots/$id" params={{ id: p.result_id ?? "" }} search={{ panel: "settings" }}>Settings</Link></> : p.body.kind === "cron" || p.body.kind === "routine" ? <>Routine added: see <Link className="underline" to="/schedules" search={{ routine: p.result_id ?? undefined }}>Schedules</Link></> : p.body.kind === "preset" ? "Preset saved: see Settings → Presets" : p.body.kind === "bot" ? <>Bot created. <Link className="underline" to="/bots/$id" params={{ id: p.result_id ?? "" }}>Send it its first task</Link>.</> : p.body.kind === "tool" ? <>Tool added. <Link className="underline" to="/settings/credentials" search={{ tool: p.result_id ?? undefined }}>Paste its token</Link> to use it.</> : "Started"}</span>}
      {p.state === "replaced" && <span className="text-[12px] text-ink-3">{p.replaced_by ? "Replaced" : "Edited in the composer"}{p.replaced_by && <> · <Link to="/proposals" search={{ proposal: p.replaced_by }} onClick={() => { if (p.workspace_id) useWorkspace.getState().switchTo(p.workspace_id); }} className="underline">View replacement</Link></>}</span>}
      {p.state === "dismissed" && <span className="text-[12px] text-ink-3">Dismissed</span>}
      {p.state === "failed" && p.error && <ErrorState error={p.error} />}
      {p.state === "uncertain" && <div className="grid gap-1.5 text-[12px] text-attn">Delivery not confirmed. Check Agents before starting a new attempt.<button className="btn btn-sm w-fit" disabled={busy} onClick={() => void act("recheck")}>I checked Agents — check again</button></div>}
      {(p.state === "proposed" || p.state === "failed") && p.body.kind !== "status" && (
        <div className="flex flex-wrap items-center gap-1.5">
          <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => void act("accept")}><Icon name="play" size={12} />{busy ? "Starting…" : START[p.body.kind]}</button>
          <EditButton p={p} />
          <button className="btn btn-sm btn-ghost ml-auto" disabled={busy} onClick={() => void act("dismiss")}>Dismiss</button>
        </div>
      )}
      {p.body.kind === "status" && p.state === "proposed" && <button className="btn btn-sm btn-ghost w-fit" onClick={() => void act("dismiss")}>Dismiss</button>}
    </article>
  );
});

