import { useState } from "react";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { enablePush, usePushState, useRefreshPush } from "@/lib/push";

const DISMISSED = "launchpad-push-prompt-dismissed";

function dismissed() {
  try { return localStorage.getItem(DISMISSED) === "1"; } catch { return true; }
}

/** One-time offer to turn on push notifications in this browser; Settings → Devices has the full controls. */
export function NotificationsPrompt() {
  const push = usePushState().data;
  const refresh = useRefreshPush();
  const [hidden, setHidden] = useState(dismissed);
  const [busy, setBusy] = useState(false);
  if (hidden || !push || push.support !== "supported" || push.permission !== "default" || push.subscribed) return null;
  const dismiss = () => {
    try { localStorage.setItem(DISMISSED, "1"); } catch { /* Hidden for this page only. */ }
    setHidden(true);
  };
  const turnOn = async () => {
    setBusy(true);
    try { await enablePush(false); toast("Notifications turned on on this browser"); dismiss(); }
    catch (error) { toastError(error); }
    finally { setBusy(false); await refresh(); }
  };
  return (
    <span className="flex shrink-0 items-center">
      <button type="button" className="btn btn-ghost" disabled={busy} aria-label="Turn on notifications" title="Turn on notifications on this browser: agents that finish or ask a question" onClick={() => void turnOn()}>
        <Icon name="bell" size={14} />
      </button>
      <button type="button" className="btn btn-ghost px-1!" aria-label="Dismiss the notifications offer" title="Not now (Settings → Devices keeps the setting)" onClick={dismiss}>
        <Icon name="x" size={12} />
      </button>
    </span>
  );
}
