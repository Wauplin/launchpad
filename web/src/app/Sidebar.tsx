import { useInboxNeeds } from "@/features/inbox/data";
import { useReviewCount } from "@/features/review/data";
import { useAskCount } from "@/features/ask/count";
import { openWindow, newWindowWord, useScope } from "@/lib/workspace";
import { Link, useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { memo, useRef, useState, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Project } from "@/gen/Project";
import type { Workspace } from "@/gen/Workspace";
import { isRunning, isQuotaPaused, needsYou } from "@/lib/sessionStatus";
import type { Session } from "@/gen/Session";
import { Icon, type IconName } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toastError } from "@/components/Toast";
import { ConfirmButton } from "@/components/ui";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useProjects, useSessions } from "@/lib/queries";
import { useTheme } from "@/stores/theme";
import { useWorkspace } from "@/stores/workspace";
import { useComposer } from "@/stores/composer";
import { useOpenTerminal } from "@/features/terminals/lib";
import { useAddFolder } from "@/features/projects/addFolderStore";

function NavItem({ to, icon, children, end }: { to: string; icon: IconName; children: ReactNode; end?: ReactNode }) {
  return (
    <Link
      to={to}
      className="flex min-h-[31px] flex-none items-center gap-[9px] rounded-[7px] px-[9px] py-1 text-ink-2 no-underline hover:bg-sunk"
      activeProps={{ className: "!bg-paper !text-ink font-semibold shadow-[inset_0_0_0_1px_var(--color-line)]" }}
      activeOptions={{ exact: to === "/" }}
    >
      <Icon name={icon} />
      {children}
      {end && <span className="ml-auto">{end}</span>}
    </Link>
  );
}

export function SideLabel({ children, end }: { children: ReactNode; end?: ReactNode }) {
  return <div className="flex min-h-[28px] items-center gap-1 px-[9px] pt-3 pb-0.5"><span className="seclabel min-w-0 flex-1 truncate">{children}</span>{end}</div>;
}

const inProject = (p: Project, s: Session) => s.project_id === p.id || s.cwd === p.path || s.cwd.startsWith(p.path + "/");
/** Needs you, quota paused or running: the states the sidebar marks. */
const active = (s: Session) => needsYou(s) || isQuotaPaused(s) || isRunning(s);

function RunDot({ p, sessions }: { p: Project; sessions: Session[] }) {
  const active = sessions.filter((s) => inProject(p, s));
  return <span className="flex items-center gap-1.5">
    {active.some(needsYou) && <span className="size-[7px] rounded-full bg-attn" title="Needs you" />}
    {active.some(isQuotaPaused) && <span className="size-[7px] rounded-full border border-attn" title="Quota paused" />}
    {active.some(isRunning) && <span className="lp-spin" title="Running" />}
  </span>;
}

const ROW = 28;

/** A folder row: the link, its state dots, and quick actions on hover or focus (always visible on touch): Files · Terminal · Launch. */
const FolderLink = memo(function FolderLink({ p, sessions }: { p: Project; sessions: Session[] }) {
  const navigate = useNavigate();
  const terminal = useOpenTerminal();
  const start = useComposer((s) => s.start);
  const name = p.is_temp ? p.name.replace(/^tmp-\d{4}-/, "") || p.name : p.name;
  return (
    <div className="group/folder relative flex h-[28px] items-center rounded-md hover:bg-sunk focus-within:bg-sunk">
      <Link
        to="/projects/$id"
        params={{ id: p.id }}
        className={`flex h-full min-w-0 flex-1 items-center gap-2 rounded-md px-[9px] no-underline ${p.is_temp ? "text-[12px] text-ink-2" : "text-[13px] text-ink"}`}
        activeProps={{ className: "!bg-paper shadow-[inset_0_0_0_1px_var(--color-line)]" }}
        activeOptions={{ exact: true }}
        title={p.is_temp ? `${p.name} · created ${formatAge(p.created_at)} ago` : p.path}
      >
        <span className="min-w-0 truncate">{name}</span>
        <span className="ml-auto group-hover/folder:hidden group-focus-within/folder:hidden [@media(hover:none)]:hidden"><RunDot p={p} sessions={sessions} /></span>
      </Link>
      <span className="absolute right-0.5 hidden items-center group-hover/folder:flex group-focus-within/folder:flex [@media(hover:none)]:static [@media(hover:none)]:flex">
        <button className="iconbtn size-[22px]!" title={`Files of ${name}`} aria-label={`Files of ${name}`} onClick={() => void navigate({ to: "/projects/$id/code", params: { id: p.id } })}><Icon name="folder" size={13} /></button>
        <button className="iconbtn size-[22px]!" title={`Terminal in ${name}`} aria-label={`Terminal in ${name}`} onClick={() => void terminal.shell(p.path)}><Icon name="term" size={13} /></button>
        <button className="iconbtn size-[22px]!" title={`Launch an agent in ${name}`} aria-label={`Launch an agent in ${name}`} onClick={() => start({ projectId: p.id })}><Icon name="play" size={13} /></button>
      </span>
    </div>
  );
});

