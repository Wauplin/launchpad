import { usePhone, layoutChoice, setLayoutChoice, type LayoutChoice } from "@/lib/phone";
// Settings: a grouped navigation (This device · Agents · Machines & access · Maintenance) and one title per page.
// Fields save when they change and show "Saved" inline; only the profile editor keeps an explicit Save.
// Daemon settings go to config.toml, which can also be edited by hand.

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import type { Config } from "@/gen/Config";
import type { Preset } from "@/gen/Preset";
import { Icon, type IconName } from "@/components/Icon";
import { ConfirmButton, Loading, SettingRow, useSaveState, type SaveState } from "@/components/ui";
import { api } from "@/lib/api";
import { DEFAULT_ZOOM, ZOOM_LEVELS, useZoom } from "@/lib/zoom";
import { useConfig, type ConfigResponse } from "@/lib/queries";
import { useTheme } from "@/stores/theme";

/** A section of a Settings page. */
export function Card({ title, end, children, id }: { title: string; end?: React.ReactNode; children: React.ReactNode; id?: string }) {
  return (
    <section id={id} className="overflow-hidden rounded-[10px] border border-line">
      <div className="flex flex-wrap items-center gap-2 border-b border-line bg-chrome px-3.5 py-2.5 text-[13px]">
        <b className="font-semibold text-ink">{title}</b>
        {end && <span className="ml-auto flex max-w-full flex-wrap justify-end gap-1.5">{end}</span>}
      </div>
      <div className="grid gap-2.5 p-3.5">{children}</div>
    </section>
  );
}

type NavEntry = { to: string; label: string; icon: IconName; exact?: boolean };

export const SETTINGS_NAV: { group: string; items: NavEntry[] }[] = [
  { group: "This device", items: [{ to: "/settings", label: "Appearance", icon: "sun", exact: true }, { to: "/settings/notifications", label: "Notifications", icon: "bell" }] },
  { group: "Agents", items: [{ to: "/settings/profiles", label: "Profiles", icon: "agents" }, { to: "/settings/presets", label: "Presets", icon: "preset" }, { to: "/settings/usage", label: "Usage", icon: "clock" }] },
  { group: "Machines & access", items: [{ to: "/settings/hosts", label: "Machines", icon: "term" }, { to: "/settings/devices", label: "Devices", icon: "window" }, { to: "/settings/github", label: "GitHub", icon: "pr" }, { to: "/settings/huggingface", label: "Hugging Face", icon: "upload" }] },
  { group: "Maintenance", items: [{ to: "/settings/worktrees", label: "Maintenance", icon: "archive" }] },
  { group: "Help", items: [{ to: "/settings/onboarding", label: "Onboarding", icon: "spark" }] },
];

/** The layout of every Settings page: grouped navigation on the left (a scrolling row on phones), one title, one short intro. */
export function SettingsPage({ title, intro, info, actions, children, tour }: { title?: string; intro?: React.ReactNode; info?: string; actions?: React.ReactNode; children: React.ReactNode; tour?: string }) {
  const phone = usePhone();
  const nav = phone ? (
    <nav className="phone-tabs flex gap-1.5" aria-label="Settings pages">
      {SETTINGS_NAV.flatMap((g) => g.items).map((e) => <Link key={e.to} to={e.to} className="btn" activeOptions={{ exact: !!e.exact }} activeProps={{ "aria-pressed": true } as never}>{e.label}</Link>)}
    </nav>
  ) : (
    <nav className="sticky top-0 grid content-start gap-3 self-start pt-1" aria-label="Settings pages">
      {SETTINGS_NAV.map((g) => (
        <div key={g.group} className="grid gap-0.5">
          <div className="seclabel px-2 pb-1">{g.group}</div>
          {g.items.map((e) => (
            <Link key={e.to} to={e.to} activeOptions={{ exact: !!e.exact }} className="flex h-[30px] items-center gap-2 rounded-[7px] px-2 text-[13px] text-ink-2 no-underline hover:bg-sunk hover:text-ink" activeProps={{ className: "bg-select font-semibold text-ink!", "aria-current": "page" } as never}>
              <Icon name={e.icon} size={14} />
              {e.label}
            </Link>
          ))}
        </div>
      ))}
    </nav>
  );
  return (
    <div className="h-full overflow-y-auto">
      <div className={`phone-page-content mx-auto grid max-w-[1080px] gap-5 p-6 ${phone ? "grid-cols-[minmax(0,1fr)]" : "grid-cols-[180px_minmax(0,1fr)]"}`}>
        {nav}
        <div className="grid min-w-0 content-start gap-4" data-tour={tour}>
          {title && (
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="m-0 text-[18px] font-semibold">{title}</h1>
              {info && <span className="text-ink-3" title={info} aria-label={info}><Icon name="info" size={14} /></span>}
              {actions && <span className="ml-auto flex flex-wrap gap-2">{actions}</span>}
              {intro && <p className="m-0 w-full text-[13px] text-ink-2">{intro}</p>}
            </div>
          )}
          {children}
        </div>
      </div>
    </div>
  );
}

