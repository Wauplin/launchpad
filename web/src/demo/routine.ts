// The message a bot routine sends to its thread, as Home writes it (`crons::bot_message`). The thread shows its
// first line as one compact row. And the routine files of bots (`routines/<name>.toml`), written and read like Home.

import type { CronAgent } from "@/gen/CronAgent";
import type { CronCondition } from "@/gen/CronCondition";

export function routineText(title: string, cause: string, instructions: string, context = ""): string {
  const what = context ? ` What started it:\n\n${context}\n` : "";
  return `Routine “${title}” · ${cause}\n\n${instructions.trim()}\n\nThis is a background run of the routine “${title}” (${cause}). Nobody is watching it.${what}\nPrepare work for the owner's review (branches, Local PRs, suggestions, drafts in the Outbox) unless these instructions explicitly ask you to publish. When you finish, write a short report for the owner. If there is nothing worth reporting, reply with exactly \`no_update\` and nothing else: Launchpad then stays silent.`;
}

export const DAILY = "Look at what changed in your folder and in the knowledge base since your last run. Report only what needs my attention, in a few lines.";
export const CI = "Read the failing logs below. If the change itself causes the failure, prepare a fix in a new worktree as a Local PR and report. If it is flaky or unrelated, reply no_update.";

// ---------------------------------------------------------------- routine files

/** What a routine file holds (Home's `crons::RoutineFile`). */
export type RoutineDef = { title: string; paused: boolean; schedule: string; time_zone?: string; instructions: string; trigger: CronAgent["trigger"]; conditions: CronCondition[] };

const HEADER = "# A routine of this bot: its runs are messages to the bot's thread. Saved from the bot's page; a hand edit\n# applies at once. trigger.kind: schedule (with schedule, a 5-field cron), github, session_done or failure.\n";
const str = (v: string) => JSON.stringify(v);
const value = (v: unknown): string => Array.isArray(v) ? `[${v.map(value).join(", ")}]` : typeof v === "string" ? str(v) : String(v);
const table = (o: Record<string, unknown>) => Object.entries(o).filter(([, v]) => v != null).map(([k, v]) => `${k} = ${value(v)}`).join("\n");

/** `routines/<name>.toml` as Home writes it (`crons::render_file`). */
export function routineToml(d: RoutineDef): string {
  const top = [`title = ${str(d.title)}`, d.paused ? "paused = true" : "", d.schedule ? `schedule = ${str(d.schedule)}` : "", d.time_zone ? `time_zone = ${str(d.time_zone)}` : "", `instructions = """\n${d.instructions}"""`].filter(Boolean).join("\n");
  const conditions = d.conditions.map((c) => `\n[[conditions]]\n${table(c)}\n`).join("");
  return `${HEADER}${top}\n\n[trigger]\n${table(d.trigger)}\n${conditions}`;
}

/** The definition of a routine record, as its file holds it. */
export const defOf = (c: CronAgent): RoutineDef => ({ title: c.title, paused: c.paused, schedule: c.schedule, ...(c.time_zone ? { time_zone: c.time_zone } : {}), instructions: c.spec.text, trigger: c.trigger, conditions: c.conditions });

/** Reads the routine files the demo writes (and simple hand edits of them): keys, strings, `"""` strings, arrays,
 * `[trigger]` (or an inline `trigger = { … }`) and `[[conditions]]`. Throws the error a real Home would show. */
