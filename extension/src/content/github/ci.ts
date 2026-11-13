// I8 CI approval helper: next to GitHub's "Approve and run" of a fork's pull request (workflows that wait for a
// maintainer), a note of what the pull request changes in files that decide what CI runs (workflows, scripts,
// dependencies, build files), from Home's cached files of its current head, and Check with an agent (a read-only
// launch). GitHub's button stays the click: nothing here touches it.

import type { ExtContext } from "@/gen/ExtContext";
import { findAnchor } from "./anchors";
import { open, refOf, type ItemPage } from "./item";
import { launchPopover } from "./launchPopover";
import { button, h, logo, more, mount, unmount } from "./widgets";

const ID = "ci-note";

export function renderCiNote(page: ItemPage, ctx: ExtContext | null, refresh: () => void) {
  const found = ctx && page.kind === "pr_conversation" ? findAnchor("ci-approval", { repo: page.repo, number: page.number }) : null;
  if (!ctx || !found) return unmount(ID);
  const ci = ctx.ci && ctx.ci.head === ctx.page.head ? ctx.ci : null;
  const w = mount(ID, JSON.stringify([ci, ctx.folder?.id, ctx.page.head]), found.el, found.place, "banner");
  if (!w.fresh) return;
  const ws = ctx.folder?.workspace_id ?? ctx.workspace?.id ?? null;
  const files = ci ? [...ci.relevant, ...(ci.more ? [`and ${ci.more} more`] : [])].join(", ") : "";
  const text = !ci
    ? "Home has not read this pull request's files at its current head yet: open them in Launchpad, or check with an agent."
    : ci.relevant.length
      ? `Changes files that decide what CI runs: ${files}.`
      : `No workflow, script, dependency or build file among its ${ci.files} changed file${ci.files === 1 ? "" : "s"}.`;
  const repoFolder = !!ctx.folder && ctx.folder.relation !== "fork";
  const extra = ci ? (ci.relevant.length ? `Its workflow-relevant files: ${files}. ` : `Home found no workflow, script, dependency or build file among its ${ci.files} changed files; confirm it. `) : "";
  const check: HTMLButtonElement = button("Check with an agent", () => launchPopover(w.root, check, ctx, "ci", refOf(page, ctx), refresh, extra), { small: true, disabled: !repoFolder, title: repoFolder ? "A read-only agent reads what would run and says whether approving is safe" : "Add this repository to Launchpad first" });
  const row = h("div", { class: "lp-row" }, logo(), h("span", { class: "lp-title", text: "Before you approve" }), h("span", { class: "lp-grow" }), check,
    more(w.root, "CI", () => [
      { label: "Open its files in Launchpad", hint: "The diff, from Home's copy", onSelect: () => open(`/github/${page.repo}/${page.number}?tab=files`, ws) },
      { note: "GitHub's Approve and run stays your click." },
    ]));
  w.root.append(h("div", { class: "lp-banner" }, row, h("div", { class: ci?.relevant.length ? "lp-warn" : "lp-muted", text, attrs: { style: "margin-top:4px" } })));
}
