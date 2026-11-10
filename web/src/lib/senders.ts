// Messages that do not come from the owner (`launchpad_core::senders`): Home stamps the sender (`from` on the
// user_message event) and the text arrives under a fixed header, fenced. The UI shows the sender instead of the
// owner's bubble, and the text without the header and fence. Pure, so it is tested (tests/chatRows.test.mjs).

import type { MessageFrom } from "@/gen/MessageFrom";

/** How Home's header starts (`senders::HEADER_START`). */
const HEADER_START = "Message from ";

/** A message from another bot or agent (a routine run's message shows as a routine instead). */
export const fromAgent = (from: MessageFrom | null | undefined): from is MessageFrom => !!from && (from.kind === "bot" || from.kind === "session");

/** Whether the text arrived under Home's header (messages; an agent launched by another one gets none). */
export const hasHeader = (text: string): boolean => text.startsWith(HEADER_START);

/** The text the sender wrote: without Home's header and fence (what follows the fence, such as a changed BOT.md or
 * context Home added, stays after it). */
export function withoutSender(text: string): string {
  const lines = text.split("\n");
  if (!lines[0]?.startsWith(HEADER_START)) return text;
  let i = 1;
  while (i < lines.length && !lines[i].trim()) i++;
  const fence = lines[i] ?? "";
  if (!/^`{3,}$/.test(fence)) return text;
  const end = lines.indexOf(fence, i + 1);
  const body = lines.slice(i + 1, end < 0 ? undefined : end).join("\n");
  const rest = end < 0 ? "" : lines.slice(end + 1).join("\n").trim();
  return rest ? `${body}\n\n${rest}` : body;
}

/** The line over a message from another bot or agent: "From bot Scout · not from you", or "From Scout, accepted by you"
 * for its card the owner accepted. */
export function senderLine(from: MessageFrom): string {
  const name = from.name.trim() || (from.kind === "bot" ? "a bot" : "an agent");
  if (from.card) return `From ${name}, ${from.accepted ? "accepted by you" : "accepted automatically"}`;
  return `From ${from.kind === "bot" ? "bot" : "agent"} ${name} · not from you`;
}
