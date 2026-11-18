import type { ItemAction } from "@/gen/ItemAction";

const inFlight = new Set<string>();

/** Keep publication limited to untouched drafts; Home owns the publication lock and reviewed-head checks. */
export async function submitSuggestion(
  suggestion: { id: string; kind: string; status: string },
  actions: ItemAction[],
  send: (args: { id: string; actions: ItemAction[] }) => Promise<unknown>,
): Promise<unknown> {
  if (suggestion.status !== "draft") throw new Error("Check GitHub before attempting publication again.");
  if (actions.some((a) => a.kind === "close") && suggestion.kind === "review") throw new Error("Reviews cannot close a pull request without merging.");
  if (suggestion.kind === "action" && !actions.length) throw new Error("Every action is unticked: discard it instead.");
  if (inFlight.has(suggestion.id)) throw new Error("Publication is already in progress.");
  inFlight.add(suggestion.id);
  try {
    return await send({ id: suggestion.id, actions });
  } finally {
    inFlight.delete(suggestion.id);
  }
}
