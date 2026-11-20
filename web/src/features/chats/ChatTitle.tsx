// A chat's title in its header, with its direct actions: the pen edits the title in place (Save, Auto, Cancel) and
// the star pins or unpins the chat. Auto closes the editor at once and names the chat from its conversation as it is
// now (`bot.thread-auto-title`); the new title shows when it is ready, and a rename made meanwhile wins.

import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type ReactNode } from "react";
import { toastError } from "@/components/Toast";
import { IconButton } from "@/components/ui";
import { useRenameRequest } from "@/features/chat/BotThreads";
import { runAction } from "@/lib/actions";
import { refreshThreads } from "@/lib/bots";
import { useChatPins, usePinChat } from "./data";

type Props = { bot: string; thread: string; pin: string | null; title: string; mark?: ReactNode };

/** The title editor's state, shared by the title and the phone's strip under its bar. */
export function useTitleEditor(bot: string, thread: string) {
  // The chat it is open for, so another chat shows its title again.
  const [open, setOpen] = useState<string | null>(null);
  const key = `${bot}\n${thread}`;
  const request = useRenameRequest((s) => s.request);
  useEffect(() => {
    if (request?.bot === bot && request.thread === thread) { setOpen(key); useRenameRequest.setState({ request: null }); }
  }, [request, bot, thread, key]);
  return { editing: open === key, setEditing: (on: boolean) => setOpen(on ? key : null) };
}

/** The title, or its editor, then the pen and the star. */
export function ChatTitle({ bot, thread, pin, title, mark, editor }: Props & { editor: ReturnType<typeof useTitleEditor> }) {
  if (editor.editing) return <TitleForm bot={bot} thread={thread} title={title} onDone={() => editor.setEditing(false)} />;
  return <span className="flex min-w-0 items-center gap-1.5">
    {mark}
    <span className="truncate" title={title}>{title}</span>
    <span className="flex flex-none items-center font-normal">
      <IconButton icon="pencil" label={`Rename “${title}”`} onClick={() => editor.setEditing(true)} />
      {pin && <PinStar pin={pin} />}
    </span>
  </span>;
}

/** Pin or unpin the chat: a shortcut in the laptop sidebar, first in the phone's list. */
export function PinStar({ pin }: { pin: string }) {
  const pinned = !!useChatPins().data?.some((p) => p.target === pin);
  const setPin = usePinChat();
  return <IconButton icon={pinned ? "star-fill" : "star"} className={pinned ? "text-teal-ink" : ""} aria-pressed={pinned}
    label={pinned ? "Unpin this chat" : "Pin this chat (a shortcut in the sidebar)"} onClick={() => void setPin({ pin }, !pinned)} />;
}

/** The title in a field: Enter or Save renames, Escape or Cancel keeps it, Auto names it from the conversation. */
export function TitleForm({ bot, thread, title: initial, onDone }: { bot: string; thread: string; title: string; onDone: () => void }) {
  const qc = useQueryClient();
  const [title, setTitle] = useState(initial);
  const [saving, setSaving] = useState(false);
  const save = async () => {
    if (!title.trim()) return;
    setSaving(true);
    try { await runAction("bot.thread-rename", { id: bot, thread }, { title: title.trim() }); await refreshThreads(qc, bot); onDone(); }
    catch (e) { toastError(e); }
    finally { setSaving(false); }
  };
  const auto = () => {
    onDone();
    void runAction("bot.thread-auto-title", { id: bot, thread }).then(() => refreshThreads(qc, bot)).catch(toastError);
  };
  return <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1 text-[13px] font-normal">
    <input autoFocus aria-label="Chat title" className="field-input min-w-0 flex-1 basis-40 font-[650]" maxLength={80} value={title} disabled={saving}
      onChange={(e) => setTitle(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void save(); if (e.key === "Escape") onDone(); }} />
    <button type="button" className="btn btn-sm" disabled={!title.trim() || saving} onClick={() => void save()}>Save</button>
    <button type="button" className="btn btn-sm" disabled={saving} title="Name this chat from its conversation as it is now; the new title shows when it is ready" onClick={auto}>Auto</button>
    <button type="button" className="btn btn-sm btn-ghost" disabled={saving} onClick={onDone}>Cancel</button>
  </span>;
}
