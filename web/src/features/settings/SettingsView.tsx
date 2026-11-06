import { usePhone, layoutChoice, setLayoutChoice, type LayoutChoice } from "@/lib/phone";
// Settings: two parts, This workspace (its bots, tools and MCP servers) and Launchpad (shared by every workspace:
// this device, agents, machines and access, maintenance), and one title per page.
// Fields save when they change and show "Saved" inline; only the profile editor keeps an explicit Save.
// Daemon settings go to config.toml, which can also be edited by hand.

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import type { Config } from "@/gen/Config";
import type { Preset } from "@/gen/Preset";
import { Icon } from "@/components/Icon";
import { ConfirmButton, IntroButton, Loading, MenuSection, MoreMenu, SettingRow, useSaveState, type SaveState } from "@/components/ui";
import { useIntro } from "@/lib/intros";
import { api } from "@/lib/api";
import { DEFAULT_ZOOM, ZOOM_LEVELS, useZoom } from "@/lib/zoom";
import { useConfig, type ConfigResponse } from "@/lib/queries";
import { useTheme } from "@/stores/theme";
import { PhoneSettings } from "./PhoneSettings";

/** A section of a Settings page. */
export function Card({ title, end, children, id, tour }: { title: string; end?: React.ReactNode; children: React.ReactNode; id?: string; tour?: string }) {
  return (
    <section id={id} data-tour={tour} className="overflow-hidden rounded-[10px] border border-line">
      <div className="flex flex-wrap items-center gap-2 border-b border-line bg-chrome px-3.5 py-2.5 text-[13px]">
        <b className="font-semibold text-ink">{title}</b>
        {end && <span className="ml-auto flex max-w-full flex-wrap justify-end gap-1.5">{end}</span>}
      </div>
      <div className="grid grid-cols-[minmax(0,1fr)] gap-2.5 p-3.5">{children}</div>
    </section>
  );
}

/** The layout of every Settings page: one title, one short intro (first visit only, then behind ⓘ). On the laptop the
 * settings sidebar (app/SettingsSidebar.tsx) lists the pages; on phones a link goes back to the list. */
