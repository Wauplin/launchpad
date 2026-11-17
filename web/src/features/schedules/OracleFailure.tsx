// A failing or broken oracle in Today (row details and card) and the one-line answer of a test: kept out of
// Oracles.tsx, so Today (bundled with the laptop entry) does not load the oracle page.

import type { OracleRunResult } from "@/gen/OracleRunResult";
import type { TodayItem } from "@/gen/TodayItem";
import { useBots } from "@/lib/bots";
import { ago, when } from "@/lib/format";
import { OUTCOME_LABEL, broken, problemText, useOracle } from "@/lib/oracles";

/** "True · tests.yml on 4be1c2a: failure" */
export function resultLine(r: Pick<OracleRunResult, "outcome" | "summary" | "error">): string {
  const what = r.outcome === "failed" && r.error ? `${r.error.class}: ${r.error.message}` : r.summary;
  return `${OUTCOME_LABEL[r.outcome]}${what ? ` · ${what}` : ""}`;
}

/** A Today item about a broken oracle (its stored file does not pass the pre-check), not a failing one. */
export const brokenItem = (item: TodayItem) => item.kind === "oracle" && item.detail.startsWith("broken:");

/** A Today card's details of a failing or broken oracle: the error and the failing call, first and last failure, what
 * waits on it; for a broken one, its problems. */
export function OracleFailure({ workspace, slug }: { workspace: string; slug: string }) {
  const d = useOracle(workspace, slug).data;
  const bots = useBots().data ?? [];
  if (!d) return null;
  const s = d.streak;
  const waiting = <>
    <span className="text-ink-3">Waiting on it</span><span>{d.used_by.length ? d.used_by.map((u) => `${bots.find((b) => b.id === u.bot_id)?.settings.name ?? "Routine"} “${u.title}”${u.run_on.includes("failed") ? " (repair run on Failed)" : " (quiet while it fails)"}`).join(", ") : "nothing"}</span>
  </>;
  if (broken(d.oracle)) return <div className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-[12px]">
    <span className="text-ink-3">Broken</span>
    <span className="grid gap-0.5">{d.oracle.problems.map((p, i) => <code key={i} className="font-mono text-[11px] break-words text-gh-closed">{problemText(p)}</code>)}<span className="text-ink-3">Not evaluated until it is saved again.</span></span>
    {waiting}
  </div>;
  if (!s) return null;
  const classes = Object.entries(s.classes).sort((a, b) => b[1] - a[1]);
  return <div className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-[12px]">
    <span className="text-ink-3">Error</span><span className="break-words">{s.last_error ? <><b>{s.last_error.class}</b> · {s.last_error.message}</> : "—"}{classes.length > 0 && <span className="text-ink-3"> ({classes.map(([c, n]) => `${n} ${c}`).join(", ")} of {s.count})</span>}</span>
    {s.last_call && <><span className="text-ink-3">Failing call</span><code className="font-mono text-[11px] break-all">{s.last_call}</code></>}
    <span className="text-ink-3">First / last</span><span>{s.first_at ? when(s.first_at) : "—"} / {s.last_at ? when(s.last_at) : "—"}</span>
    {waiting}
    {s.last_good_at && <><span className="text-ink-3">Last good</span><span>{ago(s.last_good_at)}</span></>}
  </div>;
}
