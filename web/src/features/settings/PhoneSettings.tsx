// Settings on the phone: device controls followed by the shared Settings navigation.

import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { SettingRow } from "@/components/ui";
import { useSetupSummary } from "@/features/setup/summary";
import { settingsPages } from "./nav";
import { DEFAULT_ZOOM, ZOOM_LEVELS, useZoom } from "@/lib/zoom";
import { layoutChoice, setLayoutChoice } from "@/lib/phone";
import { useConfig, useSharedSettings } from "@/lib/queries";
import { useOfflineRunners, useOutdatedRunners, useRefusedRunners, useRunners } from "@/lib/runners";
import { useScope } from "@/lib/workspace";
import { useTheme } from "@/stores/theme";
import { LAYOUTS } from "./SettingsView";

function Group({ title, children }: { title: string; children: ReactNode }) {
  return <section className="grid">
    <h2 className="seclabel m-0 px-4 pt-4 pb-1">{title}</h2>
    <div className="border-y border-line-2">{children}</div>
  </section>;
}

function Row({ to, icon, label, state, attn }: { to: string; icon: IconName; label: string; state?: string; attn?: boolean }) {
  return <Link to={to} className="flex min-h-12 items-center gap-3 border-b border-line-2 px-4 text-[15px] text-ink no-underline last:border-b-0">
    <Icon name={icon} size={18} className="text-ink-3" />
    <span className="min-w-0 flex-1 truncate">{label}</span>
    {state && <span className={`shrink-0 text-[13px] ${attn ? "font-semibold text-attn" : "text-ink-3"}`}>{state}</span>}
    <Icon name="chev" size={14} className="-rotate-90 text-ink-3" />
  </Link>;
}

const count = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export function PhoneSettings() {
  const { theme, toggle } = useTheme();
  const layout = layoutChoice();
  const { zoom, setZoom } = useZoom();
  const pages = settingsPages("");
  const workspace = useScope().workspace;
  const config = useConfig().data?.config;
  const presets = useSharedSettings().data?.presets;
  const runners = (useRunners().data ?? []).filter((r) => r.runner.state === "approved");
  const offline = useOfflineRunners().length;
  const outdated = useOutdatedRunners().length;
  const refused = useRefusedRunners().length;
  const setup = useSetupSummary().data;
  const states: Record<string, { state?: string; attn?: boolean }> = {
    "/settings/workspace": { state: workspace.name },
    "/settings/presets": { state: presets ? count(presets.length, "preset") : undefined },
    "/settings/profiles": { state: config ? count(config.profiles.length, "profile") : undefined, attn: config?.profiles.length === 0 },
    "/settings/runners": { state: refused ? `${refused} refused` : outdated ? `${outdated} to update` : offline ? `${offline} offline` : count(runners.length, "runner"), attn: refused + outdated + offline > 0 },
  };
  return (
    <div className="h-full overflow-y-auto pb-4" data-tour="settings.phone">
      <Group title="This device">
        <div className="grid gap-3 px-4 py-3">
          <SettingRow label="Zoom" hint="Scales the whole interface." scope="device">
            <label className="grid w-full gap-2">
              <span className="font-semibold">{ZOOM_LEVELS[zoom]}%{zoom === DEFAULT_ZOOM ? " (default)" : ""}</span>
              <input aria-label="Interface zoom" type="range" min={0} max={4} step={1} value={zoom} onChange={(e) => setZoom(Number(e.target.value))} className="w-full accent-teal" />
              <span className="flex justify-between text-[12px] text-ink-2">{ZOOM_LEVELS.map((z) => <span key={z}>{z}%</span>)}</span>
            </label>
          </SettingRow>
          <SettingRow label="Theme" scope="device">
            <div className="flex gap-1.5" role="group" aria-label="Theme">
              {(["light", "dark"] as const).map((t) => <button key={t} className="lp-chip phone-choice" aria-pressed={theme === t} onClick={() => { if (theme !== t) toggle(); }}><Icon name={t === "light" ? "sun" : "moon"} size={14} />{t === "light" ? "Light" : "Dark"}</button>)}
            </div>
          </SettingRow>
          <SettingRow label="Layout" hint="Automatic uses the phone layout below 720 px. Changing it reloads the page." scope="device">
            <div className="flex flex-wrap gap-1.5" role="group" aria-label="Layout">
              {LAYOUTS.map((l) => <button key={l.id} className="lp-chip phone-choice" aria-pressed={layout === l.id} onClick={() => { if (layout !== l.id) setLayoutChoice(l.id); }}>{l.label}</button>)}
            </div>
          </SettingRow>
        </div>
      </Group>
      <Group title="Workspace">
        {pages.workspace.map((e) => <Row key={e.to} {...e} {...states[e.to]} />)}
      </Group>
      {pages.groups.filter((g) => g.group !== "This device").map((g) => <Group key={g.group} title={g.group}>
        {g.items.map((e) => <Row key={e.to} {...e} {...states[e.to]} />)}
      </Group>)}
      <Group title="Setup">
        <Row to="/setup" icon="check" label="Setup checklist" state={setup ? setup.required_left ? `${count(setup.required_left, "step")} left` : "Done" : undefined} attn={!!setup?.required_left} />
      </Group>
    </div>
  );
}
