import type { Label } from "./types";

export function LabelChip({ label }: { label: Label }) {
  return (
    <span
      className="inline-block max-w-[160px] truncate rounded border border-line bg-paper px-2 py-0.5 text-xs text-ink"
      style={{ borderLeft: `4px solid #${label.color}` }}
      title={label.name}
    >
      {label.name}
    </span>
  );
}
