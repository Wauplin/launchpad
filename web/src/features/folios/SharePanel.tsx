// Share a snapshot (plan §12): a read-only static copy of one rev as a Hugging Face Space under the owner's namespace,
// private by default, expiring after 30 days unless kept. Publishing is the owner's click (`folio.share`, with its
// confirmation; an agent can only prepare it as a ready card). Comments, verdicts and the data API stay on Home.

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { Folio } from "@/gen/Folio";
import { ActionButton } from "@/components/ActionButton";
import { Choice, Choices } from "@/components/ui";
import { toast } from "@/components/Toast";
import { ago, when } from "@/lib/format";
import { openExternal } from "@/lib/platform";

export function SharePanel({ f, rev }: { f: Folio; rev: number }) {
  const qc = useQueryClient();
  const s = f.share;
  const [visibility, setVisibility] = useState<"private" | "public">((s?.visibility as "private" | "public") ?? "private");
  const [keep, setKeep] = useState(s ? s.expires_at == null : false);
  const [decisions, setDecisions] = useState(s?.decisions ?? false);
  const refresh = () => void qc.invalidateQueries({ queryKey: ["folio", f.workspace_id, f.slug] }).then(() => qc.invalidateQueries({ queryKey: ["folios", f.workspace_id] }));
  const params = { workspace: f.workspace_id, slug: f.slug };
  return <div className="grid gap-3 p-3 text-[13px]">
    <p className="m-0 text-[12px] text-ink-3">Renders rev {rev} as a read-only static Space under your Hugging Face account. Comments, verdicts and the data API stay on Home.</p>
    {s && <div className={`grid gap-1 rounded-card border bg-paper p-2.5 ${s.state === "failed" ? "border-danger" : "border-line-2"}`}>
      <b className="truncate">{s.space}{s.dry_run ? " (local folder, dry run)" : ""}</b>
      <span className="text-[12px] text-ink-3">rev {s.rev} · {s.visibility} · shared {ago(s.at)} · {s.expires_at == null ? "kept until you stop sharing" : `deleted ${when(s.expires_at)}`}</span>
      {s.state === "failed" && <span className="text-[12px] text-danger">{s.error}</span>}
      {s.state === "sharing" && <span className="text-[12px] text-attn">Publishing… (if this stays, the last publish was interrupted: publish again)</span>}
      <div className="flex flex-wrap gap-1.5 pt-1">
        {!s.dry_run && <button type="button" className="btn btn-sm" onClick={() => openExternal(s.url)}>Open the Space</button>}
        {s.expires_at != null && <ActionButton name="folio.share-extend" params={params} body={{ days: 30 }} onDone={() => { toast("Kept 30 more days"); refresh(); }}>Extend 30 days</ActionButton>}
        {s.expires_at != null && <ActionButton name="folio.share-extend" params={params} body={{ keep: true }} onDone={() => { toast("Kept until you stop sharing"); refresh(); }}>Keep</ActionButton>}
        <ActionButton name="folio.unshare" params={params} onDone={() => { toast("Stopped sharing: the Space is deleted"); refresh(); }} />
      </div>
    </div>}
    <label className="grid gap-1 text-[12px] text-ink-3">Space
      <input className="field-input" readOnly value={s?.space ?? `<you>/folio-${f.slug}`} />
    </label>
    <Choices label="Visibility">
      <Choice active={visibility === "private"} onClick={() => setVisibility("private")} title="Only you (and your organizations' members you allow) can open it">Private</Choice>
      <Choice active={visibility === "public"} onClick={() => setVisibility("public")} title="Anyone with the link can read it">Public</Choice>
    </Choices>
    <Choices label="Expiry">
      <Choice active={!keep} onClick={() => setKeep(false)}>Delete after 30 days</Choice>
      <Choice active={keep} onClick={() => setKeep(true)}>Keep</Choice>
    </Choices>
    <label className="flex min-h-8 items-center gap-2"><input type="checkbox" checked={decisions} onChange={(e) => setDecisions(e.target.checked)} />Show your decisions as text</label>
    <div className="flex justify-end">
      <ActionButton name="folio.share" params={params} vars={{ rev: String(rev) }} primary body={{ visibility, keep, days: 30, decisions, rev }} onDone={() => { toast(s ? "Snapshot updated" : "Snapshot published"); refresh(); }}>{s ? "Update snapshot" : "Publish snapshot"}</ActionButton>
    </div>
  </div>;
}
