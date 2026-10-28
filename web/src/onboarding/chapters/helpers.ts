// Small helpers for chapter steps: navigation, clicks on anchors, waiting for the UI.

import type { AnyRouter } from "@tanstack/react-router";

let router: AnyRouter | null = null;
/** TourRoot registers the app router, so steps can navigate. */
export const setTourRouter = (r: AnyRouter) => { router = r; };
export const navigate = (to: string) => router?.navigate({ to } as never);
export const currentPath = () => (router ? router.state.location.pathname + (router.state.location.searchStr ?? "") : location.pathname + location.search);

/** The first visible element with this `data-tour` anchor (and `data-tour-id`). */
export function anchor(name: string, id?: string): HTMLElement | null {
  for (const el of document.querySelectorAll<HTMLElement>(`[data-tour="${name}"]${id ? `[data-tour-id="${id}"]` : ""}`)) if (el.getClientRects().length) return el;
  return null;
}

/** Waits up to `ms` for `check` to return a value. */
export async function waitFor<T>(check: () => T | null | undefined | false, ms = 5000): Promise<T | null> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = check();
    if (v) return v;
    await new Promise((r) => setTimeout(r, 100));
  }
  return null;
}

/** Clicks an element, or the first enabled button inside it when it is a wrapper. */
export function activate(el: HTMLElement | null) {
  if (!el) return;
  const target = el.matches("button,a,[role=button],[role=tab]") ? el : el.querySelector<HTMLElement>("button:not(:disabled),a,[role=button],[role=tab]") ?? el;
  target.click();
}

/** Clicks an anchor once it is on screen. */
export async function clickAnchor(name: string, id?: string) {
  activate(await waitFor(() => anchor(name, id)));
}

/** Clicks the first visible button whose text or label is `label`, inside `within` when given. */
export async function clickButton(label: string, within?: string) {
  const el = await waitFor(() => {
    const root = within ? anchor(within) : document;
    return [...(root?.querySelectorAll<HTMLElement>("button,[role=button],a,[role=tab],[role=menuitem]") ?? [])].find((b) => b.getClientRects().length && (b.textContent?.trim() === label || b.getAttribute("aria-label") === label));
  });
  el?.click();
}

/** Types into a React-controlled input or textarea. */
export function typeInto(el: HTMLInputElement | HTMLTextAreaElement | null, text: string) {
  if (!el) return;
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, text);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}
