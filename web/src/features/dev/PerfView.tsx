// Spike 4: WebKitGTK performance check (docs/spikes/4-webkitgtk.md).
// A virtualized 5,000-line diff and a 2,000-event session with realistic DOM weight, and an
// automatic scroll benchmark that measures frame times. Open /dev/perf?auto=1 to run everything
// and write the results to the daemon log.

import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { logToDaemon } from "@/lib/api";

type DiffLine = { kind: "ctx" | "add" | "del" | "hunk"; old?: number; new?: number; tokens: [string, string][] };

const WORDS = ["self", "repo_id", "revision", "filename", "token", "return", "await", "client", "headers", "path", "os", "None", "True", "kwargs", "cache_dir", "etag"];
const KW = ["def", "if", "for", "return", "import", "from", "with", "class", "raise", "elif"];

function rng(seed: number) {
  return () => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return seed / 0x7fffffff;
  };
}

function makeDiff(n: number): DiffLine[] {
  const r = rng(7);
  const lines: DiffLine[] = [];
  let o = 1;
  let nw = 1;
  for (let i = 0; i < n; i++) {
    if (i % 60 === 0) {
      lines.push({ kind: "hunk", tokens: [["hunk", `@@ -${o},40 +${nw},44 @@ def snapshot_download(repo_id: str, *, revision: Optional[str] = None):`]] });
      continue;
    }
    const x = r();
    const kind = x < 0.15 ? "add" : x < 0.25 ? "del" : "ctx";
    const indent = "    ".repeat(Math.floor(r() * 4));
    const tokens: [string, string][] = [["", indent]];
    const count = 3 + Math.floor(r() * 7);
    for (let t = 0; t < count; t++) {
      const y = r();
      if (t === 0 && y < 0.4) tokens.push(["kw", KW[Math.floor(r() * KW.length)] + " "]);
      else if (y < 0.15) tokens.push(["str", `"${WORDS[Math.floor(r() * WORDS.length)]}" `]);
      else if (y < 0.25) tokens.push(["num", `${Math.floor(r() * 1000)} `]);
      else tokens.push(["", WORDS[Math.floor(r() * WORDS.length)] + (r() < 0.3 ? "(" : r() < 0.5 ? ", " : ".")]);
    }
    lines.push({ kind, old: kind === "add" ? undefined : o, new: kind === "del" ? undefined : nw, tokens });
    if (kind !== "add") o++;
    if (kind !== "del") nw++;
  }
  return lines;
}

type SessionEvent =
  | { kind: "user"; text: string }
  | { kind: "assistant"; paras: string[]; code?: string }
  | { kind: "tool"; name: string; input: string; secs: number }
  | { kind: "edit"; path: string; lines: DiffLine[] };

function makeSession(n: number): SessionEvent[] {
  const r = rng(11);
  const diff = makeDiff(400);
  const sentence = () =>
    Array.from({ length: 8 + Math.floor(r() * 18) }, () => WORDS[Math.floor(r() * WORDS.length)]).join(" ") + ".";
  const out: SessionEvent[] = [];
  for (let i = 0; i < n; i++) {
    const x = r();
    if (i % 25 === 0) out.push({ kind: "user", text: sentence() + " " + sentence() });
    else if (x < 0.35)
      out.push({
        kind: "assistant",
        paras: Array.from({ length: 1 + Math.floor(r() * 3) }, () => sentence() + " " + sentence()),
        code: r() < 0.3 ? Array.from({ length: 4 + Math.floor(r() * 10) }, sentence).join("\n") : undefined,
      });
    else if (x < 0.85)
      out.push({ kind: "tool", name: r() < 0.5 ? "Bash" : "Read", input: `pytest tests/test_${WORDS[Math.floor(r() * WORDS.length)]}.py -k ${WORDS[i % WORDS.length]}`, secs: Math.floor(r() * 30) });
    else {
      const start = Math.floor(r() * 380);
      out.push({ kind: "edit", path: `src/huggingface_hub/${WORDS[i % WORDS.length]}.py`, lines: diff.slice(start, start + 6 + Math.floor(r() * 8)) });
    }
  }
  return out;
}

const TOKEN_CLASS: Record<string, string> = { kw: "text-gh-closed", str: "text-work", num: "text-work", hunk: "text-ink-3", "": "" };
const LINE_CLASS: Record<DiffLine["kind"], string> = { add: "bg-diff-add", del: "bg-diff-del", ctx: "", hunk: "bg-work-soft" };

