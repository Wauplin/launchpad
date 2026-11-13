// The phone's More tab: every place without a tab of its own, each with what waits there (bots are in Chats; the
// workspace switch is the top bar's). The laptop sidebar holds the same places.

import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { useSetupSummary } from "@/features/setup/summary";
import { useTerminalsHere } from "@/features/terminals/lib";
import { useTodayHere } from "@/features/today/data";
import { useKnowledgeUnseen } from "@/lib/knowledge";

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
  const failed = useTodayHere().items.filter((i) => i.kind === "routine").length;
  const knowledgeNew = useKnowledgeUnseen();
  const terminalsHere = useTerminalsHere();
  const setupLeft = useSetupSummary().data?.required_left ?? 0;
  return (
    <div className="h-full overflow-y-auto" data-tour="phone.more">
      <nav aria-label="Places">
        <Place to="/schedules" icon="clock" label="Schedules" attn={failed > 0} state={failed ? `${failed} failed` : undefined} />
        <Place to="/projects" icon="folder" label="Folders" />
        <Place to="/topics" icon="book" label="Topics" />
        <Place to="/knowledge" icon="book" label="Knowledge" state={knowledgeNew ? `${knowledgeNew} new` : undefined} />
        {terminalsHere && <Place to="/terminals" icon="term" label="Terminals" />}
        <Place to="/settings" icon="sliders" label="Settings" />
        {setupLeft > 0 && <Place to="/setup" icon="check" label="Finish setup" attn state={`${plural(setupLeft, "step")} left`} />}
      </nav>
    </div>
  );
}
