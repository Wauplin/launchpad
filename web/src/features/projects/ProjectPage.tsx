import { usePhone } from "@/lib/phone";
import { FolderToolDefaults } from "@/features/settings/ToolDefaults";
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
import type { HomeBranch } from "@/gen/HomeBranch";
import type { AssetInfo } from "@/gen/AssetInfo";
import type { Change } from "@/gen/Change";
import { useComposer } from "@/stores/composer";
import { RepoItems } from "@/features/github/RepoItems";
import { Link, useParams, useSearch } from "@tanstack/react-router";
import { Badge, ConfirmButton, Empty, Choice, Choices, PageHeader, SettingRow, Tabs, useSaveState } from "@/components/ui";
import { ago } from "@/lib/format";
import { moveFolder, useWorkspaces } from "@/lib/workspace";
import { toast } from "@/components/Toast";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { SessionStatus } from "@/features/agents/SessionBits";
import { useCodeSources } from "@/features/code/lib";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { api } from "@/lib/api";
import { useProject, useSessions } from "@/lib/queries";
import { useLayout } from "@/stores/layout";
import { useNavigate } from "@tanstack/react-router";
import { TerminalMenu } from "@/features/terminals/TerminalMenu";
import { Description, TEMP_INTRO, TempProjectCard } from "./ProjectsView";
import { Branches } from "./Branches";

