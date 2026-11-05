// Outbox: everything Launchpad would send to other people on the owner's behalf, waiting for a click. Agent
// drafts for outgoing channels (Slack, Discord, webhooks) and GitHub suggestions. Sending is never automatic
// and never retried: a send that did not complete shows "uncertain" until the owner checks the destination.

import { useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Link } from "@tanstack/react-router";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { OutboxMessage } from "@/gen/OutboxMessage";
import type { Suggestion } from "@/gen/Suggestion";
import { Icon } from "@/components/Icon";
import { toast } from "@/components/Toast";
import { ActionButton } from "@/components/ActionButton";
import { Badge, Empty, ErrorState, Loading, PageHeader, Tabs, Token } from "@/components/ui";
import { SuggestionCard } from "@/features/suggestions/Suggestions";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { isGitHubSuggestion, messageTab, suggestionTab, useAllSuggestions, useOutboxMessages, type OutboxTab } from "./data";

type Row = { kind: "message"; m: OutboxMessage; at: number } | { kind: "suggestion"; s: Suggestion; at: number };

const KIND_LABEL: Record<OutboxMessage["kind"], string> = { slack: "Slack", discord: "Discord", generic: "Webhook" };

export function OutboxView() {
  const phone = usePhone();
  const messages = useOutboxMessages();
  const suggestions = useAllSuggestions();
  const [tab, setTab] = useState<OutboxTab>("to_send");
  const rows = useMemo(() => {
    const all: Row[] = [
      ...(messages.data ?? []).map((m) => ({ kind: "message" as const, m, at: m.created_at })),
      ...(suggestions.data ?? []).filter(isGitHubSuggestion).map((s) => ({ kind: "suggestion" as const, s, at: s.created_at })),
    ];
    return all.sort((a, b) => b.at - a.at);
  }, [messages.data, suggestions.data]);
  const tabOf = (r: Row) => (r.kind === "message" ? messageTab(r.m) : suggestionTab(r.s));
  const count = (t: OutboxTab) => rows.filter((r) => tabOf(r) === t).length;
  const shown = rows.filter((r) => tabOf(r) === tab);
  const tabs = <Tabs label="Outbox" value={tab} onChange={setTab} items={[{ id: "to_send", label: "To send", count: count("to_send") }, { id: "sent", label: "Sent", count: count("sent") }, { id: "discarded", label: "Discarded", count: count("discarded") }]} />;
  const channels = <Link to="/settings/channels" className="btn btn-sm no-underline"><Icon name="sliders" size={12} />Channels</Link>;
  const error = messages.error ?? suggestions.error;
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="outbox.page">
      {phone
        // The shell's top bar shows the title.
        ? <div className="phone-page-header flex min-w-0 shrink-0 flex-wrap items-center gap-2 border-b border-line px-3 py-1.5">{tabs}</div>
        : <PageHeader
          title="Outbox"
          meta={<span>Messages agents drafted for other people. Nothing is sent before you click Send. Messages to your own Telegram chat go out directly.</span>}
          actions={channels}
          tabs={tabs}
        />}
      {error ? <ErrorState className="m-4" error={error} /> : messages.isLoading || suggestions.isLoading ? <Loading what="the Outbox" /> : <OutboxList rows={shown} tab={tab} />}
    </div>
  );
}

function OutboxList({ rows, tab }: { rows: Row[]; tab: OutboxTab }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, getItemKey: (i) => (rows[i].kind === "message" ? rows[i].m.id : rows[i].s.id), estimateSize: () => 240, overscan: 3, useFlushSync: false });
  if (!rows.length)
    return <Empty fill icon="send">{tab === "to_send" ? "Nothing waits to be sent. Agents draft messages with launchpad send <channel> and suggestions with launchpad suggest." : tab === "sent" ? "Nothing sent yet." : "No discarded message. Discarded GitHub suggestions are deleted, so they are not listed."}</Empty>;
  return (
    <div ref={ref} className="min-h-0 flex-1 overflow-auto">
      <div className="relative mx-auto max-w-[860px]" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((item) => {
          const r = rows[item.index];
          return (
            <div key={item.key} data-index={item.index} ref={v.measureElement} className="absolute top-0 left-0 w-full px-4 pt-3" style={{ transform: `translateY(${item.start}px)` }}>
              {r.kind === "message" ? <MessageCard m={r.m} /> : <SuggestionRow s={r.s} />}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function SessionLink({ id, children }: { id: string | null; children: React.ReactNode }) {
  if (!id) return <span>{children}</span>;
  return <Link to="/agents" search={{ session: id } as never} className="text-teal-ink no-underline hover:underline">{children}</Link>;
}

const SuggestionRow = memo(function SuggestionRow({ s }: { s: Suggestion }) {
  if (s.target.kind !== "git_hub") return null;
  const [owner, repo] = s.target.repo.split("/");
  return (
    <div className="grid gap-1.5">
      <div className="flex flex-wrap items-center gap-2 text-[12px] text-ink-3">
        <Token icon="pr">GitHub</Token>
        <Link to="/github/$owner/$repo/$number" params={{ owner, repo, number: String(s.target.number) }} search={{ draft: s.id } as never} className="text-teal-ink no-underline hover:underline">{s.target.repo}#{s.target.number}</Link>
        <span>· drafted by <SessionLink id={s.session_id}>{s.author}</SessionLink> {formatAge(s.created_at)} ago</span>
      </div>
      <SuggestionCard suggestion={s} />
    </div>
  );
});

const MessageCard = memo(function MessageCard({ m }: { m: OutboxMessage }) {
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
    <article className="rounded-lg border border-line bg-paper p-4" data-tour="outbox.card">
      <div className="mb-3 flex flex-wrap items-center gap-2 text-[12px] text-ink-3">
        <Token icon="send">{m.channel}</Token>
        <span>{KIND_LABEL[m.kind]} · drafted by <SessionLink id={m.session_id}>{m.author}</SessionLink> {formatAge(m.created_at)} ago</span>
        {m.status === "sent" && <Badge tone="ok" icon="check">Sent{m.sent_at ? ` ${formatAge(m.sent_at)} ago` : ""}</Badge>}
        {uncertain && <Badge tone="attn" icon="alert">{m.status === "sending" ? "Sending" : "Uncertain"}</Badge>}
        {m.status === "discarded" && <Badge tone="done">Discarded</Badge>}
      </div>
      {draft ? (
        <div className="grid gap-2">
          {(m.title != null || title) && <input className="field-input text-[13px] font-semibold" aria-label="Title" value={title} disabled={busy} onChange={(e) => setTitle(e.target.value)} placeholder="Title (optional)" />}
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
