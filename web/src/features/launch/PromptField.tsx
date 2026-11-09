// Shared launch/reply editor. Search is read-only; accepted # references are draft transactions.
import { memo, useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Icon, type IconName } from "@/components/Icon";
import { api } from "@/lib/api";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { useZoomScale } from "@/lib/zoom";
import { usePhone } from "@/lib/phone";
import { editText, insertMention, replaceText, PromptHistory, type ContextPick, type PromptDocument } from "@/lib/promptDocument";
import type { PathResults } from "@/gen/PathResults";
import type { SkillResults } from "@/gen/SkillResults";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { GhItem } from "@/gen/GhItem";
import { matchScore } from "./ProjectPicker";

export { itemIcon } from "./itemIcon";
export { mentionAt } from "@/lib/promptTriggers";
import { mentionAt, type Mention } from "@/lib/promptTriggers";

type Suggestion = { id: string; group: string; label: string; detail: string; token: string; icon: IconName; unavailable?: boolean; pick?: ContextPick };
const SuggestionRow = memo(function SuggestionRow({ item, active, id, onPick }: { item: Suggestion; active: boolean; id: string; onPick: () => void }) {
  return <button type="button" tabIndex={-1} id={id} role="option" aria-selected={active} disabled={item.unavailable} onMouseDown={(e) => e.preventDefault()} onClick={onPick} className={`grid h-12 w-full cursor-pointer text-left disabled:opacity-50 grid-cols-[20px_minmax(0,1fr)] content-center gap-x-2 rounded-control px-2 text-[13px] ${active ? "bg-select" : "hover:bg-sunk"}`}>
    <Icon name={item.icon} size={14} className="row-span-2 self-center" /><span className="truncate font-semibold">{item.label}</span><span className="truncate text-[11px] text-ink-3">{item.group} · {item.detail}</span>
  </button>;
});

