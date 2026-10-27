import { usePhone } from "@/lib/phone";
import { PhoneHeader, PhoneActions } from "@/mobile/PhoneHeader";
// A project: running agents, GitHub items, worktrees and all agents.

import { openExternal } from "@/lib/platform";
import { memo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Session } from "@/gen/Session";
import { isRunning, needsYou } from "@/lib/sessionStatus";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { Project } from "@/gen/Project";
import { toastError } from "@/components/Toast";
import type { Predraft } from "@/gen/Predraft";
import { useComposer } from "@/stores/composer";
import { RepoItems } from "@/features/github/RepoItems";
import { Link, useParams, useSearch } from "@tanstack/react-router";
import { ConfirmButton, Empty, FilterChip, FilterChips, SettingRow, Tabs, useSaveState } from "@/components/ui";
import { setWorkspaceProjects, useWorkspaces } from "@/lib/workspace";
import { toast } from "@/components/Toast";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { StatusBadge } from "@/features/agents/SessionBits";
import { useCodeSources } from "@/features/code/lib";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { api } from "@/lib/api";
import { useConfig, useProject, useSessions } from "@/lib/queries";
import { useLayout } from "@/stores/layout";
import { useNavigate } from "@tanstack/react-router";
import { TerminalMenu } from "@/features/terminals/TerminalMenu";
import { Description, TempProjectCard } from "./ProjectsView";

export function ProjectPage() {
  const phone = usePhone();
  const { id } = useParams({ strict: false }) as { id: string };
  const p = useProject(id);
  const sessions = (useSessions().data ?? []).filter((s) => s.project_id === id);
  const worktrees = (useCodeSources(p?.id).data?.worktrees ?? []).filter((w) => !w.main);
  const predrafts = (useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<Predraft[]>("/api/predrafts") }).data ?? []).filter((d) => d.project_id === id);
  const show = useLayout((s) => s.show);
  const navigate = useNavigate();
  const tab = (useSearch({ strict: false }) as { tab?: string }).tab === "settings" ? "settings" : "overview";
  const setTab = (t: "overview" | "settings") => void navigate({ to: "/projects/$id", params: { id }, search: (t === "settings" ? { tab: t } : {}) as never, replace: true });
  if (!p) return <Empty fill icon="folder">This folder is not registered in Launchpad.</Empty>;
  const tabs = <Tabs label="Folder views" value={tab} onChange={setTab} items={[{ id: "overview", label: "Overview" }, { id: "settings", label: "Settings", icon: "sliders" }]} />;
  return (
    <div className="h-full overflow-y-auto">
      <div className="phone-page-content mx-auto grid max-w-[920px] grid-cols-[minmax(0,1fr)] gap-4 p-6">
        {phone ? <PhoneHeader title={p.name} onBack={() => void navigate({ to: "/projects" })} backLabel="Back to folders" primary={<LaunchButton ctx={{ projectId: p.id }} />} subtitle={`${sessions.filter(isRunning).length} running · ${sessions.filter(needsYou).length} waiting · ${predrafts.filter((d) => !d.archived && !d.published_url).length} Local PR`} actions={
          <div className="ml-auto flex gap-1.5">
            {p.gh_repo && (
              <button className="btn" title="Open on GitHub" onClick={() => openExternal(`https://github.com/${p.gh_repo}`)}>
                <Icon name="ext" size={14} />
                GitHub
              </button>
            )}
            <Link to="/projects/$id/code" params={{ id: p.id }} className="btn no-underline" title="Browse the files of this folder">
              <Icon name="folder" size={14} />
              Files
            </Link>
            <TerminalMenu cwd={p.path} phone={phone} />

          </div>        } details={<div className="grid gap-2"><span className="break-all font-mono text-xs text-ink-3">{p.path}</span><Description p={p} /></div>} /> : <>
        <div className="flex items-start gap-3">
          <div className="grid min-w-0 gap-1">
            <h2 className="m-0 text-lg font-semibold">{p.name}</h2>
            <span className="font-mono text-xs text-ink-3">{p.path}</span>
            {p.description && <span className="text-xs text-ink-2">{p.description}</span>}
            <div className="mt-1 flex flex-wrap gap-1.5 text-xs">
              {[
                [sessions.filter(isRunning).length, "running"],
                [sessions.filter(needsYou).length, "waiting"],
                [predrafts.filter((d) => !d.archived && !d.published_url).length, "Local PR"],
              ].map(([n, label]) => (
                <span key={label} className={`rounded-full border px-2 py-0.5 ${n ? "border-line bg-paper font-semibold text-ink" : "border-line-2 text-ink-3"}`}>{n} {label}</span>
              ))}
            </div>
          </div>
          <div className="ml-auto flex gap-1.5">
            {p.gh_repo && (
              <button className="btn" title="Open on GitHub" onClick={() => openExternal(`https://github.com/${p.gh_repo}`)}>
                <Icon name="ext" size={14} />
                GitHub
              </button>
            )}
            <Link to="/projects/$id/code" params={{ id: p.id }} className="btn no-underline" title="Browse the files of this folder">
              <Icon name="folder" size={14} />
              Files
            </Link>
            <TerminalMenu cwd={p.path} phone={phone} />
            <LaunchButton ctx={{ projectId: p.id }} />
          </div>
        </div>
        </>}
        {tabs}
        {tab === "settings" ? <FolderSettings p={p} /> : <>
        {p.is_temp && <TempProjectCard p={p} />}
        <ProjectAgents title="Running agents" sessions={sessions.filter(isRunning)} empty="No running agents in this folder." />
        <RepoItems key={p.gh_repo ?? p.id} repo={p.gh_repo ?? ""} projectId={p.id} />
        {worktrees.length > 0 && (
          <>
            <div className="seclabel">Worktrees · {worktrees.length}</div>
            <div className="overflow-hidden rounded-lg border border-line">
              {worktrees.map((w) => {
                const session = sessions.filter((s) => s.cwd === w.path).sort((a, b) => b.started_at - a.started_at)[0];
                const draft = predrafts.find((d) => d.root === w.path);
                const canLaunch = draft && !draft.archived && !draft.published_url && !draft.pushed_head;
                const rowActions = <>
                  {session && <button className="btn btn-sm" title={session.title} onClick={() => { show(session.id); void navigate({ to: "/agents", search: phone ? { session: session.id } : {} }); }}>Agent</button>}
                    {canLaunch && !session && <button className="btn btn-sm btn-primary" title="Launch an agent in this worktree" onClick={() => { useComposer.getState().start({ projectId: p.id }); useComposer.getState().update((s) => ({ ...s, target: { kind: "existing_predraft", predraft_id: draft.id } })); }}><Icon name="play" size={12} />Launch here</button>}
                    <Link to="/projects/$id/code" params={{ id: p.id }} search={{ root: w.path }} className="btn btn-sm no-underline" title="Browse the files of this worktree">
                      <Icon name="folder" size={12} />
                      Files
                    </Link>
                    <TerminalMenu cwd={w.path} small phone={phone} />
                </>;
                return (
                  <div key={w.path} className="phone-worktree-row flex h-[52px] items-center gap-2.5 border-b border-line-2 px-4 last:border-0">
                    <Icon name="branch" size={14} className="text-ink-3" />
                    <span className="grid min-w-0 flex-1">
                      <span className="flex min-w-0 items-center gap-2">
                        <span className="truncate font-mono text-xs font-semibold">{w.branch ?? "detached"}</span>
                        {session && <StatusBadge s={session} />}
                        {draft && <Link to="/predrafts/$id" params={{ id: draft.id }} className="truncate text-xs text-teal-ink">Local PR · {draft.archived ? "discarded" : draft.published_number ? `published #${draft.published_number}` : draft.title}</Link>}
                      </span>
                      <span className="truncate font-mono text-[12px] text-ink-3" title={w.path}>{w.path}</span>
                    </span>
                    {phone ? <PhoneActions label="Worktree actions" children={rowActions} details={<p className="break-all font-mono text-xs">{w.branch}<br />{w.path}</p>} /> : rowActions}
                  </div>
                );
              })}
            </div>
          </>
        )}
        <ProjectAgents title="All agents" sessions={sessions} empty="No agents in this folder yet." />
        </>}
      </div>
    </div>
  );
}

