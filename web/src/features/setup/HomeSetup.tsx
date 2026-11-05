// The setup checklist on Home: every onboarding step, what was found, and where to finish it. On a Home in a Space the
// gateway's onboarding page (`/_home/setup`) reports the steps; another Home computes the ones it can from its own data.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useEffect, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { ErrorState, Loading } from "@/components/ui";
import { delegatedAccess } from "@/lib/deployment";
import { useConfig, useProjects } from "@/lib/queries";
import { useRunners } from "@/lib/runners";
import { useBots } from "@/lib/bots";
import { useWorkspaces } from "@/lib/workspace";
import { useVault } from "@/features/settings/CredentialsView";
import { useAddFolder } from "@/features/projects/addFolderStore";
import { SUMMARY_KEY, homeLeft, homeStatus, homeSteps, type HomeStep } from "./summary";


type Row = HomeStep & { action: ReactNode };

const SETUP = "/_home/setup";
const openSetup = <a href={SETUP} className="btn btn-sm btn-primary">Open setup</a>;

function useSpaceStatus() {
  const qc = useQueryClient();
  return useQuery({
    queryKey: ["home-setup-status"],
    queryFn: async () => {
      const s = await homeStatus();
      qc.setQueryData(SUMMARY_KEY, homeLeft(s));
      return s;
    },
    enabled: delegatedAccess(),
    refetchInterval: 15_000,
  });
}

export function HomeSetup() {
  const space = useSpaceStatus();
  const qc = useQueryClient();
  const vault = useVault().data;
  const config = useConfig().data?.config;
  const projects = useProjects().data ?? [];
  const runners = useRunners().data ?? [];
  const workspaces = useWorkspaces().data ?? [];
  const bots = useBots().data ?? [];
  const showAddFolder = useAddFolder((s) => s.show);
  const s = space.data;
  // Outside a Space, the same steps from Home's own data (no Hugging Face, key or GitHub steps).
  const local = s ? null : homeSteps({ vault, profiles: config?.profiles.length ?? 0, workspaces: workspaces.length, projects: projects.length, runners, bots: bots.length });
  const localLeft = local && vault && config ? local.filter((r) => r.required && !r.done).length : null;
  useEffect(() => {
    if (localLeft !== null) qc.setQueryData(SUMMARY_KEY, { required_left: localLeft });
  }, [qc, localLeft]);
  if (delegatedAccess() && space.isPending) return <Loading what="Home setup" />;
  if (delegatedAccess() && space.error) return <ErrorState error={space.error} onRetry={() => void space.refetch()} />;
  const settings = (to: string, label = "Open") => <Link to={to as never} className="btn btn-sm">{label}</Link>;
  const fix = (to: string, label: string) => (s ? openSetup : settings(to, label));
  const action = (r: HomeStep): ReactNode => {
    switch (r.id) {
      case "huggingface": return r.done ? settings("/settings/huggingface") : openSetup;
      case "key": return r.done ? null : openSetup;
      case "github": return r.done ? settings("/settings/github") : openSetup;
      case "ssh": return r.done ? settings("/settings/credentials") : fix("/settings/credentials", "Add the SSH key");
      case "identity": return settings("/settings/credentials", r.done ? "Open" : "Set it");
      case "profile": return r.done ? settings("/settings/profiles") : fix("/settings/profiles", "Add a profile");
      case "folders": return <button className="btn btn-sm" onClick={() => showAddFolder()}>Add a folder</button>;
      case "runner": return settings("/settings/runners", r.done ? "Open" : "Add a runner");
      case "agent": return r.done ? null : openSetup;
      case "bot": return r.done ? settings("/bots") : <Link to="/bots" search={{ new: true } as never} className="btn btn-sm">Create a bot</Link>;
      default: return s ? openSetup : null;
    }
  };
  const rows: Row[] = (s?.steps ?? local ?? []).map((r) => ({ ...r, action: action(r) }));
  const required = rows.filter((r) => r.required);
  const done = required.filter((r) => r.done).length;
  return (
    <div className="grid gap-3">
      <div className="grid gap-2 rounded-lg border border-line bg-paper p-4">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="m-0 flex-1 text-[15px] font-semibold">{done === required.length ? "Home is ready" : `${done} of ${required.length} required steps done`}</h2>
          {s && <a href={SETUP} className="btn btn-sm">Open the setup page</a>}
        </div>
        <div className="h-1 overflow-hidden rounded-full bg-line-2"><div className="h-full bg-teal transition-[width]" style={{ width: `${Math.round((100 * done) / Math.max(required.length, 1))}%` }} /></div>
        <p className="m-0 text-[12px] text-ink-3">Home keeps your records and credentials. Agents run on your runners. Each step below says what it is for and where to change it. Optional steps never keep Home from being ready.</p>
      </div>
      <ol className="m-0 grid list-none overflow-hidden rounded-lg border border-line bg-paper p-0">
        {rows.map((r, i) => (
          <li key={r.id} className="flex items-center gap-3 border-b border-line-2 px-4 py-3 last:border-b-0">
            <span className={`grid size-6 flex-none place-items-center rounded-full text-[12px] font-semibold ${r.done ? "bg-teal text-white" : "border border-line text-ink-3"}`}>{r.done ? <Icon name="check" size={12} /> : i + 1}</span>
            <span className="grid min-w-0 flex-1">
              <span className="flex items-center gap-2 text-[13px] font-semibold">{r.title}{r.tag && !r.done && <span className="rounded-full border border-line-2 px-1.5 text-[11px] font-normal text-ink-3">{r.tag}</span>}</span>
              <span className="text-[12px] text-ink-3">{r.why}</span>
            </span>
            <span className={`text-[12px] ${r.done ? "text-gh-open" : "text-ink-3"}`}>{r.done ? "Done" : r.skipped ? "Skipped" : "To do"}</span>
            {r.action}
          </li>
        ))}
      </ol>
    </div>
  );
}
