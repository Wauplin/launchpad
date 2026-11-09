// Shared context picker for new tasks and follow-up messages.
import { useState } from "react";
import type { AgentTraceRef } from "@/gen/AgentTraceRef";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Project } from "@/gen/Project";
import type { Preset } from "@/gen/Preset";
import type { Session } from "@/gen/Session";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { ProjectPicker } from "./ProjectPicker";
import { AgentTracePicker } from "./AgentTracePicker";
import { PresetChecklist } from "@/features/agents/MessagePresets";

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
        className="field-input"
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

/** The presets offered by the menu: the available ones, the chosen IDs and how to change them. */
export type PresetPick = { presets: Preset[]; selected: string[]; onChange: (ids: string[]) => void };

/** The "+ Add context" popover: one menu for projects, GitHub links, agent traces, presets and recent contexts. */
export function AddContextMenu({ projects, recent, sessions, selectedTraces, onProject, onGitHub, onToggleTrace, presets, disabled, compact = false, icon = false }: {
  disabled?: boolean;
  compact?: boolean;
  /** Only a "+" icon button, for the icon row of a message box. */
  icon?: boolean;
  projects: Project[];
  recent: { key: string; label: string; add: () => void }[];
  sessions: Session[];
  selectedTraces: AgentTraceRef[];
  onProject: (p: Project) => void;
  onGitHub: (r: GitHubRef) => void;
  onToggleTrace: (t: AgentTraceRef) => void;
  /** Omitted where presets do not apply (answers to a question). */
  presets?: PresetPick;
}) {
  const [mode, setMode] = useState<"menu" | "project" | "github" | "trace" | "preset">("menu");
  const presetCount = presets ? presets.presets.filter((p) => presets.selected.includes(p.id)).length : 0;
  return (
    <Popover
      width={340}
      viewport
      label="Add context"
      className={icon ? "flex shrink-0" : undefined}
      trigger={({ open, toggle }) => icon ? (
        <button disabled={disabled} aria-haspopup="dialog" aria-expanded={open} className="iconbtn" title={presets ? "Add context: a folder, a GitHub link, an agent trace or presets" : "Add context: a folder, a GitHub link or an agent trace"} aria-label="Add context" onClick={() => { setMode("menu"); toggle(); }}>
          <Icon name="plus" size={14} />
        </button>
      ) : (
        <button disabled={disabled} aria-haspopup="dialog" aria-expanded={open} className={`btn btn-sm ${compact ? "btn-ghost" : ""}`} onClick={() => { setMode("menu"); toggle(); }}>
          <Icon name="plus" size={12} />
          Add context
        </button>
      )}
    >
      {(close) => {
        const back = <MenuItem icon={<Icon name="chev" size={12} className="rotate-90" />} label="Back" onClick={() => setMode("menu")} />;
        if (mode === "project") return <>{back}<ProjectPicker projects={projects} temporary={false} emptyMessage="No other folder." onPick={(p) => { if (p) { onProject(p); close(); } }} /></>;
        if (mode === "github") return <>{back}<GitHubLinkForm onAdd={onGitHub} onDone={close} /></>;
        if (mode === "preset" && presets) return <>{back}<PresetChecklist presets={presets.presets} selected={presets.selected} onChange={presets.onChange} close={close} /></>;
        if (mode === "trace") return <>{back}<AgentTracePicker sessions={sessions} selected={selectedTraces} onToggle={onToggleTrace} /><div className="flex items-center justify-between px-2 pt-1 text-[12px] text-ink-3"><span>{selectedTraces.length} selected</span><button className="btn btn-sm btn-primary" onClick={close}>Done</button></div></>;
        return (
          <>
            <MenuItem icon={<Icon name="folder" size={12} />} label="Folder…" detail="Another folder as read-only context" onClick={() => setMode("project")} />
            <MenuItem icon={<Icon name="issue" size={12} />} label="GitHub link…" detail="An issue, PR or comment URL" onClick={() => setMode("github")} />
            <MenuItem icon={<Icon name="agents" size={12} />} label="Agent trace…" detail="The stored trace of another agent" onClick={() => setMode("trace")} />
            {presets && <MenuItem icon={<Icon name="preset" size={12} />} label={presetCount ? `Presets… · ${presetCount}` : "Presets…"} detail="Your usual instructions, added after the text" onClick={() => setMode("preset")} />}
            {!!recent.length && <div className="px-[9px] pt-2 pb-1 seclabel">Recent contexts</div>}
            {recent.slice(0, 6).map((c) => <MenuItem key={c.key} icon={<Icon name="plus" size={12} />} label={<span className="block truncate" title={c.label}>{c.label}</span>} onClick={() => (c.add(), close())} />)}
          </>
        );
      }}
    </Popover>
  );
}
