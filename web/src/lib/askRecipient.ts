// The bot the top-bar Ask sends to, and the label of its button: one function decides both, so they never disagree.
// Order: the bot you picked on this page (only on a page that has its own bot, such as a bot's page), else the
// page's bot, else the bot you picked elsewhere in this workspace, else the workspace's Ask bot. Picks live in the
// window's memory for one workspace: another workspace starts from its defaults, and a pick made on a bot's page
// lasts only while you stay on that page.

export type AskPicks = {
  workspace: string | null;
  /** Picked on a page without its own bot: lasts across those pages of the workspace. */
  general: string | null;
  /** Picked on a page with its own bot: lasts while that page (key) is shown. */
  page: { key: string; id: string } | null;
};

export const NO_PICKS: AskPicks = { workspace: null, general: null, page: null };

export function askRecipient<B extends { id: string }>(bots: B[], fallback: B | null, workspace: string | null, pageKey: string, pageBot: string | undefined, picks: AskPicks): B | null {
  const find = (id: string | null | undefined) => (id ? bots.find((b) => b.id === id) : undefined);
  const mine = picks.workspace === workspace ? picks : NO_PICKS;
  const own = find(pageBot);
  if (own) return (mine.page?.key === pageKey ? find(mine.page.id) : undefined) ?? own;
  return find(mine.general) ?? (fallback && find(fallback.id) ? fallback : bots[0] ?? null);
}

/** The picks after choosing `id` on the page `pageKey`. */
export function pickRecipient(picks: AskPicks, workspace: string | null, pageKey: string, pageBot: string | undefined, id: string): AskPicks {
  const mine = picks.workspace === workspace ? picks : { ...NO_PICKS, workspace };
  return pageBot ? { ...mine, page: { key: pageKey, id } } : { ...mine, general: id };
}
