import { useState } from "react";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { enablePush, usePushState, useRefreshPush } from "@/lib/push";

const DISMISSED = "launchpad-push-prompt-dismissed";

function dismissed() {
  try { return localStorage.getItem(DISMISSED) === "1"; } catch { return true; }
}

/** One-time dismissible banner offering push notifications in this browser; Settings → Notifications has the full controls. */
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
    <div role="region" aria-label="Notifications offer" className="flex flex-none flex-wrap items-center gap-2 border-b border-line-2 bg-teal-soft px-3.5 py-1.5 text-[13px] text-teal-ink">
      <Icon name="bell" size={14} />
      <span className="min-w-0 flex-1">Get notified when agents finish or need you, in this browser.</span>
      <button type="button" className="btn btn-sm btn-primary" disabled={busy} onClick={() => void turnOn()}>Turn on</button>
      <button type="button" className="btn btn-sm btn-ghost" title="Settings → Notifications keeps the setting" onClick={dismiss}>Not now</button>
    </div>
  );
}
