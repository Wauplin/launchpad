// Launchpad references in the window: the short ID (`lp:chat/<bot>/<thread>/<session>/<seq>`) and the address of
// an object's page, built the same way as Home's `launchpad_core::refs` (both check `web/tests/refs/cases.json`).
// Home alone parses and resolves pasted links and IDs (`GET /api/refs/resolve`), so a reference reads the same in
// Go to, a message box and an agent's `launchpad ref read`. Pure: no imports with effects (tested with node).

import type { LpRef } from "../gen/LpRef.ts";
import type { RefKind } from "../gen/RefKind.ts";

const WORD: Record<RefKind, string> = {
  session: "session", chat: "chat", channel: "channel", folio: "folio", topic: "topic", bot: "bot",
  local_pr: "local-pr", folder: "folder", card: "card", oracle: "oracle", routine: "routine", note: "note",
};

/** What a reference is, in words (`RefKind::label`). */
export const REF_LABEL: Record<RefKind, string> = {
  session: "Agent session", chat: "Chat", channel: "Channel", folio: "Folio", topic: "Topic", bot: "Bot",
  local_pr: "Local PR", folder: "Folder", card: "Card", oracle: "Oracle", routine: "Routine", note: "Knowledge base note",
};

/** The short ID: `lp:<kind>/<id>[/…]`. Also the key of a reference in a message's context. */
export function refId(r: LpRef): string {
  let s = `lp:${WORD[r.kind]}/${r.id}`;
  if (r.kind === "session" && r.seq != null) s += `/${r.seq}`;
  if (r.kind === "chat") {
    s += `/${r.thread ?? ""}`;
    if (r.session && r.seq != null) s += `/${r.session}/${r.seq}`;
  }
  if (r.kind === "channel" && r.message != null) s += `/${r.message}`;
  if (r.kind === "folio") {
    if (r.page) s += `/${r.page}`;
    if (r.section) s += `#${r.section}`;
  }
  return s;
}

const query = (pairs: [string, string][]) => {
  const q = new URLSearchParams(pairs).toString();
  return q ? `?${q}` : "";
};

/** The address of its page in a window (path and query). */
export function refPath(r: LpRef): string {
  switch (r.kind) {
    case "session": return `/agents${query([["session", r.id], ...(r.seq != null ? [["msg", String(r.seq)] as [string, string]] : [])])}`;
    case "chat": return `/bots/${r.id}${query([["thread", r.thread ?? ""], ...(r.session && r.seq != null ? [["msg", `${r.session}.${r.seq}`] as [string, string]] : [])])}`;
    case "channel": return `/channels/${r.id}${r.message != null ? query([["msg", String(r.message)]]) : ""}`;
    case "folio": return `/folios/${r.id}${query([...(r.page ? [["page", r.page] as [string, string]] : []), ...(r.section ? [["section", r.section] as [string, string]] : [])])}`;
    case "topic": return `/topics/${r.id}`;
    case "bot": return `/bots/${r.id}/settings`;
    case "local_pr": return `/predrafts/${r.id}`;
    case "folder": return `/projects/${r.id}`;
    case "card": return `/${query([["card", `proposal:${r.id}`]])}`;
    case "oracle": return `/schedules${query([["tab", "oracles"], ["oracle", r.id]])}`;
    case "routine": return `/schedules${query([["tab", "routines"], ["routine", r.id]])}`;
    case "note": return `/knowledge${query([["note", r.id]])}`;
  }
}

/** A link that opens it in this Launchpad, in its workspace (`?ws=` switches a window that shows another). */
export function refLink(r: LpRef, origin: string, workspace?: string | null): string {
  const path = refPath(r);
  if (!workspace) return origin + path;
  return `${origin}${path}${path.includes("?") ? "&" : "?"}ws=${encodeURIComponent(workspace)}`;
}

/** Pages whose addresses are references (`launchpad_core::refs::parse`). */
const PAGES = /^\/(agents|bots\/[^/]+(\/settings)?|channels\/[^/]+(\/settings)?|folios\/[^/]+|topics\/[^/]+|predrafts\/[^/]+|projects\/[^/]+|schedules|knowledge)?\/?$/;

/** Pure: whether a pasted text is one reference worth asking Home about: a short ID, or a link to a Launchpad page
 * of this window's origin (`origin`) or of any host when it names a session, a thread or a message. */
export function looksLikeRef(text: string, origin?: string): boolean {
  const t = text.trim();
  if (!t || /\s/.test(t) || t.length > 600) return false;
  if (/^lp:[a-z_-]+\/\S+$/.test(t)) return true;
  let url: URL;
  try { url = new URL(t); } catch { return false; }
  if (url.protocol !== "https:" && url.protocol !== "http:") return false;
  if (!PAGES.test(url.pathname)) return false;
  if (url.pathname === "/" || url.pathname === "") return /^(session|proposal|local-pr):/.test(url.searchParams.get("card") ?? "");
  if (origin && url.origin === origin) return true;
  return ["session", "thread", "msg", "note", "oracle", "routine"].some((k) => url.searchParams.has(k)) || /^\/(channels|folios|topics|predrafts)\//.test(url.pathname);
}

/** The words of a reference (`refs::describe`): its kind, title and the part it names. */
export function describeRef(r: LpRef): string {
  const named = r.title ? `${REF_LABEL[r.kind]} “${r.title}”` : REF_LABEL[r.kind];
  if ((r.kind === "session" || r.kind === "chat") && r.seq != null) return `${named}, one message`;
  if (r.kind === "channel" && r.message != null) return `${named}, one message`;
  if (r.kind === "folio" && r.section) return `${named}, section ${r.section}`;
  if (r.kind === "folio" && r.page) return `${named}, page ${r.page}`;
  return named;
}

/** The text token a pasted reference becomes in a prompt (`@chat:Fix-login`): its kind and a short name. */
export function refToken(r: LpRef): string {
  const name = (r.title || r.id).replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-|-$/g, "").slice(0, 40) || "ref";
  return `@${WORD[r.kind]}:${name}${(r.seq != null || r.message != null) ? "#msg" : ""}`;
}

/** The icon of a reference's kind (an `IconName` of the sprite). */
export const REF_ICON: Record<RefKind, "comment" | "agents" | "folio" | "tag" | "pr-local" | "folder" | "spark" | "check" | "refresh" | "book"> = {
  session: "agents", chat: "comment", channel: "comment", folio: "folio", topic: "tag", bot: "agents",
  local_pr: "pr-local", folder: "folder", card: "spark", oracle: "check", routine: "refresh", note: "book",
};
