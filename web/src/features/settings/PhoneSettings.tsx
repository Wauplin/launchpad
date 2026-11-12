// Settings on the phone (`/settings`): this device first, then the pages that work on a phone, then what stays on
// the laptop with its status only. Each page opens the same component as on the laptop.

import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { SettingRow } from "@/components/ui";
import { useSetupSummary } from "@/features/setup/summary";
import { delegatedAccess } from "@/lib/deployment";
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

/** A page left to the laptop: its status, no link. */
function Status({ icon, label, state, attn }: { icon: IconName; label: string; state?: string; attn?: boolean }) {
  return <div className="flex min-h-12 items-center gap-3 border-b border-line-2 px-4 text-[15px] text-ink-2 last:border-b-0">
    <Icon name={icon} size={18} className="text-ink-3" />
    <span className="min-w-0 flex-1 truncate">{label}</span>
    {state && <span className={`shrink-0 text-[13px] ${attn ? "font-semibold text-attn" : "text-ink-3"}`}>{state}</span>}
  </div>;
}

const count = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export function PhoneSettings() {
  const { theme, toggle } = useTheme();
  const layout = layoutChoice();
  const workspace = useScope().workspace;
  const config = useConfig().data?.config;
  const presets = useSharedSettings().data?.presets;
  const runners = (useRunners().data ?? []).filter((r) => r.runner.state === "approved");
  const offline = useOfflineRunners().length;
  const outdated = useOutdatedRunners().length;
  const refused = useRefusedRunners().length;
  const setup = useSetupSummary().data;
  return (
    <div className="h-full overflow-y-auto pb-4" data-tour="settings.phone">
      <Group title="This device">
        <div className="grid gap-3 px-4 py-3">
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
        {!delegatedAccess() && <Row to="/settings/devices" icon="window" label="Devices" />}
      </Group>
      <Group title="On the phone">
        <Row to="/settings/connections" icon="send" label="Connections" />
        <Row to="/settings/workspace" icon="folder" label="Workspace" state={workspace.name} />
        <Row to="/settings/voice" icon="mic" label="Voice" />
        <Row to="/settings/presets" icon="preset" label="Presets" state={presets ? count(presets.length, "preset") : undefined} />
        <Row to="/settings/github" icon="pr" label="GitHub" />
        <Row to="/settings/usage" icon="clock" label="Usage" />
      </Group>
      <Group title="On a computer">
        <p className="m-0 border-b border-line-2 px-4 py-2 text-[13px] text-ink-3">Open Launchpad on a computer to change these.</p>
        <Status icon="agents" label="Profiles" state={config ? count(config.profiles.length, "profile") : undefined} attn={config?.profiles.length === 0} />
        {runners.length > 0 && <Status icon="window" label="Home and runners" state={refused ? `${refused} refused` : outdated ? `${outdated} to update` : offline ? `${offline} offline` : count(runners.length, "runner")} attn={refused + outdated + offline > 0} />}
        <Status icon="sliders" label="Credentials and tools" />
        <Status icon="huggingface" label="Hugging Face" />
        <Status icon="archive" label="Disk cleanup" />
        <Status icon="check" label="Setup checklist" state={setup ? setup.required_left ? `${count(setup.required_left, "step")} left` : "Done" : undefined} attn={!!setup?.required_left} />
      </Group>
    </div>
  );
}