/** The workspace section's menu: New workspace, Rename, Open in new window and Delete. */
function WorkspaceMenu({ workspace }: { workspace: Workspace | null }) {
  const qc = useQueryClient();
  const switchTo = useWorkspace((s) => s.switchTo);
  const [mode, setMode] = useState<"menu" | "new" | "rename">("menu");
  const [name, setName] = useState("");
  const done = () => void qc.invalidateQueries({ queryKey: ["workspaces"] });
  const submit = async (close: () => void) => {
    const n = name.trim();
    if (!n) return;
    try {
      if (mode === "new") {
        const w = await api.post<Workspace>("/api/workspaces", { name: n, project_ids: [] });
        switchTo(w.id);
      } else if (workspace) await api.patch(`/api/workspaces/${workspace.id}`, { name: n });
      done();
      close();
      setMode("menu");
    } catch (e) { toastError(e); }
  };
  return (
    <Popover width={260} align="right" trigger={({ open, toggle }) => (
      <button className="iconbtn size-[22px]!" aria-label="Workspace actions" title="Workspace actions" aria-expanded={open} onClick={() => { setMode("menu"); toggle(); }}><Icon name="more" size={14} /></button>
    )}>
      {(close) => mode !== "menu" ? (
        <form className="grid gap-2 p-1.5" onSubmit={(e) => { e.preventDefault(); void submit(close); }}>
          <label className="grid gap-1 text-[12px] text-ink-2">{mode === "new" ? "New workspace name" : `Rename ${workspace?.name}`}<input autoFocus className="field-input h-[30px]" maxLength={40} value={name} onChange={(e) => setName(e.target.value)} /></label>
          <span className="flex justify-end gap-1.5"><button type="button" className="btn btn-sm btn-ghost" onClick={() => setMode("menu")}>Cancel</button><button type="submit" className="btn btn-sm btn-primary" disabled={!name.trim()}>{mode === "new" ? "Create and show here" : "Rename"}</button></span>
        </form>
      ) : (
        <>
          <MenuItem icon={<Icon name="plus" size={13} />} label="New workspace" detail="A named set of folders" onClick={() => { setName(""); setMode("new"); }} />
          {workspace && <MenuItem icon={<Icon name="pencil" size={13} />} label="Rename" onClick={() => { setName(workspace.name); setMode("rename"); }} />}
          <MenuItem icon={<Icon name="window" size={13} />} label={`Open in new ${newWindowWord}`} onClick={() => { close(); void openWindow(workspace?.id ?? null); }} />
          {workspace && (
            <div className="border-t border-line-2 px-2 pt-1.5 pb-1">
              <ConfirmButton icon="trash" consequence="Folders and agents are kept." confirmLabel={`Delete ${workspace.name}`} onConfirm={async () => { try { await api.delete(`/api/workspaces/${workspace.id}`); done(); close(); } catch (e) { toastError(e); } }}>Delete workspace</ConfirmButton>
            </div>
          )}
        </>
      )}
    </Popover>
  );
}

/** Folders of the workspace (or every folder in Everything), virtualized with fixed row heights. */
function FolderList({ folders, sessions }: { folders: Project[]; sessions: Session[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: folders.length, getScrollElement: () => ref.current, estimateSize: () => ROW, overscan: 6, useFlushSync: false });
  return (
    <div ref={ref} className="min-h-0 overflow-y-auto" style={{ maxHeight: Math.min(folders.length, 14) * ROW }}>
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((it) => <div key={folders[it.index].id} className="absolute inset-x-0" style={{ top: it.start, height: ROW }}><FolderLink p={folders[it.index]} sessions={sessions} /></div>)}
      </div>
    </div>
  );
}

