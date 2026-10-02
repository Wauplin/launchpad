// Code lines (ARCHITECTURE.md, section 8.1): TanStack Virtual with a fixed row height, no wrapping,
// plain text until the Shiki worker answers. Bars next to the numbers mark the changed lines; a click
// on a number selects a line, a drag or a shift-click extends the selection.

import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, type ReactNode, type Ref } from "react";
import type { LineMark } from "@/gen/LineMark";
import type { Tokens } from "./highlight";

export const ROW = 20;

export type Selection = { start: number; end: number };
export type CodeLinesHandle = { scrollToLine: (line: number, align?: "start" | "center") => void; topLine: () => number; revealMatch: () => void };

type Token = Tokens[number][number];

const BAR: Record<string, string> = { add: "bg-gh-open", mod: "bg-[#d4a72c]", del: "bg-gh-closed" };

function style(color: string | undefined, fs: number | undefined) {
  if (!color && !fs) return undefined;
  return { color, fontStyle: fs && fs & 1 ? "italic" : undefined, fontWeight: fs && fs & 2 ? 600 : undefined };
}

/** Splits the colored tokens at the find matches. Text comes from the line itself, so a token list
 * that does not add up to the line never changes what is shown. */
function pieces(text: string, toks: Token[] | undefined, ranges: [number, number][] | undefined, current: number): ReactNode {
  if (!ranges?.length && !toks) return text;
  const base: Token[] = toks ?? [[text, undefined, undefined]];
  const out: ReactNode[] = [];
  let pos = 0;
  let ri = 0;
  const push = (a: number, b: number, color: string | undefined, fs: number | undefined, mark: boolean, cur: boolean) => {
    if (b <= a) return;
    const s = <span key={out.length} style={style(color, fs)}>{text.slice(a, b)}</span>;
    out.push(mark ? <mark key={out.length} className={cur ? "cv-m cur" : "cv-m"} data-cur={cur || undefined}>{s}</mark> : s);
  };
  for (const [t, color, fs] of base) {
    const end = Math.min(text.length, pos + t.length);
    let at = pos;
    while (at < end) {
      while (ranges && ri < ranges.length && ranges[ri][1] <= at) ri++;
      const r = ranges?.[ri];
      if (!r || r[0] >= end) {
        push(at, end, color, fs, false, false);
        break;
      }
      if (r[0] > at) {
        push(at, r[0], color, fs, false, false);
        at = r[0];
        continue;
      }
      const stop = Math.min(end, r[1]);
      push(at, stop, color, fs, true, r[0] === current);
      at = stop;
    }
    pos = end;
  }
  if (pos < text.length) push(pos, text.length, undefined, undefined, false, false);
  return out;
}

const Line = memo(function Line({
  n,
  text,
  toks,
  mark,
  ranges,
  current,
  selected,
  start,
  digits,
  onDown,
  onEnter,
}: {
  n: number;
  digits: number;
  text: string;
  toks: Token[] | undefined;
  mark: string | undefined;
  ranges: [number, number][] | undefined;
  current: number;
  selected: boolean;
  start: number;
  onDown: (line: number, shift: boolean) => void;
  onEnter: (line: number) => void;
}) {
  return (
    <div
      className={`cv-line absolute top-0 left-0 flex w-full ${selected ? "bg-select" : ""}`}
      style={{ height: ROW, transform: `translateY(${start}px)` }}
      onMouseEnter={() => onEnter(n)}
    >
      <span
        className={`sticky left-0 z-[1] flex flex-none items-stretch select-none ${selected ? "bg-select" : "bg-paper"}`}
        onMouseDown={(e) => {
          e.preventDefault();
          onDown(n, e.shiftKey);
        }}
        title="Select the line; drag or shift-click to extend"
      >
        <span className={`my-px w-[3px] flex-none rounded-sm ${mark ? BAR[mark] : ""}`} />
        <span className={`pr-3 pl-2 text-right tabular-nums ${selected ? "font-semibold text-ink" : "text-ink-3"} hover:text-ink`} style={{ width: `calc(${digits}ch + 20px)` }}>
          {n}
        </span>
      </span>
      <span className="cv-t pr-6 pl-1">{pieces(text, toks, ranges, current)}</span>
    </div>
  );
});

