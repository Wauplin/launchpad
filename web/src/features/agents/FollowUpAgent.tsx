import type { Session } from "@/gen/Session";
import { isRunning } from "@/lib/sessionStatus";
import { useComposer } from "@/stores/composer";

/** Prepare a new conversation; never send input or change the source session. */
export function FollowUpAgent({ session, onPrepared }: { session: Session; onPrepared?: () => void }) {
  if (isRunning(session) || session.status === "in_terminal") return null;
  return <button className="btn btn-sm" title="Start a separate agent in the same worktree or folder with saved trace context" onClick={() => {
    useComposer.getState().followUpFrom(session);
    onPrepared?.();
  }}>Follow up</button>;
}
