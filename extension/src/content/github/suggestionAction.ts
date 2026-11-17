const inFlight = new Set<string>();

/** Keep publication limited to untouched drafts; Home owns the publication lock and reviewed-head checks. */
export async function submitSuggestion(
  suggestion: { id: string; kind: string; status: string },
  close: boolean,
  send: (args: { id: string; close: boolean }) => Promise<unknown>,
): Promise<unknown> {
  if (suggestion.status !== "draft") throw new Error("Check GitHub before attempting publication again.");
  if (close && suggestion.kind !== "comment") throw new Error("Reviews cannot close a pull request without merging.");
  if (inFlight.has(suggestion.id)) throw new Error("Publication is already in progress.");
  inFlight.add(suggestion.id);
  try {
    return await send({ id: suggestion.id, close });
  } finally {
    inFlight.delete(suggestion.id);
  }
}
