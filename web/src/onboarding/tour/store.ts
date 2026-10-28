// Tour state. Each chapter starts with a full page load of the demo (`restartDemo`), so the world, the scenario
// player, the query cache and every UI store start from the same state; the tab's `window.name` carries
// `<track>:<chapter>` across that reload. Nothing here is persisted except chapters seen (onboarding record).

import { create } from "zustand";
import { api } from "@/lib/api";
import { exitDemo, restartDemo } from "@/demo/mode";
import { CHAPTERS, chapter, trackChapters } from "../chapters";
import type { Track } from "./types";

type Mode = "card" | "recap" | "explore";

type TourState = {
  track: Track | null;
  chapter: string | null;
  step: number;
  mode: Mode;
  next: () => void;
  back: () => void;
  goTo: (step: number) => void;
  /** Ends the chapter: the recap card. */
  finish: () => void;
  explore: () => void;
  resume: () => void;
};

/** Parses `window.name` after `lp-demo:`: "" explores freely, `<track>:<chapter>` plays a chapter. */
export function parseEntry(entry: string): { track: Track | null; chapter: string | null } {
  const [track, id] = entry.split(":");
  if ((track === "essentials" || track === "full") && id && chapter(id)) return { track, chapter: id };
  if (track === "essentials" || track === "full") return { track, chapter: trackChapters(track)[0].id };
  if (chapter(entry)) return { track: null, chapter: entry };
  return { track: null, chapter: null };
}

export const useTour = create<TourState>((set, get) => ({
  track: null,
  chapter: null,
  step: 0,
  mode: "explore",
  next: () => {
    const c = chapter(get().chapter ?? "");
    if (!c) return;
    if (get().step + 1 < c.steps.length) set({ step: get().step + 1 });
    else get().finish();
  },
  back: () => set({ step: Math.max(0, get().step - 1), mode: "card" }),
  goTo: (step) => set({ step, mode: "card" }),
  finish: () => {
    const id = get().chapter;
    if (id) void api.patch("/api/onboarding", { chapters_seen: [id] }).catch(() => {});
    set({ mode: "recap" });
  },
  explore: () => set({ mode: "explore" }),
  resume: () => set({ mode: get().chapter ? "card" : "explore" }),
}));

/** Starts `id` from its own starting state (a page reload). Without a track, only that chapter plays. */
export function playChapter(id: string, track: Track | null) {
  const c = chapter(id);
  if (!c) return;
  restartDemo(track ? `${track}:${id}` : id, c.steps[0]?.route ?? "/agents");
}

/** The chapter after the current one in its track, if any. */
export function nextChapter(): string | null {
  const { track, chapter: id } = useTour.getState();
  const list = track ? trackChapters(track) : [];
  const i = list.findIndex((c) => c.id === id);
  return i >= 0 && i + 1 < list.length ? list[i + 1].id : null;
}

/** Skips to the next chapter of the track; the last one ends the tour. */
export function skipChapter() {
  const n = nextChapter();
  if (n) playChapter(n, useTour.getState().track);
  else useTour.getState().finish();
}

/** Leaves the demo for the setup checklist. */
export const setUpThisMachine = () => exitDemo("/setup");
export const exitTour = () => exitDemo();

export { CHAPTERS };
