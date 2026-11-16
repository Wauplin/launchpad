// A failing oracle in Today (row details and card) and the one-line answer of a test: kept out of Oracles.tsx, so
// Today (bundled with the laptop entry) does not load the oracle page.

import type { OracleRunResult } from "@/gen/OracleRunResult";
import { useBots } from "@/lib/bots";
import { ago, when } from "@/lib/format";
import { OUTCOME_LABEL, useOracle } from "@/lib/oracles";

/** "True · tests.yml on 4be1c2a: failure" */
export function resultLine(r: Pick<OracleRunResult, "outcome" | "summary" | "error">): string {
  const what = r.outcome === "failed" && r.error ? `${r.error.class}: ${r.error.message}` : r.summary;
  return `${OUTCOME_LABEL[r.outcome]}${what ? ` · ${what}` : ""}`;
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
