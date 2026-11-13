// Ask a bot about items of a list (M2): one or several pull requests or issues (a row's hover action, the rows selected
// with GitHub's own checkboxes, a notification), or a repository without a folder (Ask without cloning). The bot gets
// the items by reference (`github_refs`), never their text. Review with agent remembers the bot and preset per
// repository (`github.review-choice`). Nothing is sent without the popover's Send.

import type { ExtPreset } from "@/gen/ExtPreset";
import type { ExtReviewChoice } from "@/gen/ExtReviewChoice";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Session } from "@/gen/Session";
import { request, type BotBrief } from "../../shared/messages";
import { workspaceBots } from "./item";
import { button, h, popover } from "./widgets";

export type Target = { repo: string; number: number | null; kind: "pr" | "issue" | "repo"; title?: string | null };
export type TargetKind = "ask" | "review" | "reply" | "many" | "repo";

const TITLES: Record<TargetKind, string> = { ask: "Ask a bot", review: "Review with agent", reply: "Draft a reply", many: "Launch one agent with them", repo: "Ask without cloning" };

/** The reference the bot gets: repository and number only (a title read from the page is never sent: Home and the
 * agent read the item themselves). */
export function refOfTarget(t: Target): GitHubRef {
  if (t.kind === "repo" || t.number === null) return { url: `https://github.com/${t.repo}`, kind: "repo", repo: t.repo, number: null, title: null };
  return { url: `https://github.com/${t.repo}/${t.kind === "pr" ? "pull" : "issues"}/${t.number}`, kind: t.kind, repo: t.repo, number: t.number, title: null };
}

const name = (t: Target) => (t.number === null ? t.repo : `${t.repo}#${t.number}`);

/** The message: the owner's note (after the preset's text), then how answers come back to GitHub. */
export function targetText(targets: Target[], kind: TargetKind, note: string, preset: ExtPreset | null) {
  const t = targets[0];
  const where = t.number !== null ? `--repo ${t.repo} --number ${t.number}` : "";
  let text: string;
  if (kind === "repo") text = `${note || `Look at the repository ${t.repo}: what is it, what is going on there, and what should I know?`}\n\nIt has no folder in Launchpad: you work in a temporary folder. Read the repository on GitHub (the gh CLI, the GitHub API) or clone what you need there; answer here.`;
  else if (kind === "many") text = `${note || "Look at these items together and tell me what to do with each."}\n\n${targets.map((x) => `- ${name(x)}`).join("\n")}\n\nWhen something should be posted on one of them, draft it as a suggestion (\`launchpad suggest comment|review --repo <repo> --number <number> …\`) for me to post.`;
  else if (kind === "review") text = `${note || "Review this pull request."}\n\nPut your findings in a suggested review (\`launchpad suggest review ${where} --head <commit you reviewed>\`, line comments with --comments-file) for me to post.`;
  else if (kind === "reply") text = `${note || `Draft a reply to this ${t.kind === "issue" ? "issue" : "pull request"}.`}\n\nIf a reply helps, draft it as a suggested comment (\`launchpad suggest comment ${where} --body-file <file>\`); I insert it into GitHub's comment box and post it myself.`;
  else if (t.number === null) text = `${note || `Look at the repository ${t.repo}: what is going on there, and what should I do?`}\n\nWhen something should be posted on GitHub, draft it as a suggestion (\`launchpad suggest comment|review --repo ${t.repo} --number <number> …\`) for me to post.`;
  else text = `${note || "Look at this item: what is it about, and what should I do?"}\n\nWhen something should be posted, draft it as a suggestion (\`launchpad suggest comment|review ${where} …\`) for me to post.`;
  return preset ? `${preset.text}\n\n${text}` : text;
}

export type TargetOptions = {
  targets: Target[];
  kind: TargetKind;
  /** The bot chosen first (the repository's remembered or default bot). */
  bot?: string | null;
  presets?: ExtPreset[];
  /** Review with agent on one repository: the bot and preset are remembered for it. */
  remember?: { repo: string; choice: ExtReviewChoice | null };
  /** A muted line about where the agent works. */
  where?: string;
  done?: (s: Session) => void;
};

/** Opens the popover under `anchor`: bot (and preset), an optional note, Send. */
export function targetPopover(root: ShadowRoot, anchor: HTMLElement, o: TargetOptions) {
  const select = h("select", { attrs: { "aria-label": "Bot" } }, h("option", { text: "Loading bots…", attrs: { value: "" } }));
  const presets = o.presets ?? [];
  const presetSelect = presets.length ? h("select", { attrs: { "aria-label": "Preset" } }, h("option", { text: "No preset", attrs: { value: "" } }), ...presets.map((p) => h("option", { text: p.label, attrs: { value: p.id } }))) : null;
  if (presetSelect && o.remember?.choice?.preset && presets.some((p) => p.id === o.remember?.choice?.preset)) presetSelect.value = o.remember.choice.preset;
  const note = h("textarea", { attrs: { "aria-label": "Note for the bot (optional)", placeholder: o.kind === "review" ? "What to look at (optional)" : "Your note (optional)" } });
  const what = o.targets.length === 1 ? `${name(o.targets[0])}${o.targets[0].title ? ` · ${o.targets[0].title}` : ""}` : `${o.targets.length} items: ${o.targets.slice(0, 4).map(name).join(", ")}${o.targets.length > 4 ? "…" : ""}`;
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const send = h("button", { class: "lp-btn lp-primary", text: "Send", attrs: { type: "button", disabled: "" } });
  let bots: BotBrief[] = [];
  let busy = false;
  const close = popover(root, anchor, TITLES[o.kind], (close) => {
    const parts: HTMLElement[] = [h("div", { class: "lp-muted", text: what }), select];
    if (presetSelect) parts.push(presetSelect);
    parts.push(note, error, h("div", { class: "lp-row" }, h("span", { class: "lp-muted lp-grow", text: o.where ?? "" }), button("Cancel", close, { small: true }), send));
    return parts;
  });
  const fail = (e: unknown) => { error.textContent = e instanceof Error ? e.message : String(e); error.hidden = false; };
  const submit = async () => {
    const bot = bots.find((b) => b.id === select.value);
    if (!bot || busy) return;
    busy = true;
    send.disabled = true;
    send.textContent = "Sending…";
    error.hidden = true;
    const preset = presets.find((p) => p.id === presetSelect?.value) ?? null;
    try {
      const session = await request("bot.message", { bot: bot.id, text: targetText(o.targets, o.kind, note.value.trim(), preset), github_refs: o.targets.slice(0, 10).map(refOfTarget), key: crypto.randomUUID() });
      if (o.remember) await request("review-choice", { repo: o.remember.repo, bot: bot.id, preset: preset?.id ?? null }).catch((e) => console.warn("Launchpad: remember", e));
      close();
      o.done?.(session);
    } catch (e) {
      fail(e);
      busy = false;
      send.disabled = false;
      send.textContent = "Send";
    }
  };
  send.addEventListener("click", (e) => { e.preventDefault(); void submit(); });
  note.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); void submit(); }
    e.stopPropagation();
  });
  void workspaceBots().then(({ bots: list, ask }) => {
    bots = list.filter((b) => !b.paused);
    select.replaceChildren(...bots.map((b) => h("option", { text: b.name, attrs: { value: b.id } })));
    if (!bots.length) { fail("No bot in this workspace: create one in Launchpad (Bots → New bot)."); return; }
    const first = [o.remember?.choice?.bot, o.bot, ask].find((id) => id && bots.some((b) => b.id === id));
    if (first) select.value = first;
    send.disabled = false;
    note.focus();
  }).catch(fail);
}