export function PromptField({ document: doc, onDocumentChange, root, harness, profile, repo, projectId, sessionId, relativePaths = true, noFolders = false, autocomplete = true, inputRef, onPaste, onSend, disabled, compact = false, placeholder }: {
  document: PromptDocument; onDocumentChange: (doc: PromptDocument) => void;
  root: string | null; harness: string | null; profile: string | null; repo: string | null; projectId?: string | null; sessionId?: string;
  relativePaths?: boolean; noFolders?: boolean; autocomplete?: boolean; inputRef?: React.RefObject<HTMLTextAreaElement | null>;
  onPaste?: React.ClipboardEventHandler<HTMLTextAreaElement>; onSend?: () => void; disabled?: boolean; compact?: boolean; placeholder?: string;
}) {
  const ownRef = useRef<HTMLTextAreaElement>(null), ref = inputRef ?? ownRef;
  const scroll = useRef<HTMLDivElement>(null), panel = useRef<HTMLDivElement>(null);
  const [mention, setMention] = useState<Mention | null>(null), [selected, setSelected] = useState(0);
  const [debounced, setDebounced] = useState("");
  const [position, setPosition] = useState<CSSProperties>({});
  const id = useId(), scale = useZoomScale(), scope = useScope();
  const phone = usePhone();
  const sessions = useSessions().data ?? [];
  const history = useRef(new PromptHistory(doc));
  const identity = `${sessionId ?? "launch"}:${scope.workspace.id}:${autocomplete}`;
  const priorIdentity = useRef(identity);
  if (priorIdentity.current !== identity) { history.current = new PromptHistory(doc); priorIdentity.current = identity; }
  // Chip operations and external draft changes are also part of full-document history.
  useEffect(() => { history.current.sync(doc); }, [doc]);
  const commit = (next: PromptDocument) => { history.current.record(next); onDocumentChange(next); };
  const scopeKey = `${root}|${harness}|${profile}|${repo}`;
  useEffect(() => { setMention(null); setSelected(0); }, [scopeKey, autocomplete]);
  useEffect(() => { const t = setTimeout(() => setDebounced(mention?.query ?? ""), 150); return () => clearTimeout(t); }, [mention?.query]);
  const params = new URLSearchParams({ ...(root ? { root } : {}), workspace: scope.workspace.id, q: debounced });
  const files = useQuery({ queryKey: ["prompt-files", root, scope.workspace.id, debounced], queryFn: ({ signal }) => api.get<PathResults>(`/api/prompt/files?${params}`, signal), enabled: autocomplete && !noFolders && mention?.trigger === "@", staleTime: 15_000 });
  const skillParams = new URLSearchParams({ harness: harness ?? "", ...(profile ? { profile } : {}), ...(root ? { root } : {}) });
  const skills = useQuery({ queryKey: ["prompt-skills", root, harness, profile], queryFn: ({ signal }) => api.get<SkillResults>(`/api/prompt/skills?${skillParams}`, signal), enabled: autocomplete && !noFolders && mention?.trigger === "/" && !!harness, staleTime: 30_000 });
  const gh = useQuery({ queryKey: ["gh-suggest", repo, debounced], queryFn: ({ signal }) => api.get<GhItem[]>(`/api/gh/${repo}/suggest?q=${encodeURIComponent(debounced)}`, signal), enabled: mention?.trigger === "#" && !!repo, staleTime: 60_000 });
  const cachedGitHub = useQuery({ queryKey: ["prompt-github", debounced], queryFn: ({ signal }) => api.get<GitHubRef[]>(`/api/prompt/github?q=${encodeURIComponent(debounced)}`, signal), enabled: mention?.trigger === "#", staleTime: 30_000 });
  const query = (mention?.query ?? "").toLowerCase();
  const rank = (text: string) => matchScore(text, query);
  const suggestions: Suggestion[] = [];
  if (mention?.trigger === "@" && !noFolders && debounced === mention.query) {
    const hits = files.data?.hits ?? [];
    for (const hit of hits) {
      const path = relativePaths && root === hit.root ? hit.path : `${hit.root.replace(/[\\/]$/, "")}/${hit.path}`;
      suggestions.push({ id: `${hit.root}:${hit.path}`, group: hit.directory ? "Folder" : "File", label: hit.path, detail: hit.project, token: /\s/.test(path) ? JSON.stringify(path) : path, icon: hit.directory ? "folder" : "file" });
    }
  }
  if (mention?.trigger === "/" && !noFolders) for (const skill of skills.data?.hits ?? []) {
    if (rank(`${skill.name} ${skill.description}`) === null) continue;
    suggestions.push({ id: skill.path, group: "Skill", label: `/${skill.name}`, detail: `${skill.description} · ${skill.source}`, token: `/${skill.name}`, icon: "file", pick: { kind: "skill", value: { name: skill.name, path: skill.path } } });
  }
  if (mention?.trigger === "#") {
    // Only the folders of this window's workspace: other workspaces show counts, never their content.
    const folders = noFolders ? [] : scope.projects.filter((p) => p.id !== projectId && rank(`${p.name} ${p.path} ${p.description}`) !== null).sort((a,b) => (rank(a.name) ?? 100) - (rank(b.name) ?? 100));
    for (const p of folders.slice(0, 10)) suggestions.push({ id: p.id, group: "Project", label: p.name, detail: p.path, token: `#${p.name.replace(/\s+/g, "-")}`, icon: "folder", pick: { kind: "project", value: { project_id: p.id, branch: null, editable: false } } });
    for (const s of sessions.filter((s) => s.id !== sessionId && s.last_seq > 0 && scope.hasSession(s) && rank(`${s.title} ${s.cwd}`) !== null).sort((a,b) => b.started_at - a.started_at).slice(0, 10)) suggestions.push({ id: s.id, group: "Trace", label: s.title, detail: `${s.cwd} · through event ${s.last_seq}`, token: `#trace:${s.title.replace(/[^\p{L}\p{N}]+/gu, "-").slice(0, 40) || "agent"}`, icon: "agents", pick: { kind: "trace", value: { session_id: s.id, through_seq: s.last_seq } } });
    const github = new Map<string, GitHubRef>();
    for (const s of sessions.filter(scope.hasSession)) for (const g of s.launch_spec.github_refs) if (rank(`${g.repo}#${g.number} ${g.title ?? ""}`) !== null) github.set(g.url, g);
    for (const g of cachedGitHub.data ?? []) github.set(g.url, g);
    for (const g of gh.data ?? []) github.set(g.url, { ...g, number: g.number, title: g.title });
    if (debounced === mention.query) for (const g of [...github.values()].slice(0, 10)) suggestions.push({ id: g.url, group: g.kind === "pr" ? "PR" : "Issue", label: `${g.repo}#${g.number} ${g.title}`, detail: g.repo, token: `#${g.repo}#${g.number}`, icon: g.kind === "pr" ? "pr" : "issue", pick: { kind: "github", value: { url: g.url, kind: g.kind, repo: g.repo, number: g.number, title: g.title } } });
  }
  const rows = suggestions.slice(0, 40), active = Math.min(selected, Math.max(0, rows.length - 1));
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => scroll.current, estimateSize: () => 48, overscan: 3, useFlushSync: false });
  useEffect(() => { virtual.scrollToIndex(active, { align: "auto" }); }, [active, rows.length]);
  useLayoutEffect(() => {
    if (!mention) return;
    const place = () => {
      const anchor = ref.current?.getBoundingClientRect(); if (!anchor) return;
      const view = window.visualViewport, x = view?.offsetLeft ?? 0, y = view?.offsetTop ?? 0, w = view?.width ?? innerWidth, h = view?.height ?? innerHeight;
      const below = Math.max(0, y + h - anchor.bottom - 12), above = Math.max(0, anchor.top - y - 12), up = above > below;
      const height = Math.min(280 * scale, up ? above : below);
      setPosition({ left: Math.max(x + 8, Math.min(anchor.left, x + w - Math.min(380 * scale, w - 16) - 8)) / scale, top: (up ? anchor.top - 4 : anchor.bottom + 4) / scale, transform: up ? "translateY(-100%)" : undefined, width: Math.min(380, (w - 16) / scale), maxHeight: height / scale });
    };
    place(); window.addEventListener("resize", place); document.addEventListener("scroll", place, true); window.visualViewport?.addEventListener("resize", place); window.visualViewport?.addEventListener("scroll", place);
    return () => { window.removeEventListener("resize", place); document.removeEventListener("scroll", place, true); window.visualViewport?.removeEventListener("resize", place); window.visualViewport?.removeEventListener("scroll", place); };
  }, [mention, rows.length, scale, ref]);
  const pick = (index: number) => {
    const hit = rows[index], el = ref.current; if (!hit || hit.unavailable || !mention || !el) return;
    const next = insertMention(doc, mention.start, el.selectionStart, hit.token, hit.pick);
    commit(next); setMention(null);
    requestAnimationFrame(() => { el.focus(); el.setSelectionRange(mention.start + hit.token.length + 1, mention.start + hit.token.length + 1); });
  };
  const composing = useRef(false);
  const selection = useRef<{ start: number; end: number; input: string } | null>(null);
  const detect = () => { const el = ref.current; setMention(autocomplete && !composing.current && el && el.selectionStart === el.selectionEnd ? mentionAt(el.value, el.selectionStart) : null); setSelected(0); };
  const restore = (redo: boolean) => { const next = redo ? history.current.redo() : history.current.undo(); if (next) onDocumentChange(next); setMention(null); };
  // React's synthetic before-input event can be backed by TextEvent rather than InputEvent.
  // Listen to the native event for exact deletion ranges and menu-driven undo/redo.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const before = (event: InputEvent) => {
      if (event.inputType === "historyUndo" || event.inputType === "historyRedo") {
        event.preventDefault();
        restore(event.inputType === "historyRedo");
        return;
      }
      selection.current = { start: el.selectionStart, end: el.selectionEnd, input: event.inputType ?? "" };
    };
    el.addEventListener("beforeinput", before);
    return () => el.removeEventListener("beforeinput", before);
  }, [ref, onDocumentChange]);
  // The full prompt grows with its text up to 300 px (240 px on a phone, so the folder fields stay in view), then scrolls; its frame (.composer-prompt) holds the attach row.
  useLayoutEffect(() => {
    const el = ref.current;
    if (compact || !el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, phone ? 240 : 300)}px`;
  }, [compact, doc.text, phone, ref]);
  const pending = mention?.trigger === "@" ? files.isFetching || debounced !== mention.query : mention?.trigger === "/" ? skills.isFetching : gh.isFetching;
  const error = mention?.trigger === "@" ? files.error : mention?.trigger === "/" ? skills.error : gh.error;
  const warnings = mention?.trigger === "@" ? files.data?.unavailable.map((p) => `${p} unavailable`) : mention?.trigger === "/" ? skills.data?.warnings : [];
  return <>
    <textarea ref={ref} disabled={disabled} value={doc.text} rows={compact ? 1 : 7} autoFocus={!compact} onPaste={onPaste} placeholder={placeholder ?? "What should the agent do?\n@ files and folders · / skills · # context"}
      aria-label={compact ? "Message agent" : "Agent prompt"} aria-autocomplete="list" aria-expanded={!!mention} aria-controls={mention ? id : undefined} aria-activedescendant={mention && rows.length ? `${id}-${active}` : undefined}
      className={compact ? "max-h-40 min-h-5 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none disabled:opacity-60" : "block min-h-[140px] w-full resize-none rounded-t-card bg-transparent px-2.5 pt-2 pb-2 text-[13px] leading-normal outline-none"}
      onChange={(e) => {
        const el = e.currentTarget, before = selection.current; selection.current = null;
        let next = editText(doc, el.value);
        if (before && before.input.startsWith("insert") && before.start <= before.end) {
          const length = el.value.length - (doc.text.length - (before.end - before.start));
          if (length >= 0 && el.value.slice(0, before.start) === doc.text.slice(0, before.start) && el.value.slice(before.start + length) === doc.text.slice(before.end)) next = replaceText(doc, before.start, before.end, el.value.slice(before.start, before.start + length));
        }
        if (before?.input.startsWith("delete")) {
          const removed = doc.text.length - el.value.length;
          const start = before.start === before.end && before.input.includes("Backward") ? before.start - removed : before.start;
          const end = before.start === before.end && !before.input.includes("Backward") ? before.start + removed : before.end;
          if (start >= 0 && doc.text.slice(0, start) + doc.text.slice(end) === el.value) next = replaceText(doc, start, end, "");
        }
        commit(next); if (!(e.nativeEvent as InputEvent).isComposing) detect();
      }}
      onSelect={detect} onCompositionStart={() => { composing.current = true; setMention(null); }} onCompositionEnd={() => { composing.current = false; detect(); }} onBlur={() => setMention(null)}
      onKeyDown={(e) => {
        // Some keyboards omit isComposing on the key that finishes a composition.
        // Leave every shortcut to the IME until its compositionend has arrived.
        if (composing.current || e.nativeEvent.isComposing || e.keyCode === 229) return;
        if ((e.ctrlKey || e.metaKey) && !e.altKey && (e.key.toLowerCase() === "z" || e.key.toLowerCase() === "y")) { e.preventDefault(); restore(e.shiftKey || e.key.toLowerCase() === "y"); return; }
        if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
        // Phone keyboards (including voice typing) use Enter for line breaks.
        // Delivery is explicit through the adjacent Send button on phones.
        if (phone && e.key === "Enter") { setMention(null); return; }
        if (mention) {
          if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); setMention(null); return; }
          if ((e.key === "ArrowDown" || e.key === "ArrowUp") && rows.length) { e.preventDefault(); setSelected((active + (e.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length); return; }
          if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey && rows.length) { e.preventDefault(); pick(active); return; }
          if (e.key === "Enter" && !e.shiftKey) { setMention(null); if (onSend) { e.preventDefault(); return; } }
        }
        if (e.key === "Enter" && !e.shiftKey && onSend) { e.preventDefault(); onSend(); }
      }} />
    {/* Above the tour overlay (z-index 80), so a Try it step can show and pick suggestions. */}
    {mention && createPortal(<div ref={panel} className="fixed z-[85] flex flex-col overflow-hidden rounded-card border border-line bg-paper p-1 shadow-xl" style={position}>
      <div id={id} role="listbox" aria-label="Prompt suggestions" ref={scroll} className="min-h-0 overflow-y-auto" style={{ height: Math.min(rows.length * 48, 240) }}>
        <div className="relative" style={{ height: virtual.getTotalSize() }}>{virtual.getVirtualItems().map((row) => <div key={rows[row.index].id} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${row.start}px)` }}><SuggestionRow id={`${id}-${row.index}`} item={rows[row.index]} active={row.index === active} onPick={() => pick(row.index)} /></div>)}</div>
      </div>
      <div role="status" className="shrink-0 border-t border-line-2 px-2 py-1 text-[11px] text-ink-3">{error ? error.message : pending ? "Searching…" : !rows.length ? noFolders && mention.trigger !== "#" ? "This agent has no filesystem access" : "No matching items" : "↑ ↓ navigate · Enter/Tab select · Esc dismiss"}{!!warnings?.length && <span className="block">{warnings.slice(0, 2).join(" · ")}</span>}</div>
    </div>, document.body)}
  </>;
}
