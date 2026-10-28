// The tour overlay (lazy chunk, demo mode only): a spotlight around one element, four transparent blockers that
// keep the rest of the app from being clicked, and a step card. One element with a box-shadow draws the dim
// (no backdrop blur), positions refresh on a short interval and on resize, so it stays light in WebKitGTK and
// follows zoom, theme and virtualized lists. Every control is a visible button: no keyboard shortcut.

import { useRouter } from "@tanstack/react-router";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { Icon } from "@/components/Icon";
import { demoChapter } from "@/demo/mode";
import { useOnboarding } from "../data";
import { CHAPTERS, chapter, trackChapters, trackTime } from "../chapters";
import { activate, setTourRouter } from "../chapters/helpers";
import { exitTour, nextChapter, parseEntry, playChapter, setUpThisMachine, skipChapter, useTour } from "./store";
import type { Step } from "./types";
import { getSpeed, isPlaying, setSpeed, skipToEnd } from "@/demo/player";

type Box = { top: number; left: number; width: number; height: number };
const PAD = 6;
const CARD_W = 340;

/** CSS zoom of the page (lib/zoom.ts sets it on <html>): one style pixel covers `zoom()` window pixels. */
const zoom = () => Number(document.documentElement.style.zoom || getComputedStyle(document.documentElement).zoom || 1) || 1;

/** Rect pixels per style pixel. Engines disagree on whether getBoundingClientRect under a root CSS zoom reports
 *  zoomed pixels (Chrome) or style pixels (WebKitGTK), so measure a fixed 1000 px probe instead of guessing. */
let probe: HTMLDivElement | null = null;
function rectScale(): number {
  if (!probe?.isConnected) {
    probe = document.createElement("div");
    probe.setAttribute("aria-hidden", "true");
    probe.style.cssText = "position:fixed;top:0;left:0;width:1000px;height:0;visibility:hidden;pointer-events:none";
    document.body.appendChild(probe);
  }
  return probe.getBoundingClientRect().width / 1000 || 1;
}

export function anchorElement(step: Step | undefined): HTMLElement | null {
  if (!step?.anchor) return null;
  const all = document.querySelectorAll<HTMLElement>(`[data-tour="${step.anchor}"]${step.anchorId ? `[data-tour-id="${step.anchorId}"]` : ""}`);
  // The first visible match: hidden panes keep their elements mounted.
  for (const el of all) if (el.getClientRects().length && el.offsetParent !== null) return el;
  return null;
}

function measure(el: HTMLElement | null): Box | null {
  if (!el) return null;
  const r = el.getBoundingClientRect();
  const z = rectScale();
  return { top: r.top / z - PAD, left: r.left / z - PAD, width: r.width / z + 2 * PAD, height: r.height / z + 2 * PAD };
}

