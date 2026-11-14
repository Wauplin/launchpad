// One-time agents launched from GitHub (I6 Investigate and Fix in a Local PR on issues): a popover with the profile
// (the last one chosen in this tab, else the first subscription profile), an optional note and Launch. The agent runs
// in the folder of the page's repository, in its own worktree: read only, or ending in a Local PR. The item goes by reference (github_refs); its text is never sent.

import type { ExtContext } from "@/gen/ExtContext";
import type { GitHubRef } from "@/gen/GitHubRef";
import { request, type LaunchWhenDone, type ProfileBrief } from "../../shared/messages";
import { itemTitle } from "./item";
import { button, h, popover } from "./widgets";

export type LaunchKind = "investigate" | "fix";

const TITLES: Record<LaunchKind, string> = { investigate: "Investigate", fix: "Fix in a Local PR" };
const WHEN: Record<LaunchKind, LaunchWhenDone> = { investigate: "read_only", fix: "local_pr" };

let profilesCache: Promise<ProfileBrief[]> | null = null;
/** The profile chosen last in this tab. */
let lastProfile: string | null = null;

/** The task each launch gets: the owner's note first, then how to work and end. */
export function launchText(kind: LaunchKind, ref: GitHubRef, note: string) {
  const item = `${ref.repo}#${ref.number}`;
  switch (kind) {
    case "investigate": return `${note || `Investigate issue ${item}: find the root cause.`}\n\nThis is a read-only investigation: change nothing. End with the root cause, the files and lines involved, and what a fix needs.`;
    case "fix": return `${note || `Fix issue ${item}.`}\n\nWork in your worktree and add a test only if the fix clearly needs one. When done, run \`launchpad work finish\` to open a Local PR whose description starts with "Fixes ${item}", for me to review and publish.`;
  }
}

/** Opens the launch popover under `anchor`. */
export function launchPopover(root: ShadowRoot, anchor: HTMLElement, ctx: ExtContext, kind: LaunchKind, ref: GitHubRef, done: () => void) {
  const folder = ctx.folder;
  const select = h("select", { attrs: { "aria-label": "Profile" } }, h("option", { text: "Loading profiles…", attrs: { value: "" } }));
  const note = h("textarea", { attrs: { "aria-label": "Note for the agent (optional)", placeholder: kind === "fix" ? "What the fix should do (optional)" : "What to look at (optional)" } });
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const go = h("button", { class: "lp-btn lp-primary", text: "Launch", attrs: { type: "button", disabled: "" } });
  let busy = false;
  const title = ref.repo && ref.number ? itemTitle({ repo: ref.repo, number: ref.number }) : null;
  const where = folder ? `Folder ${folder.name} · ${folder.workspace} · ${kind === "fix" ? "ends in a Local PR" : "read only"}` : "";
  const close = popover(root, anchor, TITLES[kind], (close) => [
    h("div", { class: "lp-muted", text: `${ref.repo}#${ref.number}${title ? ` · ${title}` : ""}` }),
    select, note, error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-muted lp-grow", text: where }), button("Cancel", close, { small: true }), go),
  ]);
  const fail = (e: unknown) => { error.textContent = e instanceof Error ? e.message : String(e); error.hidden = false; };
  if (!folder || folder.relation === "fork") {
    fail("Add this repository to Launchpad first: agents run in a folder of it.");
    return;
  }
  const submit = async () => {
    if (busy || !select.value) return;
    busy = true;
    go.disabled = true;
    go.textContent = "Launching…";
    error.hidden = true;
    try {
      lastProfile = select.value;
      await request("launch", { folder: folder.id, text: launchText(kind, ref, note.value.trim()), github_refs: [ref], when_done: WHEN[kind], profile: select.value, key: crypto.randomUUID() });
      close();
      done();
    } catch (e) {
      fail(e);
      busy = false;
      go.disabled = false;
      go.textContent = "Launch";
    }
  };
  go.addEventListener("click", (e) => { e.preventDefault(); void submit(); });
  note.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); void submit(); }
    e.stopPropagation();
  });
  profilesCache ??= request("profiles");
  profilesCache.then((list) => {
    select.replaceChildren(...list.map((p) => h("option", { text: `${p.name} · ${p.harness}`, attrs: { value: p.id } })));
    if (!list.length) return fail("No profile on this Home: add one in Launchpad (Settings → Profiles).");
    const first = [lastProfile, list.find((p) => p.subscription)?.id, list[0]?.id].find((id) => id && list.some((p) => p.id === id));
    if (first) select.value = first;
    go.disabled = false;
    note.focus();
  }).catch((e) => { profilesCache = null; fail(e); });
}
