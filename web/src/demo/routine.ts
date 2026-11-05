// The message a bot routine sends to its thread, as Home writes it (`crons::bot_message`). The thread shows its
// first line as one compact row.

export function routineText(title: string, cause: string, instructions: string, context = ""): string {
  const what = context ? ` What started it:\n\n${context}\n` : "";
  return `Routine “${title}” · ${cause}\n\n${instructions.trim()}\n\nThis is a background run of the routine “${title}” (${cause}). Nobody is watching it.${what}\nPrepare work for the owner's review (branches, Local PRs, suggestions, drafts in the Outbox) unless these instructions explicitly ask you to publish. When you finish, write a short report for the owner. If there is nothing worth reporting, reply with exactly \`no_update\` and nothing else: Launchpad then stays silent.`;
}

export const DAILY = "Look at what changed in your folder and in the knowledge base since your last run. Report only what needs my attention, in a few lines.";
export const CI = "Read the failing logs below. If the change itself causes the failure, prepare a fix in a new worktree as a Local PR and report. If it is flaky or unrelated, reply no_update.";