export function TourRoot() {
  const router = useRouter();
  setTourRouter(router);
  const { chapter: id, step: index, mode, track } = useTour();
  const c = chapter(id ?? "");
  const step = mode === "card" ? c?.steps[index] : undefined;
  const [box, setBox] = useState<Box | null>(null);
  const [view, setView] = useState({ w: window.innerWidth, h: window.innerHeight });
  const [ready, setReady] = useState(false);

  // Boot: the chapter in window.name, prepared once.
  useEffect(() => {
    const entry = parseEntry(demoChapter());
    useTour.setState({ track: entry.track, chapter: entry.chapter, step: 0, mode: entry.chapter ? "card" : "explore" });
    const c = chapter(entry.chapter ?? "");
    void Promise.resolve(c?.setup?.()).finally(() => setReady(true));
  }, []);

  // Each step: open its route, run its preparation.
  useEffect(() => {
    if (!ready || !step) return;
    const here = router.state.location.pathname + (router.state.location.searchStr ?? "");
    if (step.route && here !== step.route && router.state.location.pathname !== step.route) void router.navigate({ to: step.route } as never);
    void Promise.resolve(step.enter?.()).catch(() => {});
  }, [ready, step, router]);

  // Follow the anchor: on resize, and every 250 ms (lists scroll, panels open, zoom or theme change).
  useLayoutEffect(() => {
    if (!step) { setBox(null); return; }
    const update = () => {
      setView({ w: window.innerWidth / zoom(), h: window.innerHeight / zoom() });
      const next = measure(anchorElement(step));
      setBox((b) => (b && next && Math.abs(b.top - next.top) + Math.abs(b.left - next.left) + Math.abs(b.width - next.width) + Math.abs(b.height - next.height) < 1 ? b : next));
    };
    update();
    const timer = setInterval(update, 250);
    window.addEventListener("resize", update);
    return () => { clearInterval(timer); window.removeEventListener("resize", update); };
  }, [step]);

  // Try it: the step advances when its condition holds, or on a click inside the anchor.
  useEffect(() => {
    if (!step?.tryIt) return;
    const { done } = step.tryIt;
    let advanced = false;
    const advance = () => { if (!advanced) { advanced = true; setTimeout(() => useTour.getState().next(), 500); } };
    if (done) {
      const timer = setInterval(() => { try { if (done()) advance(); } catch { /* Not ready yet. */ } }, 300);
      return () => clearInterval(timer);
    }
    const click = (e: MouseEvent) => { if (anchorElement(step)?.contains(e.target as Node)) advance(); };
    document.addEventListener("click", click, true);
    return () => document.removeEventListener("click", click, true);
  }, [step]);

  if (!ready) return null;
  if (mode === "explore") return <ExplorePill />;
  if (mode === "recap" && c) return <Recap />;
  if (!c || !step) return null;
  const tryIt = !!step.tryIt;
  return (
    <div className="lp-tour" aria-live="polite">
      <Blockers box={box} view={view} open={tryIt} />
      {box && <div className="lp-tour-spot" style={{ top: box.top, left: box.left, width: box.width, height: box.height }} />}
      {!box && <div className="lp-tour-dim" />}
      <StepCard key={step.id} step={step} index={index} total={c.steps.length} title={c.title} box={box} view={view} track={track} />
    </div>
  );
}

/** Four transparent panels around the spotlight. A Try it step leaves the spotlight itself clickable. */
function Blockers({ box, view, open }: { box: Box | null; view: { w: number; h: number }; open: boolean }) {
  if (!box || !open) return <div className="lp-tour-block" style={{ inset: 0 }} />;
  const right = box.left + box.width, bottom = box.top + box.height;
  const panels: CSSProperties[] = [
    { top: 0, left: 0, width: view.w, height: Math.max(0, box.top) },
    { top: bottom, left: 0, width: view.w, height: Math.max(0, view.h - bottom) },
    { top: box.top, left: 0, width: Math.max(0, box.left), height: box.height },
    { top: box.top, left: right, width: Math.max(0, view.w - right), height: box.height },
  ];
  return <>{panels.map((s, i) => <div key={i} className="lp-tour-block" style={s} />)}</>;
}

function place(box: Box | null, view: { w: number; h: number }, height: number): CSSProperties {
  const clampTop = (t: number) => Math.max(8, Math.min(t, view.h - height - 8));
  const clampLeft = (l: number) => Math.max(8, Math.min(l, view.w - CARD_W - 8));
  if (!box) return { top: clampTop(view.h / 2 - height / 2), left: clampLeft(view.w / 2 - CARD_W / 2) };
  if (box.left + box.width + 12 + CARD_W < view.w) return { top: clampTop(box.top), left: box.left + box.width + 12 };
  if (box.left - 12 - CARD_W > 0) return { top: clampTop(box.top), left: box.left - 12 - CARD_W };
  if (box.top + box.height + 12 + height < view.h) return { top: box.top + box.height + 12, left: clampLeft(box.left) };
  if (box.top - 12 - height > 0) return { top: box.top - 12 - height, left: clampLeft(box.left) };
  return { top: clampTop(view.h - height - 16), left: clampLeft(view.w - CARD_W - 16) };
}

