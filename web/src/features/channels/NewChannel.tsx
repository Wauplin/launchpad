// The owner's New channel form (Bots page, `?new=channel`): a name (lowercase letters, digits and dashes), a purpose
// and the member bots of the workspace, each with its mode. It runs `channel.create`; Home checks the name again
// (unique in the workspace, archived channels included).

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { BotAvatar } from "@/components/BotAvatar";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import type { Channel } from "@/gen/Channel";
import type { ChannelMemberMode } from "@/gen/ChannelMemberMode";
import { runAction } from "@/lib/actions";
import { useBots } from "@/lib/bots";
import { channelNameError, MODES, useChannels } from "@/lib/channels";
import { useScope } from "@/lib/workspace";

export function NewChannel({ onCancel }: { onCancel: () => void }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const scope = useScope();
  const ws = scope.workspace.id;
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === ws).sort((a, b) => a.settings.name.localeCompare(b.settings.name));
  const taken = (useChannels().data ?? []).filter((c) => c.workspace_id === ws && c.kind === "channel").map((c) => c.name);
  const [name, setName] = useState("");
  const [purpose, setPurpose] = useState("");
  const [members, setMembers] = useState<Record<string, ChannelMemberMode>>({});
  const [busy, setBusy] = useState(false);
  const error = name ? channelNameError(name, taken) : null;
  const toggle = (id: string, on: boolean) => setMembers((m) => { const next = { ...m }; if (on) next[id] = "mentions"; else delete next[id]; return next; });
  const create = async () => {
    if (!ws || !name || error) return;
    setBusy(true);
    try {
      const ch = await runAction<Channel>("channel.create", {}, { workspace_id: ws, name, purpose: purpose.trim(), members: Object.entries(members).map(([bot, mode]) => ({ bot, mode })) });
      await qc.invalidateQueries({ queryKey: ["channels"] });
      void navigate({ to: "/channels/$id", params: { id: ch.id } });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <section className="grid gap-3 rounded-card border border-line bg-paper p-4" data-tour="channels.form">
      <h2 className="m-0 text-[15px] font-semibold">New channel</h2>
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Name
        <span className="flex items-center gap-1.5 font-normal"><span className="text-[13px] text-ink-3" aria-hidden>#</span>
          <input className="field-input min-w-0 flex-1 font-mono text-[12px]" maxLength={40} value={name} placeholder="release-train" autoFocus aria-invalid={!!error}
            onChange={(e) => setName(e.target.value.toLowerCase().replace(/[\s_]+/g, "-").replace(/^#/, ""))} />
        </span>
        {error && <span className="font-normal text-gh-closed">{error}</span>}
      </label>
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Purpose
        <input className="field-input font-normal" maxLength={200} value={purpose} placeholder="Plan and ship the weekly release" onChange={(e) => setPurpose(e.target.value)} />
      </label>
      <fieldset className="m-0 grid gap-1 border-0 p-0">
        <legend className="mb-1 p-0 text-[12px] font-semibold text-ink-2">Member bots <span className="font-normal text-ink-3">· you are always a member</span></legend>
        {bots.length === 0 && <span className="text-[12px] text-ink-3">No bot in {scope.workspace.name} yet: add members later.</span>}
        {bots.map((b) => {
          const on = b.id in members;
          return <div key={b.id} className="flex min-h-10 items-center gap-2">
            <label className="flex min-h-10 min-w-0 flex-1 cursor-pointer items-center gap-2 text-[13px]">
              <input type="checkbox" checked={on} onChange={(e) => toggle(b.id, e.target.checked)} />
              <BotAvatar name={b.settings.name} size={20} /><span className="min-w-0 truncate">{b.settings.name}</span>
            </label>
            {on && <select className="field-input w-auto" aria-label={`When ${b.settings.name} is woken`} value={members[b.id]} onChange={(e) => setMembers((m) => ({ ...m, [b.id]: e.target.value as ChannelMemberMode }))}>
              {MODES.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
            </select>}
          </div>;
        })}
      </fieldset>
      <div className="flex flex-wrap items-center justify-end gap-2">
        <span className="mr-auto text-[12px] text-ink-3">Creating a channel wakes nobody.</span>
        <button className="btn btn-sm btn-ghost" onClick={onCancel}>Cancel</button>
        <button className="btn btn-sm btn-primary" disabled={busy || !name || !!error || !ws} onClick={() => void create()}><Icon name="plus" size={12} />Create channel</button>
      </div>
    </section>
  );
}
