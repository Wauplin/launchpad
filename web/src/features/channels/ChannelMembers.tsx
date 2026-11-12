// A named channel's Members panel (`?panel=members`: a drawer on the laptop, a page on the phone): you first, always a
// member, then each member bot with its mode (`channel.set-mode`) and Remove in ⋯ (`channel.remove-member`), then
// Add a member from the workspace's other bots (`channel.add-member`). An admin bot's changes to members are in the
// activity list with their values before.

import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { ActionMenuItem } from "@/components/ActionButton";
import { BotAvatar } from "@/components/BotAvatar";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { toastError } from "@/components/Toast";
import { RowActions } from "@/components/ui";
import type { Channel } from "@/gen/Channel";
import type { ChannelMemberMode } from "@/gen/ChannelMemberMode";
import { runAction } from "@/lib/actions";
import { useBots } from "@/lib/bots";
import { MODES } from "@/lib/channels";

export function ChannelMembers({ channel }: { channel: Channel }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === channel.workspace_id && !channel.members.some((m) => m.bot_id === b.id)).sort((a, b) => a.settings.name.localeCompare(b.settings.name));
  const [adding, setAdding] = useState("");
  const [mode, setMode] = useState<ChannelMemberMode>("mentions");
  const [busy, setBusy] = useState<string | null>(null);
  const refresh = () => Promise.all([qc.invalidateQueries({ queryKey: ["channel", channel.id] }), qc.invalidateQueries({ queryKey: ["channels"] })]);
  const run = async (key: string, f: () => Promise<unknown>) => {
    setBusy(key);
    try { await f(); await refresh(); } catch (e) { toastError(e); } finally { setBusy(null); }
  };
  const closed = channel.archived;
  return (
    <div className="grid gap-1 p-3" data-tour="channel.members">
      <div className="flex min-h-11 items-center gap-2 px-1">
        <span className="grid size-6 flex-none place-items-center rounded-control bg-sunk text-[12px] font-semibold text-ink-2" aria-hidden>Y</span>
        <span className="min-w-0 flex-1 text-[13px]"><b className="font-semibold">You</b> <span className="text-ink-3">· always a member</span></span>
      </div>
      {channel.members.map((m) => (
        <div key={m.bot_id} className="flex min-h-11 items-center gap-2 px-1">
          <BotAvatar name={m.name} size={24} />
          <Link to="/bots/$id" params={{ id: m.bot_id }} className="min-w-0 flex-1 truncate text-[13px] font-semibold text-ink no-underline hover:underline" title={m.last_read ? `Read up to message ${m.last_read}` : "Has not read the channel yet"}>{m.name}</Link>
          <RowActions label={`More actions for ${m.name}`} width={140}
            main={<select className="field-input w-full" aria-label={`When ${m.name} is woken`} disabled={closed || busy === m.bot_id} value={m.mode}
              onChange={(e) => { const next = e.target.value as ChannelMemberMode; void run(m.bot_id, () => runAction("channel.set-mode", { id: channel.id, bot: m.bot_id }, { mode: next })); }}>
              {MODES.map((x) => <option key={x.id} value={x.id}>{x.label}</option>)}
            </select>}
            more={(close) => <>
              <MenuItem icon={<Icon name="comment" size={14} />} label={`Open ${m.name}'s chat`} onClick={() => { close(); void navigate({ to: "/bots/$id", params: { id: m.bot_id } }); }} />
              <ActionMenuItem name="channel.remove-member" params={{ id: channel.id, bot: m.bot_id }} close={close} label="Remove from the channel" detail="It can no longer read or post here" onDone={refresh} />
            </>} />
        </div>
      ))}
      {channel.members.length === 0 && <p className="m-0 px-1 text-[12px] text-ink-3">No bot yet: only you post here.</p>}
      {!closed && <div className="mt-2 grid gap-1.5 border-t border-line-2 px-1 pt-3" data-tour="channel.add-member">
        <span className="text-[12px] font-semibold text-ink-2">Add a member</span>
        {bots.length === 0
          ? <span className="text-[12px] text-ink-3">Every bot of the workspace is a member.</span>
          : <div className="flex flex-wrap items-center gap-1.5">
            <select className="field-input min-w-0 flex-1" aria-label="Bot to add" value={adding} onChange={(e) => setAdding(e.target.value)}>
              <option value="">Pick a bot…</option>
              {bots.map((b) => <option key={b.id} value={b.id}>{b.settings.name}</option>)}
            </select>
            <select className="field-input w-auto" aria-label="When it is woken" value={mode} onChange={(e) => setMode(e.target.value as ChannelMemberMode)}>
              {MODES.map((x) => <option key={x.id} value={x.id}>{x.label}</option>)}
            </select>
            <button className="btn btn-sm" disabled={!adding || busy === "add"} onClick={() => void run("add", async () => { await runAction("channel.add-member", { id: channel.id, bot: adding }, { mode }); setAdding(""); })}><Icon name="plus" size={12} />Add</button>
          </div>}
      </div>}
    </div>
  );
}
