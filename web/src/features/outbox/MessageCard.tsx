// An Outbox message: what an agent drafted for an outgoing channel (Slack, Discord, webhooks), on its Today card.
// Sending is never automatic and never retried: a send that did not complete shows "uncertain" until the owner
// checks the destination.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { memo, useEffect, useState, type ReactNode } from "react";
import type { OutboxMessage } from "@/gen/OutboxMessage";
import { Icon } from "@/components/Icon";
import { toast } from "@/components/Toast";
import { ActionButton } from "@/components/ActionButton";
import { Badge, ErrorState, Token } from "@/components/ui";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { usePhone } from "@/lib/phone";

export const useOutboxMessages = () => useQuery({ queryKey: ["outbox"], queryFn: () => api.get<OutboxMessage[]>("/api/outbox"), staleTime: 30_000 });

const KIND_LABEL: Record<OutboxMessage["kind"], string> = { slack: "Slack", discord: "Discord", generic: "Webhook" };

export function SessionLink({ id, children }: { id: string | null; children: ReactNode }) {
  if (!id) return <span>{children}</span>;
  return <Link to="/agents" search={{ session: id } as never} className="text-teal-ink no-underline hover:underline">{children}</Link>;
}

export const MessageCard = memo(function MessageCard({ m }: { m: OutboxMessage }) {
  const qc = useQueryClient();
  const phone = usePhone();
  const [body, setBody] = useState(m.body);
  const [title, setTitle] = useState(m.title ?? "");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { setBody(m.body); setTitle(m.title ?? ""); }, [m.body, m.title]);
  const draft = m.status === "draft";
  const edited = body !== m.body || title !== (m.title ?? "");
  const refresh = () => qc.invalidateQueries({ queryKey: ["outbox"] });
  const save = async () => { if (edited) await api.patch(`/api/outbox/${m.id}`, { body, title: title.trim() || null }); };
  const run = async (action: "save" | "mark-sent") => {
    setBusy(true); setError(null);
    try {
      if (action === "save") await save();
      if (action === "mark-sent") await api.post(`/api/outbox/${m.id}/mark-sent`);
    } catch (e) { setError(e); }
    finally { setBusy(false); void refresh(); }
  };
  const failed = (e: unknown) => { setError(e); void refresh(); };
  const uncertain = m.status === "uncertain" || m.status === "sending";
  return (
    <article className="rounded-card border border-line bg-paper p-4" data-tour="outbox.card">
      <div className="mb-3 flex flex-wrap items-center gap-2 text-[12px] text-ink-3">
        <Token icon="send">{m.channel}</Token>
        <span>{KIND_LABEL[m.kind]} · drafted by <SessionLink id={m.session_id}>{m.author}</SessionLink> {ago(m.created_at)}</span>
        {m.status === "sent" && <Badge tone="ok" icon="check">Sent{m.sent_at ? ` ${ago(m.sent_at)}` : ""}</Badge>}
        {uncertain && <Badge tone="attn" icon="alert">{m.status === "sending" ? "Sending" : "Uncertain"}</Badge>}
        {m.status === "discarded" && <Badge tone="done">Discarded</Badge>}
      </div>
      {draft ? (
        <div className="grid gap-2">
          {(m.title != null || title) && <input className="field-input font-semibold" aria-label="Title" value={title} disabled={busy} onChange={(e) => setTitle(e.target.value)} placeholder="Title (optional)" />}
          <textarea aria-label="Message" className="field-input min-h-32 text-[13px]" value={body} disabled={busy} onChange={(e) => setBody(e.target.value)} />
        </div>
      ) : (
        <div className="grid gap-1 text-[13px]">
          {m.title && <b>{m.title}</b>}
          <p className="m-0 whitespace-pre-wrap break-words text-ink-2">{m.body}</p>
        </div>
      )}
      {uncertain && <p className="mt-3 flex items-center gap-1.5 text-[12px] text-attn"><Icon name="alert" size={12} />Uncertain, check the destination: the send did not complete and may have arrived in {m.channel}. Launchpad never sends it again.</p>}
      {m.error && draft && <ErrorState className="mt-2" error={m.error} />}
      {!!error && <ErrorState className="mt-2" error={error} />}
      <div className={`mt-3 flex flex-wrap items-center gap-2 ${phone ? "[&_.btn]:min-h-11" : ""}`}>
        {draft && <>
          <ActionButton name="outbox.send" params={{ id: m.id }} vars={{ channel: m.channel }} disabled={busy || !body.trim()} confirmLabel="Send now" before={() => { setError(null); return save(); }} onDone={() => { toast(`Sent to ${m.channel}`); void refresh(); }} onError={failed} />
          {edited && <button className="btn btn-sm" disabled={busy || !body.trim()} onClick={() => void run("save")}>Save edit</button>}
          {edited && <button className="btn btn-sm" disabled={busy} onClick={() => { setBody(m.body); setTitle(m.title ?? ""); }}>Cancel edit</button>}
        </>}
        {m.status === "uncertain" && <button className="btn btn-sm" disabled={busy} title="You found the message at its destination" onClick={() => void run("mark-sent")}><Icon name="check" size={12} />It arrived</button>}
        {(draft || m.status === "uncertain") && <ActionButton name="outbox.discard" params={{ id: m.id }} disabled={busy} consequence={m.status === "uncertain" ? "Check the destination first: it may already be there." : undefined} onDone={() => { toast("Message discarded"); void refresh(); }} onError={failed} />}
      </div>
    </article>
  );
});