export function SettingsPage({ title, intro, info, actions, children, tour }: { title?: string; intro?: React.ReactNode; info?: string; actions?: React.ReactNode; children: React.ReactNode; tour?: string }) {
  const phone = usePhone();
  // The intro shows on the first visit of the page on this device, then behind ⓘ.
  const explain = useIntro(intro && title ? `settings:${title}` : null);
  // The phone lists the pages on /settings (PhoneSettings); each page links back to it.
  const nav = phone && <Link to="/settings" className="btn btn-ghost w-fit px-1! no-underline"><Icon name="chev" size={14} className="rotate-90" />All settings</Link>;
  return (
    <div className="h-full overflow-y-auto">
      <div className="phone-page-content mx-auto grid max-w-[900px] grid-cols-[minmax(0,1fr)] gap-5 p-6">
        {nav}
        <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] content-start gap-4" data-tour={tour}>
          {title && (
            <div className="flex flex-wrap items-center gap-2">
              <h1 className={`m-0 font-semibold ${phone ? "text-[15px]" : "text-[18px]"}`}>{title}</h1>
              {intro ? <IntroButton {...explain} /> : info && <span className="text-ink-3" title={info} aria-label={info}><Icon name="info" size={14} /></span>}
              {actions && <span className="ml-auto flex flex-wrap gap-2">{actions}</span>}
              {intro && explain.open && <p className="m-0 w-full text-[13px] text-ink-2">{intro}{info && <span className="mt-1 block text-[12px] text-ink-3">{info}</span>}</p>}
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

export const LAYOUTS: { id: LayoutChoice; label: string }[] = [{ id: "auto", label: "Automatic" }, { id: "phone", label: "Phone" }, { id: "desktop", label: "Desktop" }];

/** Settings → Appearance (this device only); the phone shows its list of settings instead. */
export function SettingsView() {
  const phone = usePhone();
  return phone ? <PhoneSettings /> : <Appearance />;
}

function Appearance() {
  const { zoom, setZoom } = useZoom();
  const { theme, toggle } = useTheme();
  const layout = layoutChoice();
  return (
    <SettingsPage title="Appearance" intro="How Launchpad looks in this browser. Other devices keep their own choices.">
      <Card title="Display">
        <SettingRow label="Zoom" hint="Scales the whole interface." scope="device">
          <label className="grid w-full gap-2">
            <span className="font-semibold">{ZOOM_LEVELS[zoom]}%{zoom === DEFAULT_ZOOM ? " (default)" : ""}</span>
            <input aria-label="Interface zoom" type="range" min={0} max={4} step={1} value={zoom} onChange={(e) => setZoom(Number(e.target.value))} className="w-full accent-teal" />
            <span className="flex justify-between text-[12px] text-ink-2">{ZOOM_LEVELS.map((z) => <span key={z}>{z}%</span>)}</span>
          </label>
        </SettingRow>
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
      info="Choose where each preset is available: messages to bots, agent composer / steering (including Send later), or both. Saved to config.toml, which you can also edit by hand."
      actions={<><SaveMark state={state} error={error} /><button className="btn btn-sm" onClick={() => save([...presets, { id: `preset-${Date.now().toString(36)}`, label: "new preset", text: "", bots: false, agent: true }])}><Icon name="plus" size={12} />New preset</button></>}
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
            <input className="field-input font-semibold" aria-label="Preset name" value={p.label} onChange={(e) => edit(i, { label: e.target.value })} onBlur={() => save(presets)} />
            <textarea rows={2} className="field-input h-auto min-h-[30px] resize-y py-1.5 text-[13px]" aria-label="Preset text" value={p.text} placeholder="The sentence added to the prompt" onChange={(e) => edit(i, { text: e.target.value })} onBlur={() => save(presets)} />
            <div className="preset-availability col-start-2 col-end-4 flex flex-wrap gap-x-5 gap-y-1 text-[12px]">
              <label className="flex min-h-8 items-center gap-2"><input type="checkbox" checked={p.bots === true} onChange={(e) => save(presets.map((x, j) => j === i ? { ...x, bots: e.target.checked } : x))} />Bots</label>
              <label className="flex min-h-8 items-center gap-2"><input type="checkbox" checked={p.agent !== false} onChange={(e) => save(presets.map((x, j) => j === i ? { ...x, agent: e.target.checked } : x))} />Agent composer / steering</label>
            </div>
            <div className="preset-delete col-start-4 row-start-1"><MoreMenu label={`More actions for ${p.label || "this preset"}`}>{(close) => <MenuSection><ConfirmButton icon="trash" consequence="Delete this preset?" confirmLabel="Delete" onConfirm={() => { close(); save(presets.filter((_, j) => j !== i)); }}>Delete</ConfirmButton></MenuSection>}</MoreMenu></div>
          </div>
        ))}
      </Card>
    </SettingsPage>
  );
}

/** Settings → Notifications: desktop notifications here, and Telegram for the phone. */
export function NotificationsView() {
  const q = useConfig();
  const patch = usePatchConfig();
  const { state, error, run } = useSaveState();
  const notify = q.data?.config.agents.notify;
  const phone = usePhone();
  const desktop = (
    <Card title="Desktop notifications">
      <SettingRow label="Agent notifications" hint="Desktop notifications on the machine that runs the agent." scope="all" state={state} error={error}>
        {notify === undefined ? <Loading /> : <label className="flex items-center gap-2 text-[13px]"><input type="checkbox" className="accent-teal" checked={notify} onChange={(e) => void run(() => patch({ notify: e.target.checked }))} />Send notifications</label>}
      </SettingRow>
    </Card>
  );
  const telegram = (
    <Card title={phone ? "On this phone" : "Your phone"}>
      <p className="m-0 text-[13px] text-ink-2">Get finished agents and routine reports on your phone through Telegram.</p>
      <div><Link to={"/settings/channels" as never} className="btn btn-sm no-underline">Open Settings → Channels</Link></div>
    </Card>
  );
  return (
    <SettingsPage title="Notifications" intro="When an agent finishes, fails or stops on its own.">
      <ConfigError />
      {/* A phone gets its notifications through Telegram: show that first there. */}
      {phone ? <>{telegram}{desktop}</> : <>{desktop}{telegram}</>}
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