const DiffRow = memo(function DiffRow({ l }: { l: DiffLine }) {
  return (
    <div className={`flex h-5 font-mono text-xs leading-5 whitespace-pre ${LINE_CLASS[l.kind]}`}>
      <span className="w-12 flex-none pr-2 text-right text-ink-3 select-none">{l.old ?? ""}</span>
      <span className="w-12 flex-none pr-2 text-right text-ink-3 select-none">{l.new ?? ""}</span>
      <span className="w-4 flex-none select-none">{l.kind === "add" ? "+" : l.kind === "del" ? "-" : " "}</span>
      <span>
        {l.tokens.map(([c, t], i) => (
          <span key={i} className={TOKEN_CLASS[c]}>
            {t}
          </span>
        ))}
      </span>
    </div>
  );
});

const EventRow = memo(function EventRow({ e }: { e: SessionEvent }) {
  switch (e.kind) {
    case "user":
      return <div className="ml-auto max-w-[80%] rounded-[10px_10px_3px_10px] bg-sunk px-2.5 py-1.5 text-[13px]">{e.text}</div>;
    case "assistant":
      return (
        <div className="text-[13px] leading-normal">
          {e.paras.map((p, i) => (
            <p key={i} className="my-1">
              {p}
            </p>
          ))}
          {e.code && <pre className="m-0 rounded-md border border-line-2 bg-code px-2 py-1.5 font-mono text-[12px] whitespace-pre-wrap">{e.code}</pre>}
        </div>
      );
    case "tool":
      return (
        <div className="flex items-center gap-2 rounded-md border border-line-2 bg-chrome px-2 py-1 text-xs text-ink-2">
          <b className="text-ink">{e.name}</b>
          <span className="truncate font-mono">{e.input}</span>
          <span className="ml-auto text-ink-3">{e.secs} s · exit 0</span>
        </div>
      );
    case "edit":
      return (
        <div className="overflow-hidden rounded-md border border-line-2">
          <div className="bg-chrome px-2 py-1 text-xs text-ink-2">{e.path}</div>
          {e.lines.map((l, i) => (
            <DiffRow key={i} l={l} />
          ))}
        </div>
      );
  }
});

type Bench = { name: string; frames: number; seconds: number; fps: number; p50: number; p95: number; max: number; janky: number; mountMs: number };

/** Scrolls `el` from top to bottom at `pxPerFrame`, measuring the time between animation frames. */
function scrollBench(el: HTMLElement, pxPerFrame: number, maxSeconds: number): Promise<Omit<Bench, "name" | "mountMs">> {
  return new Promise((resolve) => {
    el.scrollTop = 0;
    const deltas: number[] = [];
    let last = performance.now();
    const start = last;
    const step = (now: number) => {
      deltas.push(now - last);
      last = now;
      el.scrollTop += pxPerFrame;
      const done = el.scrollTop + el.clientHeight >= el.scrollHeight - 1 || now - start > maxSeconds * 1000;
      if (!done) requestAnimationFrame(step);
      else {
        deltas.shift();
        const sorted = [...deltas].sort((a, b) => a - b);
        const q = (p: number) => sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))] ?? 0;
        const seconds = (now - start) / 1000;
        resolve({
          frames: deltas.length,
          seconds: Math.round(seconds * 10) / 10,
          fps: Math.round(deltas.length / seconds),
          p50: Math.round(q(0.5) * 10) / 10,
          p95: Math.round(q(0.95) * 10) / 10,
          max: Math.round((sorted[sorted.length - 1] ?? 0) * 10) / 10,
          janky: deltas.filter((d) => d > 50).length,
        });
      }
    };
    requestAnimationFrame(step);
  });
}

function VirtualDiff({ lines, onMount }: { lines: DiffLine[]; onMount: (el: HTMLDivElement, ms: number) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const t0 = useRef(performance.now());
  const v = useVirtualizer({ count: lines.length, getScrollElement: () => ref.current, estimateSize: () => 20, overscan: 10, useFlushSync: false });
  useEffect(() => {
    requestAnimationFrame(() => onMount(ref.current!, performance.now() - t0.current));
  }, [onMount]);
  return (
    <div ref={ref} className="h-full overflow-auto">
      <div style={{ height: v.getTotalSize(), position: "relative" }}>
        {v.getVirtualItems().map((it) => (
          <div key={it.key} style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${it.start}px)` }}>
            <DiffRow l={lines[it.index]} />
          </div>
        ))}
      </div>
    </div>
  );
}

function VirtualSession({ events, onMount }: { events: SessionEvent[]; onMount: (el: HTMLDivElement, ms: number) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const t0 = useRef(performance.now());
  const v = useVirtualizer({ count: events.length, getScrollElement: () => ref.current, estimateSize: () => 60, overscan: 6, useFlushSync: false });
  useEffect(() => {
    requestAnimationFrame(() => onMount(ref.current!, performance.now() - t0.current));
  }, [onMount]);
  return (
    <div ref={ref} className="h-full overflow-auto px-3">
      <div style={{ height: v.getTotalSize(), position: "relative" }}>
        {v.getVirtualItems().map((it) => (
          <div
            key={it.key}
            data-index={it.index}
            ref={v.measureElement}
            className="flex flex-col py-1.5"
            style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${it.start}px)` }}
          >
            <EventRow e={events[it.index]} />
          </div>
        ))}
      </div>
    </div>
  );
}

