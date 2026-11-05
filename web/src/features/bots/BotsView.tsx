// Bots: persistent agents with one thread each. The list (with Ask Launchpad as the built-in bot), the form
// that creates one (name, job, profile and folder only), and a bot's page: its thread on the left, and on the
// right its memory (notes in the knowledge base), its files (BOT.md, bot.toml, with versions), its sessions and
// its settings. Creating saves; the first task is a separate Send.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { memo, useEffect, useMemo, useState, type ReactNode } from "react";
import { HarnessMark } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { Markdown } from "@/components/Markdown";
import { toast, toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, Loading, PageHeader, Tabs } from "@/components/ui";
import { PatchView } from "@/features/github/DiffReview";
import { SessionPane } from "@/features/agents/SessionPane";
import type { Bot } from "@/gen/Bot";
import type { BotSettings } from "@/gen/BotSettings";
import type { Change } from "@/gen/Change";
import type { Notes } from "@/gen/Notes";
import type { PermissionMode } from "@/gen/PermissionMode";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { VaultOverview } from "@/gen/VaultOverview";
import { api } from "@/lib/api";
import { botInitial, botJob, botState, botThread, currentSession, rotationReason, useBots, type BotState } from "@/lib/bots";
import { daemonUrl } from "@/lib/connection";
import { formatAge } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useProfiles } from "@/lib/profiles";
import { useRunnerTag } from "@/lib/burst";
import { useCrons, useHarnessDefaults, useProjects, useSessions } from "@/lib/queries";
import { routineState, triggerSummary } from "@/lib/routines";
import { NewRoutine, RoutineDetails, RunStrip } from "@/features/schedules/Routines";
import type { CronAgent } from "@/gen/CronAgent";
import { useScope, useWorkspaces } from "@/lib/workspace";
import { PhoneHeader } from "@/mobile/PhoneHeader";

const PERMISSIONS: { id: PermissionMode; label: string; detail: string }[] = [
  { id: "yolo", label: "YOLO", detail: "Runs commands without asking, inside its jail" },
  { id: "approvals", label: "Ask for approval", detail: "Asks you before commands and edits" },
  { id: "auto", label: "Auto", detail: "The harness's own reviewer decides each request" },
];

const STATE: Record<BotState, { label: string; tone: "attn" | "work" | "neutral" | "teal" }> = {
  needs: { label: "Needs you", tone: "attn" },
  running: { label: "Working", tone: "work" },
  paused: { label: "Quota paused", tone: "attn" },
  idle: { label: "Idle", tone: "neutral" },
  new: { label: "No task yet", tone: "teal" },
};

export function BotAvatar({ name, size = 28 }: { name: string; size?: number }) {
  return <span className="grid flex-none place-items-center rounded-[8px] bg-teal-soft font-semibold text-teal-ink" style={{ width: size, height: size, fontSize: Math.round(size * 0.46) }} aria-hidden>{botInitial(name)}</span>;
}

function StateBadge({ state }: { state: BotState }) {
  const s = STATE[state];
  return <Badge tone={s.tone} dot={state === "needs"}>{state === "running" && <span className="lp-spin mr-1" />}{s.label}</Badge>;
}

const useBotSessions = () => useSessions().data ?? [];
const folderLabel = (projects: Project[], id: string) => projects.find((p) => p.id === id)?.name ?? "a removed folder";

// ---------------------------------------------------------------- the list

export function BotsView() {
  const phone = usePhone();
  const bots = useBots();
  const scope = useScope();
  const sessions = useBotSessions();
  const projects = useProjects().data ?? [];
  const search = useSearch({ strict: false }) as { new?: boolean };
  const navigate = useNavigate();
  const mine = (bots.data ?? []).filter((b) => !scope.workspace || b.workspace_id === scope.workspace.id);
  const creating = !!search.new || (bots.isSuccess && mine.length === 0);
  const close = () => void navigate({ to: "/bots", search: {} as never });
  const header = phone
    ? <PhoneHeader title="Bots" subtitle={scope.workspace?.name ?? "Everything"} primary={!creating ? <Link to="/bots" search={{ new: true } as never} className="btn btn-sm btn-primary"><Icon name="plus" size={12} />New bot</Link> : undefined} />
    : <PageHeader icon={<Icon name="spark" size={18} className="text-teal-ink" />} title="Bots" meta={<>{scope.workspace?.name ?? "Everything"} · persistent agents with one thread, memory in the knowledge base and files on Home</>} actions={!creating && <Link to="/bots" search={{ new: true } as never} className="btn btn-sm btn-primary" data-tour="bots.new"><Icon name="plus" size={12} />New bot</Link>} />;
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="bots.page">
      {header}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto grid w-full max-w-[880px] grid-cols-[minmax(0,1fr)] gap-4 p-4">
          {bots.isLoading ? <Loading what="bots" /> : bots.error ? <ErrorState error={bots.error} onRetry={() => void bots.refetch()} /> : <>
            {creating && <NewBot first={mine.length === 0} onCancel={mine.length ? close : undefined} />}
            <div className="grid grid-cols-[minmax(0,1fr)] gap-2">
              <AskRow />
              {mine.map((b) => <BotRow key={b.id} bot={b} thread={botThread(sessions, b.id)} folder={folderLabel(projects, b.settings.folder)} />)}
            </div>
          </>}
        </div>
      </div>
    </div>
  );
}

