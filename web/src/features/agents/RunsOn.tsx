// Home: where a session runs, and Move between turns: another connected runner, or Burst (an HF Job).

import { useState } from "react";
import type { Session } from "@/gen/Session";
import { ConfirmButton } from "@/components/ui";
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
          <select className="field-input h-7 w-auto py-0 text-xs" aria-label="Move to" value={to} onChange={(e) => setTo(e.target.value)}>
            <option value="">Move to…</option>
            {targets.map((r) => <option key={r.runner.id} value={r.runner.id}>{r.runner.name}{r.runner.capabilities.vm ? " · Local VM" : ""}</option>)}
            {canBurst && <option value={BURST}>Burst · HF Job</option>}
          </select>
          {to && <ConfirmButton icon="send" consequence={`The session continues on ${label} from your next message. Its branches stay on Home; uncommitted files go to a scratch branch, and its folders on ${name} are removed.`} confirmLabel={`Move to ${label}`} onConfirm={move}>Move</ConfirmButton>}
        </>
      )}
    </span>
  );
}
