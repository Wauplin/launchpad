// Add context → Chat or channel…: a chat (a bot thread and its conversation) or a bot channel or DM of the
// workspace, as a Launchpad reference the agent reads with `launchpad ref read` (like an agent trace). Searches the
// Chats list's titles; newest first.

import { useMemo, useState } from "react";
import type { LpRef } from "@/gen/LpRef";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { useChats } from "@/features/chats/data";
import { matchScore } from "./ProjectPicker";

export function ChatChoices({ onPick, close }: { onPick: (r: LpRef) => void; close: () => void }) {
  const [q, setQ] = useState("");
  const { chats, loading } = useChats();
  const rows = useMemo(() => chats.filter((c) => matchScore(`${c.title} ${c.kind === "thread" ? c.participants.map((b) => b.settings.name).join(" ") : ""}`, q.toLowerCase()) !== null).slice(0, 40), [chats, q]);
  return <div className="grid gap-1">
    <input autoFocus className="field-input" placeholder="Search chats and channels" value={q} onChange={(e) => setQ(e.target.value)} />
    <div className="max-h-72 overflow-y-auto">
      {loading && !rows.length && <div className="p-2 text-[12px] text-ink-3">Loading…</div>}
      {!loading && !rows.length && <div className="p-2 text-[12px] text-ink-3">No chat or channel.</div>}
      {rows.map((c) => <MenuItem key={c.pin} icon={<Icon name="comment" size={12} />} label={<span className="block truncate">{c.title}</span>}
        detail={c.kind === "thread" ? c.participants.map((b) => b.settings.name).join(", ") : c.channel.kind === "dm" ? "DM" : "Channel"}
        onClick={() => { onPick(c.kind === "thread" ? { kind: "chat", id: c.bot.id, thread: c.thread, title: c.title } : { kind: "channel", id: c.channel.id, title: c.title }); close(); }} />)}
    </div>
  </div>;
}