export function ProjectPage() {
  const phone = usePhone();
  const { id } = useParams({ strict: false }) as { id: string };
  const p = useProject(id);
  const sessions = (useSessions().data ?? []).filter((s) => s.project_id === id);
  const worktrees = (useCodeSources(p?.id).data?.worktrees ?? []).filter((w) => !w.main);
  const predrafts = (useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<Predraft[]>("/api/predrafts") }).data ?? []).filter((d) => d.project_id === id);
  const show = useLayout((s) => s.show);
  const navigate = useNavigate();
  const asked = (useSearch({ strict: false }) as { tab?: string }).tab;
  const tab = asked === "settings" || asked === "branches" ? asked : "overview";
  const setTab = (t: "overview" | "branches" | "settings") => void navigate({ to: "/projects/$id", params: { id }, search: (t === "overview" ? {} : { tab: t }) as never, replace: true });
  if (!p) return <Empty fill icon="folder">This folder is not registered in Launchpad.</Empty>;
  const tabs = <Tabs tour="folder.tabs" label="Folder views" value={tab} onChange={setTab} items={[{ id: "overview", label: "Overview" }, { id: "branches", label: "Branches" }, { id: "settings", label: "Settings" }]} />;
  const running = sessions.filter(isRunning).length;
  const waiting = sessions.filter(needsYou).length;
  const open = predrafts.filter((d) => !d.archived && !d.published_url && !d.merged).length;
  const counts = `${running} running · ${waiting} waiting · ${open} Local PR${open === 1 ? "" : "s"}`;
  const buttons = <>
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
    {!p.plain && <TerminalMenu cwd={p.path} runner={p.is_temp ? p.runner_id : null} phone={phone} />}
  </>;
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="folder.page">
      {phone ? <PhoneHeader title={p.name} onBack={() => void navigate({ to: "/projects" })} backLabel="Back to folders" primary={<LaunchButton ctx={{ projectId: p.id }} />} subtitle={counts} actions={<div className="ml-auto flex gap-1.5">{buttons}</div>}
        details={<div className="grid gap-2"><span className="break-all font-mono text-[12px] text-ink-3">{p.path}</span><Description p={p} />{p.is_temp && <p className="m-0 text-[13px] text-ink-2">{TEMP_INTRO}</p>}</div>} />
        : <PageHeader title={p.name} introId="temp-folder" intro={p.is_temp ? TEMP_INTRO : undefined} tabs={tabs} actions={<>{buttons}<LaunchButton ctx={{ projectId: p.id }} /></>} meta={<>
          <span className="truncate font-mono" title={p.path}>{p.path}</span>
          <span>· {counts}</span>
          {p.description && <span className="truncate">· {p.description}</span>}
        </>} />}
      <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="phone-page-content grid max-w-page grid-cols-[minmax(0,1fr)] gap-4 px-5 py-5">
        {phone && tabs}
        {tab === "settings" ? <FolderSettings p={p} /> : tab === "branches" ? <Branches projectId={p.id} /> : <>
        {p.is_temp && <TempProjectCard p={p} />}
        <ProjectAgents title="Running agents" sessions={sessions.filter(isRunning)} empty="No running agents in this folder." />
        <RepoItems key={p.gh_repo ?? p.id} repo={p.gh_repo ?? ""} projectId={p.id} />
        {worktrees.length > 0 && (
          <>
            <div className="seclabel">Worktrees · {worktrees.length}</div>
            <div className="overflow-hidden rounded-card border border-line">
              {worktrees.map((w) => {
                const session = sessions.filter((s) => s.cwd === w.path).sort((a, b) => b.started_at - a.started_at)[0];
                const draft = predrafts.find((d) => d.project_id === p.id && d.branch === w.branch);
                const canLaunch = draft && !draft.archived && !draft.published_url && !draft.merged && !draft.pushed_head;
                const rowActions = <>
                  {session && <button className="btn btn-sm" title={session.title} onClick={() => { show(session.id); void navigate({ to: "/agents", search: phone ? { session: session.id } : {} }); }}>Agent</button>}
                    {canLaunch && !session && <button className="btn btn-sm" title="Launch an agent in this worktree" onClick={() => { useComposer.getState().start({ projectId: p.id }); useComposer.getState().update((s) => ({ ...s, target: { kind: "existing_predraft", predraft_id: draft.id } })); }}><Icon name="play" size={12} />Launch</button>}
                    <Link to="/projects/$id/code" params={{ id: p.id }} search={{ root: w.path }} className="btn btn-sm no-underline" title="Browse the files of this worktree">
                      <Icon name="folder" size={12} />
                      Files
                    </Link>
                    <TerminalMenu cwd={w.path} runner={session?.runner_id} small phone={phone} />
                </>;
                return (
                  <div key={w.path} className="phone-worktree-row flex h-[52px] items-center gap-2.5 border-b border-line-2 px-4 last:border-0">
                    <Icon name="branch" size={14} className="text-ink-3" />
                    <span className="grid min-w-0 flex-1">
                      <span className="flex min-w-0 items-center gap-2">
                        <span className="truncate font-mono text-[12px] font-semibold">{w.branch ?? "detached"}</span>
                        {session && <SessionStatus s={session} />}
                        {draft && <Link to="/predrafts/$id" params={{ id: draft.id }} className="truncate text-[12px] text-teal-ink">Local PR · {draft.archived ? "discarded" : draft.published_number ? `published #${draft.published_number}` : draft.title}</Link>}
                      </span>
                      <span className="truncate font-mono text-[12px] text-ink-3" title={w.path}>{w.path}</span>
                    </span>
                    {phone ? <PhoneActions label="Worktree actions" children={rowActions} details={<p className="break-all font-mono text-[12px]">{w.branch}<br />{w.path}</p>} /> : rowActions}
                  </div>
                );
              })}
            </div>
          </>
        )}
        {p.home_git && <HomeBranches projectId={p.id} predrafts={predrafts} />}
        {p.home_git && <AssetFolders projectId={p.id} />}
        {p.plain && <PlainHistory projectId={p.id} sessions={sessions} />}
        <ProjectAgents title="All agents" sessions={sessions} empty="No agents in this folder yet." />
        </>}
      </div>
      </div>
    </div>
  );
}