function PlainDiff({ lines, onMount }: { lines: DiffLine[]; onMount: (el: HTMLDivElement, ms: number) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const t0 = useRef(performance.now());
  useEffect(() => {
    requestAnimationFrame(() => onMount(ref.current!, performance.now() - t0.current));
  }, [onMount]);
  return (
    <div ref={ref} className="h-full overflow-auto">
      {lines.map((l, i) => (
        <DiffRow key={i} l={l} />
      ))}
    </div>
  );
}

/** Pure JavaScript work, to compare engines without layout or painting. */
function jsBench() {
  const t = (f: () => void) => {
    const s = performance.now();
    f();
    return Math.round(performance.now() - s);
  };
  const fib = (n: number): number => (n < 2 ? n : fib(n - 1) + fib(n - 2));
  return {
    fib30Ms: t(() => void fib(30)),
    sort300kMs: t(() => void Array.from({ length: 300_000 }, (_, i) => (i * 7919) % 100_003).sort((a, b) => a - b)),
    objects1mMs: t(() => {
      const a = [];
      for (let i = 0; i < 1_000_000; i++) a.push({ i, s: `row ${i}` });
      JSON.stringify(a.slice(0, 50_000));
    }),
  };
}

type Mode = "diff" | "session" | "plain-diff";

export function PerfView() {
  const diff = useMemo(() => makeDiff(5000), []);
  const session = useMemo(() => makeSession(2000), []);
  const [mode, setMode] = useState<Mode | null>(null);
  const [results, setResults] = useState<Bench[]>([]);
  const pending = useRef<((el: HTMLDivElement, ms: number) => void) | null>(null);
  const auto = new URLSearchParams(location.search).get("auto") === "1";

  const run = (m: Mode) =>
    new Promise<Bench>((resolve) => {
      setMode(null);
      setTimeout(() => {
        pending.current = async (el, mountMs) => {
          await new Promise((r) => setTimeout(r, 300));
          const b = await scrollBench(el, m === "session" ? 30 : 40, 20);
          const res = { name: m, mountMs: Math.round(mountMs), ...b };
          setResults((rs) => [...rs, res]);
          resolve(res);
        };
        setMode(m);
      }, 50);
    });

  const onMount = useMemo(() => (el: HTMLDivElement, ms: number) => pending.current?.(el, ms), []);

  useEffect(() => {
    if (!auto) return;
    (async () => {
      await new Promise((r) => setTimeout(r, 500));
      logToDaemon(`perf: js ${JSON.stringify(jsBench())}`);
      const out: Bench[] = [];
      for (const m of ["diff", "session", "plain-diff"] as Mode[]) out.push(await run(m));
      logToDaemon(`perf: ${navigator.userAgent}`);
      for (const b of out) logToDaemon(`perf: ${JSON.stringify(b)}`);
      logToDaemon("perf: done");
    })();
  }, []);

  return (
    <div className="flex h-full flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-line p-3">
        <b>WebKitGTK check</b>
        <button className="btn btn-sm" onClick={() => void run("diff")}>
          Virtualized diff (5,000 lines)
        </button>
        <button className="btn btn-sm" onClick={() => void run("session")}>
          Virtualized session (2,000 events)
        </button>
        <button className="btn btn-sm" onClick={() => void run("plain-diff")}>
          Plain diff (5,000 lines, no virtualization)
        </button>
        <span className="font-mono text-xs text-ink-2">{navigator.userAgent.match(/AppleWebKit\/[\d.]+|Chrome\/[\d.]+/g)?.join(" ")}</span>
      </div>
      <div className="grid min-h-0 flex-1 grid-cols-[1fr_420px]">
        <div className="min-h-0 border-r border-line">
          {mode === "diff" && <VirtualDiff lines={diff} onMount={onMount} />}
          {mode === "session" && <VirtualSession events={session} onMount={onMount} />}
          {mode === "plain-diff" && <PlainDiff lines={diff} onMount={onMount} />}
        </div>
        <div className="overflow-auto p-3 font-mono text-xs">
          {results.map((r, i) => (
            <pre key={i} className="m-0 mb-2 whitespace-pre-wrap">
              {JSON.stringify(r, null, 1)}
            </pre>
          ))}
        </div>
      </div>
    </div>
  );
}
