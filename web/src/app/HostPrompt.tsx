// An `ssh` question for a host (password, key passphrase, unknown host key), shown wherever the owner is.
// The answer goes straight to ssh through the daemon and is never stored.

import { useState } from "react";
import type { HostStatus } from "@/gen/HostStatus";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useHosts } from "@/lib/hosts";

function Prompt({ h }: { h: HostStatus }) {
  const p = h.prompt!;
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const send = (answer: string | null) => {
    setBusy(true);
    api.post(`/api/hosts/${h.host.id}/prompt`, { prompt_id: p.id, answer }).catch(toastError).finally(() => setBusy(false));
  };
  return (
    <div className="grid gap-2.5 rounded-[10px] border border-line bg-paper p-4 text-[12.5px] shadow-[0_12px_32px_-12px_rgba(23,32,43,.45)]">
      <b className="font-[650]">ssh asks about {h.host.id}</b>
      <pre className="m-0 max-h-48 overflow-auto rounded bg-chrome p-2 font-mono text-xs whitespace-pre-wrap">{p.text}</pre>
      {p.secret ? (
        <form className="grid gap-2" onSubmit={(e) => { e.preventDefault(); send(value); }}>
          <input className="field-input" type="password" autoFocus autoComplete="off" value={value} onChange={(e) => setValue(e.target.value)} aria-label="Answer" />
          <span className="text-xs text-ink-3">Sent to ssh only; Launchpad does not store it.</span>
          <span className="flex justify-end gap-1.5">
            <button type="button" className="btn btn-sm" disabled={busy} onClick={() => send(null)}>Cancel</button>
            <button type="submit" className="btn btn-sm btn-primary" disabled={busy}>Send</button>
          </span>
        </form>
      ) : (
        <span className="flex justify-end gap-1.5">
          <button className="btn btn-sm" disabled={busy} onClick={() => send("no")}>No</button>
          <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => send("yes")}>Yes, continue</button>
        </span>
      )}
    </div>
  );
}

export function HostPrompts() {
  const pending = useHosts().filter((h) => h.prompt);
  if (pending.length === 0) return null;
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/30 p-4" role="dialog" aria-modal="true" aria-label="SSH question">
      <div className="grid w-full max-w-[480px] gap-3">
        {pending.map((h) => <Prompt key={h.prompt!.id} h={h} />)}
      </div>
    </div>
  );
}
