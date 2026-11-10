// Home: where a session runs, and Move between turns: another connected runner, or Burst (an HF Job).

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { MachineStint } from "@/gen/MachineStint";
import type { Session } from "@/gen/Session";
import { ConfirmButton } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useBurst, useBurstRun, useRunnerTag } from "@/lib/burst";
import { useRunners } from "@/lib/runners";

const BURST = "burst";

export function RunsOn({ session: s }: { session: Session }) {
  const runners = useRunners().data ?? [];
  const burst = useBurst().data;
  const run = useBurstRun(s.runner_id);
  const tag = useRunnerTag(s.runner_id);
  const [to, setTo] = useState("");
  const current = runners.find((r) => r.runner.id === s.runner_id);
  const name = run ? "Burst" : current?.runner.name ?? "a runner";
  const idle = !["starting", "working", "background"].includes(s.status) && !s.open_question && s.status !== "closed" && s.status !== "quota_paused";
  const targets = runners.filter((r) => r.runner.state === "approved" && r.connected && r.runner.id !== s.runner_id && r.compat === "same");
  const canBurst = !burst?.unavailable && !(run && (run.state === "starting" || run.state === "running"));
  const label = to === BURST ? "Burst" : targets.find((r) => r.runner.id === to)?.runner.name ?? "";
  const move = () => api.post(`/api/sessions/${s.id}/move`, { to }).then(() => { toast(to === BURST ? "Starting Burst for this session" : `Moved to ${label}`); setTo(""); }, toastError);
  return (
    <span className="inline-flex flex-wrap items-center gap-2">
      <span title={tag?.title}>{tag && run ? tag.label : name}{tag && !run ? ` · ${tag.label.replace(`${name} `, "")}` : ""}</span>
      {idle && (targets.length > 0 || canBurst) && (
        <>
          {/* A menu, not a native select: the phone's 16 px field rule would make it larger than the text around it. */}
          <Popover viewport align="left" width={240} trigger={({ open, toggle }) => <button className="btn btn-sm" aria-haspopup="menu" aria-expanded={open} onClick={toggle}>{to ? `Move to ${label}` : "Move to…"}<Icon name="chev" size={12} /></button>}>
            {(close) => <>
              {targets.map((r) => <MenuItem key={r.runner.id} active={to === r.runner.id} label={r.runner.name} detail={r.runner.capabilities.vm ? "Local VM" : undefined} onClick={() => { setTo(r.runner.id); close(); }} />)}
              {canBurst && <MenuItem active={to === BURST} label="Burst" detail="An HF Job" onClick={() => { setTo(BURST); close(); }} />}
            </>}
          </Popover>
          {to && <ConfirmButton icon="send" consequence={`The session continues on ${label} from your next message. Its branches stay on Home; uncommitted files go to a scratch branch, and its folders on ${name} are removed.`} confirmLabel={`Move to ${label}`} onConfirm={move}>Move</ConfirmButton>}
        </>
      )}
    </span>
  );
}

/** Where the session runs now: its runner (or Burst job). A record without one cannot continue. */
export function CurrentMachine({ session: s }: { session: Session }) {
  if (s.runner_id) return <RunsOn session={s} />;
  return <span>No runner</span>;
}

/** The machines that ran the turns, in order, when more than one did or the earlier one is not the current one. */
export function useTurnMachines(s: Session): string | null {
  const runners = useRunners().data;
  // Refetch when a turn ends or the session moves.
  const stints = useQuery({ queryKey: ["session-machines", s.id, s.status_since, s.runner_id], queryFn: () => api.get<MachineStint[]>(`/api/sessions/${s.id}/machines`), retry: false }).data ?? [];
  const current = s.runner_id ? runners?.find((r) => r.runner.id === s.runner_id)?.runner.name : null;
  if (!stints.length || stints.length === 1 && (current === undefined || stints[0].runner === current)) return null;
  let turn = 0;
  return stints.map((m) => {
    const range = m.turns === 1 ? `turn ${turn + 1}` : `turns ${turn + 1}–${turn + m.turns}`;
    turn += m.turns;
    return `${m.runner ?? "an earlier machine"} · ${range}`;
  }).join(" → ");
}
