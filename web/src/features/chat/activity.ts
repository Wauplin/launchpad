// The activity list: what sessions with admin Launchpad CLI access changed directly, with the values before
// (crates/automation/src/activity.rs). The bot chat shows each as a receipt on its turn's steps line ("changed
// routine Daily check", linking to the record) and the full line, with the values before, when the steps are open.

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
  let body = (a.body ?? {}) as Record<string, unknown>;
  let before = a.before as Record<string, unknown> | null;
  // bot.update with `set`: each bot.toml field changed.
  if (body.set && typeof body.set === "object" && Object.keys(body).length === 1) { body = body.set as Record<string, unknown>; before = (before?.set ?? null) as Record<string, unknown> | null; }
  const values = before ? Object.keys(body).map((k) => `${k.replace(/_/g, " ")}: ${short(before[k])} → ${short(body[k])}`) : [];
  if (values.length) return { before: "", target: a.target, after: (a.target ? " · " : "") + values.join(" · ") };
  return { before: a.target ? `${what} · ` : what, target: a.target, after: "" };
}

export const activityLine = (a: Activity): string => { const p = activityParts(a); return `${p.before}${p.target ?? ""}${p.after}`; };

/** The record kind of an action group, for receipts. */
const NOUN: Record<string, string> = { bot: "bot", routine: "routine", schedule: "scheduled launch", session: "agent", folder: "folder", tool: "tool", tools: "default tools", mcp: "MCP tool", workspace: "workspace", settings: "settings", proposal: "card", home: "Home" };

/** A short receipt of a direct change: "changed routine Daily check", "added routine Review requests". */
export function activityReceipt(a: Pick<Activity, "action" | "target" | "body">): string {
  const [group, verb = ""] = a.action.split(".");
  if (a.action === "bot.add-routine") return `added a routine${a.target ? ` to ${a.target}` : ""}`;
  const did = verb === "create" || verb.startsWith("add") ? "added" : verb === "delete" ? "deleted" : verb === "cancel" ? "cancelled" : verb === "pause" ? ((a.body as { paused?: boolean } | null)?.paused === false ? "resumed" : "paused") : verb === "run" || verb === "start" ? "started" : verb === "message" ? "messaged" : verb === "launch" ? "launched" : "changed";
  const noun = NOUN[group] ?? group;
  return `${did} ${noun}${a.target ? ` ${a.target}` : ""}`;
}
