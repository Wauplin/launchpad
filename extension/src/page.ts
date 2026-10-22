/** Runs in Chrome's isolated world in the top frame. No page scripts or network calls. Refs live only in this isolated document. */
export function readPage(expected?: { document: string; url: string }, tool = "browser_read", args: Record<string, unknown> = {}) {
  const state = globalThis as typeof globalThis & { __launchpadDocument?: string; __launchpadRefs?: Map<string, Element>; __launchpadSnapshot?: string };
  state.__launchpadDocument ??= crypto.randomUUID();
  const documentId = state.__launchpadDocument;
  // Full URL binding also stops same-document SPA navigation from changing the grant.
  const page = { document: documentId, url: location.href, title: document.title.slice(0, 1000) };
  if (expected && (documentId !== expected.document || location.href !== expected.url)) return { ...page, error: "This page changed. Add it again to grant access to the new document." };
  if (!expected) return page;
  const excluded = "script,style,noscript,template,input[type=password],[autocomplete~=one-time-code],[autocomplete~=current-password],[autocomplete~=new-password],[data-launchpad-private]";
  const visible = (el: Element) => !el.closest(excluded) && !el.closest("[hidden],[aria-hidden=true]") && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
  const clean = (s: string) => s.replace(/\s+/g, " ").trim();
  let text = "";
  let truncated = false;
  let visited = 0;
  const limit = 200_000;
  if (tool === "browser_outline") {
    const lines: string[] = [];
    state.__launchpadRefs = new Map();
    state.__launchpadSnapshot = crypto.randomUUID().slice(0, 8);
    const elements = document.querySelectorAll("h1,h2,h3,h4,h5,h6,a,button,input,textarea,select,[role],[aria-label]");
    for (const el of elements) {
      if (++visited > 10_000 || lines.length >= 600) { truncated = true; break; }
      if (!visible(el)) continue;
      const tag = el.tagName.toLowerCase();
      const input = el instanceof HTMLInputElement ? el : null;
      const name = clean(el.getAttribute("aria-label") || el.getAttribute("alt") || el.textContent?.slice(0, 1000) || input?.labels?.[0]?.textContent || input?.placeholder || "").slice(0, 180);
      const secret = !!el.closest(excluded) || el.matches("input,textarea,[role=textbox],[contenteditable]") && /password|one.?time|otp|verification|security.?code/i.test(`${el.getAttribute("type")} ${el.getAttribute("name")} ${el.getAttribute("autocomplete")} ${el.id} ${name}`);
      if (secret) continue;
      const role = el.getAttribute("role") || ({ a: "link", button: "button", input: input?.type || "input", textarea: "textbox", select: "combobox" } as Record<string, string>)[tag] || tag;
      const value = input && !["hidden", "file"].includes(input.type) ? input.value : el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement ? el.value : null;
      const ref = `${state.__launchpadSnapshot}:e${lines.length + 1}`;
      state.__launchpadRefs.set(ref, el);
      lines.push(`${ref} ${role} ${JSON.stringify(name)}${value != null ? ` value=${JSON.stringify(value.slice(0, 300))}` : ""}`);
    }
    text = lines.join("\n");
  } else if (args.mode === "selection") {
    const selection = window.getSelection();
    // A selection crossing any secret field/container is omitted as a whole.
    const safe = !!selection?.rangeCount && !Array.from(document.querySelectorAll(excluded)).some((el) => selection.getRangeAt(0).intersectsNode(el));
    text = safe ? (selection?.toString() ?? "").slice(0, limit) : "";
  } else {
    const root = args.mode === "full" ? document.body : document.querySelector("article,main,[role=main]") ?? document.body;
    let extracted = 0;
    const walk = (node: Node, depth = 0): string => {
      if (++visited > 10_000 || extracted >= limit || depth > 80) { truncated = true; return ""; }
      if (node.nodeType === Node.TEXT_NODE) { const t = clean((node.textContent ?? "").slice(0, limit - extracted)); extracted += t.length; return t; }
      if (!(node instanceof Element) || !visible(node) || node.matches("input,textarea,select,iframe")) return "";
      const parts: string[] = [];
      for (const child of node.childNodes) {
        if (visited >= 10_000 || extracted >= limit) { truncated = true; break; }
        const part = walk(child, depth + 1);
        if (part) parts.push(part);
      }
      const content = parts.join(" ");
      if (/^H[1-6]$/.test(node.tagName)) return `\n\n${"#".repeat(Number(node.tagName[1]))} ${content}\n\n`;
      if (node.tagName === "A") {
        const href = (node as HTMLAnchorElement).href.slice(0, 8192);
        extracted += href.length;
        return /^https?:/.test(href) ? `[${content}](${href})` : content;
      }
      if (node.tagName === "LI") return `\n- ${content}`;
      if (node.tagName === "PRE") return `\n\n\`\`\`\n${content}\n\`\`\`\n\n`;
      return /^(P|DIV|SECTION|ARTICLE|MAIN|BR|UL|OL|TR)$/.test(node.tagName) ? `\n${content}\n` : content;
    };
    text = root ? walk(root) : "";
    if (text.length > limit) { text = text.slice(0, limit); truncated = true; }
  }
  const offset = Math.max(0, Math.min(Number(args.offset) || 0, text.length));
  const size = Math.max(1, Math.min(Number(args.limit) || 12000, 16000));
  const end = Math.min(offset + size, text.length);
  return { ...page, text: text.slice(offset, end), offset, next_offset: end < text.length ? end : null, truncated, note: "Top frame only. Cross-origin iframes, closed shadow roots, hidden content and secret fields are omitted." };
}
