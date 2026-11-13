// Widgets of the GitHub layer: each lives in its own Shadow DOM host (GitHub's CSS cannot reach it, ours cannot reach
// GitHub), styled with GitHub's Primer variables (light, dark and high-contrast themes follow) plus a teal accent and
// the 14 px Launchpad logo, so you always know what Launchpad added. Built with `h()` from text only: never page HTML.
// One visible button plus a menu per place.

import logoUrl from "../../../public/icons/48.png?inline";

/** Launchpad's teal (web/src/styles.css `--color-teal`, `--color-teal-ink`). */
const TEAL = { light: "#0d7266", lightInk: "#0a5c52", dark: "#0d7266", darkInk: "#79d7c6" };

const STYLE = `
:host { all: initial; display: inline-flex; vertical-align: middle; position: relative; margin: 0 4px;
  font: 500 12px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", "Noto Sans", Helvetica, Arial, sans-serif;
  color: var(--fgColor-default, var(--color-fg-default, #1f2328));
  --lp-teal: ${TEAL.light}; --lp-ink: ${TEAL.lightInk}; }
:host([data-dark]) { --lp-teal: ${TEAL.dark}; --lp-ink: ${TEAL.darkInk}; }
* { box-sizing: border-box; }
.lp-chip { display: inline-flex; align-items: center; gap: 5px; height: 24px; max-width: 360px; padding: 0 8px 0 6px; border-radius: 999px;
  border: 1px solid var(--borderColor-default, var(--color-border-default, #d1d9e0));
  background: var(--bgColor-muted, var(--color-canvas-subtle, #f6f8fa)); color: inherit; font: inherit; cursor: pointer; white-space: nowrap; }
.lp-chip:hover { background: var(--control-transparent-bgColor-hover, var(--bgColor-neutral-muted, #818b981f)); }
.lp-chip:focus-visible { outline: 2px solid var(--focus-outlineColor, var(--lp-teal)); outline-offset: 1px; }
.lp-chip.lp-accent { border-color: color-mix(in srgb, var(--lp-teal) 55%, transparent); color: var(--lp-ink); }
.lp-chip.lp-quiet { color: var(--fgColor-muted, var(--color-fg-muted, #59636e)); }
.lp-text { overflow: hidden; text-overflow: ellipsis; }
.lp-caret { width: 10px; height: 10px; flex: none; opacity: .8; }
.lp-logo { width: 14px; height: 14px; flex: none; border-radius: 3px; }
.lp-menu { position: fixed; z-index: 2147483000; font-size: 13px; min-width: 260px; max-width: 340px; padding: 4px; border-radius: 12px;
  background: var(--overlay-bgColor, var(--bgColor-default, var(--color-canvas-overlay, #ffffff)));
  border: 1px solid var(--borderColor-default, var(--color-border-default, #d1d9e0));
  box-shadow: var(--shadow-floating-large, 0 8px 24px rgba(140, 149, 159, .2)); }
.lp-head { display: flex; align-items: center; gap: 6px; padding: 6px 8px 4px; color: var(--fgColor-muted, #59636e); font-weight: 400; }
.lp-item { display: flex; flex-direction: column; width: 100%; padding: 6px 8px; border: 0; border-radius: 6px; background: none; color: inherit;
  font: inherit; text-align: left; cursor: pointer; }
.lp-item:hover, .lp-item:focus-visible { background: var(--control-transparent-bgColor-hover, var(--bgColor-neutral-muted, #818b981f)); outline: none; }
.lp-item[aria-disabled="true"] { cursor: default; opacity: .6; }
.lp-item[aria-disabled="true"]:hover { background: none; }
.lp-item small, .lp-note, .lp-head { color: var(--fgColor-muted, var(--color-fg-muted, #59636e)); font-weight: 400; font-size: 12px; }
.lp-note { padding: 4px 8px 6px; }
.lp-sep { height: 1px; margin: 4px 0; background: var(--borderColor-muted, var(--color-border-muted, #d1d9e0b3)); }
`;

type Props = { class?: string; text?: string; title?: string; attrs?: Record<string, string>; on?: Partial<Record<keyof HTMLElementEventMap, (e: Event) => void>> };

/** An element built from text and attributes only (no HTML parsing). */
export function h<K extends keyof HTMLElementTagNameMap>(tag: K, props: Props = {}, ...children: (Node | string | null | false | undefined)[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (props.class) el.className = props.class;
  if (props.text !== undefined) el.textContent = props.text;
  if (props.title) el.title = props.title;
  for (const [k, v] of Object.entries(props.attrs ?? {})) el.setAttribute(k, v);
  for (const [k, fn] of Object.entries(props.on ?? {})) if (fn) el.addEventListener(k, fn);
  for (const c of children) if (c !== null && c !== false && c !== undefined) el.append(c);
  return el;
}

/** The 14 px Launchpad logo. */
export function logo() {
  return h("img", { class: "lp-logo", attrs: { src: logoUrl, alt: "", width: "14", height: "14" } });
}

/** The ▾ of a chip that opens a menu. */
export function caret() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("class", "lp-caret");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "M4 6l4 4 4-4");
  path.setAttribute("fill", "none");
  path.setAttribute("stroke", "currentColor");
  path.setAttribute("stroke-width", "1.8");
  svg.append(path);
  return svg;
}

/** GitHub shows a dark theme: Primer's variables follow on their own; the teal ink needs to know. */
function dark() {
  if (!document.body) return false;
  const bg = getComputedStyle(document.body).backgroundColor.match(/\d+(\.\d+)?/g)?.map(Number);
  return !!bg && bg.length >= 3 && 0.2126 * bg[0] + 0.7152 * bg[1] + 0.0722 * bg[2] < 128;
}

