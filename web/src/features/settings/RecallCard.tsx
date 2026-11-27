// Settings → Home and runners: Recall, the search index over each workspace's past sessions (launchpad_recall).
// On or off for Home, and for the window's workspace: how far its index is, what it left out, Backfill and Rebuild.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ActionButton } from "@/components/ActionButton";
import { toast, toastError } from "@/components/Toast";
import { Choice, Choices, ErrorState, Loading, SettingRow } from "@/components/ui";
import { runAction } from "@/lib/actions";
import { api } from "@/lib/api";
import { useWorkspace } from "@/stores/workspace";
import type { RecallConfig } from "@/gen/RecallConfig";
import { Card } from "./SettingsView";

/** `GET /api/recall/{workspace}/status` (the fields this card shows). */
type RecallStatus = {
  available: boolean;
  problem: string | null;
  kept_indexed: boolean;
  backfill_running: boolean;
  sessions: { total: number; up_to_date: number; to_index: number; running: number; left_out: Record<string, number> };
  index: null | { sessions: number; turns: number; blocks_truncated: number; sessions_failed: number; newest_turn: string | null; size_bytes: number };
  backfill: null | { state: string; counts: { sessions_seen: number; sessions_indexed: number; turns_indexed: number; sessions_failed: number } };
};

const megabytes = (bytes: number) => `${(bytes / 1_000_000).toFixed(bytes < 10_000_000 ? 1 : 0)} MB`;

export function RecallCard() {
  const qc = useQueryClient();
  const ws = useWorkspace((s) => s.current);
  const config = useQuery({ queryKey: ["recall-config"], queryFn: () => api.get<RecallConfig>("/api/recall/config") });
  const on = config.data?.enabled ?? false;
  const status = useQuery({ queryKey: ["recall-status", ws], queryFn: () => api.get<RecallStatus>(`/api/recall/${ws}/status`), enabled: !!ws && on, refetchInterval: (q) => (q.state.data?.backfill_running ? 5000 : false) });
  if (config.isLoading) return <Card title="Recall"><Loading what="Recall" /></Card>;
  if (config.error || !config.data) return <Card title="Recall"><ErrorState error={config.error} /></Card>;
  const set = (enabled: boolean) => runAction<RecallConfig>("recall.set", {}, { enabled }).then((c) => { qc.setQueryData(["recall-config"], c); void qc.invalidateQueries({ queryKey: ["recall-status"] }); }, toastError);
  const s = status.data;
  const refresh = () => void qc.invalidateQueries({ queryKey: ["recall-status"] });
  const leftOut = s ? Object.entries(s.sessions.left_out).map(([reason, n]) => `${n} ${reason}`).join(", ") : "";
  return (
    <Card title="Recall">
      <span className="text-[13px] text-ink-2">A search index over the past sessions of each workspace, built on Home. Agents search it with <code>launchpad recall</code>; a bot needs <code>recall = true</code> in its bot.toml. Results are derived evidence: the knowledge base, bot memory and instructions stay the authority. The index can always be built again from the stored sessions.</span>
      <SettingRow label="Recall" hint={on && s?.problem ? s.problem : "Off: Home indexes nothing and answers no search. The indexes stay."}>
        <Choices label="Recall">
          <Choice active={on} onClick={() => !on && void set(true)}>On</Choice>
          <Choice active={!on} onClick={() => on && void set(false)}>Off</Choice>
        </Choices>
      </SettingRow>
      {on && ws && (
        <SettingRow
          label="This workspace"
          hint={s ? [
            `${s.sessions.total} sessions: ${s.sessions.up_to_date} indexed`,
            s.sessions.to_index > 0 ? `${s.sessions.to_index} to index` : "",
            s.sessions.running > 0 ? `${s.sessions.running} running` : "",
            leftOut ? `left out: ${leftOut}` : "",
            s.index?.sessions_failed ? `${s.index.sessions_failed} failed` : "",
            s.index ? `${s.index.turns} turns, ${megabytes(s.index.size_bytes)}${s.index.newest_turn ? `, newest ${s.index.newest_turn}` : ""}` : "nothing indexed yet",
            s.backfill_running ? "a backfill runs now" : "",
          ].filter(Boolean).join(" · ") : status.error ? String((status.error as Error).message ?? status.error) : "Reading the index…"}
        >
          <span className="flex flex-wrap items-center gap-2">
            <ActionButton name="recall.backfill" params={{ workspace: ws }} disabled={!s?.available || s?.backfill_running} onDone={() => { toast("Backfill started on Home"); refresh(); }} />
            {s?.index && <ActionButton name="recall.rebuild" params={{ workspace: ws }} disabled={!s.available || s.backfill_running} onDone={() => { toast("Rebuild started on Home"); refresh(); }} />}
          </span>
        </SettingRow>
      )}
    </Card>
  );
}
