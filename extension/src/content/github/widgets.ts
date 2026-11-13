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
[hidden] { display: none !important; }
:host([data-block="strip"]) { display: block; max-width: 1280px; margin: 12px auto 0; padding: 0 32px; font-weight: 400; font-size: 13px; }
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
:host([data-block]) { display: block; margin: 0 0 16px; font-weight: 400; font-size: 13px; }
:host([data-block="card"]) { margin: 8px 0 16px; }
:host([data-block="banner"]) { margin: 0 0 8px; }
.lp-box, .lp-card, .lp-banner { display: block; border: 1px solid var(--borderColor-default, var(--color-border-default, #d1d9e0)); border-radius: 6px;
  background: var(--bgColor-default, var(--color-canvas-default, #ffffff)); padding: 10px 12px; }
.lp-box { border-top: 2px solid var(--lp-teal); }
.lp-card, .lp-banner { border-left: 3px solid var(--lp-teal); }
.lp-reserve { min-height: 92px; }
.lp-row { display: flex; align-items: center; gap: 6px; min-width: 0; }
.lp-row + .lp-row, .lp-sect + .lp-sect, .lp-row + .lp-sect, .lp-sect + .lp-row { margin-top: 6px; }
.lp-title { font-weight: 600; color: var(--lp-ink); }
.lp-muted { color: var(--fgColor-muted, var(--color-fg-muted, #59636e)); font-size: 12px; }
.lp-grow { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.lp-label { font-size: 12px; font-weight: 600; color: var(--fgColor-muted, #59636e); margin: 8px 0 2px; }
.lp-dot { width: 8px; height: 8px; border-radius: 50%; flex: none; background: var(--fgColor-muted, #59636e); }
.lp-dot.lp-run { background: var(--fgColor-accent, #0969da); animation: lp-pulse 1.4s ease-in-out infinite; }
.lp-dot.lp-need { background: var(--fgColor-attention, #9a6700); }
.lp-dot.lp-ok { background: var(--fgColor-success, #1a7f37); }
.lp-dot.lp-bad { background: var(--fgColor-danger, #d1242f); }
@keyframes lp-pulse { 50% { opacity: .35; } }
.lp-btn { display: inline-flex; align-items: center; justify-content: center; gap: 4px; height: 28px; padding: 0 12px; border-radius: 6px; font: inherit; font-size: 13px; font-weight: 500;
  border: 1px solid var(--button-default-borderColor-rest, var(--borderColor-default, #d1d9e0)); background: var(--button-default-bgColor-rest, var(--bgColor-muted, #f6f8fa));
  color: var(--button-default-fgColor-rest, var(--fgColor-default, #1f2328)); cursor: pointer; white-space: nowrap; }
.lp-btn:hover { background: var(--button-default-bgColor-hover, var(--bgColor-neutral-muted, #eff2f5)); }
.lp-btn:disabled { opacity: .55; cursor: default; }
.lp-btn.lp-primary { background: var(--lp-teal); border-color: var(--lp-teal); color: #ffffff; }
.lp-btn.lp-primary:hover { filter: brightness(1.08); }
.lp-btn.lp-small { height: 24px; padding: 0 8px; font-size: 12px; }
.lp-btn.lp-full { width: 100%; }
.lp-btn:focus-visible, .lp-more:focus-visible { outline: 2px solid var(--focus-outlineColor, var(--lp-teal)); outline-offset: 1px; }
.lp-more { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; flex: none; border-radius: 6px; border: 1px solid transparent; background: none;
  color: var(--fgColor-muted, #59636e); font: inherit; font-size: 16px; line-height: 1; cursor: pointer; }
.lp-more:hover { background: var(--control-transparent-bgColor-hover, var(--bgColor-neutral-muted, #818b981f)); }
.lp-actions { display: flex; align-items: center; gap: 6px; margin-top: 8px; }
.lp-preview { margin-top: 6px; max-height: 7.5em; overflow: hidden; white-space: pre-wrap; word-break: break-word; font-size: 13px; color: var(--fgColor-default, #1f2328);
  -webkit-mask-image: linear-gradient(180deg, #000 70%, transparent); mask-image: linear-gradient(180deg, #000 70%, transparent); }
.lp-error { color: var(--fgColor-danger, #d1242f); font-size: 12px; margin-top: 6px; }
.lp-warn { color: var(--fgColor-attention, #9a6700); font-size: 12px; }
.lp-ghost { display: block; border: 1px dashed var(--lp-teal); border-radius: 6px; padding: 8px 10px; background: color-mix(in srgb, var(--lp-teal) 6%, var(--bgColor-default, #ffffff)); }
.lp-answer { margin-top: 6px; max-height: 16em; overflow: auto; white-space: pre-wrap; word-break: break-word; font-size: 13px; color: var(--fgColor-default, #1f2328); }
.lp-mention { color: var(--fgColor-attention, #9a6700); border-color: color-mix(in srgb, var(--fgColor-attention, #9a6700) 50%, transparent) !important; }
.lp-link { display: inline; padding: 0; border: 0; background: none; font: inherit; color: var(--fgColor-default, #1f2328); cursor: pointer; text-decoration: none; white-space: nowrap; }
.lp-link:hover { color: var(--fgColor-accent, #0969da); text-decoration: underline; }
.lp-strip > .lp-row { margin-top: 0 !important; }
.lp-strip { display: flex; align-items: center; flex-wrap: wrap; gap: 6px 16px; padding: 8px 12px; }
.lp-count { display: inline-flex; align-items: center; height: 20px; padding: 0 6px; border-radius: 999px; font-size: 12px; font-weight: 500; text-decoration: none; white-space: nowrap;
  color: var(--lp-ink); border: 1px solid color-mix(in srgb, var(--lp-teal) 55%, transparent); }
.lp-tag { display: inline-flex; align-items: center; gap: 4px; height: 20px; max-width: 260px; padding: 0 7px; border-radius: 999px; font-size: 12px; font-weight: 500; white-space: nowrap;
  border: 1px solid var(--borderColor-default, #d1d9e0); background: none; color: var(--fgColor-muted, #59636e); font-family: inherit; cursor: pointer; }
.lp-tag > span { overflow: hidden; text-overflow: ellipsis; }
.lp-tag.lp-need { color: var(--fgColor-attention, #9a6700); border-color: color-mix(in srgb, var(--fgColor-attention, #9a6700) 50%, transparent); }
.lp-tag.lp-run { color: var(--fgColor-accent, #0969da); border-color: color-mix(in srgb, var(--fgColor-accent, #0969da) 50%, transparent); }
.lp-tag.lp-ok { color: var(--fgColor-success, #1a7f37); border-color: color-mix(in srgb, var(--fgColor-success, #1a7f37) 50%, transparent); }
.lp-tag:hover { background: var(--control-transparent-bgColor-hover, #818b981f); }
.lp-hover { display: inline-flex; align-items: center; gap: 2px; }
:host([data-hover]:not([data-show])) .lp-hover { visibility: hidden; }
.lp-section { border: 1px solid var(--borderColor-default, #d1d9e0); border-top: 2px solid var(--lp-teal); border-radius: 6px; margin: 0 0 16px; background: var(--bgColor-default, #ffffff); }
.lp-section-head { display: flex; align-items: center; gap: 6px; padding: 8px 12px; background: var(--bgColor-muted, #f6f8fa); border-bottom: 1px solid var(--borderColor-muted, #d1d9e0b3); border-radius: 6px 6px 0 0; }
.lp-section-row { display: flex; align-items: center; gap: 8px; padding: 8px 12px; }
.lp-section-row + .lp-section-row { border-top: 1px solid var(--borderColor-muted, #d1d9e0b3); }
.lp-pop label { display: flex; align-items: center; gap: 6px; font-size: 13px; }
.lp-pop input[type="text"] { width: 100%; font: inherit; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; color: inherit; border-radius: 6px; padding: 5px 8px;
  border: 1px solid var(--borderColor-default, #d1d9e0); background: var(--bgColor-default, #ffffff); }
.lp-confirm { padding: 8px; border-radius: 6px; background: var(--bgColor-attention-muted, #fff8c5); color: var(--fgColor-default, #1f2328); font-size: 12px; }
.lp-bar { position: fixed; z-index: 2147483000; left: 50%; bottom: 24px; transform: translateX(-50%); display: flex; align-items: center; gap: 10px; padding: 8px 10px 8px 12px;
  border-radius: 12px; background: var(--overlay-bgColor, var(--bgColor-default, #ffffff)); border: 1px solid var(--borderColor-default, #d1d9e0);
  box-shadow: var(--shadow-floating-large, 0 8px 24px rgba(140, 149, 159, .2)); font-size: 13px; }
.lp-pop { position: fixed; z-index: 2147483000; width: 340px; padding: 12px; border-radius: 12px; font-size: 13px; font-weight: 400;
  background: var(--overlay-bgColor, var(--bgColor-default, #ffffff)); border: 1px solid var(--borderColor-default, #d1d9e0);
  box-shadow: var(--shadow-floating-large, 0 8px 24px rgba(140, 149, 159, .2)); display: grid; gap: 8px; }
.lp-pop select, .lp-pop textarea { width: 100%; font: inherit; font-size: 13px; color: inherit; border-radius: 6px; padding: 5px 8px;
  border: 1px solid var(--borderColor-default, #d1d9e0); background: var(--bgColor-default, #ffffff); }
.lp-pop textarea { min-height: 64px; resize: vertical; }
.lp-pop select:focus-visible, .lp-pop textarea:focus-visible { outline: 2px solid var(--lp-teal); outline-offset: -1px; }
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
  // The first opaque background of the body or the document (a transparent one says nothing).
  for (const el of [document.body, document.documentElement]) {
    if (!el) continue;
    const bg = getComputedStyle(el).backgroundColor.match(/\d+(\.\d+)?/g)?.map(Number);
    if (!bg || bg.length < 3 || (bg.length >= 4 && bg[3] === 0)) continue;
    return 0.2126 * bg[0] + 0.7152 * bg[1] + 0.0722 * bg[2] < 128;
  }
  return false;
}

export type Widget = { host: HTMLElement; root: ShadowRoot; fresh: boolean };
const HOST_ATTR = "data-launchpad";
/** Closed shadow roots and what each host shows: kept here, so the page's scripts read neither. */
const roots = new WeakMap<HTMLElement, ShadowRoot>();
const keys = new WeakMap<HTMLElement, string>();

/** The host of widget `id` next to `anchor`, created or moved when needed. `key` names what it shows (anchor and
 * item, plus anything that changes its content): the same key on a host still in place means nothing to do
 * (`fresh` false), so a caller re-renders only on a change. */
export function mount(id: string, key: string, anchor: Element, place: InsertPosition, block?: "box" | "card" | "banner" | "strip"): Widget {
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
  if (block) host.setAttribute("data-block", block); else host.removeAttribute("data-block");
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

/** The ⋯ of a place: its other actions in a menu. */
export function more(root: ShadowRoot, title: string, entries: () => MenuEntry[]) {
  const button = h("button", {
    class: "lp-more",
    title: "More",
    attrs: { type: "button", "aria-label": "More", "aria-haspopup": "menu", "aria-expanded": "false" },
    on: { click: (e) => {
      e.preventDefault();
      e.stopPropagation();
      const close = open.get(root);
      if (close) return close();
      openMenu(root, button, title, entries());
    } },
  }, kebab());
  return button;
}

/** ⋯ as an icon (fonts may lack the character). */
function kebab() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("width", "16");
  svg.setAttribute("height", "16");
  svg.setAttribute("aria-hidden", "true");
  for (const cx of ["3", "8", "13"]) {
    const c = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    c.setAttribute("cx", cx);
    c.setAttribute("cy", "8");
    c.setAttribute("r", "1.5");
    c.setAttribute("fill", "currentColor");
    svg.append(c);
  }
  return svg;
}

/** A button of a widget (never one of GitHub's). */
export function button(text: string, onClick: () => void, opts: { primary?: boolean; small?: boolean; full?: boolean; title?: string; disabled?: boolean } = {}) {
  const b = h("button", {
    class: `lp-btn${opts.primary ? " lp-primary" : ""}${opts.small ? " lp-small" : ""}${opts.full ? " lp-full" : ""}`,
    text,
    title: opts.title,
    attrs: { type: "button" },
    on: { click: (e) => { e.preventDefault(); e.stopPropagation(); onClick(); } },
  });
  b.disabled = !!opts.disabled;
  return b;
}

/** A popover in `root` under `anchor` (closed by Escape, a click elsewhere, or its own `close`). */
export function popover(root: ShadowRoot, anchor: HTMLElement, label: string, build: (close: () => void) => HTMLElement[]) {
  open.get(root)?.();
  const close = () => {
    open.delete(root);
    pop.remove();
    document.removeEventListener("pointerdown", outside, true);
    document.removeEventListener("keydown", key, true);
  };
  const outside = (e: Event) => { if (!e.composedPath().includes(root.host)) close(); };
  const key = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); close(); anchor.focus(); } };
  const pop = h("div", { class: "lp-pop", attrs: { role: "dialog", "aria-label": label } }, h("div", { class: "lp-head", attrs: { style: "padding:0" } }, logo(), h("span", { text: label })), ...build(close));
  const r = anchor.getBoundingClientRect();
  const below = r.bottom + 6 + 260 < window.innerHeight;
  pop.style.top = below ? `${Math.round(r.bottom + 6)}px` : `${Math.max(8, Math.round(r.top - 266))}px`;
  pop.style.left = `${Math.round(Math.max(8, Math.min(r.left, window.innerWidth - 356)))}px`;
  root.append(pop);
  open.set(root, close);
  document.addEventListener("pointerdown", outside, true);
  document.addEventListener("keydown", key, true);
  return close;
}
