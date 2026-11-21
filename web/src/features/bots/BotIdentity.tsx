// A bot's look in its settings: the avatar seed (DiceBear's bottts robot, cached by Home) and the color of its name,
// with a preview of the unsaved choice. Colors come only from the theme's identity palette (no hex input): Suggest
// another color picks a different one on every click. Both save to bot.toml (`avatar_seed`, `color`); empty means
// the default from the bot's name.

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { BotAvatar, inkClass } from "@/components/BotAvatar";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import type { Bot } from "@/gen/Bot";
import { runAction } from "@/lib/actions";
import { useBots } from "@/lib/bots";
import { identityOf, isIdentityColor, nextColor, randomSeed } from "@/lib/identity";
import { usePhone } from "@/lib/phone";

export function BotIdentity({ bot }: { bot: Bot }) {
  const qc = useQueryClient();
  const phone = usePhone();
  const all = useBots().data ?? [];
  const savedSeed = bot.settings.avatar_seed?.trim() ?? "";
  const savedColor = isIdentityColor(bot.settings.color) ? bot.settings.color : null;
  const [seed, setSeed] = useState(savedSeed);
  const [color, setColor] = useState<string | null>(savedColor);
  const [busy, setBusy] = useState(false);
  const defaults = identityOf({ ...bot, settings: { ...bot.settings, avatar_seed: null, color: null } }, all);
  const preview = identityOf({ ...bot, settings: { ...bot.settings, avatar_seed: seed, color } }, all);
  const dirty = seed.trim() !== savedSeed || color !== savedColor;
  const tooLong = [...seed.trim()].length > 128;
  const save = async () => {
    setBusy(true);
    try {
      await runAction<Bot>("bot.update", { id: bot.id }, { set: { avatar_seed: seed.trim() || null, color } });
      await qc.invalidateQueries({ queryKey: ["bots"] });
      toast("Saved: the new look shows everywhere");
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <div className="grid gap-3">
    <div className="flex min-w-0 items-center gap-3">
      <BotAvatar name={bot.settings.name} size={phone ? 48 : 56} identity={preview} />
      <div className="min-w-0">
        <div className={`truncate text-[15px] font-semibold ${inkClass(preview.color)}`}>{bot.settings.name}</div>
        <div className="text-[12px] text-ink-3">{color ? `Color: ${color}` : `Color: ${defaults.color} (from its name)`}</div>
      </div>
    </div>
    <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Avatar seed
      <input className="field-input font-normal" value={seed} maxLength={160} placeholder={defaults.seed} onChange={(e) => setSeed(e.target.value)} aria-describedby="bot-seed-hint" />
      <span id="bot-seed-hint" className={`font-normal ${tooLong ? "text-gh-closed" : "text-ink-3"}`}>{tooLong ? "128 characters at most" : "Any word draws a robot; empty uses the bot's name."}</span>
    </label>
    <div className="flex flex-wrap items-center gap-2">
      <button className="btn btn-sm" title="A new random seed" onClick={() => setSeed(randomSeed())}><Icon name="refresh" size={12} />Shuffle avatar</button>
      <button className="btn btn-sm" title="Another color from the palette, readable in both themes" onClick={() => setColor(nextColor(preview.color))}><Icon name="spark" size={12} />Suggest another color</button>
      {(seed || color) && <button className="btn btn-sm" onClick={() => { setSeed(""); setColor(null); }}>Use defaults</button>}
      <span className="flex-1" />
      {dirty && <button className="btn btn-sm" disabled={busy} onClick={() => { setSeed(savedSeed); setColor(savedColor); }}>Cancel</button>}
      <button className="btn btn-sm btn-primary" disabled={!dirty || busy || tooLong} onClick={() => void save()}>Save</button>
    </div>
  </div>;
}