/** The folder page's Settings tab: everything about this folder, saved as it changes. */
function FolderSettings({ p }: { p: Project }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const remote = useConfig().data?.remote ?? true;
  const workspaces = useWorkspaces().data ?? [];
  const name = useSaveState();
  const description = useSaveState();
  const setup = useSaveState();
  const done = useSaveState();
  const member = useSaveState();
  const [text, setText] = useState({ name: p.name, description: p.description ?? "", setup: p.worktree_setup ?? "" });
  const patch = (body: Record<string, unknown>) => api.patch(`/api/projects/${p.id}`, body).then(() => qc.invalidateQueries({ queryKey: ["projects"] }));
  const field = (key: "name" | "description" | "setup", state: ReturnType<typeof useSaveState>, body: (v: string) => Record<string, unknown>, saved: string) => ({
    value: text[key],
    onChange: (e: React.ChangeEvent<HTMLInputElement>) => setText((t) => ({ ...t, [key]: e.target.value })),
    onBlur: () => { if (text[key] !== saved && (key !== "name" || text.name.trim())) void state.run(() => patch(body(text[key]))); },
    onKeyDown: (e: React.KeyboardEvent) => { if (e.key === "Enter") (e.target as HTMLInputElement).blur(); },
  });
  return (
    <section className="rounded-lg border border-line px-4">
      <SettingRow label="Name" hint="Shown in the sidebar, the composer and @ mentions." state={name.state} error={name.error}>
        <input className="field-input min-w-0 flex-1" aria-label="Folder name" maxLength={80} {...field("name", name, (v) => ({ name: v.trim() }), p.name)} />
      </SettingRow>
      <SettingRow label="Description" hint="One line, used when the folder is given as context." state={description.state} error={description.error}>
        <input className="field-input min-w-0 flex-1" aria-label="Folder description" placeholder="Add a description" {...field("description", description, (v) => ({ description: v }), p.description ?? "")} />
      </SettingRow>
      <SettingRow label="Workspaces" hint="Windows showing these workspaces list this folder." state={member.state} error={member.error}>
        {workspaces.length === 0 ? <span className="text-[12px] text-ink-3">No workspace yet: create one from the sidebar menu.</span> : (
          <FilterChips label="Workspaces of this folder">
            {workspaces.map((w) => {
              const inside = w.project_ids.includes(p.id);
              return <FilterChip key={w.id} active={inside} onClick={() => void member.run(() => setWorkspaceProjects(w, inside ? w.project_ids.filter((x) => x !== p.id) : [...w.project_ids, p.id]))}>{inside && <Icon name="check" size={11} />}{w.name}</FilterChip>;
            })}
          </FilterChips>
        )}
      </SettingRow>
      {!p.is_temp && (
        <SettingRow label="Worktree setup" hint={remote ? "Edit on the host while remote access is on." : `Runs in each new worktree before the agent starts (${navigator.platform.startsWith("Win") ? "PowerShell" : "sh -c"}, 30 min limit).`} info="$LAUNCHPAD_SOURCE_CHECKOUT points to this checkout. A failure is reported to the agent, which starts anyway. Without a command, new worktrees start with the committed files and the ignored files listed in .worktreeinclude." state={setup.state} error={setup.error}>
          <input className="field-input min-w-0 flex-1 font-mono text-xs" aria-label="Worktree setup command" disabled={remote} placeholder="pnpm install --frozen-lockfile" {...field("setup", setup, (v) => ({ worktree_setup: v }), p.worktree_setup ?? "")} />
        </SettingRow>
      )}
      {!p.is_temp && (
        <SettingRow label="When done" hint="Default for new agents of this folder; the composer can change it per launch." state={done.state} error={done.error}>
          <FilterChips label="When done">
            <FilterChip active={!p.merge_when_done} onClick={() => p.merge_when_done && void done.run(() => patch({ merge_when_done: false }))}>Keep the branch for review</FilterChip>
            <FilterChip active={p.merge_when_done} onClick={() => !p.merge_when_done && void done.run(() => patch({ merge_when_done: true }))}>Merge when done</FilterChip>
          </FilterChips>
        </SettingRow>
      )}
      <SettingRow label={p.is_temp ? "Delete" : "Remove"} hint={p.is_temp ? "Deletes the scratch folder from disk and stops its agents." : "Unregisters the folder; files on disk are kept."}>
        {p.is_temp
          ? <ConfirmButton icon="trash" consequence={`Deletes ${p.path} from disk.`} confirmLabel="Delete files" onConfirm={() => api.delete(`/api/projects/${p.id}`).then(() => { toast(`Deleted ${p.path}`); void qc.invalidateQueries({ queryKey: ["projects"] }); void navigate({ to: "/projects" }); }, toastError)}>Delete files</ConfirmButton>
          : <ConfirmButton icon="trash" consequence="Files on disk are kept." confirmLabel="Remove from Launchpad" onConfirm={() => api.delete(`/api/projects/${p.id}`).then(() => { toast(`Removed ${p.name} from Launchpad`); void qc.invalidateQueries({ queryKey: ["projects"] }); void navigate({ to: "/projects" }); }, toastError)}>Remove from Launchpad</ConfirmButton>}
      </SettingRow>
    </section>
  );
}

