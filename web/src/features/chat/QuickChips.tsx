// Quick chips over a bot's message box on the phone: short ready-made messages that follow what the workspace
// shows (decisions waiting, agents running). A complete question sends at once; one ending in "…" fills the box
// so you finish it. Plain messages to the bot: no code of their own on Home, and any bot gets them.

import { useTodayHere } from "@/features/today/data";
import { useSessions } from "@/lib/queries";
import { isRunning } from "@/lib/sessionStatus";
import { useScope } from "@/lib/workspace";

type Chip = { label: string; text: string; send: boolean };

function useChips(): Chip[] {
  const scope = useScope();
  const waiting = useTodayHere().items.length;
  const running = (useSessions().data ?? []).filter((s) => scope.hasSession(s) && isRunning(s) && !s.launch_spec.bot).length;
  const folder = scope.projects.filter((p) => !p.is_temp).sort((a, b) => a.name.localeCompare(b.name))[0];
  return [
    waiting > 0 ? { label: `What needs me? · ${waiting}`, text: "What needs me? Go through what waits in Today, most urgent first.", send: true } : { label: "What happened today?", text: "What happened today? Summarize what agents finished and what I handled.", send: true },
    ...(running > 0 ? [{ label: `How are my ${running === 1 ? "agent" : `${running} agents`} doing?`, text: "How are my running agents doing? One line each.", send: true }] : []),
    { label: "Start an agent…", text: folder ? `Start an agent in ${folder.name} to ` : "Start an agent to ", send: false },
    { label: "Status of…", text: "What is the status of ", send: false },
  ];
}

export function QuickChips({ onSend, onFill }: { onSend: (text: string) => void; onFill: (text: string) => void }) {
  const chips = useChips();
  return <div className="flex gap-1.5 overflow-x-auto pb-1.5 [scrollbar-width:none]" role="group" aria-label="Quick messages" data-tour="chat.chips">
    {chips.map((c) => <button key={c.label} className="lp-chip" title={c.send ? `Send: ${c.text}` : "Write the rest of the message"} onClick={() => (c.send ? onSend(c.text) : onFill(c.text))}>{c.label}</button>)}
  </div>;
}
