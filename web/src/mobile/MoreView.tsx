// The phone's More tab: every place without a tab of its own, each with what waits there, and the workspace.
// The laptop sidebar holds the same places.

import { Link } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { useSetupSummary } from "@/features/setup/summary";
import { useTerminalsHere } from "@/features/terminals/lib";
import { useTodayHere } from "@/features/today/data";
import { botState, botThread, useBots } from "@/lib/bots";
import { useKnowledgeUnseen } from "@/lib/knowledge";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { WorkspaceSheet } from "./WorkspaceSheet";

/** A place, with a short state (`attn`: something needs you). */
function Place({ to, icon, label, state, attn, tour }: { to: string; icon: IconName; label: string; state?: ReactNode; attn?: boolean; tour?: string }) {
  return <Link to={to} data-tour={tour} className="flex min-h-12 items-center gap-3 border-b border-line-2 px-4 text-[15px] text-ink no-underline">
    <Icon name={icon} size={18} className="text-ink-3" />
    <span className="min-w-0 flex-1 truncate">{label}</span>
    {state && <span className={`shrink-0 text-[13px] ${attn ? "font-semibold text-attn" : "text-ink-3"}`}>{state}</span>}
    <Icon name="chev" size={14} className="-rotate-90 text-ink-3" />
  </Link>;
}

function plural(n: number, one: string, many = `${one}s`) { return `${n} ${n === 1 ? one : many}`; }

export function MoreView() {
  const scope = useScope();
  const [sheet, setSheet] = useState(false);
  const sessions = useSessions().data ?? [];
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === scope.workspace.id);
  const states = bots.map((b) => botState(botThread(sessions, b.id)));
  const needs = states.filter((s) => s === "needs").length;
  const working = states.filter((s) => s === "running").length;
  const failed = useTodayHere().items.filter((i) => i.kind === "routine").length;
  const knowledgeNew = useKnowledgeUnseen();
  const terminalsHere = useTerminalsHere();
  const setupLeft = useSetupSummary().data?.required_left ?? 0;
  return (
    <div className="h-full overflow-y-auto" data-tour="phone.more">
      <nav aria-label="Places">
        <Place to="/bots" icon="spark" label="Bots" attn={needs > 0} state={needs ? `${needs} need${needs === 1 ? "s" : ""} you` : working ? `${working} working` : undefined} />
        <Place to="/schedules" icon="clock" label="Schedules" attn={failed > 0} state={failed ? `${failed} failed` : undefined} />
        <Place to="/projects" icon="folder" label="Folders" />
        <Place to="/knowledge" icon="book" label="Knowledge" state={knowledgeNew ? `${knowledgeNew} new` : undefined} />
        {terminalsHere && <Place to="/terminals" icon="term" label="Terminals" />}
        <Place to="/settings" icon="sliders" label="Settings" />
        {setupLeft > 0 && <Place to="/setup" icon="check" label="Finish setup" attn state={`${plural(setupLeft, "step")} left`} />}
      </nav>
      <button className="flex min-h-12 w-full items-center gap-3 px-4 text-left text-[15px]" onClick={() => setSheet(true)}>
        <span className="min-w-0 flex-1 truncate text-ink-2">Workspace: <b className="text-ink">{scope.workspace.name}</b></span>
        <span className="text-teal-ink">Change</span>
      </button>
      {sheet && <WorkspaceSheet onClose={() => setSheet(false)} />}
    </div>
  );
}