function AskRow() {
  return <Link to="/ask" className="flex min-h-[56px] items-center gap-3 rounded-lg border border-line bg-paper px-3 py-2 no-underline hover:bg-sunk">
    <span className="grid size-[32px] flex-none place-items-center rounded-[8px] bg-teal-soft text-teal-ink"><Icon name="spark" size={16} /></span>
    <span className="min-w-0 flex-1">
      <span className="flex items-center gap-2 text-[13px] font-semibold text-ink">Ask Launchpad <Badge>Built in</Badge></span>
      <span className="block truncate text-[12px] text-ink-3">Reads your workspace and proposes launches, messages and schedules; nothing starts without your click.</span>
    </span>
  </Link>;
}

const BotRow = memo(function BotRow({ bot, thread, folder }: { bot: Bot; thread: Session[]; folder: string }) {
  const last = thread.at(-1);
  return <Link to="/bots/$id" params={{ id: bot.id }} className="flex min-h-[56px] items-center gap-3 rounded-lg border border-line bg-paper px-3 py-2 no-underline hover:bg-sunk">
    <BotAvatar name={bot.settings.name} size={32} />
    <span className="min-w-0 flex-1">
      <span className="flex items-center gap-2 text-[13px] font-semibold text-ink"><span className="truncate">{bot.settings.name}</span><StateBadge state={botState(thread)} /></span>
      <span className="block truncate text-[12px] text-ink-3">{botJob(bot) || "No instructions yet"}</span>
    </span>
    <span className="hidden flex-none text-right text-[12px] text-ink-3 sm:block">
      <span className="block">{folder}</span>
      <span className="block">{last ? `${formatAge(last.status_since)} ago` : "never ran"}</span>
    </span>
  </Link>;
});

// ---------------------------------------------------------------- creating a bot

/** Tools granted to a workspace (Home's registry): what a bot's sessions reach with Home's tokens. */
function useGrantedTools(workspaceId: string | null) {
  const vault = useQuery({ queryKey: ["vault"], queryFn: () => api.get<VaultOverview>("/api/vault"), staleTime: 30_000, retry: false });
  return (vault.data?.tools ?? []).filter((t) => !!workspaceId && (t.workspaces.includes("*") || t.workspaces.includes(workspaceId)));
}

