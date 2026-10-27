// The composer prompt. `@` (projects) and `#` (issues and PRs) open a suggestion list, only at the
// start of a word (ARCHITECTURE.md, section 9.7): `user@host`, `C#` and Markdown headings never do.
// Picking an item adds a context chip and leaves a readable `@name` or `#123` in the text.

import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import type { GhItem } from "@/gen/GhItem";
import type { Project } from "@/gen/Project";
import { Icon, type IconName } from "@/components/Icon";
import { api } from "@/lib/api";

type Mention = { trigger: "@" | "#"; start: number; query: string };

/** The mention being typed at the caret, if any. */
export function mentionAt(text: string, caret: number): Mention | null {
  let i = caret;
  while (i > 0 && !/\s/.test(text[i - 1])) i--;
  const word = text.slice(i, caret);
  const trigger = word[0];
  if (trigger !== "@" && trigger !== "#") return null;
  // The trigger must start the word: nothing glued before it (already ensured by the scan).
  const query = word.slice(1);
  // `#` waits for a first character, so a Markdown heading (`# Title`, `## Title`) never opens the list.
  if (trigger === "#" && (query === "" || query.startsWith("#"))) return null;
  return { trigger, start: i, query };
}

export const itemIcon = (kind: string, state: string): { name: IconName; color: string } => {
  if (kind === "pr") {
    if (state === "merged") return { name: "pr-merged", color: "text-gh-merged" };
    if (state === "draft") return { name: "pr-draft", color: "text-gh-draft" };
    if (state === "closed") return { name: "pr", color: "text-gh-closed" };
    return { name: "pr", color: "text-gh-open" };
  }
  if (state === "closed") return { name: "issue-done", color: "text-gh-merged" };
  return { name: "issue", color: "text-gh-open" };
};

export function PromptField({
  value,
  onChange,
  projects,
  repo,
  onPickProject,
  onPickItem,
  onPaste,
  disabled,
}: {
  onPaste?: React.ClipboardEventHandler<HTMLTextAreaElement>;
  disabled?: boolean;
  value: string;
  onChange: (v: string) => void;
  projects: Project[];
  repo: string | null;
  onPickProject: (p: Project) => void;
  onPickItem: (i: GhItem) => void;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const [mention, setMention] = useState<Mention | null>(null);
  const [sel, setSel] = useState(0);
  const [debounced, setDebounced] = useState("");

  useEffect(() => {
    const t = setTimeout(() => setDebounced(mention?.trigger === "#" ? mention.query : ""), 150);
    return () => clearTimeout(t);
  }, [mention]);

  const gh = useQuery({
    queryKey: ["gh-suggest", repo, debounced],
    queryFn: () => api.get<GhItem[]>(`/api/gh/${repo}/suggest?q=${encodeURIComponent(debounced)}`),
    enabled: mention?.trigger === "#" && !!repo,
    staleTime: 60_000,
  });

  const projectHits = mention?.trigger === "@" ? projects.filter((p) => p.name.toLowerCase().includes(mention.query.toLowerCase())).slice(0, 8) : [];
  const itemHits = mention?.trigger === "#" ? (gh.data ?? []) : [];
  const count = mention?.trigger === "@" ? projectHits.length : itemHits.length;

  const update = (text: string, caret: number) => {
    onChange(text);
    const m = mentionAt(text, caret);
    setMention(m);
    setSel(0);
  };

  const pick = (index: number) => {
    if (!mention) return;
    const el = ref.current!;
    const caret = el.selectionStart;
    let token: string;
    if (mention.trigger === "@") {
      const p = projectHits[index];
      if (!p) return;
      token = `@${p.name}`;
      onPickProject(p);
    } else {
      const it = itemHits[index];
      if (!it) return;
      token = `#${it.number}`;
      onPickItem(it);
    }
    const text = value.slice(0, mention.start) + token + " " + value.slice(caret);
    onChange(text);
    setMention(null);
    const pos = mention.start + token.length + 1;
    requestAnimationFrame(() => {
      el.focus();
      el.setSelectionRange(pos, pos);
    });
  };

  return (
    <div className="relative">
      <textarea
        ref={ref}
        onPaste={onPaste}
        disabled={disabled}
        value={value}
        rows={7}
        autoFocus
        placeholder={"What should the agent do?\nType @ for a folder or # for an issue or PR. Add more context with + Add context, or add lines from Files."}
        className="block min-h-[140px] w-full resize-y rounded-lg border-[1.5px] border-line bg-paper px-2.5 pt-2 pb-9 text-[13px] leading-normal outline-none focus:border-teal focus:shadow-[0_0_0_3px_var(--color-teal-soft)]"
        onChange={(e) => update(e.target.value, e.target.selectionStart)}
        onClick={(e) => setMention(mentionAt(value, e.currentTarget.selectionStart))}
        onBlur={() => setTimeout(() => setMention(null), 150)}
        onKeyDown={(e) => {
          // Only while the list is open; otherwise keys type text as usual.
          if (!mention || count === 0) {
            if (e.key === "Escape") setMention(null);
            return;
          }
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setSel((s) => (s + 1) % count);
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setSel((s) => (s - 1 + count) % count);
          } else if (e.key === "Enter") {
            e.preventDefault();
            pick(sel);
          } else if (e.key === "Escape") {
            e.preventDefault();
            setMention(null);
          }
        }}
      />
      {mention && (
        <div className="absolute top-[calc(100%-4px)] left-2 z-30 grid w-[340px] gap-px rounded-[9px] border border-line bg-paper p-[5px] shadow-[0_12px_28px_-10px_rgba(23,32,43,.35)]">
          {mention.trigger === "@" &&
            projectHits.map((p, i) => (
              <button
                key={p.id}
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(i);
                }}
                className={`grid grid-cols-[16px_1fr] gap-x-[9px] rounded-md px-[9px] py-[6px] text-left text-[13px] ${i === sel ? "bg-select" : "hover:bg-sunk"}`}
              >
                <Icon name="folder" size={14} className="mt-0.5 text-ink-2" />
                <span className="truncate">
                  <b className="font-semibold">{p.name}</b> <span className="text-ink-3">{p.description}</span>
                </span>
              </button>
            ))}
          {mention.trigger === "#" &&
            itemHits.map((it, i) => {
              const ic = itemIcon(it.kind, it.state);
              return (
                <button
                  key={it.number}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    pick(i);
                  }}
                  className={`grid grid-cols-[16px_1fr] gap-x-[9px] gap-y-px rounded-md px-[9px] py-[6px] text-left text-[13px] ${i === sel ? "bg-select" : "hover:bg-sunk"}`}
                >
                  <Icon name={ic.name} size={14} className={`mt-0.5 ${ic.color}`} />
                  <span className="truncate">
                    <b className="mr-1 font-mono text-xs font-semibold">#{it.number}</b>
                    {it.title}
                  </span>
                  <span className="col-start-2 text-[12px] text-ink-3">
                    {it.kind === "pr" ? "PR" : "Issue"} · {it.author} · {it.state}
                  </span>
                </button>
              );
            })}
          <div className="border-t border-line-2 px-[9px] pt-[5px] pb-[3px] text-[11px] text-ink-3">
            {mention.trigger === "@"
              ? projectHits.length
                ? "Registered projects"
                : "No matching folder"
              : !repo
                ? "Pick a project with a GitHub remote first"
                : gh.isLoading
                  ? "Loading…"
                  : gh.isError
                    ? `Could not load: ${(gh.error as Error).message}`
                    : itemHits.length
                      ? `Issues and PRs of ${repo}`
                      : "No match"}
          </div>
        </div>
      )}
    </div>
  );
}
