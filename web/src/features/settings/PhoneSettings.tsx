// Settings on the phone: the Settings pages (device controls in This device → Appearance), then the setup checklist.

import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { useSetupSummary } from "@/features/setup/summary";
import { settingsPages } from "./nav";
import { layoutChoice } from "@/lib/phone";
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
  const { theme } = useTheme();
  const layout = layoutChoice();
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
    "/settings/appearance": { state: `${theme === "dark" ? "Dark" : "Light"} · ${LAYOUTS.find((l) => l.id === layout)?.label ?? "Automatic"}` },
    "/settings/presets": { state: presets ? count(presets.length, "preset") : undefined },
    "/settings/profiles": { state: config ? count(config.profiles.length, "profile") : undefined, attn: config?.profiles.length === 0 },
    "/settings/runners": { state: refused ? `${refused} refused` : outdated ? `${outdated} to update` : offline ? `${offline} offline` : count(runners.length, "runner"), attn: refused + outdated + offline > 0 },
  };
  return (
    <div className="h-full overflow-y-auto pb-4" data-tour="settings.phone">
      <Group title="Workspace">
        {pages.workspace.map((e) => <Row key={e.to} {...e} {...states[e.to]} />)}
      </Group>
      {pages.groups.map((g) => <Group key={g.group} title={g.group}>
        {g.items.map((e) => <Row key={e.to} {...e} {...states[e.to]} />)}
      </Group>)}
      <Group title="Setup">
        <Row to="/setup" icon="check" label="Setup checklist" state={setup ? setup.required_left ? `${count(setup.required_left, "step")} left` : "Done" : undefined} attn={!!setup?.required_left} />
      </Group>
    </div>
  );
}
