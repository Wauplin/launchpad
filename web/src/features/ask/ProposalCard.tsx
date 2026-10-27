import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
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
import { useLayout } from "@/stores/layout";
import { StatusBadge } from "@/features/agents/SessionBits";

export const useProposalList = (enabled = true) => useQuery({ enabled, queryKey: ["proposals"], queryFn: () => api.get<Proposal[]>("/api/proposals"), staleTime: 30_000 });

const START: Record<Proposal["body"]["kind"], string> = { launch: "Start", message: "Send", schedule: "Schedule", cron: "Create cron", status: "" };

function specOf(p: Proposal): LaunchSpec | null {
  return p.body.kind === "launch" || p.body.kind === "schedule" || p.body.kind === "cron" ? p.body.spec : null;
}

/** Profile, then only values that differ from defaults, then warnings, like the composer's option line. */
function SpecTokens({ spec }: { spec: LaunchSpec }) {
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  const profile = profiles.find((x) => x.id === spec.profile);
  const folder = spec.target.kind === "project" || spec.target.kind === "predraft" ? projects.find((x) => x.id === (spec.target as { project_id: string }).project_id)?.name : spec.target.kind === "temp" ? "scratch folder" : null;
  return (
    <span className="flex flex-wrap gap-1">
      {folder && <Token icon="folder">{folder}</Token>}
      <Token>{spec.harness && <HarnessMark harness={spec.harness} size={11} />}{profile?.name ?? spec.profile ?? "no profile"}</Token>
      {spec.model && spec.model !== profile?.model && <Token>{spec.model}</Token>}
      {spec.target.kind === "project" && !spec.worktree && <Token>checkout</Token>}
      {spec.worktree?.merge_when_done && <Token>merge when done</Token>}
      {spec.github_refs.length > 0 && <Token icon="pr">{spec.github_refs.length} GitHub item{spec.github_refs.length === 1 ? "" : "s"}</Token>}
      {(spec.agent_traces ?? []).length > 0 && <Token icon="agents">+ {spec.agent_traces.length} trace</Token>}
      {profile?.billing === "pay_as_you_go" && <Token tone="warn">pay as you go</Token>}
      {spec.budget_usd != null && <Token tone="warn">limit ${spec.budget_usd.toFixed(2)}</Token>}
    </span>
  );
}

/** Puts a proposal's spec into the composer draft (asks first when an unsent draft exists). */
function editInComposer(p: Proposal) {
  const spec = specOf(p);
  if (!spec) return;
  const c = useComposer.getState();
  useComposer.setState({ open: true, minimized: false });
  c.update((s) => ({ ...s, ...spec }));
  if (p.body.kind === "schedule") c.setConditions(p.body.conditions);
  if (p.body.kind === "cron") toast(`Pick Repeat on a schedule… in Launch ▾ (cron ${p.body.schedule})`);
}

function EditButton({ p }: { p: Proposal }) {
  const hasDraft = useComposer((s) => s.open && (!!s.spec.text.trim() || s.spec.github_refs.length > 0));
  if (!specOf(p)) return null;
  if (hasDraft) return <ConfirmButton tone="publish" consequence="Replace the current draft?" confirmLabel="Replace draft" onConfirm={() => editInComposer(p)}>Edit</ConfirmButton>;
  return <button className="btn btn-sm" title="Open it in the composer to change anything" onClick={() => editInComposer(p)}>Edit</button>;
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

export const ProposalCard = memo(function ProposalCard({ p, onAccepted }: { p: Proposal; onAccepted?: () => void }) {
  const qc = useQueryClient();
  const sessions = useSessions().data ?? [];
  const [busy, setBusy] = useState(false);
  const act = async (action: "accept" | "dismiss" | "recheck") => {
    setBusy(true);
    try {
      const next = await api.post<Proposal>(`/api/proposals/${p.id}/${action}`);
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
    <article className={`grid gap-2 rounded-xl border bg-paper p-3 ${p.body.kind === "cron" && !started ? "border-attn-line" : "border-line"}`}>
      <div className="flex items-start gap-2">
        <Icon name={p.body.kind === "message" ? "send" : p.body.kind === "schedule" ? "clock" : p.body.kind === "cron" ? "refresh" : p.body.kind === "status" ? "agents" : "play"} size={14} className="mt-0.5 flex-none text-ink-3" />
        <div className="min-w-0 flex-1">
          <b className="block text-[13px]">{p.title}</b>
          {p.note && <p className="m-0 text-[12px] text-ink-2">{p.note}</p>}
        </div>
      </div>
      {spec && <SpecTokens spec={spec} />}
      {p.body.kind === "cron" && <Token tone="warn" icon="refresh">Creates a repeating agent · {p.body.schedule}</Token>}
      {p.body.kind === "schedule" && <Token tone="warn" icon="clock">{p.body.conditions.length} start condition{p.body.conditions.length === 1 ? "" : "s"}</Token>}
      {spec && !started && <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">{spec.text}</p>}
      {p.body.kind === "message" && !started && <p className="m-0 line-clamp-3 text-[12px] whitespace-pre-wrap text-ink-2">To “{sessions.find((s) => s.id === (p.body as { session_id: string }).session_id)?.title ?? "an agent"}”: {p.body.text}</p>}
      {statusSessions.length > 0 && <div className="grid">{statusSessions.map((s) => <SessionRow key={s.id} s={s} />)}</div>}
      {session && <SessionRow s={session} />}
      {started && !session && p.body.kind !== "status" && <span className="text-[12px] text-gh-open"><Icon name="check" size={11} /> {p.body.kind === "schedule" ? "Scheduled: see Agents → Scheduled" : p.body.kind === "cron" ? "Cron created: see Agents → Cron" : "Started"}</span>}
      {p.state === "failed" && p.error && <ErrorState error={p.error} />}
      {p.state === "uncertain" && <div className="grid gap-1.5 text-[12px] text-attn">Delivery not confirmed. Check Agents before starting a new attempt.<button className="btn btn-sm w-fit" disabled={busy} onClick={() => void act("recheck")}>I checked Agents — check again</button></div>}
      {(p.state === "proposed" || p.state === "failed") && p.body.kind !== "status" && (
        <div className="flex flex-wrap items-center gap-1.5">
          <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => void act("accept")}><Icon name="play" size={12} />{busy ? "Starting…" : `${START[p.body.kind]}${p.session_id ? " & close thread" : ""}`}</button>
          <EditButton p={p} />
          <button className="btn btn-sm btn-ghost ml-auto" disabled={busy} onClick={() => void act("dismiss")}>Dismiss</button>
        </div>
      )}
      {p.body.kind === "status" && p.state === "proposed" && <button className="btn btn-sm btn-ghost w-fit" onClick={() => void act("dismiss")}>Dismiss</button>}
    </article>
  );
});

