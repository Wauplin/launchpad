// Shared context picker for new tasks and follow-up messages.
import { useState } from "react";
import type { AgentTraceRef } from "@/gen/AgentTraceRef";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { ProjectPicker } from "./ProjectPicker";
import { AgentTracePicker } from "./AgentTracePicker";

function GitHubLinkForm({ onAdd, onDone }: { onAdd: (r: GitHubRef) => void; onDone: () => void }) {
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const add = async () => {
    if (!url.trim()) return;
    setBusy(true);
    try {
      onAdd(await api.post<GitHubRef>("/api/gh/resolve", { url: url.trim() }));
      onDone();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="grid gap-1.5 p-1">
      <input
        autoFocus
        className="field-input h-7 text-xs"
        placeholder="https://github.com/owner/repo/issues/123"
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void add();
        }}
      />
      <span className="flex justify-end gap-1.5">
        <button className="btn btn-sm btn-ghost" onClick={onDone}>Cancel</button>
        <button className="btn btn-sm btn-primary" disabled={busy || !url.trim()} onClick={() => void add()}>{busy ? "Adding…" : "Add"}</button>
      </span>
    </div>
  );
}

/** The "+ Add context" popover: one menu for projects, GitHub links, agent traces and recent contexts. */
export function AddContextMenu({ projects, recent, sessions, selectedTraces, onProject, onGitHub, onToggleTrace, disabled, compact = false }: {
  disabled?: boolean;
  compact?: boolean;
  projects: Project[];
  recent: { key: string; label: string; add: () => void }[];
  sessions: Session[];
  selectedTraces: AgentTraceRef[];
  onProject: (p: Project) => void;
  onGitHub: (r: GitHubRef) => void;
  onToggleTrace: (t: AgentTraceRef) => void;
}) {
  const [mode, setMode] = useState<"menu" | "project" | "github" | "trace">("menu");
  return (
    <Popover
      width={340}
      viewport
      label="Add context"
      trigger={({ open, toggle }) => (
        <button disabled={disabled} aria-haspopup="dialog" aria-expanded={open} className={`btn btn-sm ${compact ? "btn-ghost" : ""}`} onClick={() => { setMode("menu"); toggle(); }}>
          <Icon name="plus" size={12} />
          Add context
        </button>
      )}
    >
      {(close) => {
        const back = <MenuItem icon={<Icon name="chev" size={13} className="rotate-90" />} label="Back" onClick={() => setMode("menu")} />;
        if (mode === "project") return <>{back}<ProjectPicker projects={projects} hasProject={() => true} temporary={false} emptyMessage="No other folder." onPick={(p) => { if (p) { onProject(p); close(); } }} /></>;
        if (mode === "github") return <>{back}<GitHubLinkForm onAdd={onGitHub} onDone={close} /></>;
        if (mode === "trace") return <>{back}<AgentTracePicker sessions={sessions} selected={selectedTraces} onToggle={onToggleTrace} /><div className="flex items-center justify-between px-2 pt-1 text-xs text-ink-3"><span>{selectedTraces.length} selected</span><button className="btn btn-sm btn-primary" onClick={close}>Done</button></div></>;
        return (
          <>
            <MenuItem icon={<Icon name="folder" size={13} />} label="Folder…" detail="Another folder as read-only context" onClick={() => setMode("project")} />
            <MenuItem icon={<Icon name="issue" size={13} />} label="GitHub link…" detail="An issue, PR or comment URL" onClick={() => setMode("github")} />
            <MenuItem icon={<Icon name="agents" size={13} />} label="Agent trace…" detail="The stored trace of another agent" onClick={() => setMode("trace")} />
            {!!recent.length && <div className="px-[9px] pt-2 pb-1 seclabel">Recent contexts</div>}
            {recent.slice(0, 6).map((c) => <MenuItem key={c.key} icon={<Icon name="plus" size={13} />} label={<span className="block truncate" title={c.label}>{c.label}</span>} onClick={() => (c.add(), close())} />)}
          </>
        );
      }}
    </Popover>
  );
}
