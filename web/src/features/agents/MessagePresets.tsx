// Preset picker for messages sent to an existing session: an icon button opening a checklist of
// the configured presets, and removable chips for the chosen ones. The texts are appended to the
// message with `withPresets` (lib/presets.ts), like the composer does at launch.

import { useNavigate } from "@tanstack/react-router";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { usePhone } from "@/lib/phone";
import { useConfig } from "@/lib/queries";
import type { Preset } from "@/gen/Preset";

const NONE: Preset[] = [];

export function usePresets(): Preset[] {
  return useConfig().data?.config.presets ?? NONE;
}

/** An icon button for message boxes, or a labeled `Presets` button for forms with `labeled`. */
export function PresetButton({ selected, onChange, disabled, labeled = false }: { selected: string[]; onChange: (ids: string[]) => void; disabled?: boolean; labeled?: boolean }) {
  const presets = usePresets();
  const phone = usePhone();
  const navigate = useNavigate();
  const count = presets.filter((p) => selected.includes(p.id)).length;
  const toggle = (id: string) => onChange(selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id]);
  return (
    <Popover
      up={!labeled}
      align={labeled ? "left" : "right"}
      viewport={phone}
      width={phone ? 340 : 320}
      label="Presets"
      className="flex shrink-0"
      trigger={({ open, toggle: toggleOpen }) => labeled ? (
        <button className="btn btn-sm" aria-haspopup="dialog" aria-expanded={open} disabled={disabled} onClick={toggleOpen}>
          <Icon name="preset" size={13} />
          {count ? `Presets · ${count}` : "Add presets"}
        </button>
      ) : (
        <button
          className={`iconbtn relative ${count ? "text-teal-ink" : ""}`}
          title={count ? `${count} preset${count > 1 ? "s" : ""} added to this message` : "Add presets to this message"}
          aria-label="Presets"
          aria-haspopup="dialog"
          aria-expanded={open}
          disabled={disabled}
          onClick={toggleOpen}
        >
          <Icon name="preset" size={14} />
          {count > 0 && <span className="absolute -top-0.5 -right-0.5 grid h-3.5 min-w-3.5 place-items-center rounded-full bg-teal px-[3px] text-[9.5px] leading-none font-semibold text-paper">{count}</span>}
        </button>
      )}
    >
      {(close) => (
        <>
          <div className="px-[9px] pt-1 pb-1.5 text-[11.5px] text-ink-3">Added after your message, in settings order.</div>
          {presets.map((p) => {
            const on = selected.includes(p.id);
            return (
              <button
                key={p.id}
                role="menuitemcheckbox"
                aria-checked={on}
                onClick={() => toggle(p.id)}
                className={`message-preset-row grid grid-cols-[18px_minmax(0,1fr)] items-start gap-x-[9px] rounded-md px-[9px] py-[7px] text-left text-[12.5px] hover:bg-select ${on ? "bg-select" : ""}`}
              >
                <span className={`mt-px grid h-[18px] w-[18px] place-items-center rounded border ${on ? "border-ink bg-ink text-paper" : "border-line bg-paper"}`}>{on && <Icon name="check" size={12} />}</span>
                <span className="min-w-0">
                  <span className={`block truncate ${on ? "font-semibold" : ""}`}>{p.label}</span>
                  <span className="line-clamp-2 text-[11.5px] text-ink-3">{p.text.trim() || "Empty text: nothing is added."}</span>
                </span>
              </button>
            );
          })}
          {presets.length === 0 && <div className="px-[9px] py-2 text-xs text-ink-3">No preset yet. Presets are reusable instructions defined in Settings.</div>}
          <div className="mt-1 flex items-center gap-2 border-t border-line-2 px-[5px] pt-1.5">
            <button className="btn btn-sm" onClick={() => { close(); void navigate({ to: "/settings", hash: "presets" }); }}>{presets.length ? "Edit presets" : "Create a preset"}</button>
            {count > 0 && <button className="btn btn-sm" onClick={() => onChange([])}>Clear</button>}
            <button className="btn btn-sm btn-primary ml-auto" onClick={close}>Done</button>
          </div>
        </>
      )}
    </Popover>
  );
}

/** The chosen presets as chips; clicking one removes it. Renders nothing when none is chosen. */
export function PresetChips({ selected, onChange, disabled, className = "" }: { selected: string[]; onChange: (ids: string[]) => void; disabled?: boolean; className?: string }) {
  const chosen = usePresets().filter((p) => selected.includes(p.id));
  if (!chosen.length) return null;
  return (
    <div className={`flex flex-wrap items-center gap-1.5 ${className}`}>
      <span className="text-[11.5px] text-ink-3">Presets</span>
      {chosen.map((p) => (
        <button
          key={p.id}
          className="message-preset-chip inline-flex h-6 max-w-full items-center gap-1 rounded-full border border-teal bg-teal-soft pr-1.5 pl-2.5 text-[11.5px] font-semibold text-teal-ink hover:bg-paper disabled:opacity-60"
          title={`${p.text.trim()}\n\nClick to remove`}
          aria-label={`Remove preset ${p.label}`}
          disabled={disabled}
          onClick={() => onChange(selected.filter((x) => x !== p.id))}
        >
          <span className="truncate">{p.label}</span>
          <Icon name="x" size={11} />
        </button>
      ))}
    </div>
  );
}
