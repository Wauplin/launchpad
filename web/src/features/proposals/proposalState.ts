import type { Proposal } from "../../gen/Proposal";

export function isWaiting(p: Pick<Proposal, "state">): boolean {
  return p.state === "proposed" || p.state === "failed" || p.state === "uncertain";
}

/** Start all covers these: waiting cards that start something. A status list has nothing to start, a report only
 * informs, a ready click runs only from its own button, and failed or uncertain cards wait for the owner's look. */
export function isStartable(p: Pick<Proposal, "state" | "body">): boolean {
  return p.state === "proposed" && p.body.kind !== "status" && p.body.kind !== "ready" && p.body.kind !== "report";
}

/** Keep a linked successor visible even when it falls outside the recent history. */
export function visibleProposals<T extends Pick<Proposal, "id" | "state">>(proposals: T[], waitingOnly: boolean, reveal: string | null): T[] {
  if (waitingOnly) return proposals.filter(isWaiting);
  return proposals.filter((p, i) => i < 50 || p.id === reveal);
}

/** Names a card's plain-words line needs, looked up by the caller (folders, bots, agents, the action list). */
export type CardNames = {
  folder?: (id: string) => string | undefined;
  bot?: (id: string) => string | undefined;
  session?: (id: string) => string | undefined;
  action?: (name: string) => string | undefined;
};

export const REPORT_OUTCOME: Record<Extract<Proposal["body"], { kind: "report" }>["outcome"], string> = { needs_you: "Needs you", merged: "Merged", approved: "Approved", commented: "Commented", waiting: "Waiting", closed: "Closed", info: "Info" };

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** `title = "…"` of a routine file (`routines/<name>.toml`), else its file name. */
function routineName(path: string, text: string | null | undefined): string {
  return text?.match(/^title\s*=\s*"((?:[^"\\]|\\.)*)"/m)?.[1] ?? path.replace(/^routines\//, "").replace(/\.toml$/, "");
}

/** "a, b and c" */
const list = (parts: string[]) => (parts.length < 2 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts.at(-1)}`);

/** The second line of a compact card in a bot's chat: what accepting it does, in plain words. */
export function proposalPurpose(p: Pick<Proposal, "body">, n: CardNames = {}): string {
  const b = p.body;
  const where = (spec: Extract<Proposal["body"], { kind: "launch" }>["spec"]) =>
    spec.target.kind === "temp" ? "a temporary folder" : "project_id" in spec.target ? (spec.target.project_id && n.folder?.(spec.target.project_id)) || "a folder" : "a folder";
  switch (b.kind) {
    case "launch": {
      const done = b.spec.worktree?.when_done;
      return `Starts an agent in ${where(b.spec)}${done === "merge" ? "; it merges when done" : done === "local_pr" ? "; it opens a Local PR when done" : done === "read_only" ? ", read only" : ""}`;
    }
    case "message": return `Sends a message to the agent “${n.session?.(b.session_id) ?? "an agent"}”`;
    case "schedule": return `Starts an agent in ${where(b.spec)} later, once ${b.conditions.length === 1 ? "its start condition holds" : `its ${plural(b.conditions.length, "start condition")} hold`}`;
    case "cron": return `Creates a routine that starts an agent in ${where(b.spec)}`;
    case "routine": return `Adds a routine to ${n.bot?.(b.bot_id) ?? "a bot"}; it starts nothing now`;
    case "preset": return b.preset_id ? `Updates the preset “${b.label}”` : `Saves a new preset “${b.label}”`;
    case "tool": return `Adds the tool ${b.tool.name}${b.tool.hosts.length ? `; its token goes to ${b.tool.hosts.map((h) => h.host).join(", ")}` : ""}`;
    case "bot": return `Creates the bot ${b.name}; it starts nothing`;
    case "bot_instructions": return `Changes the BOT.md of ${n.bot?.(b.bot_id) ?? "a bot"}`;
    case "bot_files": {
      const paths = Object.keys(b.files).sort((x, y) => (x === "BOT.md" ? -1 : y === "BOT.md" ? 1 : x.localeCompare(y)));
      const parts = paths.map((path) => {
        if (path === "BOT.md") return "changes BOT.md";
        const name = routineName(path, b.files[path] ?? null);
        return b.files[path] == null ? `deletes the routine ${name}` : b.before[path] == null ? `adds the routine ${name}` : `changes the routine ${name}`;
      });
      const text = `${list(parts)} of ${n.bot?.(b.bot_id) ?? "a bot"}`;
      return text.charAt(0).toUpperCase() + text.slice(1);
    }
    case "channel": return `Creates the channel #${b.name}${b.members.length ? ` with ${list(b.members.map((m) => m.name || "a bot"))}` : ""}; it starts nothing`;
    case "channel_join": return `Adds ${b.bot_name || "a bot"} to #${b.channel_name} (mode ${b.mode})`;
    case "ready": return `Your click: ${n.action?.(b.action) ?? b.action}`;
    case "report": return `${REPORT_OUTCOME[b.outcome]}${b.github ? ` · ${b.github}` : ""}`;
    case "status": return `Lists ${plural(b.session_ids.length, "agent")} to look at`;
  }
}

/** The one-line receipt of a card that was handled: what happened to it. Null while it waits. */
export function proposalReceipt(p: Pick<Proposal, "body" | "state" | "title" | "replaced_by">): string | null {
  const t = p.title;
  switch (p.state) {
    case "proposed": case "failed": case "uncertain": return null;
    case "starting": return `Starting ${t}…`;
    case "dismissed": return p.body.kind === "report" ? `Read: ${t}` : `Dismissed ${t}`;
    case "replaced": return p.replaced_by ? `Replaced ${t}` : `Edited in the composer: ${t}`;
    case "started": break;
  }
  switch (p.body.kind) {
    case "launch": return `Started ${t}`;
    case "message": return `Sent ${t}`;
    case "schedule": return `Scheduled ${t}`;
    case "cron": case "routine": return `Added routine ${t}`;
    case "preset": return `Saved preset ${t}`;
    case "tool": return `Added tool ${t}`;
    case "bot": return `Created bot ${t}`;
    case "bot_instructions": return `Applied to BOT.md: ${t}`;
    case "bot_files": return `Applied ${t}`;
    case "channel": return `Created channel ${t}`;
    case "channel_join": return `Accepted: ${t}`;
    case "ready": return `Done: ${t}`;
    case "report": return `Replied: ${t}`;
    case "status": return `Seen: ${t}`;
  }
}