function ProjectAgents({ title, sessions, empty }: { title: string; sessions: Session[]; empty: string }) {
  const phone = usePhone();
  const rowHeight = phone ? 56 : 40;
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: sessions.length,
    getScrollElement: () => ref.current,
    estimateSize: () => rowHeight,
    overscan: 3,
    useFlushSync: false,
  });
  if (!sessions.length) return <p className="m-0 text-[13px] text-ink-3"><span className="font-semibold">{title}</span> · {empty}</p>;
  return (
    <section className="grid gap-3">
      <div className="seclabel">{title} · {sessions.length}</div>
      <div ref={ref} className="overflow-auto rounded-lg border border-line" style={sessions.length ? { height: Math.min(sessions.length, 8) * rowHeight + 2 } : undefined}>
        {sessions.length === 0 ? (
          <div className="p-4 text-[13px] text-ink-2">{empty}</div>
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
  const phone = usePhone();
  const show = useLayout((s) => s.show);
  const navigate = useNavigate();
  return (
    <button
      className="phone-project-agent flex h-10 w-full items-center gap-2.5 border-b border-line-2 px-4 text-left hover:bg-chrome"
      onClick={() => {
        show(s.id);
        void navigate({ to: "/agents", search: phone ? { session: s.id } : {} });
      }}
    >
      <HarnessMark harness={s.harness} size={16} />
      <span className="min-w-0 flex-1 truncate text-[13px] font-semibold">{s.title}</span>
      <StatusBadge s={s} />
      {!phone && <span className="text-xs text-ink-3">{new Date(s.started_at).toLocaleDateString()}</span>}
    </button>
  );
});
