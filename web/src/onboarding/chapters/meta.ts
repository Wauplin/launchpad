// Chapter titles and shown times, without their steps: Settings → Onboarding lists them in real mode without
// loading the demo. Keep in the order of `chapters/index.ts`.

export type ChapterMeta = { id: string; title: string; time: string; essentials: boolean };

export const CHAPTER_META: ChapterMeta[] = [
  { id: "layout", title: "Welcome & layout", time: "0:30", essentials: true },
  { id: "launch", title: "Launch an agent", time: "1:00", essentials: true },
  { id: "answering", title: "Agents & answering", time: "0:45", essentials: true },
  { id: "review", title: "Review the work", time: "0:45", essentials: true },
  { id: "github", title: "GitHub inbox & PRs", time: "1:15", essentials: false },
  { id: "ask", title: "Ask Launchpad", time: "1:15", essentials: false },
  { id: "terminals", title: "Terminals", time: "1:00", essentials: false },
  { id: "folders", title: "Folders & workspaces", time: "1:00", essentials: false },
  { id: "automation", title: "Start later, crons & quotas", time: "1:15", essentials: false },
  { id: "settings", title: "Profiles, presets & devices", time: "1:15", essentials: false },
];

export const meta = (id: string) => CHAPTER_META.find((c) => c.id === id)!;