/** What the bot can reach, in plain words: its folder, the knowledge base, the granted tools and the mode. */
function Access({ folder, workspace, workspaceId, mode }: { folder: Project | null; workspace: string; workspaceId: string | null; mode: PermissionMode }) {
  const tools = useGrantedTools(workspaceId);
  const permission = PERMISSIONS.find((p) => p.id === mode)!;
  return <ul className="m-0 grid gap-1 pl-4 text-[12px] text-ink-2">
    <li>Runs contained on a runner, like every agent. <b>{permission.label}</b>: {permission.detail.toLowerCase()}.</li>
    <li>{folder ? (folder.plain ? <>Works in the plain folder <b>{folder.name}</b>, synced by Home.</> : <>Works in <b>{folder.name}</b>, in a fresh worktree per session; it prepares branches and Local PRs for your review.</>) : <>Works in its own new plain folder, synced by Home.</>}</li>
    <li>Remembers in the knowledge base of <b>{workspace}</b>: every change records it, and you can restore it.</li>
    <li>{tools.length ? <>Uses the tools granted to {workspace} with Home's tokens: {tools.map((t) => t.name).join(", ")}.</> : <>No tool is granted to {workspace} (Settings → Credentials and tools).</>}</li>
  </ul>;
}

function NewBot({ first, onCancel }: { first: boolean; onCancel?: () => void }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const scope = useScope();
  const workspaces = useWorkspaces().data ?? [];
  const projects = useProjects().data ?? [];
  const profiles = useProfiles();
  const [workspaceId, setWorkspaceId] = useState<string | null>(scope.workspace?.id ?? null);
  const ws = scope.workspace?.id ?? workspaceId ?? workspaces[0]?.id ?? null;
  const folders = projects.filter((p) => p.workspace_id === ws && !p.is_temp).sort((a, b) => a.name.localeCompare(b.name));
  const [name, setName] = useState("");
  const [job, setJob] = useState("");
  const [profile, setProfile] = useState<string>(profiles.find((p) => p.billing === "subscription")?.id ?? profiles[0]?.id ?? "");
  const [folder, setFolder] = useState<string>("");
  const [busy, setBusy] = useState(false);
  useEffect(() => { if (!profile && profiles.length) setProfile(profiles.find((p) => p.billing === "subscription")?.id ?? profiles[0].id); }, [profiles, profile]);
  const repo = folders.find((p) => !p.plain);
  const create = async () => {
    if (!ws || !name.trim()) return;
    setBusy(true);
    try {
      const bot = await api.post<Bot>("/api/bots", { workspace_id: ws, name: name.trim(), instructions: job.trim() ? `# ${name.trim()}\n\n${job.trim()}` : "", profile: profile || null, folder: folder || null });
      await Promise.all([qc.invalidateQueries({ queryKey: ["bots"] }), qc.invalidateQueries({ queryKey: ["projects"] })]);
      void navigate({ to: "/bots/$id", params: { id: bot.id } });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  if (!workspaces.length) return <Empty icon="folder">Create a workspace first: a bot belongs to a workspace and remembers in its knowledge base.</Empty>;
  return (
    <section className="grid gap-3 rounded-lg border border-line bg-paper p-4" data-tour="bots.form">
      <div>
        <h2 className="m-0 text-[15px] font-semibold">{first ? "Create your first bot" : "New bot"}</h2>
        <p className="m-0 mt-1 text-[13px] text-ink-2">A bot is an agent that stays: one thread you come back to, its own instructions, and notes it keeps in the knowledge base between sessions. Describe its job; you can change everything later.</p>
      </div>
      {repo && !name && !job && <div className="flex flex-wrap items-center gap-1.5 text-[12px] text-ink-3">Start from
        <button className="btn btn-sm" onClick={() => { setName("Maintainer"); setJob(`Maintain ${repo.name}: answer my questions about the code, investigate issues and failing checks, and prepare fixes as Local PRs for my review.`); setFolder(repo.id); }}><Icon name="branch" size={12} />Repo maintainer for {repo.name}</button>
      </div>}
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Name
          <input className="field-input h-[32px] font-normal" maxLength={40} value={name} placeholder="Scout" onChange={(e) => setName(e.target.value)} autoFocus />
        </label>
        {!scope.workspace && workspaces.length > 1 && <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Workspace
          <select className="field-input h-[32px] font-normal" value={ws ?? ""} onChange={(e) => { setWorkspaceId(e.target.value); setFolder(""); }}>{workspaces.map((w) => <option key={w.id} value={w.id}>{w.name}</option>)}</select>
        </label>}
      </div>
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Its job
        <textarea className="field-input min-h-[76px] py-1.5 font-normal leading-5" value={job} placeholder="What it does and how you like it done. For example: “Watch the Xet changelog and tell me only about breaking changes.”" onChange={(e) => setJob(e.target.value)} />
        <span className="font-normal text-ink-3">Saved as BOT.md, the bot's instructions.</span>
      </label>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Profile
          {profiles.length ? <select className="field-input h-[32px] font-normal" value={profile} onChange={(e) => setProfile(e.target.value)}>{profiles.map((p) => <option key={p.id} value={p.id}>{p.name} · {p.billing === "subscription" ? "subscription" : "pay as you go"}</option>)}</select>
            : <Link to="/settings/profiles" className="btn btn-sm w-fit">Add a profile first</Link>}
        </label>
        <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Folder
          <select className="field-input h-[32px] font-normal" value={folder} onChange={(e) => setFolder(e.target.value)}>
            <option value="">Its own folder (a new plain folder{name.trim() ? ` “${name.trim()}”` : ""})</option>
            {folders.map((p) => <option key={p.id} value={p.id}>{p.name}{p.plain ? " · plain folder" : ""}</option>)}
          </select>
        </label>
      </div>
      <div className="rounded-md bg-sunk px-3 py-2">
        <div className="seclabel pb-1">What it can reach</div>
        <Access folder={projects.find((p) => p.id === folder) ?? null} workspace={workspaces.find((w) => w.id === ws)?.name ?? ""} workspaceId={ws} mode="yolo" />
      </div>
      <div className="flex flex-wrap items-center justify-end gap-2">
        <span className="mr-auto text-[12px] text-ink-3">Creating saves the bot. Nothing runs until you send it a task.</span>
        {onCancel && <button className="btn btn-sm btn-ghost" onClick={onCancel}>Cancel</button>}
        <button className="btn btn-sm btn-primary" disabled={busy || !name.trim() || !ws || !profiles.length} onClick={() => void create()}><Icon name="plus" size={12} />Create bot</button>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- a bot's page

type Panel = "memory" | "routines" | "files" | "thread" | "settings";
const PANELS: Panel[] = ["memory", "routines", "files", "thread", "settings"];

export function BotPage() {
  const { id } = useParams({ strict: false }) as { id: string };
  const phone = usePhone();
  const bots = useBots();
  const bot = bots.data?.find((b) => b.id === id);
  const sessions = useBotSessions();
  const thread = useMemo(() => botThread(sessions, id), [sessions, id]);
  const current = currentSession(thread);
  const search = useSearch({ strict: false }) as { panel?: string };
  const [panel, setPanel] = useState<Panel | "chat">(PANELS.find((p) => p === search.panel) ?? (phone ? "chat" : "memory"));
  const routines = (useCrons().data ?? []).filter((c) => c.bot_id === id);
  useEffect(() => { const p = PANELS.find((x) => x === search.panel); if (p) setPanel(p); }, [search.panel]);
  const [fresh, setFresh] = useState(false);
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  const where = useRunnerTag(current?.runner_id);
  const navigate = useNavigate();
  const qc = useQueryClient();
  useEffect(() => setFresh(false), [current?.id]);
  if (bots.isLoading) return <Loading what="the bot" />;
  if (bots.error) return <ErrorState error={bots.error} onRetry={() => void bots.refetch()} />;
  if (!bot) return <div className="flex h-full flex-col"><PageHeader title="Bot" /><Empty fill icon="spark" action={<Link to="/bots" className="btn btn-sm">All bots</Link>}>This bot no longer exists.</Empty></div>;
  const profile = profiles.find((p) => p.id === bot.settings.profile) ?? null;
  const rotation = rotationReason(bot, current);
  const folder = folderLabel(projects, bot.settings.folder);
  const showSession = current && !rotation && !fresh;
  const tabs = <Tabs label="Bot panels" value={panel} onChange={setPanel} tour="bots.panels" items={[...(phone ? [{ id: "chat" as const, label: "Thread" }] : []), { id: "memory", label: "Memory" }, { id: "routines", label: "Routines", count: routines.length || undefined }, { id: "files", label: "Instructions" }, { id: "thread", label: "Sessions", count: thread.length || undefined }, { id: "settings", label: "Settings" }]} />;
  const meta = <>
    <StateBadge state={botState(thread)} />
    {bot.paused && <Badge tone="attn" title="Its routines and incoming hooks start nothing until you resume it">Paused</Badge>}
    <span>{profile ? profile.name : "no profile"}{bot.settings.model ? ` · ${bot.settings.model}` : ""}</span>
    <span>works in {folder}</span>
    <span>{PERMISSIONS.find((p) => p.id === bot.settings.permission_mode)?.label}</span>
    {where && <span className={where.tone === "quiet" && where.label.startsWith("Burst") ? "text-ink-3" : "text-attn"} title={where.title}>{where.label}</span>}
  </>;
  const pause = <button className="btn btn-sm" data-tour="bots.pause" title={bot.paused ? "Routines and incoming hooks start runs again" : "Routines and incoming hooks start nothing until you resume. Your own messages still go through; Stop ends a run that is working."} onClick={() => void api.post<Bot>(`/api/bots/${bot.id}/pause`, { paused: !bot.paused }).then(() => Promise.all([qc.invalidateQueries({ queryKey: ["bots"] }), qc.invalidateQueries({ queryKey: ["crons"] })])).then(() => toast(bot.paused ? `${bot.settings.name} resumed` : `${bot.settings.name} paused: no routine starts`), toastError)}><Icon name={bot.paused ? "play" : "stop"} size={12} />{bot.paused ? "Resume" : "Pause"}</button>;
  const newSession = thread.length > 0 && !fresh && <button className="btn btn-sm" title="Start a new session of this thread; it references the previous session's trace" disabled={!!current && current.status !== "waiting" && current.status !== "error" && current.status !== "interrupted" && current.status !== "stopped"} onClick={() => setFresh(true)}><Icon name="plus" size={12} />New session</button>;
  const conversation = showSession
    ? <div className="relative min-h-0 flex-1" data-tour="bots.thread"><SessionPane key={current.id} sessionId={current.id} initialView="trace" onClear={() => void navigate({ to: "/bots" })} /></div>
    : <FirstMessage bot={bot} reason={fresh ? "You asked for a new session" : rotation} previous={thread.at(-1) ?? null} needsProfile={!profile} openSettings={() => setPanel("settings")} onCancel={fresh ? () => setFresh(false) : undefined} />;
  const side = <>
    {panel === "memory" && <MemoryPanel bot={bot} thread={thread} />}
    {panel === "routines" && <RoutinesPanel bot={bot} routines={routines} />}
    {panel === "files" && <FilesPanel bot={bot} />}
    {panel === "thread" && <ThreadPanel bot={bot} thread={thread} current={current} />}
    {panel === "settings" && <SettingsPanel bot={bot} />}
  </>;
  if (phone)
    return <div className="flex h-full min-h-0 flex-col">
      <PhoneHeader title={bot.settings.name} onBack={() => void navigate({ to: "/bots" })} backLabel="All bots" subtitle={<span className="flex flex-wrap items-center gap-x-2">{meta}</span>} primary={panel === "chat" ? newSession : panel === "routines" ? pause : undefined} />
      <div className="border-b border-line px-3 py-1">{tabs}</div>
      {panel === "chat" ? conversation : <div className="min-h-0 flex-1 overflow-y-auto">{side}</div>}
    </div>;
  return (
    <div className="flex h-full min-h-0 flex-col" data-tour="bots.bot">
      <PageHeader icon={<BotAvatar name={bot.settings.name} size={24} />} title={bot.settings.name} back={<Link to="/bots" className="iconbtn" title="All bots" aria-label="All bots"><Icon name="chev" size={14} className="rotate-90" /></Link>} meta={meta} actions={<>{routines.length > 0 && pause}{newSession}</>} />
      <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_380px]">
        <div className="flex min-h-0 flex-col border-r border-line">{conversation}</div>
        <aside className="flex min-h-0 flex-col">
          <div className="border-b border-line px-3 pt-1">{tabs}</div>
          <div className="min-h-0 flex-1 overflow-y-auto">{side}</div>
        </aside>
      </div>
    </div>
  );
}

/** The message that starts a session: the first task, or the next one after a rotation. */
function FirstMessage({ bot, reason, previous, needsProfile, openSettings, onCancel }: { bot: Bot; reason: string | null; previous: Session | null; needsProfile: boolean; openSettings: () => void; onCancel?: () => void }) {
  const qc = useQueryClient();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const send = async () => {
    if (!text.trim() || busy) return;
    setBusy(true);
    try {
      await api.post<Session>(`/api/bots/${bot.id}/message`, { text: text.trim(), new_session: !!previous });
      setText("");
      await qc.invalidateQueries({ queryKey: ["sessions"] });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <div className="grid min-h-0 flex-1 place-items-center overflow-y-auto p-6" data-tour="bots.first">
      <div className="grid w-full max-w-[560px] gap-3">
        {previous
          ? <p className="m-0 text-[13px] text-ink-2">{reason ? `${reason}: ` : ""}your next message starts a new session of {bot.settings.name}'s thread. It gets a reference to the previous session's trace, and its notes in the knowledge base. <Link to="/agents" search={{ session: previous.id } as never}>Open the previous session</Link></p>
          : <p className="m-0 text-[13px] text-ink-2">Send {bot.settings.name} its first task. It reads its instructions (Instructions → BOT.md) and the workspace's knowledge base, works in its folder, and keeps notes for its next sessions. To have it work on its own, add a routine in the Routines tab.</p>}
        {needsProfile
          ? <div className="flex items-center gap-2 rounded-md bg-attn-soft px-3 py-2 text-[13px] text-attn">This bot has no profile.<button className="btn btn-sm ml-auto" onClick={openSettings}>Choose a profile</button></div>
          : <div className="session-message-input flex items-end gap-2 rounded-lg border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
            <textarea autoFocus rows={3} value={text} disabled={busy} aria-label={`Message ${bot.settings.name}`} placeholder={`Message ${bot.settings.name}…`} onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void send(); } }}
              className="max-h-40 min-h-14 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none" />
            <button className="iconbtn border-line! bg-paper" title="Send" aria-label="Send" disabled={busy || !text.trim()} onClick={() => void send()}><Icon name="send" size={14} /></button>
          </div>}
        {busy && <Loading what={`${bot.settings.name}'s session`} />}
        {onCancel && <button className="btn btn-sm btn-ghost w-fit" onClick={onCancel}>Back to the current session</button>}
      </div>
    </div>
  );
}

function Section({ title, children, action }: { title: ReactNode; children: ReactNode; action?: ReactNode }) {
  return <section className="grid gap-2 border-b border-line-2 p-3">
    <div className="flex items-center gap-2"><h3 className="seclabel m-0 min-w-0 flex-1">{title}</h3>{action}</div>
    {children}
  </section>;
}

// ---------------------------------------------------------------- routines

function RoutinesPanel({ bot, routines }: { bot: Bot; routines: CronAgent[] }) {
  const projects = useProjects().data ?? [];
  const profiles = useProfiles();
  const navigate = useNavigate();
  const [adding, setAdding] = useState(false);
  const [open, setOpen] = useState<string | null>(routines.length === 1 ? routines[0].id : null);
  const folder = projects.find((p) => p.id === bot.settings.folder);
  const repo = folder && !folder.plain ? folder.gh_repo ?? folder.gh_upstream ?? null : null;
  const harness = profiles.find((p) => p.id === bot.settings.profile)?.harness ?? null;
  return <div data-tour="bots.routines">
    <Section title="Routines" action={!adding && <button className="btn btn-sm" onClick={() => setAdding(true)}><Icon name="plus" size={12} />New routine</button>}>
      <p className="m-0 text-[12px] text-ink-2">A routine sends {bot.settings.name} a task by itself: on a schedule, on GitHub activity, or when another agent is done. Each run goes to its thread and ends with a short report, or with no update, which never notifies you.{bot.paused ? ` ${bot.settings.name} is paused: no routine starts until you resume it.` : ""}</p>
      {adding && <NewRoutine bot={bot} repo={repo} harness={harness} onDone={() => setAdding(false)} />}
      {!adding && routines.length === 0 && <button className="btn btn-sm btn-primary w-fit" data-tour="bots.routine-new" onClick={() => setAdding(true)}><Icon name="plus" size={12} />Add a routine</button>}
    </Section>
    {routines.map((r) => <Section key={r.id} title={<button className="flex w-full min-w-0 items-center gap-2 text-left normal-case" aria-expanded={open === r.id} onClick={() => setOpen(open === r.id ? null : r.id)}><span className="min-w-0 flex-1 truncate">{r.title}</span><RunStrip history={r.history} count={10} /></button>}>
      {open === r.id ? <RoutineDetails cron={r} bot={bot} onOpen={(sid) => void navigate({ to: "/agents", search: { session: sid } as never })} onDeleted={() => setOpen(null)} />
        : <p className="m-0 text-[12px] text-ink-3">{triggerSummary(r, (fid) => projects.find((p) => p.id === fid)?.name ?? "a removed folder")} · {routineState(r, bot.settings.name)}</p>}
    </Section>)}
  </div>;
}

// ---------------------------------------------------------------- memory

function MemoryPanel({ bot, thread }: { bot: Bot; thread: Session[] }) {
  const ws = bot.workspace_id;
  const notes = useQuery({ queryKey: ["knowledge", ws, "notes"], queryFn: () => api.get<Notes>(`/api/knowledge/${ws}`) });
  const changes = useQuery({ queryKey: ["knowledge", ws, "changes"], queryFn: () => api.get<{ changes: Change[] }>(`/api/knowledge/${ws}/changes`) });
  const prefix = `bots/${bot.slug}/`;
  const own = (notes.data?.files ?? []).filter((f) => f.path.startsWith(prefix));
  const ids = new Set(thread.map((s) => s.id));
  const recent = (changes.data?.changes ?? []).filter((c) => c.author?.session_id && ids.has(c.author.session_id)).slice(0, 12);
  const [open, setOpen] = useState<string | null>(null);
  return <div data-tour="bots.memory">
    <Section title={`Its notes · ${prefix}`} action={<Link to="/knowledge" className="text-[12px]">Open Knowledge</Link>}>
      {notes.isLoading ? <Loading what="notes" /> : own.length === 0
        ? <p className="m-0 text-[12px] text-ink-3">No note yet. {bot.settings.name} writes here what it needs to remember between sessions; it also reads and writes the rest of the knowledge base.</p>
        : own.map((f) => <div key={f.path}>
          <button className="flex min-h-[28px] w-full items-center gap-1.5 text-left text-[13px] hover:underline" aria-expanded={open === f.path} onClick={() => setOpen(open === f.path ? null : f.path)}><Icon name="file" size={12} className="text-ink-3" />{f.path.slice(prefix.length)}</button>
          {open === f.path && <NoteText ws={ws} path={f.path} sha={f.sha256} />}
        </div>)}
    </Section>
    <Section title="Recent changes by this bot">
      {recent.length === 0 ? <p className="m-0 text-[12px] text-ink-3">None in the last 30 days.</p>
        : <ul className="m-0 grid gap-1 p-0">{recent.map((c) => <li key={c.id} className="flex list-none items-center gap-2 text-[12px]"><span className="min-w-0 flex-1 truncate" title={c.path}>{c.new_sha256 ? (c.sha256 ? "Edited" : "Created") : "Removed"} {c.path}</span><span className="text-ink-3 tabular-nums">{c.added != null && `+${c.added} −${c.removed ?? 0}`}</span><span className="text-ink-3">{formatAge(c.at)}</span></li>)}</ul>}
      <p className="m-0 text-[12px] text-ink-3">Correct a note in Knowledge; every change can be restored there for 30 days.</p>
    </Section>
  </div>;
}

function NoteText({ ws, path, sha }: { ws: string; path: string; sha: string }) {
  const q = useQuery({ queryKey: ["knowledge", ws, "file", path, sha], queryFn: async () => { const r = await fetch(daemonUrl(`/api/knowledge/${ws}/file?path=${encodeURIComponent(path)}`), { credentials: "include" }); if (!r.ok) throw new Error(await r.text()); return r.text(); }, staleTime: Infinity });
  return q.isLoading ? <Loading what="the note" /> : q.error ? <ErrorState error={q.error} /> : <div className="mb-2 rounded-md bg-sunk px-3 py-2"><Markdown text={q.data ?? ""} className="text-[13px]" /></div>;
}

// ---------------------------------------------------------------- files

function FilesPanel({ bot }: { bot: Bot }) {
  const qc = useQueryClient();
  const history = useQuery({ queryKey: ["bot-history", bot.id, bot.version], queryFn: () => api.get<Change[]>(`/api/bots/${bot.id}/history`) });
  const save = async (patch: Record<string, string>) => {
    try { await api.patch<Bot>(`/api/bots/${bot.id}`, patch); await qc.invalidateQueries({ queryKey: ["bots"] }); toast("Saved: the next session reads it"); return true; } catch (e) { toastError(e); return false; }
  };
  return <div data-tour="bots.files">
    <Section title="BOT.md · what it does">
      <FileEditor key={`i-${bot.version}`} value={bot.instructions} onSave={(t) => save({ instructions: t })} rows={10} />
    </Section>
    <Section title="bot.toml · settings">
      {bot.settings_error && <p className="m-0 rounded-md bg-attn-soft px-2 py-1 text-[12px] text-attn">{bot.settings_error}</p>}
      <FileEditor key={`s-${bot.version}`} value={bot.settings_text} mono onSave={(t) => save({ settings_text: t })} rows={8} />
    </Section>
    <Section title="Versions · last 30 days">
      {history.isLoading ? <Loading what="versions" /> : (history.data ?? []).length === 0 ? <p className="m-0 text-[12px] text-ink-3">No change yet.</p>
        : (history.data ?? []).map((c) => <Version key={c.id} bot={bot} change={c} />)}
    </Section>
  </div>;
}

function FileEditor({ value, onSave, rows, mono }: { value: string; onSave: (text: string) => Promise<boolean>; rows: number; mono?: boolean }) {
  const [text, setText] = useState(value);
  const [busy, setBusy] = useState(false);
  const dirty = text !== value;
  return <div className="grid gap-1.5">
    <textarea className={`field-input min-h-[120px] py-1.5 text-[12px] leading-5 ${mono ? "font-mono" : ""}`} rows={rows} value={text} onChange={(e) => setText(e.target.value)} spellCheck={!mono} />
    {dirty && <div className="flex justify-end gap-1.5"><button className="btn btn-sm btn-ghost" onClick={() => setText(value)}>Discard</button><button className="btn btn-sm btn-primary" disabled={busy} onClick={async () => { setBusy(true); await onSave(text); setBusy(false); }}>Save</button></div>}
  </div>;
}

function Version({ bot, change }: { bot: Bot; change: Change }) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const diff = useQuery({ queryKey: ["bot-diff", bot.id, change.id], queryFn: () => api.get<{ patch: string }>(`/api/bots/${bot.id}/history/${change.id}`), enabled: open, staleTime: Infinity });
  return <div className="grid gap-1">
    <div className="flex items-center gap-2 text-[12px]">
      <button className="min-w-0 flex-1 truncate text-left hover:underline" aria-expanded={open} onClick={() => setOpen(!open)}>{change.path} · {change.author?.name ?? "You"} · {formatAge(change.at)} ago{change.added != null && <span className="text-ink-3"> · +{change.added} −{change.removed ?? 0}</span>}</button>
      {change.sha256 && <ConfirmButton icon="refresh" tone="publish" consequence={`Puts back ${change.path} as it was before this change. The restore is itself a version.`} confirmLabel="Restore" onConfirm={async () => { try { await api.post(`/api/bots/${bot.id}/restore/${change.id}`); await qc.invalidateQueries({ queryKey: ["bots"] }); toast("Restored"); } catch (e) { toastError(e); } }}>Restore previous</ConfirmButton>}
    </div>
    {open && (diff.isLoading ? <Loading what="the diff" /> : diff.data?.patch ? <PatchView patch={diff.data.patch} path={change.path} language={change.path.endsWith(".toml") ? "toml" : "markdown"} split={false} wrap /> : <p className="m-0 text-[12px] text-ink-3">No text difference.</p>)}
  </div>;
}

// ---------------------------------------------------------------- sessions

function ThreadPanel({ bot, thread, current }: { bot: Bot; thread: Session[]; current: Session | null }) {
  return <div data-tour="bots.sessions">
    <Section title="One thread, many sessions">
      <p className="m-0 text-[12px] text-ink-2">Your messages go to the current session. A new session starts when the bot's profile or folder changes, or when you click New session. It reads the previous session's trace only when it needs it, and its notes in the knowledge base.</p>
    </Section>
    <Section title={`Sessions · ${thread.length}`}>
      {thread.length === 0 ? <p className="m-0 text-[12px] text-ink-3">{bot.settings.name} has not run yet.</p>
        : [...thread].reverse().map((s, i) => {
          const context = s.usage.context_tokens != null && s.usage.context_window ? `${Math.round((100 * s.usage.context_tokens) / s.usage.context_window)}% context` : null;
          return <Link key={s.id} to="/agents" search={{ session: s.id } as never} className="grid gap-0.5 rounded-md px-2 py-1.5 no-underline hover:bg-sunk">
            <span className="flex items-center gap-2 text-[13px] text-ink"><span className="min-w-0 flex-1 truncate">Session {thread.length - i}{s.id === current?.id ? " · current" : ""}</span><HarnessMark harness={s.harness} size={12} /></span>
            <span className="truncate text-[12px] text-ink-3">{s.title} · {s.status === "closed" ? "archived" : s.status} · {formatAge(s.started_at)} ago{context ? ` · ${context}` : ""}</span>
          </Link>;
        })}
    </Section>
  </div>;
}

// ---------------------------------------------------------------- settings

function SettingsPanel({ bot }: { bot: Bot }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const profiles = useProfiles();
  const projects = useProjects().data ?? [];
  const workspaces = useWorkspaces().data ?? [];
  const sessions = useBotSessions();
  const [s, setS] = useState<BotSettings>(bot.settings);
  useEffect(() => setS(bot.settings), [bot.settings]);
  const profile = profiles.find((p) => p.id === s.profile) ?? null;
  const models = useHarnessDefaults(profile && !profile.models_url ? profile.harness : null).data?.models ?? [];
  const folders = projects.filter((p) => p.workspace_id === bot.workspace_id && !p.is_temp);
  const dirty = JSON.stringify(s) !== JSON.stringify(bot.settings);
  const running = botThread(sessions, bot.id).some((x) => ["starting", "working", "background"].includes(x.status));
  const save = async () => {
    try { await api.patch<Bot>(`/api/bots/${bot.id}`, { settings: s }); await qc.invalidateQueries({ queryKey: ["bots"] }); toast(s.profile !== bot.settings.profile || s.folder !== bot.settings.folder ? "Saved: the next message starts a new session" : "Saved"); } catch (e) { toastError(e); }
  };
  return <div data-tour="bots.settings">
    <Section title="Settings">
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Name<input className="field-input h-[30px] font-normal" maxLength={40} value={s.name} onChange={(e) => setS({ ...s, name: e.target.value })} /></label>
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Profile
        <select className="field-input h-[30px] font-normal" value={s.profile ?? ""} onChange={(e) => setS({ ...s, profile: e.target.value || undefined, model: undefined, effort: undefined })}>
          {!profile && <option value="">Choose a profile</option>}
          {profiles.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
      </label>
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Model
        <select className="field-input h-[30px] font-normal" value={s.model ?? ""} onChange={(e) => setS({ ...s, model: e.target.value || undefined })}>
          <option value="">Default of the profile{profile?.model ? ` (${profile.model})` : ""}</option>
          {models.map((m) => <option key={m.id} value={m.id}>{m.display_name || m.id}</option>)}
          {s.model && !models.some((m) => m.id === s.model) && <option value={s.model}>{s.model}</option>}
        </select>
      </label>
      <label className="grid gap-1 text-[12px] font-semibold text-ink-2">Folder
        <select className="field-input h-[30px] font-normal" value={s.folder} onChange={(e) => setS({ ...s, folder: e.target.value })}>
          {!folders.some((p) => p.id === s.folder) && <option value={s.folder}>A removed folder</option>}
          {folders.map((p) => <option key={p.id} value={p.id}>{p.name}{p.plain ? " · plain folder" : ""}</option>)}
        </select>
      </label>
      <fieldset className="m-0 grid gap-1 border-0 p-0">
        <legend className="pb-1 text-[12px] font-semibold text-ink-2">Permissions</legend>
        {PERMISSIONS.map((p) => <label key={p.id} className="flex min-h-[28px] items-start gap-2 text-[13px]"><input type="radio" className="mt-1" name="bot-permission" checked={s.permission_mode === p.id} onChange={() => setS({ ...s, permission_mode: p.id })} /><span><span className="font-semibold">{p.label}</span> <span className="text-ink-3">· {p.detail}</span></span></label>)}
      </fieldset>
      {dirty && <div className="flex justify-end gap-1.5"><button className="btn btn-sm btn-ghost" onClick={() => setS(bot.settings)}>Discard</button><button className="btn btn-sm btn-primary" disabled={!s.name.trim()} onClick={() => void save()}>Save</button></div>}
    </Section>
    <Section title="What it can reach">
      <Access folder={projects.find((p) => p.id === s.folder) ?? null} workspace={workspaces.find((w) => w.id === bot.workspace_id)?.name ?? ""} workspaceId={bot.workspace_id} mode={s.permission_mode} />
    </Section>
    <Section title="Delete">
      <p className="m-0 text-[12px] text-ink-3">Deleting removes the bot, its files and its routines. Its sessions stay in Agents, its notes stay in the knowledge base{bot.own_folder ? ", and its folder stays in Folders" : ""}.</p>
      <ConfirmButton icon="trash" disabled={running} title={running ? "Stop its session first" : undefined} consequence={`Delete ${bot.settings.name}? Its sessions, notes${bot.own_folder ? " and folder" : ""} stay.`} confirmLabel={`Delete ${bot.settings.name}`} onConfirm={async () => { try { await api.delete(`/api/bots/${bot.id}`); await qc.invalidateQueries({ queryKey: ["bots"] }); void navigate({ to: "/bots" }); } catch (e) { toastError(e); } }}>Delete bot</ConfirmButton>
    </Section>
  </div>;
}
