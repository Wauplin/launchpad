import { useRemote } from "@/lib/remote";
// A project: header, description, its worktrees and its sessions. Issues and PRs arrive in Phase 3.

import { RepoItems } from "@/features/github/RepoItems";
import { Link, useParams } from "@tanstack/react-router";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { StatusBadge } from "@/features/agents/SessionBits";
import { useCodeSources } from "@/features/code/lib";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { api } from "@/lib/api";
import { useProject, useSessions } from "@/lib/queries";
import { useLayout } from "@/stores/layout";
import { useNavigate } from "@tanstack/react-router";
import { Description, TempProjectCard } from "./ProjectsView";

export function ProjectPage() {
  const remote = useRemote((s) => s.enabled);
  const { id } = useParams({ strict: false }) as { id: string };
  const p = useProject(id);
  const sessions = (useSessions().data ?? []).filter((s) => s.project_id === id);
  const show = useLayout((s) => s.show);
  const navigate = useNavigate();
  const worktrees = (useCodeSources(p?.id).data?.worktrees ?? []).filter((w) => !w.main);
  if (!p) return <div className="p-6 text-ink-3">This project is not registered.</div>;
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto grid max-w-[920px] grid-cols-[minmax(0,1fr)] gap-4 p-6">
        <div className="flex items-start gap-3">
          <div className="grid min-w-0 gap-1">
            <h2 className="m-0 text-lg font-semibold">{p.name}</h2>
            <span className="font-mono text-xs text-ink-3">{p.path}</span>
            <Description p={p} />
          </div>
          <div className="ml-auto flex gap-1.5">
            {p.gh_repo && (
              <button className="btn" title="Open on GitHub" onClick={() => void api.post("/api/open", { url: `https://github.com/${p.gh_repo}` })}>
                <Icon name="ext" size={14} />
                GitHub
              </button>
            )}
            <Link to="/projects/$id/code" params={{ id: p.id }} className="btn no-underline" title="Read the code of this project">
              <Icon name="search" size={14} />
              Code
            </Link>
            <button className="btn" title={remote ? "Copy editor command" : "Open in editor"} onClick={() => void api.post("/api/open-editor", { path: p.path })}>
              <Icon name="file" size={14} />
              {remote ? "Copy editor command" : "Editor"}
            </button>
            <LaunchButton ctx={{ projectId: p.id }} />
          </div>
        </div>
        {p.is_temp && <TempProjectCard p={p} />}
        <RepoItems key={p.gh_repo ?? p.id} repo={p.gh_repo ?? ""} projectId={p.id} />
        {worktrees.length > 0 && (
          <>
            <div className="seclabel">Worktrees</div>
            <div className="overflow-hidden rounded-lg border border-line">
              {worktrees.map((w) => (
                <div key={w.path} className="flex items-center gap-2.5 border-b border-line-2 px-4 py-2 last:border-0">
                  <Icon name="branch" size={14} className="text-ink-3" />
                  <span className="font-mono text-xs font-semibold">{w.branch ?? "detached"}</span>
                  <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-ink-3">{w.path}</span>
                  <Link to="/projects/$id/code" params={{ id: p.id }} search={{ root: w.path }} className="btn btn-sm no-underline" title="Read the code of this worktree">
                    <Icon name="search" size={12} />
                    Code
                  </Link>
                </div>
              ))}
            </div>
          </>
        )}
        <div className="seclabel">Sessions</div>
        <div className="overflow-hidden rounded-lg border border-line">
          {sessions.length === 0 && <div className="p-4 text-[12.5px] text-ink-2">No session in this project yet.</div>}
          {sessions.map((s) => (
            <button
              key={s.id}
              className="flex w-full items-center gap-2.5 border-b border-line-2 px-4 py-2 text-left last:border-0 hover:bg-[#f5f8fb]"
              onClick={() => {
                show(s.id);
                void navigate({ to: "/agents" });
              }}
            >
              <HarnessMark harness={s.harness} size={16} />
              <span className="min-w-0 flex-1 truncate text-[12.5px] font-semibold">{s.title}</span>
              <StatusBadge s={s} />
              <span className="text-xs text-ink-3">{new Date(s.started_at).toLocaleDateString()}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
