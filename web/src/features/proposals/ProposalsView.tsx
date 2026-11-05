// Proposals: the inert cards agents write with the Launchpad CLI tool (`launchpad propose`), for this window's
// workspace. Nothing starts until the owner clicks Start (or Send, Schedule, Create routine), except one-time
// starts of a bot in auto mode. A bot's page also lists the cards its sessions wrote.

import { useQueryClient } from "@tanstack/react-query";
import { useSearch } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader } from "@/components/ui";
import { api } from "@/lib/api";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { useWorkspace } from "@/stores/workspace";
import { isWaiting, visibleProposals } from "./proposalState";
import { ProposalCard, useProposalList } from "./ProposalCard";
import type { Proposal } from "@/gen/Proposal";

/** The cards of a list, with Waiting/All and Start all. `reveal`: a card to focus (a replacement link). */
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
  const startable = waiting.filter((p) => p.state === "proposed" && p.body.kind === "launch");
  const startAll = async () => {
    setBusy(true);
    try { for (const p of startable) await api.post(`/api/proposals/${p.id}/accept`); await Promise.all([qc.invalidateQueries({ queryKey: ["proposals"] }), qc.invalidateQueries({ queryKey: ["sessions"] })]); }
    catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <section data-tour="proposals.list" className="grid gap-2 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <FilterChips label="Proposals shown">
          <FilterChip active={filter === "open"} count={waiting.length} onClick={() => setFilter("open")}>Waiting</FilterChip>
          <FilterChip active={filter === "all"} onClick={() => setFilter("all")}>All</FilterChip>
        </FilterChips>
        {startable.length > 1 && <button className="btn btn-sm btn-primary ml-auto" disabled={busy} onClick={() => void startAll()}><Icon name="play" size={12} />Start all ({startable.length})</button>}
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