export function CodeLines({
  lines,
  tokens,
  marks,
  matchesByLine,
  current,
  selection,
  onSelect,
  initialTop,
  onTop,
  handle,
}: {
  lines: string[];
  tokens: Tokens | null;
  marks: LineMark[];
  matchesByLine: Map<number, [number, number][]>;
  current: { line: number; start: number } | null;
  selection: Selection | null;
  onSelect: (s: Selection | null) => void;
  initialTop: { line: number; align: "start" | "center" } | null;
  onTop: (line: number) => void;
  handle: Ref<CodeLinesHandle>;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: lines.length, getScrollElement: () => scrollRef.current, estimateSize: () => ROW, overscan: 12, useFlushSync: false });
  const markOf = useMemo(() => {
    const out: (string | undefined)[] = [];
    for (const m of marks) for (let l = m.start; l <= m.end && l <= lines.length; l++) out[l] = m.kind;
    return out;
  }, [marks, lines.length]);
  const width = useMemo(() => {
    let max = 0;
    for (const l of lines) {
      const tabs = l.length - l.replaceAll("\t", "").length;
      max = Math.max(max, l.length + tabs * 3);
    }
    return max;
  }, [lines]);
  const digits = String(lines.length).length;

  // Selection by the line numbers.
  const anchor = useRef<number | null>(null);
  const dragging = useRef(false);
  const sel = useRef(selection);
  sel.current = selection;
  const onDown = useCallback(
    (line: number, shift: boolean) => {
      const s = sel.current;
      if (shift && s) {
        const a = anchor.current ?? s.start;
        onSelect({ start: Math.min(a, line), end: Math.max(a, line) });
      } else if (s && s.start === line && s.end === line) {
        anchor.current = null;
        onSelect(null);
        return;
      } else {
        anchor.current = line;
        onSelect({ start: line, end: line });
      }
      dragging.current = true;
    },
    [onSelect],
  );
  const onEnter = useCallback(
    (line: number) => {
      if (!dragging.current || anchor.current === null) return;
      const a = anchor.current;
      onSelect({ start: Math.min(a, line), end: Math.max(a, line) });
    },
    [onSelect],
  );
  useEffect(() => {
    const up = () => (dragging.current = false);
    window.addEventListener("mouseup", up);
    return () => window.removeEventListener("mouseup", up);
  }, []);

  useImperativeHandle(
    handle,
    () => ({
      scrollToLine: (line, align = "center") => v.scrollToIndex(Math.max(0, line - 1), { align }),
      topLine: () => Math.floor((scrollRef.current?.scrollTop ?? 0) / ROW) + 1,
      revealMatch: () =>
        requestAnimationFrame(() => requestAnimationFrame(() => scrollRef.current?.querySelector("mark[data-cur]")?.scrollIntoView({ block: "nearest", inline: "nearest" }))),
    }),
    [v],
  );

  // The component is keyed by file: restore the position once.
  useLayoutEffect(() => {
    if (initialTop) v.scrollToIndex(Math.max(0, initialTop.line - 1), { align: initialTop.align });
  }, []);

  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  return (
    <div
      ref={scrollRef}
      className="cv-code min-h-0 flex-1 overflow-auto"
      onScroll={() => {
        clearTimeout(timer.current);
        timer.current = setTimeout(() => onTop(Math.floor((scrollRef.current?.scrollTop ?? 0) / ROW) + 1), 300);
      }}
    >
      <div className="relative" style={{ height: v.getTotalSize() + 96, width: `max(100%, calc(${width + digits}ch + 60px))` }}>
        {v.getVirtualItems().map((it) => {
          const i = it.index;
          const n = i + 1;
          return (
            <Line
              key={i}
              n={n}
              text={lines[i]}
              toks={tokens?.[i]}
              mark={markOf[n]}
              ranges={matchesByLine.get(i)}
              current={current?.line === i ? current.start : -1}
              selected={!!selection && n >= selection.start && n <= selection.end}
              start={it.start}
              digits={digits}
              onDown={onDown}
              onEnter={onEnter}
            />
          );
        })}
      </div>
    </div>
  );
}