/** The folder page's Settings tab: everything about this folder, saved as it changes. */
function FolderSettings({ p }: { p: Project }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
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
    <section className="rounded-card border border-line px-4">
      <SettingRow label="Name" hint="Shown in the sidebar, the composer and @ mentions." state={name.state} error={name.error}>
        <input className="field-input min-w-0 flex-1" aria-label="Folder name" maxLength={80} {...field("name", name, (v) => ({ name: v.trim() }), p.name)} />
      </SettingRow>
      <SettingRow label="Description" hint="One line, used when the folder is given as context." state={description.state} error={description.error}>
        <input className="field-input min-w-0 flex-1" aria-label="Folder description" placeholder="Add a description" {...field("description", description, (v) => ({ description: v }), p.description ?? "")} />
      </SettingRow>
      <SettingRow label="Workspace" hint="Every folder belongs to one workspace. Its agents, schedules and inbox threads follow it." state={member.state} error={member.error}>
        {workspaces.length === 0 ? <span className="text-[12px] text-ink-3">No workspace yet: create one from the workspace menu.</span> : (
          <Choices label="Workspace of this folder">
            {workspaces.map((w) => {
              const inside = p.workspace_id === w.id;
              return <Choice key={w.id} active={inside} onClick={() => !inside && void member.run(() => moveFolder(p, w).then(() => qc.invalidateQueries({ queryKey: ["projects"] })))}>{inside && <Icon name="check" size={12} />}{w.name}</Choice>;
            })}
          </Choices>
        )}
      </SettingRow>
      {!p.is_temp && (
        <SettingRow label="Worktree setup" hint="Runs in each new worktree before the agent starts, in the agent's jail with its tools (sh -c, 30 min limit)." info="$LAUNCHPAD_WORKTREE is the new worktree and $LAUNCHPAD_SOURCE_CHECKOUT the runner's copy of this folder. A failure is reported to the agent, which starts anyway." state={setup.state} error={setup.error}>
          <input className="field-input min-w-0 flex-1 font-mono text-[12px]" aria-label="Worktree setup command" placeholder="pnpm install --frozen-lockfile" {...field("setup", setup, (v) => ({ worktree_setup: v }), p.worktree_setup ?? "")} />
        </SettingRow>
      )}
      {!p.is_temp && (
        <SettingRow label="When done" hint="Default for new agents of this folder; the composer can change it per launch." state={done.state} error={done.error}>
          <Choices label="When done">
            {([["keep", "Keep the branch for review"], ["local_pr", "Open a Local PR"], ["merge", "Merge when done"]] as const).map(([value, label]) => (
              <Choice key={value} active={p.when_done === value} onClick={() => p.when_done !== value && void done.run(() => patch({ when_done: value }))}>{label}</Choice>
            ))}
          </Choices>
        </SettingRow>
      )}
      {!p.is_temp && (
        <SettingRow label="Default tools" hint="What new agents of this folder get, over the workspace's defaults; the composer can change it per launch.">
          <FolderToolDefaults folderId={p.id} workspaceId={p.workspace_id} />
        </SettingRow>
      )}
      {p.plain && (
        <SettingRow label="Git" hint="Track this folder with Git: its current files become the first commit of a new Git folder on Home, without a remote. Agents then work in their own worktrees and branches, with Merge and Local PRs. Earlier versions stay in the trash until they expire.">
          <ConfirmButton tone="publish" consequence="Commits the current files to a new Git folder. This cannot be turned back into a plain folder." confirmLabel="Turn into a Git folder" onConfirm={() => api.post(`/api/projects/${p.id}/to-git`, {}).then(() => { toast(`${p.name} is a Git folder now`); return qc.invalidateQueries({ queryKey: ["projects"] }); }, toastError)}>Turn into a Git folder</ConfirmButton>
        </SettingRow>
      )}
      <SettingRow label={p.is_temp ? "Delete" : "Remove"} hint={p.is_temp ? "Deletes the temporary folder from disk and stops its agents. Archiving its last agent removes it too. Home keeps a copy of its files 30 days, for a resume." : "Unregisters the folder; files on disk are kept."}>
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
      <div ref={ref} className="overflow-auto rounded-card border border-line" style={sessions.length ? { height: Math.min(sessions.length, 8) * rowHeight + 2 } : undefined}>
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
      <SessionStatus s={s} />
      {!phone && <span className="text-[12px] text-ink-3">{ago(s.started_at)}</span>}
    </button>
  );
});

