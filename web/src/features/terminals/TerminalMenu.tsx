// The Terminal split button used at every Terminal entry point: the main part opens a shell in the default
// folder (the checkout when the worktree was merged or removed), ▾ opens one shared folder menu:
// This worktree · Checkout · other worktrees of that folder · Other folder….

import { useState, type ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { Loading, ErrorState } from "@/components/ui";
import { ProjectPicker } from "@/features/launch/ProjectPicker";
import { useScope } from "@/lib/workspace";
import { useOpenTerminal, useTerminalFolders } from "./lib";

/** The folder choices for `cwd`. `onPick` receives the folder to open a shell in. */
export function FolderMenu({ cwd, onPick }: { cwd: string | null; onPick: (path: string) => void }) {
  const folders = useTerminalFolders(cwd);
  const scope = useScope();
  const [other, setOther] = useState(!cwd);
  if (other) {
    const projects = [...scope.projects].sort((a, b) => a.name.localeCompare(b.name));
    return <ProjectPicker projects={projects} hasProject={scope.hasProject} temporary={false} emptyMessage={scope.workspace ? "No folders in this workspace. Add one from the sidebar." : "No folder yet. Add one from the sidebar."} onPick={(p) => { if (p) onPick(p.path); }} />;
  }
  const f = folders.data;
  return (
    <>
      {folders.isPending && <Loading what="folders" />}
      {folders.error && <ErrorState error={folders.error} />}
      {f?.notice && <div className="flex items-start gap-1.5 px-[9px] py-1.5 text-[12px] text-ink-2" role="note"><Icon name="info" size={13} className="mt-px flex-none text-ink-3" /><span>{f.notice}</span></div>}
      {f?.current && <MenuItem
        icon={<Icon name="branch" size={14} />}
        label={f.checkout ? `This worktree · ${f.current.label}` : `This folder · ${f.current.label}`}
        detail={f.current.reason ?? f.current.branch ?? undefined}
        disabled={!f.current.available}
        active={f.default === f.current.path}
        onClick={() => onPick(f.current!.path)}
      />}
      {f?.checkout && <MenuItem
        icon={<Icon name="folder" size={14} />}
        label={`Checkout${f.checkout.branch ? ` · ${f.checkout.branch}` : ""}`}
        detail={f.checkout.reason ?? f.project_name ?? undefined}
        disabled={!f.checkout.available}
        active={f.default === f.checkout.path}
        onClick={() => onPick(f.checkout!.path)}
      />}
      {(f?.worktrees.length ?? 0) > 0 && <div className="px-[9px] pt-1.5 pb-0.5 text-[11px] font-semibold text-ink-3">Other worktrees</div>}
      {f?.worktrees.map((w) => <MenuItem key={w.path} icon={<Icon name="branch" size={14} />} label={w.label} detail={w.branch ?? undefined} onClick={() => onPick(w.path)} />)}
      <div className="my-0.5 border-t border-line-2" />
      <MenuItem icon={<Icon name="more" size={14} />} label="Other folder…" onClick={() => setOther(true)} />
    </>
  );
}

/**
 * Split button: Terminal opens a shell in the default folder of `cwd`, ▾ chooses another folder.
 * Without `cwd` the main part opens the folder menu's Other folder list.
 */
export function TerminalMenu({ cwd, label = "Terminal", icon = "term", small, phone, viewport = phone, className = "", align = "right", up, title, disabled }: {
  cwd: string | null;
  label?: ReactNode;
  icon?: IconName;
  small?: boolean;
  phone?: boolean;
  viewport?: boolean;
  className?: string;
  align?: "left" | "right";
  up?: boolean;
  title?: string;
  disabled?: boolean;
}) {
  const open = useOpenTerminal();
  const size = small ? "btn-sm" : "";
  return (
    <Popover
      align={align}
      up={up}
      width={300}
      viewport={viewport}
      label="Open a shell in"
      className={`inline-flex ${className}`}
      trigger={({ open: menuOpen, toggle }) => (
        <span className="inline-flex" data-tour="terminal.menu">
          <button type="button" className={`btn ${size} rounded-r-none`} disabled={disabled} title={title ?? (cwd ? `Open a shell in ${cwd}` : "Open a shell")} onClick={() => (cwd ? void open.folder(cwd) : toggle())}>
            <Icon name={icon} size={small ? 12 : 14} />
            {label}
          </button>
          <button type="button" className={`btn ${size} rounded-l-none border-l-0 px-1.5!`} disabled={disabled} aria-label="Choose the folder" title="Choose the folder" aria-haspopup="dialog" aria-expanded={menuOpen} onClick={toggle}>
            <Icon name="chev" size={12} />
          </button>
        </span>
      )}
    >
      {(close) => <FolderMenu cwd={cwd} onPick={(path) => { close(); void open.shell(path); }} />}
    </Popover>
  );
}
