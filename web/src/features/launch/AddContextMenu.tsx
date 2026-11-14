// Shared context picker for new tasks and follow-up messages.
import { TopicChoices } from "@/features/topics/TopicPicker";
import type { TopicContext } from "@/gen/TopicContext";
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
import { usePhone } from "@/lib/phone";
import { Sheet } from "@/mobile/Sheet";
import { ATTACH_LIMITS } from "@/features/agents/MessageAttachments";

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

/** Files and images in the menu: opens the box's file picker (pasting and dropping still work in the text). */
export type AttachPick = { onPick: () => void; images: boolean; disabled?: boolean };

/** The "+ Add context" popover: one menu for attachments, projects, GitHub links, agent traces, presets and recent contexts. */
export function AddContextMenu({ projects, recent, sessions, selectedTraces, onProject, onGitHub, onToggleTrace, presets, attach, topics = [], onTopic, disabled, compact = false, icon = false }: {
  topics?: TopicContext[];
  onTopic?: (topic: TopicContext) => void;
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
  /** Omitted where the box takes no attachments. */
  attach?: AttachPick;
}) {
  const [mode, setMode] = useState<"menu" | "project" | "github" | "trace" | "preset" | "topic">("menu");
  const [sheet, setSheet] = useState(false);
  const phone = usePhone();
  const presetCount = presets ? presets.presets.filter((p) => presets.selected.includes(p.id)).length : 0;
  const title = `Add context: ${attach ? (attach.images ? "files or images, " : "files, ") : ""}a folder, a GitHub link, an agent trace${presets ? " or presets" : ""}${attach ? `. Files: ${ATTACH_LIMITS}` : ""}`;
  const body = (close: () => void) => {
    const back = <MenuItem icon={<Icon name="chev" size={12} className="rotate-90" />} label="Back" onClick={() => setMode("menu")} />;
    if (mode === "topic") return <>{back}<TopicChoices exclude={topics.map(t => t.id)} close={close} onPick={t => onTopic?.({id:t.id,name:t.name})} /></>;
    if (mode === "project") return <>{back}<ProjectPicker projects={projects} temporary={false} emptyMessage="No other folder." onPick={(p) => { if (p) { onProject(p); close(); } }} /></>;
    if (mode === "github") return <>{back}<GitHubLinkForm onAdd={onGitHub} onDone={close} /></>;
    if (mode === "preset" && presets) return <>{back}<PresetChecklist presets={presets.presets} selected={presets.selected} onChange={presets.onChange} close={close} /></>;
    if (mode === "trace") return <>{back}<AgentTracePicker sessions={sessions} selected={selectedTraces} onToggle={onToggleTrace} /><div className="flex items-center justify-between px-2 pt-1 text-[12px] text-ink-3"><span>{selectedTraces.length} selected</span><button className="btn btn-sm btn-primary" onClick={close}>Done</button></div></>;
    return (
      <>
        {attach && <MenuItem icon={<Icon name="clip" size={12} />} label={attach.images ? "Files or images…" : "Files…"} detail={attach.images ? "Or paste or drop them in the text" : "Or drop them in the text"} disabled={attach.disabled} onClick={() => { attach.onPick(); close(); }} />}
        <MenuItem icon={<Icon name="folder" size={12} />} label="Folder…" detail="Another folder as read-only context" onClick={() => setMode("project")} />
        <MenuItem icon={<Icon name="issue" size={12} />} label="GitHub link…" detail="An issue, PR or comment URL" onClick={() => setMode("github")} />
        {onTopic && <MenuItem icon={<Icon name="tag" size={12} />} label="Topic…" detail="The traces of every session in a topic" onClick={() => setMode("topic")} />}
        <MenuItem icon={<Icon name="agents" size={12} />} label="Agent trace…" detail="The stored trace of another agent" onClick={() => setMode("trace")} />
        {presets && <MenuItem icon={<Icon name="preset" size={12} />} label={presetCount ? `Presets… · ${presetCount}` : "Presets…"} detail="Your usual instructions, added after the text" onClick={() => setMode("preset")} />}
        {!!recent.length && <div className="px-[9px] pt-2 pb-1 seclabel">Recent contexts</div>}
        {recent.slice(0, 6).map((c) => <MenuItem key={c.key} icon={<Icon name="plus" size={12} />} label={<span className="block truncate" title={c.label}>{c.label}</span>} onClick={() => (c.add(), close())} />)}
      </>
    );
  };
  // On the phone, the "+" of a message box opens a bottom sheet: a popover over the keyboard has no room.
  if (phone) return (
    <>
      <button disabled={disabled} aria-haspopup="dialog" aria-expanded={sheet} className={icon ? "iconbtn shrink-0" : "btn btn-sm btn-ghost"} title={title} aria-label="Add context" onClick={() => { setMode("menu"); setSheet(true); }}>
        <Icon name="plus" size={14} />{!icon && "Add context"}
      </button>
      {sheet && <Sheet title="Add context" onClose={() => setSheet(false)}>
        <div className="phone-action-popover grid gap-px p-1.5">{body(() => setSheet(false))}</div>
      </Sheet>}
    </>
  );
  return (
    <Popover
      width={340}
      viewport
      label="Add context"
      className={icon ? "flex shrink-0" : undefined}
      trigger={({ open, toggle }) => icon ? (
        <button disabled={disabled} aria-haspopup="dialog" aria-expanded={open} className="iconbtn" title={title} aria-label="Add context" onClick={() => { setMode("menu"); toggle(); }}>
          <Icon name="plus" size={14} />
        </button>
      ) : (
        <button disabled={disabled} aria-haspopup="dialog" aria-expanded={open} className={`btn btn-sm ${compact ? "btn-ghost" : ""}`} onClick={() => { setMode("menu"); toggle(); }}>
          <Icon name="plus" size={12} />
          Add context
        </button>
      )}
    >
      {body}
    </Popover>
  );
}