export type Widget = { host: HTMLElement; root: ShadowRoot; fresh: boolean };
const HOST_ATTR = "data-launchpad";
/** Closed shadow roots and what each host shows: kept here, so the page's scripts read neither. */
const roots = new WeakMap<HTMLElement, ShadowRoot>();
const keys = new WeakMap<HTMLElement, string>();

/** The host of widget `id` next to `anchor`, created or moved when needed. `key` names what it shows (anchor and
 * item, plus anything that changes its content): the same key on a host still in place means nothing to do
 * (`fresh` false), so a caller re-renders only on a change. */
export function mount(id: string, key: string, anchor: Element, place: InsertPosition): Widget {
  let host = document.querySelector<HTMLElement>(`[${HOST_ATTR}="${CSS.escape(id)}"]`);
  const inPlace = (el: HTMLElement) => {
    switch (place) {
      case "afterend": return anchor.nextElementSibling === el;
      case "beforebegin": return anchor.previousElementSibling === el;
      case "beforeend": return el.parentElement === anchor;
      case "afterbegin": return anchor.firstElementChild === el;
    }
  };
  if (host && !roots.has(host)) { host.remove(); host = null; }
  if (host && keys.get(host) === key && inPlace(host)) return { host, root: roots.get(host)!, fresh: false };
  if (!host) {
    host = document.createElement("launchpad-mark");
    host.setAttribute(HOST_ATTR, id);
    const shadow = host.attachShadow({ mode: "closed" });
    shadow.append(h("style", { text: STYLE }));
    roots.set(host, shadow);
  }
  if (!inPlace(host)) anchor.insertAdjacentElement(place, host);
  host.toggleAttribute("data-dark", dark());
  keys.set(host, key);
  const root = roots.get(host)!;
  for (const child of Array.from(root.childNodes)) if (!(child instanceof HTMLStyleElement)) child.remove();
  return { host, root, fresh: true };
}

/** Removes widget `id` from the page. */
export function unmount(id: string) {
  document.querySelectorAll(`[${HOST_ATTR}="${CSS.escape(id)}"]`).forEach((el) => el.remove());
}

/** Removes every widget (signed out, or a page without the layer). */
export function unmountAll() {
  document.querySelectorAll(`[${HOST_ATTR}]`).forEach((el) => el.remove());
}

export type MenuEntry =
  | { label: string; hint?: string; disabled?: boolean; onSelect?: () => void }
  | { note: string }
  | { separator: true };

/** A menu under `button` (in the same shadow root), closed by a choice, Escape, a click elsewhere or scrolling. */
const open = new WeakMap<ShadowRoot, () => void>();

export function openMenu(root: ShadowRoot, button: HTMLElement, title: string, entries: MenuEntry[]) {
  open.get(root)?.();
  const close = () => {
    open.delete(root);
    menu.remove();
    button.setAttribute("aria-expanded", "false");
    document.removeEventListener("pointerdown", outside, true);
    window.removeEventListener("scroll", close, true);
    window.removeEventListener("resize", close);
    document.removeEventListener("keydown", key, true);
  };
  const outside = (e: Event) => { if (!e.composedPath().includes(root.host)) close(); };
  const key = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); close(); button.focus(); } };
  const items = entries.map((e) => {
    if ("separator" in e) return h("div", { class: "lp-sep", attrs: { role: "separator" } });
    if ("note" in e) return h("div", { class: "lp-note", text: e.note });
    return h("button", {
      class: "lp-item",
      attrs: { role: "menuitem", type: "button", ...(e.disabled ? { "aria-disabled": "true" } : {}) },
      on: { click: () => { if (e.disabled) return; close(); e.onSelect?.(); } },
    }, h("span", { text: e.label }), e.hint ? h("small", { text: e.hint }) : null);
  });
  const menu = h("div", { class: "lp-menu", attrs: { role: "menu", "aria-label": title } }, h("div", { class: "lp-head" }, logo(), h("span", { text: "Launchpad" })), ...items);
  const r = button.getBoundingClientRect();
  menu.style.top = `${Math.round(r.bottom + 4)}px`;
  menu.style.left = `${Math.round(Math.max(8, Math.min(r.left, window.innerWidth - 348)))}px`;
  root.append(menu);
  open.set(root, close);
  button.setAttribute("aria-expanded", "true");
  document.addEventListener("pointerdown", outside, true);
  window.addEventListener("scroll", close, true);
  window.addEventListener("resize", close);
  document.addEventListener("keydown", key, true);
  menu.querySelector<HTMLElement>('.lp-item:not([aria-disabled="true"])')?.focus();
}

/** The chip of a place: logo, text and ▾, opening its menu. `tone`: accent (in Launchpad) or quiet (not). */
export function chip(root: ShadowRoot, text: string, tone: "accent" | "quiet", menu: () => { title: string; entries: MenuEntry[] }) {
  const button = h("button", {
    class: `lp-chip lp-${tone}`,
    title: text,
    attrs: { type: "button", "aria-haspopup": "menu", "aria-expanded": "false" },
    on: { click: (e) => {
      e.preventDefault();
      e.stopPropagation();
      const close = open.get(root);
      if (close) return close();
      const m = menu();
      openMenu(root, button, m.title, m.entries);
    } },
  }, logo(), h("span", { class: "lp-text", text }), caret());
  root.append(button);
  return button;
}
