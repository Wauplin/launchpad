// The setup checklist on Home: every onboarding step, what was found, and where to finish it. On a Home in a Space the
// gateway's onboarding page (`/_home/setup`) reports the steps; another Home computes the ones it can from its own data.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { ErrorState, Loading } from "@/components/ui";
import { delegatedAccess } from "@/lib/deployment";
import { useConfig, useProjects } from "@/lib/queries";
import { useRunners } from "@/lib/runners";
import { useBots } from "@/lib/bots";
import { useWorkspaces } from "@/lib/workspace";
import { useVault } from "@/features/settings/CredentialsView";
import { useAddFolder } from "@/features/projects/addFolderStore";
import { SUMMARY_KEY, homeLeft, homeStatus } from "./summary";


type Row = { id: string; title: string; why: string; done: boolean; state?: string; optional?: boolean; action: ReactNode };

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
  const vault = useVault().data;
  const config = useConfig().data?.config;
  const projects = useProjects().data ?? [];
  const runners = useRunners().data ?? [];
  const workspaces = useWorkspaces().data ?? [];
  const bots = useBots().data ?? [];
  const showAddFolder = useAddFolder((s) => s.show);
  if (delegatedAccess() && space.isPending) return <Loading what="Home setup" />;
  if (delegatedAccess() && space.error) return <ErrorState error={space.error} onRetry={() => void space.refetch()} />;
  const s = space.data;
  const fallback = !s;
  const ssh = vault?.entries.some((e) => e.kind === "ssh_key") ?? false;
  const settings = (to: string, label = "Open") => <Link to={to as never} className="btn btn-sm">{label}</Link>;
  const fix = (to: string, label: string) => (fallback ? settings(to, label) : openSetup);
  const rows: Row[] = [
    ...(s ? [
      { id: "huggingface", title: "Hugging Face", why: "Home's login for its private bucket.", done: s.huggingface.done, action: s.huggingface.done ? settings("/settings/huggingface") : openSetup },
      { id: "key", title: "Credential key", why: "Encrypts every credential Home stores. Keep your copy safe.", done: s.key.done, action: s.key.done ? null : openSetup },
      { id: "github", title: "GitHub", why: "Inbox, pull requests, private clones and publication.", done: s.github.done, action: s.github.done ? settings("/settings/github") : openSetup },
    ] : []),
    { id: "ssh", title: "SSH key", why: "Lets agents push over SSH. The private key never leaves Home.", done: s ? s.ssh.done : ssh, action: (s ? s.ssh.done : ssh) ? settings("/settings/credentials") : fix("/settings/credentials", "Add the SSH key") },
    { id: "identity", title: "Git identity", why: "The author of your agents' commits.", done: s ? s.identity.done : !!(vault?.git_name && vault.git_email), action: settings("/settings/credentials", s?.identity.done || vault?.git_name ? "Open" : "Set it") },
    { id: "profile", title: "Agent profile", why: "A subscription login and the default model and effort.", done: s ? s.profile.done : (config?.profiles.length ?? 0) > 0, action: (s ? s.profile.done : (config?.profiles.length ?? 0) > 0) ? settings("/settings/profiles") : fix("/settings/profiles", "Add a profile") },
    { id: "folders", title: "Workspace and first folder", why: "Folders your agents work in, grouped by workspace.", done: s ? s.folders.done : workspaces.length > 0 && projects.length > 0, action: <button className="btn btn-sm" onClick={showAddFolder}>Add a folder</button> },
    { id: "runner", title: "Runner", why: "A Linux machine of yours that runs the agents.", optional: true, done: s ? s.runner.done : runners.some((r) => r.connected), action: settings("/settings/runners", runners.some((r) => r.connected) ? "Open" : "Add a runner") },
    ...(s ? [{ id: "agent", title: "First agent", why: "Ask an agent what one of your projects is about.", optional: true, done: s.agent.done, action: s.agent.done ? null : openSetup }] : []),
    { id: "bot", title: "First bot", why: "An agent that stays: one thread, its own instructions, and notes it keeps between sessions.", optional: true, done: bots.length > 0, action: bots.length ? settings("/bots") : <Link to="/bots" search={{ new: true } as never} className="btn btn-sm">Create a bot</Link> },
  ];
  const done = rows.filter((r) => r.done).length;
  return (
    <div className="grid gap-3">
      <div className="grid gap-2 rounded-lg border border-line bg-paper p-4">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="m-0 flex-1 text-[15px] font-semibold">{done === rows.length ? "Home is ready" : `${done} of ${rows.length} steps done`}</h2>
          {s && <a href={SETUP} className="btn btn-sm">Open the setup page</a>}
        </div>
        <div className="h-1 overflow-hidden rounded-full bg-line-2"><div className="h-full bg-teal transition-[width]" style={{ width: `${Math.round((100 * done) / rows.length)}%` }} /></div>
        <p className="m-0 text-[12px] text-ink-3">Home keeps your records and credentials. Agents run on your runners. Each step below says what it is for and where to change it.</p>
      </div>
      <ol className="m-0 grid list-none overflow-hidden rounded-lg border border-line bg-paper p-0">
        {rows.map((r, i) => (
          <li key={r.id} className="flex items-center gap-3 border-b border-line-2 px-4 py-3 last:border-b-0">
            <span className={`grid size-6 flex-none place-items-center rounded-full text-[12px] font-semibold ${r.done ? "bg-teal text-white" : "border border-line text-ink-3"}`}>{r.done ? <Icon name="check" size={12} /> : i + 1}</span>
            <span className="grid min-w-0 flex-1">
              <span className="flex items-center gap-2 text-[13px] font-semibold">{r.title}{r.optional && !r.done && <span className="rounded-full border border-line-2 px-1.5 text-[11px] font-normal text-ink-3">recommended</span>}</span>
              <span className="text-[12px] text-ink-3">{r.why}</span>
            </span>
            <span className={`text-[12px] ${r.done ? "text-gh-open" : "text-ink-3"}`}>{r.done ? "Done" : "To do"}</span>
            {r.action}
          </li>
        ))}
      </ol>
    </div>
  );
}
