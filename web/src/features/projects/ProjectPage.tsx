// A project: running agents, GitHub items, worktrees and all agents.

import { memo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Session } from "@/gen/Session";
import { isRunning } from "@/lib/sessionStatus";
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
import { useOpenTerminal } from "@/features/terminals/lib";
import { Description, TempProjectCard } from "./ProjectsView";

export function ProjectPage() {
  const { id } = useParams({ strict: false }) as { id: string };
  const p = useProject(id);
  const sessions = (useSessions().data ?? []).filter((s) => s.project_id === id);
  const worktrees = (useCodeSources(p?.id).data?.worktrees ?? []).filter((w) => !w.main);
  const terminal = useOpenTerminal();
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
            <button className="btn" title={`Open a shell in ${p.path}`} onClick={() => void terminal.shell(p.path)}>
              <Icon name="term" size={14} />
              Terminal
            </button>
            <button className="btn" title="Open in editor" onClick={() => void api.post("/api/open-editor", { path: p.path })}>
              <Icon name="file" size={14} />
              Editor
            </button>
            <LaunchButton ctx={{ projectId: p.id }} />
          </div>
        </div>
        {p.is_temp && <TempProjectCard p={p} />}
        <ProjectAgents title="Running agents" sessions={sessions.filter(isRunning)} empty="No running agents in this project." />
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
                  <button className="btn btn-sm" title={`Open a shell in ${w.path}`} onClick={() => void terminal.shell(w.path)}>
                    <Icon name="term" size={12} />
                    Terminal
                  </button>
                </div>
              ))}
            </div>
          </>
        )}
        <ProjectAgents title="All agents" sessions={sessions} empty="No agents in this project yet." />
      </div>
    </div>
  );
}

function ProjectAgents({ title, sessions, empty }: { title: string; sessions: Session[]; empty: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: sessions.length,
    getScrollElement: () => ref.current,
    estimateSize: () => 40,
    overscan: 3,
    useFlushSync: false,
  });
  return (
    <section className="grid gap-3">
      <div className="seclabel">{title} · {sessions.length}</div>
      <div ref={ref} className="overflow-auto rounded-lg border border-line" style={sessions.length ? { height: Math.min(sessions.length, 8) * 40 + 2 } : undefined}>
        {sessions.length === 0 ? (
          <div className="p-4 text-[12.5px] text-ink-2">{empty}</div>
        ) : (
          <div className="relative" style={{ height: v.getTotalSize() }}>
            {v.getVirtualItems().map((row) => (
              <div key={sessions[row.index].id} className="absolute w-full" style={{ transform: `translateY(${row.start}px)` }}>
                <ProjectAgentRow s={sessions[row.index]} />
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

const ProjectAgentRow = memo(function ProjectAgentRow({ s }: { s: Session }) {
  const show = useLayout((s) => s.show);
  const navigate = useNavigate();
  return (
    <button
      className="flex h-10 w-full items-center gap-2.5 border-b border-line-2 px-4 text-left hover:bg-[#f5f8fb]"
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
  );
});