export function parseRoutineToml(text: string): RoutineDef {
  const out: Record<string, unknown> = {};
  const conditions: Record<string, unknown>[] = [];
  let target: Record<string, unknown> = out;
  const lines = text.split("\n");
  const parse = (raw: string): unknown => {
    const v = raw.trim();
    if (v === "true" || v === "false") return v === "true";
    if (/^-?\d+$/.test(v)) return Number(v);
    if (v.startsWith("[")) return v.slice(1, -1).split(",").map((x) => x.trim()).filter(Boolean).map(parse);
    if (v.startsWith("{")) return Object.fromEntries(v.slice(1, -1).split(/,(?![^[]*\])/).map((kv) => kv.split("=")).map(([k, x]) => [k.trim(), parse(x)]));
    if (v.startsWith("\"")) return JSON.parse(v);
    throw new Error(`routine file: cannot read ${v}`);
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line || line.startsWith("#")) continue;
    if (line === "[trigger]") { target = (out.trigger = {}) as Record<string, unknown>; continue; }
    if (line === "[[conditions]]") { target = {}; conditions.push(target); continue; }
    const m = line.match(/^([a-z_]+)\s*=\s*(.*)$/);
    if (!m) throw new Error(`routine file: cannot read line ${i + 1}`);
    if (m[2].startsWith("\"\"\"")) {
      const body: string[] = [];
      let rest = m[2].slice(3).replace(/^\n/, "");
      while (!rest.includes("\"\"\"")) { body.push(rest); rest = lines[++i] ?? "\"\"\""; }
      body.push(rest.slice(0, rest.indexOf("\"\"\"")));
      target[m[1]] = body.join("\n").replace(/^\n/, "");
    } else target[m[1]] = parse(m[2]);
  }
  const d: RoutineDef = { title: String(out.title ?? "").trim(), paused: out.paused === true, schedule: String(out.schedule ?? ""), ...(out.time_zone ? { time_zone: String(out.time_zone) } : {}), instructions: String(out.instructions ?? "").trim(), trigger: (out.trigger as CronAgent["trigger"]) ?? { kind: "schedule" }, conditions: conditions as CronCondition[] };
  if (!d.title) throw new Error("routine file: give the routine a title");
  if (!d.instructions) throw new Error("routine file: write what the bot does at each run (instructions)");
  return d;
}

/** `routines/<slug of the title>.toml`, not one of `taken` (Home's `crons::file_path_for`). */
export function routinePath(title: string, taken: string[]): string {
  const base = title.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 32) || "routine";
  let name = base;
  for (let n = 2; taken.includes(`routines/${name}.toml`); n++) name = `${base}-${n}`;
  return `routines/${name}.toml`;
}

// The Maintainer's BOT.md, and the change it proposes with its new routine (a bot-files card).
export const MAINTAINER_BOT_MD = "# Maintainer\n\nMaintain storefront: answer my questions about the code, investigate issues and failing checks, and prepare fixes as Local PRs for my review. Keep answers short and cite commits.\n";
/** Its BOT.md before your last edit: its current session still runs this one ("Instructions updated"). */
export const MAINTAINER_BOT_MD_PREV = "# Maintainer\n\nMaintain storefront: answer my questions about the code, investigate issues and failing checks, and prepare fixes as Local PRs for my review.\n";
/** A short version of BOT.md (Home's `bots::version`, a hash): sessions record the one they run. */
export const botVersion = (text: string) => { let a = 5381, b = 52711; for (let i = 0; i < text.length; i++) { const c = text.charCodeAt(i); a = (a * 33) ^ c; b = (b * 31) ^ c; } return ((a >>> 0).toString(16).padStart(8, "0") + (b >>> 0).toString(16).padStart(8, "0")); };
export const MAINTAINER_BOT_MD_NEXT = `${MAINTAINER_BOT_MD}\nReview requests: prepare each review as a suggestion for me to post, never post it yourself.\n`;
export const MAINTAINER_BOT_MD_PATCH = "@@ -2,3 +2,5 @@\n \n Maintain storefront: answer my questions about the code, investigate issues and failing checks, and prepare fixes as Local PRs for my review. Keep answers short and cite commits.\n+\n+Review requests: prepare each review as a suggestion for me to post, never post it yourself.\n";
export const REVIEW_ROUTINE = routineToml({ title: "Review requests", paused: false, schedule: "", instructions: "Review each pull request below. Prepare your review as a suggestion for me to post; do not post it yourself. Report one line per pull request with your verdict.", trigger: { kind: "github", repo: "acme-labs/storefront", events: ["review_requested"] }, conditions: [] });
