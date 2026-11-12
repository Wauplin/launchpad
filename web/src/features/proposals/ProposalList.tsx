// A list of proposal cards (a bot's page): the inert cards agents write with the Launchpad CLI tool (`launchpad
// propose`). Nothing starts until the owner clicks Start (or Send, Schedule, Create routine), except one-time
// starts of a bot in auto mode. Waiting cards of every bot and agent are also in Today → Approve.

import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { toast, toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading } from "@/components/ui";
import { isStartable, isWaiting, visibleProposals } from "./proposalState";
import { ProposalCard, runProposalAction, useProposalList } from "./ProposalCard";
import type { Proposal } from "@/gen/Proposal";

const KIND: Record<Proposal["body"]["kind"], [string, string]> = {
  launch: ["launch", "launches"], message: ["message", "messages"], schedule: ["scheduled launch", "scheduled launches"], cron: ["routine", "routines"], status: ["status list", "status lists"],
  preset: ["preset", "presets"], tool: ["tool", "tools"], bot: ["bot", "bots"], routine: ["routine", "routines"], bot_instructions: ["BOT.md change", "BOT.md changes"], bot_files: ["bot file change", "bot file changes"], channel: ["channel", "channels"], channel_join: ["join request", "join requests"], ready: ["ready click", "ready clicks"], report: ["report", "reports"],
};

/** "2 launches, 1 tool": what a group action covers. */
export function summary(cards: Proposal[]): string {
  const counts = new Map<string, { n: number; one: string; many: string }>();
  for (const p of cards) { const [one, many] = KIND[p.body.kind]; const c = counts.get(many) ?? { n: 0, one, many }; c.n += 1; counts.set(many, c); }
  return [...counts.values()].map((c) => `${c.n} ${c.n === 1 ? c.one : c.many}`).join(", ");
}

/** Starts or dismisses cards one after another, then says what happened: a card that failed to start (an error, or a
 * `failed` result) does not stop the others. */
export async function runCards(qc: QueryClient, action: "accept" | "dismiss", cards: Proposal[]) {
  let done = 0;
  const errors: string[] = [];
  for (const p of cards) {
    try {
      const next = await runProposalAction<Proposal>(p.id, action);
      if (next.state === "failed") errors.push(next.error ?? "It could not start");
      else done += 1;
    } catch (e) { errors.push(e instanceof Error ? e.message : String(e)); }
  }
  await Promise.all([qc.invalidateQueries({ queryKey: ["proposals"] }), qc.invalidateQueries({ queryKey: ["sessions"] })]);
  const verb = action === "accept" ? "Started" : "Dismissed";
  if (!errors.length) toast(`${verb} ${done} card${done === 1 ? "" : "s"}`);
  else toastError(new Error(`${verb} ${done} of ${cards.length}. ${errors.length} failed: ${errors[0]}${errors.length > 1 ? ` (and ${errors.length - 1} more)` : ""}`));
}

/** Start all: an outline button with one confirmation naming what starts. */
export function StartAll({ cards, className = "" }: { cards: Proposal[]; className?: string }) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const startable = cards.filter(isStartable);
  if (startable.length < 2) return null;
  const run = async () => { setBusy(true); try { await runCards(qc, "accept", startable); } finally { setBusy(false); } };
  return <ConfirmButton tone="publish" className={className} icon="play" disabled={busy} consequence={`Start ${summary(startable)}?`} confirmLabel="Start all" onConfirm={run}>Start all ({startable.length})</ConfirmButton>;
}

/** The cards of a list, with Waiting/All, Start all and Dismiss all. `reveal`: a card to focus (a replacement link). */
export function ProposalList({ proposals, reveal = null, empty }: { proposals: Proposal[]; reveal?: string | null; empty: string }) {
  const qc = useQueryClient();
  const query = useProposalList();
  const [filter, setFilter] = useState<"open" | "all">(reveal ? "all" : "open");
  const [busy, setBusy] = useState(false);
  const focused = useRef<string | null>(null);
  useEffect(() => { if (reveal) setFilter("all"); }, [reveal]);
  useEffect(() => {
    if (!reveal || filter !== "all" || focused.current === reveal) return;
    const card = document.getElementById(`proposal-${reveal}`);
    if (card) { card.focus(); focused.current = reveal; }
  }, [reveal, filter, proposals]);
  const waiting = proposals.filter(isWaiting);
  const shown = visibleProposals(proposals, filter === "open", reveal);
  const dismissable = waiting.filter((p) => p.state !== "uncertain");
  const runAll = async (action: "accept" | "dismiss", cards: Proposal[]) => {
    setBusy(true);
    try { await runCards(qc, action, cards); } finally { setBusy(false); }
  };
  return (
    <section data-tour="proposals.list" className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <FilterChips label="Proposals shown">
          <FilterChip active={filter === "open"} count={waiting.length} onClick={() => setFilter("open")}>Waiting</FilterChip>
          <FilterChip active={filter === "all"} onClick={() => setFilter("all")}>All</FilterChip>
        </FilterChips>
        {dismissable.length > 1 && <ConfirmButton className="ml-auto" disabled={busy} consequence={`Dismiss ${summary(dismissable)}?`} confirmLabel="Dismiss all" onConfirm={() => runAll("dismiss", dismissable)}>Dismiss all ({dismissable.length})</ConfirmButton>}
        <StartAll cards={waiting} className={dismissable.length > 1 ? "" : "ml-auto"} />
      </div>
      {query.isLoading ? <Loading what="proposals" /> : query.error ? <ErrorState error={query.error} onRetry={() => void query.refetch()} /> : shown.length === 0 ? <Empty icon="spark">{filter === "open" ? "No proposal waits for you." : empty}</Empty> : shown.map((p) => <ProposalCard key={p.id} p={p} anchor />)}
    </section>
  );
}
