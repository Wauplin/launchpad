// The activity list: what sessions with admin Launchpad CLI access changed directly, with the values before
// (crates/automation/src/activity.rs). The bot chat shows it as Changed lines whose record name links to its page.

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

/** Where the changed record has its page, when it still has one. */
export type ActivityLink =
  | { to: "/bots/$id"; params: { id: string } }
  | { to: "/schedules"; search: { routine: string } | { launch: string } }
  | { to: "/agents"; search: { session: string } }
  | { to: "/projects/$id"; params: { id: string } }
  | { to: "/settings/credentials"; search: { tool: string } };

/** The record an activity changed: its route parameter, or for a create the ID its undo (a delete) names. */
export function activityLink(a: Activity): ActivityLink | null {
  const [group, verb] = a.action.split(".");
  if (verb === "delete" || verb === "cancel") return null;
  const created = a.undo && /\.(delete|cancel)$/.test(a.undo.action) ? a.undo.args[0] : undefined;
  if (a.action === "bot.add-routine") return created ? { to: "/schedules", search: { routine: created } } : null;
  const id = verb === "create" ? created : a.args[0];
  if (!id) return null;
  switch (group) {
    case "bot": return { to: "/bots/$id", params: { id } };
    case "routine": return { to: "/schedules", search: { routine: id } };
    case "schedule": return { to: "/schedules", search: { launch: id } };
    case "session": return { to: "/agents", search: { session: id } };
    case "folder": return { to: "/projects/$id", params: { id } };
    case "tool": return { to: "/settings/credentials", search: { tool: id } };
    default: return null;
  }
}

/** One line, in parts around the record name: what was done and to what, or the record and each changed value
 * before → after. */
export function activityParts(a: Activity): { before: string; target: string | null; after: string } {
  const what = (ACTIONS as Record<string, { help: string }>)[a.action]?.help ?? a.action;
  const body = (a.body ?? {}) as Record<string, unknown>;
  const before = a.before as Record<string, unknown> | null;
  const values = before ? Object.keys(body).map((k) => `${k.replace(/_/g, " ")}: ${short(before[k])} → ${short(body[k])}`) : [];
  if (values.length) return { before: "", target: a.target, after: (a.target ? " · " : "") + values.join(" · ") };
  return { before: a.target ? `${what} · ` : what, target: a.target, after: "" };
}

export const activityLine = (a: Activity): string => { const p = activityParts(a); return `${p.before}${p.target ?? ""}${p.after}`; };