function StepCard({ step, index, total, title, box, view, track }: { step: Step; index: number; total: number; title: string; box: Box | null; view: { w: number; h: number }; track: string | null }) {
  const { next, back } = useTour();
  const ref = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(220);
  const [menu, setMenu] = useState(false);
  const [busy, setBusy] = useState(false);
  useLayoutEffect(() => { if (ref.current) setHeight(ref.current.offsetHeight); }, [step, menu]);
  // Focus the card's main button, so screen readers and keyboard users start there.
  useEffect(() => { ref.current?.querySelector<HTMLButtonElement>("[data-primary]")?.focus({ preventScroll: true }); }, [step]);
  const doIt = async () => {
    setBusy(true);
    try {
      if (step.tryIt?.doIt) await step.tryIt.doIt();
      else activate(anchorElement(step));
      if (!step.tryIt?.done) setTimeout(() => useTour.getState().next(), 400);
    } finally { setBusy(false); }
  };
  return (
    <div ref={ref} role="dialog" aria-labelledby="lp-tour-title" className="lp-tour-card" style={{ width: CARD_W, ...place(box, view, height) }}>
      <div className="flex items-center gap-2 text-[11px] font-semibold tracking-[0.04em] text-demo uppercase">
        <span className="min-w-0 flex-1 truncate">{title}</span>
        <span className="text-ink-3">Step {index + 1} of {total}</span>
      </div>
      <div className="mt-1 flex gap-1" aria-hidden>{Array.from({ length: total }, (_, i) => <span key={i} className={`h-1 flex-1 rounded-full ${i <= index ? "bg-demo" : "bg-line-2"}`} />)}</div>
      <h2 id="lp-tour-title" className="mt-2.5 mb-1 text-[15px] font-semibold">{step.title}</h2>
      <p className="m-0 text-[13px] text-ink-2">{step.body}</p>
      {step.run && <RunControls run={step.run} />}
      {step.tryIt && <p className="mt-2 mb-0 flex items-start gap-1.5 text-[13px] font-semibold text-demo"><Icon name="play" size={12} className="mt-1 flex-none" />Try it: {step.tryIt.text}</p>}
      <div className="mt-3 flex items-center gap-2">
        {index > 0 && <button type="button" className="btn btn-sm" onClick={back}>Back</button>}
        <span className="flex-1" />
        {step.tryIt
          ? <button type="button" data-primary className="btn btn-sm btn-demo" disabled={busy} onClick={() => void doIt()}>Do it for me</button>
          : <button type="button" data-primary className="btn btn-sm btn-demo" onClick={next}>{index + 1 === total ? "Finish chapter" : "Next"}</button>}
      </div>
      <div className="mt-2.5 flex flex-wrap gap-x-3 gap-y-1 border-t border-line-2 pt-2 text-[12px]">
        <button type="button" className="lp-tour-link" aria-expanded={menu} onClick={() => setMenu(!menu)}>Chapters</button>
        {track && <button type="button" className="lp-tour-link" onClick={skipChapter}>Skip chapter</button>}
        <button type="button" className="lp-tour-link" onClick={() => useTour.getState().explore()}>Explore freely</button>
        <button type="button" className="lp-tour-link ml-auto" onClick={exitTour}>Exit tour</button>
      </div>
      {menu && <ChapterList track={track === "essentials" ? "essentials" : "full"} />}
    </div>
  );
}

/** 1×, 2× and Skip to end for the prerecorded run a step watches. */
function RunControls({ run }: { run: () => string | null }) {
  const [, tick] = useState(0);
  useEffect(() => { const t = setInterval(() => tick((n) => n + 1), 400); return () => clearInterval(t); }, []);
  const id = run();
  if (!id || !isPlaying(id)) return null;
  const speed = getSpeed();
  return (
    <div className="mt-2 flex items-center gap-1.5 text-[12px] text-ink-3" role="group" aria-label="Replay speed">
      <span className="lp-spin" />Replaying
      <span className="flex-1" />
      <button type="button" className="btn btn-sm" aria-pressed={speed === 1} onClick={() => { setSpeed(1); tick((n) => n + 1); }}>1×</button>
      <button type="button" className="btn btn-sm" aria-pressed={speed === 2} onClick={() => { setSpeed(2); tick((n) => n + 1); }}>2×</button>
      <button type="button" className="btn btn-sm" onClick={() => skipToEnd(id)}>Skip to end</button>
    </div>
  );
}

