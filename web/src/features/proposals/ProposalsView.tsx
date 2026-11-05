// Proposals: the inert cards agents write with the Launchpad CLI tool (`launchpad propose`), for this window's
// workspace. Nothing starts until the owner clicks Start (or Send, Schedule, Create routine), except one-time
// starts of a bot in auto mode. A bot's page also lists the cards its sessions wrote.

import { useQueryClient } from "@tanstack/react-query";
import { useSearch } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader } from "@/components/ui";
import { api } from "@/lib/api";
import { browserZone } from "@/lib/routines";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { useWorkspace } from "@/stores/workspace";
import { isWaiting, visibleProposals } from "./proposalState";
import { ProposalCard, useProposalList } from "./ProposalCard";
import type { Proposal } from "@/gen/Proposal";

const KIND: Record<Proposal["body"]["kind"], [string, string]> = {
  launch: ["launch", "launches"], message: ["message", "messages"], schedule: ["scheduled launch", "scheduled launches"], cron: ["routine", "routines"], status: ["status list", "status lists"],
  preset: ["preset", "presets"], tool: ["tool", "tools"], bot: ["bot", "bots"], routine: ["routine", "routines"], bot_instructions: ["BOT.md change", "BOT.md changes"],
};

/** "2 launches, 1 tool": what a group action covers. */
function summary(cards: Proposal[]): string {
  const counts = new Map<string, { n: number; one: string; many: string }>();
  for (const p of cards) { const [one, many] = KIND[p.body.kind]; const c = counts.get(many) ?? { n: 0, one, many }; c.n += 1; counts.set(many, c); }
  return [...counts.values()].map((c) => `${c.n} ${c.n === 1 ? c.one : c.many}`).join(", ");
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
  // Every kind starts; a status list has nothing to start. Uncertain cards wait for a check in Agents.
  const startable = waiting.filter((p) => p.state === "proposed" && p.body.kind !== "status");
  const dismissable = waiting.filter((p) => p.state !== "uncertain");
  const runAll = async (action: "accept" | "dismiss", cards: Proposal[]) => {
    setBusy(true);
    try {
      for (const p of cards) {
        // A routine or cron keeps the time zone of the browser that accepts it.
        try { await api.post(`/api/proposals/${p.id}/${action}${action === "accept" ? `?time_zone=${encodeURIComponent(browserZone())}` : ""}`); }
        catch (e) { toastError(e); }
      }
      await Promise.all([qc.invalidateQueries({ queryKey: ["proposals"] }), qc.invalidateQueries({ queryKey: ["sessions"] })]);
    } finally { setBusy(false); }
  };
  return (
    <section data-tour="proposals.list" className="grid gap-2 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <FilterChips label="Proposals shown">
          <FilterChip active={filter === "open"} count={waiting.length} onClick={() => setFilter("open")}>Waiting</FilterChip>
          <FilterChip active={filter === "all"} onClick={() => setFilter("all")}>All</FilterChip>
        </FilterChips>
        {dismissable.length > 1 && <ConfirmButton className="ml-auto" disabled={busy} consequence={`Dismiss ${summary(dismissable)}?`} confirmLabel="Dismiss all" onConfirm={() => runAll("dismiss", dismissable)}>Dismiss all ({dismissable.length})</ConfirmButton>}
        {startable.length > 1 && <ConfirmButton tone="publish" className={dismissable.length > 1 ? "btn-primary" : "btn-primary ml-auto"} icon="play" disabled={busy} consequence={`Start ${summary(startable)}?`} confirmLabel="Start all" onConfirm={() => runAll("accept", startable)}>Start all ({startable.length})</ConfirmButton>}
      </div>
      {query.isLoading ? <Loading what="proposals" /> : query.error ? <ErrorState error={query.error} onRetry={() => void query.refetch()} /> : shown.length === 0 ? <Empty icon="spark">{filter === "open" ? "No proposal waits for you." : empty}</Empty> : shown.map((p) => <ProposalCard key={p.id} p={p} anchor />)}
    </section>
  );
}

export function ProposalsView() {
  const phone = usePhone();
  const workspace = useWorkspace((s) => s.current) ?? null;
  const scope = useScope();
  const search = useSearch({ strict: false }) as { proposal?: string };
  const proposals = (useProposalList().data ?? []).filter((p) => !workspace || (p.workspace_id ?? null) === workspace);
  const header = phone
    ? <PhoneHeader title="Proposals" subtitle={scope.workspace?.name ?? "Everything"} />
    : <PageHeader icon={<Icon name="spark" size={18} className="text-teal-ink" />} title="Proposals" meta={<>{scope.workspace?.name ?? "Everything"} · cards your bots and agents wrote with the Launchpad CLI; nothing starts without your click</>} />;
  return (
    <div className="flex h-full min-h-0 flex-col">
      {header}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-[720px]">
          <ProposalList proposals={proposals} reveal={search.proposal ?? null} empty="No proposal yet. Ask Launchpad (in Bots) turns your requests into proposals." />
        </div>
      </div>
    </div>
  );
}