/** Asset folders of a Git folder: large or generated files stored by Home. Unused ones go to the trash after 14 days. */
function AssetFolders({ projectId }: { projectId: string }) {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["plain", "assets", projectId], queryFn: () => api.get<AssetInfo[]>(`/api/plain/assets/${projectId}`) });
  const rows = q.data ?? [];
  if (q.isError || !rows.length) return null;
  const untrash = (a: AssetInfo) => api.post(`/api/plain/${a.asset.id}/untrash`, {}).then(() => { toast(`${a.asset.name} restored`); return qc.invalidateQueries({ queryKey: ["plain", "assets", projectId] }); }, toastError);
  const mb = (n: number) => n < 1 << 20 ? `${Math.max(1, Math.round(n / 1024))} KB` : `${(n / (1 << 20)).toFixed(1)} MB`;
  return <>
    <div className="seclabel">Asset folders · {rows.length}</div>
    <span className="text-[12px] text-ink-3">Large or generated files stored by Home instead of Git. A folder that no branch, kept tip or note names, unchanged for 14 days, goes to the trash; Restore works for 30 days.</span>
    <div className="max-h-[320px] overflow-y-auto rounded-card border border-line">
      {rows.map((a) => <div key={a.asset.id} className="flex min-h-[44px] items-center gap-2.5 border-b border-line-2 px-4 text-[12px]">
        <Icon name="folder" size={14} className="text-ink-3" />
        <span className="grid min-w-0 flex-1">
          <span className="flex min-w-0 items-center gap-2"><span className="truncate font-mono font-semibold">{a.asset.name}</span>{a.asset.trashed_at ? <Badge tone="attn">in the trash</Badge> : !a.in_use && <Badge title="No branch, kept tip or note names it">unused</Badge>}</span>
          <span className="truncate text-[12px] text-ink-3">{a.asset.trashed_at ? `Moved to the trash ${ago(a.asset.trashed_at)}` : `${a.files} files · ${mb(a.size)}`} · changed {ago(a.last_change)}</span>
        </span>
        {a.asset.trashed_at && <ConfirmButton tone="publish" consequence={`Puts the files of ${a.asset.name} back.`} confirmLabel="Restore" onConfirm={() => untrash(a)}>Restore</ConfirmButton>}
      </div>)}
    </div>
  </>;
}

const BRANCH_ROW = 52;

/** Launchpad branches on Home Git, with every previous tip (kept 30 days) and Restore. */
function HomeBranches({ projectId, predrafts }: { projectId: string; predrafts: Predraft[] }) {
  const q = useQuery({ queryKey: ["home-git", projectId, "branches"], queryFn: () => api.get<HomeBranch[]>(`/api/home-git/${projectId}/branches`) });
  const [open, setOpen] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const rows = q.data ?? [];
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => BRANCH_ROW, overscan: 6, useFlushSync: false });
  if (q.isError || !rows.length) return null;
  const selected = rows.find((b) => b.name === open);
  return <>
    <div className="seclabel">Launchpad branches · {rows.length}</div>
    <div ref={scrollRef} className="max-h-[312px] overflow-y-auto rounded-card border border-line">
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((r) => <div key={rows[r.index].name} className="absolute inset-x-0" style={{ top: r.start, height: BRANCH_ROW }}>
          <BranchRow b={rows[r.index]} draft={predrafts.find((d) => d.branch === rows[r.index].name)} open={open === rows[r.index].name} onToggle={() => setOpen((o) => o === rows[r.index].name ? null : rows[r.index].name)} />
        </div>)}
      </div>
    </div>
    {selected && <KeptTips projectId={projectId} b={selected} />}
  </>;
}

const BranchRow = memo(function BranchRow({ b, draft, open, onToggle }: { b: HomeBranch; draft?: Predraft; open: boolean; onToggle: () => void }) {
  return <div className="flex h-full items-center gap-2.5 border-b border-line-2 px-4 text-[12px]">
    <Icon name="branch" size={14} className="text-ink-3" />
    <span className="grid min-w-0 flex-1">
      <span className="flex min-w-0 items-center gap-2"><span className="truncate font-mono font-semibold">{b.name}</span>{draft && <Link to="/predrafts/$id" params={{ id: draft.id }} className="truncate text-teal-ink">Local PR · {draft.title}</Link>}</span>
      <span className="truncate text-[12px] text-ink-3">{b.head ? `${b.head.slice(0, 8)} ${b.subject}` : "deleted"} · {ago(b.committed_at)}</span>
    </span>
    {b.kept.length > 0 && <button className="btn btn-sm" aria-expanded={open} onClick={onToggle}>{b.kept.length} previous tip{b.kept.length === 1 ? "" : "s"}</button>}
  </div>;
});

