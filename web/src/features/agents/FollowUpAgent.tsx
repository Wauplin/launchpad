import type { Session } from "@/gen/Session";
import { Popover } from "@/components/Popover";
import { isRunning } from "@/lib/sessionStatus";
import { useComposer } from "@/stores/composer";

/** Prepare a new conversation; never send input or change the source session. */
export function FollowUpAgent({ session, inline = false, onPrepared }: { session: Session; inline?: boolean; onPrepared?: () => void }) {
  const draftOpen = useComposer((s) => s.open);
  if (isRunning(session) || session.status === "in_terminal") return null;
  const content = (close: () => void) => <div className="grid gap-2 p-2 text-xs">
      <p>Start a separate agent in the same worktree or folder, with the saved trace as context when available. Prepare a review or write a new task, choose the agent and model, then click Launch.</p>
      {session.status === "quota_paused" && <p className="text-attn">This agent stays quota paused and may automatically continue. Stop it first to cancel that continuation.</p>}
      {draftOpen && <p className="text-attn">This replaces your current unsent draft.</p>}
      <div className="flex flex-wrap gap-2"><button className="btn btn-sm btn-primary" onClick={() => { useComposer.getState().followUpFrom(session, true); close(); onPrepared?.(); }}>{draftOpen ? "Replace draft with review" : "Prepare review"}</button><button className="btn btn-sm" onClick={() => { useComposer.getState().followUpFrom(session); close(); onPrepared?.(); }}>{draftOpen ? "Replace draft with new task" : "New task"}</button>{!inline && <button className="btn btn-sm" onClick={close}>Cancel</button>}</div>
    </div>;
  if (inline) return <section className="rounded-lg border border-line"><h3 className="px-2 pt-2 text-sm font-semibold">Follow-up agent</h3>{content(() => {})}</section>;
  return <Popover align="right" width={300} trigger={({ open, toggle }) => <button className="btn btn-sm" aria-expanded={open} title="Launch a separate agent in the same worktree or folder" onClick={toggle}>Follow-up agent</button>}>
    {content}
  </Popover>;
}
