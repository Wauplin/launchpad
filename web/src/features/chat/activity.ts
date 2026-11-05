// The activity list: what sessions with admin Launchpad CLI access changed directly, with the values before and
// the call that undoes it (crates/automation/src/activity.rs). The bot chat shows it as Changed lines with Undo.

import { useQuery } from "@tanstack/react-query";
import type { Activity } from "@/gen/Activity";
import { ACTIONS } from "@/gen/actions";
import { api } from "@/lib/api";

export const useActivity = (workspace: string) =>
  useQuery({ queryKey: ["activity", workspace], queryFn: () => api.get<Activity[]>(`/api/activity?workspace=${encodeURIComponent(workspace)}`), staleTime: 30_000 });

const short = (v: unknown): string => {
  const s = typeof v === "string" ? `“${v}”` : JSON.stringify(v) ?? "nothing";
  return s.length > 40 ? `${s.slice(0, 39)}…` : s;
};

/** One line: the record and each changed value before → after; or what was done, to what. */
export function activityLine(a: Activity): string {
  const what = (ACTIONS as Record<string, { help: string }>)[a.action]?.help ?? a.action;
  const body = (a.body ?? {}) as Record<string, unknown>;
  const before = a.before as Record<string, unknown> | null;
  const values = before ? Object.keys(body).map((k) => `${k.replace(/_/g, " ")}: ${short(before[k])} → ${short(body[k])}`) : [];
  return values.length ? [a.target, ...values].filter(Boolean).join(" · ") : [what, a.target].filter(Boolean).join(" · ");
}
