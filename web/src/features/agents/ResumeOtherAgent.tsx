import type { Session } from "@/gen/Session";
import { Popover } from "@/components/Popover";
import { isRunning } from "@/lib/sessionStatus";
import { useComposer } from "@/stores/composer";

/** Prepare a new conversation; never send input or change the source session. */
export function ResumeOtherAgent({ session }: { session: Session }) {
  const draftOpen = useComposer((s) => s.open);
  if (isRunning(session) || session.status === "in_terminal") return null;
  return <Popover align="right" width={300} trigger={({ open, toggle }) => <button className="btn btn-sm" aria-expanded={open} disabled={session.last_seq === 0} title="Continue in a new conversation using this agent’s saved trace" onClick={toggle}>Resume with another agent</button>}>
    {(close) => <div className="grid gap-2 p-2 text-xs">
      <p>Prepare a continuation prompt with this saved trace in the same folder. Choose the harness and model in the new draft, then click Launch.</p>
      {session.status === "quota_paused" && <p className="text-attn">This agent stays quota paused and may automatically continue. Stop it first to cancel that continuation.</p>}
      {draftOpen && <p className="text-attn">This replaces your current unsent draft.</p>}
      <div className="flex flex-wrap gap-2"><button className="btn btn-sm btn-primary" onClick={() => { useComposer.getState().resumeFrom(session); close(); }}>{draftOpen ? "Replace draft" : "Prepare draft"}</button><button className="btn btn-sm" onClick={close}>Cancel</button></div>
    </div>}
  </Popover>;
}