function KeptTips({ projectId, b }: { projectId: string; b: HomeBranch }) {
  const qc = useQueryClient();
  const restore = (commit: string) => api.post(`/api/home-git/${projectId}/restore`, { branch: b.name, commit }).then(() => { toast(`${b.name} restored to ${commit.slice(0, 8)}`); return qc.invalidateQueries({ queryKey: ["home-git", projectId] }); }, toastError);
  return <div className="grid gap-1 rounded-card border border-line p-3 text-[12px]">
    <b className="font-mono">{b.name}: previous tips</b>
    <span className="text-ink-3">Kept for 30 days after the branch moved away from them. Restore points the branch back; the current tip is kept in turn.</span>
    {b.kept.map((k) => <div key={`${k.commit}${k.replaced_at}`} className="flex min-h-[32px] items-center gap-2">
      <span className="min-w-0 flex-1 truncate"><span className="font-mono">{k.commit.slice(0, 8)}</span> {k.subject} <span className="text-ink-3">· replaced {ago(k.replaced_at)}</span></span>
      <ConfirmButton consequence={`Points ${b.name} on Home back to ${k.commit.slice(0, 8)}. Runners fetch it on their next sync; the current tip stays restorable.`} confirmLabel="Restore" onConfirm={() => restore(k.commit)}>Restore</ConfirmButton>
    </div>)}
  </div>;
}

const CHANGE_ROW = 40;

/** A plain folder's changes on Home: previous versions and deleted files of the last 30 days, with Restore. */
function PlainHistory({ projectId, sessions }: { projectId: string; sessions: Session[] }) {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["plain", projectId, "history"], queryFn: () => api.get<{ changes: Change[]; locked_by: string | null }>(`/api/plain/${projectId}/history`) });
  const scrollRef = useRef<HTMLDivElement>(null);
  const rows = q.data?.changes ?? [];
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => CHANGE_ROW, overscan: 8, useFlushSync: false });
  if (q.isError || !q.data) return null;
  const holder = sessions.find((s) => s.id === q.data.locked_by);
  const restore = (c: Change) => api.post(`/api/plain/${projectId}/restore`, { change: c.id }).then(() => { toast(`${c.path} restored`); return qc.invalidateQueries({ queryKey: ["plain", projectId] }); }, toastError);
  return <>
    <div className="seclabel">Versions and trash · {rows.length}</div>
    {q.data.locked_by && <span className="text-[12px] text-ink-3">In use by {holder ? `the agent "${holder.title}"` : "an agent"}: Restore works once it is archived.</span>}
    {rows.length === 0 ? <Empty icon="folder">No change yet. Agents' changes reach Home at the end of every turn.</Empty> : <div ref={scrollRef} className="max-h-[320px] overflow-y-auto rounded-card border border-line">
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((r) => <div key={rows[r.index].id} className="absolute inset-x-0" style={{ top: r.start, height: CHANGE_ROW }}>
          <ChangeRow c={rows[r.index]} locked={!!q.data!.locked_by} onRestore={restore} />
        </div>)}
      </div>
    </div>}
  </>;
}

const ChangeRow = memo(function ChangeRow({ c, locked, onRestore }: { c: Change; locked: boolean; onRestore: (c: Change) => unknown }) {
  const what = !c.sha256 ? "created" : !c.new_sha256 ? "deleted" : "changed";
  return <div className="flex h-full items-center gap-2 border-b border-line-2 px-4 text-[12px]">
    <span className="min-w-0 flex-1 truncate"><span className="font-mono">{c.path}</span> <span className="text-ink-3">· {what} · {ago(c.at)}</span></span>
    {c.sha256 && <ConfirmButton disabled={locked} consequence={what === "deleted" ? `Puts ${c.path} back.` : `Puts back the version of ${c.path} from before this change. The current version stays restorable.`} confirmLabel="Restore" onConfirm={() => onRestore(c)}>{what === "deleted" ? "Restore" : "Restore previous"}</ConfirmButton>}
  </div>;
});
