// R3 Add to Launchpad: Clone from GitHub into a workspace you pick (`folder.clone`, the same call as Launchpad's Clone
// from GitHub). R4 Ask without cloning: a bot of the extension's workspace answers about the repository (or one of its
// items) with it as its only context; a bot without a folder works in a temporary folder of the workspace.

import type { ExtContext } from "@/gen/ExtContext";
import { request } from "../../shared/messages";
import { targetPopover, type Target } from "./target";
import { button, h, popover } from "./widgets";

/** Opens the Add to Launchpad popover under `anchor`. */
export function addPopover(root: ShadowRoot, anchor: HTMLElement, repo: string, done: () => void) {
  const select = h("select", { attrs: { "aria-label": "Workspace" } }, h("option", { text: "Loading workspaces…", attrs: { value: "" } }));
  const error = h("div", { class: "lp-error", attrs: { role: "alert", hidden: "" } });
  const go = h("button", { class: "lp-btn lp-primary", text: "Clone", attrs: { type: "button", disabled: "" } });
  const close = popover(root, anchor, "Add to Launchpad", (close) => [
    h("div", { class: "lp-muted", text: `Clone ${repo} from GitHub into a new folder of a workspace. Home keeps a mirror; agents get worktrees of it.` }),
    select,
    error,
    h("div", { class: "lp-row" }, h("span", { class: "lp-grow" }), button("Cancel", close, { small: true }), go),
  ]);
  const fail = (e: unknown) => { error.textContent = e instanceof Error ? e.message : String(e); error.hidden = false; };
  go.addEventListener("click", (e) => {
    e.preventDefault();
    if (!select.value || go.disabled) return;
    go.disabled = true;
    go.textContent = "Cloning…";
    error.hidden = true;
    request("folder.clone", { repo, workspace_id: select.value })
      .then(() => { close(); done(); })
      .catch((err) => { fail(err); go.disabled = false; go.textContent = "Clone"; });
  });
  void request("state").then((s) => {
    if (!s.workspaces.length) { fail("No workspace yet: create one in Launchpad first."); return; }
    select.replaceChildren(...s.workspaces.map((w) => h("option", { text: w.name, attrs: { value: w.id } })));
    if (s.workspace) select.value = s.workspace;
    go.disabled = false;
  }).catch(fail);
}

/** Opens Ask without cloning under `anchor` for the repository, or one of its items. */
export function askWithoutCloning(root: ShadowRoot, anchor: HTMLElement, ctx: ExtContext | null, target: Target, done?: () => void) {
  targetPopover(root, anchor, {
    targets: [target],
    kind: target.kind === "repo" ? "repo" : "ask",
    bot: ctx?.defaults.bot?.id ?? null,
    presets: ctx?.presets ?? [],
    where: `${ctx?.workspace?.name ?? "Your workspace"} · no clone`,
    done: () => done?.(),
  });
}
