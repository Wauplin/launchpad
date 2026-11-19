// Widths of the laptop layout's resizable columns (the sidebar, list columns): one entry per column, shared by
// every page that shows that column. Pure, so the clamp is tested without a browser.

export type ColumnName = "sidebar" | "agents-list" | "chats-list" | "knowledge-notes" | "knowledge-changes" | "files-tree" | "review-files";

export type ColumnSpec = { initial: number; min: number; max: number; /** Share of the window it may take at most. */ maxRatio: number };

export const COLUMNS: Record<ColumnName, ColumnSpec> = {
  sidebar: { initial: 200, min: 160, max: 360, maxRatio: 0.3 },
  "agents-list": { initial: 280, min: 220, max: 560, maxRatio: 0.4 },
  "chats-list": { initial: 300, min: 220, max: 560, maxRatio: 0.4 },
  "knowledge-notes": { initial: 300, min: 220, max: 560, maxRatio: 0.4 },
  "knowledge-changes": { initial: 380, min: 260, max: 640, maxRatio: 0.45 },
  "files-tree": { initial: 284, min: 200, max: 560, maxRatio: 0.4 },
  "review-files": { initial: 240, min: 180, max: 480, maxRatio: 0.35 },
};

/** The largest width a column may have in a window `viewport` px wide (CSS px at the current zoom), never under its min. */
export const columnMax = (spec: ColumnSpec, viewport: number) => Math.max(spec.min, Math.min(spec.max, viewport > 0 ? viewport * spec.maxRatio : spec.max));

/** A stored or dragged width brought within the column's bounds; anything unreadable is the default width. */
export function clampColumn(spec: ColumnSpec, value: unknown, viewport: number): number {
  const width = typeof value === "number" && Number.isFinite(value) && value > 0 ? value : spec.initial;
  return Math.round(Math.max(spec.min, Math.min(columnMax(spec, viewport), width)));
}

/** Only known columns with usable numbers survive a read of browser storage. */
export function parseWidths(raw: string | null): Partial<Record<ColumnName, number>> {
  let data: unknown;
  try { data = JSON.parse(raw ?? "{}"); } catch { return {}; }
  if (!data || typeof data !== "object" || Array.isArray(data)) return {};
  const out: Partial<Record<ColumnName, number>> = {};
  for (const [name, value] of Object.entries(data)) {
    if (name in COLUMNS && typeof value === "number" && Number.isFinite(value) && value > 0) out[name as ColumnName] = value;
  }
  return out;
}
