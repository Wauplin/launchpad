// The Go to dialog (`GoTo.tsx`): Home resolves the pasted link or ID, then the window opens it.

import { useState } from "react";
import type { RefTarget } from "@/gen/RefTarget";
import { Dialog } from "@/features/chats/Dialog";
import { humanError } from "@/components/ui";
import { openTarget, resolveRef } from "@/lib/refActions";
import { describeRef } from "@/lib/refs";

export function GoToDialog({ onClose }: { onClose: () => void }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [found, setFound] = useState<RefTarget | null>(null);
  const go = async () => {
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const t = await resolveRef(text);
      setFound(t);
      openTarget(t);
      onClose();
    } catch (e) {
      setError(humanError(e));
    } finally {
      setBusy(false);
    }
  };
  return <Dialog title="Go to" onClose={onClose}>
    <label className="grid gap-1.5 text-[12px] text-ink-3">
      A Launchpad link or ID: a chat, a message, a channel, an agent, a folio section…
      <input autoFocus className="field-input font-mono text-[12px]" value={text} placeholder="lp:chat/… or https://…/bots/…?thread=…" autoCapitalize="off" autoCorrect="off" spellCheck={false}
        onChange={(e) => { setText(e.target.value); setError(null); }} onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); void go(); } }} />
    </label>
    {error && <p role="alert" className="m-0 text-[12px] text-danger-ink">{error}</p>}
    {found && <p className="m-0 text-[12px] text-ink-3">{describeRef(found.ref)}</p>}
    <span className="flex justify-end gap-1.5">
      <button className="btn btn-sm btn-ghost" onClick={onClose}>Cancel</button>
      <button className="btn btn-sm btn-primary" disabled={busy || !text.trim()} onClick={() => void go()}>{busy ? "Opening…" : "Go to"}</button>
    </span>
  </Dialog>;
}