/** Saves some daemon settings at once (`PATCH /api/config`) and updates the cached config. */
export function usePatchConfig() {
  const qc = useQueryClient();
  return async (patch: Record<string, unknown>) => {
    const res = await api.patch<ConfigResponse>("/api/config", patch);
    qc.setQueryData(["config"], res);
    return res;
  };
}

/** A text field that saves on blur or Enter, with the inline save state next to it. */
export function SavedInput({ value, onSave, className = "", label, placeholder, type = "text", disabled }: { value: string; onSave: (value: string) => Promise<unknown>; className?: string; label: string; placeholder?: string; type?: string; disabled?: boolean }) {
  const [text, setText] = useState(value);
  const { state, error, run } = useSaveState();
  useEffect(() => setText(value), [value]);
  const commit = () => { if (text !== value) void run(() => onSave(text)); };
  return (
    <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
      <input className={`field-input min-w-0 flex-1 ${className}`} type={type} aria-label={label} placeholder={placeholder} disabled={disabled} value={text} onChange={(e) => setText(e.target.value)} onBlur={commit} onKeyDown={(e) => { if (e.key === "Enter") commit(); }} />
      <SaveMark state={state} error={error} />
    </span>
  );
}

function SaveMark({ state, error }: { state: SaveState; error?: string }) {
  if (state === "idle") return null;
  return state === "saving" ? <span className="text-[11px] text-ink-3">Saving…</span> : state === "error" ? <span className="text-[11px] text-danger-ink" role="alert">{error}</span> : <span className="inline-flex items-center gap-1 text-[11px] text-gh-open"><Icon name="check" size={11} />Saved</span>;
}

export function ConfigError() {
  const q = useConfig();
  if (!q.data?.error) return null;
  return <div role="alert" className="rounded-lg bg-attn-soft px-3 py-2 text-[12px] text-attn">config.toml has an error, the previous settings are in use: {q.data.error}</div>;
}

const LAYOUTS: { id: LayoutChoice; label: string }[] = [{ id: "auto", label: "Automatic" }, { id: "phone", label: "Phone" }, { id: "desktop", label: "Desktop" }];

/** Settings → Appearance (this device only). */
export function SettingsView() {
  const phone = usePhone();
  const { zoom, setZoom } = useZoom();
  const { theme, toggle } = useTheme();
  const layout = layoutChoice();
  return (
    <SettingsPage title="Appearance" intro="How Launchpad looks in this browser. Other devices keep their own choices.">
      <Card title="Display">
        {!phone && (
          <SettingRow label="Zoom" hint="Scales the whole interface." scope="device">
            <label className="grid w-full gap-2">
              <span className="font-semibold">{ZOOM_LEVELS[zoom]}%{zoom === DEFAULT_ZOOM ? " (default)" : ""}</span>
              <input aria-label="Interface zoom" type="range" min={0} max={4} step={1} value={zoom} onChange={(e) => setZoom(Number(e.target.value))} className="w-full accent-teal" />
              <span className="flex justify-between text-[12px] text-ink-2">{ZOOM_LEVELS.map((z) => <span key={z}>{z}%</span>)}</span>
            </label>
          </SettingRow>
        )}
        <SettingRow label="Theme" hint="Also switchable from the sidebar." scope="device">
          <div className="flex gap-1.5" role="group" aria-label="Theme">
            {(["light", "dark"] as const).map((t) => <button key={t} className="lp-chip" aria-pressed={theme === t} onClick={() => { if (theme !== t) toggle(); }}><Icon name={t === "light" ? "sun" : "moon"} size={12} />{t === "light" ? "Light" : "Dark"}</button>)}
          </div>
        </SettingRow>
        <SettingRow label="Layout" hint="Automatic uses the phone layout below 720 px. Changing it reloads the page." scope="device">
          <div className="flex flex-wrap gap-1.5" role="group" aria-label="Layout">
            {LAYOUTS.map((l) => <button key={l.id} className="lp-chip" aria-pressed={layout === l.id} onClick={() => { if (layout !== l.id) setLayoutChoice(l.id); }}>{l.label}</button>)}
          </div>
        </SettingRow>
      </Card>
    </SettingsPage>
  );
}

