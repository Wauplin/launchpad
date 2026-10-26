// The message box: tab chips, text, pasted images and the Tabs · Agent · Mode · Project pills.
// Used to start a task and to reply in one.

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useHarnesses } from "@/components/HarnessMark";
import { toastError } from "@/components/Toast";
import type { FileAttachment } from "@/gen/FileAttachment";
import type { Profile } from "@/gen/Profile";
import type { Project } from "@/gen/Project";
import { attachmentSrc, pastedImages, readAttachments } from "@/lib/attachments";
import { quotaFor, supportsModelCost, useProfiles } from "@/lib/profiles";
import { useProjects, useStatus } from "@/lib/queries";
import { rememberedRun } from "@/stores/composer";
import { Favicon, Icon } from "./icons";
import { hostOf, isWebPage, openInLaunchpad, prefs, siteOf, type TabChip } from "./lp";

export type Quote = { text: string; tab: number };
type MenuKind = "tabs" | "agent" | "mode" | "project" | null;

/** The open tabs of the current window, refreshed while the panel is open. */
export function useWindowTabs() {
  const [tabs, setTabs] = useState<chrome.tabs.Tab[]>([]);
  useEffect(() => {
    const refresh = () => void chrome.tabs.query({ currentWindow: true }).then(setTabs).catch(() => {});
    refresh();
    chrome.tabs.onActivated.addListener(refresh);
    chrome.tabs.onUpdated.addListener(refresh);
    chrome.tabs.onRemoved.addListener(refresh);
    return () => { chrome.tabs.onActivated.removeListener(refresh); chrome.tabs.onUpdated.removeListener(refresh); chrome.tabs.onRemoved.removeListener(refresh); };
  }, []);
  return tabs;
}
export const chipOf = (t: chrome.tabs.Tab): TabChip => ({ tab: t.id!, title: t.title || hostOf(t.url ?? ""), url: t.url ?? "", favIconUrl: t.favIconUrl });

/** Profiles with the one used last from the extension on top. */
export function useAgents(): Profile[] {
  const profiles = useProfiles();
  return useMemo(() => {
    const last = prefs().profile;
    return [...profiles].sort((a, b) => Number(b.id === last) - Number(a.id === last));
  }, [profiles]);
}

function useAgentDetail() {
  const status = useStatus().data;
  const harnesses = useHarnesses();
  return (p: Profile) => {
    if (p.billing === "pay_as_you_go") {
      const model = rememberedRun(p.harness, p.id).model ?? p.model;
      const descriptor = harnesses.find((h) => h.id === p.harness);
      return { text: "Pay per use", untracked: !!descriptor?.capabilities.cost && !supportsModelCost(descriptor, model) };
    }
    const q = quotaFor(status, p.harness, p.id);
    const week = q?.weekly?.used_percent, five = q?.five_hour?.used_percent;
    const used = Math.max(week ?? 0, five ?? 0);
    return { text: week == null && five == null ? "Subscription" : used >= 100 ? "Subscription · usage limit reached" : `Subscription · ${Math.round(used)}% of the limit used`, untracked: false };
  };
}

