// The message box of a bot channel or DM: on its page, and under a Today row where a bot asks you (Reply).

import { useLayoutEffect, useRef, useState } from "react";
import { BotAvatar } from "@/components/BotAvatar";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import type { Channel } from "@/gen/Channel";
import { runAction } from "@/lib/actions";
import { mentionAt } from "@/lib/channels";
import { useKeptDraft } from "@/lib/keptDraft";
import { usePhone } from "@/lib/phone";

/** Your message: Enter sends, Shift+Enter adds a line; `@` suggests the member bots (Enter or Tab picks the first).
 * Also under a Today row where a bot asks you (`replyTo`: the ask it answers; same kept draft as the page). */
export function ChannelComposer({ channel, onSent, replyTo, className = "border-t border-line-2 px-5 pt-2 pb-2.5", autoFocus }: { channel: Channel; onSent: () => unknown; replyTo?: number; className?: string; autoFocus?: boolean }) {
  const phone = usePhone();
  const { text, setText, clear } = useKeptDraft(`channel.${channel.id}`, "");
  const [busy, setBusy] = useState(false);
  const [caret, setCaret] = useState(0);
  const [dismissed, setDismissed] = useState<number | null>(null);
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => { const el = ref.current; if (!el) return; el.style.height = "auto"; el.style.height = `${Math.min(el.scrollHeight, 220)}px`; }, [text]);
  const at = mentionAt(text, caret);
  const suggestions = at && at.start !== dismissed ? channel.members.filter((m) => m.slug && (m.slug.startsWith(at.query) || m.name.toLowerCase().startsWith(at.query))).slice(0, 6) : [];
  const pick = (slug: string) => {
    if (!at) return;
    const next = `${text.slice(0, at.start)}@${slug} ${text.slice(caret)}`;
    const pos = at.start + slug.length + 2;
    setText(next);
    setCaret(pos);
    requestAnimationFrame(() => { const el = ref.current; if (el) { el.focus(); el.setSelectionRange(pos, pos); } });
  };
  const send = async () => {
    const t = text.trim();
    if (!t || busy) return;
    setBusy(true);
    try {
      await runAction("channel.post", { id: channel.id }, replyTo != null ? { text: t, reply_to: replyTo } : { text: t });
      setText("");
      clear();
      await onSent();
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const dm = channel.kind === "dm";
  const closed = channel.archived;
  const name = dm ? "both bots" : channel.title;
  return (
    <div className={className} data-tour={replyTo != null ? undefined : "channel.reply"}>
      <div className="relative w-full max-w-text">
        {dm && !closed && <div className="mb-1.5 text-[12px] text-ink-3">Both bots will see your message</div>}
        {closed && <div className="mb-1.5 text-[12px] text-ink-3">Archived: nobody posts here. ⋯ → Bring back to post again.</div>}
        {suggestions.length > 0 && <div role="listbox" aria-label="Mention a bot" className="absolute bottom-full left-0 z-10 mb-1 grid min-w-[220px] gap-px rounded-card border border-line bg-paper p-1 shadow-[0_8px_24px_-12px_rgba(23,32,43,.5)]">
          {suggestions.map((m, i) => <button key={m.bot_id} role="option" aria-selected={i === 0} className={`flex ${phone ? "min-h-11" : "min-h-8"} items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-sunk ${i === 0 ? "bg-sunk" : ""}`} onMouseDown={(e) => e.preventDefault()} onClick={() => pick(m.slug)}>
            <BotAvatar name={m.name} size={18} /><span className="min-w-0 flex-1 truncate">{m.name}</span><span className="text-[12px] text-ink-3">@{m.slug}</span>
          </button>)}
        </div>}
        <div className="session-message-input flex items-end gap-2 rounded-control border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
          <textarea ref={ref} rows={1} value={text} disabled={busy || closed} aria-label={`Message ${name}`} autoFocus={autoFocus}
            placeholder={replyTo != null ? `Reply in ${dm ? "the DM" : channel.title}: your message answers this one` : dm ? "Message both bots…" : `Message ${channel.title}… (@ to wake a bot)`}
            onChange={(e) => { setText(e.target.value); setCaret(e.target.selectionStart); setDismissed(null); }}
            onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
            onKeyDown={(e) => {
              if (suggestions.length && !e.nativeEvent.isComposing && (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey))) { e.preventDefault(); pick(suggestions[0].slug); return; }
              if (suggestions.length && e.key === "Escape") { e.preventDefault(); setDismissed(at?.start ?? null); return; }
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void send(); }
            }}
            className="min-h-6 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none" />
          <button className="iconbtn border-line! bg-paper" title="Send" aria-label="Send" disabled={busy || closed || !text.trim()} onClick={() => void send()}><Icon name="send" size={14} /></button>
        </div>
      </div>
    </div>
  );
}