export function Sidebar() {
  const { theme, toggle } = useTheme();
  const needs = useInboxNeeds();
  const toReview = useReviewCount();
  const proposals = useAskCount();
  const scope = useScope();
  const all = useProjects().data ?? [];
  const showAdd = useAddFolder((s) => s.show);
  // A workspace lists its folders; Everything lists all of them.
  const projects = scope.workspace ? scope.projects : all;
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const [allTemp, setAllTemp] = useState(false);
  const waiting = sessions.filter(needsYou).length;
  const running = sessions.filter(isRunning).length;
  const quotaPaused = sessions.filter(isQuotaPaused).length;
  const folders = projects.filter((p) => !p.is_temp).sort((a, b) => a.name.localeCompare(b.name));
  const temp = projects.filter((p) => p.is_temp).sort((a, b) => b.created_at - a.created_at);
  // Scratch folders: only those with agents that need you, run or wait for quota, unless expanded.
  const tempActive = temp.filter((p) => sessions.some((s) => inProject(p, s) && active(s)));
  const shownTemp = allTemp ? temp : tempActive;
  return (
    <aside className="flex w-[200px] flex-none flex-col gap-px overflow-y-auto border-r border-line bg-chrome px-2 py-2.5">
      <NavItem to="/ask" icon="spark" end={proposals > 0 ? <span className="rounded-full bg-teal-soft px-1.5 text-[11px] font-semibold text-teal-ink tabular-nums" title={`${proposals} proposals wait for your tap`}>{proposals}</span> : undefined}>
        Ask
      </NavItem>
      <NavItem to="/" icon="inbox" end={needs > 0 ? <span className="rounded-full bg-attn-soft px-1.5 text-[11px] font-semibold text-attn tabular-nums" title={`${needs} for you: unread review requests, mentions and assignments. The Inbox "For you" filter lists them.`} aria-label={`${needs} for you`}>{needs}</span> : undefined}>
        Inbox
      </NavItem>
      <NavItem to="/agents" icon="agents">
        <span className="min-w-0 flex-1">
          <span className="block">Agents</span>
          {(waiting > 0 || quotaPaused > 0 || running > 0) && (
            <span className="mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-[11px] font-semibold tabular-nums">
              {waiting > 0 && <span className="rounded-full bg-attn-soft px-1.5 whitespace-nowrap text-attn" title={`${waiting} need you: questions and unseen Done, Failed or Interrupted sessions`}>{waiting} waiting</span>}
              {quotaPaused > 0 && <span className="rounded-full bg-attn-soft px-1.5 whitespace-nowrap text-attn" title={`${quotaPaused} quota paused`}>{quotaPaused} paused</span>}
              {running > 0 && <span className="flex items-center gap-1 whitespace-nowrap text-work" title={`${running} running`}><span className="lp-spin" />{running} running</span>}
            </span>
          )}
        </span>
      </NavItem>
      <NavItem to="/review" icon="pr-local" end={toReview > 0 ? <span className="text-[11px] font-semibold text-ink-3 tabular-nums" title={`${toReview} to review: suggestions and Local PRs to publish, agent branches and review requests`}>{toReview}</span> : undefined}>
        Review
      </NavItem>
      <NavItem to="/projects" icon="folder">
        Folders
      </NavItem>
      <SideLabel end={<>
        <button className="iconbtn size-[22px]!" title={`Add folders${scope.workspace ? ` to ${scope.workspace.name}` : ""}`} aria-label="Add folders" onClick={showAdd}><Icon name="plus" size={13} /></button>
        <WorkspaceMenu workspace={scope.workspace} />
      </>}>{scope.workspace?.name ?? "Everything"}</SideLabel>
      {folders.length === 0 && <button className="mx-[9px] my-1 text-left text-[12px] text-teal-ink hover:underline" onClick={showAdd}>+ Add folder</button>}
      <FolderList folders={folders} sessions={sessions} />
      {temp.length > 0 && <SideLabel>{allTemp ? "Scratch folders" : "Scratch folders · with agents"}</SideLabel>}
      {shownTemp.map((p) => (
        <FolderLink key={p.id} p={p} sessions={sessions} />
      ))}
      {temp.length > tempActive.length && (
        <button className="h-[25px] rounded-md px-[9px] text-left text-[12px] text-ink-3 hover:bg-sunk" aria-expanded={allTemp} onClick={() => setAllTemp(!allTemp)}>
          {allTemp ? "Show only folders with agents" : `+ ${temp.length - tempActive.length} more scratch ${temp.length - tempActive.length === 1 ? "folder" : "folders"}`}
        </button>
      )}
      <div className="mt-auto flex items-center gap-1 pt-2">
        <div className="min-w-0 flex-1">
          <NavItem to="/settings" icon="sliders">
            Settings
          </NavItem>
        </div>
        <button type="button" className="flex size-[31px] flex-none items-center justify-center rounded-[7px] text-ink-2 hover:bg-sunk" aria-label="Dark theme" aria-pressed={theme === "dark"} title={`Switch to ${theme === "light" ? "dark" : "light"} theme on this device`} onClick={toggle}>
          <Icon name={theme === "dark" ? "moon" : "sun"} size={15} />
        </button>
      </div>
    </aside>
  );
}