/** A project whose GitHub repository matches one of the chips. */
export function suggestProject(projects: Project[], chips: TabChip[]): Project | null {
  for (const chip of chips) {
    const m = /^https:\/\/github\.com\/([^/]+\/[^/#?]+)/.exec(chip.url);
    const repo = m?.[1].toLowerCase();
    const p = repo && projects.find((x) => !x.is_temp && (x.gh_repo?.toLowerCase() === repo || x.gh_upstream?.toLowerCase() === repo));
    if (p) return p;
  }
  return null;
}

type BoxProps = {
  reply?: boolean;
  chips: TabChip[];
  setChips: (chips: TabChip[]) => void;
  /** Already shared with the task (reply only): shown checked and not removable here. */
  shared?: number[];
  text: string;
  setText: (text: string) => void;
  images: FileAttachment[];
  setImages: (images: FileAttachment[]) => void;
  quote?: Quote | null;
  clearQuote?: () => void;
  profile?: Profile | null;
  setProfile?: (id: string) => void;
  act: boolean;
  /** `always`: the sites of the chips, to remember (on) or forget (off) as always allowed for Act. */
  setAct: (act: boolean, always?: { sites: string[]; on: boolean }) => void;
  /** Sites (origins) where Act is always allowed. */
  sites: string[];
  project?: string | null;
  setProject?: (id: string | null) => void;
  running?: boolean;
  busy?: boolean;
  canSend: boolean;
  placeholder: string;
  onSend: () => void;
  onStop?: () => void;
  hint?: ReactNode;
  /** Shown as a chip when no tab is chosen: what happens without one. */
  noTabs?: string;
};

export function Box(p: BoxProps) {
  const [menu, setMenu] = useState<MenuKind>(null);
  const [reading, setReading] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);
  const projects = (useProjects().data ?? []).filter((x) => !x.is_temp);
  const project = projects.find((x) => x.id === p.project) ?? null;
  useEffect(() => {
    const el = area.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [p.text]);
  const addImages = async (files: File[] | Promise<File[]>) => {
    setReading(true);
    try { p.setImages([...p.images, ...await readAttachments(await files, p.images, true)]); }
    catch (e) { toastError(e); }
    finally { setReading(false); }
  };
  const sendable = p.canSend && !p.busy && !reading;
  const showStop = p.running && !p.text.trim() && !p.images.length && !!p.onStop;
  return <>
    {menu && <div className="x-scrim" onClick={() => setMenu(null)} />}
    {menu === "tabs" && <TabsMenu {...p} close={() => setMenu(null)} />}
    {menu === "agent" && <AgentMenu {...p} close={() => setMenu(null)} />}
    {menu === "mode" && <ModeMenu {...p} close={() => setMenu(null)} />}
    {menu === "project" && <ProjectMenu {...p} projects={projects} close={() => setMenu(null)} />}
    <div className="x-box">
      {(p.chips.length > 0 || p.quote || p.images.length > 0 || p.noTabs) && <div className="x-chips">
        {!p.chips.length && p.noTabs && <span className="x-chip x-ghost" title="Without a shared tab, the agent opens the sites it needs in new tabs. Each new site asks you first unless you always allow it."><Icon name="globe" size={14} /><span>{p.noTabs}</span></span>}
        {p.quote && <span className="x-chip x-quote" title={p.quote.text}><Icon name="quote" size={14} /><span>{p.quote.text}</span><button aria-label="Remove the quote" onClick={p.clearQuote}><Icon name="close" size={12} /></button></span>}
        {p.chips.map((c) => <span key={c.tab} className="x-chip" title={c.url}><Favicon url={c.url} title={c.title} src={c.favIconUrl} /><span>{c.title}</span><button aria-label={`Do not share ${c.title}`} onClick={() => p.setChips(p.chips.filter((x) => x.tab !== c.tab))}><Icon name="close" size={12} /></button></span>)}
        {p.images.map((f, i) => <span key={f.data_url} className="x-chip"><img className="x-thumb" src={attachmentSrc(f.data_url)} alt="" /><span>{f.name}</span><button aria-label={`Remove ${f.name}`} onClick={() => p.setImages(p.images.filter((_, j) => j !== i))}><Icon name="close" size={12} /></button></span>)}
      </div>}
      <textarea ref={area} rows={1} value={p.text} placeholder={p.placeholder} aria-label="Message"
        onChange={(e) => p.setText(e.target.value)}
        onPaste={(e) => { const files = pastedImages(e); if (files) void addImages(files); }}
        onDrop={(e) => { const files = Array.from(e.dataTransfer.files).filter((f) => f.type.startsWith("image/")); if (files.length) { e.preventDefault(); void addImages(files); } }}
        onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); if (sendable) p.onSend(); } }} />
      {reading && <div className="x-small x-faint" style={{ padding: "0 6px 4px" }}>Reading images…</div>}
      <div className="x-row">
        <div className="x-pills">
        <button className={`x-pill ${menu === "tabs" ? "x-on" : ""}`} title="Share tabs" aria-label="Share tabs" onClick={() => setMenu("tabs")}><Icon name="plus" size={16} /></button>
        {!p.reply && p.profile !== undefined && <button className={`x-pill x-shrink ${menu === "agent" ? "x-on" : ""}`} title="Agent" onClick={() => setMenu("agent")}><Icon name="agent" size={14} /><span>{p.profile?.name ?? "Agent"}</span></button>}
        <button className={`x-pill ${menu === "mode" ? "x-on" : ""} ${p.act ? "x-acting" : ""}`} title="What the agent can do" onClick={() => setMenu("mode")}><Icon name={p.act ? "hand" : "eye"} size={14} /><span>{p.act ? "Act" : "Read"}</span></button>
        {!p.reply && p.setProject && projects.length > 0 && <button className={`x-pill x-shrink ${menu === "project" ? "x-on" : ""}`} title="Project" onClick={() => setMenu("project")}><Icon name="folder" size={14} /><span>{project?.name ?? "No project"}</span></button>}
        </div>
        {showStop
          ? <button className="x-send x-stop" aria-label="Stop" title="Stop" onClick={p.onStop}><Icon name="stop" size={16} /></button>
          : <button className="x-send" aria-label="Send" title="Send" disabled={!sendable} onClick={p.onSend}>{p.busy ? <span className="x-spin" /> : <Icon name="send" size={16} />}</button>}
      </div>
    </div>
    {p.hint && <div className="x-hint x-small x-faint">{p.hint}</div>}
  </>;
}

