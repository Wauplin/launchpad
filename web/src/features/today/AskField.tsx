// The Ask field of Today: a message to a bot of the workspace (Ask Launchpad unless you changed it), then its page.

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { PresetButton, PresetChips, usePresets } from "@/features/agents/MessagePresets";
import { api } from "@/lib/api";
import { useBots } from "@/lib/bots";
import { withPresets } from "@/lib/presets";
import { useScope } from "@/lib/workspace";

/** A message to a bot of the workspace (the first one, Ask Launchpad unless you changed it), then its page. */
export function BotField() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const scope = useScope();
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === scope.workspace.id);
  const [chosen, setChosen] = useState<string | null>(null);
  const bot = bots.find((b) => b.id === chosen) ?? bots.find((b) => b.slug === "ask-launchpad") ?? bots[0];
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [presets, setPresets] = useState<string[]>([]);
  const allPresets = usePresets("bots");
  const message = withPresets(text, presets, allPresets);
  if (!bot) return null;
  const send = async () => {
    if (!message || busy) return;
    setBusy(true);
    try {
      await api.post(`/api/bots/${bot.id}/message`, { text: message });
      await qc.invalidateQueries({ queryKey: ["sessions"] });
      void navigate({ to: "/bots/$id", params: { id: bot.id } });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <div>
      <PresetChips surface="bots" selected={presets} onChange={setPresets} className="mb-1.5" />
      <div className="flex items-center gap-2 rounded-lg border border-line px-2.5 py-1.5 focus-within:border-teal">
        <Icon name="spark" size={14} className="text-teal-ink" />
        {bots.length > 1 && <select className="max-w-[140px] bg-transparent text-[13px] font-semibold outline-none" aria-label="Bot" value={bot.id} onChange={(e) => setChosen(e.target.value)}>{bots.map((b) => <option key={b.id} value={b.id}>{b.settings.name}</option>)}</select>}
        <input className="min-w-0 flex-1 bg-transparent text-[13px] outline-none" aria-label={`Message ${bot.settings.name}`} placeholder={`Message ${bot.settings.name}…`} value={text} disabled={busy} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void send(); }} />
        <PresetButton surface="bots" selected={presets} onChange={setPresets} />
        <button className="iconbtn" aria-label={`Send to ${bot.settings.name}`} title="Send" disabled={!message || busy} onClick={() => void send()}><Icon name="send" size={13} /></button>
      </div>
    </div>
  );
}
