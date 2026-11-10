// Quick chips over a bot's message box: the bot's own `starters` (bot.toml), ready-made messages. A message sends
// at once; one ending in "…" fills the box so you finish it. A bot without starters shows none. Data, not code:
// Ask Launchpad gets its starters when Home creates it.

/** A starter ending in "…" fills the box (without the "…") instead of sending. */
export function starterChip(s: string): { label: string; text: string; send: boolean } {
  const t = s.trim();
  if (!t.endsWith("…") && !t.endsWith("...")) return { label: t, text: t, send: true };
  const text = t.replace(/(…|\.\.\.)$/, "").trimEnd();
  return { label: `${text}…`, text: `${text} `, send: false };
}

export function QuickChips({ starters, onSend, onFill }: { starters: string[]; onSend: (text: string) => void; onFill: (text: string) => void }) {
  const chips = starters.filter((s) => s.trim()).map(starterChip);
  if (!chips.length) return null;
  return <div className="flex gap-1.5 overflow-x-auto pb-1.5 [scrollbar-width:none]" role="group" aria-label="Quick messages" data-tour="chat.chips">
    {chips.map((c) => <button key={c.label} className="lp-chip" title={c.send ? `Send: ${c.text}` : "Write the rest of the message"} onClick={() => (c.send ? onSend(c.text) : onFill(c.text))}>{c.label}</button>)}
  </div>;
}