function TabsMenu(p: BoxProps & { close: () => void }) {
  const tabs = useWindowTabs().filter((t) => t.id != null);
  const shared = new Set(p.shared ?? []);
  const toggle = (t: chrome.tabs.Tab) => {
    if (shared.has(t.id!)) return;
    p.setChips(p.chips.some((c) => c.tab === t.id) ? p.chips.filter((c) => c.tab !== t.id) : [...p.chips, chipOf(t)]);
  };
  return <div className="x-menu" role="dialog" aria-label="Tabs to share">
    <h2>Tabs in this window</h2>
    {tabs.map((t) => {
      const web = isWebPage(t.url);
      const on = shared.has(t.id!) || p.chips.some((c) => c.tab === t.id);
      return <button key={t.id} className={`x-opt ${on ? "x-sel" : ""}`} disabled={!web} style={web ? undefined : { opacity: 0.5, cursor: "default" }} onClick={() => web && toggle(t)}>
        <span className={`x-check ${on ? "x-on" : ""}`}>{on && <Icon name="check" size={12} />}</span>
        <Favicon url={t.url ?? ""} title={t.title ?? ""} src={t.favIconUrl} />
        <span className="x-d"><b>{t.title || hostOf(t.url ?? "")}</b><small className="x-one">{shared.has(t.id!) ? "Already shared with this task" : web ? `${hostOf(t.url!)}${t.active ? " · current tab" : ""}` : "Chrome pages can't be shared"}</small></span>
      </button>;
    })}
    <div className="x-sep" />
    <div className="x-note">Chrome may ask to allow access to new sites when you send. Share no tab to let the agent open its own tabs.</div>
    <div className="x-btns" style={{ padding: "0 4px 4px" }}><button className="x-btn x-pri x-wide" onClick={p.close}>Done</button></div>
  </div>;
}

function AgentMenu(p: BoxProps & { close: () => void }) {
  const agents = useAgents();
  const detail = useAgentDetail();
  const harnesses = useHarnesses();
  const chosen = p.profile ? detail(p.profile) : null;
  return <div className="x-menu" role="dialog" aria-label="Agent">
    <h2>Agent</h2>
    {agents.map((a) => {
      const d = detail(a);
      return <button key={a.id} className={`x-opt ${a.id === p.profile?.id ? "x-sel" : ""}`} onClick={() => { p.setProfile?.(a.id); p.close(); }}>
        <span className={`x-radio ${a.id === p.profile?.id ? "x-on" : ""}`} />
        <span className="x-d"><b>{a.name}</b><small className="x-one">{harnesses.find((h) => h.id === a.harness)?.name ?? a.harness} · {d.text}</small></span>
      </button>;
    })}
    {!agents.length && <div className="x-note">No agents yet. Add a profile in Launchpad → Settings → Profiles.</div>}
    {chosen?.untracked && <div className="x-note x-warn"><Icon name="warn" size={14} /><span>{p.profile!.name}: costs for this model can't be tracked. Check your provider's dashboard.</span></div>}
    <div className="x-sep" />
    <button className="x-opt x-muted" onClick={() => openInLaunchpad("/settings/profiles")}><Icon name="ext" size={14} />Manage agents in Launchpad</button>
  </div>;
}

