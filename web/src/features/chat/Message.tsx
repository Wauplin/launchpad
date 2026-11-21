// One message of a conversation, the same in the bot chat and bot channels: the sender's mark on the left, a header
// with its name (in the bot's color), a provenance tag and the time, then the text in a bubble. Every sender is on
// the left, like a team chat; the bubble says whose words they are: yours (grey), the bot you talk to (white,
// outlined) or someone else's (white, dashed, "not from you"). A message between bots of a conversation adds thin
// edges in their identity colors: the sender's on the left, each recipient's on the right (stacked when several).
// Lines about the work (steps, receipts, deliveries) stay outside the bubble, under it (`after`).

import { botEdge, useIdentitiesVersion } from "@/components/BotAvatar";

/** Whose words: yours, the bot of this chat (or a bot posting in a channel), or another sender's. */
export type MessageLook = "owner" | "bot" | "other";

const BUBBLE: Record<MessageLook, string> = {
  owner: "bg-sunk",
  bot: "border border-line-2 bg-paper shadow-[0_1px_2px_rgba(23,32,43,.06)]",
  other: "border border-dashed border-line bg-paper",
};

/** The indent of lines that belong to a message but sit outside its bubble: the mark (28 px) and the gap. */
export const MESSAGE_INDENT = "pl-9.5";

export function ChatMessage({ avatar, name, tag, time, look, faded, edges, children, after, tour }: {
  avatar: React.ReactNode;
  /** The sender's name, a link to its page when it has one. */
  name: React.ReactNode;
  /** Small words after the name: provenance ("bot · not from you"), "writing…", a badge. */
  tag?: React.ReactNode;
  /** When it was written, already formatted. */
  time?: React.ReactNode;
  look: MessageLook;
  /** Not delivered yet (your accepted message on its way): the bubble is faded. */
  faded?: boolean;
  /** A message between bots: the sender's name (left edge) and the recipients' names (right edge), drawn in their
   * identity colors. Decoration only: the header names them in words. */
  edges?: { from: string; to: readonly string[] };
  children: React.ReactNode;
  /** Under the bubble, outside it: how far it got, its files, deliveries. */
  after?: React.ReactNode;
  tour?: string;
}) {
  return <div className="group flex items-start gap-2.5 py-2" data-tour={tour}>
    {avatar}
    <div className="grid min-w-0 flex-1 grid-cols-[minmax(0,1fr)] justify-items-start gap-1">
      <div className="flex max-w-full min-w-0 flex-wrap items-baseline gap-x-1.5 text-[12px] leading-5 text-ink-3">
        <span className="min-w-0 truncate text-[13px] font-semibold">{name}</span>
        {tag}
        {time}
      </div>
      <div className={`max-w-full min-w-0 rounded-card rounded-tl-sm px-3 py-2 text-[13px] ${BUBBLE[look]} ${faded ? "opacity-70" : ""} ${edges ? "relative overflow-hidden" : ""}`}>
        {edges && <Edges from={edges.from} to={edges.to} />}
        {children}
      </div>
      {after}
    </div>
  </div>;
}

/** The provenance tag of a header: muted words with the reason in a tooltip. */
export function SenderTag({ children, title }: { children: React.ReactNode; title?: string }) {
  return <span className="min-w-0 text-[11px] text-ink-3" title={title}>{children}</span>;
}

/** The thin colored edges of a message between bots: the sender on the left, the recipients on the right, one
 * segment each from top to bottom. */
function Edges({ from, to }: { from: string; to: readonly string[] }) {
  useIdentitiesVersion();
  return <>
    <span className={`pointer-events-none absolute inset-y-0 left-0 w-0.5 opacity-60 ${botEdge(from)}`} aria-hidden />
    {to.length > 0 && <span className="pointer-events-none absolute inset-y-0 right-0 flex w-0.5 flex-col opacity-60" aria-hidden>
      {to.map((name, i) => <span key={i} className={`min-h-0 flex-1 ${botEdge(name)}`} />)}
    </span>}
  </>;
}