/** Settings → Presets: each change saves at once. */
export function PresetsView() {
  const q = useConfig();
  const patch = usePatchConfig();
  const { state, error, run } = useSaveState();
  const [presets, setPresets] = useState<Preset[] | null>(null);
  useEffect(() => { if (q.data) setPresets(q.data.config.presets); }, [q.data]);
  if (!presets || !q.data) return <SettingsPage title="Presets"><Loading what="presets" /></SettingsPage>;
  const saved = q.data.config.presets;
  const save = (next: Preset[]) => { setPresets(next); if (JSON.stringify(next) !== JSON.stringify(saved)) void run(() => patch({ presets: next })); };
  const edit = (i: number, p: Partial<Preset>) => setPresets(presets.map((x, j) => (j === i ? { ...x, ...p } : x)));
  const move = (i: number, d: -1 | 1) => {
    const next = [...presets];
    const j = i + d;
    if (j < 0 || j >= next.length) return;
    [next[i], next[j]] = [next[j], next[i]];
    save(next);
  };
  return (
    <SettingsPage
      tour="settings.presets"
      title="Presets"
      intro="Instructions added to prompts, in this order."
      info="Every preset is offered in the composer, the session message box and Send later. Saved to config.toml, which you can also edit by hand."
      actions={<><SaveMark state={state} error={error} /><button className="btn btn-sm" onClick={() => save([...presets, { id: `preset-${Date.now().toString(36)}`, label: "new preset", text: "" }])}><Icon name="plus" size={12} />New preset</button></>}
    >
      <ConfigError />
      <Card title={`${presets.length} preset${presets.length === 1 ? "" : "s"}`}>
        {presets.length === 0 && <div className="text-[12px] text-ink-3">No preset yet.</div>}
        {presets.map((p, i) => (
          <div key={p.id} className="phone-preset grid grid-cols-[26px_150px_1fr_auto] items-start gap-2.5 rounded-lg border border-line p-2.5">
            <span className="grid">
              <button className="iconbtn size-[22px]! disabled:opacity-30" disabled={i === 0} title="Move up" aria-label="Move up" onClick={() => move(i, -1)}><Icon name="up" size={12} /></button>
              <button className="iconbtn size-[22px]! disabled:opacity-30" disabled={i === presets.length - 1} title="Move down" aria-label="Move down" onClick={() => move(i, 1)}><Icon name="down" size={12} /></button>
            </span>
            <input className="field-input h-[30px] text-[13px] font-semibold" aria-label="Preset name" value={p.label} onChange={(e) => edit(i, { label: e.target.value })} onBlur={() => save(presets)} />
            <textarea rows={2} className="field-input h-auto min-h-[30px] resize-y py-1.5 text-[13px]" aria-label="Preset text" value={p.text} placeholder="The sentence added to the prompt" onChange={(e) => edit(i, { text: e.target.value })} onBlur={() => save(presets)} />
            <ConfirmButton icon="trash" title="Delete preset" consequence="Delete this preset?" confirmLabel="Delete" onConfirm={() => save(presets.filter((_, j) => j !== i))}>{""}</ConfirmButton>
          </div>
        ))}
      </Card>
    </SettingsPage>
  );
}

/** Settings → Notifications: this browser's push subscription and the daemon-wide switch. */
export function NotificationsView({ browser }: { browser: React.ReactNode }) {
  const q = useConfig();
  const patch = usePatchConfig();
  const { state, error, run } = useSaveState();
  const notify = q.data?.config.agents.notify;
  return (
    <SettingsPage title="Notifications" intro="When an agent finishes, fails or stops on its own.">
      <ConfigError />
      <Card title="All devices">
        <SettingRow label="Agent notifications" hint="Desktop and phone notifications for every paired browser." scope="all" state={state} error={error}>
          {notify === undefined ? <Loading /> : <label className="flex items-center gap-2 text-[13px]"><input type="checkbox" className="accent-teal" checked={notify} onChange={(e) => void run(() => patch({ notify: e.target.checked }))} />Send notifications</label>}
        </SettingRow>
      </Card>
      {browser}
    </SettingsPage>
  );
}

/** Settings → Maintenance → Local PR cleanup, shared by the Maintenance page. */
export function LocalPrCleanupRow({ config }: { config: Config }) {
  const patch = usePatchConfig();
  const { state, error, run } = useSaveState();
  return (
    <SettingRow label="Published Local PRs" hint="Remove the clean worktree after a successful publication or local merge." scope="all" state={state} error={error}>
      <label className="flex items-center gap-2 text-[13px]"><input type="checkbox" className="accent-teal" checked={config.remove_published_worktree} onChange={(e) => void run(() => patch({ remove_published_worktree: e.target.checked }))} />Remove after publishing or merging locally</label>
    </SettingRow>
  );
}