function ModeMenu(p: BoxProps & { close: () => void }) {
  const sites = [...new Set(p.chips.map((c) => siteOf(c.url)).filter(Boolean))];
  const [always, setAlways] = useState(sites.length > 0 && sites.every((s) => p.sites.includes(s)));
  const pick = (act: boolean) => { p.setAct(act, sites.length ? { sites, on: act && always } : undefined); p.close(); };
  return <div className="x-menu" role="dialog" aria-label="What the agent can do">
    <h2>What can the agent do?</h2>
    <button className={`x-opt ${!p.act ? "x-sel" : ""}`} onClick={() => pick(false)}><span className={`x-radio ${!p.act ? "x-on" : ""}`} /><Icon name="eye" /><span className="x-d"><b>Read</b><small>Reads the tabs you share. Never clicks or types.</small></span></button>
    <button className={`x-opt ${p.act ? "x-sel" : ""}`} onClick={() => pick(true)}><span className={`x-radio ${p.act ? "x-on" : ""}`} /><Icon name="hand" /><span className="x-d"><b>Act</b><small>Can click, type and navigate in the shared tabs. Asks you before submitting, paying or going to a new site.</small></span></button>
    {sites.length > 0 && <button className="x-opt" onClick={() => setAlways(!always)}>
      <span className={`x-check ${always ? "x-on" : ""}`}>{always && <Icon name="check" size={12} />}</span>
      <span className="x-d"><b style={{ fontWeight: 500 }}>Always allow Act on {sites.map(hostOf).join(", ")}</b><small>New tasks on these sites start in Act, and the agent can go to them without asking.</small></span>
    </button>}
    <div className="x-sep" />
    <div className="x-note">In both modes the agent only sees the tabs you share{p.project ? " (and the project's files)" : ""}. It never types passwords or one-time codes.</div>
  </div>;
}

function ProjectMenu(p: BoxProps & { projects: Project[]; close: () => void }) {
  const [q, setQ] = useState("");
  const suggested = suggestProject(p.projects, p.chips);
  const shown = p.projects.filter((x) => `${x.name} ${x.gh_repo ?? ""} ${x.description}`.toLowerCase().includes(q.toLowerCase()));
  const pick = (id: string | null) => { p.setProject?.(id); p.close(); };
  return <div className="x-menu" role="dialog" aria-label="Project">
    <h2>Project</h2>
    {p.projects.length > 6 && <input type="search" autoFocus placeholder="Search projects" value={q} onChange={(e) => setQ(e.target.value)} />}
    {!q && <button className={`x-opt ${!p.project ? "x-sel" : ""}`} onClick={() => pick(null)}><span className={`x-radio ${!p.project ? "x-on" : ""}`} /><span className="x-d"><b>No project</b><small>A browsing task, in its own empty folder</small></span></button>}
    {shown.map((x) => <button key={x.id} className={`x-opt ${p.project === x.id ? "x-sel" : ""}`} onClick={() => pick(x.id)}>
      <span className={`x-radio ${p.project === x.id ? "x-on" : ""}`} /><Icon name="folder" size={16} />
      <span className="x-d"><b>{x.name}</b><small className="x-one">{x.id === suggested?.id ? `Suggested: matches ${x.gh_repo ?? x.gh_upstream}` : x.gh_repo ?? x.path}</small></span>
    </button>)}
    <div className="x-sep" />
    <div className="x-note">With a project, the agent works in a new task worktree of it with its normal tools, and the task appears in that project in Launchpad. Page text can try to mislead an agent: share only pages you trust with a project task.</div>
  </div>;
}
