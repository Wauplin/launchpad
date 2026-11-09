// Preset picker: a checklist of the configured presets (shown in the Add context menu of the
// composer and the message box, or behind a labeled button in Send later), and removable chips for
// the chosen ones. Message texts are appended with `withPresets` (lib/presets.ts), like at launch.

import { useNavigate } from "@tanstack/react-router";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { usePhone } from "@/lib/phone";
import { useSharedSettings } from "@/lib/queries";
import { availablePresets, type PresetSurface } from "@/lib/presets";
import type { Preset } from "@/gen/Preset";

const NONE: Preset[] = [];

export function usePresets(surface?: PresetSurface): Preset[] {
  const presets = useSharedSettings().data?.presets ?? NONE;
  return surface ? availablePresets(presets, surface) : presets;
}

/** A labeled `Presets` button opening the checklist, for forms like Send later. */
export function PresetButton({ selected, onChange, disabled, surface = "agent" }: { selected: string[]; onChange: (ids: string[]) => void; disabled?: boolean; surface?: PresetSurface }) {
  const presets = usePresets(surface);
  const phone = usePhone();
  const count = presets.filter((p) => selected.includes(p.id)).length;
  return (
    <Popover
      align="left"
      viewport={phone}
      width={phone ? 340 : 320}
      label="Presets"
      className="flex shrink-0"
      trigger={({ open, toggle }) => (
        <button className="btn btn-sm" aria-haspopup="dialog" aria-expanded={open} disabled={disabled} onClick={toggle}>
          <Icon name="preset" size={12} />
          {count ? `Presets · ${count}` : "Add presets"}
        </button>
      )}
    >
      {(close) => <PresetChecklist presets={presets} selected={selected} onChange={onChange} close={close} />}
    </Popover>
  );
}

/** The checklist of presets inside a popover: rows to toggle, then Edit presets, Clear and Done. */
export function PresetChecklist({ presets, selected, onChange, close }: { presets: Preset[]; selected: string[]; onChange: (ids: string[]) => void; close: () => void }) {
  const navigate = useNavigate();
  const count = presets.filter((p) => selected.includes(p.id)).length;
  const toggle = (id: string) => onChange(selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id]);
  return (
    <>
      <div className="px-[9px] pt-1 pb-1.5 text-[12px] text-ink-3">Added after your text, in settings order.</div>
      {presets.map((p) => {
        const on = selected.includes(p.id);
        return (
          <button
            key={p.id}
            role="menuitemcheckbox"
            aria-checked={on}
            onClick={() => toggle(p.id)}
            className={`message-preset-row grid grid-cols-[18px_minmax(0,1fr)] items-start gap-x-2 rounded-control px-[9px] py-[7px] text-left text-[13px] hover:bg-select ${on ? "bg-select" : ""}`}
          >
            <span className={`mt-px grid h-[18px] w-[18px] place-items-center rounded border ${on ? "border-ink bg-ink text-paper" : "border-line bg-paper"}`}>{on && <Icon name="check" size={12} />}</span>
            <span className="min-w-0">
              <span className={`block truncate ${on ? "font-semibold" : ""}`}>{p.label}</span>
              <span className="line-clamp-2 text-[12px] text-ink-3">{p.text.trim() || "Empty text: nothing is added."}</span>
            </span>
          </button>
        );
      })}
      {presets.length === 0 && <div className="px-[9px] py-2 text-[12px] text-ink-3">No preset yet. Presets are reusable instructions defined in Settings.</div>}
      <div className="mt-1 flex items-center gap-2 border-t border-line-2 px-[5px] pt-1.5">
        <button className="btn btn-sm" onClick={() => { close(); void navigate({ to: "/settings/presets" }); }}>{presets.length ? "Edit presets" : "Create a preset"}</button>
        {count > 0 && <button className="btn btn-sm" onClick={() => onChange([])}>Clear</button>}
        <button className="btn btn-sm btn-primary ml-auto" onClick={close}>Done</button>
      </div>
    </>
  );
}

/** The chosen presets as chips; clicking one removes it. Renders nothing when none is chosen. */
export function PresetChips({ selected, onChange, disabled, className = "", surface = "agent" }: { selected: string[]; onChange: (ids: string[]) => void; disabled?: boolean; className?: string; surface?: PresetSurface }) {
  const chosen = usePresets(surface).filter((p) => selected.includes(p.id));
  if (!chosen.length) return null;
  return (
    <div className={`flex flex-wrap items-center gap-1.5 ${className}`}>
      <span className="text-[12px] text-ink-3">Presets</span>
      {chosen.map((p) => (
        <button
          key={p.id}
          className="message-preset-chip inline-flex h-6 max-w-full items-center gap-1 rounded-full border border-teal bg-teal-soft pr-1.5 pl-2.5 text-[12px] font-semibold text-teal-ink hover:bg-paper disabled:opacity-60"
          title={`${p.text.trim()}\n\nClick to remove`}
          aria-label={`Remove preset ${p.label}`}
          disabled={disabled}
          onClick={() => onChange(selected.filter((x) => x !== p.id))}
        >
          <span className="truncate">{p.label}</span>
          <Icon name="x" size={12} />
        </button>
      ))}
    </div>
  );
}