/** Chapters of a track with check marks from the onboarding record; each starts from its own state. */
export function ChapterList({ track }: { track: "essentials" | "full" }) {
  const seen = useOnboarding().data?.chapters_seen ?? [];
  const current = useTour((s) => s.chapter);
  return (
    <ol className="m-0 mt-2 grid list-none gap-0.5 p-0">
      {trackChapters(track).map((c, i) => (
        <li key={c.id}>
          <button type="button" className="flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left text-[13px] hover:bg-sunk" aria-current={c.id === current ? "step" : undefined} onClick={() => playChapter(c.id, track)}>
            <span className="w-4 text-center text-demo">{seen.includes(c.id) ? <Icon name="check" size={12} /> : i + 1}</span>
            <span className={`min-w-0 flex-1 truncate ${c.id === current ? "font-semibold" : ""}`}>{c.title}</span>
            <span className="text-[12px] text-ink-3">{c.time}</span>
          </button>
        </li>
      ))}
    </ol>
  );
}

function Recap() {
  const { chapter: id, track } = useTour();
  const c = chapter(id ?? "")!;
  const following = chapter(nextChapter() ?? "");
  const [menu, setMenu] = useState(false);
  return (
    <div className="lp-tour">
      <div className="lp-tour-dim" />
      <div role="dialog" aria-labelledby="lp-tour-recap" className="lp-tour-card" style={{ width: 380, top: "50%", left: "50%", transform: "translate(-50%, -50%)" }}>
        <div className="text-[11px] font-semibold tracking-[0.04em] text-demo uppercase">Chapter done</div>
        <h2 id="lp-tour-recap" className="mt-1 mb-2 text-[15px] font-semibold">{c.title}</h2>
        <p className="m-0 text-[13px] text-ink-2">You learned:</p>
        <ul className="mt-1 mb-0 pl-5 text-[13px]">{c.learned.map((l) => <li key={l}>{l}</li>)}</ul>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          {following
            ? <button type="button" data-primary className="btn btn-sm btn-demo" onClick={() => playChapter(following.id, track)}>Next: {following.title} ({following.time.replace(/^0:/, "0:")})</button>
            : <button type="button" data-primary className="btn btn-sm btn-demo" onClick={setUpThisMachine}>Set up this machine →</button>}
          {following && <button type="button" className="btn btn-sm" onClick={setUpThisMachine}>Set up this machine →</button>}
          <button type="button" className="btn btn-sm" onClick={() => playChapter(c.id, track)}>Replay</button>
        </div>
        {!following && track === "essentials" && <p className="mt-2 mb-0 text-[12px] text-ink-3">The full tour ({trackTime("full")}) adds GitHub, Ask, terminals, workspaces, automation and settings.</p>}
        <div className="mt-2.5 flex flex-wrap gap-x-3 border-t border-line-2 pt-2 text-[12px]">
          <button type="button" className="lp-tour-link" aria-expanded={menu} onClick={() => setMenu(!menu)}>Chapters</button>
          <button type="button" className="lp-tour-link" onClick={() => useTour.getState().explore()}>Explore freely</button>
          <button type="button" className="lp-tour-link ml-auto" onClick={exitTour}>Exit tour</button>
        </div>
        {menu && <ChapterList track={track === "essentials" ? "essentials" : "full"} />}
      </div>
    </div>
  );
}

/** Explore freely: the demo stays usable; a floating pill offers chapters and the way out. */
function ExplorePill() {
  const [menu, setMenu] = useState(false);
  const { chapter: id } = useTour();
  return (
    <div className="lp-tour-pill" role="region" aria-label="Demo tour">
      {menu && <div className="lp-tour-card absolute right-0 bottom-[calc(100%+8px)] w-[320px]"><ChapterList track="full" /></div>}
      <span className="font-semibold text-demo">Demo</span>
      {id && <button type="button" className="btn btn-sm" onClick={() => useTour.getState().resume()}>Back to the tour</button>}
      <button type="button" className="btn btn-sm" aria-expanded={menu} onClick={() => setMenu(!menu)}>Start a chapter</button>
      <button type="button" className="btn btn-sm" onClick={exitTour}>Exit demo</button>
    </div>
  );
}

export { CHAPTERS };
