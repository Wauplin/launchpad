// The folio renderer: one page of a folio, split by the shared line-based splitter (lib/folioSplit.ts) into Markdown,
// `:::` containers and typed blocks (fenced YAML, the catalog of gen/folioCatalog.ts). Markdown renders with
// markdown-it with raw HTML off, then DOMPurify; images load only from the folio (sandboxed raw route), never the
// network. A typed block with an error of Home's check becomes a problem box with its raw text, so the rest of the
// page still renders. Blocks that take marks (roadmap and findings items, decisions, options, checklists) show the
// owner's verdicts, choices and ticks, saved at once through `MarkApi`.

import MarkdownIt from "markdown-it";
import DOMPurify from "dompurify";
import { parse as parseYaml } from "yaml";
import { createContext, memo, useContext, useMemo, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import type { FolioMark } from "@/gen/FolioMark";
import type { FolioPage } from "@/gen/FolioPage";
import type { FolioProblem } from "@/gen/FolioProblem";
import { FOLIO_CATALOG } from "@/gen/folioCatalog";
import { Icon } from "@/components/Icon";
import { copyText } from "@/lib/clipboard";
import { toast } from "@/components/Toast";
import { daemonUrl } from "@/lib/connection";
import { attrs as parseAttrs, lines as splitLines, outline as splitOutline, type OutlineItem } from "@/lib/folioSplit";
import { folioAsset, folioPath, txt, VERDICTS } from "@/lib/folios";

/** What the page needs from the reader: where it is, the owner's marks and how to change them. */
export type MarkApi = {
  ws: string;
  slug: string;
  rev: number | null;
  page: string;
  marks: FolioMark[];
  problems: FolioProblem[];
  /** An older rev or a shared view: marks show but do not change. */
  readOnly: boolean;
  setMark: (kind: "verdict" | "choice" | "check", target: string, value: string | null) => void;
  /** Opens the comment box on a block, an item or a section (with a quote from a selection). */
  comment: (target: string, quote?: { quote: string; prefix: string; suffix: string }) => void;
  /** Opens another page (and an anchor on it). */
  open: (page: string, anchor?: string) => void;
};

const Marks = createContext<MarkApi | null>(null);
const useMarks = () => useContext(Marks)!;

const BLOCKS = new Set<string>(FOLIO_CATALOG.blocks.map((b) => b.name));
export const isTyped = (lang: string) => BLOCKS.has(lang) || lang.startsWith("x-");
const ALERTS = new Set<string>(FOLIO_CATALOG.alerts);

// ---------------------------------------------------------------- Markdown

type Env = { offset: number; headings: Map<number, string>; ws: string; slug: string; page: string; rev: number | null };

const md = new MarkdownIt({ html: false, linkify: true, breaks: false });
md.renderer.rules.heading_open = (tokens, idx, options, e, self) => {
  const env = e as Env;
  const t = tokens[idx];
  const id = t.map ? env.headings.get(env.offset + t.map[0]) : undefined;
  if (id) { t.attrSet("id", `f-${id}`); t.attrSet("data-anchor", id); t.attrJoin("class", "folio-h"); }
  return self.renderToken(tokens, idx, options);
};
md.renderer.rules.heading_close = (tokens, idx, options, _env, self) => {
  const open = tokens[idx - 2];
  const id = open?.attrGet("data-anchor") as string | null | undefined;
  const button = id ? `<button type="button" class="folio-hc" data-comment="${md.utils.escapeHtml(id)}" title="Comment on this section" aria-label="Comment on this section">Comment</button>` : "";
  return button + self.renderToken(tokens, idx, options);
};
md.renderer.rules.link_open = (tokens, idx, options, _env, self) => {
  const t = tokens[idx];
  const href = String(t.attrGet("href") ?? "");
  if (/^https?:\/\//.test(href) || href.startsWith("mailto:")) { t.attrSet("target", "_blank"); t.attrSet("rel", "noreferrer noopener"); }
  else { t.attrSet("data-folio-link", href); t.attrSet("href", "#"); }
  return self.renderToken(tokens, idx, options);
};
md.renderer.rules.image = (tokens, idx, _options, e) => {
  const env = e as Env;
  const t = tokens[idx];
  const src = String(t.attrGet("src") ?? "");
  const alt = md.utils.escapeHtml(t.content);
  if (/^(https?:)?\/\//.test(src)) return `<span class="folio-blocked" title="Images from the network do not load: the agent adds them with launchpad folio add-media">Image from the network: ${alt || md.utils.escapeHtml(src)}</span>`;
  const path = resolve(env.page, src.split("#")[0]);
  return `<img src="${md.utils.escapeHtml(folioAsset(env.ws, env.slug, path, env.rev))}" alt="${alt}" loading="lazy">`;
};
// An explicit anchor (`## Waves {#waves}`) is the heading's id, not its text.
md.core.ruler.push("folio_heading_ids", (state) => {
  const ts = state.tokens;
  for (let i = 1; i < ts.length; i++) {
    if (ts[i].type !== "inline" || ts[i - 1].type !== "heading_open") continue;
    const strip = (x: string) => x.replace(/\s*\{#[A-Za-z0-9_.:-]+\}\s*$/, "");
    ts[i].content = strip(ts[i].content);
    const last = ts[i].children?.[ts[i].children!.length - 1];
    if (last?.type === "text") last.content = strip(last.content);
  }
});
// GitHub alerts (`> [!NOTE]`) become callouts.
md.core.ruler.push("folio_alerts", (state) => {
  const ts = state.tokens;
  for (let i = 0; i < ts.length - 2; i++) {
    if (ts[i].type !== "blockquote_open" || ts[i + 1].type !== "paragraph_open" || ts[i + 2].type !== "inline") continue;
    const m = /^\[!([A-Za-z]+)\]\s*/.exec(ts[i + 2].content);
    if (!m) continue;
    const kind = m[1].toUpperCase();
    const tone = kind === "WARNING" || kind === "CAUTION" ? "warn" : kind === "IMPORTANT" ? "verdict" : kind === "TIP" ? "tip" : ALERTS.has(kind) ? "note" : "info";
    ts[i].attrJoin("class", `folio-callout tone-${tone}`);
    ts[i + 2].content = ts[i + 2].content.slice(m[0].length);
    const first = ts[i + 2].children?.[0];
    if (first?.type === "text") first.content = first.content.replace(/^\[![A-Za-z]+\]\s*/, "");
    if (ts[i + 2].children?.[1]?.type === "softbreak" && !first?.content) ts[i + 2].children!.splice(0, 2);
  }
});

const PURIFY = { ADD_ATTR: ["target", "data-folio-link", "data-comment", "data-anchor"] };
const html = (text: string, env: Env) => DOMPurify.sanitize(md.render(text, env), PURIFY);
const inline = (text: string, env: Env) => DOMPurify.sanitize(md.renderInline(text, env), PURIFY);

/** A relative path from a page (`../x` from `a/b.md` → `x`). */
export function resolve(page: string, target: string): string {
  const parts = page.includes("/") ? page.split("/").slice(0, -1) : [];
  for (const seg of target.replace(/^\.\//, "").split("/")) {
    if (seg === "..") parts.pop();
    else if (seg && seg !== ".") parts.push(seg);
  }
  return parts.join("/");
}

const EnvCtx = createContext<Env | null>(null);

function Md({ text, offset }: { text: string; offset: number }) {
  const env = useContext(EnvCtx)!;
  const out = useMemo(() => html(text, { ...env, offset }), [text, offset, env]);
  if (!text.trim()) return null;
  return <div className="folio-md" dangerouslySetInnerHTML={{ __html: out }} />;
}

/** Inline Markdown of a YAML field (titles, texts). */
function Inline({ text, block = false }: { text: unknown; block?: boolean }) {
  const env = useContext(EnvCtx)!;
  const s = txt(text);
  const out = useMemo(() => (block ? html(s, { ...env, offset: -1e9 }) : inline(s, { ...env, offset: -1e9 })), [s, block, env]);
  if (!s) return null;
  return block ? <div className="folio-md" dangerouslySetInnerHTML={{ __html: out }} /> : <span dangerouslySetInnerHTML={{ __html: out }} />;
}

// ---------------------------------------------------------------- the page tree

type Node =
  | { k: "md"; from: number; to: number }
  | { k: "container"; item: Extract<OutlineItem, { t: "container" }>; children: Node[] }
  | { k: "block"; item: Extract<OutlineItem, { t: "fence" }> };

/** Pure: the nodes of lines `from..to` (1-based, inclusive) at container depth `depth`. */
export function buildTree(items: OutlineItem[], from: number, to: number, depth: number): Node[] {
  const out: Node[] = [];
  let cursor = from;
  for (const it of items) {
    if (it.line < cursor || it.line > to || it.t === "heading" || it.depth !== depth) continue;
    if (it.t === "fence" && !isTyped(it.lang)) continue;
    if (it.line > cursor) out.push({ k: "md", from: cursor, to: it.line - 1 });
    if (it.t === "container") {
      out.push({ k: "container", item: it, children: buildTree(items, it.line + 1, it.closed ? it.end - 1 : it.end, depth + 1) });
      cursor = (it.closed ? it.end : it.end) + 1;
    } else {
      out.push({ k: "block", item: it });
      cursor = it.end + 1;
    }
  }
  if (cursor <= to) out.push({ k: "md", from: cursor, to });
  return out;
}

/** One page of a folio. */
export const FolioPageView = memo(function FolioPageView({ page, api: marks, mood }: { page: FolioPage; api: MarkApi; mood: "report" | "editorial" }) {
  const ls = useMemo(() => splitLines(page.text), [page.text]);
  const o = useMemo(() => splitOutline(page.text), [page.text]);
  const env = useMemo<Env>(() => ({ offset: 0, headings: new Map(o.items.flatMap((i) => (i.t === "heading" ? [[i.line - 1, i.id] as [number, string]] : []))), ws: marks.ws, slug: marks.slug, page: page.path, rev: marks.rev }), [o, marks.ws, marks.slug, page.path, marks.rev]);
  const tree = useMemo(() => buildTree(o.items, (o.front?.end ?? 0) + 1, ls.length, 0), [o, ls.length]);
  return <Marks.Provider value={marks}>
    <EnvCtx.Provider value={env}>
      <article className={`folio folio-${mood}`} onClick={(e) => click(e, marks)}>
        <Nodes nodes={tree} ls={ls} items={o.items} />
      </article>
    </EnvCtx.Provider>
  </Marks.Provider>;
});

/** Links inside Markdown: other pages and anchors stay in the reader; section Comment buttons open the comment box. */
function click(e: React.MouseEvent, marks: MarkApi) {
  const el = e.target as HTMLElement;
  const c = el.closest("[data-comment]");
  if (c) { e.preventDefault(); marks.comment(c.getAttribute("data-comment")!); return; }
  const a = el.closest("a[data-folio-link]");
  if (!a) return;
  e.preventDefault();
  const href = a.getAttribute("data-folio-link") ?? "";
  if (href.startsWith("folio:")) { location.assign(`/folios/${encodeURIComponent(href.slice(6).split("#")[0])}`); return; }
  const [file, anchor] = href.split("#");
  marks.open(file ? resolve(marks.page, file) : marks.page, anchor);
}

function Nodes({ nodes, ls, items }: { nodes: Node[]; ls: string[]; items: OutlineItem[] }) {
  return <>{nodes.map((n) => <NodeView key={n.k === "md" ? `m${n.from}` : `${n.k}${n.item.line}`} node={n} ls={ls} items={items} />)}</>;
}

function NodeView({ node, ls, items }: { node: Node; ls: string[]; items: OutlineItem[] }) {
  if (node.k === "md") return <Md text={ls.slice(node.from - 1, node.to).join("\n")} offset={node.from - 1} />;
  if (node.k === "block") return <BlockView item={node.item} ls={ls} />;
  return <ContainerView node={node} ls={ls} items={items} />;
}

/** Splits a container's lines at its `###` headings (cards, columns, tabs, scope): a part per heading. */
function parts(node: Extract<Node, { k: "container" }>, items: OutlineItem[]): { title: string; id: string; from: number; to: number }[] {
  const it = node.item;
  const last = it.closed ? it.end - 1 : it.end;
  const nested = items.filter((x) => x.t === "container" && x.depth > it.depth && x.line > it.line && x.line <= last);
  const heads = items.filter((x): x is Extract<OutlineItem, { t: "heading" }> => x.t === "heading" && x.level === 3 && x.line > it.line && x.line <= last && !nested.some((c) => c.t === "container" && x.line > c.line && x.line < c.end));
  return heads.map((h, i) => ({ title: h.text, id: h.id, from: h.line + 1, to: (heads[i + 1]?.line ?? last + 1) - 1 }));
}

function ContainerView({ node, ls, items }: { node: Extract<Node, { k: "container" }>; ls: string[]; items: OutlineItem[] }) {
  const it = node.item;
  const a = parseAttrs(it.attrs);
  const target = a.id ?? `${it.name}-${it.line}`;
  const sub = (from: number, to: number) => <Nodes nodes={buildTree(items, from, to, it.depth + 1)} ls={ls} items={items} />;
  const split = parts(node, items);
  const comments = <CommentButton target={target} />;
  switch (it.name) {
    case "callout": {
      const tone = a.pairs.tone ?? "info";
      return <aside id={`f-${target}`} className={`folio-callout tone-${tone}`} data-folio-target={target}>
        <div className="folio-callout-head">{it.title || a.pairs.title || TONE_TITLE[tone] || "Note"}{comments}</div>
        <Nodes nodes={node.children} ls={ls} items={items} />
      </aside>;
    }
    case "cards":
    case "columns":
      return <div id={a.id ? `f-${a.id}` : undefined} className={it.name === "cards" ? "folio-cards" : "folio-columns"}>
        {split.length ? split.map((p) => <section key={p.id} className={it.name === "cards" ? "folio-card" : ""} data-folio-target={p.id}><h3 id={`f-${p.id}`}>{p.title}</h3>{sub(p.from, p.to)}</section>) : <Nodes nodes={node.children} ls={ls} items={items} />}
      </div>;
    case "tabs":
      return <Tabs parts={split} render={(p) => sub(p.from, p.to)} fallback={<Nodes nodes={node.children} ls={ls} items={items} />} />;
    case "details":
      return <details className="folio-details" open={a.flags.includes("open")} data-folio-target={target}><summary>{it.title || "Details"}</summary><Nodes nodes={node.children} ls={ls} items={items} /></details>;
    case "scope":
      return <div id={`f-${target}`} className="folio-scope" data-folio-target={target}>
        {split.length ? split.map((p) => <section key={p.id} className={`folio-scope-box ${/^out\b/i.test(p.title) ? "out" : "in"}`}><div className="folio-callout-head">{p.title}</div>{sub(p.from, p.to)}</section>) : <Nodes nodes={node.children} ls={ls} items={items} />}
        <div className="folio-scope-c">{comments}</div>
      </div>;
    case "hero":
      return <div className="folio-hero"><Nodes nodes={node.children} ls={ls} items={items} /></div>;
    default:
      return <div className="folio-box" title={`:::${it.name} is not a known container: it renders as a plain box`}><Nodes nodes={node.children} ls={ls} items={items} /></div>;
  }
}

const TONE_TITLE: Record<string, string> = { verdict: "Verdict", info: "Note", note: "Note", tip: "Tip", warn: "Warning", risk: "Risk" };

function Tabs<P extends { title: string; id: string }>({ parts: ps, render, fallback }: { parts: P[]; render: (p: P) => ReactNode; fallback: ReactNode }) {
  const [current, setCurrent] = useState(ps[0]?.id ?? "");
  if (!ps.length) return <>{fallback}</>;
  const shown = ps.find((p) => p.id === current) ?? ps[0];
  return <div className="folio-tabs">
    <div role="tablist" className="folio-tablist">{ps.map((p) => <button key={p.id} role="tab" type="button" aria-selected={p.id === shown.id} className={p.id === shown.id ? "on" : ""} onClick={() => setCurrent(p.id)}>{p.title}</button>)}</div>
    <div role="tabpanel">{render(shown)}</div>
  </div>;
}


// ---------------------------------------------------------------- typed blocks

/** The error problems of Home's check inside a block (they make it a problem box). */
function blockErrors(problems: FolioProblem[], page: string, from: number, to: number) {
  return problems.filter((p) => p.file === page && p.severity === "error" && p.line != null && p.line >= from && p.line <= to && ["F102", "F201", "F202", "F204"].includes(p.code));
}

function BlockView({ item, ls }: { item: Extract<OutlineItem, { t: "fence" }>; ls: string[] }) {
  const marks = useMarks();
  const body = ls.slice(item.line, item.closed ? item.end - 1 : item.end).join("\n");
  const parsed = useMemo(() => { try { return { value: parseYaml(body) as unknown, error: null }; } catch (e) { return { value: null, error: String((e as Error).message ?? e) }; } }, [body]);
  const errors = blockErrors(marks.problems, marks.page, item.line, item.end);
  if (errors.length || parsed.error || (parsed.value != null && typeof parsed.value !== "object")) {
    return <ProblemBox lang={item.lang} line={item.line} body={body} problems={errors} error={parsed.error} />;
  }
  const v = (parsed.value ?? {}) as Record<string, unknown>;
  const C = RENDER[item.lang];
  return <div className="folio-block" data-block={item.lang}>{C ? <C v={v} /> : <KeyValues name={item.lang} v={v} />}</div>;
}

function ProblemBox({ lang, line, body, problems, error }: { lang: string; line: number; body: string; problems: FolioProblem[]; error: string | null }) {
  const marks = useMarks();
  return <div className="folio-problem" role="note">
    <div className="folio-problem-head"><Icon name="alert" size={12} /> This {lang} block cannot render</div>
    {problems.map((p, i) => <div key={i} className="folio-problem-line">{p.code} · {p.message} · {marks.page}:{p.line}</div>)}
    {!problems.length && error && <div className="folio-problem-line">{error.split("\n")[0]} · {marks.page}:{line}</div>}
    <pre>{body}</pre>
  </div>;
}

type R = (p: { v: Record<string, unknown> }) => ReactNode;
const list = (v: unknown): Record<string, unknown>[] => (Array.isArray(v) ? v.filter((x): x is Record<string, unknown> => !!x && typeof x === "object") : []);
const id = (x: Record<string, unknown>) => txt(x.id);

function Verdict({ target }: { target: string }) {
  const marks = useMarks();
  if (!target) return null;
  const current = marks.marks.find((m) => m.kind === "verdict" && m.target === target && m.author.kind === "owner")?.value ?? null;
  return <span className="folio-verdict" role="group" aria-label={`Your verdict on ${target}`}>
    {VERDICTS.map((x) => <button key={x.value} type="button" disabled={marks.readOnly} className={current === x.value ? `on v-${x.value}` : ""} aria-pressed={current === x.value} title={current === x.value ? "Click again to clear" : `Mark ${target}: ${x.value === "wont" ? "won't do" : x.value}`} onClick={() => marks.setMark("verdict", target, current === x.value ? null : x.value)}>{x.label}</button>)}
  </span>;
}

function CommentButton({ target }: { target: string }) {
  const marks = useMarks();
  const n = marks.marks.filter((m) => m.kind === "comment" && m.target === target && m.resolved_at == null).length;
  return <button type="button" className="folio-cb" title={`Comment on ${target}`} aria-label={`Comment on ${target}`} disabled={marks.readOnly} onClick={() => marks.comment(target)}><Icon name="comment" size={12} />{n > 0 && <span>{n}</span>}</button>;
}

function ItemRow({ x, children, verdict = true }: { x: Record<string, unknown>; children: ReactNode; verdict?: boolean }) {
  const t = id(x);
  return <div className="folio-item" id={t ? `f-${t}` : undefined} data-folio-target={t || undefined}>
    {t && <span className="folio-id">{t}</span>}
    <div className="folio-item-body">{children}</div>
    {t && <span className="folio-item-marks">{verdict && <Verdict target={t} />}<CommentButton target={t} /></span>}
  </div>;
}

const Roadmap: R = ({ v }) => <div className="folio-roadmap">
  {list(v.waves).map((w, i) => <section key={id(w) || i} className="folio-wave" id={id(w) ? `f-${id(w)}` : undefined} data-folio-target={id(w) || undefined}>
    <div className="folio-wave-head"><span className="folio-id">{id(w)}</span><b><Inline text={w.title} /></b>{w.size != null && <span className="folio-pill">{txt(w.size)}</span>}{id(w) && <CommentButton target={id(w)} />}</div>
    {w.text != null && <Inline text={w.text} block />}
    {list(w.items).map((x, j) => <ItemRow key={id(x) || j} x={x}>
      <span className="folio-item-title"><Inline text={x.title} /></span>
      <span className="folio-item-meta">
        {x.size != null && <span className="folio-pill">{txt(x.size)}</span>}
        {x.status != null && <span className={`folio-pill s-${txt(x.status)}`}>{txt(x.status)}</span>}
        {Array.isArray(x.deps) && x.deps.length > 0 && <span title="Waits for">after {x.deps.map(txt).join(", ")}</span>}
        {x.advice != null && <span title="The author's advice">advice: {txt(x.advice)}</span>}
      </span>
      {x.text != null && <Inline text={x.text} block />}
    </ItemRow>)}
  </section>)}
</div>;

const SEVERITY: Record<string, string> = { critical: "s-risk", high: "s-risk", medium: "s-warn", low: "s-todo", info: "s-todo" };
const Findings: R = ({ v }) => <div className="folio-findings">
  {list(v.items).map((x, i) => <ItemRow key={id(x) || i} x={x}>
    <span className="folio-item-title">{x.severity != null && <span className={`folio-pill ${SEVERITY[txt(x.severity)] ?? ""}`}>{txt(x.severity)}</span>} <Inline text={x.title} /></span>
    <span className="folio-item-meta">{x.where != null && <code>{txt(x.where)}</code>}{x.advice != null && <span title="The author's advice">advice: {txt(x.advice)}</span>}</span>
    {x.text != null && <Inline text={x.text} block />}
    {x.fix != null && <div className="folio-fix"><b>Fix</b> <Inline text={x.fix} /></div>}
  </ItemRow>)}
</div>;

function Choose({ target, options, question, why }: { target: string; options: Record<string, unknown>[]; question?: unknown; why?: unknown }) {
  const marks = useMarks();
  const picked = marks.marks.find((m) => m.kind === "choice" && m.target === target && m.author.kind === "owner")?.value ?? null;
  return <div className="folio-decision" id={target ? `f-${target}` : undefined} data-folio-target={target || undefined}>
    <div className="folio-wave-head">{target && <span className="folio-id">{target}</span>}<b><Inline text={question} /></b>{target && <CommentButton target={target} />}</div>
    {why != null && <Inline text={why} block />}
    <div className="folio-options" role="radiogroup" aria-label={txt(question) || target}>
      {options.map((o, i) => {
        const oid = id(o);
        const on = picked != null && picked === oid;
        return <button key={oid || i} type="button" role="radio" aria-checked={on} disabled={marks.readOnly || !target || !oid} className={`folio-option ${on ? "on" : ""}`} title={on ? "Your pick: click again to clear" : "Pick this option"} onClick={() => marks.setMark("choice", target, on ? null : oid)}>
          <span className="folio-option-head"><b><Inline text={o.label} /></b>{o.recommended === true && <span className="folio-pill s-done">recommended</span>}{on && <span className="folio-pill s-pick">your pick</span>}</span>
          {o.text != null && <Inline text={o.text} block />}
          {Array.isArray(o.pros) && o.pros.length > 0 && <ul className="folio-pros">{o.pros.map((p, k) => <li key={k}><Inline text={p} /></li>)}</ul>}
          {Array.isArray(o.cons) && o.cons.length > 0 && <ul className="folio-cons">{o.cons.map((p, k) => <li key={k}><Inline text={p} /></li>)}</ul>}
          {o.cost != null && <span className="folio-item-meta">cost: {txt(o.cost)}</span>}
        </button>;
      })}
    </div>
  </div>;
}

const Decision: R = ({ v }) => <Choose target={id(v)} options={list(v.options)} question={v.question} why={v.why} />;
const Options: R = ({ v }) => <Choose target={id(v)} options={list(v.items)} question={v.question} />;

const Decisions: R = ({ v }) => <div className="folio-table"><table><thead><tr><th>Question</th><th>Decision</th><th>Effect</th></tr></thead><tbody>
  {list(v.rows).map((r, i) => <tr key={i} id={id(r) ? `f-${id(r)}` : undefined}><td><Inline text={r.question} /></td><td><b><Inline text={r.decision} /></b></td><td><Inline text={r.effect} /></td></tr>)}
</tbody></table></div>;

const Status: R = ({ v }) => <div className="folio-tiles">{list(v.tiles).map((t, i) => <div key={i} className="folio-tile" id={id(t) ? `f-${id(t)}` : undefined}><span className="folio-tile-head">{txt(t.area)}<span className={`folio-pill s-${txt(t.state)}`}>{txt(t.state)}</span></span>{t.text != null && <Inline text={t.text} />}{t.link != null && <a href={txt(t.link)} target="_blank" rel="noreferrer noopener">{txt(t.link)}</a>}</div>)}</div>;

const Timeline: R = ({ v }) => <ol className="folio-timeline">{list(v.items).map((t, i) => <li key={i} id={id(t) ? `f-${id(t)}` : undefined}><span className="folio-date">{txt(t.date)}</span><b><Inline text={t.title} /></b>{t.text != null && <Inline text={t.text} block />}</li>)}</ol>;

const Steps: R = ({ v }) => <ol className="folio-steps">{list(v.items).map((t, i) => <li key={i} id={id(t) ? `f-${id(t)}` : undefined}><span className="folio-step-n">{i + 1}</span><div><b><Inline text={t.title} /></b>{t.text != null && <Inline text={t.text} block />}</div></li>)}</ol>;

const Checklist: R = ({ v }) => {
  const marks = useMarks();
  return <ul className="folio-checklist">{list(v.items).map((t, i) => {
    const tid = id(t);
    const mark = marks.marks.find((m) => m.kind === "check" && m.target === tid && m.author.kind === "owner")?.value;
    const on = mark != null ? mark === "true" : t.done === true;
    return <li key={tid || i} id={tid ? `f-${tid}` : undefined} data-folio-target={tid || undefined}>
      <label><input type="checkbox" checked={on} disabled={marks.readOnly || !tid} onChange={() => marks.setMark("check", tid, String(!on))} /> <Inline text={t.text} /></label>
      {tid && <CommentButton target={tid} />}
    </li>;
  })}</ul>;
};

const Metrics: R = ({ v }) => <div className="folio-metrics">{list(v.items).map((t, i) => <div key={i} className={`folio-metric t-${txt(t.tone) || "neutral"}`}><span className="folio-metric-v">{txt(t.value)}</span><span className="folio-metric-l">{txt(t.label)}</span>{t.text != null && <span className="folio-item-meta"><Inline text={t.text} /></span>}</div>)}</div>;

function csvRows(text: string): string[][] {
  return text.split(/\r?\n/).filter((l) => l.trim()).map((l) => l.split(",").map((c) => c.trim().replace(/^"|"$/g, "")));
}

const Table: R = ({ v }) => {
  const marks = useMarks();
  const src = typeof v.src === "string" ? v.src : null;
  const csv = useQuery({ queryKey: ["folio", marks.ws, marks.slug, "csv", src, marks.rev], queryFn: async () => { const r = await fetch(daemonUrl(`${folioPath(marks.ws, marks.slug)}/file?path=${encodeURIComponent(src!)}${marks.rev ? `&rev=${marks.rev}` : ""}`), { credentials: "include" }); if (!r.ok) throw new Error(`${src}: HTTP ${r.status}`); return r.text(); }, enabled: !!src, staleTime: 60_000 });
  let head = Array.isArray(v.columns) ? v.columns.map(txt) : [];
  let rows = Array.isArray(v.rows) ? (v.rows as unknown[]).map((r) => (Array.isArray(r) ? r.map(txt) : [txt(r)])) : [];
  if (src && csv.data) {
    const all = csvRows(csv.data);
    const header = all[0] ?? [];
    const pick = head.length ? head.map((c) => header.indexOf(c)) : header.map((_, i) => i);
    head = pick.map((i) => header[i] ?? "");
    rows = all.slice(1).map((r) => pick.map((i) => r[i] ?? ""));
  }
  return <div className="folio-table">{v.caption != null && <div className="folio-caption">{txt(v.caption)}</div>}<table>{head.length > 0 && <thead><tr>{head.map((h, i) => <th key={i}>{h}</th>)}</tr></thead>}<tbody>{rows.map((r, i) => <tr key={i}>{r.map((c, j) => <td key={j}><Inline text={c} /></td>)}</tr>)}</tbody></table></div>;
};

const Prompt: R = ({ v }) => <div className="folio-prompt">
  <div className="folio-wave-head"><b>{txt(v.title) || "Prompt"}</b><button type="button" className="btn btn-sm" onClick={() => void copyText(txt(v.text)).then(() => toast("Prompt copied"))}><Icon name="copy" size={12} />Copy</button></div>
  <pre>{txt(v.text)}</pre>
  {(v.profile != null || v.folder != null) && <div className="folio-item-meta">{[v.profile != null && `profile: ${txt(v.profile)}`, v.folder != null && `folder: ${txt(v.folder)}`].filter(Boolean).join(" · ")}</div>}
</div>;

const Sources: R = ({ v }) => <ol className="folio-sources">{list(v.items).map((s, i) => <li key={i}>{s.url != null ? <a href={txt(s.url)} target="_blank" rel="noreferrer noopener">{txt(s.title)}</a> : <b>{txt(s.title)}</b>}{s.note != null && <span className="folio-item-meta"> · <Inline text={s.note} /></span>}</li>)}</ol>;

function KeyValues({ name, v }: { name: string; v: Record<string, unknown> }) {
  return <div className="folio-kv" title={`\`\`\`${name} is not in the catalog: it shows as keys and values`}>
    <div className="folio-caption">{name}</div>
    <dl>{Object.entries(v).map(([k, x]) => <div key={k}><dt>{k}</dt><dd>{typeof x === "object" && x ? <pre>{JSON.stringify(x, null, 2)}</pre> : txt(x)}</dd></div>)}</dl>
  </div>;
}

const RENDER: Record<string, R> = { roadmap: Roadmap, findings: Findings, decision: Decision, options: Options, decisions: Decisions, status: Status, timeline: Timeline, steps: Steps, checklist: Checklist, metrics: Metrics, table: Table, prompt: Prompt, sources: Sources };
