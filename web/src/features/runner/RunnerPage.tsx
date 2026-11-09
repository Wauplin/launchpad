// A runner's own window: its status only (link to Home, Takes new sessions, its setup checklist: jail, network
// capture, agent CLIs, vault logins, Git identity). Everything else lives on Home; the runner's daemon answers this
// window only for these calls (`auth::runner_window_path`). Missing parts show a command to copy into a terminal on
// this machine; Launchpad never runs them.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";
import type { Checklist } from "@/gen/Checklist";
import type { MissingPart } from "@/gen/MissingPart";
import type { SetupItem } from "@/gen/SetupItem";
import { Icon, IconSprite } from "@/components/Icon";
import { toast, toastError, Toasts } from "@/components/Toast";
import { Badge, Choice, Choices, ErrorState, Loading, PageHeader, SettingRow } from "@/components/ui";
import { api } from "@/lib/api";
import { runAction } from "@/lib/actions";
import { copyText } from "@/lib/clipboard";
import { externalLinkProps } from "@/lib/platform";
import { useStatus } from "@/lib/queries";
import { Card } from "@/features/settings/SettingsView";
import { UpdateThisRunnerButton } from "@/features/settings/RunnerUpdate";

const TUNNEL: Record<string, string> = { not_connected: "Not paired", connecting: "Connecting…", connected: "Connected", refused: "Refused", offline: "Home unreachable" };

export function RunnerApp() {
  return (
    <div className="flex h-full flex-col">
      <IconSprite />
      <RunnerPage />
      <Toasts />
    </div>
  );
}

function RunnerPage() {
  const status = useStatus().data;
  const checklist = useQuery({ queryKey: ["setup-checklist"], queryFn: () => api.get<Checklist>("/api/setup/checklist"), staleTime: 30_000 });
  const left = checklist.data?.required_left;
  const meta = <>{status && <span>{status.host} · build {status.commit.slice(0, 8)}</span>}{left != null && <span>{left ? `${left} required step${left === 1 ? "" : "s"} left` : "Ready"}</span>}</>;
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-paper">
      <PageHeader title="This runner" meta={meta} intro="This machine runs agents for your Home. Its window shows its status only: launch, review and change settings on Home." />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="phone-page-content grid max-w-page grid-cols-[minmax(0,1fr)] gap-4 px-5 py-5">
          {!status ? <Loading what="status" /> : <HomeCard />}
          <div className="flex flex-wrap items-center gap-2">
            <b className="flex-1 text-[15px] font-semibold">Setup</b>
            <button type="button" className="btn btn-sm" disabled={checklist.isFetching} onClick={() => void checklist.refetch()}>{checklist.isFetching ? <span className="lp-spin" aria-hidden="true" /> : <Icon name="refresh" size={12} />}Check again</button>
          </div>
          {checklist.error ? <ErrorState error={checklist.error} onRetry={() => void checklist.refetch()} /> : !checklist.data ? <Loading what="setup checks" /> : checklist.data.items.map((i) => <ItemCard key={i.id} item={i} />)}
        </div>
      </div>
    </div>
  );
}

/** The link to Home, Takes new sessions, and the update when this runner's build differs from Home's. */
function HomeCard() {
  const qc = useQueryClient();
  const status = useStatus().data;
  const home = status?.home;
  const paused = !!home?.guard.paused;
  const save = (paused: boolean) => runAction("runner.takes-sessions", {}, { paused }).then(() => qc.invalidateQueries({ queryKey: ["status"] }), toastError);
  return (
    <Card title="Home" end={home?.url && <a className="btn btn-sm no-underline" {...externalLinkProps(home.url)}><Icon name="ext" size={12} />Open Home</a>}>
      <SettingRow label="Home" hint={home?.message ?? (home?.state === "not_connected" ? "Run launchpad connect <Home address> in a terminal here, then approve the runner on Home." : undefined)}>
        <span className="flex flex-wrap items-center gap-2 text-[13px]">{home?.url && <span className="font-mono text-[12px]">{home.url}</span>}<Badge tone={home?.state === "connected" ? "ok" : "attn"}>{TUNNEL[home?.state ?? "not_connected"]}</Badge></span>
      </SettingRow>
      <SettingRow label="Takes new sessions" hint="Paused: Home starts no new session here; running sessions continue.">
        <Choices label="Takes new sessions">
          <Choice active={!paused} onClick={() => paused && void save(false)}>Yes</Choice>
          <Choice active={paused} onClick={() => !paused && void save(true)}>Paused</Choice>
        </Choices>
      </SettingRow>
      {home?.compat === "outdated" && (
        <div className="grid gap-1 rounded-card bg-attn-soft px-3 py-2 text-[12px] text-ink-2">
          <span><b className="text-attn">Update this runner.</b> Its build ({status?.commit.slice(0, 8)}) differs from Home's: its running sessions continue, but Home sends it no new session.</span>
          <UpdateThisRunnerButton />
          <span className="text-ink-3">It downloads Home's build and restarts as soon as none of its agents is in a turn. By hand: run the install command from Home's Settings → Home and runners again here.</span>
        </div>
      )}
    </Card>
  );
}

const STATE_BADGE = {
  ready: <Badge tone="ok" icon="check">Ready</Badge>,
  unverified: <Badge tone="neutral" icon="check" title="Set up, but Home could not confirm it now.">Not confirmed</Badge>,
  missing: <Badge tone="attn">Missing</Badge>,
};

function ItemCard({ item }: { item: SetupItem }) {
  return (
    <Card title={item.title} end={STATE_BADGE[item.state]}>
      <p className="m-0 text-[12px] text-ink-3">{item.why}</p>
      {item.found.length > 0 && (
        <dl className="m-0 grid grid-cols-[minmax(0,140px)_minmax(0,1fr)] gap-x-3 gap-y-1 text-[13px]">
          {item.found.map((f, i) => (
            <div key={i} className="contents">
              <dt className="truncate text-ink-3">{f.label}</dt>
              <dd className="m-0 min-w-0 break-words">{f.value}{f.source && <span className="text-[12px] text-ink-3"> · {f.source}</span>}</dd>
            </div>
          ))}
        </dl>
      )}
      {item.missing.map((p) => <PartRow key={p.id} part={p} />)}
    </Card>
  );
}

/** One missing part: what is missing, and its fixed command to copy into a terminal on this machine. */
function PartRow({ part }: { part: MissingPart }): ReactNode {
  return (
    <div className="grid gap-1.5 rounded-card border border-line-2 bg-chrome p-2.5">
      <div className="text-[13px] font-semibold">{part.label}</div>
      {part.detail && <div className="text-[12px] text-ink-2">{part.detail}</div>}
      {part.command && <>
        <code className="block max-h-[120px] overflow-auto rounded-control bg-paper px-2 py-1.5 font-mono text-[12px] break-all whitespace-pre-wrap">{part.command}</code>
        <div className="flex flex-wrap items-center gap-1.5">
          <button type="button" className="btn btn-sm" onClick={() => void copyText(part.command!).then(() => toast("Command copied"))}><Icon name="copy" size={12} />Copy</button>
          <span className="text-[12px] text-ink-3">Run it in a terminal on this machine{part.sudo ? "; it asks for your password" : ""}, then Check again.</span>
        </div>
      </>}
    </div>
  );
}
