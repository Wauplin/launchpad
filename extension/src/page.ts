/**
 * A snapshot of the page for the page chat: readable text (main content, the whole page or the selection), URL and
 * title. Runs in Chrome's isolated world in the top frame (`chrome.scripting.executeScript`), with no page scripts or
 * network calls; secret fields and hidden content are left out.
 */
export function readPage(args: { mode?: "main" | "full" | "selection"; offset?: number; limit?: number } = {}) {
  const page = { url: location.href, title: document.title.slice(0, 1000) };
  const excluded = "script,style,noscript,template,input[type=password],[autocomplete~=one-time-code],[autocomplete~=current-password],[autocomplete~=new-password],[data-launchpad-private]";
  const visible = (el: Element) => !el.closest(excluded) && !el.closest("[hidden],[aria-hidden=true]") && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
  const clean = (s: string) => s.replace(/\s+/g, " ").trim();
  let text = "";
  let truncated = false;
  let visited = 0;
  const limit = 200_000;
  if (args.mode === "selection") {
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
